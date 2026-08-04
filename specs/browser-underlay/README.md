# Browser underlay operator README

The browser underlay renders a live WKWebView *behind* a Warp window's terminal content, with the terminal drawn over it on translucent "night glass". It serves two workflows:

- **Ambience** — a muted video stream (YouTube aquarium, lofi radio, etc.) playing behind the whole window.
- **Monitorable agent browsing** — an agent attaches a browser to its own pane via the `warp-browser-mcp` MCP server; that pane turns dark glass so the operator can watch exactly what the agent's browser is doing while the agent works.

Everything is gated by `FeatureFlag::BrowserUnderlay` (macOS only) and, for external control, the `allow_browser_control` setting (Settings › Scripting, default off).

## Quickstart: aquarium background

Build with the feature and set the test URL (until the control plane lands):

```bash
cargo run -p warp --features browser_underlay
# with:
export WARP_BROWSER_UNDERLAY_TEST_URL='https://www.youtube.com/embed/<video-id>?autoplay=1&mute=1&controls=0&loop=1&playlist=<video-id>'
```

Every new window attaches an underlay loading that URL. Use YouTube **embed** URLs with `autoplay=1&mute=1` — muted autoplay is always permitted; sound requires interactive mode or a JS unmute.

Once the control plane lands (see `TECH.md` Phase 3):

```bash
warpctrl browser attach --url 'https://www.youtube.com/embed/...'
warpctrl browser detach
```

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
