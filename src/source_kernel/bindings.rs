use std::cell::RefCell;
use std::collections::BTreeMap;

use ra_ap_syntax::ast::{self, HasName};
use ra_ap_syntax::{AstNode, SyntaxNode, TextSize, WalkEvent};

use super::aliases::{AliasMap, Provenance};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Binding {
    Value(ast::Expr),
    Typed(ast::Type),
    Indeterminate,
    Unbound,
}

#[derive(Debug, Default)]
pub(super) struct Resolver {
    body: RefCell<Option<BodyIndex>>,
}

impl Resolver {
    pub(super) fn binding(&self, aliases: &AliasMap, site: &SyntaxNode, name: &str) -> Binding {
        self.local(site, |index| {
            index
                .nearest(site, name)
                .map(|pattern| index.let_value(pattern, site, name))
        })
        .unwrap_or_else(|| constant(aliases, site, name))
    }

    pub(super) fn bound_locally(&self, site: &SyntaxNode, name: &str) -> bool {
        self.local(site, |index| index.nearest(site, name).map(|_| ())).is_some()
    }

    fn local<T>(&self, site: &SyntaxNode, answer: impl FnOnce(&BodyIndex) -> Option<T>) -> Option<T> {
        let body = site.ancestors().find_map(ast::Fn::cast)?;
        let mut cache = self.body.borrow_mut();
        if cache.as_ref().is_none_or(|index| index.body != body) {
            *cache = Some(BodyIndex::build(body));
        }
        cache.as_ref().and_then(answer)
    }
}

#[derive(Debug)]
struct BodyIndex {
    body: ast::Fn,
    patterns: BTreeMap<String, Vec<ast::IdentPat>>,
    assignments: BTreeMap<String, Vec<TextSize>>,
}

impl BodyIndex {
    fn build(body: ast::Fn) -> Self {
        let mut patterns: BTreeMap<String, Vec<ast::IdentPat>> = BTreeMap::new();
        let mut assignments: BTreeMap<String, Vec<TextSize>> = BTreeMap::new();
        let mut walk = body.syntax().preorder();
        while let Some(event) = walk.next() {
            let WalkEvent::Enter(node) = event else {
                continue;
            };
            if node != *body.syntax() && ast::Fn::can_cast(node.kind()) {
                walk.skip_subtree();
            } else if let Some(pattern) = ast::IdentPat::cast(node.clone()) {
                if let Some(name) = pattern.name() {
                    patterns.entry(name.text().to_string()).or_default().push(pattern);
                }
            } else if let Some(assignment) = ast::BinExpr::cast(node)
                && matches!(assignment.op_kind(), Some(ast::BinaryOp::Assignment { .. }))
                && let Some(name) = assignment.lhs().as_ref().and_then(identifier)
            {
                assignments
                    .entry(name)
                    .or_default()
                    .push(assignment.syntax().text_range().start());
            }
        }
        Self {
            body,
            patterns,
            assignments,
        }
    }

    fn nearest(&self, site: &SyntaxNode, name: &str) -> Option<&ast::IdentPat> {
        let start = site.text_range().start();
        self.patterns.get(name)?.iter().rev().find(|pattern| {
            pattern.syntax().text_range().start() < start
                && !pattern
                    .syntax()
                    .parent()
                    .and_then(ast::LetStmt::cast)
                    .is_some_and(|statement| {
                        statement.syntax().text_range().contains_range(site.text_range())
                    })
        })
    }

    fn let_value(&self, pattern: &ast::IdentPat, site: &SyntaxNode, name: &str) -> Binding {
        let plain = pattern.ref_token().is_none() && pattern.pat().is_none();
        let typed = || {
            plain
                .then(|| declared_type(pattern, site))
                .flatten()
                .map_or(Binding::Indeterminate, Binding::Typed)
        };
        let Some(statement) = pattern.syntax().parent().and_then(ast::LetStmt::cast) else {
            return typed();
        };
        let encloses = statement
            .syntax()
            .parent()
            .is_some_and(|block| block.text_range().contains_range(site.text_range()));
        if !encloses {
            return Binding::Indeterminate;
        }
        let Some(initializer) = statement.initializer() else {
            return typed();
        };
        let after = statement.syntax().text_range().end();
        let reassigned = self
            .assignments
            .get(name)
            .is_some_and(|starts| starts.iter().any(|start| *start >= after));
        if !plain || statement.let_else().is_some() || reassigned {
            return typed();
        }
        Binding::Value(initializer)
    }
}

fn declared_type(pattern: &ast::IdentPat, site: &SyntaxNode) -> Option<ast::Type> {
    let parent = pattern.syntax().parent()?;
    if let Some(statement) = ast::LetStmt::cast(parent.clone()) {
        return statement.ty();
    }
    let parameter = ast::Param::cast(parent)?;
    let owner = parameter.syntax().parent()?.parent()?;
    owner
        .text_range()
        .contains_range(site.text_range())
        .then(|| parameter.ty())
        .flatten()
}

fn constant(aliases: &AliasMap, site: &SyntaxNode, name: &str) -> Binding {
    match aliases.provenance(site, name) {
        Provenance::Unbound => {}
        Provenance::Item(_) => return Binding::Unbound,
        Provenance::GlobMasked(_) | Provenance::Indeterminate => return Binding::Indeterminate,
    }
    for scope in site.ancestors() {
        let items = if let Some(block) = ast::BlockExpr::cast(scope.clone()) {
            block.stmt_list().map(|list| list.syntax().clone())
        } else if ast::SourceFile::cast(scope.clone()).is_some() {
            Some(scope.clone())
        } else if let Some(module) = ast::Module::cast(scope.clone()) {
            module.item_list().map(|list| list.syntax().clone())
        } else {
            continue;
        };
        let mut found = items
            .iter()
            .flat_map(SyntaxNode::children)
            .filter_map(ast::Const::cast)
            .filter(|item| item.name().is_some_and(|declared| declared.text() == name));
        match (found.next(), found.next()) {
            (Some(item), None) => {
                return item.body().map_or(Binding::Indeterminate, Binding::Value);
            }
            (Some(_), Some(_)) => return Binding::Indeterminate,
            (None, _) => {}
        }
        if ast::SourceFile::cast(scope.clone()).is_some() || ast::Module::cast(scope).is_some() {
            break;
        }
    }
    Binding::Unbound
}

pub(super) fn identifier(expression: &ast::Expr) -> Option<String> {
    let ast::Expr::PathExpr(path) = expression else {
        return None;
    };
    let path = path.path()?;
    if path.qualifier().is_some() {
        return None;
    }
    let segment = path.segment()?;
    if segment.syntax().children().count() != 1 {
        return None;
    }
    match segment.kind()? {
        ast::PathSegmentKind::Name(name) => Some(name.text().to_string()),
        _ => None,
    }
}
