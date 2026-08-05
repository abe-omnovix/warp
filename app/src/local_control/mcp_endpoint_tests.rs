use serde_json::{Value, json};

use super::*;

const ALL_TOOLS: &[&str] = &[
    "browser_attach",
    "browser_navigate",
    "browser_eval",
    "browser_screenshot",
    "browser_set_glass",
    "browser_set_interactive",
    "browser_status",
    "browser_detach",
];

fn args_for(tool: &str) -> Value {
    match tool {
        "browser_attach" | "browser_navigate" => json!({ "url": "https://example.com" }),
        "browser_eval" => json!({ "javascript": "document.title" }),
        "browser_set_glass" | "browser_set_interactive" => json!({ "enabled": true }),
        _ => json!({}),
    }
}

#[test]
fn every_tool_maps_to_a_pane_scoped_action() {
    for tool in ALL_TOOLS {
        let (action, params) =
            build_action(tool, &args_for(tool), "abc123").unwrap_or_else(|error| {
                panic!("tool {tool} failed to map: {error}");
            });
        // Agents may never reach the user's ambience underlay, and the pane
        // binding must survive into the dispatched params. (browser_set_glass
        // maps to PaneGlassParams, which has no ambience route at all —
        // covered separately below.)
        if *tool != "browser_set_glass" {
            assert_eq!(params.get("ambience"), Some(&json!(false)), "{tool}");
        }
        assert_eq!(
            params.get("pane_session_uuid"),
            Some(&json!("abc123")),
            "{tool}"
        );
        let expected = match *tool {
            "browser_attach" => ActionKind::BrowserAttach,
            "browser_navigate" => ActionKind::BrowserNavigate,
            "browser_eval" => ActionKind::BrowserEval,
            "browser_screenshot" => ActionKind::BrowserScreenshot,
            "browser_set_glass" => ActionKind::PaneGlassSet,
            "browser_set_interactive" => ActionKind::BrowserInteractive,
            "browser_status" => ActionKind::BrowserStatus,
            "browser_detach" => ActionKind::BrowserDetach,
            _ => unreachable!(),
        };
        assert_eq!(action, expected, "{tool}");
    }
}

#[test]
fn pane_glass_has_no_ambience_route() {
    // PaneGlassParams has no ambience field at all; the serialized params
    // must not smuggle one in.
    let (_, params) = build_action("browser_set_glass", &json!({ "enabled": false }), "abc123")
        .expect("glass maps");
    assert_eq!(params.get("ambience"), None);
    assert_eq!(params.get("pane_session_uuid"), Some(&json!("abc123")));
}

#[test]
fn missing_required_arguments_are_rejected() {
    for (tool, missing) in [
        ("browser_attach", "url"),
        ("browser_navigate", "url"),
        ("browser_eval", "javascript"),
        ("browser_set_glass", "enabled"),
        ("browser_set_interactive", "enabled"),
    ] {
        let error = build_action(tool, &json!({}), "abc123")
            .expect_err("missing required argument must fail");
        assert!(error.contains(missing), "{tool}: {error}");
    }
}

#[test]
fn unknown_tools_are_rejected() {
    let error = build_action("browser_launch_missiles", &json!({}), "abc123")
        .expect_err("unknown tool must fail");
    assert!(error.contains("unknown tool"), "{error}");
}

#[test]
fn attach_defaults_glass_on_and_clamps_opacity() {
    let (_, params) = build_action(
        "browser_attach",
        &json!({ "url": "https://example.com", "glass_opacity": 400 }),
        "abc123",
    )
    .expect("attach maps");
    assert_eq!(params.get("glass"), Some(&json!(true)));
    assert_eq!(params.get("glass_opacity"), Some(&json!(100)));
}

#[test]
fn tool_catalog_annotates_pane_with_mcp_header() {
    let tools = tool_definitions();
    let tools = tools.as_array().expect("tool list is an array");
    assert_eq!(tools.len(), ALL_TOOLS.len());
    for tool in tools {
        let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
        assert!(ALL_TOOLS.contains(&name), "unexpected tool {name}");
        // Every tool routes by pane, so every schema must expose the header
        // annotation clients use to mirror the binding (2026-07-28
        // `x-mcp-header`).
        assert_eq!(
            tool.pointer("/inputSchema/properties/pane/x-mcp-header"),
            Some(&json!("Pane")),
            "{name}"
        );
    }
}

#[test]
fn screenshot_success_becomes_image_content_with_visibility_note() {
    let data = json!({ "data_base64": "cGln", "format": "png", "tab_visible": false });
    let result = tool_success_content("browser_screenshot", data);
    assert_eq!(result.pointer("/content/0/type"), Some(&json!("image")));
    assert_eq!(result.pointer("/content/0/data"), Some(&json!("cGln")));
    let note = result
        .pointer("/content/1/text")
        .and_then(Value::as_str)
        .expect("hidden-tab capture carries a staleness note");
    assert!(note.contains("not currently visible"));

    let visible = json!({ "data_base64": "cGln", "tab_visible": true });
    let result = tool_success_content("browser_screenshot", visible);
    assert!(result.pointer("/content/1").is_none());
}

#[test]
fn eval_success_returns_raw_result_text() {
    let result = tool_success_content("browser_eval", json!({ "result": "Example Domain" }));
    assert_eq!(
        result.pointer("/content/0/text"),
        Some(&json!("Example Domain"))
    );
}
