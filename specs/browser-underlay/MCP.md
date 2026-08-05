# Warp-hosted MCP endpoint (`POST /mcp`)

Warp itself serves the agent-browser MCP endpoint on the local-control HTTP
server (Phase 6). There is no separate server binary to install or spawn —
the earlier stdio crate `warp_browser_mcp` (Phase 4) is retired. The endpoint
follows the **stateless Streamable HTTP** shape of MCP revision 2026-07-28:

- Single `POST /mcp` endpoint; no GET/SSE stream, no protocol sessions, no
  `Mcp-Session-Id`. Every request is self-contained.
- Legacy clients speaking 2025-era revisions still work: `initialize` is
  answered statically (echoing the requested `protocolVersion`) and no
  session is minted; `notifications/initialized` (and any notification) gets
  `202 Accepted`.
- Supported methods: `initialize`, `ping`, `tools/list`, `tools/call`.

## Discovery and auth

Warp injects two variables into every pane shell (new panes only — restart a
pane to pick them up after enabling Scripting):

| Env var | Meaning |
|---|---|
| `WARP_MCP_URL` | Full endpoint URL, e.g. `http://127.0.0.1:<port>/mcp` |
| `WARP_MCP_TOKEN` | Per-launch bearer token, required on every request |

Layered gates: loopback-only bind → browser `Origin` rejection → bearer token
(`Authorization: Bearer $WARP_MCP_TOKEN`) → the full local-control permission
check per action (Scripting enabled + `allow_browser_control`, default off).

Registration for Claude Code (or any Streamable HTTP client):

```json
{
  "mcpServers": {
    "warp-browser": {
      "type": "http",
      "url": "${WARP_MCP_URL}",
      "headers": {
        "Authorization": "Bearer ${WARP_MCP_TOKEN}",
        "X-Warp-Pane": "${WARP_TERMINAL_SESSION_UUID}"
      }
    }
  }
}
```

## Pane binding

Agent browsers are pane-isolated: each tool call must resolve to the pane
(terminal session UUID) that owns the browser. Resolution order:

1. Explicit `pane` tool argument. The schema annotates it with
   `x-mcp-header: "Pane"` (2026-07-28), so conforming clients also mirror it
   as an `Mcp-Param-Pane` routing header.
2. The client-configured `X-Warp-Pane` header (interpolated from
   `WARP_TERMINAL_SESSION_UUID`, which Warp injects into every pane shell).

There is deliberately **no active-pane fallback and no ambience route**: a
missing binding is a tool error, and an agent can never navigate the user's
default (ambience) background. The pane's browser is only composited while
that pane's tab is frontmost in its window; it stays alive — but invisible —
from other tabs.

## Tools

| Tool | Input | Result |
|---|---|---|
| `browser_attach` | `{url, glass? = true, glass_opacity?, pane?}` | attaches this pane's browser and loads the URL; night-glasses the pane |
| `browser_navigate` | `{url, pane?}` | ack |
| `browser_eval` | `{javascript, pane?}` | result text (non-strings JSON-encoded) |
| `browser_screenshot` | `{pane?}` | MCP image content (PNG); a note is appended when the pane's tab is hidden (capture may be stale) |
| `browser_set_glass` | `{enabled, opacity?, pane?}` | ack |
| `browser_set_interactive` | `{enabled, pane?}` | ack |
| `browser_status` | `{pane?}` | `{attached, url, visible, glass, interactive}` |
| `browser_detach` | `{pane?}` | ack |

Requires the Warp fork build with `FeatureFlag::BrowserUnderlay` and the
`allow_browser_control` setting enabled (Settings › Scripting).
