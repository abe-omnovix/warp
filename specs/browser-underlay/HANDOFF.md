# Browser underlay — session handoff

State: **Phases 1–4 implemented and committed** on `abe/browser-underlay`
(fork `abe-omnovix/warp`, upstream `warpdotdev/warp`, base master `af2f7c61`).
Phase 1 `ffdf295c`, Phase 2 `114b66ff`, Phase 3 `4753b8bc`, Phase 4 is the
commit adding this paragraph. Phase 5 stays documented-only. Remaining work is
runtime verification (below), which needs a real Metal build.

## Standing instruction from Abe

Implement **Phases 1–4 without stopping** (Phase 5 is documented-only),
committing per phase. Decisions in TECH.md "Decisions" + PRODUCT.md are
settled — do not re-litigate.

## What shipped per phase

- **Phase 1 (`ffdf295c`)** — WKWebView underlay ObjC shim + C API, contentView
  container restructure, `WindowManager` trait plumbing,
  `BrowserUnderlayState` singleton, `WARP_BROWSER_UNDERLAY_TEST_URL` hook,
  interim window-level glass. A macOS-only compile bug it introduced
  (`crates/warpui/src/platform/mac/window.rs:740` — `Retained::as_ptr` on what
  became a plain `&NSView`) was found and fixed during Phase 3 verification.
- **Phase 2 (`114b66ff`)** — per-pane night glass: glass state
  (`HashSet<PaneId>` + per-window opacity override) on `BrowserUnderlayState`;
  inversion (workspace tint 10 / non-glass panes 95 / glass panes at the
  `glass_opacity` setting, default 55) in `PaneView::render` +
  `workspace/util.rs`; `BrowserUnderlaySettings` group (`ambience_url`,
  `glass_opacity`, TOML `appearance.browser_ambience.*`, registered behind the
  feature flag) with live attach/navigate/detach-on-change wiring
  (`browser_underlay::init`); Settings › Appearance › "Browser ambience"
  category; unit tests in `app/src/browser_underlay_tests.rs`.
- **Phase 3 (this commit)** — control plane: `browser.attach|navigate|eval|
  screenshot|interactive|status|detach` + `pane.glass.set` in the catalog,
  protocol params/results, bridge dispatch, and a new
  `app/src/local_control/handlers/browser.rs`; `HandlerResponse::{Ready,
  Pending(oneshot)}` with a 10s transport timeout (runtime now enables
  timers) for eval/screenshot; `allow_browser_control` secure setting
  (default off, `InsufficientPermissions` otherwise, toggle in Settings ›
  Scripting); `pane.list` rows gained `session_uuid` (hex, matches
  `WARP_TERMINAL_SESSION_UUID`; `PaneGroup::terminal_session_uuid_hex`);
  `warpctrl browser …` + `warpctrl pane glass` subcommands (screenshot has
  `--output <path>`). Tests in `local_control/mod_tests.rs` +
  `settings/local_control_tests.rs`.

- **Phase 4 (this commit)** — `crates/warp_browser_mcp` (bin
  `warp-browser-mcp`): rmcp 1.6 stdio server (`#[tool_router]` +
  `#[tool_handler]`), tools per MCP.md (`browser_attach|navigate|eval|
  screenshot|set_glass|set_interactive|status|detach`); binding order
  implemented as explicit `pane` param → `--pane-session-uuid` (clap
  env-fallback to `WARP_TERMINAL_SESSION_UUID`) → active pane, resolved fresh
  per call by matching `session_uuid` in a selector-less `pane.list` (which
  spans all windows); underlay ops target the bound window id, glass ops the
  window+tab+pane ids; screenshots return MCP image content; blocking
  local-control client calls run via `spawn_blocking`. Unit tests in
  `src/main_tests.rs`.

## Later hooks (out of scope)

- `CallMCPToolExecutor` already holds the calling pane's `terminal_view_id`
  (`app/src/ai/blocklist/action_model/execute/call_mcp_tool.rs:26`) — that's
  the Warp-agent auto-bind (4b) hook.

## Verification environment (updated — supersedes "macOS blocked")

- **macOS type-check now works on Abe's machine**: prepend
  `PATH=/tmp/fake-xcrun:$PATH` (stub that fakes `metal`/`metallib`, delegates
  the rest). `cargo check -p warp` and `cargo nextest run -p warp …` both
  work. Never *run* a GUI binary built with the stub (placeholder metallib).
  Real Metal builds still need Xcode + `script/macos/install_build_deps`.
- **Do not share `CARGO_TARGET_DIR` across worktrees**: workspace-crate units
  collide between worktrees (a subagent's build of the same package at an
  older commit gets reused as "fresh", yielding phantom E0432/E0599). If it
  happens: `cargo clean -p <edited crates>` — beware this also drops the
  multi-GB incremental caches.
- Pipe trap: `cargo check | tail` hides cargo's exit code — check `pipestatus`.
- Still outstanding for runtime verification (needs a real Metal build):
  integration suite, the muted-YouTube occlusion spike (WebKit pausing →
  fallback plan B child-NSWindow, TECH.md risk 2), and manual settings-page
  checks per `gui-settings-ui` "How to verify".

## Local machine state (for the session running on Abe's Mac)

Worktree: `~/Dev/warp/.claude/worktrees/browser-underlay` (branch checked out
there; main checkout is on `abe/markdown-block-output` — leave it alone).
`warpctrl` dev invocation: `cargo run -p warp --bin warp -- --warpctrl …`.
