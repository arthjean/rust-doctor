use super::{CustomRule, RuleContext, has_cfg_test, is_test_context};
use crate::catalog::Confidence;
use crate::diagnostics::{Category, Diagnostic, Severity, SourceSurface};
use std::path::Path;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, ExprMethodCall, ItemFn, ItemImpl, ItemMod};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReliabilityKind {
    AwaitHoldingRefcellRef,
    BlockingLockInAsync,
    CatchUnwindDiscarded,
    MemForgetResource,
    ProcessExitInLibrary,
    SpawnInDrop,
}

struct ReliabilityRule {
    kind: ReliabilityKind,
}

impl ReliabilityRule {
    const fn new(kind: ReliabilityKind) -> Self {
        Self { kind }
    }

    fn analyze(&self, syntax: &syn::File, path: &Path, context: RuleContext) -> Vec<Diagnostic> {
        if matches!(
            context.source_surface,
            SourceSurface::Test | SourceSurface::Bench | SourceSurface::Example
        ) || (self.kind == ReliabilityKind::ProcessExitInLibrary
            && context.source_surface != SourceSurface::Library)
        {
            return Vec::new();
        }
        let mut visitor = ReliabilityVisitor {
            rule: self,
            path,
            diagnostics: Vec::new(),
            async_depth: 0,
            loop_depth: 0,
            drop_depth: 0,
            refcell_borrow_active: false,
        };
        visitor.visit_file(syntax);
        visitor.diagnostics
    }
}

impl CustomRule for ReliabilityRule {
    fn name(&self) -> &'static str {
        match self.kind {
            ReliabilityKind::AwaitHoldingRefcellRef => "await-holding-refcell-ref",
            ReliabilityKind::BlockingLockInAsync => "blocking-lock-in-async",
            ReliabilityKind::CatchUnwindDiscarded => "catch-unwind-discarded",
            ReliabilityKind::MemForgetResource => "mem-forget-resource",
            ReliabilityKind::ProcessExitInLibrary => "process-exit-in-library",
            ReliabilityKind::SpawnInDrop => "spawn-in-drop",
        }
    }

    fn category(&self) -> Category {
        match self.kind {
            ReliabilityKind::AwaitHoldingRefcellRef
            | ReliabilityKind::BlockingLockInAsync
            | ReliabilityKind::SpawnInDrop => Category::Async,
            ReliabilityKind::CatchUnwindDiscarded
            | ReliabilityKind::MemForgetResource
            | ReliabilityKind::ProcessExitInLibrary => Category::Correctness,
        }
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn description(&self) -> &'static str {
        match self.kind {
            ReliabilityKind::AwaitHoldingRefcellRef => {
                "Detect RefCell borrows retained across an await point"
            }
            ReliabilityKind::BlockingLockInAsync => {
                "Detect blocking mutex acquisition inside async functions"
            }
            ReliabilityKind::CatchUnwindDiscarded => {
                "Detect panic payloads discarded after catch_unwind"
            }
            ReliabilityKind::MemForgetResource => "Detect resource guards passed to mem::forget",
            ReliabilityKind::ProcessExitInLibrary => {
                "Detect process termination from library source"
            }
            ReliabilityKind::SpawnInDrop => "Detect Tokio task spawning from Drop implementations",
        }
    }

    fn fix_hint(&self) -> &'static str {
        match self.kind {
            ReliabilityKind::AwaitHoldingRefcellRef => {
                "End the RefCell borrow before awaiting, then borrow again after resumption."
            }
            ReliabilityKind::BlockingLockInAsync => {
                "Use an async-aware mutex or prove the critical section cannot block."
            }
            ReliabilityKind::CatchUnwindDiscarded => {
                "Inspect the panic payload and restore or report the failed invariant."
            }
            ReliabilityKind::MemForgetResource => {
                "Release the guard explicitly or transfer ownership through a documented resource API."
            }
            ReliabilityKind::ProcessExitInLibrary => {
                "Return a typed error and let the binary boundary decide whether to exit."
            }
            ReliabilityKind::SpawnInDrop => {
                "Expose an explicit async shutdown method and keep Drop synchronous."
            }
        }
    }

    fn default_enabled(&self) -> bool {
        matches!(
            self.kind,
            ReliabilityKind::CatchUnwindDiscarded
                | ReliabilityKind::MemForgetResource
                | ReliabilityKind::ProcessExitInLibrary
                | ReliabilityKind::SpawnInDrop
        )
    }

    fn confidence(&self) -> Confidence {
        match self.kind {
            ReliabilityKind::AwaitHoldingRefcellRef | ReliabilityKind::BlockingLockInAsync => {
                Confidence::Medium
            }
            ReliabilityKind::CatchUnwindDiscarded
            | ReliabilityKind::MemForgetResource
            | ReliabilityKind::ProcessExitInLibrary
            | ReliabilityKind::SpawnInDrop => Confidence::High,
        }
    }

    fn applicable_frameworks(&self) -> &'static [&'static str] {
        match self.kind {
            ReliabilityKind::AwaitHoldingRefcellRef | ReliabilityKind::BlockingLockInAsync => {
                &["tokio", "async-std", "smol"]
            }
            ReliabilityKind::SpawnInDrop => &["tokio"],
            _ => &[],
        }
    }

    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        self.analyze(
            syntax,
            path,
            RuleContext {
                source_surface: crate::config::classify_source_surface(
                    &path.to_string_lossy(),
                    false,
                ),
            },
        )
    }

    fn check_file_with_context(
        &self,
        syntax: &syn::File,
        path: &Path,
        context: RuleContext,
    ) -> Vec<Diagnostic> {
        self.analyze(syntax, path, context)
    }
}

struct ReliabilityVisitor<'a> {
    rule: &'a ReliabilityRule,
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
    async_depth: usize,
    loop_depth: usize,
    drop_depth: usize,
    refcell_borrow_active: bool,
}

impl ReliabilityVisitor<'_> {
    fn emit(&mut self, span: proc_macro2::Span) {
        let start = span.start();
        self.diagnostics.push(self.rule.diagnostic(
            self.path,
            self.rule.description().to_string(),
            Some(self.rule.fix_hint().to_string()),
            Some(start.line as u32),
            Some(start.column as u32 + 1),
        ));
    }
}

impl<'ast> Visit<'ast> for ReliabilityVisitor<'_> {
    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        if !has_cfg_test(&node.attrs) {
            visit::visit_item_mod(self, node);
        }
    }

    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        if is_test_context(&node.attrs) {
            return;
        }
        let previous_borrow_state = self.refcell_borrow_active;
        self.refcell_borrow_active = false;
        self.async_depth += usize::from(node.sig.asyncness.is_some());
        visit::visit_item_fn(self, node);
        self.async_depth -= usize::from(node.sig.asyncness.is_some());
        self.refcell_borrow_active = previous_borrow_state;
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if is_test_context(&node.attrs) {
            return;
        }
        let previous_borrow_state = self.refcell_borrow_active;
        self.refcell_borrow_active = false;
        self.async_depth += usize::from(node.sig.asyncness.is_some());
        visit::visit_impl_item_fn(self, node);
        self.async_depth -= usize::from(node.sig.asyncness.is_some());
        self.refcell_borrow_active = previous_borrow_state;
    }

    fn visit_block(&mut self, node: &'ast syn::Block) {
        let previous_borrow_state = self.refcell_borrow_active;
        visit::visit_block(self, node);
        self.refcell_borrow_active = previous_borrow_state;
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        visit::visit_local(self, node);
        if self.rule.kind == ReliabilityKind::AwaitHoldingRefcellRef
            && self.async_depth > 0
            && node
                .init
                .as_ref()
                .is_some_and(|init| expression_contains_refcell_borrow(&init.expr))
        {
            self.refcell_borrow_active = true;
        }
    }

    fn visit_expr_await(&mut self, node: &'ast syn::ExprAwait) {
        if self.rule.kind == ReliabilityKind::AwaitHoldingRefcellRef && self.refcell_borrow_active {
            self.emit(node.await_token.span);
        }
        visit::visit_expr_await(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast ItemImpl) {
        let is_drop = node
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last())
            .is_some_and(|segment| segment.ident == "Drop");
        self.drop_depth += usize::from(is_drop);
        visit::visit_item_impl(self, node);
        self.drop_depth -= usize::from(is_drop);
    }

    fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
        self.loop_depth += 1;
        visit::visit_expr_loop(self, node);
        self.loop_depth -= 1;
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        self.loop_depth += 1;
        visit::visit_expr_while(self, node);
        self.loop_depth -= 1;
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.loop_depth += 1;
        visit::visit_expr_for_loop(self, node);
        self.loop_depth -= 1;
    }

    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        match self.rule.kind {
            ReliabilityKind::AwaitHoldingRefcellRef
                if call_path_ends(node, &["drop"]) || call_path_ends(node, &["mem", "drop"]) =>
            {
                self.refcell_borrow_active = false;
            }
            ReliabilityKind::MemForgetResource
                if call_path_ends(node, &["mem", "forget"])
                    && node.args.first().is_some_and(resource_expression) =>
            {
                self.emit(node.span());
            }
            ReliabilityKind::ProcessExitInLibrary if call_path_ends(node, &["process", "exit"]) => {
                self.emit(node.span());
            }
            ReliabilityKind::SpawnInDrop
                if self.drop_depth > 0 && call_path_ends(node, &["tokio", "spawn"]) =>
            {
                self.emit(node.span());
            }
            _ => {}
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        match self.rule.kind {
            ReliabilityKind::BlockingLockInAsync
                if self.async_depth > 0
                    && node.method == "lock"
                    && receiver_name(&node.receiver).is_some_and(|name| name.contains("mutex")) =>
            {
                self.emit(node.span());
            }
            ReliabilityKind::CatchUnwindDiscarded
                if matches!(node.method.to_string().as_str(), "ok" | "unwrap_or_default")
                    && expression_is_call(&node.receiver, &["catch_unwind"]) =>
            {
                self.emit(node.span());
            }
            _ => {}
        }
        visit::visit_expr_method_call(self, node);
    }
}

fn call_path_ends(call: &ExprCall, suffix: &[&str]) -> bool {
    let Expr::Path(path) = call.func.as_ref() else {
        return false;
    };
    let segments: Vec<_> = path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    segments.len() >= suffix.len()
        && segments[segments.len() - suffix.len()..]
            .iter()
            .map(String::as_str)
            .eq(suffix.iter().copied())
}

fn expression_is_call(expression: &Expr, suffix: &[&str]) -> bool {
    matches!(expression, Expr::Call(call) if call_path_ends(call, suffix))
}

fn receiver_name(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string().to_ascii_lowercase()),
        Expr::Field(field) => receiver_name(&field.base),
        Expr::Paren(paren) => receiver_name(&paren.expr),
        _ => None,
    }
}

fn expression_contains_refcell_borrow(expression: &Expr) -> bool {
    struct BorrowVisitor(bool);
    impl<'ast> Visit<'ast> for BorrowVisitor {
        fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
            if matches!(node.method.to_string().as_str(), "borrow" | "borrow_mut") {
                self.0 = true;
            } else {
                visit::visit_expr_method_call(self, node);
            }
        }
    }
    let mut visitor = BorrowVisitor(false);
    visitor.visit_expr(expression);
    visitor.0
}

fn resource_expression(expression: &Expr) -> bool {
    match expression {
        Expr::Path(path) => path.path.segments.last().is_some_and(|segment| {
            let name = segment.ident.to_string().to_ascii_lowercase();
            [
                "guard",
                "lock",
                "file",
                "socket",
                "stream",
                "permit",
                "transaction",
                "handle",
                "child",
            ]
            .iter()
            .any(|marker| name.contains(marker))
        }),
        Expr::Reference(reference) => resource_expression(&reference.expr),
        Expr::Paren(paren) => resource_expression(&paren.expr),
        _ => false,
    }
}

pub fn all_rules() -> Vec<Box<dyn CustomRule>> {
    vec![
        Box::new(ReliabilityRule::new(
            ReliabilityKind::AwaitHoldingRefcellRef,
        )),
        Box::new(ReliabilityRule::new(ReliabilityKind::BlockingLockInAsync)),
        Box::new(ReliabilityRule::new(ReliabilityKind::CatchUnwindDiscarded)),
        Box::new(ReliabilityRule::new(ReliabilityKind::MemForgetResource)),
        Box::new(ReliabilityRule::new(ReliabilityKind::ProcessExitInLibrary)),
        Box::new(ReliabilityRule::new(ReliabilityKind::SpawnInDrop)),
    ]
}
