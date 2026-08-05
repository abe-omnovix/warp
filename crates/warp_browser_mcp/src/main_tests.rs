use serde_json::json;

use super::*;

fn pane_row(session_uuid: Option<&str>) -> serde_json::Value {
    json!({
        "pane_id": "pane_1",
        "tab_id": "tab_1",
        "window_id": "window_1",
        "session_uuid": session_uuid,
    })
}

#[test]
fn finds_pane_row_by_session_uuid_case_insensitively() {
    let rows = vec![pane_row(None), pane_row(Some("ABCDEF0123"))];

    let row = find_pane_row_by_session_uuid(&rows, "abcdef0123")
        .expect("hex uuid should match case-insensitively");

    assert_eq!(row.get("pane_id").and_then(|v| v.as_str()), Some("pane_1"));
}

#[test]
fn returns_none_for_unknown_session_uuid() {
    let rows = vec![pane_row(Some("abcdef0123"))];

    assert!(find_pane_row_by_session_uuid(&rows, "0000000000").is_none());
}

#[test]
fn non_terminal_panes_with_null_uuid_never_match() {
    let rows = vec![pane_row(None)];

    assert!(find_pane_row_by_session_uuid(&rows, "abcdef0123").is_none());
}

#[test]
fn binding_from_row_reads_ids() {
    let binding = binding_from_row(&pane_row(Some("abc"))).expect("row has all ids");

    assert_eq!(binding.window_id, "window_1");
    assert_eq!(binding.tab_id, "tab_1");
    assert_eq!(binding.pane_id, "pane_1");
}

#[test]
fn binding_from_row_rejects_incomplete_rows() {
    let err = binding_from_row(&json!({ "pane_id": "pane_1" }))
        .expect_err("missing window/tab ids are rejected");

    assert_eq!(err.code, ErrorCode::Internal);
}

#[test]
fn window_target_selects_only_the_window() {
    let binding = binding_from_row(&pane_row(None)).expect("row has all ids");

    let target = window_target(&binding);

    assert_eq!(
        target.window,
        Some(WindowTarget::Id {
            id: WindowSelector("window_1".to_owned())
        })
    );
    assert_eq!(target.tab, None);
    assert_eq!(target.pane, None);
    assert_eq!(target.session, None);
}

#[test]
fn pane_target_selects_window_tab_and_pane() {
    let binding = binding_from_row(&pane_row(None)).expect("row has all ids");

    let target = pane_target(&binding);

    assert_eq!(
        target.window,
        Some(WindowTarget::Id {
            id: WindowSelector("window_1".to_owned())
        })
    );
    assert_eq!(
        target.tab,
        Some(TabTarget::Id {
            id: TabSelector("tab_1".to_owned())
        })
    );
    assert_eq!(
        target.pane,
        Some(PaneTarget::Id {
            id: PaneSelector("pane_1".to_owned())
        })
    );
    assert_eq!(target.session, None);
}
