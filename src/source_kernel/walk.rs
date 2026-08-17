//! The workspace walk: which files the kernel is allowed to read, and reading
//! them once.
//!
//! Every target Cargo names is a root, and `mod` declarations are followed from
//! there. A file is loaded at most once per edition, and the reachability of
//! each unit accumulates as more targets arrive at it, which is what lets a
//! finding name its package only when every reacher agrees on one.
//!
//! Nothing outside the workspace is ever read. Containment is decided twice,
//! lexically before `canonicalize` is called at all, so a `..` climb is refused
//! without touching the filesystem, and again on the resolved path, which is
//! what catches a symbolic link pointing out of the tree.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use cargo_metadata::Metadata;
use ra_ap_syntax::ast::{self, HasAttrs, HasName};
use ra_ap_syntax::{AstNode, Edition, SourceFile};

use crate::report::DiagnosticContext;
use crate::source_text::intersects_errors;

use super::detectors::{self, CrateAliases};
use super::{
    Enumeration, Identity, LIMITS, Limit, Limits, Reachability, SourceError, SourceUnit,
    WalkCounters, literal_string, push_error, push_limit_error,
};

/// One file to visit, and the module context it is visited in. Two targets
/// reaching the same file produce two items: the file is parsed once, but each
/// traversal contributes its own reachability.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct WorkItem {
    lexical_path: PathBuf,
    edition: Edition,
    module_directory: PathBuf,
    reachability: Reachability,
    depth: usize,
}

/// What loading one unit produced.
///
/// The three outcomes drive three different decisions, and that is the whole
/// reason this is a type: `Skipped` leaves the walk running, `BudgetExhausted`
/// ends it, because the byte budget is global and no later file can fit in what
/// is left.
enum Loaded {
    Unit(SourceUnit),
    Skipped,
    BudgetExhausted,
}

pub(crate) fn enumerate(metadata: &Metadata) -> Enumeration {
    enumerate_with_limits(metadata, LIMITS)
}

pub(super) fn enumerate_with_limits(metadata: &Metadata, limits: Limits) -> Enumeration {
    let Ok(workspace_root) = metadata.workspace_root.as_std_path().canonicalize() else {
        return Enumeration {
            errors: vec![SourceError {
                code: "read-failed",
                message: "Workspace source root could not be resolved.".to_owned(),
            }],
            ..Enumeration::default()
        };
    };
    let mut errors = Vec::new();
    let mut queue = source_roots(metadata, &mut errors);
    let mut units = BTreeMap::<Identity, SourceUnit>::new();
    let mut counters = WalkCounters::default();

    while let Some(work) = queue.pop_first() {
        if work.depth > limits.module_depth {
            push_limit_error(&mut errors, Limit::ModuleDepth, limits.module_depth);
            continue;
        }
        let canonical = match confine(&workspace_root, &work.lexical_path) {
            Ok(canonical) => canonical,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };

        // Read before the entry borrows the map, which is the only reason this
        // is not asked inside the arm that needs it.
        let occupancy = units.len();
        let unit = match units.entry(Identity {
            path: canonical,
            edition: work.edition,
        }) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                if occupancy >= limits.units {
                    push_limit_error(&mut errors, Limit::SourceUnits, limits.units);
                    break;
                }
                let loaded = load_unit(
                    entry.key(),
                    &workspace_root,
                    limits,
                    &mut counters,
                    &mut errors,
                );
                match loaded {
                    Loaded::Unit(unit) => entry.insert(unit),
                    Loaded::Skipped => continue,
                    Loaded::BudgetExhausted => break,
                }
            }
        };

        unit.reachability.insert(work.reachability.clone());
        if !unit
            .traversals
            .insert((work.reachability.clone(), work.module_directory.clone()))
        {
            continue;
        }
        queue.extend(module_requests(unit, &work, &workspace_root, &mut errors));
    }

    Enumeration {
        units,
        tables: crate_alias_tables(metadata),
        contexts: target_contexts(metadata),
        errors,
        counters,
    }
}

fn source_roots(metadata: &Metadata, errors: &mut Vec<SourceError>) -> BTreeSet<WorkItem> {
    let mut roots = BTreeSet::new();
    for package in workspace_packages(metadata) {
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
                    target_key: target_key(&package.id.repr, target_index),
                    target_name: target.name.clone(),
                },
                depth: 0,
            });
        }
    }
    roots
}

fn workspace_packages(metadata: &Metadata) -> impl Iterator<Item = &cargo_metadata::Package> {
    metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
}

/// How `Reachability` names a target, and how `target_contexts` keys it. One
/// spelling, so the two tables cannot key the same target differently.
fn target_key(package_id: &str, target_index: usize) -> String {
    format!("{package_id}:{target_index}")
}

fn crate_alias_tables(metadata: &Metadata) -> BTreeMap<String, CrateAliases> {
    workspace_packages(metadata)
        .map(|package| (package.id.repr.clone(), detectors::crate_aliases(package)))
        .collect()
}

/// Non-production mark of every workspace target, keyed the way `Reachability`
/// names it. The kind comes from Cargo, so a producer never guesses a context
/// from a path.
fn target_contexts(metadata: &Metadata) -> BTreeMap<String, Option<DiagnosticContext>> {
    workspace_packages(metadata)
        .flat_map(|package| {
            package
                .targets
                .iter()
                .enumerate()
                .map(move |(target_index, target)| {
                    let kinds: Vec<String> = target.kind.iter().map(ToString::to_string).collect();
                    (
                        target_key(&package.id.repr, target_index),
                        DiagnosticContext::from_target_kinds(&kinds),
                    )
                })
        })
        .collect()
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
    limits: Limits,
    counters: &mut WalkCounters,
    errors: &mut Vec<SourceError>,
) -> Loaded {
    let relative_path = relative_path(workspace_root, &identity.path);
    // One shape for every reason a file cannot become a unit, so the path is
    // sanitized in one place rather than at each refusal.
    let refused = |reason: &str| SourceError {
        code: "read-failed",
        message: format!("Source path \"{relative_path}\" {reason}."),
    };

    let metadata = match fs::metadata(&identity.path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            errors.push(refused("is not a regular file"));
            return Loaded::Skipped;
        }
        Err(_) => {
            errors.push(refused("could not be inspected"));
            return Loaded::Skipped;
        }
    };
    if metadata.len() > limits.file_bytes {
        push_limit_error(errors, Limit::FileBytes, limits.file_bytes);
        return Loaded::Skipped;
    }
    if counters.bytes_read.saturating_add(metadata.len()) > limits.total_bytes {
        push_limit_error(errors, Limit::TotalBytes, limits.total_bytes);
        return Loaded::BudgetExhausted;
    }

    let Ok(file) = File::open(&identity.path) else {
        errors.push(refused("could not be opened"));
        return Loaded::Skipped;
    };
    // One byte past the largest read that could still be admitted, so a file
    // that overruns a limit is detected instead of being silently truncated.
    let remaining = limits.total_bytes.saturating_sub(counters.bytes_read);
    let read_limit = limits.file_bytes.min(remaining).saturating_add(1);
    let mut bytes = Vec::with_capacity(metadata.len().min(read_limit) as usize);
    if file.take(read_limit).read_to_end(&mut bytes).is_err() {
        errors.push(refused("could not be read"));
        return Loaded::Skipped;
    }
    // The budget charges what left the disk, not what survived decoding. A file
    // that is not UTF-8 was still read, and a walk that did not charge it could
    // read the per-file limit over and over with the total never moving.
    counters.bytes_read = counters.bytes_read.saturating_add(bytes.len() as u64);
    if bytes.len() as u64 > limits.file_bytes {
        push_limit_error(errors, Limit::FileBytes, limits.file_bytes);
        return Loaded::Skipped;
    }
    if counters.bytes_read > limits.total_bytes {
        push_limit_error(errors, Limit::TotalBytes, limits.total_bytes);
        return Loaded::BudgetExhausted;
    }
    let Ok(source) = String::from_utf8(bytes) else {
        errors.push(refused("is not valid UTF-8"));
        return Loaded::Skipped;
    };
    counters.files_read += 1;

    let parse = SourceFile::parse(&source, identity.edition);
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

    Loaded::Unit(SourceUnit {
        source,
        edition: identity.edition,
        error_ranges: parse_errors.iter().map(|error| error.range()).collect(),
        parse,
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
    let tree = unit.tree();
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
        let unresolved = |reason: &str| SourceError {
            code: "module-not-found",
            message: format!(
                "Module \"{name}\" declared in \"{}\" {reason}.",
                unit.relative_path()
            ),
        };

        let base = inline_module_directory(&work.module_directory, &module);
        let candidates = match path_attribute(&module) {
            PathAttribute::Absent => vec![
                base.join(format!("{name}.rs")),
                base.join(&name).join("mod.rs"),
            ],
            PathAttribute::Literal(path) => vec![base.join(path)],
            PathAttribute::Invalid => {
                errors.push(unresolved("has no supported literal path"));
                continue;
            }
        };
        let (confined, escaping): (Vec<PathBuf>, Vec<PathBuf>) = candidates
            .into_iter()
            .partition(|path| lexically_within_workspace(workspace_root, path));
        for path in &escaping {
            errors.push(outside_workspace(workspace_root, path));
        }
        if confined.is_empty() {
            continue;
        }

        let mut existing = confined.into_iter().filter(|path| path_exists(path));
        let path = match (existing.next(), existing.next()) {
            (Some(path), None) => path,
            (Some(_), Some(_)) => {
                push_error(
                    errors,
                    "module-ambiguous",
                    format!(
                        "Module \"{name}\" declared in \"{}\" has both supported file forms.",
                        unit.relative_path()
                    ),
                );
                continue;
            }
            (None, _) => {
                errors.push(unresolved("could not be resolved"));
                continue;
            }
        };

        let resolved = path.canonicalize().unwrap_or_else(|_| path.clone());
        requests.push(WorkItem {
            lexical_path: path,
            edition: work.edition,
            module_directory: module_directory_for_file(&resolved),
            reachability: work.reachability.clone(),
            depth: work.depth + 1,
        });
    }
    requests
}

/// Directory a `mod` declaration resolves against, once the inline `mod` blocks
/// it is nested in have been walked back down from the file's own directory.
fn inline_module_directory(module_directory: &Path, module: &ast::Module) -> PathBuf {
    let mut inline: Vec<String> = module
        .syntax()
        .ancestors()
        .skip(1)
        .filter_map(ast::Module::cast)
        .filter(|ancestor| ancestor.item_list().is_some())
        .filter_map(|ancestor| ancestor.name().map(|name| name.text().to_string()))
        .collect();
    inline.reverse();
    let mut base = module_directory.to_path_buf();
    base.extend(inline);
    base
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

/// Does something occupy this path? A path the process cannot stat counts as
/// occupied: with two candidate forms for one `mod`, an unreadable `foo.rs`
/// beside a readable `foo/mod.rs` is reported ambiguous rather than silently
/// resolved to the one form that happened to be visible.
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

/// The canonical path of a file the walk may read, or the error refusing it.
fn confine(workspace_root: &Path, path: &Path) -> Result<PathBuf, SourceError> {
    if !lexically_within_workspace(workspace_root, path) {
        return Err(outside_workspace(workspace_root, path));
    }
    let Ok(canonical) = path.canonicalize() else {
        return Err(SourceError {
            code: "read-failed",
            message: format!(
                "Source path \"{}\" could not be resolved.",
                safe_lexical_path(workspace_root, path)
            ),
        });
    };
    if !canonical.starts_with(workspace_root) {
        return Err(outside_workspace(workspace_root, path));
    }
    Ok(canonical)
}

fn outside_workspace(workspace_root: &Path, path: &Path) -> SourceError {
    SourceError {
        code: "path-outside-workspace",
        message: format!(
            "Source path \"{}\" resolves outside the workspace.",
            safe_lexical_path(workspace_root, path)
        ),
    }
}

/// Workspace-relative path of a file the kernel did not enumerate, or nothing
/// when it resolves outside the workspace.
///
/// It canonicalizes before comparing, because the caller's root is canonical
/// while a path assembled from a manifest or read from a directory is not: where
/// the workspace sits under a symbolic link, a lexical comparison of the two
/// answers no for every file of it. The structural pass relativizes through this
/// and nothing else, so the invariant every report depends on, that no absolute
/// path ever reaches the wire, has one implementation to be right in.
pub(crate) fn workspace_relative(workspace_root: &Path, path: &Path) -> Option<String> {
    let canonical = path.canonicalize().ok()?;
    display_path(canonical.strip_prefix(workspace_root).ok()?)
}

/// Path a finding publishes, for a file the walk already proved contained.
fn relative_path(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .ok()
        .and_then(display_path)
        .unwrap_or_else(|| ".".to_owned())
}

/// Path an error message may name. A path that escapes the workspace is
/// reduced to its file name, and an unnameable one to a placeholder: an error
/// is published, and no absolute path or directory outside the tree belongs in
/// what gets published.
fn safe_lexical_path(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .ok()
        .filter(|relative| {
            relative
                .components()
                .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
        })
        .and_then(display_path)
        .or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "<source>".to_owned())
}

/// Wire spelling of a relative path: forward slashes on every platform, and
/// nothing at all for the empty path.
fn display_path(relative: &Path) -> Option<String> {
    let relative = relative.to_string_lossy().replace('\\', "/");
    (!relative.is_empty()).then_some(relative)
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/workspace")
    }

    #[test]
    fn a_parent_climb_is_refused_before_the_filesystem_is_touched() {
        assert!(lexically_within_workspace(
            &root(),
            Path::new("/workspace/src/./lib.rs")
        ));
        assert!(lexically_within_workspace(
            &root(),
            Path::new("/workspace/src/../src/lib.rs")
        ));
        assert!(!lexically_within_workspace(
            &root(),
            Path::new("/workspace/../outside.rs")
        ));
        assert!(!lexically_within_workspace(&root(), Path::new("/elsewhere")));
    }

    /// An error message names a workspace-relative path, a bare file name, or
    /// nothing. It never names a directory the reader did not scan.
    #[test]
    fn a_published_path_never_escapes_the_workspace() {
        assert_eq!(
            safe_lexical_path(&root(), Path::new("/workspace/src/lib.rs")),
            "src/lib.rs"
        );
        assert_eq!(
            safe_lexical_path(&root(), Path::new("/somewhere/private/secret.rs")),
            "secret.rs"
        );
        assert_eq!(
            safe_lexical_path(&root(), Path::new("/workspace/../secret.rs")),
            "secret.rs"
        );
        // The root reduces to its own directory name, which the reader already
        // knows, and a path carrying no file name at all reaches the
        // placeholder rather than publishing anything.
        assert_eq!(safe_lexical_path(&root(), &root()), "workspace");
        assert_eq!(safe_lexical_path(&root(), Path::new("/")), "<source>");
    }

    #[test]
    fn the_module_directory_descends_into_a_named_directory_only_for_a_file_module() {
        assert_eq!(
            module_directory_for_file(Path::new("/workspace/src/nested.rs")),
            PathBuf::from("/workspace/src/nested")
        );
        assert_eq!(
            module_directory_for_file(Path::new("/workspace/src/nested/mod.rs")),
            PathBuf::from("/workspace/src/nested")
        );
    }

    /// Two targets of the same package never share a key, and the two tables
    /// keyed by target agree on the spelling.
    #[test]
    fn every_target_of_a_package_gets_its_own_key() {
        let keys: HashSet<String> = (0..4).map(|index| target_key("alpha", index)).collect();
        assert_eq!(keys.len(), 4);
        assert_eq!(target_key("alpha", 2), "alpha:2");
    }
}
