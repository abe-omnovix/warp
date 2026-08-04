# Browser underlay — session handoff

State as of commit `ffdf295c` on `abe/browser-underlay` (fork `abe-omnovix/warp`,
upstream `warpdotdev/warp`, base master `af2f7c61`).

## Standing instruction from Abe

Implement **Phases 1–4 without stopping** (Phase 5 is documented-only). Phase 1
is code-complete and committed; its *runtime* verification is parked on the
macOS Metal toolchain (below). Continue with Phases 2, 3, 4 per `TECH.md`,
committing per phase.

## Decisions already settled (do not re-litigate)

See `TECH.md` "Decisions" + `PRODUCT.md`. In brief: system WKWebView via ObjC
shim (no wry); contentView container restructure (Route A); v1 window-level
webview with per-pane glass; interactive toggle ships in v1; the
`allow_browser_control` permission defaults **off**; settings-page ambience
URL + glass opacity are **in scope**; Warp-agent auto-bind (4b) and animated
image backgrounds (Phase 0) are **out**; keep the diff surgical and
feature-flagged (upstreamable); Claude implements directly using the repo
skills (`add-feature-flag` done; use `gui-settings-ui`, `gui-ui-guidelines`,
`rust-unit-tests`, `gui-integration-test`, `logging-and-error-reporting`).

## What Phase 1 shipped (commit ffdf295c)

- `crates/warpui/src/platform/mac/objc/browser_underlay.{h,m}` — inert-by-default
  WKWebView underlay + C API (attach/navigate/eval/snapshot/interactive/detach).
- `objc/window.m` — contentView is now a plain container (host view full-size
  subview); all former `contentView == WarpHostView` sites use
  `warp_host_view_for_window` (declared in `host_view.h`). Panels unchanged.
- `crates/warpui/src/platform/mac/window.rs` — FFI decls, trampolines,
  `Window::*_background_webview` statics; host-view lookups for Device/
  metal_layer/IME.
- `crates/warpui_core/src/platform/mod.rs` — `WindowManager` trait methods with
  default no-ops + `BrowserJsEvalCallback`/`BrowserSnapshotCallback` types;
  wrappers in `windowing/state.rs`.
- `app/src/browser_underlay.rs` — `BrowserUnderlayState` singleton (registered
  in `app/src/lib.rs` next to `GPUState`), attach/detach helpers,
  `WARP_BROWSER_UNDERLAY_TEST_URL` hook in
  `root_view.rs::open_new_with_workspace_source`.
- `app/src/workspace/{view,util}.rs` — glass path when underlay attached
  (interim window-level opacity 85; Phase 2 replaces with per-pane).
- Flag: `FeatureFlag::BrowserUnderlay` (dogfood, macOS) + `browser_underlay`
  cargo feature (`app/Cargo.toml`, mapped in `app/src/features.rs`).

## Verification environment split

- **Linux (cloud runners): full `cargo check -p warp` works** — the mac module
  is target-gated out, trait defaults cover the app-side calls, and the Metal
  shader build only runs when targeting macOS. Type-check every phase here.
- **macOS (Abe's machine): currently BLOCKED for GUI builds** — `xcrun -f
  metal` fails (no Xcode.app, MetalToolchain asset absent; repo fix is
  `script/macos/install_build_deps`, which needs Xcode). ObjC changes were
  verified with `clang -fsyntax-only -Wall`. The integration suite and the
  video spike (muted YouTube embed via the env hook; watch for WebKit
  occlusion pausing → fallback plan B child-NSWindow, see TECH.md risk 2)
  must run on macOS once Xcode is installed.
- Pipe trap: `cargo check | tail` hides cargo's exit code — check `pipestatus`.

## Phase 2–4 implementation notes (beyond TECH.md)

- Per-pane glass: panes paint NO background today (`PaneView::render`,
  `app/src/pane_group/pane/view/mod.rs:375-440`); add a background Container;
  inversion pattern per TECH.md. Glass state (`HashSet<PaneId>` + opacity) on
  `BrowserUnderlayState` — `PaneId` is process-local, never persist it.
- Control plane recipe: catalog entry (`crates/local_control/src/catalog.rs`)
  + bridge arm (`app/src/local_control/bridge.rs`) + handler
  (`app/src/local_control/handlers/browser.rs`, new) + resolver
  (`resolver.rs`) + permissions (`permissions.rs`,
  `app/src/settings/local_control.rs`). Async results need
  `HandlerResponse::{Ready, Pending(oneshot)}` — transport task at
  `app/src/local_control/mod.rs:579-590` awaits with ~10s timeout. Add
  `session_uuid` to `pane.list` rows (`handlers/metadata.rs:478-499`).
- Dev invocation for warpctrl: `cargo run -p warp --bin warp -- --warpctrl …`.
- MCP crate: `crates/warp_browser_mcp` (workspace glob auto-members it); rmcp
  is already a workspace dep (server+transport-io+macros features); link
  `local_control` client/discovery/selection directly; pane binding order per
  `MCP.md`. Spawn processes only via `crates/command` (lint).
- `CallMCPToolExecutor` already holds the calling pane's `terminal_view_id`
  (`app/src/ai/blocklist/action_model/execute/call_mcp_tool.rs:26`) — that's
  the 4b hook; out of scope now.

## Local machine state (for the session running on Abe's Mac)

Worktree: `~/Dev/warp/.claude/worktrees/browser-underlay` (branch checked out
there; main checkout is on `abe/markdown-block-output` — leave it alone).
