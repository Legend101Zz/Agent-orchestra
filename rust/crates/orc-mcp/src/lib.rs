//! pi-orchestra MCP server: the seven `orch_*` tools over stdio (issue #8).
//!
//! This crate is the *only* place tokio and the MCP protocol live. Each tool is
//! a thin async wrapper that hands its typed request to the matching synchronous
//! [`orc_core::orch`] function on a blocking thread and returns the durable
//! outcome as structured JSON. The core stays sync and tokio-free (per the #16
//! crate decision record); the CLI verbs call the very same `orch` functions, so
//! the two surfaces cannot drift (issue #8, AC3).

#![warn(missing_docs)]

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::transport::stdio;
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde::Serialize;

use orc_core::orch::{self, AwaitRequest, DelegateRequest, PlanRequest, StatusRequest, TaskRef};
use rmcp::model::CallToolResult;

/// The pi-orchestra MCP server. Holds the generated tool router; otherwise
/// stateless, so a fresh instance is cheap to build per connection.
#[derive(Clone)]
pub struct OrchServer {
    tool_router: ToolRouter<Self>,
}

impl Default for OrchServer {
    fn default() -> Self {
        Self::new()
    }
}

impl OrchServer {
    /// Build a server with every `orch_*` tool registered.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    /// Advertised tool names, sorted — used by the parity tests (issue #8, AC3).
    #[must_use]
    pub fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .tool_router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();
        names
    }

    /// Every advertised tool with its schema and description, for tests and
    /// introspection (issue #8, AC1/AC3).
    #[must_use]
    pub fn advertised_tools(&self) -> Vec<rmcp::model::Tool> {
        self.tool_router.list_all()
    }
}

/// Serialize a successful outcome as an MCP structured-content result.
fn ok_json<T: Serialize>(value: &T) -> Result<CallToolResult, String> {
    serde_json::to_value(value)
        .map(CallToolResult::structured)
        .map_err(|error| format!("failed to serialize orch outcome: {error}"))
}

/// Run one synchronous `orch` operation on the blocking pool so the reactor is
/// never blocked by filesystem locks or a worker invocation. Errors are flattened
/// to a plain string, which rmcp renders as a tool-level error result.
async fn run_blocking<T, F>(operation: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(operation).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(format!("{error:#}")),
        Err(join) => Err(format!("orch worker task failed: {join}")),
    }
}

// The `#[tool]` description literals below are kept byte-identical to
// `orc_core::orch::Verb::description()`; `tests/tools.rs` asserts they match, so
// the surface's advertised help can never drift from the source of truth.
#[tool_router(router = tool_router)]
impl OrchServer {
    /// Record a contracted backlog task without delegating it.
    #[tool(
        name = "orch_plan",
        description = "Record a contracted task on the board (backlog) without delegating it yet."
    )]
    async fn plan(
        &self,
        Parameters(request): Parameters<PlanRequest>,
    ) -> Result<CallToolResult, String> {
        ok_json(&run_blocking(move || orch::plan(request)).await?)
    }

    /// Assign, start, and dispatch a task to a worker harness.
    #[tool(
        name = "orch_delegate",
        description = "Delegate a task to a worker harness: assign it, start it, and dispatch its brief. Creates the task inline when no task id is given."
    )]
    async fn delegate(
        &self,
        Parameters(request): Parameters<DelegateRequest>,
    ) -> Result<CallToolResult, String> {
        ok_json(&run_blocking(move || orch::delegate(request)).await?)
    }

    /// Read one task (with its dispatches) or the whole board.
    #[tool(
        name = "orch_status",
        description = "Read the durable state of one task (or the whole board) with its dispatch records."
    )]
    async fn status(
        &self,
        Parameters(request): Parameters<StatusRequest>,
    ) -> Result<CallToolResult, String> {
        ok_json(&run_blocking(move || orch::status(request)).await?)
    }

    /// Block until a delegated task's newest delivery is terminal.
    #[tool(
        name = "orch_await",
        description = "Block until a delegated task's newest delivery reaches a terminal state (confirmed or failed), or the timeout elapses."
    )]
    async fn await_delegation(
        &self,
        Parameters(request): Parameters<AwaitRequest>,
    ) -> Result<CallToolResult, String> {
        ok_json(&run_blocking(move || orch::await_delegation(request)).await?)
    }

    /// Move a running task into review.
    #[tool(name = "orch_review", description = "Move a running task into review.")]
    async fn review(
        &self,
        Parameters(request): Parameters<TaskRef>,
    ) -> Result<CallToolResult, String> {
        ok_json(&run_blocking(move || orch::review(request)).await?)
    }

    /// Drop a task and stop a linked live worker best-effort.
    #[tool(
        name = "orch_cancel",
        description = "Drop a task, preserving its audit record, and stop a linked live worker best-effort."
    )]
    async fn cancel(
        &self,
        Parameters(request): Parameters<TaskRef>,
    ) -> Result<CallToolResult, String> {
        ok_json(&run_blocking(move || orch::cancel(request)).await?)
    }

    /// Mark a reviewed task done.
    #[tool(name = "orch_finish", description = "Mark a reviewed task done.")]
    async fn finish(
        &self,
        Parameters(request): Parameters<TaskRef>,
    ) -> Result<CallToolResult, String> {
        ok_json(&run_blocking(move || orch::finish(request)).await?)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for OrchServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "pi-orchestra",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "pi-orchestra control surface. Seven tools: orch_plan, orch_delegate, \
                 orch_status, orch_await, orch_review, orch_cancel, orch_finish. Delegation is \
                 contract-driven; a worker receives the task's rendered acceptance brief."
                    .to_owned(),
            )
    }
}

/// Serve the seven `orch_*` tools over stdio until the client disconnects.
///
/// Builds a self-contained current-thread runtime so tokio never appears in the
/// synchronous binary entry point's signature.
pub fn run_stdio() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let service = OrchServer::new().serve(stdio()).await?;
        service.waiting().await?;
        anyhow::Ok(())
    })
}
