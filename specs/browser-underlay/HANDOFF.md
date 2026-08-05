# Browser underlay — session handoff

State: **Phases 1–6 implemented and committed** on `abe/browser-underlay`
(fork `abe-omnovix/warp`, upstream `warpdotdev/warp`, base master `af2f7c61`).
Phase 1 `ffdf295c`, Phase 2 `114b66ff`, Phase 3 `4753b8bc`, Phase 4
`31e75a30` (retired again in Phase 6), Phase 5 hotkey slice `4d530977`,
Phase 6 is the commit adding this paragraph. Remaining work is the runtime
verification tail (below).

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

- **Phase 5 slice (`4d530977`)** — interactive-mode hotkeys: OS-level
  CapsLock→F18 remap (hidutil, documented in README), local NSEvent monitor
  (tap = latched toggle, >300 ms hold = momentary, ⌘Esc force-off),
  first-responder transfer with the interactive flag set *before* the
  transfer, hitTest/sendEvent/performKeyEquivalent routing fixes, accent
  border while interactive, fake-fullscreen JS shim (keeps element
  fullscreen inside the underlay instead of splitting to a Space), and a
  pointer-events guard so inert pages show no hover UI.

- **Phase 6 (this commit)** — pane-isolated agent browsers + Warp-hosted
  MCP: owner-keyed multi-webview platform layer (`owner` "" = ambience at
  the absolute back; agent underlays keyed by terminal-session UUID hex,
  created hidden, composited only while their pane's tab is frontmost —
  `browser_underlay_set_visible`, visibility sync via
  `Workspace::set_active_tab_index`, re-homing on pane moves);
  `BrowserUnderlayOwner` plumbing through `WindowManager`; app model split
  into ambience (settings-driven, agent-unreachable) vs agent underlays
  with lifecycle (detach on pane close; 15-min idle TTL sweeper; cap 3 per
  window, LRU-evicted); control-plane routing flags (`--ambience`
  human-only, mutually exclusive with `--pane-session-uuid`); the
  stateless Streamable HTTP `/mcp` endpoint (MCP 2026-07-28: no sessions,
  legacy `initialize` accommodation, `x-mcp-header`-annotated `pane`
  argument, `X-Warp-Pane` header binding, per-launch bearer token) in
  `app/src/local_control/mcp_endpoint.rs`, with `WARP_MCP_URL` /
  `WARP_MCP_TOKEN` injected into new pane shells; `crates/warp_browser_mcp`
  deleted. Tests: `mcp_endpoint_tests.rs`, rewritten
  `browser_underlay_tests.rs` (ownership/visibility/eviction/hotkey).

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
- **Xcode 26.6 + MetalToolchain are now installed** (license accepted,
  `xcodebuild -runFirstLaunch` + `-downloadComponent MetalToolchain` done),
  so real Metal builds and the integration suite run on this machine.
- **Runtime verification so far (real Metal build, 2026-08-05):**
  - Live smoke test on `warp-oss --features browser_underlay`: ambience URL
    setting attaches on startup and on change; WebGL aquarium animates
    continuously behind night glass (occlusion risk 2 answered for
    rAF/compositing); glass opacity slider works; interactive hotkeys work.
    YouTube *embeds* refuse to play (error 153 — no referring page); use
    direct pages/WebGL, or later load embeds via a wrapper page with a real
    origin.
  - **Full integration suite: 243/243 pass** (`cargo nextest run -p
    integration -E 'not test(ssh)'`; the ssh tests dial Warp-internal CI
    fixtures like `ubuntu-14-04:25784` and cannot run on this machine).
    The first run caught a real regression — window dealloc still read the
    state ivar off the contentView (now a plain container), so closing any
    window panicked; fixed to use `warp_host_view_for_window` (`31e75a30`).
  - Dev-channel binaries (`warp`, `stable` bins) abort at startup without the
    private `warp-channel-config` on PATH — use the `warp-oss` bin (inline
    config) for source-built runs on this machine.
- Phase 5 slice verified live end-to-end (CapsLock tap/hold/⌘Esc, YouTube
  playback with pointer-guard + fake fullscreen, stdio MCP demo before its
  retirement). Scripting + `allow_browser_control` are enabled in Abe's
  build.
- Still outstanding: `<video>`-element half of the occlusion spike
  (`browser navigate` to a muted autoplay video, `browser eval` that
  `currentTime` advances), the manual settings-page search pass per
  `gui-settings-ui` "How to verify", and a live `/mcp` demo from inside a
  Warp pane (`curl -X POST "$WARP_MCP_URL" -H "Authorization: Bearer
  $WARP_MCP_TOKEN" …` then Claude Code registration per MCP.md).

## Local machine state (for the session running on Abe's Mac)

Worktree: `~/Dev/warp/.claude/worktrees/browser-underlay` (branch checked out
there; main checkout is on `abe/markdown-block-output` — leave it alone).
`warpctrl` dev invocation: `cargo run -p warp --bin warp -- --warpctrl …`.
