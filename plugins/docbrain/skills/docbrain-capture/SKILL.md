---
name: docbrain-capture
description: Capture what this conversation worked out into DocBrain, so the whole organization can find it later and DocBrain can tell when it stops being true. Use when a piece of work has produced knowledge written nowhere — a decision and its reason, a fix that took digging, a caveat, a procedure (how we deploy, rotate, roll back, get access), how a system is actually wired —, when a commit is about to be made whose reason is not in its message, or when asked to "capture this", "write this down", "document how we did that". Checks DocBrain first, drafts in the team's words with the file and the facts it rests on, shows the draft, and writes only after a yes.
allowed-tools: Read
license: MIT
---

# Capture knowledge into DocBrain

The knowledge worth keeping is created in the middle of work and gone by Friday. This skill turns the
moment it exists into a record a stranger can find, act on, and that DocBrain can flag the day it
stops being true. It works through the DocBrain MCP tools. Nothing leaves the session until the
person says yes; everything else is automated.

## When to run

- The person asks: "capture this", "write this down", "document how we did that", "/docbrain-capture".
- A piece of work just produced something that is written nowhere:
  - a decision and why (and what was rejected),
  - a fix that took digging: the symptom, the cause, the change, the trap,
  - a caveat: "X looks like Y but is not, because…",
  - a procedure: how we deploy, rotate, roll back, get access, run the migration,
  - how a system is actually wired: which cluster, which queue, which flag, who consumes it.
- A commit is about to be made whose *why* is not in the message.

Offer once, at a natural breakpoint, in one sentence: "Worth capturing to DocBrain: <what>?". Never
interrupt a task to offer. Never offer twice for the same thing. If the person says no, drop it.

## Step 1 — check before writing

1. Call `docbrain_context` with `file_paths`, an array of the repo-relative paths the knowledge
   concerns — always the array, even for one path; bare paths such as `deploy/ingress.yaml`, never
   `deploy/ingress.yaml:42`. When no file is involved, call `docbrain_ask` with the question a future
   engineer would type instead; never call `docbrain_context` without paths.
2. If DocBrain already has it: say so, cite it, and offer the delta as an update instead of a duplicate.
   When the capture is marked as recorded by you and the work just proved it wrong or out of date, do
   not write a second one: draft the corrected text (and premises) and, after the yes, call
   `docbrain_revise` with its `id`. If it should simply stop being served, ask for a one-line reason
   and call `docbrain_revise` with `withdraw_reason`. Someone else's capture is credited to its author
   on its page: say so and suggest they ask that author; never revise it.
3. If the tools are not available in this session: say "DocBrain's tools are not connected in this
   session" and stop. Do not search the repository instead. Captured knowledge does not live in the
   repository, so an absent connector looks exactly like an empty knowledge base and is not one.

## Step 2 — draft, in the team's words

One capture per fact. Fill `resources/capture-template.md`; finished examples are in
`resources/examples.md`. The rules:

- Say what, why and how, with the exact command, path, flag and value. A capture a stranger can act on
  without asking anyone.
- Anchor it. Give the file and line range it concerns (`file_path`, `line_range`). Read the file first:
  the lines you name must hold what the capture says. If the change described is not in the file, say
  so and ask where it lives; never anchor a capture to lines that do not contain it. Pass those exact
  lines as `code_snippet`: DocBrain hashes them and flags the capture the day that code changes.
- `docbrain_commit_capture` is for one moment: the person is about to commit, or asks to record why
  a commit was made, and the reason is not in the message. The signals are the person's own words —
  "about to commit", "before I commit", "the message will say", "record why I changed" — and when
  one is present the diff is the anchor: the intent, the files and the message go through
  `docbrain_commit_capture`, never an annotation on lines that are about to change. Send `file_paths`:
  DocBrain turns each changed file into a premise and flags the capture the day one of them moves or
  disappears, so the file list is what keeps a commit capture checkable — without it nothing can ever
  tell whether the reasoning still holds. Everything else — a fix, a procedure, a caveat, a fact —
  goes through `docbrain_annotate`, anchored to the lines that hold it. A change that is already
  committed is not a commit capture: it is a fact about the file, whatever commit put it there, so do
  not go looking for its commit — read the file.
- When no file holds the knowledge — a process, a fact about how two systems are wired — anchor to the
  document a reader would open first (a README, a runbook, a manifest, a config) and say so in the
  capture. If no repository holds anything for it, use `docbrain_commit_capture` with the intent alone;
  it is served at once and labelled *Not checked against code*, so readers know nothing checks it.
- Declare the premises it rests on: paths that must exist for the capture to hold
  (`premise_type: "path"`). Both capture tools take them; on a commit capture they are what the
  reasoning rests on beyond the files it changed. Every premise path must exist in the repository right
  now — check it before you show the draft; a path that does not exist is not a premise, and a wrong one
  goes silent forever. DocBrain re-checks the paths and flags the capture the day one disappears. A
  capture is served the moment it is written, labelled with who wrote it and what checks it: with
  premises that hold it reads *Checked against code*; with none it reads *Not checked against code*.
  Declaring the premises is what earns the first label.
- When the capture answers an open question DocBrain suggested (`docbrain_suggest_capture`), pass that
  question's id as `gap_id`: the question closes for whoever asks next.
- Choose the type: `decision` (why something was chosen), `fact` (how something works), `caveat` (a
  gotcha or limit), `procedure` (steps — a capture with steps to run is a procedure even when it also
  explains why), `context` (background a reader needs first).
- Keep it to what was actually established. No guesses, no "probably", no restating the code.
- Leave out: hostnames from `.env` files, customer or personal names, pasted logs, chat, opinions
  without a reason, and anything the person said to keep out; secrets have their own section below.
  When in doubt, leave it out. Say what kind of thing was left out ("a key, a hostname, a name") —
  never repeat the value, not even to say it was omitted.

## Secrets

Other people retrieve what you capture, so a secret in a capture is shared with every one of them.

- Never put a secret value in a draft: no password, token, API key, private key, or connection string
  or URL with credentials. Say where it lives instead: "the database password is in the team vault",
  or the variable that carries it, `${DB_PASSWORD}`.
- Never paste a `.env` file, a credential file or a connection string with a password, not even part
  of one.
- Before you show the draft, read it for anything that looks like a credential — a value after
  `password`, `token`, `secret` or `key`, an `Authorization` header, a private key block, a URL with
  `user:password@`, a long random string that is not a hash or an id — and replace it with where it
  lives.
- If DocBrain refuses the capture for a secret, say so, take the value out and show the corrected
  draft for a new yes. Never encode, split, space out or reword the value to get past the check.

## Step 3 — ask once, then write

Show the draft exactly as it will be sent: the fields and their values, nothing hidden. Ask for a
yes. If the person asks for changes, show the edited draft in full and ask again. Write only the draft
that was shown and answered with a yes, unchanged, with `docbrain_annotate` (or
`docbrain_commit_capture`). On no, stop.

If the write fails — permission, validation, the server unreachable — say exactly what the tool
reported and stop: no silent retry, no writing anywhere else. Leave the draft in the conversation so it
can be sent once the cause is fixed. A refusal names its reason — a capture limit, a duplicate (with
the existing id), an instruction in the text, a key with no user — repeat it and do not retry; for a
duplicate, offer `docbrain_revise` on the id it gave. A refusal for a secret goes by Secrets above.

## Step 4 — report in one line, then stop

Repeat what the tool reported: the capture's id, its status in the tool's word (`indexed`: it is
served now and Ask can answer from it), and any warning it returned (for example a premise path that does not exist). Its page in the console says
who recorded it, what checks it and where it helped.
Do not summarise the capture again. Do not offer another one.
