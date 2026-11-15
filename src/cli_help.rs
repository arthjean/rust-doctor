use clap::CommandFactory;

use super::Cli;

fn undescribed(command: &clap::Command, path: &str, missing: &mut Vec<String>) {
    for argument in command.get_arguments() {
        let id = argument.get_id().as_str();
        if matches!(id, "help" | "version") {
            continue;
        }
        let described = argument
            .get_help()
            .or_else(|| argument.get_long_help())
            .is_some_and(|help| !help.to_string().trim().is_empty());
        if !described {
            missing.push(format!("{path} {id}"));
        }
    }
    for subcommand in command.get_subcommands() {
        undescribed(subcommand, &format!("{path} {}", subcommand.get_name()), missing);
    }
}

pub(crate) fn missing_help(command: &clap::Command) -> Vec<String> {
    let mut missing = Vec::new();
    undescribed(command, command.get_name(), &mut missing);
    missing
}

#[test]
fn every_argument_of_every_subcommand_carries_a_description() {
    let missing = missing_help(&Cli::command());
    assert!(missing.is_empty(), "arguments without help: {missing:#?}");
}

#[test]
fn an_argument_added_without_help_fails_the_walk() {
    let command = Cli::command().subcommand(
        clap::Command::new("probe").arg(clap::Arg::new("undescribed").long("undescribed")),
    );
    assert!(missing_help(&command).contains(&"rust-doctor probe undescribed".to_owned()));
}
