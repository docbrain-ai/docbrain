# docbrain-evidence

The offline verifier for DocBrain **evidence bundles** (`.dbev`) — a sealed, signed, hash-chained record of what your knowledge said and when, that **anyone can verify without trusting DocBrain**.

**This crate is the verifier, not the exporter.** Bundles are produced server-side; this MIT crate — and the standalone `docbrain-verify` binary built from [`docbrain-cli`](../docbrain-cli/) — only *checks* them, offline, with no server, no network, and no DocBrain install. It returns one of three verdicts:

| Verdict | Exit | Meaning |
|---|---|---|
| `VALID` | 0 | Every signature, hash, and chain link checks out. |
| `TAMPERED` | 1 | Something was altered — it names the record and the exact check that failed. |
| `CANNOT_VERIFY` | 2 | The bundle is malformed, or references a key/anchor it doesn't contain. |

## Verify a bundle

```bash
# The Rust binary (built from this workspace)
docbrain-verify evidence.dbev

# Or the dependency-free Python reference verifier — bare python3, nothing to install
python3 tools/verify_dbev.py evidence.dbev
```

Add `--json` for machine-readable output. Both exit `0` / `1` / `2` per the table above.

## Two implementations, one answer

A verifier that says "trust me" is worthless to an auditor. So the bundle is checked by **two codebases that share no code** — this Rust crate and a [stdlib Python script](../../tools/verify_dbev.py) — proven to return the **byte-identical verdict on every input**, honest or hostile, via exhaustive differential testing: every byte of a bundle flipped, thousands of adversarial numbers, the full container and signature surface, with zero disagreement. When a second, independent implementation agrees, the verdict isn't an opinion — it's a fact about the bytes.

## What's inside

Domain-separated SHA-256 hash chains, DSSE envelopes over a pinned ed25519 verifier, a restricted STORE-only ZIP container profile, and a uniform strict-JSON parse profile enforced identically in both verifiers. The trust root is a genesis key you confirm out-of-band — never DocBrain's word for it.

A `VALID` verdict proves the evidence is **intact and provenance-continuous** — not that you are "compliant," and only for what actually flowed through DocBrain. That limit is printed on every bundle. The compliance edges are a data-driven profile: the EU AI Act ships as profile #1; adding SOC 2, ISO 42001 or SEC is a new profile, not a new engine.

Full concept guide, verdict taxonomy, and profile design: [docs/evidence.md](../../docs/evidence.md).

## License

MIT — fully open source, so the party who distrusts you can read it, build it, and run it themselves.
