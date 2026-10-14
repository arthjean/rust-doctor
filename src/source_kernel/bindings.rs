//! What an identifier was bound to, read from the function body it sits in.
//!
//! The alias map answers which item a path names; this answers the question
//! one step closer to the code: which expression a bare identifier holds. It
//! follows a `let` to its initializer and a `const` item of the enclosing
//! module or block to its value, and nothing further. The answer is closed
//! like the alias map's: whenever the binding could hold something other than
//! its initializer, the answer is `Indeterminate` and the detector abstains.
//! That covers a binding reassigned after it was made, one bound by a pattern
//! other than a plain name, one taken from a parameter, and one shadowed by a
//! second binding between it and the use, in a branch or anywhere else.
//! Closures, loops and method calls taking `&mut self` are not followed: a
//! builder mutated through a method is still the builder it was initialized to.

use std::cell::RefCell;
use std::collections::BTreeMap;

use ra_ap_syntax::ast::{self, HasName};
use ra_ap_syntax::{AstNode, SyntaxNode, TextSize, WalkEvent};

use super::aliases::{AliasMap, Provenance};

/// What an identifier holds at one point of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Binding {
    /// A `let` binding or a `const` item, initialized to this expression.
    Value(ast::Expr),
    /// Bound, but not to something the detectors can read: a parameter, a
    /// pattern, a reassignment, a shadowing between the binding and the use,
    /// two `const` items of one name, or a glob that may bind it.
    Indeterminate,
    /// No local binding and no `const` item of the module carries the name.
    Unbound,
}

/// Answers `binding` for one unit, indexing each function body once.
///
/// The detectors ask about the bindings of the body they are in, one call
/// site after the other, and the walk visits a body's call sites together:
/// a single-slot cache of the last body indexed turns a scan of the body per
/// question into one scan per body.
#[derive(Debug, Default)]
pub(super) struct Resolver {
    body: RefCell<Option<BodyIndex>>,
}

impl Resolver {
    /// Binding of `name` where `site` sits.
    pub(super) fn binding(&self, aliases: &AliasMap, site: &SyntaxNode, name: &str) -> Binding {
        if let Some(body) = site.ancestors().find_map(ast::Fn::cast) {
            let mut cache = self.body.borrow_mut();
            if cache.as_ref().is_none_or(|index| index.body != body) {
                *cache = Some(BodyIndex::build(body));
            }
            if let Some(index) = cache.as_ref()
                && let Some(pattern) = index.nearest(site, name)
            {
                return index.let_value(pattern, site, name);
            }
        }
        constant(aliases, site, name)
    }
}

/// The bindings and assignments of one function body, nested items left out.
#[derive(Debug)]
struct BodyIndex {
    body: ast::Fn,
    /// Identifier patterns by name, in source order. A parameter, a closure
    /// parameter, a match arm and a `let` are all identifier patterns.
    patterns: BTreeMap<String, Vec<ast::IdentPat>>,
    /// Where an assignment, plain or compound, targets a bare identifier.
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
            // A function nested in the body is a scope of its own: nothing it
            // binds is visible outside it.
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

    /// The last binding of `name` written before `site`, whatever its form.
    /// A `let` whose own initializer holds the use is skipped, since `let cmd
    /// = cmd.arg(..)` reads the binding before it.
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

    /// The initializer of the `let` that binds `pattern`, when that `let` is
    /// the only thing the name can hold at `site`: a plain name, in a block
    /// enclosing the use, never assigned again. `insecure &= check` counts as
    /// an assignment, since it leaves a flag nobody can read off its `let`.
    fn let_value(&self, pattern: &ast::IdentPat, site: &SyntaxNode, name: &str) -> Binding {
        let plain = pattern.ref_token().is_none() && pattern.pat().is_none();
        let Some(statement) = pattern.syntax().parent().and_then(ast::LetStmt::cast) else {
            return Binding::Indeterminate;
        };
        let Some(initializer) = statement.initializer() else {
            return Binding::Indeterminate;
        };
        let encloses = statement
            .syntax()
            .parent()
            .is_some_and(|block| block.text_range().contains_range(site.text_range()));
        let after = statement.syntax().text_range().end();
        let reassigned = self
            .assignments
            .get(name)
            .is_some_and(|starts| starts.iter().any(|start| *start >= after));
        if !plain || statement.let_else().is_some() || !encloses || reassigned {
            return Binding::Indeterminate;
        }
        Binding::Value(initializer)
    }
}

/// A `const` item named `name`, in the nearest block or module scope around
/// `site` that declares one. An item is visible anywhere in its scope, so the
/// order does not matter; two items of one name, which `cfg` makes possible,
/// are indeterminate.
fn constant(aliases: &AliasMap, site: &SyntaxNode, name: &str) -> Binding {
    match aliases.provenance(site, name) {
        Provenance::Unbound => {}
        Provenance::Item(_) => return Binding::Unbound,
        Provenance::Indeterminate => return Binding::Indeterminate,
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

/// The name an expression reads when it is a bare identifier: one plain path
/// segment, no qualifier, no generic arguments.
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
