use std::path::Path;
use std::sync::{Arc, Mutex};

use reshape_browser::{BrowserRenderError, BrowserRenderer};
use reshape_cli::{CliArgs, build_runtime};
use reshape_core::bus::InProcessBus;
use reshape_core::ingress::IngressSource;
use reshape_core::ingress::cli_stdin::CliStdinIngress;
use reshape_core::llm::mock::MockLlmProvider;
use reshape_core::llm::{LlmResponse, ToolCall};
use reshape_core::protocol::{Envelope, InputEvent, InputSource, OutputEvent};

#[test]
fn cli_reports_missing_workspace_during_config_build() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    let args = CliArgs::parse_from(["reshape", "--workspace", missing.to_str().unwrap()]);

    let error = args.into_config().unwrap_err();

    assert!(error.to_string().contains("workspace does not exist"));
}

#[test]
fn cli_default_workspace_is_created_under_reshape_home() {
    let home = tempfile::tempdir().unwrap();
    let args = CliArgs::parse_from(["reshape"]);

    let config = args.into_config_with_home(home.path()).unwrap();

    let workspace = home.path().join(".reshape").join("workspace");
    assert_eq!(config.workspace.root, workspace);
    assert!(config.workspace.root.is_dir());
}

#[test]
fn cli_default_config_path_is_under_reshape_home() {
    let home = tempfile::tempdir().unwrap();
    let args = CliArgs::parse_from(["reshape"]);

    assert_eq!(
        args.resolved_config_path_with_home(home.path()),
        home.path().join(".reshape").join("config.toml")
    );
}

#[test]
fn cli_startup_initializes_reshape_directory_and_default_config() {
    let home = tempfile::tempdir().unwrap();
    let args = CliArgs::parse_from(["reshape", "version"]);

    args.prepare_user_environment_with_home(home.path())
        .unwrap();

    let reshape_dir = home.path().join(".reshape");
    let workspace = reshape_dir.join("workspace");
    let config_path = reshape_dir.join("config.toml");
    let config = std::fs::read_to_string(&config_path).unwrap();

    assert!(reshape_dir.is_dir());
    assert!(workspace.is_dir());
    assert!(config.contains(&format!("workspace = \"{}\"", workspace.to_string_lossy())));
    assert!(config.contains("mock_llm = true"));
    assert!(config.contains("log_level = \"info\""));
    assert!(config.contains("[runtime]"));
    assert!(config.contains("max_tool_iterations = 8"));
    assert!(config.contains("max_tool_calls = 32"));
}

#[test]
fn cli_loads_toml_config_and_cli_overrides_it() {
    let home = tempfile::tempdir().unwrap();
    let config_workspace = home.path().join("configured-workspace");
    let cli_workspace = home.path().join("cli-workspace");
    std::fs::create_dir_all(&config_workspace).unwrap();
    std::fs::create_dir_all(&cli_workspace).unwrap();
    let config_path = home.path().join(".reshape").join("config.toml");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(
        &config_path,
        format!(
            r#"
workspace = "{}"
model = "configured-model"
mock_llm = false

[runtime]
max_tool_iterations = 3
max_tool_calls = 9
"#,
            config_workspace.to_string_lossy()
        ),
    )
    .unwrap();

    let args = CliArgs::parse_from([
        "reshape",
        "--workspace",
        cli_workspace.to_str().unwrap(),
        "--model",
        "cli-model",
    ]);
    let config = args.into_config_with_home(home.path()).unwrap();

    assert_eq!(config.workspace.root, cli_workspace);
    assert_eq!(config.llm.model.as_deref(), Some("cli-model"));
    assert!(!config.llm.use_mock);
    assert_eq!(config.runtime.max_tool_iterations, 3);
    assert_eq!(config.runtime.max_tool_calls, 9);
}

#[test]
fn cli_config_defaults_to_usable_mock_provider() {
    let dir = tempfile::tempdir().unwrap();
    let args = CliArgs::parse_from(["reshape", "--workspace", dir.path().to_str().unwrap()]);

    let config = args.into_config().unwrap();

    assert!(config.llm.use_mock);
}

#[tokio::test]
async fn cli_runtime_builder_can_create_page_with_mock_provider() {
    let dir = tempfile::tempdir().unwrap();
    let config = CliArgs::parse_from(["reshape", "--workspace", dir.path().to_str().unwrap()])
        .into_config()
        .unwrap();
    let provider = Arc::new(MockLlmProvider::new([
        LlmResponse {
            content: "Writing".to_string(),
            tool_calls: vec![ToolCall {
                id: "write".to_string(),
                name: "write_file".to_string(),
                arguments: serde_json::json!({
                    "path": "index.html",
                    "content": "<h1>CLI</h1>"
                }),
            }],
        },
        LlmResponse {
            content: "Done".to_string(),
            tool_calls: vec![ToolCall {
                id: "complete".to_string(),
                name: "complete_task".to_string(),
                arguments: serde_json::json!({"summary": "CLI page created"}),
            }],
        },
    ]));
    let runtime = build_runtime(config, provider).unwrap();

    let output = runtime
        .process(Envelope::new(InputEvent::UserText {
            text: "create page".to_string(),
            source: InputSource::Cli,
        }))
        .await
        .unwrap();

    assert_eq!(
        output.payload,
        OutputEvent::Completed {
            summary: "CLI page created".to_string()
        }
    );
    assert_eq!(
        tokio::fs::read_to_string(dir.path().join("index.html"))
            .await
            .unwrap(),
        "<h1>CLI</h1>"
    );
}

#[tokio::test]
async fn cli_default_mock_runtime_creates_page_file() {
    let dir = tempfile::tempdir().unwrap();
    let config = CliArgs::parse_from(["reshape", "--workspace", dir.path().to_str().unwrap()])
        .into_config()
        .unwrap();
    let runtime = build_runtime(config, Arc::new(MockLlmProvider::default())).unwrap();

    let output = runtime
        .process(Envelope::new(InputEvent::UserText {
            text: "create a page".to_string(),
            source: InputSource::Cli,
        }))
        .await
        .unwrap();

    assert_eq!(
        output.payload,
        OutputEvent::Completed {
            summary: "Mock page generated in index.html".to_string()
        }
    );
    assert!(dir.path().join("index.html").exists());
}

#[test]
fn completed_output_with_index_html_triggers_browser_render() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<h1>CLI</h1>").unwrap();
    let renderer = FakeBrowserRenderer::default();
    let calls = renderer.opened.clone();

    let warning = reshape_cli::render_completed_output(
        Some(&renderer),
        dir.path(),
        &OutputEvent::Completed {
            summary: "done".to_string(),
        },
    );

    assert!(warning.is_none());
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].ends_with("index.html"));
}

#[test]
fn non_completed_output_does_not_trigger_browser_render() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<h1>CLI</h1>").unwrap();
    let renderer = FakeBrowserRenderer::default();

    let warning = reshape_cli::render_completed_output(
        Some(&renderer),
        dir.path(),
        &OutputEvent::FinalMessage {
            text: "still working".to_string(),
        },
    );

    assert!(warning.is_none());
    assert!(renderer.opened.lock().unwrap().is_empty());
}

#[derive(Default)]
struct FakeBrowserRenderer {
    opened: Arc<Mutex<Vec<String>>>,
}

impl BrowserRenderer for FakeBrowserRenderer {
    fn open_workspace_entry(&self, path: &Path) -> Result<(), BrowserRenderError> {
        self.opened
            .lock()
            .unwrap()
            .push(path.to_string_lossy().to_string());
        Ok(())
    }

    fn reload(&self) -> Result<(), BrowserRenderError> {
        Ok(())
    }

    fn snapshot(&self) -> Result<(), BrowserRenderError> {
        Ok(())
    }

    fn screenshot(&self, _path: Option<&Path>) -> Result<(), BrowserRenderError> {
        Ok(())
    }

    fn close(&self) -> Result<(), BrowserRenderError> {
        Ok(())
    }
}

#[tokio::test]
async fn cli_stdin_ingress_skips_blank_lines_without_ending() {
    let input = tokio::io::BufReader::new(&b"\nmake a page\n"[..]);
    let mut ingress = CliStdinIngress::new(input);

    let event = ingress.next_event().await.unwrap().unwrap();

    assert_eq!(
        event,
        InputEvent::UserText {
            text: "make a page".to_string(),
            source: InputSource::Cli,
        }
    );
}

#[tokio::test]
async fn cli_ingress_bus_runtime_chain_creates_workspace_file() {
    let dir = tempfile::tempdir().unwrap();
    let config = CliArgs::parse_from(["reshape", "--workspace", dir.path().to_str().unwrap()])
        .into_config()
        .unwrap();
    let runtime = build_runtime(config, Arc::new(MockLlmProvider::default())).unwrap();
    let (bus, mut inbound_rx, mut outbound_rx) = InProcessBus::new(8);
    let input = tokio::io::BufReader::new(&b"create a page\n"[..]);
    let mut ingress = CliStdinIngress::new(input);

    let output = reshape_cli::process_ingress_once(
        &mut ingress,
        &bus,
        &mut inbound_rx,
        &mut outbound_rx,
        &runtime,
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(
        output,
        OutputEvent::Completed {
            summary: "Mock page generated in index.html".to_string()
        }
    );
    assert!(dir.path().join("index.html").exists());
}
