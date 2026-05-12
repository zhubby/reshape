use std::sync::Arc;

use crate::llm::LlmProvider;
use crate::observability::TelemetrySink;
use crate::session::store::SessionStore;
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;

#[derive(Clone)]
pub struct RuntimeDeps {
    pub llm: Arc<dyn LlmProvider>,
    pub tools: Arc<dyn ToolRegistry>,
    pub sessions: Arc<dyn SessionStore>,
    pub workspace: Arc<dyn Workspace>,
    pub telemetry: Arc<dyn TelemetrySink>,
}
