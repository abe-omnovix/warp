# Browser underlay operator README

The browser underlay renders a live WKWebView *behind* a Warp window's terminal content, with the terminal drawn over it on translucent "night glass". It serves two workflows:

- **Ambience** — a muted video stream (YouTube aquarium, lofi radio, etc.) playing behind the whole window.
- **Monitorable agent browsing** — an agent attaches a browser to its own pane via the `warp-browser-mcp` MCP server; that pane turns dark glass so the operator can watch exactly what the agent's browser is doing while the agent works.

Everything is gated by `FeatureFlag::BrowserUnderlay` (macOS only) and, for external control, the `allow_browser_control` setting (Settings › Scripting, default off).

## Quickstart: aquarium background

Build with the feature:

```bash
cargo run -p warp --features browser_underlay
```

Then set the ambience URL in **Settings › Appearance › Browser ambience** (or
`appearance.browser_ambience.url` in `settings.toml`); every open and future
window attaches an underlay loading it, and clearing the field detaches them.
The "Glass opacity" slider next to it controls how translucent the terminal
fill is. Use YouTube **embed** URLs with `autoplay=1&mute=1` — muted autoplay
is always permitted; sound requires interactive mode or a JS unmute:

```
https://www.youtube.com/embed/<video-id>?autoplay=1&mute=1&controls=0&loop=1&playlist=<video-id>
```

The `WARP_BROWSER_UNDERLAY_TEST_URL` environment variable still works as a
smoke-test hook and takes precedence over the setting.

With `allow_browser_control` enabled (Settings › Scripting), the control plane
does the same per window:

```bash
warpctrl browser attach 'https://www.youtube.com/embed/...' --no-glass
warpctrl browser status
warpctrl browser detach
```

(`attach` without `--no-glass` also turns the targeted pane to night glass —
that's the agent-browsing porthole; `warpctrl pane glass true|false` toggles
it independently.)

## Interacting with the page yourself

The underlay never steals input by default. Three gestures flip that,
window-locally, whenever an underlay is attached:

- **⌘⇧B** — toggle interactive mode (input goes to the page until toggled
  back; an accent border marks the mode).
- **Hold Globe/Fn** — interactive only while held; release snaps input back
  to the terminal. Quick Fn-combos (fn+arrows) stay in the terminal.
- **⌘Esc** — always returns to the terminal, clearing both modes.

Fancy a different key (e.g. CapsLock-as-hold)? Remap CapsLock to the Globe
key at the OS level (`hidutil` / Karabiner) and the hold gesture follows.

## Quickstart: agent browsing via MCP

Register `warp-browser-mcp` for Claude Code (or any MCP client) — see `MCP.md`. An agent running inside a Warp pane self-binds to that pane via `WARP_TERMINAL_SESSION_UUID` and can then:

1. `browser_attach {url}` — browser appears behind its pane, pane goes night glass
2. `browser_eval` / `browser_navigate` / `browser_screenshot` — drive and read the page
3. `browser_detach` — clean up

## Files

- `PRODUCT.md` — UX intent and scope decisions
- `TECH.md` — architecture, compositing model, phases, ADR-style decisions
- `SECURITY.md` — permission model and threat notes
- `MCP.md` — `warp-browser-mcp` tool surface and pane binding
