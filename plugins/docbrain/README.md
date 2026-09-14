# DocBrain plugin for Claude Code

Skills that make an agent use DocBrain the way the product intends. Today: one.

| Skill | What it does |
|---|---|
| `docbrain-capture` | Turns what a conversation worked out into a DocBrain capture: checks DocBrain first, drafts in the team's words with the file it concerns and the premises it rests on, shows the draft, writes after a yes, and reports where it landed. |

## Install

The plugin connects DocBrain's MCP server itself. The install asks for your server URL and an API key with capture permission — create one with `docbrain token create --name "Claude Code" --role editor` — or pass them non-interactively with `--config server_url=https://docbrain.your-org.internal --config api_key=…`. If you already connected the server by hand, either works; two connectors only duplicate the tools.

For the whole team, from the repository you work in:

```bash
claude plugin marketplace add docbrain-ai/docbrain --scope project
claude plugin install docbrain@docbrain --scope project
```

Both commands write to `.claude/settings.json` — the marketplace under `extraKnownMarketplaces`, the plugin under `enabledPlugins`; commit that file and everyone who trusts the project gets the skill. Invoke it as `/docbrain:docbrain-capture`, or just say "capture this" — the skill also triggers itself when a conversation has produced knowledge written nowhere.

Prefer a plain directory with no plugin? Copy `skills/docbrain-capture` into your repository's `.claude/skills/` and invoke it as `/docbrain-capture`.

## Test it

The plugin ships an eval suite under `evals/` with the connector mocked, so it runs anywhere with no server and no key: `claude plugin eval plugins/docbrain --scaffold --trust-plugin --ablation none`. Seven cases check what the skill makes Claude do — checks first, drafts with the anchor and a premise, writes once after a yes, offers an update when the capture exists, refuses lines that do not hold the fix, leaves secrets and names out, reports a refused write once. Add `--json results.json --threshold 1.0` in CI.

## Prove it before you trust it

`claude --plugin-dir plugins/docbrain` loads the plugin from a checkout for a session, and `claude plugin validate plugins/docbrain` checks the manifest and the skill. The repository's own live proof drives a headless Claude Code session with the skill against a test stack and checks, by API and by Ask, that a capture lands only after a yes.
