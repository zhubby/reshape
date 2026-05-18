use std::path::Path;

use clap::CommandFactory;
use reshape_cli::{CliArgs, CliCommand};
use reshape_core::config::AppConfig;

#[test]
fn parses_workspace_and_optional_overrides() {
    let args = CliArgs::parse_from([
        "reshape",
        "--workspace",
        "./page",
        "--config",
        "./reshape.toml",
        "--model",
        "test-model",
        "--log-level",
        "debug",
        "--host",
        "127.0.0.1",
        "--port",
        "7332",
    ]);

    let agent = args.agent_options();
    assert_eq!(agent.workspace.as_deref(), Some(Path::new("./page")));
    assert_eq!(agent.config.as_deref(), Some(Path::new("./reshape.toml")));
    assert_eq!(agent.model.as_deref(), Some("test-model"));
    assert_eq!(agent.host.as_deref(), Some("127.0.0.1"));
    assert_eq!(agent.port, Some(7332));
    assert_eq!(args.log_level.as_deref(), Some("debug"));
    assert_eq!(args.command_kind(), CliCommand::Agent);
}

#[test]
fn parses_agent_subcommand_as_default_behavior() {
    let args = CliArgs::parse_from([
        "reshape",
        "agent",
        "--workspace",
        "./page",
        "--model",
        "test-model",
    ]);

    let agent = args.agent_options();
    assert_eq!(agent.workspace.as_deref(), Some(Path::new("./page")));
    assert_eq!(agent.model.as_deref(), Some("test-model"));
    assert_eq!(args.command_kind(), CliCommand::Agent);
}

#[test]
fn parses_version_subcommand() {
    let args = CliArgs::parse_from(["reshape", "version"]);

    assert_eq!(args.command_kind(), CliCommand::Version);
}

#[test]
fn cli_help_documents_options_and_subcommands() {
    let mut command = CliArgs::command();
    let mut help = Vec::new();
    command.write_long_help(&mut help).unwrap();
    let help = String::from_utf8(help).unwrap();

    for expected in [
        "Run the local agent runtime and render server",
        "Print the reshape CLI version",
        "Manage the local page workspace",
        "Path to the workspace directory the agent may read and modify",
        "Path to the TOML configuration file",
        "OpenAI chat model to use for agent responses",
        "Render completed agent output through agent-browser",
        "agent-browser session name used for rendering",
        "Run the render browser in headed mode",
        "Host address for the JSON-RPC render server",
        "Port for the JSON-RPC render server",
        "Tracing log level",
    ] {
        assert!(help.contains(expected), "missing help text: {expected}");
    }
}

#[test]
fn workspace_help_documents_nested_subcommands() {
    let mut command = CliArgs::command()
        .find_subcommand_mut("workspace")
        .unwrap()
        .clone();
    let mut help = Vec::new();
    command.write_long_help(&mut help).unwrap();
    let help = String::from_utf8(help).unwrap();

    for expected in [
        "Remove all generated files from the configured workspace",
        "Create the configured workspace and default page files",
    ] {
        assert!(help.contains(expected), "missing help text: {expected}");
    }
}

#[test]
fn default_config_uses_single_local_session() {
    let config = AppConfig::default();

    assert_eq!(config.session_key, "local:main");
    assert_eq!(config.runtime.max_tool_iterations, 8);
    assert_eq!(config.runtime.max_tool_calls, 32);
}

#[test]
fn validates_existing_workspace_directory() {
    let dir = tempfile::tempdir().unwrap();
    let config = AppConfig::for_workspace(dir.path()).unwrap();

    assert_eq!(config.workspace.root, dir.path());
}

#[test]
fn rejects_missing_workspace_directory() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");

    let error = AppConfig::for_workspace(&missing).unwrap_err();

    assert!(error.to_string().contains("workspace does not exist"));
}
