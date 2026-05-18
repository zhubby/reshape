use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use reshape_browser::agent_browser::{BrowserOptions, BrowserSession};
use reshape_browser::{AgentBrowserRenderer, BrowserRenderer};
use reshape_core::config::{AppConfig, validate_workspace};
use reshape_core::error::{ReshapeError, Result};
use reshape_core::llm::{LlmProvider, OpenAiChatCompletionProvider};
use reshape_core::observability::NoopTelemetry;
use reshape_core::protocol::OutputEvent;
use reshape_core::runtime::{AgentRuntime, RuntimeDeps, RuntimeLimits};
use reshape_core::session::store::{FileSessionStore, InMemorySessionStore, SessionStore};
use reshape_core::tools::InMemoryToolRegistry;
use reshape_core::tools::complete::CompleteTaskTool;
use reshape_core::tools::file::FileTool;
use reshape_core::tools::web_fetch::WebFetchTool;
use reshape_core::tools::web_search::WebSearchTool;
use reshape_core::workspace::Workspace;
use reshape_core::workspace::local::LocalWorkspace;
use serde::Deserialize;
use tokio::net::TcpListener;
use tracing_subscriber::filter::LevelFilter;

pub mod rpc_protocol;
pub mod rpc_server;

#[derive(Debug, Clone, Parser, PartialEq, Eq)]
#[command(name = "reshape")]
#[command(about = "Local single-session agent runtime for AI-rendered pages")]
pub struct CliArgs {
    #[command(flatten)]
    agent: AgentOptions,

    #[arg(long)]
    pub log_level: Option<String>,

    #[command(subcommand)]
    command: Option<CommandArgs>,
}

#[derive(Debug, Clone, Args, Default, PartialEq, Eq)]
pub struct AgentOptions {
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    #[arg(long)]
    pub config: Option<PathBuf>,

    #[arg(long)]
    pub model: Option<String>,

    #[arg(long)]
    pub render_browser: bool,

    #[arg(long, default_value = "reshape-main")]
    pub browser_session: String,

    #[arg(long)]
    pub browser_headed: bool,

    #[arg(long)]
    pub host: Option<String>,

    #[arg(long)]
    pub port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliCommand {
    Agent,
    Version,
    WorkspaceClean,
    WorkspaceInit,
}

#[derive(Debug, Clone, Subcommand, PartialEq, Eq)]
enum CommandArgs {
    Agent(AgentOptions),
    Version,
    Workspace(WorkspaceCommandArgs),
}

#[derive(Debug, Clone, Args, PartialEq, Eq)]
struct WorkspaceCommandArgs {
    #[command(subcommand)]
    command: WorkspaceSubcommandArgs,
}

#[derive(Debug, Clone, Subcommand, PartialEq, Eq)]
enum WorkspaceSubcommandArgs {
    Clean(AgentOptions),
    Init(AgentOptions),
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileConfig {
    workspace: Option<PathBuf>,
    log_level: Option<String>,
    llm: Option<FileLlmConfig>,
    runtime: Option<FileRuntimeConfig>,
    server: Option<FileServerConfig>,
    tools: Option<FileToolsConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileLlmConfig {
    provider: Option<String>,
    openai: Option<FileOpenAiConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileOpenAiConfig {
    model: Option<String>,
    base_url: Option<String>,
    api_key: Option<String>,
    stream: Option<bool>,
    timeout_secs: Option<u64>,
    organization: Option<String>,
    project: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileRuntimeConfig {
    max_tool_iterations: Option<usize>,
    max_tool_calls: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileServerConfig {
    host: Option<String>,
    port: Option<u16>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileToolsConfig {
    web_search: Option<FileWebSearchConfig>,
    web_fetch: Option<FileWebFetchConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileWebSearchConfig {
    enabled: Option<bool>,
    provider: Option<String>,
    tavily: Option<FileTavilyConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileTavilyConfig {
    api_key: Option<String>,
    env_key: Option<String>,
    base_url: Option<String>,
    search_depth: Option<String>,
    max_results: Option<usize>,
    topic: Option<String>,
    include_answer: Option<bool>,
    include_images: Option<bool>,
    include_favicon: Option<bool>,
    timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileWebFetchConfig {
    enabled: Option<bool>,
    max_bytes: Option<usize>,
    timeout_secs: Option<u64>,
    max_redirects: Option<usize>,
    download_dir: Option<String>,
    allowed_content_types: Option<Vec<String>>,
    ssrf_allowlist: Option<Vec<String>>,
}

impl CliArgs {
    pub fn parse_from<I, T>(itr: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        <Self as Parser>::parse_from(itr)
    }

    pub fn into_config(self) -> Result<AppConfig> {
        let home = dirs::home_dir().ok_or_else(|| {
            ReshapeError::Config("could not determine user home directory".into())
        })?;
        self.into_config_with_home(&home)
    }

    pub fn into_config_with_home(self, home: &Path) -> Result<AppConfig> {
        self.prepare_user_environment_with_home(home)?;
        let options = self.agent_options();
        let config_path = self.resolved_config_path_with_home(home);
        let explicit_config_path = options.config.is_some();
        tracing::debug!(
            config_path = %config_path.display(),
            explicit_config_path,
            "building app config from cli and file config"
        );
        let file_config = load_file_config(&config_path, explicit_config_path)?;
        let workspace = resolve_workspace(home, &options, &file_config)?;

        let mut config = AppConfig::for_workspace(workspace)?;
        if let Some(runtime) = file_config.runtime {
            if let Some(max_tool_iterations) = runtime.max_tool_iterations {
                config.runtime.max_tool_iterations = max_tool_iterations;
            }
            if let Some(max_tool_calls) = runtime.max_tool_calls {
                config.runtime.max_tool_calls = max_tool_calls;
            }
        }
        apply_llm_file_config(&mut config, file_config.llm)?;
        apply_tools_file_config(&mut config, file_config.tools);

        if let Some(model) = options.model {
            config = config.with_model(model);
        }
        tracing::debug!(
            workspace = %config.workspace.root.display(),
            provider = config.llm.provider.as_str(),
            model = config.llm.openai.model,
            max_tool_iterations = config.runtime.max_tool_iterations,
            max_tool_calls = config.runtime.max_tool_calls,
            "app config built"
        );
        Ok(config)
    }

    pub fn browser_renderer(&self) -> Option<Box<dyn BrowserRenderer>> {
        let options = self.agent_options();
        if !options.render_browser {
            return None;
        }

        Some(Box::new(AgentBrowserRenderer::new(BrowserSession::new(
            self.render_browser_options(),
        ))))
    }

    pub fn startup_browser_renderer(&self) -> Box<dyn BrowserRenderer> {
        Box::new(AgentBrowserRenderer::new(BrowserSession::new(
            self.startup_browser_options(),
        )))
    }

    #[must_use]
    pub fn startup_browser_options(&self) -> BrowserOptions {
        let options = self.agent_options();
        managed_browser_options(options.browser_session, true)
    }

    #[must_use]
    pub fn render_browser_options(&self) -> BrowserOptions {
        let options = self.agent_options();
        managed_browser_options(options.browser_session, options.browser_headed)
    }

    #[must_use]
    pub fn agent_options(&self) -> AgentOptions {
        match &self.command {
            Some(CommandArgs::Agent(options)) => self.agent.clone().merge(options.clone()),
            Some(CommandArgs::Workspace(WorkspaceCommandArgs {
                command:
                    WorkspaceSubcommandArgs::Clean(options) | WorkspaceSubcommandArgs::Init(options),
            })) => self.agent.clone().merge(options.clone()),
            _ => self.agent.clone(),
        }
    }

    #[must_use]
    pub fn command_kind(&self) -> CliCommand {
        match &self.command {
            Some(CommandArgs::Version) => CliCommand::Version,
            Some(CommandArgs::Workspace(WorkspaceCommandArgs {
                command: WorkspaceSubcommandArgs::Clean(_),
            })) => CliCommand::WorkspaceClean,
            Some(CommandArgs::Workspace(WorkspaceCommandArgs {
                command: WorkspaceSubcommandArgs::Init(_),
            })) => CliCommand::WorkspaceInit,
            _ => CliCommand::Agent,
        }
    }

    #[must_use]
    pub fn resolved_config_path_with_home(&self, home: &Path) -> PathBuf {
        self.agent_options()
            .config
            .unwrap_or_else(|| default_app_dir(home).join("config.toml"))
    }

    pub fn resolved_log_level_with_home(&self, home: &Path) -> Result<String> {
        if let Some(level) = &self.log_level {
            return Ok(level.clone());
        }

        let options = self.agent_options();
        let config_path = self.resolved_config_path_with_home(home);
        let file_config = load_file_config(&config_path, options.config.is_some())?;
        Ok(file_config.log_level.unwrap_or_else(|| "info".to_string()))
    }

    pub fn server_config_with_home(&self, home: &Path) -> Result<ServerConfig> {
        let options = self.agent_options();
        let config_path = self.resolved_config_path_with_home(home);
        tracing::debug!(config_path = %config_path.display(), "loading server config");
        let file_config = load_file_config(&config_path, options.config.is_some())?;
        let file_server = file_config.server.unwrap_or_default();
        let config = ServerConfig {
            host: options
                .host
                .or(file_server.host)
                .unwrap_or_else(|| "127.0.0.1".to_string()),
            port: options.port.or(file_server.port).unwrap_or(7331),
        };
        config.validate()?;
        tracing::debug!(host = %config.host, port = config.port, "server config resolved");
        Ok(config)
    }

    pub fn prepare_user_environment_with_home(&self, home: &Path) -> Result<()> {
        let workspace = default_app_dir(home).join("workspace");
        tracing::debug!(workspace = %workspace.display(), "ensuring default workspace directory");
        std::fs::create_dir_all(&workspace)?;

        let options = self.agent_options();
        if options.config.is_some() {
            tracing::debug!(
                "skipping default config creation because explicit config was provided"
            );
            return Ok(());
        }

        let config_path = default_app_dir(home).join("config.toml");
        if config_path.exists() {
            tracing::debug!(config_path = %config_path.display(), "default config already exists");
            return Ok(());
        }

        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&config_path, default_config_toml(&workspace))?;
        tracing::info!(config_path = %config_path.display(), "wrote default config file");
        Ok(())
    }

    pub async fn init_workspace_with_home(&self, home: &Path) -> Result<bool> {
        self.prepare_user_environment_with_home(home)?;
        let workspace = self.resolved_workspace_for_init_with_home(home)?;
        tokio::fs::create_dir_all(&workspace).await?;
        ensure_workspace_index_html(&workspace).await
    }

    fn resolved_workspace_for_init_with_home(&self, home: &Path) -> Result<PathBuf> {
        let options = self.agent_options();
        if let Some(workspace) = options.workspace {
            return Ok(workspace);
        }

        let config_path = self.resolved_config_path_with_home(home);
        let file_config = load_file_config(&config_path, options.config.is_some())?;
        if let Some(workspace) = file_config.workspace {
            return Ok(workspace);
        }

        Ok(default_app_dir(home).join("workspace"))
    }
}

impl AgentOptions {
    fn merge(mut self, override_options: Self) -> Self {
        self.workspace = override_options.workspace.or(self.workspace);
        self.config = override_options.config.or(self.config);
        self.model = override_options.model.or(self.model);
        self.render_browser |= override_options.render_browser;
        self.browser_session = if override_options.browser_session != "reshape-main" {
            override_options.browser_session
        } else {
            self.browser_session
        };
        self.browser_headed |= override_options.browser_headed;
        self.host = override_options.host.or(self.host);
        self.port = override_options.port.or(self.port);
        self
    }
}

fn managed_browser_options(session: String, headed: bool) -> BrowserOptions {
    BrowserOptions {
        session,
        headed,
        allow_file_access: true,
        extensions: bundled_browser_extension_paths()
            .into_iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect(),
        ..BrowserOptions::default()
    }
}

pub async fn run() -> Result<()> {
    let args = CliArgs::parse();
    tracing::debug!(command = ?args.command_kind(), "cli arguments parsed");
    let home = dirs::home_dir()
        .ok_or_else(|| ReshapeError::Config("could not determine user home directory".into()))?;
    let log_level = args.resolved_log_level_with_home(&home)?;
    init_logging(&log_level)?;
    tracing::debug!(log_level, command = ?args.command_kind(), "logging initialized");
    args.prepare_user_environment_with_home(&home)?;
    if matches!(args.command_kind(), CliCommand::Version) {
        tracing::info!("printing reshape version");
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if matches!(args.command_kind(), CliCommand::WorkspaceClean) {
        let config = args.into_config_with_home(&home)?;
        let removed = clean_workspace(&config.workspace.root).await?;
        println!(
            "removed {removed} workspace entries from {}",
            config.workspace.root.display()
        );
        return Ok(());
    }

    if matches!(args.command_kind(), CliCommand::WorkspaceInit) {
        let generated = args.init_workspace_with_home(&home).await?;
        let workspace = args.resolved_workspace_for_init_with_home(&home)?;
        if generated {
            println!("initialized workspace at {}", workspace.display());
        } else {
            println!("workspace already initialized at {}", workspace.display());
        }
        return Ok(());
    }

    let server_config = args.server_config_with_home(&home)?;
    let config = args.clone().into_config_with_home(&home)?;
    let workspace_root = config.workspace.root.clone();
    if ensure_workspace_index_html(&workspace_root).await? {
        tracing::info!(workspace = %workspace_root.display(), "generated default workspace index.html");
    }
    let runtime = build_runtime_with_session_store(
        config,
        Arc::new(FileSessionStore::new(default_session_path(&home))),
    )?;
    let listener = TcpListener::bind(server_config.bind_addr()?).await?;
    let browser: Arc<dyn BrowserRenderer> = Arc::new(AgentBrowserRenderer::new(
        BrowserSession::new(args.startup_browser_options()),
    ));
    spawn_startup_browser_open(browser.clone(), server_config.clone());
    rpc_server::serve_rpc_listener_with_render_loop(
        listener,
        runtime,
        workspace_root,
        rpc_server::RpcRenderLoop {
            browser,
            local_url: server_config.local_url(),
        },
    )
    .await
}

fn spawn_startup_browser_open(browser: Arc<dyn BrowserRenderer>, server_config: ServerConfig) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        match tokio::task::spawn_blocking(move || {
            open_startup_browser(Some(browser.as_ref()), &server_config)
        })
        .await
        {
            Ok(Some(warning)) => tracing::warn!("{warning}"),
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "browser startup open task failed"),
        }
    });
}

pub fn init_logging(log_level: &str) -> Result<()> {
    let level = log_level
        .parse::<LevelFilter>()
        .map_err(|_| ReshapeError::Config(format!("invalid log level: {log_level}")))?;
    tracing_subscriber::fmt()
        .with_max_level(level)
        .try_init()
        .ok();
    Ok(())
}

fn load_file_config(path: &Path, explicit_path: bool) -> Result<FileConfig> {
    if !path.exists() {
        if explicit_path {
            tracing::warn!(config_path = %path.display(), "explicit config file does not exist");
            return Err(ReshapeError::Config(format!(
                "config file does not exist: {}",
                path.display()
            )));
        }
        tracing::debug!(config_path = %path.display(), "config file missing; using defaults");
        return Ok(FileConfig::default());
    }

    tracing::debug!(config_path = %path.display(), "reading config file");
    let content = std::fs::read_to_string(path)?;
    let config = toml::from_str(&content).map_err(|error| {
        tracing::warn!(config_path = %path.display(), %error, "failed to parse config TOML");
        ReshapeError::Config(format!("failed to parse config TOML: {error}"))
    })?;
    tracing::debug!(config_path = %path.display(), "config file parsed");
    Ok(config)
}

fn resolve_workspace(
    home: &Path,
    options: &AgentOptions,
    file_config: &FileConfig,
) -> Result<PathBuf> {
    if let Some(workspace) = &options.workspace {
        tracing::debug!(workspace = %workspace.display(), "using workspace from cli");
        return validate_workspace(workspace);
    }

    if let Some(workspace) = &file_config.workspace {
        tracing::debug!(workspace = %workspace.display(), "using workspace from config file");
        return validate_workspace(workspace);
    }

    let workspace = default_app_dir(home).join("workspace");
    tracing::debug!(workspace = %workspace.display(), "using default workspace");
    std::fs::create_dir_all(&workspace)?;
    validate_workspace(&workspace)
}

fn default_app_dir(home: &Path) -> PathBuf {
    home.join(".reshape")
}

pub fn bundled_browser_extension_paths() -> Vec<PathBuf> {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")));
    let extension = repo_root
        .join("extensions")
        .join("reshape")
        .join("build")
        .join("chrome-mv3-prod");

    if extension.join("manifest.json").is_file() {
        vec![extension]
    } else {
        Vec::new()
    }
}

fn default_config_toml(workspace: &Path) -> String {
    format!(
        r#"workspace = "{}"
log_level = "info"

[llm]
provider = "openai"

[llm.openai]
model = "gpt-5.5"
base_url = "https://api.openai.com/v1"
api_key = ""
stream = true
timeout_secs = 120

[runtime]
max_tool_iterations = 8
max_tool_calls = 32

[server]
host = "127.0.0.1"
port = 7331

[tools.web_search]
enabled = false
provider = "tavily"

[tools.web_search.tavily]
api_key = ""
env_key = "TAVILY_API_KEY"
base_url = "https://api.tavily.com"
search_depth = "basic"
max_results = 5
topic = "general"
include_answer = false
include_images = false
include_favicon = true
timeout_secs = 15

[tools.web_fetch]
enabled = false
max_bytes = 52428800
timeout_secs = 60
max_redirects = 5
download_dir = "assets/downloads"
allowed_content_types = [
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
  "video/mp4",
  "video/webm",
  "audio/mpeg",
  "audio/wav",
  "application/pdf",
]
ssrf_allowlist = []
"#,
        workspace.to_string_lossy()
    )
}

impl ServerConfig {
    fn validate(&self) -> Result<()> {
        let ip: IpAddr = self.host.parse().map_err(|error| {
            ReshapeError::Config(format!("invalid server host {}: {error}", self.host))
        })?;
        if !ip.is_loopback() {
            return Err(ReshapeError::Config(format!(
                "server host must be a loopback address, got {}",
                self.host
            )));
        }
        Ok(())
    }

    fn bind_addr(&self) -> Result<SocketAddr> {
        let ip: IpAddr = self.host.parse().map_err(|error| {
            ReshapeError::Config(format!("invalid server host {}: {error}", self.host))
        })?;
        self.validate()?;
        Ok(SocketAddr::new(ip, self.port))
    }

    #[must_use]
    pub fn local_url(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("http://{host}:{}/", self.port)
    }
}

fn apply_llm_file_config(config: &mut AppConfig, file_llm: Option<FileLlmConfig>) -> Result<()> {
    let Some(file_llm) = file_llm else {
        return Ok(());
    };

    if let Some(provider) = file_llm.provider {
        config.llm.provider = provider.as_str().try_into()?;
    }

    let Some(openai) = file_llm.openai else {
        return Ok(());
    };
    if let Some(model) = openai.model {
        config.llm.openai.model = model;
    }
    if let Some(base_url) = openai.base_url {
        config.llm.openai.base_url = base_url;
    }
    if let Some(api_key) = openai.api_key {
        config.llm.openai.api_key = api_key;
    }
    if let Some(stream) = openai.stream {
        config.llm.openai.stream = stream;
    }
    if let Some(timeout_secs) = openai.timeout_secs {
        config.llm.openai.timeout_secs = timeout_secs;
    }
    if let Some(organization) = openai.organization {
        config.llm.openai.organization = Some(organization);
    }
    if let Some(project) = openai.project {
        config.llm.openai.project = Some(project);
    }
    Ok(())
}

fn apply_tools_file_config(config: &mut AppConfig, file_tools: Option<FileToolsConfig>) {
    let Some(file_tools) = file_tools else {
        return;
    };

    if let Some(web_search) = file_tools.web_search {
        if let Some(enabled) = web_search.enabled {
            config.tools.web_search.enabled = enabled;
        }
        if let Some(provider) = web_search.provider {
            config.tools.web_search.provider = provider;
        }
        if let Some(tavily) = web_search.tavily {
            if let Some(api_key) = tavily.api_key {
                config.tools.web_search.tavily.api_key = api_key;
            }
            if let Some(env_key) = tavily.env_key {
                config.tools.web_search.tavily.env_key = env_key;
            }
            if let Some(base_url) = tavily.base_url {
                config.tools.web_search.tavily.base_url = base_url;
            }
            if let Some(search_depth) = tavily.search_depth {
                config.tools.web_search.tavily.search_depth = search_depth;
            }
            if let Some(max_results) = tavily.max_results {
                config.tools.web_search.tavily.max_results = max_results;
            }
            if let Some(topic) = tavily.topic {
                config.tools.web_search.tavily.topic = topic;
            }
            if let Some(include_answer) = tavily.include_answer {
                config.tools.web_search.tavily.include_answer = include_answer;
            }
            if let Some(include_images) = tavily.include_images {
                config.tools.web_search.tavily.include_images = include_images;
            }
            if let Some(include_favicon) = tavily.include_favicon {
                config.tools.web_search.tavily.include_favicon = include_favicon;
            }
            if let Some(timeout_secs) = tavily.timeout_secs {
                config.tools.web_search.tavily.timeout_secs = timeout_secs;
            }
        }
    }

    if let Some(web_fetch) = file_tools.web_fetch {
        if let Some(enabled) = web_fetch.enabled {
            config.tools.web_fetch.enabled = enabled;
        }
        if let Some(max_bytes) = web_fetch.max_bytes {
            config.tools.web_fetch.max_bytes = max_bytes;
        }
        if let Some(timeout_secs) = web_fetch.timeout_secs {
            config.tools.web_fetch.timeout_secs = timeout_secs;
        }
        if let Some(max_redirects) = web_fetch.max_redirects {
            config.tools.web_fetch.max_redirects = max_redirects;
        }
        if let Some(download_dir) = web_fetch.download_dir {
            config.tools.web_fetch.download_dir = download_dir;
        }
        if let Some(allowed_content_types) = web_fetch.allowed_content_types {
            config.tools.web_fetch.allowed_content_types = allowed_content_types;
        }
        if let Some(ssrf_allowlist) = web_fetch.ssrf_allowlist {
            config.tools.web_fetch.ssrf_allowlist = ssrf_allowlist;
        }
    }
}

pub fn build_runtime(config: AppConfig) -> Result<AgentRuntime> {
    build_runtime_with_session_store(config, Arc::new(InMemorySessionStore::default()))
}

pub fn build_runtime_with_session_store(
    config: AppConfig,
    sessions: Arc<dyn SessionStore>,
) -> Result<AgentRuntime> {
    let provider: Arc<dyn LlmProvider> = match config.llm.provider {
        reshape_core::config::LlmProviderKind::OpenAi => Arc::new(
            OpenAiChatCompletionProvider::from_config(config.llm.openai.clone())?,
        ),
    };
    build_runtime_with_provider_and_session_store(config, provider, sessions)
}

pub fn build_runtime_with_provider(
    config: AppConfig,
    provider: Arc<dyn LlmProvider>,
) -> Result<AgentRuntime> {
    build_runtime_with_provider_and_session_store(
        config,
        provider,
        Arc::new(InMemorySessionStore::default()),
    )
}

pub fn build_runtime_with_provider_and_session_store(
    config: AppConfig,
    provider: Arc<dyn LlmProvider>,
    sessions: Arc<dyn SessionStore>,
) -> Result<AgentRuntime> {
    tracing::debug!(
        workspace = %config.workspace.root.display(),
        provider = %provider.name(),
        max_tool_iterations = config.runtime.max_tool_iterations,
        max_tool_calls = config.runtime.max_tool_calls,
        "building agent runtime"
    );
    let workspace = Arc::new(LocalWorkspace::new(config.workspace.root)?);
    let mut tools = InMemoryToolRegistry::new()
        .register(FileTool::list_files())
        .register(FileTool::read_file())
        .register(FileTool::write_file())
        .register(FileTool::delete_file())
        .register(CompleteTaskTool);
    if config.tools.web_search.enabled {
        tools = tools.register(WebSearchTool::new(config.tools.web_search.clone())?);
    }
    if config.tools.web_fetch.enabled {
        tools = tools.register(WebFetchTool::new(config.tools.web_fetch.clone())?);
    }

    Ok(AgentRuntime::new(
        RuntimeDeps {
            llm: provider,
            tools: Arc::new(tools),
            sessions,
            workspace,
            telemetry: Arc::new(NoopTelemetry),
        },
        RuntimeLimits {
            max_tool_iterations: config.runtime.max_tool_iterations,
            max_tool_calls: config.runtime.max_tool_calls,
        },
    ))
}

fn default_session_path(home: &Path) -> PathBuf {
    default_app_dir(home).join("session-local-main.json")
}

pub fn render_completed_output(
    browser: Option<&dyn BrowserRenderer>,
    workspace_root: &Path,
    output: &OutputEvent,
) -> Option<String> {
    if !matches!(output, OutputEvent::Completed { .. }) {
        return None;
    }

    let browser = browser?;
    let entry = workspace_root.join("index.html");
    if !entry.exists() {
        return None;
    }

    browser
        .open_workspace_entry(&entry)
        .err()
        .map(|error| format!("browser render failed: {error}"))
}

pub async fn ensure_workspace_index_html(workspace_root: &Path) -> Result<bool> {
    let index_path = workspace_root.join("index.html");
    match tokio::fs::metadata(&index_path).await {
        Ok(metadata) if metadata.is_file() => return Ok(false),
        Ok(_) => {
            return Err(ReshapeError::Config(format!(
                "workspace index path exists but is not a file: {}",
                index_path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    ensure_default_site_css(workspace_root).await?;
    tokio::fs::write(&index_path, default_workspace_index_html()).await?;
    Ok(true)
}

pub async fn clean_workspace(workspace_root: &Path) -> Result<usize> {
    let workspace = LocalWorkspace::new(workspace_root)?;
    let root = workspace.root();
    let mut removed = 0;
    let mut entries = tokio::fs::read_dir(&root).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let file_type = tokio::fs::symlink_metadata(&path).await?.file_type();
        if file_type.is_dir() {
            tokio::fs::remove_dir_all(&path).await?;
        } else {
            tokio::fs::remove_file(&path).await?;
        }
        removed += 1;
    }

    Ok(removed)
}

async fn ensure_default_site_css(workspace_root: &Path) -> Result<()> {
    let assets_path = workspace_root.join("assets");
    match tokio::fs::metadata(&assets_path).await {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(ReshapeError::Config(format!(
                "workspace assets path exists but is not a directory: {}",
                assets_path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tokio::fs::create_dir_all(&assets_path).await?;
        }
        Err(error) => return Err(error.into()),
    }

    let css_path = assets_path.join("site.css");
    match tokio::fs::metadata(&css_path).await {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(ReshapeError::Config(format!(
            "workspace stylesheet path exists but is not a file: {}",
            css_path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tokio::fs::write(&css_path, default_workspace_site_css()).await?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

#[must_use]
pub fn default_workspace_index_html() -> &'static str {
    include_str!("../templates/default_index.html")
}

#[must_use]
pub fn default_workspace_site_css() -> &'static str {
    include_str!("../templates/default_site.css")
}

pub fn open_startup_browser(
    browser: Option<&dyn BrowserRenderer>,
    server_config: &ServerConfig,
) -> Option<String> {
    let browser = browser?;
    let url = server_config.local_url();
    let _ = browser.close();
    std::thread::sleep(Duration::from_millis(250));
    browser
        .open_url(&url)
        .err()
        .map(|error| format!("browser startup open failed: {error}"))
}
