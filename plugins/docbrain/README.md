# DocBrain plugin for Claude Code

Skills that make an agent use DocBrain the way the product intends, and DocBrain's MCP server, connected with a browser sign-in. Today: one skill.

| Skill | What it does |
|---|---|
| `docbrain-capture` | Turns what a conversation worked out into a DocBrain capture: checks DocBrain first, drafts in the team's words with the file it concerns and the premises it rests on, shows the draft, writes after a yes, and reports where it landed. |

## Install

From the repository you work in:

```bash
claude plugin marketplace add docbrain-ai/docbrain --scope project
claude plugin install docbrain@docbrain --scope project
```

The install asks for one thing, your **DocBrain MCP address**: your DocBrain address followed by `/api/v1/mcp`, exactly. With Docker Compose that is `http://localhost:3001/api/v1/mcp`. In DocBrain, your account menu › **Connected apps** shows it with a Copy button. To skip the prompt, add `--config mcp_url=https://docbrain.your-org.internal/api/v1/mcp`.

Then sign in once: run `/mcp` in Claude Code, choose `docbrain` and sign in. Your browser opens DocBrain, you sign in if you have not in the last 10 minutes, and click **Allow**. No API key. Over SSH or on a machine without a browser, run `claude mcp login plugin:docbrain:docbrain --no-browser` (the plugin's server has that full name) and paste back the address it asks for. The connection lasts while you use it: it ends after 30 days unused or 90 days after you allowed it (your admin can change that), and you allow it again the same way.

Both install commands write to `.claude/settings.json` (the marketplace under `extraKnownMarketplaces`, the plugin under `enabledPlugins`). Commit that file and everyone who trusts the project gets the skill; each person enters the address and signs in as themselves, and DocBrain acts with their own role, never more.

Needs Claude Code 2.1.186 or later and DocBrain 1.6.4 or later, with **Sign-in for apps** on. It is on by default with Docker Compose. On Kubernetes an admin turns it on (`oauthServer.enabled: true` in the Helm values); while it is off, DocBrain answers the sign-in with "app sign-in is off on this DocBrain". [Turning it on →](https://github.com/docbrain-ai/docbrain/blob/main/docs/agents.md#sign-in-for-apps)

To disconnect, revoke Claude Code under your account › **Connected apps**; an admin can revoke any app under **Settings › Sign-in for apps**.

**Upgrading from 1.0.0.** The plugin no longer asks for a server URL and an API key; it asks for the MCP address and you sign in. If you created a key only for the plugin, revoke it under your account › **My API keys**.

Invoke the skill as `/docbrain:docbrain-capture`, or just say "capture this": it also triggers itself when a conversation has produced knowledge written nowhere.

Prefer a plain directory with no plugin? Copy `skills/docbrain-capture` into your repository's `.claude/skills/`, connect the server with `claude mcp add --transport http docbrain <your MCP address>`, and invoke the skill as `/docbrain-capture`.

## Privacy

The plugin has no code and sends nothing to DocBrain's makers. Claude Code connects only to the DocBrain server at the address you enter, which your organization runs. Only what a tool sends reaches it (a question, the paths being checked, a capture you said yes to), never the session's transcript. What that server keeps and who can see it: [Privacy properties](https://github.com/docbrain-ai/docbrain/blob/main/docs/agents.md#privacy-properties).

## Test it

The plugin ships an eval suite under `evals/` with the connector mocked, so it runs anywhere with no server and no key: `claude plugin eval plugins/docbrain --scaffold --trust-plugin --ablation none`. The cases check what the skill makes Claude do: checks first, drafts with the anchor and a premise, writes once after a yes, offers an update when the capture exists, refuses lines that do not hold the fix, leaves secrets and names out, files where a pasted credential lives instead of its value, reports a refused write once. Add `--json results.json --threshold 1.0` in CI.

## Prove it before you trust it

`claude --plugin-dir plugins/docbrain` loads the plugin from a checkout for a session, and `claude plugin validate plugins/docbrain` checks the manifest and the skill. The repository's own live proof drives a headless Claude Code session with the skill against a test stack and checks, by API and by Ask, that a capture lands only after a yes.
