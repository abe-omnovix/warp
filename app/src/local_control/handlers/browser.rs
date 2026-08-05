//! Handlers for the `browser.*` and `pane.glass.*` local-control actions.
//!
//! Two disjoint targets exist (see `crate::browser_underlay`):
//! - the window's **ambience** underlay (user-owned): reachable only with the
//!   explicit `ambience: true` parameter, which the human `warpctrl` surface
//!   sets and the agent-facing `/mcp` surface never does;
//! - **agent** underlays, keyed by the owning pane's terminal session UUID
//!   (`pane_session_uuid`, or resolved from the request's pane selector).
//!
//! Every action in this family additionally requires the default-off
//! `allow_browser_control` permission, enforced in
//! `crate::local_control::permissions` before dispatch reaches this module.
use ::local_control::protocol::{
    BrowserAttachParams, BrowserInteractiveParams, BrowserTargetParams, JavascriptParams,
    PaneGlassParams, UrlParams,
};
use ::local_control::{
    ActionKind, ControlError, ErrorCode, InstanceId, RequestEnvelope, ResponseEnvelope,
};
use base64::Engine as _;
use serde_json::json;
use warpui::platform::BrowserUnderlayOwner;
use warpui::{ModelContext, SingletonEntity as _, WindowId};

use crate::browser_underlay::{self, BrowserUnderlayState};
use crate::local_control::LocalControlBridge;
use crate::local_control::bridge::HandlerResponse;
use crate::local_control::handlers::ack;
use crate::local_control::resolver::{
    target_pane_group, target_pane_id, target_window_id_for_target,
};

fn ensure_browser_underlay_available() -> Result<(), ControlError> {
    if BrowserUnderlayState::feature_enabled() {
        return Ok(());
    }
    Err(ControlError::new(
        ErrorCode::UnsupportedAction,
        "the browser underlay feature is not enabled in this Warp instance",
    ))
}

/// A resolved routing target for a `browser.*` action.
enum BrowserTargetRef {
    /// The user-owned ambience underlay of a window.
    Ambience(WindowId),
    /// The agent underlay owned by this pane (terminal session UUID, hex).
    Agent(String),
}

impl BrowserTargetRef {
    /// The platform owner plus the window the underlay currently lives in.
    /// Errors when an agent target has no attached underlay.
    fn attached(
        &self,
        action: ActionKind,
        ctx: &ModelContext<LocalControlBridge>,
    ) -> Result<(WindowId, BrowserUnderlayOwner), ControlError> {
        match self {
            Self::Ambience(window_id) => {
                if !BrowserUnderlayState::as_ref(ctx).ambience_attached(*window_id) {
                    return Err(ControlError::new(
                        ErrorCode::TargetStateConflict,
                        format!(
                            "{} requires an ambience underlay attached to the target window",
                            action.as_str()
                        ),
                    ));
                }
                Ok((*window_id, BrowserUnderlayOwner::Ambience))
            }
            Self::Agent(pane_key) => {
                let window_id = BrowserUnderlayState::as_ref(ctx)
                    .agent(pane_key)
                    .map(|agent| agent.window_id)
                    .ok_or_else(|| {
                        ControlError::new(
                            ErrorCode::TargetStateConflict,
                            format!(
                                "{} requires an agent browser attached for pane {pane_key}",
                                action.as_str()
                            ),
                        )
                    })?;
                Ok((window_id, BrowserUnderlayOwner::Agent(pane_key.clone())))
            }
        }
    }
}

/// Resolves the routing target from the `ambience`/`pane_session_uuid` params
/// (falling back to the request's pane selector for agent targets).
fn resolve_target(
    action: ActionKind,
    ambience: bool,
    pane_session_uuid: Option<&str>,
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<BrowserTargetRef, ControlError> {
    if ambience {
        if pane_session_uuid.is_some() {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "ambience and pane_session_uuid are mutually exclusive",
            ));
        }
        let window_id = target_window_id_for_target(ctx, &request.target, action)?;
        return Ok(BrowserTargetRef::Ambience(window_id));
    }
    let pane_key = match pane_session_uuid {
        Some(uuid) if !uuid.is_empty() => uuid.to_owned(),
        _ => {
            // Resolve the pane from the target selector (active pane by
            // default) and map it to its persistent session UUID.
            let pane_group = target_pane_group(action, &request.target, ctx)?;
            let pane_id = target_pane_id(action, &request.target, &pane_group, ctx)?;
            pane_group
                .read(ctx, |pane_group, _| {
                    pane_group.terminal_session_uuid_hex(pane_id)
                })
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::MissingTarget,
                        format!(
                            "{} requires a terminal pane (the resolved pane has no session)",
                            action.as_str()
                        ),
                    )
                })?
        }
    };
    Ok(BrowserTargetRef::Agent(pane_key))
}

pub(crate) fn attach(
    instance_id: &Option<InstanceId>,
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    ensure_browser_underlay_available()?;
    let params = request.action.params_as::<BrowserAttachParams>()?;
    let target = resolve_target(
        ActionKind::BrowserAttach,
        params.ambience,
        params.pane_session_uuid.as_deref(),
        request,
        ctx,
    )?;
    match target {
        BrowserTargetRef::Ambience(window_id) => {
            if !browser_underlay::attach_ambience(window_id, &params.url, ctx) {
                return Err(ControlError::new(
                    ErrorCode::TargetStateConflict,
                    "the target window cannot host a browser underlay",
                ));
            }
        }
        BrowserTargetRef::Agent(pane_key) => {
            browser_underlay::agent_attach(&pane_key, &params.url, ctx)
                .map_err(|error| ControlError::new(ErrorCode::TargetStateConflict, error))?;
            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_agent_glass(&pane_key, params.glass, params.glass_opacity, ctx);
            });
        }
    }
    Ok(ack(instance_id, ActionKind::BrowserAttach))
}

pub(crate) fn navigate(
    instance_id: &Option<InstanceId>,
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    ensure_browser_underlay_available()?;
    let params = request.action.params_as::<UrlParams>()?;
    let target = resolve_target(
        ActionKind::BrowserNavigate,
        params.ambience,
        params.pane_session_uuid.as_deref(),
        request,
        ctx,
    )?;
    match target {
        BrowserTargetRef::Ambience(window_id) => {
            if !browser_underlay::navigate_ambience(window_id, &params.url, ctx) {
                return Err(ControlError::new(
                    ErrorCode::TargetStateConflict,
                    "no ambience underlay attached to the target window",
                ));
            }
        }
        BrowserTargetRef::Agent(pane_key) => {
            browser_underlay::agent_navigate(&pane_key, &params.url, ctx)
                .map_err(|error| ControlError::new(ErrorCode::TargetStateConflict, error))?;
        }
    }
    Ok(ack(instance_id, ActionKind::BrowserNavigate))
}

pub(crate) fn interactive(
    instance_id: &Option<InstanceId>,
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    ensure_browser_underlay_available()?;
    let params = request.action.params_as::<BrowserInteractiveParams>()?;
    let target = resolve_target(
        ActionKind::BrowserInteractive,
        params.ambience,
        params.pane_session_uuid.as_deref(),
        request,
        ctx,
    )?;
    let (window_id, owner) = target.attached(ActionKind::BrowserInteractive, ctx)?;
    // The hold hotkey may be physically held right now, so the state model
    // decides the effective mode that reaches the webview.
    let effective = BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.set_interactive(window_id, &owner, params.value, ctx)
    });
    if let Some(effective) = effective {
        ctx.windows()
            .set_background_webview_interactive(window_id, &owner, effective);
    }
    Ok(ack(instance_id, ActionKind::BrowserInteractive))
}

pub(crate) fn status(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    ensure_browser_underlay_available()?;
    let params = request.action.params_as::<BrowserTargetParams>()?;
    let target = resolve_target(
        ActionKind::BrowserStatus,
        params.ambience,
        params.pane_session_uuid.as_deref(),
        request,
        ctx,
    )?;
    let state = BrowserUnderlayState::as_ref(ctx);
    let data = match &target {
        BrowserTargetRef::Ambience(window_id) => match state.ambience(*window_id) {
            Some(ambience) => json!({
                "action": ActionKind::BrowserStatus.as_str(),
                "target": "ambience",
                "window_id": window_id.to_string(),
                "attached": true,
                "url": ambience.url,
                "interactive": ambience.interactive || ambience.interactive_hold,
            }),
            None => json!({
                "action": ActionKind::BrowserStatus.as_str(),
                "target": "ambience",
                "window_id": window_id.to_string(),
                "attached": false,
            }),
        },
        BrowserTargetRef::Agent(pane_key) => match state.agent(pane_key) {
            Some(agent) => json!({
                "action": ActionKind::BrowserStatus.as_str(),
                "target": "agent",
                "pane_session_uuid": pane_key,
                "window_id": agent.window_id.to_string(),
                "attached": true,
                "url": agent.url,
                "visible": agent.visible,
                "glass": agent.glass,
                "glass_opacity": browser_underlay::glass_fill_opacity(agent.window_id, ctx),
                "interactive": agent.interactive || agent.interactive_hold,
            }),
            None => json!({
                "action": ActionKind::BrowserStatus.as_str(),
                "target": "agent",
                "pane_session_uuid": pane_key,
                "attached": false,
            }),
        },
    };
    Ok(data)
}

pub(crate) fn detach(
    instance_id: &Option<InstanceId>,
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    ensure_browser_underlay_available()?;
    let params = request.action.params_as::<BrowserTargetParams>()?;
    let target = resolve_target(
        ActionKind::BrowserDetach,
        params.ambience,
        params.pane_session_uuid.as_deref(),
        request,
        ctx,
    )?;
    match target {
        BrowserTargetRef::Ambience(window_id) => browser_underlay::detach_ambience(window_id, ctx),
        BrowserTargetRef::Agent(pane_key) => browser_underlay::agent_detach(&pane_key, ctx),
    }
    Ok(ack(instance_id, ActionKind::BrowserDetach))
}

pub(crate) fn pane_glass_set(
    instance_id: &Option<InstanceId>,
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    ensure_browser_underlay_available()?;
    let params = request.action.params_as::<PaneGlassParams>()?;
    let BrowserTargetRef::Agent(pane_key) = resolve_target(
        ActionKind::PaneGlassSet,
        false,
        params.pane_session_uuid.as_deref(),
        request,
        ctx,
    )?
    else {
        unreachable!("pane.glass.set always resolves an agent target");
    };
    let applied = BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.set_agent_glass(&pane_key, params.enabled, params.opacity, ctx)
    });
    if !applied {
        return Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            format!("no agent browser attached for pane {pane_key}"),
        ));
    }
    Ok(ack(instance_id, ActionKind::PaneGlassSet))
}

/// `browser.mcp.env` — the Warp-hosted MCP endpoint URL and bearer token.
///
/// Normally these reach agents through pane-shell environment variables; this
/// action covers shells that did not inherit them (tmux, ssh, editors).
/// Revealing the token here grants no capability the caller lacks: it is
/// broker-authenticated (same-UID) and gated on `allow_browser_control`, and
/// such a caller can already perform every browser action directly.
pub(crate) fn mcp_env(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    ensure_browser_underlay_available()?;
    let env = ctx
        .has_singleton_model::<crate::local_control::LocalControlServer>()
        .then(|| {
            crate::local_control::LocalControlServer::as_ref(ctx)
                .mcp_env
                .clone()
        })
        .flatten();
    let Some((url, token)) = env else {
        return Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            "the MCP endpoint is not running (enable Settings › Scripting)",
        ));
    };
    Ok(json!({
        "action": ActionKind::BrowserMcpEnv.as_str(),
        "url": url,
        "token": token.secret(),
    }))
}

pub(crate) fn eval(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> HandlerResponse {
    let request_id = request.request_id;
    match eval_pending(request, ctx) {
        Ok(receiver) => HandlerResponse::Pending(receiver),
        Err(error) => HandlerResponse::Ready(ResponseEnvelope::error(request_id, error)),
    }
}

fn eval_pending(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<tokio::sync::oneshot::Receiver<ResponseEnvelope>, ControlError> {
    ensure_browser_underlay_available()?;
    let params = request.action.params_as::<JavascriptParams>()?;
    let target = resolve_target(
        ActionKind::BrowserEval,
        params.ambience,
        params.pane_session_uuid.as_deref(),
        request,
        ctx,
    )?;
    let (window_id, owner) = target.attached(ActionKind::BrowserEval, ctx)?;
    if let BrowserUnderlayOwner::Agent(pane_key) = &owner {
        BrowserUnderlayState::handle(ctx).update(ctx, |state, _| {
            state.touch_agent(pane_key, None);
        });
    }
    let request_id = request.request_id;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    ctx.windows().eval_background_webview_js(
        window_id,
        &owner,
        &params.javascript,
        Box::new(move |result| {
            let envelope = match result {
                Ok(value) => ResponseEnvelope::ok(
                    request_id,
                    json!({
                        "action": ActionKind::BrowserEval.as_str(),
                        "result": value,
                    }),
                ),
                Err(error) => ResponseEnvelope::error(
                    request_id,
                    ControlError::with_details(
                        ErrorCode::Internal,
                        "browser.eval failed in the webview",
                        error,
                    ),
                ),
            };
            let _ = sender.send(envelope);
        }),
    );
    Ok(receiver)
}

pub(crate) fn screenshot(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> HandlerResponse {
    let request_id = request.request_id;
    match screenshot_pending(request, ctx) {
        Ok(receiver) => HandlerResponse::Pending(receiver),
        Err(error) => HandlerResponse::Ready(ResponseEnvelope::error(request_id, error)),
    }
}

fn screenshot_pending(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<tokio::sync::oneshot::Receiver<ResponseEnvelope>, ControlError> {
    ensure_browser_underlay_available()?;
    let params = request.action.params_as::<BrowserTargetParams>()?;
    let target = resolve_target(
        ActionKind::BrowserScreenshot,
        params.ambience,
        params.pane_session_uuid.as_deref(),
        request,
        ctx,
    )?;
    let (window_id, owner) = target.attached(ActionKind::BrowserScreenshot, ctx)?;
    // Hidden webviews (agent tab not frontmost) may render lazily; annotate
    // so callers can tell a possibly-stale capture apart.
    let tab_visible = match &owner {
        BrowserUnderlayOwner::Ambience => true,
        BrowserUnderlayOwner::Agent(pane_key) => {
            let state = BrowserUnderlayState::handle(ctx);
            state.update(ctx, |state, _| {
                state.touch_agent(pane_key, None);
            });
            state
                .as_ref(ctx)
                .agent(pane_key)
                .is_some_and(|agent| agent.visible)
        }
    };
    let request_id = request.request_id;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    ctx.windows().snapshot_background_webview(
        window_id,
        &owner,
        Box::new(move |result| {
            let envelope = match result {
                Ok(png_bytes) => ResponseEnvelope::ok(
                    request_id,
                    json!({
                        "action": ActionKind::BrowserScreenshot.as_str(),
                        "format": "png",
                        "tab_visible": tab_visible,
                        "data_base64":
                            base64::engine::general_purpose::STANDARD.encode(png_bytes),
                    }),
                ),
                Err(error) => ResponseEnvelope::error(
                    request_id,
                    ControlError::with_details(
                        ErrorCode::Internal,
                        "browser.screenshot failed in the webview",
                        error,
                    ),
                ),
            };
            let _ = sender.send(envelope);
        }),
    );
    Ok(receiver)
}
