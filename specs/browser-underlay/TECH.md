# Browser underlay TECH

## Architecture

One `WarpBrowserUnderlayView : WKWebView` per window, created by an ObjC shim
(`crates/warpui/src/platform/mac/objc/browser_underlay.{h,m}`) and inserted as
a sibling NSView *below* the Metal host view inside the window's content-view
container:

```
NSWindow (backgroundColor ≈ clear)
└─ container NSView                      (plain; contentView)
   ├─ WarpBrowserUnderlayView            (WKWebView; bottom)
   └─ WarpHostView                       (CAMetalLayer, opaque = NO;
      └─ terminal scene                   render pass clears alpha 0.0)
```

Why this composites correctly with **zero Warp render-loop changes**: the
window's Metal surface was already non-opaque with an alpha-0 clear color
(shipping window-transparency support), so wherever the scene does not paint
opaque pixels, the WindowServer shows what is behind the Metal layer — now the
webview instead of the desktop. WebKit renders and decodes video in its own
out-of-process tree (`com.apple.WebKit.WebContent` / GPU process), composited
by Core Animation at full rate, independent of Warp's on-demand
`setNeedsDisplay` render loop. Warp draws nothing per video frame.

"Night glass" is therefore pure Warp-side paint: with an underlay attached the
workspace takes the existing "background image present" code path (no opaque
`surface_2` window fill; terminals draw a translucent `theme.background()`
fill via `get_terminal_background_fill`).

### Input

The underlay is inert by default: `hitTest:` returns nil and
`acceptsFirstResponder` is NO, so it is invisible to event routing. An
explicit `interactive` flag (agent tool / control action) flips both;
disabling it returns first responder to the host view.

### Layering summary of changes

- `objc/browser_underlay.{h,m}` — the webview subclass + C API
  (attach/navigate/eval/snapshot/set_interactive/detach), ephemeral
  `WKWebsiteDataStore`, `mediaTypesRequiringUserActionForPlayback = None`.
- `objc/window.m` — `create_warp_nswindow` wraps the host view in a plain
  container content view; all sites that assumed `contentView == WarpHostView`
  now use `warp_host_view_for_window` (declared in `host_view.h`). Panels are
  unchanged (host view remains their content view; no underlay support).
- `platform/mac/window.rs` — FFI decls, callback trampolines, `Window::`
  statics; Metal device/layer/IME lookups go through the host-view accessor.
- `warpui_core/platform/mod.rs` — `WindowManager` trait methods with default
  no-ops (only macOS overrides), so winit/headless/test builds need no code.
- `warpui_core/windowing/state.rs` — public wrappers (`app.windows().…`).
- `app/src/browser_underlay.rs` — `BrowserUnderlayState` singleton model
  mirroring platform state for rendering; attach/detach helpers; env-var
  smoke hook.
- `app/src/workspace/{view,util}.rs` — glass path when underlay attached.

## Decisions (ADR-style)

**Engine: system WKWebView.** Zero-install, Safari-class media (hardware
decode, out of process), lightest credible option. Rejected: Ultralight (HTML5
video experimental/Windows-only, no WebGL), CEF (~200 MB + process zoo),
headless-Chromium-pixel-streaming (CPU copies + forced continuous repaints).

**Bindings: ObjC shim, not wry and not objc2-web-kit.** wry assumes it creates
the window and drags in tao; the repo already compiles ObjC via `cc`
(`window_blur.m` precedent) and the `hitTest:` override wants an ObjC
subclass anyway. objc2-web-kit was unnecessary once the subclass lived in ObjC.

**Compositing: content-view container restructure (Route A).** A true sibling
underlay behind the Metal view. Rejected: child-NSWindow-ordered-below (no
contentView churn, but one-frame resize lag, fullscreen/Spaces detach
hazards); offscreen-render-to-texture (Route B: per-frame CPU readback +
continuous repaints — wrong for an ambient background, but documented below as
the per-pane/cross-platform path).

**V1 scope: window-level webview.** One underlay per window; per-pane *glass*
(Phase 2) delivers the "agent pane as porthole" UX without per-pane native
view geometry mirroring. Per-pane *webviews* are Phase 5.

## Phases

1. **Platform underlay + window-level glass** — this change set. Smoke hook:
   `WARP_BROWSER_UNDERLAY_TEST_URL`.
2. **Night glass per pane** — per-pane background fill in `PaneView::render`
   (panes paint no background today); inversion: workspace tint ~0–15,
   non-glass panes near-opaque, glass panes dark ~55; `glass_opacity` +
   ambience URL settings.
3. **Control plane** — `browser.attach|detach|navigate|eval|screenshot|interactive|status`
   + `pane.glass.set` local-control actions; `HandlerResponse::Pending`
   (oneshot) for async eval/snapshot results; `allow_browser_control`
   (default off) in Settings › Scripting; `pane.list` gains `session_uuid`;
   `warpctrl browser …` subcommands.
4. **`warp-browser-mcp`** — workspace crate; rmcp stdio server linking
   `local_control` client directly. Pane binding order: explicit param →
   `--pane-session-uuid` → `WARP_TERMINAL_SESSION_UUID` env → active pane.
5. **Partially shipped: interactive-mode hotkeys + indicator.** A local
   NSEvent monitor (installed with the first attach; local monitors see
   events before the first responder, so the exit gestures work while the
   webview owns keys) recognizes ⌘⇧B (toggle latched interactive), Globe/Fn
   held ≥150 ms (momentary interactive while held; the grace period keeps
   Fn-combos like fn+arrows in the terminal), and ⌘Esc (force off). Gestures
   flow platform → `AppCallbacks::on_browser_underlay_hotkey` →
   `browser_underlay::handle_hotkey`, so the app model stays the single
   owner of interactive state (`interactive` latched ‖ `interactive_hold`
   momentary). While effectively interactive the workspace draws a 2px
   accent border. Still later: per-pane webview rects (mirror
   `PaneId::position_id()` → `element_position_by_id` geometry into native
   view frames, or Route B frame-push via `AssetSource::Raw` for
   cross-platform); rebindable hotkey chords (the chords are currently
   fixed in the ObjC monitor).

## Known risks

1. **contentView refactor blast radius** (first responder, titlebar drag,
   drag&drop, IME, fullscreen) — mitigated by the single accessor and the
   integration suite; land Phase 1 alone.
2. **WebKit occlusion heuristics** could pause video behind the (transparent
   but present) Metal layer — the Phase 1 spike proves this; fallback plan B
   is the child-NSWindow variant.
3. **Power draw** — hardware out-of-process decode; detach is one call; a
   pause-on-occlusion eval is a cheap later addition.
