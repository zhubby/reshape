use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::sync::mpsc;

use crate::bus::{EventBus, InProcessBus};
use crate::config::AppConfig;
use crate::error::Result;
use crate::ingress::IngressSource;
use crate::ingress::cli_stdin::CliStdinIngress;
use crate::llm::LlmProvider;
use crate::llm::mock::MockLlmProvider;
use crate::observability::NoopTelemetry;
use crate::protocol::{Envelope, InputEvent, OutputEvent};
use crate::runtime::{AgentRuntime, RuntimeDeps, RuntimeLimits};
use crate::session::store::InMemorySessionStore;
use crate::tools::InMemoryToolRegistry;
use crate::tools::complete::CompleteTaskTool;
use crate::tools::file::FileTool;
use crate::workspace::local::LocalWorkspace;

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
}

pub async fn run() -> Result<()> {
    tracing_subscriber::fmt::try_init().ok();
    let args = CliArgs::parse();
    let config = args.into_config()?;
    let runtime = build_runtime(config, Arc::new(MockLlmProvider::default()))?;
    run_stdin_loop(runtime).await
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

async fn run_stdin_loop(runtime: AgentRuntime) -> Result<()> {
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
        print_output(output);
    }

    Ok(())
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

fn closed_bus_error() -> crate::error::ReshapeError {
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
