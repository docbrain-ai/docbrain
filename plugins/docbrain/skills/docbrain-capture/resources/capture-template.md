# Capture template

Fill every line that applies; delete the ones that do not. One capture per fact.

```
Type:        decision | fact | caveat | procedure | context
File:        <repo-relative path the knowledge concerns>        Lines: <start-end, if it has a place>
Space:       <team or project space, if the organization uses them>

<What: one sentence a stranger understands without the conversation.>

<Why: the reason, the constraint, or the symptom that led here. For a decision, what was rejected and why.>

<How: the exact command, path, flag, value or steps. Numbered when order matters.>

Premises (paths that must exist for this to hold — each one checked in the repository before the draft is shown):
  - <path>
  - <path>
```

What the fields become on the wire:

| Template line | `docbrain_annotate` field |
|---|---|
| Type | `fragment_type` |
| File, Lines | `file_path`, `line_range` — and the exact lines at that range, read from the file, as `code_snippet` |
| Space | `space` |
| What, Why, How | `annotation` (one text, in that order) |
| Premises | `premises: [{ "premise_type": "path", "expression": "<path>" }]` |

For the *why* of a commit, use `docbrain_commit_capture` instead: `intent` (the Why), `file_paths`,
`commit_message`, and a `diff_summary` in words. `file_paths` carries the Premises line there — each
file becomes a `path` premise — and `premises` takes any the reasoning rests on beyond them.
