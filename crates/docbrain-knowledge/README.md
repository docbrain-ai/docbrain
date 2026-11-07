# docbrain-knowledge

The open evaluator of `.knowledge/` records. It parses a record, computes every check's and record's state in
a tree, routes records to the paths an edit touches, judges a change between two trees with the two rules,
and screens record files for well-known secret shapes. It is synchronous, offline and pure over a `Tree`, and
it never runs anything a tree contains. Every DocBrain surface (the CLI, the hook, MCP, the server's sync)
calls it for meaning.

The API in five lines:

```rust
let parsed = record::parse_record(scope, bytes);                 // one record file: valid, or invalid at a line and column
let status = folder::status(&tree)?;                             // every record in the tree, with its state and reasons
let routed = route::context(&tree, &paths, bounds::CONTEXT_CAP)?; // the records for these paths, in order, then the cap
let report = gate::gate(&base, &head, &changed)?;                // the change from base to head, judged by the two rules
let hits = secret::screen(text);                                 // secret shapes by rule, line and column, never the text
```

A `Tree` is a `MemTree`, or a commit's tree from `git::GitRepo`, which reads objects through the repository's
own git (`discover`, `resolve_commit`, `merge_base`, `commit_tree`, `changed_paths`).

[`SEMANTICS.md`](SEMANTICS.md) is the published meaning of schema 1; where this code disagrees with it, the
code is the bug. The conformance corpus is [`conformance/`](conformance/README.md): `cargo test -p
docbrain-knowledge --test conformance`.

MIT licensed ([`LICENSE`](LICENSE)).
