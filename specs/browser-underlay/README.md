# Browser underlay operator README

The browser underlay renders a live WKWebView *behind* a Warp window's terminal content, with the terminal drawn over it on translucent "night glass". It serves two workflows:

- **Ambience** — a muted video stream (YouTube aquarium, lofi radio, etc.) playing behind the whole window.
- **Monitorable agent browsing** — an agent attaches a browser to its own pane via Warp's built-in MCP endpoint; that pane turns dark glass so the operator can watch exactly what the agent's browser is doing while the agent works. Agent browsers are **pane-isolated**: each lives behind the tab its pane belongs to, is visible only while that tab is frontmost, and can never touch the ambience background.

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

The underlay never steals input by default. The gesture key is **F18** — a
key no physical keyboard has, so it can't collide with any app binding —
meant to be produced by remapping CapsLock at the OS level:

```bash
# CapsLock emits F18 (reversible; resets on reboot):
hidutil property --set '{"UserKeyMapping":[{"HIDKeyboardModifierMappingSrc":0x700000039,"HIDKeyboardModifierMappingDst":0x70000006D}]}'
# undo:
hidutil property --set '{"UserKeyMapping":[]}'
```

With that in place, in any window with a visible underlay (the gesture
controls whatever browser is currently composited — an agent's browser in
its tab, the ambience elsewhere):

- **Tap CapsLock** — toggle interactive mode (input goes to the page until
  toggled back; an accent border marks the mode).
- **Hold CapsLock** (>300ms) — interactive only while held; release snaps
  input back to the terminal.
- **⌘Esc** — always returns to the terminal, clearing both modes.

Note the remap takes CapsLock away from everything else (caps-locking
included) system-wide while active; Karabiner-Elements can scope it if that
matters.

## Quickstart: agent browsing via MCP

Warp serves the MCP endpoint itself — no extra binary. Every pane shell gets
`WARP_MCP_URL` and `WARP_MCP_TOKEN` (restart panes opened before Scripting was
enabled); register the endpoint for Claude Code or any Streamable HTTP MCP
client — see `MCP.md` for the JSON. An agent running inside a Warp pane binds
to that pane via the `X-Warp-Pane` header (from `WARP_TERMINAL_SESSION_UUID`)
and can then:

1. `browser_attach {url}` — browser appears behind its pane's tab, pane goes night glass
2. `browser_eval` / `browser_navigate` / `browser_screenshot` — drive and read the page
3. `browser_detach` — clean up (also automatic on pane close, after 15 idle minutes, or when a window exceeds 3 agent browsers)

Switching to another tab hides the agent's browser (audio keeps playing);
switching back shows it again. The ambience underlay is untouchable from the
endpoint: only `warpctrl --ambience` and the settings field drive it.

## Files

- `PRODUCT.md` — UX intent and scope decisions
- `TECH.md` — architecture, compositing model, phases, ADR-style decisions
- `SECURITY.md` — permission model and threat notes
- `MCP.md` — Warp-hosted `/mcp` endpoint, tool surface, and pane binding
