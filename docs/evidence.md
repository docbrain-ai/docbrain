# Evidence Bundles

`docbrain evidence export` produces a sealed, offline-verifiable bundle of your
organization's DocBrain history — captures, decisions, approvals, premise
verdicts — that anyone can check **without trusting DocBrain, your server, or
your database**:

```
docbrain-verify bundle.dbev
```

returns one of three verdicts — `VALID`, `TAMPERED`, or `CANNOT_VERIFY` — with a
report explaining exactly why. From organizational memory to organizational
evidence.

## The honest claim: one-directional soundness

**A `VALID` verdict is never wrong, within its stated meaning.** The design
does not claim the converse: tampering does not always read `TAMPERED`. An
attacker can always demote proof of tampering down to `CANNOT_VERIFY` (corrupt
one more byte, withhold an anchor), and a journal forked at its very first
record is indistinguishable from a single honest bundle taken alone.
Soundness runs one way, and every report says so.

This matters for how you read a bundle: `VALID` is a strong, narrow claim.
`CANNOT_VERIFY` is not an accusation — it just means the bundle can't prove
what it's being asked to prove. `TAMPERED` is the only verdict that asserts
something went wrong, and the report always lists which specific check failed.

## The three verdicts

| Verdict | Exit code | Meaning |
|---|---|---|
| `VALID` | `0` | Every signature, hash link, and chain boundary checked out. The bundle is internally consistent and unmodified since it was signed. |
| `TAMPERED` | `1` | A specific, provable inconsistency was found — a bad signature, a broken hash chain, an unauthorized key-chain record. The report names which one. |
| `CANNOT_VERIFY` | `2` | Nothing was proven either way — missing content with no matching erasure record, an unrecognized key, a malformed or non-conforming container, an unsupported format. This is the closed-world default: any input state the verifier doesn't recognize also lands here, never silently as `VALID`. |

*(Verified against `crates/docbrain-evidence/src/verdict.rs` — the `Verdict`
enum and its `exit_code()`, and confirmed live: `docbrain-verify` and the
Python reference verifier both returned exit `0` on a valid corpus bundle,
`1` on a tampered-signature bundle, and `2` on an unknown-key bundle.)*

## What `VALID` does NOT mean

A `VALID` verdict proves the **integrity, continuity, and provenance** of what
is inside the bundle — nothing more. Specifically, it does **not** mean:

- **"Compliant."** No regulation is checked or certified by the verifier. The
  bundle is evidence you can *present* toward a compliance question; it is
  never a certification that you *are* compliant with anything.
- **"Complete."** A bundle only ever evidences what actually flowed through
  DocBrain. If a decision was made outside DocBrain and never captured, no
  bundle can conjure it — the verifier only speaks to what is present.
- **"The system's own audit log."** If you're asked to produce the automatically
  generated logs of a high-risk AI system, DocBrain's journal is not that log
  unless you specifically route those events through DocBrain. What the
  journal evidences is your organization's own knowledge and decision
  lifecycle *about* such systems.
- **A guarantee that nothing was ever tampered with, anywhere, forever.**
  Soundness is one-directional (see above) — `VALID` means no tampering was
  *detected*, not that none is possible under every conceivable attack.

Every report — `VALID` or not — prints this negative space explicitly:

> Not a runtime agent-action logger; not an IT-general-controls platform; not
> field-level redaction (v2, Merkle epochs + salted field commitments); not a
> compliance certification; not "the Art 12 log" of systems that log
> elsewhere.

*(Verbatim from `NEGATIVE_SPACE` in `crates/docbrain-evidence/src/verdict.rs:156-158`. "v2" refers to a future format version of DocBrain's own bundle spec, not any external standard.)*

## Verify a bundle, offline, in one line

You do not need DocBrain installed, a server, an API key, or a network
connection to verify a bundle. Two independent verifiers exist so you never
have to trust ours alone — a Rust CLI and a dependency-free Python script that
implement the identical check, byte-for-byte.

**Python — nothing but the standard library:**

```bash
python3 tools/verify_dbev.py bundle.dbev
```

Real, unedited output — run against the `valid.dbev` bundle shipped in this repo's
own test corpus (`tests-evidence/corpus/valid.dbev`):

```
Verdict: VALID (row 1, valid)
Reason: all checks passed
Scope: range [0, 5], classes ['evidence-record']
Counts: 5 records, 0 withheld-erased, 0 closure
Anchor tier: none
Negative space (what this does NOT prove):
Not a runtime agent-action logger; not an IT-general-controls platform; not field-level redaction (v2, Merkle epochs + salted field commitments); not a compliance certification; not "the Art 12 log" of systems that log elsewhere.
```

Add `--json` for the machine-readable verdict (every check listed
individually, never collapsed into one summary line):

```bash
python3 tools/verify_dbev.py bundle.dbev --json
```

The exit code **is** the verdict — script it directly: `0` VALID, `1`
TAMPERED, `2` CANNOT_VERIFY, `3` a CLI-level failure (e.g. the file doesn't
exist — not a verdict at all).

**Rust — the standalone `docbrain-verify` binary**, if you'd rather hand an
auditor a compiled, dependency-free binary instead of a Python script (same
verdicts, same exit codes, same report):

```bash
docbrain-verify bundle.dbev
docbrain-verify bundle.dbev --json
```

Both were run against DocBrain's own test corpus for this documentation —
`valid.dbev` returned `VALID`/exit `0` on both verifiers; a bundle with a
forged record signature returned `TAMPERED`/exit `1`; a bundle referencing an
unrecognized signing key returned `CANNOT_VERIFY`/exit `2`.

## Producing a bundle

From a machine that can reach your DocBrain server (this is the *only*
evidence operation that touches the network — verification never does):

```bash
docbrain evidence export --preset logs-6m -o bundle.dbev
docbrain evidence export --range 0,1200 -o bundle.dbev
```

`--range` (explicit checkpoint positions) and `--preset` (a named retention
window) are alternatives — you use one or the other, never both. Once you
have a bundle, everything else is offline:

```bash
docbrain evidence verify bundle.dbev              # same verdict as docbrain-verify
docbrain evidence verify bundle.dbev --against earlier.dbev   # fork detection
docbrain evidence why <record-id> bundle.dbev     # reconstruct one decision's story
docbrain evidence tables bundle.dbev out.csv      # populations CSV, VALID bundles only
```

`why` and `tables` both refuse to run on a bundle that isn't `VALID` — they
never render content the verifier couldn't first stand behind.

### What ships today, honestly

v1 ships the verifier's human-readable text report (`render_human`, shown
above) and machine-readable JSON/CSV populations. A signed PDF/A Evidence
Report is on the roadmap for a future release — if you need a document to
attach to a filing today, generate the text report and CSV populations from
a `VALID` bundle and attach those.

## What's inside a bundle

A `.dbev` file is a restricted, ZIP-compatible container the verifier checks
byte-first, before interpreting anything inside it: every member's declared
hash is checked against the signed manifest *before* that member is read as
JSON. STORE only (no compression — an inflater in front of an
authentication check is exactly the kind of thing two independent verifier
implementations could disagree about), no zip64 (bundles are capped at the
classic ZIP 4 GiB limit), no encryption, UTF-8 names only, a fixed set of
member paths:

```
manifest.json          signed manifest: scope, counts, checkpoint, hash of every other member
journal/epoch-*.jsonl  the exported record range, exact signed bytes
journal/closure.jsonl  erasure records from outside the exported range, closing the scope
checkpoints.jsonl      the full checkpoint chain, from genesis
trust/keys.jsonl       the full signing-key chain, from genesis (rotations included)
anchors/*              timestamp/witness receipts, each bound to a checkpoint
content/<record-id>    the actual record content, salted and hashed
```

Every export carries the full key chain and checkpoint chain from genesis —
even a narrow date-range export roots all the way back, so nothing in a
range export has to be taken on faith.

## Regulation-agnostic engine, pluggable compliance profiles

A fair question: is any of this hardcoded to the EU AI Act? If the Act is
amended, or you need SOC 2, ISO 42001, or FDA/SEC evidence instead — is that a
different product?

No. The cryptographic core — the journal, the hash chain, the key chain,
checkpoints, the container format, the verifier — references **no
regulation by name**. It proves integrity, continuity, and decision
provenance, full stop, which is what every evidentiary regime (SOC 2, ISO
42001, FDA Part 11, SEC 17a-4, the EU AI Act) ultimately asks for underneath
its specific vocabulary.

What's regulation-specific lives entirely in a **compliance profile**: a
named set of retention windows (each citing the statutory basis it mirrors)
plus a report-template selector — data, not engine logic. Today's registry
ships exactly one profile:

| Profile | Presets |
|---|---|
| `eu-ai-act` (default) | `logs-6m` — 183 days, modeled on Art 19 / Art 26(6) operational log retention; `docs-10y` — 3650 days, modeled on Art 18(1) technical-documentation retention |

A regulation change, or an amendment to the AI Act itself, is an edit to
this profile's data — never a change to the engine, the chain, or what
counts as `VALID`/`TAMPERED`/`CANNOT_VERIFY`. Adding a second regime (SOC 2,
ISO 42001, FDA, SEC) is adding a new profile entry alongside it, with its own
presets and report template, on the same engine.

**The hard limit that no profile changes:** a profile only changes which
range gets exported and how it's labeled. It can never make a bundle
evidence something that never actually flowed through DocBrain.

### What the bundle can honestly say about the EU AI Act

The table below is deliberately framed as **supports / exceeds**, never
"compliant" or "required by" — no regulation mandates cryptographic
tamper-evidence, and no bundle certifies anything on its own.

| An auditor asks | Statutory basis | What the bundle honestly answers |
|---|---|---|
| "Demonstrate conformity on request" | Art 21(1) | The bundle plus the verifier-generated text report, in the language the request was made in |
| "Provide access to the automatically generated logs" | Art 21(2) → Art 12(1) | **Scoped:** the journal is not the high-risk system's own Art 12 log unless you route those events through DocBrain. It independently evidences your organization's knowledge/decision lifecycle *about* such systems |
| "Show lifecycle changes and the design decisions behind them" | Art 11, Annex IV §2/§6, Art 18(1) | Fragment version history plus decision records with approver identity and timestamps — the `docs-10y` preset |
| "Demonstrate log retention" | Art 19(1) / Art 26(6) | The `logs-6m` preset; checkpoint continuity demonstrates gap-free retention for what flows through DocBrain |
| "When did you become aware, and when did you establish the causal link?" | Art 73 | Both moments, each independently timestamped and tamper-evident, when both are journaled |
| "Provide data supporting post-market monitoring" | Art 72(2) | Journaled captures, decisions, and premise transitions feeding your monitoring plan |

No fixed statutory deadline exists for responding to a documentation
request, and the Act does not mandate signatures or tamper-evidence anywhere
in it — DocBrain exceeds what's required because tamper-evidence makes the
evidence more defensible, not because it's asked for.

## Learn more

- [`docs/configuration.md`](configuration.md) for the `EVIDENCE_*` server
  settings that turn this on and tune it.
