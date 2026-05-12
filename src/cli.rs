use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::io::AsyncBufReadExt;

use crate::config::AppConfig;
use crate::error::Result;
use crate::llm::LlmProvider;
use crate::llm::mock::MockLlmProvider;
use crate::observability::NoopTelemetry;
use crate::protocol::{Envelope, InputEvent, InputSource, OutputEvent};
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
        Ok(AppConfig::for_workspace(self.workspace)?
            .with_model(self.model)
            .with_mock_llm(self.mock_llm))
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
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let output = runtime
            .process(Envelope::new(InputEvent::UserText {
                text: line.to_string(),
                source: InputSource::Cli,
            }))
            .await?;
        print_output(output.payload);
    }

    Ok(())
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
