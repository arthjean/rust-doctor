use super::{CustomRule, RuleContext, has_cfg_test, is_test_context};
use crate::catalog::Confidence;
use crate::diagnostics::{Category, Diagnostic, Severity, SourceSurface};
use std::path::Path;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, ExprLit, ExprMethodCall, ItemFn, ItemMod, Lit};

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrancheKind {
    CommandShellInterpolation,
    FfiCStringLifetime,
    InsecureHttpClient,
    RegexCreatedInLoop,
    TemporaryCStringPointer,
    UnboundedCollect,
}

struct TrancheRule {
    kind: TrancheKind,
}

impl TrancheRule {
    const fn new(kind: TrancheKind) -> Self {
        Self { kind }
    }

    fn analyze(
        &self,
        syntax: &syn::File,
        path: &Path,
        context: RuleContext<'_>,
    ) -> Vec<Diagnostic> {
        if matches!(
            context.source_surface,
            SourceSurface::Test | SourceSurface::Example | SourceSurface::Bench
        ) || (matches!(
            self.kind,
            TrancheKind::RegexCreatedInLoop | TrancheKind::UnboundedCollect
        ) && context.source_surface == SourceSurface::BuildScript)
        {
            return Vec::new();
        }
        let mut visitor = TrancheVisitor {
            rule: self,
            path,
            diagnostics: Vec::new(),
            loop_depth: 0,
        };
        visitor.visit_file(syntax);
        visitor.diagnostics
    }
}

impl CustomRule for TrancheRule {
    fn name(&self) -> &'static str {
        match self.kind {
            TrancheKind::CommandShellInterpolation => "command-shell-interpolation",
            TrancheKind::FfiCStringLifetime => "ffi-cstring-lifetime",
            TrancheKind::InsecureHttpClient => "insecure-http-client",
            TrancheKind::RegexCreatedInLoop => "regex-created-in-loop",
            TrancheKind::TemporaryCStringPointer => "temporary-cstring-pointer",
            TrancheKind::UnboundedCollect => "unbounded-collect",
        }
    }

    fn category(&self) -> Category {
        match self.kind {
            TrancheKind::RegexCreatedInLoop | TrancheKind::UnboundedCollect => {
                Category::Performance
            }
            TrancheKind::CommandShellInterpolation
            | TrancheKind::FfiCStringLifetime
            | TrancheKind::InsecureHttpClient
            | TrancheKind::TemporaryCStringPointer => Category::Security,
        }
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn description(&self) -> &'static str {
        match self.kind {
            TrancheKind::CommandShellInterpolation => {
                "Detect dynamic input passed through a command shell"
            }
            TrancheKind::FfiCStringLifetime => {
                "Detect CString pointers stored or returned after the owner is dropped"
            }
            TrancheKind::InsecureHttpClient => "Detect non-local plain HTTP transport literals",
            TrancheKind::RegexCreatedInLoop => "Detect regular expressions compiled inside loops",
            TrancheKind::TemporaryCStringPointer => {
                "Detect raw pointers borrowed from temporary CString values"
            }
            TrancheKind::UnboundedCollect => {
                "Detect input-like iterators collected without an explicit bound"
            }
        }
    }

    fn fix_hint(&self) -> &'static str {
        match self.kind {
            TrancheKind::CommandShellInterpolation => {
                "Invoke the program directly and pass untrusted values as individual arguments."
            }
            TrancheKind::FfiCStringLifetime => {
                "Bind the CString owner before taking its pointer and keep that binding alive."
            }
            TrancheKind::InsecureHttpClient => {
                "Use HTTPS or document and narrowly suppress a trusted local transport."
            }
            TrancheKind::RegexCreatedInLoop => {
                "Compile the expression once outside the loop or store it in LazyLock."
            }
            TrancheKind::TemporaryCStringPointer => {
                "Bind the CString so its owner outlives every use of the raw pointer."
            }
            TrancheKind::UnboundedCollect => {
                "Apply a validated take limit or stream the input without collecting it all."
            }
        }
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn confidence(&self) -> Confidence {
        match self.kind {
            TrancheKind::RegexCreatedInLoop | TrancheKind::TemporaryCStringPointer => {
                Confidence::High
            }
            TrancheKind::CommandShellInterpolation
            | TrancheKind::FfiCStringLifetime
            | TrancheKind::InsecureHttpClient => Confidence::Medium,
            TrancheKind::UnboundedCollect => Confidence::Low,
        }
    }

    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        self.analyze(syntax, path, RuleContext::unresolved_for_path(path))
    }

    fn check_file_with_context(
        &self,
        syntax: &syn::File,
        path: &Path,
        context: RuleContext<'_>,
    ) -> Vec<Diagnostic> {
        self.analyze(syntax, path, context)
    }
}

struct TrancheVisitor<'a> {
    rule: &'a TrancheRule,
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
    loop_depth: usize,
}

impl TrancheVisitor<'_> {
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

impl<'ast> Visit<'ast> for TrancheVisitor<'_> {
    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        if !has_cfg_test(&node.attrs) {
            visit::visit_item_mod(self, node);
        }
    }

    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        if !is_test_context(&node.attrs) {
            if self.rule.kind == TrancheKind::FfiCStringLifetime
                && let Some(statement) = node.block.stmts.last()
                && tail_expression(statement).is_some_and(ffi_cstring_pointer)
            {
                self.emit(statement.span());
            }
            visit::visit_item_fn(self, node);
        }
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if !is_test_context(&node.attrs) {
            if self.rule.kind == TrancheKind::FfiCStringLifetime
                && let Some(statement) = node.block.stmts.last()
                && tail_expression(statement).is_some_and(ffi_cstring_pointer)
            {
                self.emit(statement.span());
            }
            visit::visit_impl_item_fn(self, node);
        }
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        if self.rule.kind == TrancheKind::FfiCStringLifetime
            && node
                .init
                .as_ref()
                .is_some_and(|init| ffi_cstring_pointer(&init.expr))
        {
            self.emit(node.span());
        }
        visit::visit_local(self, node);
    }

    fn visit_expr_return(&mut self, node: &'ast syn::ExprReturn) {
        if self.rule.kind == TrancheKind::FfiCStringLifetime
            && node.expr.as_deref().is_some_and(ffi_cstring_pointer)
        {
            self.emit(node.span());
        }
        visit::visit_expr_return(self, node);
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
            TrancheKind::RegexCreatedInLoop
                if self.loop_depth > 0 && call_path_ends(node, &["Regex", "new"]) =>
            {
                self.emit(node.span());
            }
            _ => {}
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        match self.rule.kind {
            TrancheKind::CommandShellInterpolation if dynamic_shell_argument(node) => {
                self.emit(node.span());
            }
            TrancheKind::InsecureHttpClient if insecure_http_argument(node) => {
                self.emit(node.span());
            }
            TrancheKind::TemporaryCStringPointer if temporary_cstring_pointer(node) => {
                self.emit(node.span());
            }
            TrancheKind::UnboundedCollect if unbounded_input_collect(node) => {
                self.emit(node.span());
            }
            _ => {}
        }
        visit::visit_expr_method_call(self, node);
    }
}

fn dynamic_shell_argument(call: &ExprMethodCall) -> bool {
    if call.method != "arg" || call.args.len() != 1 || string_literal(call.args.first()).is_some() {
        return false;
    }
    let Expr::MethodCall(flag) = call.receiver.as_ref() else {
        return false;
    };
    if flag.method != "arg"
        || !string_literal(flag.args.first())
            .is_some_and(|value| matches!(value.as_str(), "-c" | "/C" | "-Command"))
    {
        return false;
    }
    let Expr::Call(command) = flag.receiver.as_ref() else {
        return false;
    };
    call_path_ends(command, &["Command", "new"])
        && string_literal(command.args.first())
            .is_some_and(|value| matches!(value.as_str(), "sh" | "bash" | "cmd" | "powershell"))
}

fn insecure_http_argument(call: &ExprMethodCall) -> bool {
    if !matches!(
        call.method.to_string().as_str(),
        "get" | "post" | "put" | "delete" | "patch" | "request"
    ) {
        return false;
    }
    call.args
        .iter()
        .filter_map(|argument| string_literal(Some(argument)))
        .any(|value| {
            value.starts_with("http://")
                && !value.starts_with("http://localhost")
                && !value.starts_with("http://127.0.0.1")
                && !value.starts_with("http://[::1]")
        })
}

fn temporary_cstring_pointer(call: &ExprMethodCall) -> bool {
    if call.method != "as_ptr" {
        return false;
    }
    let Expr::MethodCall(unwrap) = call.receiver.as_ref() else {
        return false;
    };
    if !matches!(unwrap.method.to_string().as_str(), "unwrap" | "expect") {
        return false;
    }
    matches!(unwrap.receiver.as_ref(), Expr::Call(constructor) if call_path_ends(constructor, &["CString", "new"]))
}

fn ffi_cstring_pointer(expression: &Expr) -> bool {
    let expression = match expression {
        Expr::Paren(paren) => return ffi_cstring_pointer(&paren.expr),
        Expr::Group(group) => return ffi_cstring_pointer(&group.expr),
        expression => expression,
    };
    let Expr::MethodCall(pointer) = expression else {
        return false;
    };
    if pointer.method != "as_ptr" {
        return false;
    }
    let Expr::Try(try_expression) = pointer.receiver.as_ref() else {
        return false;
    };
    matches!(
        try_expression.expr.as_ref(),
        Expr::Call(constructor) if call_path_ends(constructor, &["CString", "new"])
    )
}

const fn tail_expression(statement: &syn::Stmt) -> Option<&Expr> {
    match statement {
        syn::Stmt::Expr(expression, None) => Some(expression),
        _ => None,
    }
}

fn unbounded_input_collect(call: &ExprMethodCall) -> bool {
    call.method == "collect"
        && !method_chain_contains(&call.receiver, "take")
        && ["lines", "bytes", "split", "split_whitespace"]
            .iter()
            .any(|method| method_chain_contains(&call.receiver, method))
        && receiver_root_name(&call.receiver).is_some_and(|name| {
            ["input", "reader", "request", "response", "body", "stream"]
                .iter()
                .any(|marker| name.contains(marker))
        })
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

fn string_literal(expression: Option<&Expr>) -> Option<String> {
    match expression {
        Some(Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        })) => Some(value.value()),
        _ => None,
    }
}

fn method_chain_contains(expression: &Expr, expected: &str) -> bool {
    match expression {
        Expr::MethodCall(call) => {
            call.method == expected || method_chain_contains(&call.receiver, expected)
        }
        Expr::Paren(paren) => method_chain_contains(&paren.expr, expected),
        _ => false,
    }
}

fn receiver_root_name(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string().to_ascii_lowercase()),
        Expr::MethodCall(call) => receiver_root_name(&call.receiver),
        Expr::Field(field) => receiver_root_name(&field.base),
        Expr::Paren(paren) => receiver_root_name(&paren.expr),
        _ => None,
    }
}

pub fn all_rules() -> Vec<Box<dyn CustomRule>> {
    vec![
        Box::new(TrancheRule::new(TrancheKind::CommandShellInterpolation)),
        Box::new(TrancheRule::new(TrancheKind::FfiCStringLifetime)),
        Box::new(TrancheRule::new(TrancheKind::InsecureHttpClient)),
        Box::new(TrancheRule::new(TrancheKind::RegexCreatedInLoop)),
        Box::new(TrancheRule::new(TrancheKind::TemporaryCStringPointer)),
        Box::new(TrancheRule::new(TrancheKind::UnboundedCollect)),
    ]
}
