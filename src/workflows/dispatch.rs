use crate::cli::{
    AgentId as CliAgentId, AnalyzerFilter, CategoryMutationArgs, CiCommand, Command, HookBlocking,
    InstallArgs, RuleLevelArg, RuleListArgs, RulesCommand, ScanCategory, TagMutationArgs,
    TelemetryArgs, UninstallArgs, WhyArgs,
};
use crate::setup::{
    AgentId, BlockingLevel, SetupRequest, StagedHook, execute as execute_setup, render_report,
};
use crate::workflows::{ci, rules, telemetry, why};
use crate::{config, discovery};
use dialoguer::Confirm;
use dialoguer::theme::ColorfulTheme;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode};
use std::time::Duration;

pub fn handle(command: &Command) -> ExitCode {
    match command {
        Command::Install(arguments) => handle_install(arguments),
        Command::Uninstall(arguments) => handle_uninstall(arguments),
        Command::Rules(arguments) => handle_rules(&arguments.command),
        Command::Why(arguments) => handle_why(arguments),
        Command::Ci(arguments) => handle_ci(&arguments.command),
        Command::Telemetry(arguments) => handle_telemetry(arguments),
        Command::Version => {
            print_version_report();
            ExitCode::SUCCESS
        }
    }
}

fn handle_telemetry(arguments: &TelemetryArgs) -> ExitCode {
    match telemetry::handle(&arguments.command) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::from(crate::run::EXIT_SCAN_ERROR)
        }
    }
}

fn handle_ci(command: &CiCommand) -> ExitCode {
    let result = match command {
        CiCommand::Install(arguments) => ci::install(arguments),
        CiCommand::Config(arguments) => ci::configure(arguments),
        CiCommand::Upgrade(arguments) => ci::upgrade(arguments),
    };
    match result {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn handle_install(arguments: &InstallArgs) -> ExitCode {
    if !arguments.yes
        && !arguments.dry_run
        && arguments.directory == Path::new(".")
        && arguments.agent.is_empty()
        && !arguments.mcp
        && !arguments.no_skill
        && arguments.hook.is_none()
    {
        return match crate::setup::run_setup() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Error: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let result = setup_roots(&arguments.directory).and_then(|(home, project)| {
        let mut request = SetupRequest::install(home, project);
        request.dry_run = arguments.dry_run;
        request.yes = arguments.yes;
        request.agents = arguments.agent.iter().copied().map(agent_id).collect();
        request.skill = !arguments.no_skill;
        request.mcp = arguments.mcp;
        request.hook = arguments.hook.map(|_| StagedHook {
            blocking: blocking_level(arguments.blocking),
        });
        authorize_and_execute(request)
    });
    finish_setup(result)
}

fn handle_uninstall(arguments: &UninstallArgs) -> ExitCode {
    let result = setup_roots(&arguments.directory).and_then(|(home, project)| {
        let selected = arguments.skill || arguments.mcp || arguments.hook;
        let mut request = SetupRequest::uninstall(home, project);
        request.dry_run = arguments.dry_run;
        request.yes = arguments.yes;
        request.agents = arguments.agent.iter().copied().map(agent_id).collect();
        request.skill = !selected || arguments.skill;
        request.mcp = !selected || arguments.mcp;
        request.hook = (!selected || arguments.hook).then_some(StagedHook {
            blocking: BlockingLevel::Warning,
        });
        authorize_and_execute(request)
    });
    finish_setup(result)
}

fn authorize_and_execute(
    mut request: SetupRequest,
) -> Result<Option<crate::setup::SetupReport>, crate::error::SetupError> {
    if !request.dry_run && !request.yes {
        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("Apply rust-doctor {:?}?", request.action))
            .default(true)
            .interact()?;
        if !confirmed {
            return Ok(None);
        }
        request.interactive = true;
    }
    execute_setup(&request).map(Some)
}

fn finish_setup(
    result: Result<Option<crate::setup::SetupReport>, crate::error::SetupError>,
) -> ExitCode {
    match result {
        Ok(Some(report)) => {
            print!("{}", render_report(&report));
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!("Cancelled.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn setup_roots(directory: &Path) -> Result<(PathBuf, PathBuf), crate::error::SetupError> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| crate::error::SetupError::Install {
            path: None,
            message: "neither HOME nor USERPROFILE is set".to_string(),
        })?;
    let requested =
        directory
            .canonicalize()
            .map_err(|error| crate::error::SetupError::Install {
                path: Some(directory.to_path_buf()),
                message: format!("failed to resolve project root: {error}"),
            })?;
    let project = discovery::find_manifest_root(&requested).ok_or_else(|| {
        crate::error::SetupError::Install {
            path: Some(requested),
            message: "no Cargo.toml found at or above the requested directory".to_string(),
        }
    })?;
    Ok((home, project))
}

const fn agent_id(value: CliAgentId) -> AgentId {
    match value {
        CliAgentId::ClaudeCode => AgentId::Claude,
        CliAgentId::Cursor => AgentId::Cursor,
        CliAgentId::Codex => AgentId::Codex,
        CliAgentId::OpenCode => AgentId::OpenCode,
        CliAgentId::Windsurf => AgentId::Windsurf,
    }
}

const fn blocking_level(value: HookBlocking) -> BlockingLevel {
    match value {
        HookBlocking::Error => BlockingLevel::Error,
        HookBlocking::Warning => BlockingLevel::Warning,
        HookBlocking::None => BlockingLevel::None,
    }
}

fn handle_rules(command: &RulesCommand) -> ExitCode {
    let result = match command {
        RulesCommand::List(arguments) => list_rules(arguments),
        RulesCommand::Explain(arguments) => {
            load_rule_config(&arguments.directory).and_then(|(_, resolved)| {
                let explanation = rules::explain_rule(&resolved, &arguments.rule)
                    .map_err(|error| error.to_string())?;
                if arguments.json {
                    rules::render_rule_explanation_json(&explanation)
                        .map_err(|error| error.to_string())
                } else {
                    Ok(rules::render_rule_explanation(&explanation))
                }
            })
        }
        RulesCommand::Set(arguments) => mutate_rule(
            &arguments.directory,
            rules::RuleMutation::Set {
                rule: arguments.rule.clone(),
                level: rule_level(arguments.level).to_string(),
                threshold: arguments.threshold,
            },
            arguments.dry_run,
        ),
        RulesCommand::Enable(arguments) => mutate_rule(
            &arguments.directory,
            rules::RuleMutation::Enable {
                rule: arguments.rule.clone(),
            },
            arguments.dry_run,
        ),
        RulesCommand::Disable(arguments) => mutate_rule(
            &arguments.directory,
            rules::RuleMutation::Disable {
                rule: arguments.rule.clone(),
            },
            arguments.dry_run,
        ),
        RulesCommand::Category(arguments) => mutate_category(arguments),
        RulesCommand::IgnoreTag(arguments) => mutate_tag(arguments, true),
        RulesCommand::UnignoreTag(arguments) => mutate_tag(arguments, false),
    };
    match result {
        Ok(output) => {
            if output.ends_with('\n') {
                print!("{output}");
            } else {
                println!("{output}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::from(crate::run::EXIT_SCAN_ERROR)
        }
    }
}

fn list_rules(arguments: &RuleListArgs) -> Result<String, String> {
    let (_, resolved) = load_rule_config(&arguments.directory)?;
    let filter = rules::RuleListFilter {
        category: arguments.category.map(category_key).map(str::to_string),
        tag: arguments.tag.clone(),
        framework: arguments.framework.clone(),
        analyzer: arguments.analyzer.map(analyzer_key).map(str::to_string),
        configured_only: arguments.configured_only,
    };
    let entries = rules::list_rules(&resolved, &filter).map_err(|error| error.to_string())?;
    if arguments.json {
        rules::render_rule_list_json(&entries).map_err(|error| error.to_string())
    } else {
        Ok(rules::render_rule_list(&entries))
    }
}

fn load_rule_config(directory: &Path) -> Result<(PathBuf, config::ResolvedConfig), String> {
    let requested = directory.canonicalize().map_err(|error| {
        format!(
            "invalid project directory '{}': {error}",
            directory.display()
        )
    })?;
    if !requested.is_dir() {
        return Err(format!(
            "project path '{}' is not a directory",
            requested.display()
        ));
    }
    let root = discovery::find_manifest_root(&requested)
        .ok_or_else(|| format!("no Cargo.toml found at or above '{}'", requested.display()))?;
    let metadata = if root.join("rust-doctor.toml").is_file() {
        None
    } else {
        cargo_package_metadata(&root)?
    };
    let file_config =
        config::load_file_config(&root, metadata.as_ref()).map_err(|error| error.to_string())?;
    Ok((root, config::resolve_config_defaults(file_config.as_ref())))
}

fn cargo_package_metadata(root: &Path) -> Result<Option<serde_json::Value>, String> {
    let path = root.join("Cargo.toml");
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to read '{}': {error}", path.display())),
    };
    let manifest: toml::Value = toml::from_str(&content)
        .map_err(|error| format!("failed to parse '{}': {error}", path.display()))?;
    manifest
        .get("package")
        .and_then(|package| package.get("metadata"))
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| format!("failed to read metadata from '{}': {error}", path.display()))
}

fn mutate_category(arguments: &CategoryMutationArgs) -> Result<String, String> {
    mutate_rule(
        &arguments.directory,
        rules::RuleMutation::Category {
            category: category_key(arguments.category).to_string(),
            level: rule_level(arguments.level).to_string(),
        },
        arguments.dry_run,
    )
}

fn mutate_tag(arguments: &TagMutationArgs, ignore: bool) -> Result<String, String> {
    let mutation = if ignore {
        rules::RuleMutation::IgnoreTag {
            tag: arguments.tag.clone(),
        }
    } else {
        rules::RuleMutation::UnignoreTag {
            tag: arguments.tag.clone(),
        }
    };
    mutate_rule(&arguments.directory, mutation, arguments.dry_run)
}

fn mutate_rule(
    directory: &Path,
    mutation: rules::RuleMutation,
    dry_run: bool,
) -> Result<String, String> {
    let requested = directory.canonicalize().map_err(|error| {
        format!(
            "invalid project directory '{}': {error}",
            directory.display()
        )
    })?;
    let root = discovery::find_manifest_root(&requested)
        .ok_or_else(|| format!("no Cargo.toml found at or above '{}'", requested.display()))?;
    let result = rules::execute_rule_mutation(&root, mutation, dry_run)
        .map_err(|error| error.to_string())?;
    if dry_run {
        return Ok(result.diff);
    }
    if result.written {
        Ok(format!("Updated {}\n", result.path.display()))
    } else if result.changed {
        Ok(format!(
            "Prepared {} without writing\n",
            result.path.display()
        ))
    } else {
        Ok(format!("No change to {}\n", result.path.display()))
    }
}

fn handle_why(arguments: &WhyArgs) -> ExitCode {
    let request = why::WhyRequest {
        directory: arguments.directory.clone(),
        location: arguments.location.clone(),
        rule: arguments.rule.clone(),
        offline: arguments.offline,
        max_duration: arguments.max_duration.map(Duration::from_secs),
        no_project_config: arguments.no_project_config,
    };
    match why::execute(&request) {
        Ok(report) if arguments.json => match serde_json::to_string_pretty(&report) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("Error: failed to serialize why output: {error}");
                ExitCode::from(crate::run::EXIT_SCAN_ERROR)
            }
        },
        Ok(report) => {
            why::render_terminal(&report);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::from(crate::run::EXIT_SCAN_ERROR)
        }
    }
}

fn print_version_report() {
    let rustc = tool_output("rustc", &["-vV"]);
    let cargo = tool_output("cargo", &["-V"]);
    let target = rustc
        .as_deref()
        .and_then(|output| output.lines().find_map(|line| line.strip_prefix("host: ")))
        .map_or_else(
            || format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
            str::to_string,
        );
    println!("rust-doctor {}", env!("CARGO_PKG_VERSION"));
    println!(
        "{}",
        rustc
            .as_deref()
            .and_then(|output| output.lines().next())
            .unwrap_or("rustc unavailable")
    );
    println!("{}", cargo.as_deref().unwrap_or("cargo unavailable"));
    println!("target {target}");
    println!("os {} ({})", std::env::consts::OS, std::env::consts::ARCH);
}

fn tool_output(program: &str, arguments: &[&str]) -> Option<String> {
    ProcessCommand::new(program)
        .args(arguments)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_string())
}

const fn category_key(value: ScanCategory) -> &'static str {
    match value {
        ScanCategory::ErrorHandling => "error-handling",
        ScanCategory::Performance => "performance",
        ScanCategory::Security => "security",
        ScanCategory::Correctness => "correctness",
        ScanCategory::Architecture => "architecture",
        ScanCategory::Dependencies => "dependencies",
        ScanCategory::Async => "async",
        ScanCategory::Framework => "framework",
        ScanCategory::Cargo => "cargo",
        ScanCategory::Style => "style",
    }
}

const fn analyzer_key(value: AnalyzerFilter) -> &'static str {
    match value {
        AnalyzerFilter::SynAst => "syn-ast",
        AnalyzerFilter::Clippy => "clippy",
        AnalyzerFilter::Dependency => "dependency",
        AnalyzerFilter::Project => "project",
        AnalyzerFilter::External => "external",
    }
}

const fn rule_level(value: RuleLevelArg) -> &'static str {
    match value {
        RuleLevelArg::Off => "off",
        RuleLevelArg::Info => "info",
        RuleLevelArg::Warning => "warning",
        RuleLevelArg::Error => "error",
    }
}
