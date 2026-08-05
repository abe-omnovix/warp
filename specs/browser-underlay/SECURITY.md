# Browser underlay SECURITY

## Gates

1. `FeatureFlag::BrowserUnderlay` — compile/runtime gate, macOS only.
2. `allow_browser_control` (Settings › Scripting, **default off**) — required
   for every `browser.*` and `pane.glass.*` local-control action. Without it,
   requests fail with `InsufficientPermissions`. The env-var smoke hook
   (`WARP_BROWSER_UNDERLAY_TEST_URL`) only affects windows of a process the
   operator launched with that variable and does not expose control surfaces.
3. Local-control transport auth (existing, unchanged): owner-only discovery
   record → Unix-socket credential broker with kernel peer-UID check →
   loopback HTTP with short-lived, action-scoped bearer tokens.
4. `/mcp` endpoint auth (Phase 6): loopback-only bind → browser `Origin`
   rejection → per-launch bearer token (`WARP_MCP_TOKEN`, generated from
   OS CSPRNG at server start, never persisted, distributed only through
   pane-shell environment). Requests then pass gate 2 exactly like
   warpctrl requests — the token authenticates the caller, it does not
   bypass permissions.

## Agent isolation (Phase 6)

- Every MCP tool call is **pane-keyed** (explicit `pane` argument or
  `X-Warp-Pane` header). There is no active-pane fallback, so an agent
  cannot affect whatever tab the operator happens to be looking at.
  `warpctrl browser`/`pane glass` commands follow the same rule: they bind
  to the pane they run in (`$WARP_TERMINAL_SESSION_UUID`) and error without
  an explicit binding otherwise — the active pane tracks the operator's
  focus, so binding to it would race against their attention.
- The **ambience underlay is unreachable from the MCP endpoint**: the
  `ambience: true` routing flag exists only on the warpctrl/control-plane
  surface, and the endpoint never sets it. An agent cannot navigate,
  evaluate JS in, screenshot, or detach the user's default background.
- Agent browsers are visible only while their pane's tab is frontmost;
  hiding one also revokes its interactive state, so a backgrounded agent
  page can never receive operator input.

## Threat notes

- **`browser.eval` is arbitrary JS in the underlay page.** It runs in the
  webview's web content process, not in Warp. The blast radius is the page and
  its (ephemeral) browsing state. It is still the most powerful action in the
  family, which is why the permission is default-off and every action is
  individually named in the catalog (no wildcard grant).
- **Navigation is unrestricted by design** (agents need it). Operators who
  want an allowlist can front it in their MCP client config; a Warp-side URL
  allowlist is a possible follow-up if demand appears.
- **Ephemeral `WKWebsiteDataStore`**: no cookies, localStorage, or cache
  persist across detach/app restart, and the underlay never shares state with
  the user's real browsers. There is no meaningful session to exfiltrate.
- **Input isolation**: with `interactive` off (default), the underlay is
  excluded from hit testing and the responder chain — a malicious page cannot
  see keystrokes or clicks. Interactive mode is operator/agent-explicit and
  visually implied by the glass treatment.
- **Screenshots** (`browser.screenshot`) capture only the webview's own
  content via WebKit's snapshot API — never the terminal pixels above it.
- **Process isolation**: web content runs in WebKit's out-of-process
  sandboxed processes (`com.apple.WebKit.WebContent`), not in the Warp
  process.
