use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use cargo_metadata::{Metadata, Package};
use ra_ap_syntax::ast::{self, HasArgList, HasAttrs, HasName, LiteralKind};
use ra_ap_syntax::{AstNode, Edition, SourceFile, SyntaxNode, TextRange};

use crate::policy::{PolicyPlan, RuleDefinition, SOURCE_DISABLED_TLS, SOURCE_DYNAMIC_SHELL};

const FILE_BYTES_LIMIT: u64 = 8_388_608;
const TOTAL_BYTES_LIMIT: u64 = 268_435_456;
const UNIT_LIMIT: usize = 20_000;
const MODULE_DEPTH_LIMIT: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) definition: &'static RuleDefinition,
    pub(crate) message: &'static str,
    pub(crate) package: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) path: String,
    pub(crate) span: SourceSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SourceSpan {
    pub(crate) line_start: usize,
    pub(crate) column_start: usize,
    pub(crate) line_end: usize,
    pub(crate) column_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SourceError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

#[derive(Debug, Default)]
pub(crate) struct SourceScan {
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) errors: Vec<SourceError>,
    #[allow(dead_code)]
    pub(crate) counters: SourceCounters,
}

#[derive(Debug, Default)]
pub(crate) struct SourceCounters {
    pub(crate) files_read: usize,
    pub(crate) files_parsed: usize,
    pub(crate) bytes_read: u64,
    pub(crate) disabled_tls_predicates: usize,
    pub(crate) dynamic_shell_predicates: usize,
}

#[derive(Debug, Clone, Copy)]
struct Limits {
    file_bytes: u64,
    total_bytes: u64,
    units: usize,
    module_depth: usize,
}

const LIMITS: Limits = Limits {
    file_bytes: FILE_BYTES_LIMIT,
    total_bytes: TOTAL_BYTES_LIMIT,
    units: UNIT_LIMIT,
    module_depth: MODULE_DEPTH_LIMIT,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Identity {
    path: PathBuf,
    edition: Edition,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Reachability {
    package_id: String,
    package_name: String,
    target_key: String,
    target_name: String,
    reqwest_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct WorkItem {
    lexical_path: PathBuf,
    edition: Edition,
    module_directory: PathBuf,
    reachability: Reachability,
    depth: usize,
}

#[derive(Debug)]
struct SourceUnit {
    source: String,
    parse: ra_ap_syntax::Parse<SourceFile>,
    edition: Edition,
    error_ranges: Vec<TextRange>,
    relative_path: String,
    reachability: BTreeSet<Reachability>,
    traversals: BTreeSet<(Reachability, PathBuf)>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CandidateKey {
    code: &'static str,
    path: String,
    span: SourceSpan,
    message: &'static str,
}

pub(crate) fn inspect(metadata: &Metadata, plan: &PolicyPlan) -> SourceScan {
    inspect_with_limits_for_plan(metadata, LIMITS, plan)
}

fn inspect_with_limits_for_plan(
    metadata: &Metadata,
    limits: Limits,
    plan: &PolicyPlan,
) -> SourceScan {
    let disabled_tls = plan.is_active(SOURCE_DISABLED_TLS.id);
    let dynamic_shell = plan.is_active(SOURCE_DYNAMIC_SHELL.id);
    if !disabled_tls && !dynamic_shell {
        return SourceScan::default();
    }

    let workspace_root = match metadata.workspace_root.as_std_path().canonicalize() {
        Ok(root) => root,
        Err(_) => {
            return SourceScan {
                errors: vec![SourceError {
                    code: "read-failed",
                    message: "Workspace source root could not be resolved.".to_owned(),
                }],
                ..SourceScan::default()
            };
        }
    };
    let mut errors = Vec::new();
    let mut queue = source_roots(metadata, &mut errors);
    let mut units = BTreeMap::<Identity, SourceUnit>::new();
    let mut counters = SourceCounters::default();

    while let Some(work) = queue.pop_first() {
        if work.depth > limits.module_depth {
            push_limit_error(&mut errors, "module-depth", limits.module_depth);
            continue;
        }

        if !lexically_within_workspace(&workspace_root, &work.lexical_path) {
            let path = safe_lexical_path(&workspace_root, &work.lexical_path);
            push_error(
                &mut errors,
                "path-outside-workspace",
                format!("Source path \"{path}\" resolves outside the workspace."),
            );
            continue;
        }

        let canonical = match work.lexical_path.canonicalize() {
            Ok(path) => path,
            Err(_) => {
                let path = safe_lexical_path(&workspace_root, &work.lexical_path);
                push_error(
                    &mut errors,
                    "read-failed",
                    format!("Source path \"{path}\" could not be resolved."),
                );
                continue;
            }
        };
        if !canonical.starts_with(&workspace_root) {
            let path = safe_lexical_path(&workspace_root, &work.lexical_path);
            push_error(
                &mut errors,
                "path-outside-workspace",
                format!("Source path \"{path}\" resolves outside the workspace."),
            );
            continue;
        }

        let identity = Identity {
            path: canonical.clone(),
            edition: work.edition,
        };
        if !units.contains_key(&identity) {
            if units.len() >= limits.units {
                push_limit_error(&mut errors, "source-units", limits.units);
                break;
            }
            let error_count = errors.len();
            let loaded = load_unit(
                &identity,
                &workspace_root,
                &limits,
                &mut counters,
                &mut errors,
            );
            let Some(unit) = loaded else {
                if errors[error_count..].iter().any(|error| {
                    error.code == "limit-exceeded" && error.message.contains("total-bytes")
                }) {
                    break;
                }
                continue;
            };
            units.insert(identity.clone(), unit);
        }

        let traversal = (work.reachability.clone(), work.module_directory.clone());
        let Some(unit) = units.get_mut(&identity) else {
            continue;
        };
        unit.reachability.insert(work.reachability.clone());
        if !unit.traversals.insert(traversal) {
            continue;
        }

        let requests = module_requests(unit, &work, &workspace_root, &mut errors);
        queue.extend(requests);
    }

    let mut candidates = BTreeMap::<CandidateKey, Candidate>::new();
    for unit in units.values() {
        analyze_unit(
            unit,
            &mut candidates,
            disabled_tls,
            dynamic_shell,
            &mut counters,
        );
    }
    errors.sort();
    errors.dedup();

    SourceScan {
        candidates: candidates.into_values().collect(),
        errors,
        counters,
    }
}

fn source_roots(metadata: &Metadata, errors: &mut Vec<SourceError>) -> BTreeSet<WorkItem> {
    let mut roots = BTreeSet::new();
    for package in metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
    {
        let Some(edition) = syntax_edition(package.edition.to_string().as_str()) else {
            push_error(
                errors,
                "parse-error",
                format!(
                    "Package \"{}\" uses unsupported Rust edition \"{}\".",
                    package.name, package.edition
                ),
            );
            continue;
        };
        let reqwest_alias = reqwest_alias(package);
        for (target_index, target) in package.targets.iter().enumerate() {
            let path = target.src_path.as_std_path().to_path_buf();
            let module_directory = path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| path.clone());
            roots.insert(WorkItem {
                lexical_path: path,
                edition,
                module_directory,
                reachability: Reachability {
                    package_id: package.id.repr.clone(),
                    package_name: package.name.to_string(),
                    target_key: format!("{}:{target_index}", package.id.repr),
                    target_name: target.name.clone(),
                    reqwest_alias: reqwest_alias.clone(),
                },
                depth: 0,
            });
        }
    }
    roots
}

fn reqwest_alias(package: &Package) -> Option<String> {
    let aliases: BTreeSet<_> = package
        .dependencies
        .iter()
        .filter(|dependency| dependency.name == "reqwest")
        .map(|dependency| {
            dependency
                .rename
                .as_deref()
                .unwrap_or(&dependency.name)
                .replace('-', "_")
        })
        .collect();
    if aliases.len() == 1 {
        aliases.into_iter().next()
    } else {
        None
    }
}

fn syntax_edition(edition: &str) -> Option<Edition> {
    match edition {
        "2015" => Some(Edition::Edition2015),
        "2018" => Some(Edition::Edition2018),
        "2021" => Some(Edition::Edition2021),
        "2024" => Some(Edition::Edition2024),
        _ => None,
    }
}

fn load_unit(
    identity: &Identity,
    workspace_root: &Path,
    limits: &Limits,
    counters: &mut SourceCounters,
    errors: &mut Vec<SourceError>,
) -> Option<SourceUnit> {
    let relative_path = relative_path(workspace_root, &identity.path);
    let metadata = match fs::metadata(&identity.path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            push_error(
                errors,
                "read-failed",
                format!("Source path \"{relative_path}\" is not a regular file."),
            );
            return None;
        }
        Err(_) => {
            push_error(
                errors,
                "read-failed",
                format!("Source path \"{relative_path}\" could not be inspected."),
            );
            return None;
        }
    };
    if metadata.len() > limits.file_bytes {
        push_limit_error(errors, "file-bytes", limits.file_bytes);
        return None;
    }
    if counters.bytes_read.saturating_add(metadata.len()) > limits.total_bytes {
        push_limit_error(errors, "total-bytes", limits.total_bytes);
        return None;
    }

    let file = match File::open(&identity.path) {
        Ok(file) => file,
        Err(_) => {
            push_error(
                errors,
                "read-failed",
                format!("Source path \"{relative_path}\" could not be opened."),
            );
            return None;
        }
    };
    let remaining = limits.total_bytes.saturating_sub(counters.bytes_read);
    let read_limit = limits.file_bytes.min(remaining).saturating_add(1);
    let mut bytes = Vec::with_capacity(metadata.len().min(read_limit) as usize);
    if file.take(read_limit).read_to_end(&mut bytes).is_err() {
        push_error(
            errors,
            "read-failed",
            format!("Source path \"{relative_path}\" could not be read."),
        );
        return None;
    }
    if bytes.len() as u64 > limits.file_bytes {
        push_limit_error(errors, "file-bytes", limits.file_bytes);
        return None;
    }
    if counters.bytes_read.saturating_add(bytes.len() as u64) > limits.total_bytes {
        push_limit_error(errors, "total-bytes", limits.total_bytes);
        return None;
    }
    let source = match String::from_utf8(bytes) {
        Ok(source) => source,
        Err(_) => {
            push_error(
                errors,
                "read-failed",
                format!("Source path \"{relative_path}\" is not valid UTF-8."),
            );
            return None;
        }
    };
    counters.files_read += 1;
    counters.bytes_read += source.len() as u64;

    let parse = SourceFile::parse(&source, identity.edition);
    counters.files_parsed += 1;
    let parse_errors = parse.errors();
    if !parse_errors.is_empty() {
        push_error(
            errors,
            "parse-error",
            format!(
                "Source path \"{relative_path}\" contains {} parse errors.",
                parse_errors.len()
            ),
        );
    }

    Some(SourceUnit {
        source,
        parse,
        edition: identity.edition,
        error_ranges: parse_errors
            .into_iter()
            .map(|error| error.range())
            .collect(),
        relative_path,
        reachability: BTreeSet::new(),
        traversals: BTreeSet::new(),
    })
}

fn module_requests(
    unit: &SourceUnit,
    work: &WorkItem,
    workspace_root: &Path,
    errors: &mut Vec<SourceError>,
) -> Vec<WorkItem> {
    let tree = unit.parse.tree();
    let mut requests = Vec::new();
    for module in tree.syntax().descendants().filter_map(ast::Module::cast) {
        if module.item_list().is_some()
            || intersects_errors(module.syntax().text_range(), &unit.error_ranges)
        {
            continue;
        }
        let Some(name) = module.name().map(|name| name.text().to_string()) else {
            continue;
        };
        let mut base = work.module_directory.clone();
        let mut inline_ancestors: Vec<_> = module
            .syntax()
            .ancestors()
            .skip(1)
            .filter_map(ast::Module::cast)
            .filter(|ancestor| ancestor.item_list().is_some())
            .filter_map(|ancestor| ancestor.name().map(|name| name.text().to_string()))
            .collect();
        inline_ancestors.reverse();
        for ancestor in inline_ancestors {
            base.push(ancestor);
        }

        let candidates = match path_attribute(&module) {
            PathAttribute::Absent => vec![
                base.join(format!("{name}.rs")),
                base.join(&name).join("mod.rs"),
            ],
            PathAttribute::Literal(path) => vec![base.join(path)],
            PathAttribute::Invalid => {
                push_error(
                    errors,
                    "module-not-found",
                    format!(
                        "Module \"{name}\" declared in \"{}\" has no supported literal path.",
                        unit.relative_path
                    ),
                );
                continue;
            }
        };
        let mut confined = Vec::new();
        for path in candidates {
            if lexically_within_workspace(workspace_root, &path) {
                confined.push(path);
            } else {
                let path = safe_lexical_path(workspace_root, &path);
                push_error(
                    errors,
                    "path-outside-workspace",
                    format!("Source path \"{path}\" resolves outside the workspace."),
                );
            }
        }
        if confined.is_empty() {
            continue;
        }
        let existing: Vec<_> = confined
            .iter()
            .filter(|path| path_exists(path))
            .cloned()
            .collect();
        if existing.len() > 1 {
            push_error(
                errors,
                "module-ambiguous",
                format!(
                    "Module \"{name}\" declared in \"{}\" has both supported file forms.",
                    unit.relative_path
                ),
            );
            continue;
        }
        let Some(path) = existing.into_iter().next() else {
            push_error(
                errors,
                "module-not-found",
                format!(
                    "Module \"{name}\" declared in \"{}\" could not be resolved.",
                    unit.relative_path
                ),
            );
            continue;
        };
        let resolved_path = path.canonicalize().unwrap_or_else(|_| path.clone());
        let module_directory = module_directory_for_file(&resolved_path);
        requests.push(WorkItem {
            lexical_path: path,
            edition: work.edition,
            module_directory,
            reachability: work.reachability.clone(),
            depth: work.depth + 1,
        });
    }
    requests
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathAttribute {
    Absent,
    Literal(String),
    Invalid,
}

fn path_attribute(module: &ast::Module) -> PathAttribute {
    let mut paths = module.attrs().filter_map(|attribute| {
        let meta = attribute.meta()?;
        if meta.simple_name().as_deref() != Some("path") {
            return None;
        }
        Some(match meta {
            ast::Meta::KeyValueMeta(key_value) => key_value.expr().and_then(literal_string),
            _ => None,
        })
    });
    match (paths.next(), paths.next()) {
        (None, _) => PathAttribute::Absent,
        (Some(Some(path)), None) => PathAttribute::Literal(path),
        _ => PathAttribute::Invalid,
    }
}

fn path_exists(path: &Path) -> bool {
    match path.symlink_metadata() {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

fn module_directory_for_file(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(path);
    if path.file_name().and_then(|name| name.to_str()) == Some("mod.rs") {
        parent.to_path_buf()
    } else {
        path.file_stem()
            .map(|stem| parent.join(stem))
            .unwrap_or_else(|| parent.to_path_buf())
    }
}

fn analyze_unit(
    unit: &SourceUnit,
    candidates: &mut BTreeMap<CandidateKey, Candidate>,
    disabled_tls: bool,
    dynamic_shell: bool,
    counters: &mut SourceCounters,
) {
    let tree = unit.parse.tree();
    let package = unique_package(&unit.reachability);
    let target = unique_target(&unit.reachability);
    let line_starts = line_starts(&unit.source);

    if disabled_tls
        && !path_contains_tests_segment(&unit.relative_path)
        && let Some(alias) = shared_reqwest_alias(&unit.reachability)
    {
        for call in tree
            .syntax()
            .descendants()
            .filter_map(ast::MethodCallExpr::cast)
        {
            counters.disabled_tls_predicates += 1;
            let Some((message, range)) = tls_match(&call, &alias, &unit.error_ranges) else {
                continue;
            };
            if excluded_test_context(&call) || alias_shadowed(&call, &alias) {
                continue;
            }
            insert_candidate(
                candidates,
                SOURCE_DISABLED_TLS,
                message,
                package.clone(),
                target.clone(),
                &unit.relative_path,
                range,
                &line_starts,
                &unit.source,
            );
        }
    }

    if dynamic_shell {
        for call in tree
            .syntax()
            .descendants()
            .filter_map(ast::MethodCallExpr::cast)
        {
            counters.dynamic_shell_predicates += 1;
            let Some(range) = shell_match(&call, &unit.error_ranges, unit.edition) else {
                continue;
            };
            insert_candidate(
                candidates,
                SOURCE_DYNAMIC_SHELL,
                "A dynamic value is interpolated into a shell command string.",
                package.clone(),
                target.clone(),
                &unit.relative_path,
                range,
                &line_starts,
                &unit.source,
            );
        }
    }
}

fn tls_match(
    call: &ast::MethodCallExpr,
    alias: &str,
    error_ranges: &[TextRange],
) -> Option<(&'static str, TextRange)> {
    let method = call.name_ref()?.text().to_string();
    let message = match method.as_str() {
        "tls_danger_accept_invalid_certs" | "danger_accept_invalid_certs" => {
            "Reqwest client builder disables TLS certificate verification."
        }
        "tls_danger_accept_invalid_hostnames" | "danger_accept_invalid_hostnames" => {
            "Reqwest client builder disables TLS hostname verification."
        }
        _ => return None,
    };
    let argument = only_argument(call.arg_list()?)?;
    if !literal_bool(&argument, true) {
        return None;
    }

    let mut receiver = call.receiver()?;
    while let ast::Expr::MethodCallExpr(method_call) = receiver {
        receiver = method_call.receiver()?;
    }
    let ast::Expr::CallExpr(builder) = receiver else {
        return None;
    };
    if builder.arg_list()?.args().next().is_some() {
        return None;
    }
    let callee = builder.expr()?;
    let path = compact(callee.syntax());
    let async_builder = format!("{alias}::Client::builder");
    let blocking_builder = format!("{alias}::blocking::Client::builder");
    if path != async_builder && path != blocking_builder {
        return None;
    }
    if intersects_errors(call.syntax().text_range(), error_ranges) {
        return None;
    }
    Some((message, argument.syntax().text_range()))
}

fn shell_match(
    call: &ast::MethodCallExpr,
    error_ranges: &[TextRange],
    edition: Edition,
) -> Option<TextRange> {
    if call.name_ref()?.text() != "arg" {
        return None;
    }
    let payload = only_argument(call.arg_list()?)?;
    if !dynamic_payload(&payload, edition) {
        return None;
    }
    let ast::Expr::MethodCallExpr(shell_arg) = call.receiver()? else {
        return None;
    };
    if shell_arg.name_ref()?.text() != "arg"
        || only_argument(shell_arg.arg_list()?)
            .and_then(literal_string)
            .as_deref()
            != Some("-c")
    {
        return None;
    }
    let ast::Expr::CallExpr(command) = shell_arg.receiver()? else {
        return None;
    };
    if compact(command.expr()?.syntax()) != "std::process::Command::new" {
        return None;
    }
    let shell = only_argument(command.arg_list()?).and_then(literal_string)?;
    if !matches!(shell.as_str(), "sh" | "bash" | "dash" | "zsh") {
        return None;
    }
    if intersects_errors(call.syntax().text_range(), error_ranges) {
        return None;
    }
    Some(payload.syntax().text_range())
}

fn only_argument(arguments: ast::ArgList) -> Option<ast::Expr> {
    let mut arguments = arguments.args();
    let argument = arguments.next()?;
    if arguments.next().is_some() {
        None
    } else {
        Some(argument)
    }
}

fn literal_bool(expression: &ast::Expr, expected: bool) -> bool {
    matches!(
        expression,
        ast::Expr::Literal(literal) if literal.kind() == LiteralKind::Bool(expected)
    )
}

fn literal_string(expression: ast::Expr) -> Option<String> {
    let ast::Expr::Literal(literal) = expression else {
        return None;
    };
    let LiteralKind::String(string) = literal.kind() else {
        return None;
    };
    string.value().ok().map(|value| value.into_owned())
}

fn strip_wrappers(mut expression: ast::Expr) -> ast::Expr {
    loop {
        expression = match expression {
            ast::Expr::ParenExpr(paren) => match paren.expr() {
                Some(inner) => inner,
                None => return ast::Expr::ParenExpr(paren),
            },
            ast::Expr::RefExpr(reference) => match reference.expr() {
                Some(inner) => inner,
                None => return ast::Expr::RefExpr(reference),
            },
            expression => return expression,
        };
    }
}

fn dynamic_payload(expression: &ast::Expr, edition: Edition) -> bool {
    let expression = strip_wrappers(expression.clone());
    match expression {
        ast::Expr::BinExpr(binary) => {
            binary.op_kind() == Some(ast::BinaryOp::ArithOp(ast::ArithOp::Add))
                && [binary.lhs(), binary.rhs()]
                    .into_iter()
                    .flatten()
                    .any(|operand| concat_operand_dynamic(&operand))
        }
        ast::Expr::MacroExpr(macro_expression) => macro_expression
            .macro_call()
            .is_some_and(|call| format_macro_dynamic(&call, edition)),
        _ => false,
    }
}

fn concat_operand_dynamic(expression: &ast::Expr) -> bool {
    let expression = strip_wrappers(expression.clone());
    match expression {
        ast::Expr::BinExpr(binary)
            if binary.op_kind() == Some(ast::BinaryOp::ArithOp(ast::ArithOp::Add)) =>
        {
            [binary.lhs(), binary.rhs()]
                .into_iter()
                .flatten()
                .any(|operand| concat_operand_dynamic(&operand))
        }
        ast::Expr::Literal(_) => false,
        _ => true,
    }
}

fn format_macro_dynamic(call: &ast::MacroCall, edition: Edition) -> bool {
    let Some(path) = call.path() else {
        return false;
    };
    if compact(path.syntax()) != "format" {
        return false;
    }
    let Some(token_tree) = call.token_tree() else {
        return false;
    };
    let arguments = token_tree_arguments(token_tree.syntax());
    let Some((format_literal, values)) = arguments.split_first() else {
        return false;
    };
    let Some(format_expression) = parsed_expression(format_literal, edition) else {
        return false;
    };
    let Some(format_text) = literal_string(format_expression) else {
        return false;
    };
    let fields = format_fields(&format_text);
    if fields.is_empty() {
        return false;
    }

    let mut positional = Vec::new();
    let mut named = BTreeMap::new();
    for value in values {
        if let Some((name, expression)) = named_format_argument(value) {
            named.insert(name, expression_is_literal(expression, edition));
        } else {
            positional.push(expression_is_literal(value, edition));
        }
    }
    let mut next_position = 0;
    fields.into_iter().any(|field| {
        let field = field.split(':').next().unwrap_or_default();
        if field.is_empty() {
            let literal = positional.get(next_position).copied().unwrap_or(true);
            next_position += 1;
            !literal
        } else if let Ok(index) = field.parse::<usize>() {
            !positional.get(index).copied().unwrap_or(true)
        } else {
            !named.get(field).copied().unwrap_or(false)
        }
    })
}

fn token_tree_arguments(tree: &SyntaxNode) -> Vec<String> {
    let mut arguments = vec![String::new()];
    for element in tree.children_with_tokens() {
        if element
            .as_token()
            .is_some_and(|token| matches!(token.text(), "(" | ")" | "[" | "]" | "{" | "}"))
        {
            continue;
        }
        if element.as_token().is_some_and(|token| token.text() == ",") {
            arguments.push(String::new());
            continue;
        }
        if element.kind().is_trivia() {
            continue;
        }
        if let Some(argument) = arguments.last_mut() {
            argument.push_str(&element.to_string());
        }
    }
    arguments
        .into_iter()
        .filter(|argument| !argument.is_empty())
        .collect()
}

fn named_format_argument(argument: &str) -> Option<(&str, &str)> {
    let (name, expression) = argument.split_once('=')?;
    if !name.is_empty()
        && !expression.starts_with('=')
        && name
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric())
    {
        Some((name, expression))
    } else {
        None
    }
}

fn expression_is_literal(expression: &str, edition: Edition) -> bool {
    parsed_expression(expression, edition)
        .is_some_and(|expression| matches!(strip_wrappers(expression), ast::Expr::Literal(_)))
}

fn parsed_expression(expression: &str, edition: Edition) -> Option<ast::Expr> {
    let parse = ast::Expr::parse(expression, edition);
    if parse.errors().is_empty() {
        ast::Expr::cast(parse.syntax_node())
    } else {
        None
    }
}

fn format_fields(format: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut characters = format.char_indices().peekable();
    while let Some((_, character)) = characters.next() {
        if character != '{' {
            continue;
        }
        if characters.peek().is_some_and(|(_, next)| *next == '{') {
            characters.next();
            continue;
        }
        let mut field = String::new();
        for (_, character) in characters.by_ref() {
            if character == '}' {
                fields.push(field);
                break;
            }
            field.push(character);
        }
    }
    fields
}

fn excluded_test_context(call: &ast::MethodCallExpr) -> bool {
    call.syntax().ancestors().any(|ancestor| {
        let has_cfg_test = ancestor
            .children()
            .filter_map(ast::Attr::cast)
            .any(|attribute| compact(attribute.syntax()) == "#[cfg(test)]");
        let is_test_function = ast::Fn::cast(ancestor.clone()).is_some_and(|function| {
            function
                .attrs()
                .any(|attribute| compact(attribute.syntax()) == "#[test]")
        });
        has_cfg_test || is_test_function
    })
}

fn alias_shadowed(call: &ast::MethodCallExpr, alias: &str) -> bool {
    let call_ancestors: Vec<_> = call.syntax().ancestors().collect();
    let visible_scopes = visible_binding_scopes(&call_ancestors);
    let root = call_ancestors
        .iter()
        .find(|node| ast::SourceFile::cast((*node).clone()).is_some())
        .cloned()
        .unwrap_or_else(|| call.syntax().clone());

    if call.syntax().ancestors().any(|ancestor| {
        ancestor
            .children()
            .filter_map(ast::GenericParamList::cast)
            .flat_map(|parameters| parameters.generic_params())
            .any(|parameter| {
                matches!(
                    parameter,
                    ast::GenericParam::TypeParam(parameter)
                        if parameter.name().is_some_and(|name| name.text() == alias)
                )
            })
    }) {
        return true;
    }

    root.descendants().any(|node| {
        let visible = binding_scope(&node)
            .as_ref()
            .is_some_and(|scope| visible_scopes.contains(scope));
        if !visible {
            return false;
        }
        if let Some(module) = ast::Module::cast(node.clone())
            && module.name().is_some_and(|name| name.text() == alias)
        {
            return true;
        }
        if let Some(item) = ast::Use::cast(node.clone()) {
            return item
                .use_tree()
                .is_none_or(|tree| use_tree_may_bind_alias(&tree, None, alias));
        }
        if let Some(item) = ast::ExternCrate::cast(node.clone()) {
            return match item.rename() {
                Some(rename) => rename.name().is_some_and(|name| name.text() == alias),
                None => item.name_ref().is_some_and(|name| name.text() == alias),
            };
        }
        if let Some(name) = type_item_name(&node) {
            return name == alias;
        }
        false
    })
}

fn use_tree_may_bind_alias(tree: &ast::UseTree, parent_name: Option<&str>, alias: &str) -> bool {
    if tree.star_token().is_some() {
        return true;
    }
    if let Some(rename) = tree.rename() {
        return rename.name().is_some_and(|name| name.text() == alias);
    }

    let path_name = tree.path().and_then(|path| {
        path.segment()
            .and_then(|segment| segment.name_ref())
            .map(|name| name.text().to_string())
    });
    let bound_name = match path_name.as_deref() {
        Some("self") => parent_name,
        Some(name) => Some(name),
        None => parent_name,
    };

    if let Some(list) = tree.use_tree_list() {
        return list
            .use_trees()
            .any(|child| use_tree_may_bind_alias(&child, bound_name, alias));
    }
    bound_name == Some(alias)
}

fn type_item_name(node: &SyntaxNode) -> Option<String> {
    if let Some(item) = ast::Struct::cast(node.clone()) {
        return item.name().map(|name| name.text().to_string());
    }
    if let Some(item) = ast::Enum::cast(node.clone()) {
        return item.name().map(|name| name.text().to_string());
    }
    if let Some(item) = ast::Union::cast(node.clone()) {
        return item.name().map(|name| name.text().to_string());
    }
    if let Some(item) = ast::Trait::cast(node.clone()) {
        return item.name().map(|name| name.text().to_string());
    }
    ast::TypeAlias::cast(node.clone())
        .and_then(|item| item.name().map(|name| name.text().to_string()))
}

fn binding_scope(node: &SyntaxNode) -> Option<SyntaxNode> {
    node.parent()?.ancestors().find(|ancestor| {
        ast::SourceFile::cast(ancestor.clone()).is_some()
            || ast::Module::cast(ancestor.clone())
                .is_some_and(|module| module.item_list().is_some())
            || ast::BlockExpr::cast(ancestor.clone()).is_some()
    })
}

fn visible_binding_scopes(call_ancestors: &[SyntaxNode]) -> Vec<SyntaxNode> {
    let mut scopes = Vec::new();
    for ancestor in call_ancestors {
        if ast::BlockExpr::cast(ancestor.clone()).is_some() {
            scopes.push(ancestor.clone());
            continue;
        }
        if ast::SourceFile::cast(ancestor.clone()).is_some()
            || ast::Module::cast(ancestor.clone())
                .is_some_and(|module| module.item_list().is_some())
        {
            scopes.push(ancestor.clone());
            break;
        }
    }
    scopes
}

#[allow(clippy::too_many_arguments)]
fn insert_candidate(
    candidates: &mut BTreeMap<CandidateKey, Candidate>,
    definition: &'static RuleDefinition,
    message: &'static str,
    package: Option<String>,
    target: Option<String>,
    path: &str,
    range: TextRange,
    line_starts: &[usize],
    source: &str,
) {
    if range.is_empty() {
        return;
    }
    let span = source_span(range, line_starts, source);
    let key = CandidateKey {
        code: definition.id,
        path: path.to_owned(),
        span,
        message,
    };
    let candidate = Candidate {
        definition,
        message,
        package,
        target,
        path: path.to_owned(),
        span,
    };
    match candidates.get_mut(&key) {
        Some(existing) => {
            merge_context(&mut existing.package, candidate.package);
            merge_context(&mut existing.target, candidate.target);
        }
        None => {
            candidates.insert(key, candidate);
        }
    }
}

fn merge_context(existing: &mut Option<String>, incoming: Option<String>) {
    if existing.as_ref() != incoming.as_ref() {
        *existing = None;
    }
}

fn unique_package(reachability: &BTreeSet<Reachability>) -> Option<String> {
    let packages: BTreeSet<_> = reachability
        .iter()
        .map(|reach| (&reach.package_id, &reach.package_name))
        .collect();
    if packages.len() == 1 {
        packages.into_iter().next().map(|(_, name)| name.clone())
    } else {
        None
    }
}

fn unique_target(reachability: &BTreeSet<Reachability>) -> Option<String> {
    let targets: BTreeSet<_> = reachability
        .iter()
        .map(|reach| (&reach.target_key, &reach.target_name))
        .collect();
    if targets.len() == 1 {
        targets.into_iter().next().map(|(_, name)| name.clone())
    } else {
        None
    }
}

fn shared_reqwest_alias(reachability: &BTreeSet<Reachability>) -> Option<String> {
    let aliases: BTreeSet<_> = reachability
        .iter()
        .map(|reach| reach.reqwest_alias.as_deref())
        .collect();
    if aliases.len() == 1 {
        aliases.into_iter().next().flatten().map(str::to_owned)
    } else {
        None
    }
}

fn intersects_errors(range: TextRange, errors: &[TextRange]) -> bool {
    let start = u32::from(range.start());
    let end = u32::from(range.end());
    errors.iter().any(|error| {
        let error_start = u32::from(error.start());
        let error_end = u32::from(error.end());
        if error_start == error_end {
            error_start >= start && error_start <= end
        } else {
            error_start < end && error_end > start
        }
    })
}

fn compact(node: &SyntaxNode) -> String {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .map(|token| token.text().to_string())
        .collect()
}

fn line_starts(source: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(
            source
                .bytes()
                .enumerate()
                .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
        )
        .collect()
}

fn source_span(range: TextRange, line_starts: &[usize], source: &str) -> SourceSpan {
    let (line_start, column_start) = source_position(range.start().into(), line_starts, source);
    let (line_end, column_end) = source_position(range.end().into(), line_starts, source);
    SourceSpan {
        line_start,
        column_start,
        line_end,
        column_end,
    }
}

fn source_position(offset: usize, line_starts: &[usize], source: &str) -> (usize, usize) {
    let bounded = offset.min(source.len());
    let line_index = line_starts.partition_point(|start| *start <= bounded) - 1;
    let column = source[line_starts[line_index]..bounded].chars().count() + 1;
    (line_index + 1, column)
}

fn path_contains_tests_segment(path: &str) -> bool {
    Path::new(path)
        .components()
        .any(|component| component == Component::Normal("tests".as_ref()))
}

fn relative_path(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .filter(|relative| !relative.is_empty())
        .unwrap_or_else(|| ".".to_owned())
}

fn safe_lexical_path(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .ok()
        .filter(|relative| {
            relative
                .components()
                .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
        })
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .filter(|relative| !relative.is_empty())
        .or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "<source>".to_owned())
}

fn lexically_within_workspace(workspace_root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(workspace_root) else {
        return false;
    };
    let mut depth = 0_usize;
    for component in relative.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

fn push_limit_error(errors: &mut Vec<SourceError>, name: &'static str, maximum: impl ToString) {
    push_error(
        errors,
        "limit-exceeded",
        format!(
            "Source limit \"{name}\" exceeded (maximum {}).",
            maximum.to_string()
        ),
    );
}

fn push_error(errors: &mut Vec<SourceError>, code: &'static str, message: String) {
    errors.push(SourceError { code, message });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{PolicyInput, Producer, RuleLevel};
    use cargo_metadata::MetadataCommand;

    fn inspect(metadata: &Metadata) -> SourceScan {
        super::inspect(metadata, &PolicyPlan::default())
    }

    fn inspect_with_limits(metadata: &Metadata, limits: Limits) -> SourceScan {
        inspect_with_limits_for_plan(metadata, limits, &PolicyPlan::default())
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/source-kernel")
            .join(name)
    }

    fn metadata(name: &str) -> Metadata {
        let manifest = fixture(name).join("Cargo.toml");
        let mut command = MetadataCommand::new();
        command
            .manifest_path(manifest)
            .no_deps()
            .other_options(["--offline".to_owned(), "--locked".to_owned()]);
        command.exec().unwrap()
    }

    #[test]
    fn pinned_parser_supports_all_target_editions_and_recoverable_errors() {
        let valid = "fn main() { let value = 1; }";
        for edition in [
            Edition::Edition2015,
            Edition::Edition2018,
            Edition::Edition2021,
            Edition::Edition2024,
        ] {
            let parse = SourceFile::parse(valid, edition);
            assert!(parse.errors().is_empty());
            assert_eq!(
                usize::from(parse.tree().syntax().text_range().len()),
                valid.len()
            );
        }

        let recoverable = "fn valid() {}\nfn broken( {\nfn after() {}";
        let parse = SourceFile::parse(recoverable, Edition::Edition2024);
        let errors = parse.errors();
        assert!(!errors.is_empty());
        assert_eq!(
            usize::from(parse.tree().syntax().text_range().len()),
            recoverable.len()
        );
        assert!(errors.iter().all(|error| {
            usize::from(error.range().start()) <= recoverable.len()
                && usize::from(error.range().end()) <= recoverable.len()
        }));
    }

    #[test]
    fn producer_uses_the_two_canonical_catalog_entries() {
        let definitions: Vec<_> = PolicyPlan::default()
            .active_rules(Producer::SourceKernel)
            .map(|(definition, _)| definition.id)
            .collect();
        assert_eq!(
            definitions,
            [SOURCE_DISABLED_TLS.id, SOURCE_DYNAMIC_SHELL.id]
        );
    }

    #[test]
    fn policy_prunes_source_io_and_each_inactive_predicate() {
        let metadata = metadata("precision");
        let all_off = PolicyInput::default()
            .with_rule(SOURCE_DISABLED_TLS.id, RuleLevel::Off)
            .with_rule(SOURCE_DYNAMIC_SHELL.id, RuleLevel::Off);
        let all_off = PolicyPlan::compile(&all_off).expect("policy should compile");
        let scan = super::inspect(&metadata, &all_off);
        assert!(scan.candidates.is_empty());
        assert!(scan.errors.is_empty());
        assert_eq!(scan.counters.files_read, 0);
        assert_eq!(scan.counters.files_parsed, 0);
        assert_eq!(scan.counters.bytes_read, 0);
        assert_eq!(scan.counters.disabled_tls_predicates, 0);
        assert_eq!(scan.counters.dynamic_shell_predicates, 0);

        let shell_off = PolicyInput::default().with_rule(SOURCE_DYNAMIC_SHELL.id, RuleLevel::Off);
        let shell_off = PolicyPlan::compile(&shell_off).expect("policy should compile");
        let scan = super::inspect(&metadata, &shell_off);
        assert!(scan.counters.files_read > 0);
        assert!(scan.counters.disabled_tls_predicates > 0);
        assert_eq!(scan.counters.dynamic_shell_predicates, 0);
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.definition.id != SOURCE_DYNAMIC_SHELL.id)
        );

        let tls_off = PolicyInput::default().with_rule(SOURCE_DISABLED_TLS.id, RuleLevel::Off);
        let tls_off = PolicyPlan::compile(&tls_off).expect("policy should compile");
        let scan = super::inspect(&metadata, &tls_off);
        assert_eq!(scan.counters.disabled_tls_predicates, 0);
        assert!(scan.counters.dynamic_shell_predicates > 0);
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.definition.id != SOURCE_DISABLED_TLS.id)
        );
    }

    #[test]
    fn reqwest_shadowing_respects_ast_binding_scopes_and_use_trees() {
        fn shadowed(source: &str) -> bool {
            let parse = SourceFile::parse(source, Edition::Edition2024);
            assert!(parse.errors().is_empty(), "{:?}", parse.errors());
            let call = parse
                .tree()
                .syntax()
                .descendants()
                .filter_map(ast::MethodCallExpr::cast)
                .find(|call| {
                    call.name_ref()
                        .is_some_and(|name| name.text() == "tls_danger_accept_invalid_certs")
                })
                .unwrap();
            alias_shadowed(&call, "http_client")
        }

        assert!(!shadowed(
            "fn target() { http_client::Client::builder().tls_danger_accept_invalid_certs(true); } fn sibling() { use local as http_client; }",
        ));
        assert!(shadowed(
            "fn target() { use local as http_client; http_client::Client::builder().tls_danger_accept_invalid_certs(true); }",
        ));
        assert!(!shadowed(
            "use local as http_client; mod child { fn target() { http_client::Client::builder().tls_danger_accept_invalid_certs(true); } }",
        ));
        assert!(shadowed(
            "fn target() { use local::http_client::{self}; http_client::Client::builder().tls_danger_accept_invalid_certs(true); }",
        ));
        assert!(shadowed(
            "fn target() { use local::*; http_client::Client::builder().tls_danger_accept_invalid_certs(true); }",
        ));
    }

    #[test]
    fn corpus_follows_modules_once_and_emits_only_closed_predicates() {
        let scan = inspect(&metadata("precision"));
        assert!(scan.errors.is_empty(), "{:?}", scan.errors);
        assert_eq!(scan.counters.files_read, 13);
        assert_eq!(scan.counters.files_parsed, 13);
        assert_eq!(scan.candidates.len(), 10, "{:?}", scan.candidates);
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.package.as_deref() == Some("source-kernel-app"))
        );
        assert!(scan.candidates.iter().any(|candidate| {
            candidate.definition.id == SOURCE_DYNAMIC_SHELL.id
                && candidate.path == "app/src/shared.rs"
                && candidate.target.is_none()
        }));
        assert_eq!(
            scan.candidates
                .iter()
                .filter(|candidate| candidate.definition.id == SOURCE_DISABLED_TLS.id)
                .count(),
            5
        );
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.path != "app/src/ignored.rs")
        );
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| !candidate.path.contains("/tests/"))
        );
    }

    #[test]
    fn partial_failures_are_private_deduplicated_and_preserve_valid_findings() {
        let scan = inspect(&metadata("errors"));
        let codes: Vec<_> = scan.errors.iter().map(|error| error.code).collect();
        assert_eq!(
            codes,
            [
                "module-ambiguous",
                "module-not-found",
                "parse-error",
                "parse-error",
                "path-outside-workspace",
                "path-outside-workspace",
            ]
        );
        assert_eq!(scan.candidates.len(), 2, "{:?}", scan.candidates);
        assert_eq!(scan.counters.files_parsed, 5);
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.definition.id == SOURCE_DYNAMIC_SHELL.id)
        );
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.path != "src/intersected.rs")
        );
        let rendered = format!("{:?}", scan.errors);
        assert!(!rendered.contains(env!("CARGO_MANIFEST_DIR")));
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("fn invalid"));
    }

    #[test]
    fn existing_and_missing_roots_outside_the_workspace_are_never_read() {
        let existing = inspect(&metadata("external-root"));
        assert_eq!(existing.counters.files_read, 0);
        assert!(existing.candidates.is_empty());
        assert_eq!(existing.errors.len(), 1);
        assert_eq!(existing.errors[0].code, "path-outside-workspace");
        assert!(
            !existing.errors[0]
                .message
                .contains(env!("CARGO_MANIFEST_DIR"))
        );

        let mut missing_metadata = metadata("external-root");
        let missing = missing_metadata
            .workspace_root
            .join("../missing-external-main.rs");
        missing_metadata.packages[0].targets[0].src_path = missing;
        let missing = inspect(&missing_metadata);
        assert_eq!(missing.counters.files_read, 0);
        assert!(missing.candidates.is_empty());
        assert_eq!(missing.errors.len(), 1);
        assert_eq!(missing.errors[0].code, "path-outside-workspace");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_outside_the_workspace_are_rejected_before_reading() {
        use std::os::unix::fs::symlink;

        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("source-kernel-symlink-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"symlink-proof\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(root.join("src/lib.rs"), "mod escape;\n").unwrap();
        symlink(fixture("outside.rs"), root.join("src/escape.rs")).unwrap();

        let mut command = MetadataCommand::new();
        command
            .manifest_path(root.join("Cargo.toml"))
            .no_deps()
            .other_options(["--offline".to_owned()]);
        let scan = inspect(&command.exec().unwrap());

        assert_eq!(scan.counters.files_read, 1);
        assert_eq!(scan.counters.files_parsed, 1);
        assert_eq!(scan.errors.len(), 1);
        assert_eq!(scan.errors[0].code, "path-outside-workspace");
        assert!(!scan.errors[0].message.contains(env!("CARGO_MANIFEST_DIR")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_utf8_is_a_private_read_failure() {
        let mut metadata = metadata("precision");
        let path = metadata.workspace_root.join(format!(
            "target/source-kernel-invalid-{}.rs",
            std::process::id()
        ));
        let package = metadata
            .packages
            .iter_mut()
            .find(|package| package.name.as_str() == "source-kernel-app")
            .unwrap();
        let mut target = package.targets[0].clone();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, [0xff, 0xfe]).unwrap();
        target.src_path = path.clone();
        package.targets = vec![target];

        let scan = inspect(&metadata);
        fs::remove_file(path).unwrap();

        assert_eq!(scan.counters.files_read, 1);
        assert_eq!(scan.counters.files_parsed, 1);
        assert_eq!(scan.errors.len(), 1);
        assert_eq!(scan.errors[0].code, "read-failed");
        assert!(!scan.errors[0].message.contains(env!("CARGO_MANIFEST_DIR")));
    }

    #[test]
    fn target_editions_do_not_override_the_package_edition() {
        let mut metadata = metadata("precision");
        let package = metadata
            .packages
            .iter_mut()
            .find(|package| package.name.as_str() == "source-kernel-app")
            .unwrap();
        let mut edition_2018 = package.targets[0].clone();
        edition_2018.name = "edition-2018".to_owned();
        edition_2018.edition = cargo_metadata::Edition::E2018;
        let mut edition_2024 = edition_2018.clone();
        edition_2024.name = "edition-2024".to_owned();
        edition_2024.edition = cargo_metadata::Edition::E2024;
        package.targets = vec![edition_2018, edition_2024];

        let scan = inspect(&metadata);

        assert!(scan.errors.is_empty(), "{:?}", scan.errors);
        assert_eq!(scan.counters.files_read, 11);
        assert_eq!(scan.counters.files_parsed, 11);
        assert_eq!(scan.candidates.len(), 10);
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.target.is_none())
        );
    }

    #[test]
    fn unsupported_package_editions_fail_closed() {
        let mut metadata = metadata("precision");
        let package = metadata
            .packages
            .iter_mut()
            .find(|package| package.name.as_str() == "source-kernel-app")
            .unwrap();
        package.edition = cargo_metadata::Edition::_E2027;

        let scan = inspect(&metadata);

        assert!(scan.candidates.is_empty());
        assert_eq!(scan.errors.len(), 1);
        assert_eq!(scan.errors[0].code, "parse-error");
        assert!(scan.errors[0].message.contains("unsupported Rust edition"));
    }

    #[test]
    fn file_total_unit_and_depth_limits_stop_the_required_work() {
        let metadata = metadata("precision");
        for (limits, name) in [
            (
                Limits {
                    file_bytes: 1,
                    ..LIMITS
                },
                "file-bytes",
            ),
            (
                Limits {
                    total_bytes: 1,
                    ..LIMITS
                },
                "total-bytes",
            ),
            (Limits { units: 0, ..LIMITS }, "source-units"),
            (
                Limits {
                    module_depth: 0,
                    ..LIMITS
                },
                "module-depth",
            ),
        ] {
            let scan = inspect_with_limits(&metadata, limits);
            assert_eq!(
                scan.errors
                    .iter()
                    .filter(|error| {
                        error.code == "limit-exceeded" && error.message.contains(name)
                    })
                    .count(),
                1
            );
        }
    }

    #[test]
    fn unicode_columns_are_scalar_based_and_end_exclusive() {
        let source = "fn main() { let _ = \"é\"; true }";
        let start = source.find("true").unwrap();
        let end = start + "true".len();
        let span = source_span(
            TextRange::new((start as u32).into(), (end as u32).into()),
            &line_starts(source),
            source,
        );
        assert_eq!(span.line_start, 1);
        assert_eq!(span.line_end, 1);
        assert_eq!(span.column_start, source[..start].chars().count() + 1);
        assert_eq!(span.column_end, source[..end].chars().count() + 1);
    }

    #[test]
    fn twenty_metadata_reachability_permutations_are_identical() {
        let mut metadata = metadata("precision");
        let package_index = metadata
            .packages
            .iter()
            .position(|package| package.name.as_str() == "source-kernel-app")
            .unwrap();
        let root = metadata.packages[package_index].targets[0].clone();
        let targets: Vec<_> = (0..5)
            .map(|index| {
                let mut target = root.clone();
                target.name = format!("target-{index}");
                target
            })
            .collect();
        let mut order = [0, 1, 2, 3, 4];
        let mut expected = None;
        for _ in 0..20 {
            metadata.packages[package_index].targets =
                order.iter().map(|index| targets[*index].clone()).collect();
            let scan = inspect(&metadata);
            let observation = format!(
                "{:?}|{:?}|{:?}",
                scan.candidates, scan.errors, scan.counters
            );
            match expected.as_ref() {
                Some(expected) => assert_eq!(&observation, expected),
                None => expected = Some(observation),
            }
            assert!(next_permutation(&mut order));
        }
    }

    #[test]
    #[ignore = "manual probe for the five explicitly approved pinned repositories"]
    fn pinned_real_world_evaluation_probe() {
        let manifest = std::env::var_os("RUST_DOCTOR_EVALUATION_MANIFEST")
            .map(PathBuf::from)
            .expect("RUST_DOCTOR_EVALUATION_MANIFEST must name an approved manifest");
        let mut command = MetadataCommand::new();
        command
            .manifest_path(manifest)
            .no_deps()
            .other_options(["--offline".to_owned()]);
        let scan = inspect(&command.exec().expect("approved metadata should load"));
        let mut counts = BTreeMap::from([
            (SOURCE_DISABLED_TLS.id, 0_usize),
            (SOURCE_DYNAMIC_SHELL.id, 0_usize),
        ]);
        for candidate in &scan.candidates {
            *counts.entry(candidate.definition.id).or_default() += 1;
        }
        let mut errors = BTreeMap::<&str, usize>::new();
        for error in &scan.errors {
            *errors.entry(error.code).or_default() += 1;
        }
        let findings: Vec<_> = scan
            .candidates
            .iter()
            .map(|candidate| {
                serde_json::json!({
                    "code": candidate.definition.id,
                    "package": candidate.package,
                    "target": candidate.target,
                    "path": candidate.path,
                    "span": {
                        "line_start": candidate.span.line_start,
                        "column_start": candidate.span.column_start,
                        "line_end": candidate.span.line_end,
                        "column_end": candidate.span.column_end,
                    }
                })
            })
            .collect();
        println!(
            "RUST_DOCTOR_SOURCE_EVALUATION={}",
            serde_json::json!({
                "source_pairs": scan.counters.files_parsed,
                "files_read": scan.counters.files_read,
                "bytes_parsed": scan.counters.bytes_read,
                "counts": counts,
                "source_errors": errors,
                "findings": findings,
            })
        );
    }

    fn next_permutation(values: &mut [usize]) -> bool {
        let Some(pivot) = (0..values.len() - 1)
            .rev()
            .find(|index| values[*index] < values[*index + 1])
        else {
            return false;
        };
        let successor = (pivot + 1..values.len())
            .rev()
            .find(|index| values[*index] > values[pivot])
            .unwrap();
        values.swap(pivot, successor);
        values[pivot + 1..].reverse();
        true
    }
}
