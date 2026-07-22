use super::{CustomRule, has_cfg_test, is_test_context};
use crate::catalog::Confidence;
use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::discovery::{Framework, FrameworkCapability};
use std::path::Path;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, GenericArgument, ItemFn, ItemMod, PathArguments, Type};

#[derive(Clone, Copy, PartialEq, Eq)]
enum PackKind {
    ActixWebDataLock,
    AxumExtensionRequestState,
    TokioUnboundedChannel,
}

struct FrameworkPackRule {
    kind: PackKind,
}

impl FrameworkPackRule {
    const fn new(kind: PackKind) -> Self {
        Self { kind }
    }
}

impl CustomRule for FrameworkPackRule {
    fn name(&self) -> &'static str {
        match self.kind {
            PackKind::ActixWebDataLock => "actix-web-data-lock",
            PackKind::AxumExtensionRequestState => "axum-extension-request-state",
            PackKind::TokioUnboundedChannel => "tokio-unbounded-channel",
        }
    }

    fn category(&self) -> Category {
        Category::Framework
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn description(&self) -> &'static str {
        match self.kind {
            PackKind::ActixWebDataLock => {
                "Detect blocking shared-state locks inside actix-web handlers"
            }
            PackKind::AxumExtensionRequestState => {
                "Detect Axum Extension used for application-wide state"
            }
            PackKind::TokioUnboundedChannel => {
                "Detect Tokio unbounded channels without backpressure"
            }
        }
    }

    fn fix_hint(&self) -> &'static str {
        match self.kind {
            PackKind::ActixWebDataLock => {
                "Use an async-aware lock or move short blocking work behind web::block."
            }
            PackKind::AxumExtensionRequestState => {
                "Use State<T> for router-owned application state and reserve Extension for request extensions."
            }
            PackKind::TokioUnboundedChannel => {
                "Choose a bounded mpsc channel and make the producer handle backpressure."
            }
        }
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn confidence(&self) -> Confidence {
        Confidence::Medium
    }

    fn applicable_frameworks(&self) -> &'static [&'static str] {
        match self.kind {
            PackKind::ActixWebDataLock => &["actix-web"],
            PackKind::AxumExtensionRequestState => &["axum"],
            PackKind::TokioUnboundedChannel => &["tokio"],
        }
    }

    fn framework_version_requirements(&self) -> &'static [(&'static str, &'static str)] {
        match self.kind {
            PackKind::ActixWebDataLock => &[("actix-web", ">=4,<5")],
            PackKind::AxumExtensionRequestState => &[("axum", ">=0.7,<0.9")],
            PackKind::TokioUnboundedChannel => &[("tokio", ">=1,<2")],
        }
    }

    fn required_framework_features(&self) -> &'static [(&'static str, &'static [&'static str])] {
        match self.kind {
            PackKind::ActixWebDataLock | PackKind::AxumExtensionRequestState => &[],
            PackKind::TokioUnboundedChannel => &[("tokio", &["sync"])],
        }
    }

    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        let mut visitor = PackVisitor {
            rule: self,
            path,
            diagnostics: Vec::new(),
            actix_locked_state_depth: 0,
        };
        visitor.visit_file(syntax);
        visitor.diagnostics
    }
}

struct PackVisitor<'a> {
    rule: &'a FrameworkPackRule,
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
    actix_locked_state_depth: usize,
}

impl PackVisitor<'_> {
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

impl<'ast> Visit<'ast> for PackVisitor<'_> {
    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        if !has_cfg_test(&node.attrs) {
            visit::visit_item_mod(self, node);
        }
    }

    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        if is_test_context(&node.attrs) {
            return;
        }
        match self.rule.kind {
            PackKind::AxumExtensionRequestState
                if node.sig.asyncness.is_some()
                    && node.sig.inputs.iter().any(|argument| {
                        matches!(argument, syn::FnArg::Typed(typed) if type_has_wrapper(&typed.ty, "Extension") && type_contains(&typed.ty, "Arc"))
                    }) =>
            {
                self.emit(node.sig.ident.span());
            }
            PackKind::ActixWebDataLock if node.sig.asyncness.is_some() => {
                let locked_state = node.sig.inputs.iter().any(|argument| {
                    matches!(argument, syn::FnArg::Typed(typed) if type_has_wrapper(&typed.ty, "Data") && (type_contains(&typed.ty, "Mutex") || type_contains(&typed.ty, "RwLock")))
                });
                self.actix_locked_state_depth += usize::from(locked_state);
                visit::visit_item_fn(self, node);
                self.actix_locked_state_depth -= usize::from(locked_state);
                return;
            }
            _ => {}
        }
        visit::visit_item_fn(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if self.rule.kind == PackKind::TokioUnboundedChannel
            && call_path_ends(node, &["mpsc", "unbounded_channel"])
        {
            self.emit(node.span());
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if self.rule.kind == PackKind::ActixWebDataLock
            && self.actix_locked_state_depth > 0
            && node.method == "lock"
        {
            self.emit(node.span());
        }
        visit::visit_expr_method_call(self, node);
    }
}

fn type_has_wrapper(ty: &Type, expected: &str) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == expected))
}

fn type_contains(ty: &Type, expected: &str) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    path.path.segments.iter().any(|segment| {
        segment.ident == expected
            || match &segment.arguments {
                PathArguments::AngleBracketed(arguments) => arguments.args.iter().any(|argument| {
                    matches!(argument, GenericArgument::Type(inner) if type_contains(inner, expected))
                }),
                _ => false,
            }
    })
}

fn call_path_ends(call: &syn::ExprCall, suffix: &[&str]) -> bool {
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

fn capability_decision(
    rule: &dyn CustomRule,
    capabilities: &[FrameworkCapability],
) -> Result<(), String> {
    let Some(framework_name) = rule.applicable_frameworks().first() else {
        return Ok(());
    };
    let Some(framework) = framework_from_name(framework_name) else {
        return Err("unknown framework capability".to_string());
    };
    let Some(capability) = capabilities
        .iter()
        .find(|capability| capability.framework == framework)
    else {
        return Err(
            "no direct dependency capability; renamed or re-exported frameworks abstain"
                .to_string(),
        );
    };
    if !capability.active {
        return Err(capability
            .gate_reason
            .clone()
            .unwrap_or_else(|| "framework capability is inactive".to_string()));
    }
    let Some(version) = capability.version.as_deref() else {
        return Err("dependency version is unknown".to_string());
    };
    let requirement = rule
        .framework_version_requirements()
        .iter()
        .find_map(|(name, requirement)| (*name == *framework_name).then_some(*requirement))
        .unwrap_or("*");
    if !version_matches(version, requirement) {
        return Err(format!(
            "dependency version {version} is outside {requirement}"
        ));
    }
    let required_features = rule
        .required_framework_features()
        .iter()
        .find_map(|(name, features)| (*name == *framework_name).then_some(*features))
        .unwrap_or_default();
    let missing: Vec<_> = required_features
        .iter()
        .filter(|feature| {
            !capability
                .enabled_features
                .iter()
                .any(|active| active == **feature)
        })
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "required Cargo features are disabled: {}",
            missing.join(", ")
        ));
    }
    if capability.target_contexts.is_empty() {
        return Err("framework has no active target context".to_string());
    }
    Ok(())
}

fn framework_from_name(name: &str) -> Option<Framework> {
    match name {
        "actix-web" => Some(Framework::ActixWeb),
        "axum" => Some(Framework::Axum),
        "tokio" => Some(Framework::Tokio),
        _ => None,
    }
}

fn version_matches(version: &str, requirement: &str) -> bool {
    if requirement == "*" {
        return true;
    }
    let mut parts = version
        .split('.')
        .filter_map(|part| part.parse::<u64>().ok());
    let Some(major) = parts.next() else {
        return false;
    };
    let minor = parts.next().unwrap_or(0);
    match requirement {
        ">=1,<2" => major == 1,
        ">=1,<3" => (1..3).contains(&major),
        ">=4,<5" => major == 4,
        ">=0.7,<0.9" => major == 0 && (7..9).contains(&minor),
        _ => false,
    }
}

pub fn all_rules() -> Vec<Box<dyn CustomRule>> {
    vec![
        Box::new(FrameworkPackRule::new(PackKind::ActixWebDataLock)),
        Box::new(FrameworkPackRule::new(PackKind::AxumExtensionRequestState)),
        Box::new(FrameworkPackRule::new(PackKind::TokioUnboundedChannel)),
    ]
}

pub fn rules_for_capabilities(
    capabilities: &[FrameworkCapability],
    verbose: bool,
) -> Vec<Box<dyn CustomRule>> {
    all_rules()
        .into_iter()
        .filter(
            |rule| match capability_decision(rule.as_ref(), capabilities) {
                Ok(()) => {
                    if verbose {
                        eprintln!("Framework pack {}: capability active", rule.name());
                    }
                    true
                }
                Err(reason) => {
                    if verbose {
                        eprintln!("Framework pack {}: skipped ({reason})", rule.name());
                    }
                    false
                }
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability(
        framework: Framework,
        version: Option<&str>,
        features: &[&str],
    ) -> FrameworkCapability {
        FrameworkCapability {
            framework,
            version: version.map(str::to_string),
            enabled_features: features
                .iter()
                .map(|feature| (*feature).to_string())
                .collect(),
            target_contexts: vec!["all-targets".to_string()],
            active: true,
            gate_reason: None,
        }
    }

    #[test]
    fn version_ranges_are_bounded() {
        assert!(version_matches("1.50.0", ">=1,<2"));
        assert!(version_matches("0.8.4", ">=0.7,<0.9"));
        assert!(!version_matches("0.6.20", ">=0.7,<0.9"));
        assert!(!version_matches("2.0.0", ">=1,<2"));
    }

    #[test]
    fn packages_receive_independent_version_and_feature_gates() {
        let supported = vec![capability(Framework::Tokio, Some("1.50.0"), &["sync"])];
        let unsupported = vec![capability(Framework::Tokio, Some("2.0.0"), &["sync"])];
        let missing_feature = vec![capability(Framework::Tokio, Some("1.50.0"), &[])];

        assert!(
            rules_for_capabilities(&supported, false)
                .iter()
                .any(|rule| rule.name() == "tokio-unbounded-channel")
        );
        assert!(
            rules_for_capabilities(&unsupported, false)
                .iter()
                .all(|rule| rule.name() != "tokio-unbounded-channel")
        );
        assert!(
            rules_for_capabilities(&missing_feature, false)
                .iter()
                .all(|rule| rule.name() != "tokio-unbounded-channel")
        );
    }

    #[test]
    fn renamed_dependency_abstains_with_its_gate_reason() {
        let mut renamed = capability(Framework::Axum, Some("0.8.4"), &[]);
        renamed.active = false;
        renamed.gate_reason =
            Some("renamed dependency requires an explicit capability mapping".to_string());
        let rule = FrameworkPackRule::new(PackKind::AxumExtensionRequestState);

        assert_eq!(
            capability_decision(&rule, &[renamed]).unwrap_err(),
            "renamed dependency requires an explicit capability mapping"
        );
    }
}
