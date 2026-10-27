# MCP client configs

Drop-in configuration for each editor. Every one points at the same address: your DocBrain address
followed by `/api/v1/mcp` (`http://localhost:3001/api/v1/mcp` with Docker Compose). Change the host
to your instance; keep the path exactly, with no trailing slash.

| File | Editor | Where it goes | How it signs in |
|---|---|---|---|
| `claude-code.json` | Claude Code (CLI and VS Code extension) | `.mcp.json` at the **project root** | `/mcp` → `docbrain` → sign in in the browser; no key |
| `cursor.json` | Cursor | `.cursor/mcp.json` | an API key from the `DOCBRAIN_API_KEY` environment variable |
| `vscode.json` | VS Code (Copilot agent mode) | `.vscode/mcp.json` | an API key VS Code asks for once and stores |

`claude-code.json` holds nothing secret, so commit it: each person signs in as themselves and DocBrain
acts with their role. Claude Code users usually want the [plugin](../../plugins/docbrain) instead,
which connects the same server and adds the capture skill.

For Cursor and VS Code, create a key in DocBrain under your account menu › **My API keys**, or with
`docbrain token create --name "MCP" --role editor` (`viewer` reads and asks; `editor` captures too). The
files read it from your environment or a prompt, so no key is written in them.

An editor that signs in with an MCP *client ID metadata document* can drop the header and sign in
through the browser, as Claude Code does. If DocBrain's page then says the app is not allowed yet, an
admin allows it once under **Settings › Sign-in for apps**.

## Two things that differ per editor

**The top-level key is not the same.** VS Code's native MCP support uses
`servers` and wants an explicit `"type": "http"`. Claude Code and Cursor use
`mcpServers`. Copying one file's *contents* into another's location silently
produces a config that loads nothing, because the key it looks for is absent —
so use the file that matches your editor rather than editing one into another.

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
tools. The `CLAUDE.md` snippet in [Coding Agents](../../docs/agents.md)
also instructs the agent to say so rather than guess.

## Editors that only start local programs

The [`docbrain-mcp`](../../crates/docbrain-mcp) connector speaks MCP over stdio. Install it once with
`npm install -g docbrain-mcp` and start the installed `docbrain-mcp`, not `npx`: an editor starts it in
the open project, and `npx` reads that project's `.npmrc`.

```json
{
  "mcpServers": {
    "docbrain": {
      "command": "docbrain-mcp",
      "env": {
        "DOCBRAIN_API_KEY": "YOUR_API_KEY_HERE",
        "DOCBRAIN_SERVER_URL": "http://localhost:3001"
      }
    }
  }
}
```

## Verifying without an editor

Check the address and a key before wiring anything up:

```bash
curl -s -w '\nHTTP %{http_code}\n' http://localhost:3001/api/v1/mcp \
  -H "Authorization: Bearer $DOCBRAIN_API_KEY" \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-06-18' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

It prints the tool list and `HTTP 200`. `HTTP 401` is a bad key and `HTTP 404` a wrong address, not a bad editor config.
