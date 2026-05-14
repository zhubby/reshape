use std::path::Path;

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
