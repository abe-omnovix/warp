//! Handlers for the `browser.*` and `pane.glass.*` local-control actions.
//!
//! These drive the browser underlay (see `crate::browser_underlay`). Every
//! action in this family additionally requires the default-off
//! `allow_browser_control` permission, enforced in
//! `crate::local_control::permissions` before dispatch reaches this module.
use ::local_control::protocol::{
    BooleanValueParams, BrowserAttachParams, JavascriptParams, PaneGlassParams, UrlParams,
};
use ::local_control::{
    ActionKind, ControlError, ErrorCode, InstanceId, RequestEnvelope, ResponseEnvelope,
};
use base64::Engine as _;
use serde_json::json;
use warpui::{ModelContext, SingletonEntity as _, WindowId};

use crate::browser_underlay::{self, BrowserUnderlayState};
use crate::local_control::LocalControlBridge;
use crate::local_control::bridge::HandlerResponse;
use crate::local_control::handlers::ack;
use crate::local_control::resolver::{
    reject_target_families, target_pane_group, target_pane_id, target_window_id_for_target,
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

/// Resolves the target window and requires an attached underlay on it.
fn require_attached_window(
    action: ActionKind,
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<WindowId, ControlError> {
    reject_target_families(
        action,
        request.target.tab.is_some()
            || request.target.pane.is_some()
            || request.target.session.is_some(),
        "tab, pane, or session selectors",
    )?;
    let window_id = target_window_id_for_target(ctx, &request.target, action)?;
    if !BrowserUnderlayState::as_ref(ctx).is_attached(window_id) {
        return Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            format!(
                "{} requires a browser underlay attached to the target window",
                action.as_str()
            ),
        ));
    }
    Ok(window_id)
}

pub(crate) fn attach(
    instance_id: &Option<InstanceId>,
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    ensure_browser_underlay_available()?;
    let params = request.action.params_as::<BrowserAttachParams>()?;
    reject_target_families(
        ActionKind::BrowserAttach,
        request.target.session.is_some(),
        "session selectors",
    )?;
    let window_id = target_window_id_for_target(ctx, &request.target, ActionKind::BrowserAttach)?;
    // Resolve the glass pane before attaching so a selector error leaves the
    // window untouched. Default (no pane selector) is the active pane.
    let glass_pane = if params.glass {
        let pane_group = target_pane_group(ActionKind::BrowserAttach, &request.target, ctx)?;
        Some(target_pane_id(
            ActionKind::BrowserAttach,
            &request.target,
            &pane_group,
            ctx,
        )?)
    } else {
        None
    };
    if !browser_underlay::attach_to_window(window_id, &params.url, ctx) {
        return Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            "the target window cannot host a browser underlay",
        ));
    }
    if let Some(pane_id) = glass_pane {
        BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
            state.set_pane_glass(window_id, pane_id, true, params.glass_opacity, ctx);
        });
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
    let window_id = require_attached_window(ActionKind::BrowserNavigate, request, ctx)?;
    browser_underlay::navigate_window(window_id, &params.url, ctx);
    Ok(ack(instance_id, ActionKind::BrowserNavigate))
}

pub(crate) fn interactive(
    instance_id: &Option<InstanceId>,
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    ensure_browser_underlay_available()?;
    let params = request.action.params_as::<BooleanValueParams>()?;
    let window_id = require_attached_window(ActionKind::BrowserInteractive, request, ctx)?;
    ctx.windows()
        .set_background_webview_interactive(window_id, params.value);
    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.set_interactive(window_id, params.value, ctx);
    });
    Ok(ack(instance_id, ActionKind::BrowserInteractive))
}

pub(crate) fn status(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    ensure_browser_underlay_available()?;
    reject_target_families(
        ActionKind::BrowserStatus,
        request.target.tab.is_some()
            || request.target.pane.is_some()
            || request.target.session.is_some(),
        "tab, pane, or session selectors",
    )?;
    let window_id = target_window_id_for_target(ctx, &request.target, ActionKind::BrowserStatus)?;
    let Some(underlay) = BrowserUnderlayState::as_ref(ctx).attached(window_id) else {
        return Ok(json!({
            "action": ActionKind::BrowserStatus.as_str(),
            "window_id": window_id.to_string(),
            "attached": false,
        }));
    };
    let mut glass_panes = underlay
        .glass_panes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    glass_panes.sort();
    let url = underlay.url.clone();
    let interactive = underlay.interactive;
    Ok(json!({
        "action": ActionKind::BrowserStatus.as_str(),
        "window_id": window_id.to_string(),
        "attached": true,
        "url": url,
        "interactive": interactive,
        "glass_panes": glass_panes,
        "glass_opacity": browser_underlay::glass_fill_opacity(window_id, ctx),
    }))
}

pub(crate) fn detach(
    instance_id: &Option<InstanceId>,
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    ensure_browser_underlay_available()?;
    let window_id = require_attached_window(ActionKind::BrowserDetach, request, ctx)?;
    browser_underlay::detach_from_window(window_id, ctx);
    Ok(ack(instance_id, ActionKind::BrowserDetach))
}

pub(crate) fn pane_glass_set(
    instance_id: &Option<InstanceId>,
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    ensure_browser_underlay_available()?;
    let params = request.action.params_as::<PaneGlassParams>()?;
    reject_target_families(
        ActionKind::PaneGlassSet,
        request.target.session.is_some(),
        "session selectors",
    )?;
    let window_id = target_window_id_for_target(ctx, &request.target, ActionKind::PaneGlassSet)?;
    let pane_group = target_pane_group(ActionKind::PaneGlassSet, &request.target, ctx)?;
    let pane_id = target_pane_id(ActionKind::PaneGlassSet, &request.target, &pane_group, ctx)?;
    let applied = BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.set_pane_glass(window_id, pane_id, params.enabled, params.opacity, ctx)
    });
    if !applied {
        return Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            "pane.glass.set requires a browser underlay attached to the pane's window",
        ));
    }
    Ok(ack(instance_id, ActionKind::PaneGlassSet))
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
    let window_id = require_attached_window(ActionKind::BrowserEval, request, ctx)?;
    let request_id = request.request_id;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    ctx.windows().eval_background_webview_js(
        window_id,
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
    let window_id = require_attached_window(ActionKind::BrowserScreenshot, request, ctx)?;
    let request_id = request.request_id;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    ctx.windows().snapshot_background_webview(
        window_id,
        Box::new(move |result| {
            let envelope = match result {
                Ok(png_bytes) => ResponseEnvelope::ok(
                    request_id,
                    json!({
                        "action": ActionKind::BrowserScreenshot.as_str(),
                        "format": "png",
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
