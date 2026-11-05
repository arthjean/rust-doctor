use std::path::Path;

use super::{MAX_FILE_BYTES, RepoFinding, RepoScan, path_context, read_bounded};
use crate::policy::REPO_SECRET_IN_BUILD_STEP;
use crate::source_text::SourceSpan;
use crate::workspace_path;

const COMPILING: [&str; 9] = ["bench", "build", "check", "clippy", "doc", "install", "publish", "run", "test"];

pub(super) fn is_workflow(path: &str) -> bool {
    path.strip_prefix(".github/workflows/")
        .is_some_and(|name| !name.contains('/') && (name.ends_with(".yml") || name.ends_with(".yaml")))
}

pub(super) fn scan_workflow(workspace_root: &Path, path: &str, scan: &mut RepoScan) {
    let Some(source) = read_bounded(&workspace_root.join(path), MAX_FILE_BYTES) else {
        return;
    };
    let Some(published) = workspace_path::normalize_changed(path) else {
        return;
    };
    for finding in compiling_steps_with_secrets(&source) {
        scan.findings.push(RepoFinding {
            definition: &REPO_SECRET_IN_BUILD_STEP,
            message: format!(
                "cargo {} compiles the workspace while the {} env holds the secret {}, which any build script can read",
                finding.subcommand, finding.scope, finding.variable
            ),
            path: published.clone(),
            span: Some(SourceSpan {
                line_start: finding.line,
                column_start: finding.column,
                line_end: finding.line,
                column_end: finding.end,
            }),
            context: path_context(path),
        });
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct BuildWithSecret {
    pub(super) subcommand: String,
    pub(super) scope: &'static str,
    pub(super) variable: String,
    pub(super) line: usize,
    pub(super) column: usize,
    pub(super) end: usize,
}

struct Entry<'a> {
    line: usize,
    column: usize,
    key: &'a str,
    value: &'a str,
    item: bool,
    block: Vec<(usize, &'a str)>,
}

fn entries(source: &str) -> Option<Vec<Entry<'_>>> {
    let lines: Vec<&str> = source.lines().collect();
    let mut entries: Vec<Entry<'_>> = Vec::new();
    let mut index = 0;
    while let Some(raw) = lines.get(index) {
        index += 1;
        let trimmed = raw.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = raw.len() - trimmed.len();
        let (item, column, text) = match trimmed.strip_prefix("- ") {
            Some(rest) => {
                let rest_trimmed = rest.trim_start();
                (true, indent + 2 + rest.len() - rest_trimmed.len(), rest_trimmed)
            }
            None => (false, indent, trimmed),
        };
        if anchored(text) {
            return None;
        }
        let Some((key, value)) = split_key(text) else {
            continue;
        };
        if key == "<<" || anchored(value) {
            return None;
        }
        let mut block = Vec::new();
        if value.starts_with('|') || value.starts_with('>') {
            while let Some(next) = lines.get(index) {
                let next_trimmed = next.trim_start();
                if !next_trimmed.is_empty() && next.len() - next_trimmed.len() <= column {
                    break;
                }
                block.push((index + 1, *next));
                index += 1;
            }
        }
        entries.push(Entry {
            line: index - block.len(),
            column,
            key,
            value,
            item,
            block,
        });
    }
    Some(entries)
}

fn split_key(text: &str) -> Option<(&str, &str)> {
    let (key, value) = match text.split_once(": ") {
        Some((key, value)) => (key, value.trim()),
        None => (text.strip_suffix(':')?, ""),
    };
    let key = key.trim().trim_matches(|quote| quote == '"' || quote == '\'');
    (!key.is_empty() && !key.contains(' ')).then_some((key, value))
}

fn anchored(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some('&' | '*'))
        && characters.next().is_some_and(|next| next.is_ascii_alphanumeric() || next == '_')
}

fn secret_variable(key: &str, value: &str) -> bool {
    value.contains("secrets.")
        || key == "CARGO_REGISTRY_TOKEN"
        || (key.starts_with("CARGO_REGISTRIES_") && key.ends_with("_TOKEN"))
}

#[derive(Default)]
struct Step {
    secret: Option<String>,
    runs: Vec<Run>,
}

struct Run {
    line: usize,
    column: usize,
    width: usize,
    command: String,
}

#[derive(Default)]
struct Job {
    secret: Option<String>,
    reusable: bool,
    steps: Vec<Step>,
}

pub(super) fn compiling_steps_with_secrets(source: &str) -> Vec<BuildWithSecret> {
    let Some(entries) = entries(source) else {
        return Vec::new();
    };
    let mut walk = Walk::default();
    let mut stack: Vec<(usize, &str)> = Vec::new();
    for entry in &entries {
        while stack.last().is_some_and(|(column, _)| *column >= entry.column) {
            stack.pop();
        }
        let path: Vec<&str> = stack.iter().map(|(_, key)| *key).collect();
        stack.push((entry.column, entry.key));
        walk.record(source, &path, entry);
    }
    walk.findings()
}

#[derive(Default)]
struct Walk {
    workflow_secret: Option<String>,
    jobs: Vec<Job>,
}

impl Walk {
    fn record(&mut self, source: &str, path: &[&str], entry: &Entry<'_>) {
        let secret = |key: &str| secret_variable(key, entry.value).then(|| key.to_owned());
        match path {
            [] if entry.key == "env" => remember(&mut self.workflow_secret, secret("env")),
            ["env"] => remember(&mut self.workflow_secret, secret(entry.key)),
            ["jobs"] => self.jobs.push(Job::default()),
            ["jobs", _] => {
                if let Some(job) = self.jobs.last_mut() {
                    job.reusable |= entry.key == "uses";
                    if entry.key == "env" {
                        remember(&mut job.secret, secret("env"));
                    }
                }
            }
            ["jobs", _, "env"] => {
                if let Some(job) = self.jobs.last_mut() {
                    remember(&mut job.secret, secret(entry.key));
                }
            }
            ["jobs", _, "steps"] => self.record_step(source, entry),
            ["jobs", _, "steps", "env"] => {
                if let Some(step) = self.jobs.last_mut().and_then(|job| job.steps.last_mut()) {
                    remember(&mut step.secret, secret(entry.key));
                }
            }
            _ => {}
        }
    }

    fn record_step(&mut self, source: &str, entry: &Entry<'_>) {
        let Some(job) = self.jobs.last_mut() else {
            return;
        };
        if entry.item || job.steps.is_empty() {
            job.steps.push(Step::default());
        }
        let Some(step) = job.steps.last_mut() else {
            return;
        };
        if entry.key == "env" && secret_variable("", entry.value) {
            remember(&mut step.secret, Some("env".to_owned()));
        }
        if entry.key != "run" {
            return;
        }
        if entry.block.is_empty() {
            step.runs.push(Run {
                line: entry.line,
                column: value_column(source, entry),
                width: entry.value.chars().count(),
                command: entry.value.to_owned(),
            });
        }
        let mut continued: Option<Run> = None;
        for (line, text) in &entry.block {
            let trimmed = text.trim_start();
            let mut run = match continued.take() {
                Some(mut run) => {
                    run.command.push(' ');
                    run.command.push_str(trimmed);
                    run
                }
                None => Run {
                    line: *line,
                    column: text.len() - trimmed.len() + 1,
                    width: trimmed.chars().count(),
                    command: trimmed.to_owned(),
                },
            };
            if run.command.ends_with('\\') {
                run.command.pop();
                continued = Some(run);
            } else {
                step.runs.push(run);
            }
        }
        step.runs.extend(continued);
    }

    fn findings(&self) -> Vec<BuildWithSecret> {
        let mut findings = Vec::new();
        for job in self.jobs.iter().filter(|job| !job.reusable) {
            for step in &job.steps {
                let Some((scope, variable)) = step
                    .secret
                    .as_ref()
                    .map(|variable| ("step", variable))
                    .or(job.secret.as_ref().map(|variable| ("job", variable)))
                    .or(self.workflow_secret.as_ref().map(|variable| ("workflow", variable)))
                else {
                    continue;
                };
                findings.extend(step.runs.iter().filter_map(|run| {
                    Some(BuildWithSecret {
                        subcommand: compiling_command(&run.command)?,
                        scope,
                        variable: variable.clone(),
                        line: run.line,
                        column: run.column,
                        end: run.column + run.width,
                    })
                }));
            }
        }
        findings
    }
}

fn remember(slot: &mut Option<String>, secret: Option<String>) {
    if slot.is_none() {
        *slot = secret;
    }
}

fn value_column(source: &str, entry: &Entry<'_>) -> usize {
    source
        .lines()
        .nth(entry.line - 1)
        .and_then(|line| line.rfind(entry.value))
        .map_or(entry.column + 1, |offset| offset + 1)
}

pub(super) fn compiling_command(line: &str) -> Option<String> {
    let comment = line
        .char_indices()
        .find(|(at, character)| {
            *character == '#' && line.get(..*at).is_some_and(|head| head.is_empty() || head.ends_with(char::is_whitespace))
        })
        .map_or(line.len(), |(at, _)| at);
    line.get(..comment)?.split([';', '&', '|', '(', ')']).find_map(|command| {
        let tokens: Vec<&str> = command
            .split(|character: char| character.is_whitespace() || matches!(character, '"' | '\'' | '`'))
            .filter(|token| !token.is_empty())
            .collect();
        let start = tokens.iter().position(|token| *token == "cargo" || token.ends_with("/cargo"))?;
        let subcommand = *tokens.get(start + 1..)?.iter().find(|token| !token.starts_with(['+', '-']))?;
        (COMPILING.contains(&subcommand) && !(subcommand == "publish" && tokens.contains(&"--no-verify")))
            .then(|| subcommand.to_owned())
    })
}

pub(super) fn holds_registry_token(source: &str) -> bool {
    let Ok(credentials) = toml::from_str::<toml::Table>(source) else {
        return false;
    };
    let holds_token = |table: &toml::Value| {
        table
            .get("token")
            .and_then(toml::Value::as_str)
            .is_some_and(|token| !token.is_empty())
    };
    credentials.get("registry").is_some_and(holds_token)
        || credentials
            .get("registries")
            .and_then(toml::Value::as_table)
            .is_some_and(|registries| registries.values().any(holds_token))
}

pub(super) fn is_cargo_credentials(path: &str) -> bool {
    let mut segments = path.rsplit('/');
    matches!(segments.next(), Some("credentials" | "credentials.toml")) && segments.next() == Some(".cargo")
}

#[cfg(test)]
mod tests;
