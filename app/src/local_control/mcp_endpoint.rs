//! Warp-hosted MCP endpoint: `POST /mcp` on the local-control server.
//!
//! Implements the stateless Streamable HTTP shape from MCP revision
//! 2026-07-28 — a single POST endpoint, no protocol sessions, every request
//! self-contained — while still accepting the legacy `initialize` handshake
//! from clients speaking 2025-era revisions (no session id is ever minted).
//!
//! The endpoint exposes the agent browser tools. Binding is explicit per the
//! spec's handle pattern: every tool takes an optional `pane` argument
//! (annotated `x-mcp-header`, mirrored as `Mcp-Param-Pane`), falling back to
//! the client-configured `X-Warp-Pane` header (typically interpolated from
//! the pane-injected `WARP_TERMINAL_SESSION_UUID`). There is deliberately no
//! active-pane fallback and no route to the user's ambience underlay: an
//! agent's browser is always scoped to its own pane.
//!
//! Auth: loopback bind + Origin rejection (shared with the control server)
//! plus a per-launch bearer token that Warp injects into pane shells as
//! `WARP_MCP_TOKEN` (alongside `WARP_MCP_URL`). Every action additionally
//! passes the full local-control permission gate, including the default-off
//! `allow_browser_control` setting.
use ::local_control::auth::CredentialGrant;
use ::local_control::protocol::{
    BrowserAttachParams, BrowserInteractiveParams, BrowserTargetParams, JavascriptParams,
    PaneGlassParams, UrlParams,
};
use ::local_control::{Action, ActionKind, ControlResponse, RequestEnvelope};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use serde_json::{Value, json};

use super::{ASYNC_ACTION_TIMEOUT, ControlServerState, bridge, validate_loopback_headers};

/// JSON-RPC error codes used by this endpoint.
const PARSE_ERROR: i64 = -32700;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

pub(super) async fn handle_mcp_request(
    State(state): State<ControlServerState>,
    headers: HeaderMap,
    payload: Bytes,
) -> Response {
    // Same browser-origin/endpoint hardening as the control endpoint.
    if let Err(error) = validate_loopback_headers(&headers, &state.expected_host) {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": error }))).into_response();
    }
    // Per-launch bearer token, injected into pane shells as WARP_MCP_TOKEN.
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if state
        .mcp_token
        .verify_authorization_header(auth_header)
        .is_err()
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing or invalid bearer token (WARP_MCP_TOKEN)" })),
        )
            .into_response();
    }

    let Ok(message) = serde_json::from_slice::<Value>(&payload) else {
        return rpc_error(Value::Null, PARSE_ERROR, "request body is not valid JSON");
    };
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    // Notifications (no id) are acknowledged with 202 per Streamable HTTP.
    if message.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }

    match method.as_str() {
        // Legacy (pre-2026-07-28) handshake: answer statically, mint no
        // session. Modern clients skip this entirely.
        "initialize" => {
            let requested = message
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2025-06-18");
            rpc_result(
                id,
                json!({
                    "protocolVersion": requested,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "warp-browser",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "instructions": "Drives an agent-scoped browser rendered behind the Warp \
                        terminal pane the agent runs in (visible only while that pane's tab is \
                        frontmost). Requires the allow_browser_control permission (Settings > \
                        Scripting). Pane binding: pass the `pane` argument (terminal session \
                        UUID) or configure the X-Warp-Pane header from \
                        $WARP_TERMINAL_SESSION_UUID.",
                }),
            )
        }
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, json!({ "tools": tool_definitions() })),
        "tools/call" => handle_tool_call(&state, &headers, id, &message).await,
        _ => rpc_error(id, METHOD_NOT_FOUND, &format!("method not found: {method}")),
    }
}

async fn handle_tool_call(
    state: &ControlServerState,
    headers: &HeaderMap,
    id: Value,
    message: &Value,
) -> Response {
    let tool = message
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let args = message
        .pointer("/params/arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    // Pane binding: explicit `pane` argument, else the client-configured
    // X-Warp-Pane header. Never the active pane, never the ambience.
    let pane = args
        .get("pane")
        .and_then(Value::as_str)
        .filter(|pane| !pane.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            headers
                .get("x-warp-pane")
                .and_then(|value| value.to_str().ok())
                .filter(|pane| !pane.is_empty())
                .map(str::to_owned)
        });
    let Some(pane) = pane else {
        return tool_error(
            id,
            "no pane binding: pass the `pane` argument (terminal session UUID) or configure \
             the X-Warp-Pane header from $WARP_TERMINAL_SESSION_UUID",
        );
    };

    let (action, params) = match build_action(&tool, &args, &pane) {
        Ok(pair) => pair,
        Err(error) => return rpc_error(id, INVALID_PARAMS, &error),
    };

    // Reuse the full control-plane path: the bridge revalidates settings
    // (including allow_browser_control) before dispatching, exactly as it
    // does for warpctrl requests. The grant is minted in-process — the
    // bearer token above already authenticated the caller.
    let grant = CredentialGrant::new(
        state.instance_id.clone(),
        action,
        chrono::Duration::minutes(1),
    );
    let request = match Action::with_params(action, params) {
        Ok(action) => RequestEnvelope::new(action),
        Err(error) => return rpc_error(id, INVALID_PARAMS, &error.to_string()),
    };
    let request_id = request.request_id;
    let response = match state
        .bridge_spawner
        .spawn(move |bridge, ctx| bridge.handle_request(request, grant, ctx))
        .await
    {
        Ok(bridge::HandlerResponse::Ready(response)) => response,
        Ok(bridge::HandlerResponse::Pending(receiver)) => {
            match tokio::time::timeout(ASYNC_ACTION_TIMEOUT, receiver).await {
                Ok(Ok(response)) => response,
                Ok(Err(_)) => return rpc_error(id, INTERNAL_ERROR, "action dropped"),
                Err(_) => return tool_error(id, "the browser action timed out"),
            }
        }
        Err(_) => return rpc_error(id, INTERNAL_ERROR, "app bridge unavailable"),
    };
    let _ = request_id;

    match response.response {
        ControlResponse::Ok { data } => rpc_result(id, tool_success_content(&tool, data)),
        ControlResponse::Error { error } => tool_error(id, &error.to_string()),
    }
}

/// Maps a tool call onto the corresponding control-plane action. `ambience`
/// is always false: agents cannot reach the user's ambience underlay.
fn build_action(tool: &str, args: &Value, pane: &str) -> Result<(ActionKind, Value), String> {
    let pane = Some(pane.to_owned());
    let str_arg = |key: &str| -> Result<String, String> {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("missing required argument `{key}`"))
    };
    let params = match tool {
        "browser_attach" => serde_json::to_value(BrowserAttachParams {
            url: str_arg("url")?,
            glass: args.get("glass").and_then(Value::as_bool).unwrap_or(true),
            glass_opacity: args
                .get("glass_opacity")
                .and_then(Value::as_u64)
                .map(|v| v.min(100) as u8),
            ambience: false,
            pane_session_uuid: pane,
        }),
        "browser_navigate" => serde_json::to_value(UrlParams {
            url: str_arg("url")?,
            ambience: false,
            pane_session_uuid: pane,
        }),
        "browser_eval" => serde_json::to_value(JavascriptParams {
            javascript: str_arg("javascript")?,
            ambience: false,
            pane_session_uuid: pane,
        }),
        "browser_screenshot" | "browser_status" | "browser_detach" => {
            serde_json::to_value(BrowserTargetParams {
                ambience: false,
                pane_session_uuid: pane,
            })
        }
        "browser_set_interactive" => serde_json::to_value(BrowserInteractiveParams {
            value: args
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or("missing required argument `enabled`")?,
            ambience: false,
            pane_session_uuid: pane,
        }),
        "browser_set_glass" => serde_json::to_value(PaneGlassParams {
            enabled: args
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or("missing required argument `enabled`")?,
            opacity: args
                .get("opacity")
                .and_then(Value::as_u64)
                .map(|v| v.min(100) as u8),
            pane_session_uuid: pane,
        }),
        _ => return Err(format!("unknown tool: {tool}")),
    }
    .map_err(|error| error.to_string())?;
    let action = match tool {
        "browser_attach" => ActionKind::BrowserAttach,
        "browser_navigate" => ActionKind::BrowserNavigate,
        "browser_eval" => ActionKind::BrowserEval,
        "browser_screenshot" => ActionKind::BrowserScreenshot,
        "browser_status" => ActionKind::BrowserStatus,
        "browser_detach" => ActionKind::BrowserDetach,
        "browser_set_interactive" => ActionKind::BrowserInteractive,
        "browser_set_glass" => ActionKind::PaneGlassSet,
        _ => unreachable!(),
    };
    Ok((action, params))
}

/// Converts a control-plane success payload into MCP tool-result content.
fn tool_success_content(tool: &str, data: Value) -> Value {
    if tool == "browser_screenshot" {
        if let Some(png_base64) = data.get("data_base64").and_then(Value::as_str) {
            let mut content = vec![json!({
                "type": "image",
                "data": png_base64,
                "mimeType": "image/png",
            })];
            if data.get("tab_visible") == Some(&Value::Bool(false)) {
                content.push(json!({
                    "type": "text",
                    "text": "note: the agent's tab is not currently visible; the capture may \
                             be stale (WebKit throttles hidden webviews)",
                }));
            }
            return json!({ "content": content });
        }
    }
    if tool == "browser_eval" {
        let text = data
            .get("result")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| data.to_string());
        return json!({ "content": [{ "type": "text", "text": text }] });
    }
    let text = serde_json::to_string_pretty(&data).unwrap_or_else(|_| data.to_string());
    json!({ "content": [{ "type": "text", "text": text }] })
}

/// The agent browser tool catalog. The `pane` parameter carries the
/// `x-mcp-header` annotation (2026-07-28) so conforming clients mirror it
/// into an `Mcp-Param-Pane` header for routing.
fn tool_definitions() -> Value {
    let pane_property = json!({
        "type": "string",
        "description": "Terminal session UUID of the pane this browser belongs to. Defaults \
                        to the X-Warp-Pane header configured from $WARP_TERMINAL_SESSION_UUID.",
        "x-mcp-header": "Pane",
    });
    json!([
        {
            "name": "browser_attach",
            "description": "Attach a browser behind this agent's pane and load a URL. The pane \
                            turns to translucent night glass so the page is visible behind its \
                            terminal output — but only while this pane's tab is frontmost; the \
                            browser is invisible from other tabs and windows.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to load" },
                    "glass": { "type": "boolean", "description": "Night-glass porthole on the pane (default true)" },
                    "glass_opacity": { "type": "integer", "description": "Glass opacity override, 0-100" },
                    "pane": pane_property,
                },
                "required": ["url"],
            },
        },
        {
            "name": "browser_navigate",
            "description": "Navigate this agent's browser to a URL.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string" },
                    "pane": pane_property,
                },
                "required": ["url"],
            },
        },
        {
            "name": "browser_eval",
            "description": "Evaluate JavaScript in this agent's browser page and return the \
                            result (non-strings JSON-encoded).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "javascript": { "type": "string" },
                    "pane": pane_property,
                },
                "required": ["javascript"],
            },
        },
        {
            "name": "browser_screenshot",
            "description": "Capture a PNG of this agent's browser page (never the terminal). \
                            If the agent's tab is not frontmost the capture may be stale.",
            "inputSchema": {
                "type": "object",
                "properties": { "pane": pane_property },
            },
        },
        {
            "name": "browser_set_glass",
            "description": "Toggle the night-glass porthole on this agent's pane, or adjust \
                            its opacity.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" },
                    "opacity": { "type": "integer", "description": "0-100" },
                    "pane": pane_property,
                },
                "required": ["enabled"],
            },
        },
        {
            "name": "browser_set_interactive",
            "description": "Toggle whether the user's input reaches this agent's browser page. \
                            Inert by default; the user can also toggle it with the CapsLock \
                            gesture while the agent's tab is frontmost.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" },
                    "pane": pane_property,
                },
                "required": ["enabled"],
            },
        },
        {
            "name": "browser_status",
            "description": "Show this agent's browser state: attached, url, visible, glass, \
                            interactive.",
            "inputSchema": {
                "type": "object",
                "properties": { "pane": pane_property },
            },
        },
        {
            "name": "browser_detach",
            "description": "Detach this agent's browser, stopping playback and discarding its \
                            ephemeral browsing state.",
            "inputSchema": {
                "type": "object",
                "properties": { "pane": pane_property },
            },
        },
    ])
}

fn rpc_result(id: Value, result: Value) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

fn rpc_error(id: Value, code: i64, message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        })),
    )
        .into_response()
}

/// A tool-level failure: a successful JSON-RPC response whose result is
/// flagged `isError`, so the model sees the message.
fn tool_error(id: Value, message: &str) -> Response {
    rpc_result(
        id,
        json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true,
        }),
    )
}

#[cfg(test)]
#[path = "mcp_endpoint_tests.rs"]
mod tests;
