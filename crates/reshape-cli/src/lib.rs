use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use reshape_browser::agent_browser::{BrowserOptions, BrowserSession};
use reshape_browser::{AgentBrowserRenderer, BrowserRenderer};
use reshape_core::bus::{EventBus, InProcessBus};
use reshape_core::config::AppConfig;
use reshape_core::error::{ReshapeError, Result};
use reshape_core::ingress::IngressSource;
use reshape_core::ingress::cli_stdin::CliStdinIngress;
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
use tokio::sync::mpsc;

#[derive(Debug, Clone, Parser, PartialEq, Eq)]
#[command(name = "reshape")]
#[command(about = "Local single-session agent runtime for AI-rendered pages")]
pub struct CliArgs {
    #[arg(long)]
    pub workspace: PathBuf,

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
        let mut config = AppConfig::for_workspace(self.workspace)?.with_model(self.model);
        if self.mock_llm {
            config = config.with_mock_llm(true);
        }
        Ok(config)
    }

    pub fn browser_renderer(&self) -> Option<Box<dyn BrowserRenderer>> {
        if !self.render_browser {
            return None;
        }

        let options = BrowserOptions {
            session: self.browser_session.clone(),
            headed: self.browser_headed,
            allow_file_access: true,
            ..BrowserOptions::default()
        };
        Some(Box::new(AgentBrowserRenderer::new(BrowserSession::new(
            options,
        ))))
    }
}

pub async fn run() -> Result<()> {
    tracing_subscriber::fmt::try_init().ok();
    let args = CliArgs::parse();
    let browser = args.browser_renderer();
    let config = args.into_config()?;
    let workspace_root = config.workspace.root.clone();
    let runtime = build_runtime(config, Arc::new(MockLlmProvider::default()))?;
    run_stdin_loop(runtime, workspace_root, browser).await
}

pub fn build_runtime(config: AppConfig, provider: Arc<dyn LlmProvider>) -> Result<AgentRuntime> {
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

async fn run_stdin_loop(
    runtime: AgentRuntime,
    workspace_root: PathBuf,
    browser: Option<Box<dyn BrowserRenderer>>,
) -> Result<()> {
    let (bus, mut inbound_rx, mut outbound_rx) = InProcessBus::new(64);
    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let mut ingress = CliStdinIngress::new(stdin);

    while let Some(output) = process_ingress_once(
        &mut ingress,
        &bus,
        &mut inbound_rx,
        &mut outbound_rx,
        &runtime,
    )
    .await?
    {
        if let Some(warning) = render_completed_output(browser.as_deref(), &workspace_root, &output)
        {
            eprintln!("{warning}");
        }
        print_output(output);
    }

    Ok(())
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
        return Ok(None);
    };

    bus.publish_inbound(Envelope::new(event)).await?;
    let inbound = inbound_rx.recv().await.ok_or_else(closed_bus_error)?;
    let outbound = runtime.process(inbound).await?;
    bus.publish_outbound(outbound).await?;

    Ok(Some(
        outbound_rx
            .recv()
            .await
            .ok_or_else(closed_bus_error)?
            .payload,
    ))
}

fn closed_bus_error() -> ReshapeError {
    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "in-process bus closed").into()
}

fn print_output(output: OutputEvent) {
    match output {
        OutputEvent::FinalMessage { text } => println!("{text}"),
        OutputEvent::Completed { summary } => println!("{summary}"),
        OutputEvent::StreamChunk { text } => print!("{text}"),
        OutputEvent::ToolProgress { tool_name, message } => println!("[{tool_name}] {message}"),
        OutputEvent::WorkspaceFileChanged { path } => {
            println!("workspace changed: {}", path.to_string_lossy())
        }
        OutputEvent::Error { code, message } => println!("{code:?}: {message}"),
    }
}
