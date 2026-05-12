use reshape_cli::CliArgs;
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
        "--mock-llm",
    ]);

    assert_eq!(args.workspace.to_string_lossy(), "./page");
    assert_eq!(
        args.config.as_deref().unwrap().to_string_lossy(),
        "./reshape.toml"
    );
    assert_eq!(args.model.as_deref(), Some("test-model"));
    assert!(args.mock_llm);
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
