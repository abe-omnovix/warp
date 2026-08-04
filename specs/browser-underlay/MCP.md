# warp-browser-mcp

Stdio MCP server (planned workspace crate `crates/warp_browser_mcp`, Phase 4)
that lets an agent attach and drive the browser underlay for the pane it is
running in. It links `local_control`'s client/discovery/selection directly and
speaks the same authenticated loopback protocol as `warpctrl`.

## Pane binding

Resolution order for "which pane does this server control":

1. Explicit `pane` tool parameter (terminal session UUID).
2. `--pane-session-uuid` CLI flag.
3. **`WARP_TERMINAL_SESSION_UUID` environment variable** — Warp injects this
   into every pane's shell, so an MCP server spawned by an agent CLI (e.g.
   Claude Code) running inside a pane inherits its pane identity
   automatically.
4. Fallback: the active pane (covers Warp-spawned MCP servers, which receive
   only static env).

The pane maps to its window for underlay operations; glass targets the pane.

## Tools

| Tool | Input | Result |
|---|---|---|
| `browser_attach` | `{url, glass? = true, glass_opacity?}` | attaches underlay to the bound pane's window; enables glass on the bound pane |
| `browser_navigate` | `{url}` | ack |
| `browser_eval` | `{javascript}` | `{result}` (string; non-strings JSON-encoded) |
| `browser_screenshot` | `{}` | MCP image content (PNG) |
| `browser_set_glass` | `{enabled, opacity?, pane?}` | ack |
| `browser_set_interactive` | `{enabled}` | ack |
| `browser_status` | `{}` | `{attached, url, interactive, glass_panes}` |
| `browser_detach` | `{}` | ack |

## Registration

Anywhere Warp-adjacent MCP configs are read (`~/.warp/.mcp.json`, project
`.mcp.json`, `~/.claude.json`):

```json
{
  "mcpServers": {
    "warp-browser": {
      "command": "/path/to/warp/target/release/warp-browser-mcp"
    }
  }
}
```

Requires the Warp fork build with `FeatureFlag::BrowserUnderlay` and the
`allow_browser_control` setting enabled (Settings › Scripting).
