# DocBrain plugin for Claude Code

Skills that make an agent use DocBrain the way the product intends. Today: one.

| Skill | What it does |
|---|---|
| `docbrain-capture` | Turns what a conversation worked out into a DocBrain capture: checks DocBrain first, drafts in the team's words with the file it concerns and the premises it rests on, shows the draft, writes after a yes, and reports where it landed. |

## Install

The plugin needs DocBrain's MCP server connected in the editor; the [agents guide](../../docs/agents.md) has the one-file setup.

For the whole team, from the repository you work in:

```bash
claude plugin marketplace add docbrain-ai/docbrain --scope project
claude plugin install docbrain@docbrain --scope project
```

Both commands write to `.claude/settings.json` — the marketplace under `extraKnownMarketplaces`, the plugin under `enabledPlugins`; commit that file and everyone who trusts the project gets the skill. Invoke it as `/docbrain:docbrain-capture`, or just say "capture this" — the skill also triggers itself when a conversation has produced knowledge written nowhere.

Prefer a plain directory with no plugin? Copy `skills/docbrain-capture` into your repository's `.claude/skills/` and invoke it as `/docbrain-capture`.

## Prove it before you trust it

`claude --plugin-dir plugins/docbrain` loads the plugin from a checkout for a session, and `claude plugin validate plugins/docbrain` checks the manifest and the skill. The repository's own live proof drives a headless Claude Code session with the skill against a test stack and checks, by API and by Ask, that a capture lands only after a yes.
