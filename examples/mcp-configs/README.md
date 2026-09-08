# MCP client configs

Drop-in configuration for each editor. Replace `YOUR_API_KEY_HERE` with a key
from `docbrain token create --name "MCP" --role viewer`, and point
`DOCBRAIN_SERVER_URL` at your instance.

| File | Editor | Where it goes |
|---|---|---|
| `claude-code.json` | Claude Code (CLI and VS Code extension) | `.mcp.json` at the **project root** |
| `cursor.json` | Cursor | `.cursor/mcp.json` |
| `vscode.json` | VS Code (Copilot agent mode) | `.vscode/mcp.json` |

## Two things that differ per editor

**The top-level key is not the same.** VS Code's native MCP support uses
`servers` and wants an explicit `"type": "stdio"`. Claude Code and Cursor use
`mcpServers` and infer the transport. Copying one file's *contents* into
another's location silently produces a config that loads nothing, because the
key it looks for is absent — so use the file that matches your editor rather
than editing one into another.

**All three are workspace-root scoped.** Every editor resolves these paths
relative to the folder you opened, not to the file you are editing. In a
monorepo, opening the parent directory instead of the service folder means the
server never loads.

That failure is silent, and worth guarding against explicitly: with the
connector unloaded, an agent asked what the organisation knew about a file
searched the repository instead, found nothing there — correctly, captured
decisions are not stored in the repo — and reported that the organisation had
recorded nothing, about a file carrying a live pinned decision. A missing
connector is indistinguishable from an empty knowledge base unless something
tells the two apart.

So: **check the connector is loaded before you trust an answer that says there
is nothing to know.** Your editor's MCP panel should list `docbrain` and its
eleven tools. The `CLAUDE.md` snippet in [Coding Agents](../../docs/agents.md)
also instructs the agent to say so rather than guess.

## Verifying without an editor

`docbrain-mcp` speaks JSON-RPC on stdin/stdout, so you can check the key and
URL before wiring anything up:

```bash
DOCBRAIN_SERVER_URL=http://localhost:3000 DOCBRAIN_API_KEY=... \
  npx -y docbrain-mcp@latest <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}
EOF
```

It prints `Connected to <url> as <you>` on stderr and the tool list on stdout.
A 401 here is a bad key, not a bad editor config.
