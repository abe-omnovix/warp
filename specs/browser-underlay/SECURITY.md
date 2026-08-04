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
