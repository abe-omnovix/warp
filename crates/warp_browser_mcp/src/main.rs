//! Stdio MCP server that lets an agent attach and drive Warp's browser
//! underlay for the pane it is running in.
//!
//! The server speaks the same authenticated loopback protocol as `warpctrl`,
//! linking `local_control`'s client, discovery, and selection directly. Every
//! call requires the running Warp instance to have the browser-underlay
//! feature and the `allow_browser_control` permission (Settings > Scripting)
//! enabled; otherwise the control plane rejects the action and the error is
//! surfaced to the MCP client.
//!
//! Pane binding resolution order (see `specs/browser-underlay/MCP.md`):
//! 1. an explicit `pane` tool parameter (terminal session UUID),
//! 2. the `--pane-session-uuid` CLI flag,
//! 3. the `WARP_TERMINAL_SESSION_UUID` environment variable (Warp injects it
//!    into every pane's shell, so a server spawned by an agent CLI running
//!    inside a pane inherits its pane identity automatically),
//! 4. the active pane.
use clap::Parser;
use local_control::protocol::{
    ActionKind, BooleanValueParams, BrowserAttachParams, ControlError, EmptyParams, ErrorCode,
    JavascriptParams, PaneGlassParams, PaneSelector, PaneTarget, TabSelector, TabTarget,
    TargetSelector, UrlParams, WindowSelector, WindowTarget,
};
use local_control::selection::{InstanceSelector, select_instance};
use local_control::{Action, RequestEnvelope};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, ErrorData, ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use warp_core::channel::ChannelState;

/// Environment variable Warp injects into every pane's shell; used as the
/// default pane identity when no explicit binding is given.
const SESSION_UUID_ENV: &str = "WARP_TERMINAL_SESSION_UUID";

#[derive(Debug, Parser)]
#[command(
    name = "warp-browser-mcp",
    about = "MCP server that drives Warp's browser underlay for a terminal pane"
)]
struct Args {
    /// Terminal session UUID (hex) of the pane this server is bound to.
    /// Defaults to $WARP_TERMINAL_SESSION_UUID, then to the active pane.
    #[arg(long = "pane-session-uuid", env = SESSION_UUID_ENV)]
    pane_session_uuid: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct AttachRequest {
    #[schemars(description = "URL to load in the browser underlay")]
    url: String,
    #[schemars(
        description = "Turn the bound pane to night glass so the page shows through (default true)"
    )]
    glass: Option<bool>,
    #[schemars(description = "Glass fill opacity override, 0-100")]
    glass_opacity: Option<u8>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct NavigateRequest {
    #[schemars(description = "URL to navigate the underlay to")]
    url: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct EvalRequest {
    #[schemars(description = "JavaScript source to evaluate in the underlay page")]
    javascript: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SetGlassRequest {
    #[schemars(description = "Whether the pane renders as night glass")]
    enabled: bool,
    #[schemars(description = "Glass fill opacity override, 0-100")]
    opacity: Option<u8>,
    #[schemars(
        description = "Terminal session UUID of the pane to change; defaults to the bound pane"
    )]
    pane: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SetInteractiveRequest {
    #[schemars(description = "Whether input events reach the underlay page")]
    enabled: bool,
}

/// The pane (and its window/tab) a tool call operates on, resolved fresh per
/// call so bindings survive pane and window rearrangement.
#[derive(Debug, Clone)]
struct PaneBinding {
    window_id: String,
    tab_id: String,
    pane_id: String,
}

/// Selector targeting the binding's window only (underlay operations).
fn window_target(binding: &PaneBinding) -> TargetSelector {
    TargetSelector {
        window: Some(WindowTarget::Id {
            id: WindowSelector(binding.window_id.clone()),
        }),
        ..TargetSelector::default()
    }
}

/// Selector targeting the binding's exact pane (glass operations, attach).
fn pane_target(binding: &PaneBinding) -> TargetSelector {
    TargetSelector {
        window: Some(WindowTarget::Id {
            id: WindowSelector(binding.window_id.clone()),
        }),
        tab: Some(TabTarget::Id {
            id: TabSelector(binding.tab_id.clone()),
        }),
        pane: Some(PaneTarget::Id {
            id: PaneSelector(binding.pane_id.clone()),
        }),
        session: None,
    }
}

fn row_str(row: &serde_json::Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn binding_from_row(row: &serde_json::Value) -> Result<PaneBinding, ControlError> {
    match (
        row_str(row, "window_id"),
        row_str(row, "tab_id"),
        row_str(row, "pane_id"),
    ) {
        (Some(window_id), Some(tab_id), Some(pane_id)) => Ok(PaneBinding {
            window_id,
            tab_id,
            pane_id,
        }),
        _ => Err(ControlError::new(
            ErrorCode::Internal,
            "pane metadata row is missing window/tab/pane ids",
        )),
    }
}

/// Finds the `pane.list` row whose `session_uuid` matches (case-insensitive
/// hex comparison).
fn find_pane_row_by_session_uuid<'a>(
    rows: &'a [serde_json::Value],
    session_uuid: &str,
) -> Option<&'a serde_json::Value> {
    rows.iter().find(|row| {
        row.get("session_uuid")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|uuid| uuid.eq_ignore_ascii_case(session_uuid))
    })
}

/// Sends one authenticated control request to the selected Warp instance.
/// Blocking — call through [`run_blocking`] from async context.
fn send_action<T: serde::Serialize>(
    action: ActionKind,
    params: T,
    target: TargetSelector,
) -> Result<serde_json::Value, ControlError> {
    let records = local_control::discovery::list_instances(&ChannelState::channel().to_string());
    let instance = select_instance(&records, &InstanceSelector::Active)?;
    let mut request = RequestEnvelope::new(Action::with_params(action, params)?);
    request.target = target;
    let response = local_control::client::send_request(&instance, &request)?;
    match response.response {
        local_control::ControlResponse::Ok { data } => Ok(data),
        local_control::ControlResponse::Error { error } => Err(error),
    }
}

/// Resolves the pane binding: by session UUID when one is known, otherwise
/// the active pane.
fn resolve_binding(session_uuid: Option<&str>) -> Result<PaneBinding, ControlError> {
    match session_uuid {
        Some(session_uuid) => {
            let data = send_action(
                ActionKind::PaneList,
                EmptyParams {},
                TargetSelector::default(),
            )?;
            let rows = data
                .get("panes")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            let row = find_pane_row_by_session_uuid(&rows, session_uuid).ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("no pane with terminal session uuid {session_uuid}"),
                )
            })?;
            binding_from_row(row)
        }
        None => {
            let data = send_action(
                ActionKind::PaneInspect,
                EmptyParams {},
                TargetSelector::default(),
            )?;
            let row = data.get("pane").cloned().ok_or_else(|| {
                ControlError::new(
                    ErrorCode::MissingTarget,
                    "could not resolve an active pane to bind to",
                )
            })?;
            binding_from_row(&row)
        }
    }
}

fn control_error(error: ControlError) -> ErrorData {
    ErrorData::internal_error(error.to_string(), serde_json::to_value(&error).ok())
}

fn text_result(data: &serde_json::Value) -> CallToolResult {
    let text = serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string());
    CallToolResult::success(vec![Content::text(text)])
}

#[derive(Clone)]
struct BrowserMcpServer {
    tool_router: ToolRouter<Self>,
    /// Session UUID from the CLI flag or environment; `None` falls back to
    /// the active pane at call time.
    bound_session_uuid: Option<String>,
}

impl BrowserMcpServer {
    fn new(bound_session_uuid: Option<String>) -> Self {
        Self {
            tool_router: Self::tool_router(),
            bound_session_uuid: bound_session_uuid.filter(|uuid| !uuid.is_empty()),
        }
    }

    /// Resolves the bound pane, preferring an explicit per-call session UUID.
    async fn binding(&self, explicit: Option<String>) -> Result<PaneBinding, ErrorData> {
        let session_uuid = explicit
            .filter(|uuid| !uuid.is_empty())
            .or_else(|| self.bound_session_uuid.clone());
        run_blocking(move || resolve_binding(session_uuid.as_deref())).await
    }
}

/// Runs a blocking local-control call off the async runtime.
async fn run_blocking<T, F>(work: F) -> Result<T, ErrorData>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ControlError> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| ErrorData::internal_error(format!("control task failed: {err}"), None))?
        .map_err(control_error)
}

#[tool_router]
impl BrowserMcpServer {
    #[tool(
        description = "Attach a browser underlay behind the bound pane's window and load a URL. \
                       By default the pane turns to translucent night glass so the page is \
                       visible behind its terminal output."
    )]
    async fn browser_attach(
        &self,
        Parameters(request): Parameters<AttachRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let binding = self.binding(None).await?;
        let params = BrowserAttachParams {
            url: request.url,
            glass: request.glass.unwrap_or(true),
            glass_opacity: request.glass_opacity,
        };
        let target = pane_target(&binding);
        let data =
            run_blocking(move || send_action(ActionKind::BrowserAttach, params, target)).await?;
        Ok(text_result(&data))
    }

    #[tool(description = "Navigate the attached browser underlay to a URL.")]
    async fn browser_navigate(
        &self,
        Parameters(request): Parameters<NavigateRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let binding = self.binding(None).await?;
        let params = UrlParams { url: request.url };
        let target = window_target(&binding);
        let data =
            run_blocking(move || send_action(ActionKind::BrowserNavigate, params, target)).await?;
        Ok(text_result(&data))
    }

    #[tool(
        description = "Evaluate JavaScript in the attached browser underlay's page and return \
                       the result (non-string results are JSON-encoded)."
    )]
    async fn browser_eval(
        &self,
        Parameters(request): Parameters<EvalRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let binding = self.binding(None).await?;
        let params = JavascriptParams {
            javascript: request.javascript,
        };
        let target = window_target(&binding);
        let data =
            run_blocking(move || send_action(ActionKind::BrowserEval, params, target)).await?;
        let result = row_str(&data, "result").unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[tool(description = "Capture a PNG screenshot of the attached browser underlay's page.")]
    async fn browser_screenshot(&self) -> Result<CallToolResult, ErrorData> {
        let binding = self.binding(None).await?;
        let target = window_target(&binding);
        let data = run_blocking(move || {
            send_action(ActionKind::BrowserScreenshot, EmptyParams {}, target)
        })
        .await?;
        let png_base64 = row_str(&data, "data_base64").ok_or_else(|| {
            ErrorData::internal_error("browser.screenshot response is missing PNG data", None)
        })?;
        Ok(CallToolResult::success(vec![Content::image(
            png_base64,
            "image/png",
        )]))
    }

    #[tool(
        description = "Enable or disable night glass on a pane (the bound pane by default), \
                       controlling whether the underlay page shows through it."
    )]
    async fn browser_set_glass(
        &self,
        Parameters(request): Parameters<SetGlassRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let binding = self.binding(request.pane).await?;
        let params = PaneGlassParams {
            enabled: request.enabled,
            opacity: request.opacity,
        };
        let target = pane_target(&binding);
        let data =
            run_blocking(move || send_action(ActionKind::PaneGlassSet, params, target)).await?;
        Ok(text_result(&data))
    }

    #[tool(
        description = "Toggle whether input events reach the underlay page. The underlay is \
                       inert by default; interactive mode is explicit and reversible."
    )]
    async fn browser_set_interactive(
        &self,
        Parameters(request): Parameters<SetInteractiveRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let binding = self.binding(None).await?;
        let params = BooleanValueParams {
            value: request.enabled,
        };
        let target = window_target(&binding);
        let data =
            run_blocking(move || send_action(ActionKind::BrowserInteractive, params, target))
                .await?;
        Ok(text_result(&data))
    }

    #[tool(
        description = "Show the browser underlay state for the bound pane's window: attached, \
                       url, interactive, glass panes, and glass opacity."
    )]
    async fn browser_status(&self) -> Result<CallToolResult, ErrorData> {
        let binding = self.binding(None).await?;
        let target = window_target(&binding);
        let data =
            run_blocking(move || send_action(ActionKind::BrowserStatus, EmptyParams {}, target))
                .await?;
        Ok(text_result(&data))
    }

    #[tool(
        description = "Detach the browser underlay from the bound pane's window, stopping any \
                       media playback and discarding the ephemeral browsing state."
    )]
    async fn browser_detach(&self) -> Result<CallToolResult, ErrorData> {
        let binding = self.binding(None).await?;
        let target = window_target(&binding);
        let data =
            run_blocking(move || send_action(ActionKind::BrowserDetach, EmptyParams {}, target))
                .await?;
        Ok(text_result(&data))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for BrowserMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Drives the browser underlay rendered behind the Warp terminal pane this server \
                 is bound to. Requires a running Warp instance with the browser-underlay feature \
                 and the allow_browser_control permission (Settings > Scripting) enabled.",
            );
        info.server_info.name = "warp-browser-mcp".to_owned();
        info.server_info.version = env!("CARGO_PKG_VERSION").to_owned();
        info
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let server = BrowserMcpServer::new(args.pane_session_uuid);
    let service = server.serve(rmcp::transport::io::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
