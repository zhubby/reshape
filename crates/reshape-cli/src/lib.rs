use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Args, Parser, Subcommand};
use reshape_browser::agent_browser::{BrowserOptions, BrowserSession};
use reshape_browser::{AgentBrowserRenderer, BrowserRenderer};
use reshape_core::bus::EventBus;
use reshape_core::config::{AppConfig, validate_workspace};
use reshape_core::error::{ReshapeError, Result};
use reshape_core::ingress::IngressSource;
use reshape_core::llm::LlmProvider;
use reshape_core::llm::mock::MockLlmProvider;
use reshape_core::observability::NoopTelemetry;
use reshape_core::protocol::{Envelope, InputEvent, OutputEvent};
use reshape_core::runtime::{AgentRuntime, RuntimeDeps, RuntimeLimits};
use reshape_core::session::store::InMemorySessionStore;
use reshape_core::tools::InMemoryToolRegistry;
use reshape_core::tools::complete::CompleteTaskTool;
use reshape_core::tools::file::FileTool;
use reshape_core::workspace::local::LocalWorkspace;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
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
    pub mock_llm: bool,

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
}

#[derive(Debug, Clone, Subcommand, PartialEq, Eq)]
enum CommandArgs {
    Agent(AgentOptions),
    Version,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileConfig {
    workspace: Option<PathBuf>,
    model: Option<String>,
    mock_llm: Option<bool>,
    log_level: Option<String>,
    runtime: Option<FileRuntimeConfig>,
    server: Option<FileServerConfig>,
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
        if let Some(model) = file_config.model {
            config.llm.model = Some(model);
        }
        if let Some(use_mock) = file_config.mock_llm {
            config.llm.use_mock = use_mock;
        }

        if let Some(model) = options.model {
            config = config.with_model(Some(model));
        }
        if options.mock_llm {
            config = config.with_mock_llm(true);
        }
        tracing::debug!(
            workspace = %config.workspace.root.display(),
            use_mock = config.llm.use_mock,
            model = config.llm.model.as_deref().unwrap_or(""),
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

        let browser_options = BrowserOptions {
            session: options.browser_session,
            headed: options.browser_headed,
            allow_file_access: true,
            ..BrowserOptions::default()
        };
        Some(Box::new(AgentBrowserRenderer::new(BrowserSession::new(
            browser_options,
        ))))
    }

    #[must_use]
    pub fn agent_options(&self) -> AgentOptions {
        match &self.command {
            Some(CommandArgs::Agent(options)) => self.agent.clone().merge(options.clone()),
            _ => self.agent.clone(),
        }
    }

    #[must_use]
    pub fn command_kind(&self) -> CliCommand {
        match &self.command {
            Some(CommandArgs::Version) => CliCommand::Version,
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
}

impl AgentOptions {
    fn merge(mut self, override_options: Self) -> Self {
        self.workspace = override_options.workspace.or(self.workspace);
        self.config = override_options.config.or(self.config);
        self.model = override_options.model.or(self.model);
        self.mock_llm |= override_options.mock_llm;
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

    let server_config = args.server_config_with_home(&home)?;
    let config = args.clone().into_config_with_home(&home)?;
    let workspace_root = config.workspace.root.clone();
    let runtime = build_runtime(config, Arc::new(MockLlmProvider::default()))?;
    let listener = TcpListener::bind(server_config.bind_addr()?).await?;
    rpc_server::serve_rpc_listener(listener, runtime, workspace_root).await
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

fn default_config_toml(workspace: &Path) -> String {
    format!(
        r#"workspace = "{}"
mock_llm = true
log_level = "info"

[runtime]
max_tool_iterations = 8
max_tool_calls = 32

[server]
host = "127.0.0.1"
port = 7331
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
}

pub fn build_runtime(config: AppConfig, provider: Arc<dyn LlmProvider>) -> Result<AgentRuntime> {
    tracing::debug!(
        workspace = %config.workspace.root.display(),
        provider = %provider.name(),
        max_tool_iterations = config.runtime.max_tool_iterations,
        max_tool_calls = config.runtime.max_tool_calls,
        "building agent runtime"
    );
    let workspace = Arc::new(LocalWorkspace::new(config.workspace.root)?);
    let tools = InMemoryToolRegistry::new()
        .register(FileTool::list_files())
        .register(FileTool::read_file())
        .register(FileTool::write_file())
        .register(FileTool::delete_file())
        .register(CompleteTaskTool);

    Ok(AgentRuntime::new(
        RuntimeDeps {
            llm: provider,
            tools: Arc::new(tools),
            sessions: Arc::new(InMemorySessionStore::default()),
            workspace,
            telemetry: Arc::new(NoopTelemetry),
        },
        RuntimeLimits {
            max_tool_iterations: config.runtime.max_tool_iterations,
            max_tool_calls: config.runtime.max_tool_calls,
        },
    ))
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

pub async fn process_ingress_once<I, B>(
    ingress: &mut I,
    bus: &B,
    inbound_rx: &mut mpsc::Receiver<Envelope<InputEvent>>,
    outbound_rx: &mut mpsc::Receiver<Envelope<OutputEvent>>,
    runtime: &AgentRuntime,
) -> Result<Option<OutputEvent>>
where
    I: IngressSource,
    B: EventBus,
{
    let Some(event) = ingress.next_event().await? else {
        tracing::debug!(ingress = ingress.name(), "ingress returned no event");
        return Ok(None);
    };

    tracing::debug!(ingress = ingress.name(), "processing ingress event");
    bus.publish_inbound(Envelope::new(event)).await?;
    let inbound = inbound_rx.recv().await.ok_or_else(closed_bus_error)?;
    tracing::debug!(
        message_id = %inbound.header.message_id,
        trace_id = %inbound.header.trace_id,
        "received event from inbound bus"
    );
    let outbound = runtime.process(inbound).await?;
    tracing::debug!(
        message_id = %outbound.header.message_id,
        trace_id = %outbound.header.trace_id,
        "runtime produced outbound event"
    );
    bus.publish_outbound(outbound).await?;

    let output = outbound_rx
        .recv()
        .await
        .ok_or_else(closed_bus_error)?
        .payload;
    tracing::debug!("received output from outbound bus");
    Ok(Some(output))
}

fn closed_bus_error() -> ReshapeError {
    tracing::error!("in-process bus closed");
    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "in-process bus closed").into()
}
