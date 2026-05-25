use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use reshape_browser::{BrowserRenderError, BrowserRenderer};
use reshape_cli::{CliArgs, CliCommand, build_runtime, build_runtime_with_provider};
use reshape_core::error::Result as CoreResult;
use reshape_core::llm::{ChatMessage, ChatOptions, LlmProvider, LlmResponse, ToolCall};
use reshape_core::protocol::{Envelope, InputEvent, InputSource, OutputEvent};
use tokio::sync::Mutex as TokioMutex;

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

    let config = args.clone().into_config_with_home(home.path()).unwrap();

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
fn cli_parses_workspace_clean_command() {
    let args = CliArgs::parse_from(["reshape", "workspace", "clean"]);

    assert_eq!(args.command_kind(), CliCommand::WorkspaceClean);
}

#[test]
fn cli_parses_workspace_init_command() {
    let args = CliArgs::parse_from(["reshape", "workspace", "init"]);

    assert_eq!(args.command_kind(), CliCommand::WorkspaceInit);
}

#[tokio::test]
async fn workspace_init_creates_default_files_in_explicit_workspace() {
    let home = tempfile::tempdir().unwrap();
    let workspace = home.path().join("custom-workspace");
    let args = CliArgs::parse_from([
        "reshape",
        "--workspace",
        workspace.to_str().unwrap(),
        "workspace",
        "init",
    ]);

    let generated = args.init_workspace_with_home(home.path()).await.unwrap();

    assert!(generated);
    assert!(workspace.join("index.html").is_file());
    assert!(workspace.join("assets").join("site.css").is_file());
}

#[tokio::test]
async fn workspace_init_accepts_workspace_option_on_init_subcommand() {
    let home = tempfile::tempdir().unwrap();
    let workspace = home.path().join("nested-option-workspace");
    let args = CliArgs::parse_from([
        "reshape",
        "workspace",
        "init",
        "--workspace",
        workspace.to_str().unwrap(),
    ]);

    let generated = args.init_workspace_with_home(home.path()).await.unwrap();

    assert!(generated);
    assert!(workspace.join("index.html").is_file());
    assert!(workspace.join("assets").join("site.css").is_file());
}

#[tokio::test]
async fn workspace_init_uses_default_workspace_without_explicit_path() {
    let home = tempfile::tempdir().unwrap();
    let args = CliArgs::parse_from(["reshape", "workspace", "init"]);

    let generated = args.init_workspace_with_home(home.path()).await.unwrap();

    let workspace = home.path().join(".reshape").join("workspace");
    assert!(generated);
    assert!(workspace.join("index.html").is_file());
    assert!(workspace.join("assets").join("site.css").is_file());
}

#[tokio::test]
async fn workspace_clean_removes_workspace_contents_but_keeps_root() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    tokio::fs::create_dir_all(dir.path().join("assets").join("nested"))
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("index.html"), "<h1>old</h1>")
        .await
        .unwrap();
    tokio::fs::write(
        dir.path().join("assets").join("nested").join("site.css"),
        "",
    )
    .await
    .unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("linked-dir")).unwrap();

    let removed = reshape_cli::clean_workspace(dir.path()).await.unwrap();

    assert_eq!(removed, 3);
    assert!(dir.path().is_dir());
    assert!(outside.path().is_dir());
    assert!(
        tokio::fs::read_dir(dir.path())
            .await
            .unwrap()
            .next_entry()
            .await
            .unwrap()
            .is_none()
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
    assert!(config.contains("[llm]"));
    assert!(config.contains("provider = \"openai\""));
    assert!(config.contains("[llm.openai]"));
    assert!(config.contains("model = \"gpt-5.5\""));
    assert!(config.contains("api_key = \"\""));
    assert!(config.contains("stream = true"));
    assert!(config.contains("log_level = \"info\""));
    assert!(config.contains("[runtime]"));
    assert!(config.contains("max_tool_iterations = 8"));
    assert!(config.contains("max_tool_calls = 32"));
    assert!(config.contains("[server]"));
    assert!(config.contains("host = \"127.0.0.1\""));
    assert!(config.contains("port = 7331"));
    assert!(config.contains("[tools.web_search]"));
    assert!(config.contains("enabled = false"));
    assert!(config.contains("[tools.web_search.tavily]"));
    assert!(config.contains("env_key = \"TAVILY_API_KEY\""));
    assert!(config.contains("[tools.web_fetch]"));
    assert!(config.contains("download_dir = \"assets/downloads\""));
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

[llm]
provider = "openai"

[llm.openai]
model = "configured-model"
base_url = "http://127.0.0.1:9999/v1"
api_key = "sk-test-configured"
stream = false
timeout_secs = 5

[runtime]
max_tool_iterations = 3
max_tool_calls = 9

[server]
host = "127.0.0.1"
port = 7331

[tools.web_search]
enabled = true
provider = "tavily"

[tools.web_search.tavily]
api_key = "tvly-configured"
base_url = "http://127.0.0.1:18888"
search_depth = "advanced"
max_results = 7
include_answer = true
include_images = true

[tools.web_fetch]
enabled = true
max_bytes = 1024
timeout_secs = 3
max_redirects = 2
download_dir = "assets/media"
ssrf_allowlist = ["127.0.0.1/32"]
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
        "--host",
        "127.0.0.2",
        "--port",
        "7332",
    ]);
    let config = args.clone().into_config_with_home(home.path()).unwrap();
    let server = args.server_config_with_home(home.path()).unwrap();

    assert_eq!(config.workspace.root, cli_workspace);
    assert_eq!(config.llm.openai.model, "cli-model");
    assert_eq!(config.llm.openai.base_url, "http://127.0.0.1:9999/v1");
    assert_eq!(config.llm.openai.api_key, "sk-test-configured");
    assert!(!config.llm.openai.stream);
    assert_eq!(config.llm.openai.timeout_secs, 5);
    assert_eq!(config.runtime.max_tool_iterations, 3);
    assert_eq!(config.runtime.max_tool_calls, 9);
    assert!(config.tools.web_search.enabled);
    assert_eq!(config.tools.web_search.provider, "tavily");
    assert_eq!(config.tools.web_search.tavily.api_key, "tvly-configured");
    assert_eq!(
        config.tools.web_search.tavily.base_url,
        "http://127.0.0.1:18888"
    );
    assert_eq!(config.tools.web_search.tavily.search_depth, "advanced");
    assert_eq!(config.tools.web_search.tavily.max_results, 7);
    assert!(config.tools.web_search.tavily.include_answer);
    assert!(config.tools.web_search.tavily.include_images);
    assert!(config.tools.web_fetch.enabled);
    assert_eq!(config.tools.web_fetch.max_bytes, 1024);
    assert_eq!(config.tools.web_fetch.timeout_secs, 3);
    assert_eq!(config.tools.web_fetch.max_redirects, 2);
    assert_eq!(config.tools.web_fetch.download_dir, "assets/media");
    assert_eq!(config.tools.web_fetch.ssrf_allowlist, vec!["127.0.0.1/32"]);
    assert_eq!(server.host, "127.0.0.2");
    assert_eq!(server.port, 7332);
}

#[test]
fn cli_config_defaults_to_disabled_network_tools() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let args = CliArgs::parse_from(["reshape", "--workspace", dir.path().to_str().unwrap()]);

    let config = args.into_config_with_home(home.path()).unwrap();

    assert!(!config.tools.web_search.enabled);
    assert_eq!(config.tools.web_search.provider, "tavily");
    assert_eq!(config.tools.web_search.tavily.env_key, "TAVILY_API_KEY");
    assert!(!config.tools.web_fetch.enabled);
    assert_eq!(config.tools.web_fetch.download_dir, "assets/downloads");
}

#[test]
fn runtime_builder_rejects_enabled_web_search_without_tavily_token() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut config = CliArgs::parse_from(["reshape", "--workspace", dir.path().to_str().unwrap()])
        .into_config_with_home(home.path())
        .unwrap();
    config.llm.openai.api_key = "sk-test".to_string();
    config.tools.web_search.enabled = true;
    config.tools.web_search.tavily.api_key = String::new();
    config.tools.web_search.tavily.env_key = "RESHAPE_TEST_MISSING_TAVILY_KEY".to_string();

    let error = match build_runtime(config) {
        Ok(_) => panic!("runtime should reject missing Tavily token"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("Tavily"));
}

#[test]
fn cli_rejects_non_loopback_server_host() {
    let home = tempfile::tempdir().unwrap();
    let args = CliArgs::parse_from(["reshape", "--host", "0.0.0.0"]);

    let error = args.server_config_with_home(home.path()).unwrap_err();

    assert!(error.to_string().contains("loopback"));
}

#[test]
fn cli_config_defaults_to_openai_provider() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let args = CliArgs::parse_from(["reshape", "--workspace", dir.path().to_str().unwrap()]);

    let config = args.into_config_with_home(home.path()).unwrap();

    assert_eq!(config.llm.provider.as_str(), "openai");
    assert_eq!(config.llm.openai.model, "gpt-5.5");
    assert_eq!(config.llm.openai.api_key, "");
    assert!(config.llm.openai.stream);
}

#[tokio::test]
async fn cli_runtime_builder_can_create_page_with_injected_provider() {
    let dir = tempfile::tempdir().unwrap();
    let config = CliArgs::parse_from(["reshape", "--workspace", dir.path().to_str().unwrap()])
        .into_config()
        .unwrap();
    let provider = Arc::new(ScriptedLlmProvider::new([
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
    let runtime = build_runtime_with_provider(config, provider).unwrap();

    let output = runtime
        .process(Envelope::new(InputEvent::UserText {
            text: "create page".to_string(),
            source: InputSource::WebSocket,
        }))
        .await
        .unwrap();

    assert_eq!(
        output.payload,
        OutputEvent::Completed {
            summary: "CLI page created".to_string()
        }
    );
    let html = tokio::fs::read_to_string(dir.path().join("index.html"))
        .await
        .unwrap();
    assert!(html.contains("<h1>CLI</h1>"));
    assert!(html.contains(r#"href="assets/site.css""#));
}

#[tokio::test]
async fn cli_runtime_builder_registers_workspace_map_tool() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(
        dir.path().join("index.html"),
        r#"<a href="pages/topic.html">Topic</a>"#,
    )
    .await
    .unwrap();
    tokio::fs::create_dir_all(dir.path().join("pages"))
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("pages/topic.html"), "<h1>Topic</h1>")
        .await
        .unwrap();
    let config = CliArgs::parse_from(["reshape", "--workspace", dir.path().to_str().unwrap()])
        .into_config()
        .unwrap();
    let provider = Arc::new(ScriptedLlmProvider::new([
        LlmResponse {
            content: "Mapping".to_string(),
            tool_calls: vec![ToolCall {
                id: "map".to_string(),
                name: "map_workspace".to_string(),
                arguments: serde_json::json!({}),
            }],
        },
        LlmResponse {
            content: "Done".to_string(),
            tool_calls: vec![ToolCall {
                id: "complete".to_string(),
                name: "complete_task".to_string(),
                arguments: serde_json::json!({"summary": "workspace mapped"}),
            }],
        },
    ]));
    let runtime = build_runtime_with_provider(config, provider).unwrap();

    let output = runtime
        .process(Envelope::new(InputEvent::UserText {
            text: "map workspace".to_string(),
            source: InputSource::WebSocket,
        }))
        .await
        .unwrap();

    assert_eq!(
        output.payload,
        OutputEvent::Completed {
            summary: "workspace mapped".to_string()
        }
    );
}

#[tokio::test]
async fn cli_runtime_builder_requires_configured_openai_api_key_value() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = CliArgs::parse_from(["reshape", "--workspace", dir.path().to_str().unwrap()])
        .into_config()
        .unwrap();
    config.llm.openai.api_key = String::new();

    let error = match build_runtime(config) {
        Ok(_) => panic!("runtime should require missing OpenAI API key"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("must not be empty"));
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
fn startup_server_url_triggers_browser_open() {
    let renderer = FakeBrowserRenderer::default();
    let events = renderer.events.clone();
    let server = reshape_cli::ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 7331,
    };

    let warning = reshape_cli::open_startup_browser(Some(&renderer), &server);

    assert!(warning.is_none());
    assert_eq!(
        events.lock().unwrap().as_slice(),
        ["close", "open:http://127.0.0.1:7331/"]
    );
}

#[test]
fn startup_browser_defaults_to_visible_window() {
    let args = CliArgs::parse_from(["reshape"]);

    let options = args.startup_browser_options();

    assert!(options.headed);
}

#[test]
fn startup_browser_loads_bundled_extension_build_when_available() {
    let args = CliArgs::parse_from(["reshape"]);

    let options = args.startup_browser_options();
    let expected = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("extensions")
        .join("reshape")
        .join("build")
        .join("chrome-mv3-prod");

    if expected.join("manifest.json").is_file() {
        assert!(
            options
                .extensions
                .iter()
                .any(|path| path == &expected.to_string_lossy())
        );
    } else {
        assert!(options.extensions.is_empty());
    }
}

#[test]
fn render_browser_uses_same_extension_set_as_startup_browser() {
    let args = CliArgs::parse_from(["reshape", "--render-browser"]);

    let render_options = args.render_browser_options();
    let startup_options = args.startup_browser_options();

    assert_eq!(render_options.session, startup_options.session);
    assert_eq!(render_options.extensions, startup_options.extensions);
}

#[tokio::test]
async fn startup_generates_default_index_when_workspace_is_empty() {
    let dir = tempfile::tempdir().unwrap();

    let generated = reshape_cli::ensure_workspace_index_html(dir.path())
        .await
        .unwrap();

    assert!(generated);
    let html = tokio::fs::read_to_string(dir.path().join("index.html"))
        .await
        .unwrap();
    assert!(html.contains("<title>Reshape"));
    assert!(html.contains("LLM 动态生成 HTML"));
    assert!(html.contains("workspace"));
    assert!(html.contains("data-reshape-default-homepage"));
    assert!(html.contains(r#"<link rel="stylesheet" href="assets/site.css">"#));
    assert!(!html.contains("<style>"));

    let css = tokio::fs::read_to_string(dir.path().join("assets").join("site.css"))
        .await
        .unwrap();
    assert!(css.contains(":root"));
    assert!(css.contains(".masthead"));
    assert!(css.contains(".hero"));
    assert!(css.contains(".prose"));
    assert!(css.contains("text-decoration: underline"));
    assert!(css.contains("text-underline-offset"));
    assert!(css.contains(".wordmark"));
    assert!(css.contains(".nav a"));
}

#[tokio::test]
async fn startup_keeps_existing_workspace_index() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("index.html"), "<h1>custom</h1>")
        .await
        .unwrap();

    let generated = reshape_cli::ensure_workspace_index_html(dir.path())
        .await
        .unwrap();

    assert!(!generated);
    assert_eq!(
        tokio::fs::read_to_string(dir.path().join("index.html"))
            .await
            .unwrap(),
        "<h1>custom</h1>"
    );
}

#[tokio::test]
async fn startup_keeps_existing_default_stylesheet() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::create_dir_all(dir.path().join("assets"))
        .await
        .unwrap();
    tokio::fs::write(
        dir.path().join("assets").join("site.css"),
        "body { color: red; }",
    )
    .await
    .unwrap();

    let generated = reshape_cli::ensure_workspace_index_html(dir.path())
        .await
        .unwrap();

    assert!(generated);
    assert_eq!(
        tokio::fs::read_to_string(dir.path().join("assets").join("site.css"))
            .await
            .unwrap(),
        "body { color: red; }"
    );
}

#[tokio::test]
async fn startup_rejects_assets_path_that_is_not_directory() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("assets"), "not a directory")
        .await
        .unwrap();

    let error = reshape_cli::ensure_workspace_index_html(dir.path())
        .await
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("workspace assets path exists but is not a directory")
    );
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
    events: Arc<Mutex<Vec<String>>>,
}

impl BrowserRenderer for FakeBrowserRenderer {
    fn open_url(&self, url: &str) -> std::result::Result<(), BrowserRenderError> {
        self.events.lock().unwrap().push(format!("open:{url}"));
        self.opened.lock().unwrap().push(url.to_string());
        Ok(())
    }

    fn open_workspace_entry(&self, path: &Path) -> std::result::Result<(), BrowserRenderError> {
        self.opened
            .lock()
            .unwrap()
            .push(path.to_string_lossy().to_string());
        Ok(())
    }

    fn reload(&self) -> std::result::Result<(), BrowserRenderError> {
        Ok(())
    }

    fn snapshot(&self) -> std::result::Result<(), BrowserRenderError> {
        Ok(())
    }

    fn screenshot(&self, _path: Option<&Path>) -> std::result::Result<(), BrowserRenderError> {
        Ok(())
    }

    fn close(&self) -> std::result::Result<(), BrowserRenderError> {
        self.events.lock().unwrap().push("close".to_string());
        Ok(())
    }
}

#[derive(Debug)]
struct ScriptedLlmProvider {
    responses: TokioMutex<std::collections::VecDeque<LlmResponse>>,
}

impl ScriptedLlmProvider {
    fn new(responses: impl IntoIterator<Item = LlmResponse>) -> Self {
        Self {
            responses: TokioMutex::new(responses.into_iter().collect()),
        }
    }
}

#[async_trait]
impl LlmProvider for ScriptedLlmProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    fn default_model(&self) -> &str {
        "scripted-model"
    }

    async fn chat(
        &self,
        _messages: Vec<ChatMessage>,
        _tools: Vec<reshape_core::tools::types::ToolDefinition>,
        _options: ChatOptions,
    ) -> CoreResult<LlmResponse> {
        self.responses.lock().await.pop_front().ok_or_else(|| {
            reshape_core::error::ReshapeError::Provider("no scripted response".to_string())
        })
    }
}
