# DocBrain plugin for Claude Code

Skills that make an agent use DocBrain the way the product intends, through DocBrain's MCP server, which you connect once with a browser sign-in. Today: one skill.

| Skill | What it does |
|---|---|
| `docbrain-capture` | Turns what a conversation worked out into a DocBrain capture: checks DocBrain first, drafts in the team's words with the file it concerns and the premises it rests on, shows the draft, writes after a yes, and reports where it landed. |

## Install

From the repository you work in:

```bash
claude plugin marketplace add docbrain-ai/docbrain --scope project
claude plugin install docbrain@docbrain --scope project
```

Then connect your DocBrain once, from the same repository:

```bash
claude mcp add --transport http --scope project docbrain https://docbrain.your-org.internal/api/v1/mcp
```

The address is your DocBrain address followed by `/api/v1/mcp`, exactly. With Docker Compose that is `http://localhost:3001/api/v1/mcp`. In DocBrain, your account menu › **Connected apps** shows it with a Copy button once Sign-in for apps is on.

Then sign in once: run `/mcp` in Claude Code, choose `docbrain` and sign in. Your browser opens DocBrain, you sign in if you have not in the last 10 minutes, and click **Allow**. No API key. Over SSH or on a machine without a browser, run `claude mcp login docbrain --no-browser` in an interactive terminal (`ssh -t`): open the link it prints on your own computer, sign in and click **Allow**, then paste back the full address your browser ends on (it starts with `http://localhost`). The connection lasts while you use it: it ends after 30 days unused or 90 days after you allowed it (an admin can change the 90 days with `MAX_PAT_DAYS`), and you allow it again the same way.

The install commands write to `.claude/settings.json` (the marketplace under `extraKnownMarketplaces`, the plugin under `enabledPlugins`) and the connection to `.mcp.json`. Neither holds a secret. Commit both and everyone who trusts the project gets the skill and the connection with the address already in it; each person approves it, signs in as themselves, and DocBrain acts with their own role, never more. To keep the connection to yourself, leave out `--scope project`.

Needs Claude Code 2.1.186 or later and DocBrain 1.6.4 or later, with **Sign-in for apps** on. It is on by default with Docker Compose. On Kubernetes an admin turns it on (`oauthServer.enabled: true` in the Helm values); while it is off, the sign-in cannot start (DocBrain's discovery documents answer 404, and its 401 says "app sign-in is off on this DocBrain"); ask your admin to turn it on, or connect with an API key. [Turning it on →](https://github.com/docbrain-ai/docbrain/blob/main/docs/agents.md#sign-in-for-apps)

To disconnect, revoke Claude Code under your account › **Connected apps**; an admin can revoke any app under **Settings › Sign-in for apps**.

**Upgrading from 1.1.** The plugin no longer carries the connection or asks for the MCP address. Connect once with the `claude mcp add` command above and sign in again; the server is now listed as `docbrain`, not `plugin:docbrain:docbrain`, so a permission rule written for `mcp__plugin_docbrain_docbrain__*` becomes `mcp__docbrain__*`. From 1.0.0, also revoke the API key you created for the plugin, under your account › **My API keys**.

Invoke the skill as `/docbrain:docbrain-capture`, or just say "capture this": it also triggers itself when a conversation has produced knowledge written nowhere.

Prefer a plain directory with no plugin? Copy `skills/docbrain-capture` into your repository's `.claude/skills/`, connect the server the same way, and invoke the skill as `/docbrain-capture`.

## Privacy

The plugin has no code and sends nothing to DocBrain's makers. Claude Code connects only to the DocBrain server you connected, which your organization runs. Only what a tool sends reaches it (a question, the paths being checked, a capture you said yes to), never the session's transcript. What that server keeps and who can see it: [Privacy properties](https://github.com/docbrain-ai/docbrain/blob/main/docs/agents.md#privacy-properties).

## Test it

The repository keeps the plugin's eval suite beside it, under `plugin-evals/docbrain/`, with the connector mocked, so it runs anywhere with no server and no key. Its README has the two commands that run it. The cases check what the skill makes Claude do: checks first, drafts with the anchor and a premise, writes once after a yes, offers an update when the capture exists, refuses lines that do not hold the fix, leaves secrets and names out, files where a pasted credential lives instead of its value, reports a refused write once.

## Prove it before you trust it

`claude --plugin-dir plugins/docbrain` loads the plugin from a checkout for a session, and `claude plugin validate plugins/docbrain` checks the manifest and the skill. The repository's own live proof drives a headless Claude Code session with the skill against a test stack and checks, by API and by Ask, that a capture lands only after a yes.
