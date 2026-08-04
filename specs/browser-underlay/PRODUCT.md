# Browser underlay PRODUCT

## What it is

A live browser rendered behind the terminal. Two user stories:

1. **Ambience.** "I want a relaxing aquarium/lofi stream behind my terminal."
   The whole window shows the stream behind translucent terminal glass, the
   same visual model as Warp's existing background-image themes — but alive.
2. **Watchable agent browsing.** "When my agent browses the web, I want to
   *see* it without giving it my screen." The agent attaches a browser to its
   own pane; that pane turns dark "night glass" so the page is visible behind
   the agent's terminal output. Other panes stay opaque and readable.

## Scope decisions (v1)

- **One webview per window**, not per pane. The per-pane experience comes from
  per-pane *glass*: only the agent's pane goes translucent, making it a
  porthole onto the window-level browser behind it. True per-pane webview
  rects are a later phase (see TECH.md Phase 5).
- **Inert by default.** The browser can never steal keyboard or mouse input.
  Interactive mode is an explicit, reversible toggle (agent tool or
  `warpctrl`), and Esc-style focus return keeps the terminal primary.
- **Muted by default.** Autoplay works muted (platform policy); sound requires
  an explicit interactive click or JS unmute.
- **Opt-in control.** External/agent control of the browser requires the
  `allow_browser_control` setting (default off) on top of the feature flag.

## Non-goals (v1)

- A general-purpose embedded browser UI (tabs, address bar, profiles).
- Cross-platform support (macOS only; the trait layer defaults keep other
  platforms building with the feature absent).
- Persistent browsing state (ephemeral data store by design).
