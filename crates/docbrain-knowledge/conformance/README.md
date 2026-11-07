# The `.knowledge/` conformance corpus

Two readers that implement `SEMANTICS.md` give the same answer on every case here. A case is one JSON file
under `cases/`, in a folder named for the design section it tests. The corpus is MIT, like the crate.

## A case

```json
{
  "id": "4.7-unknown-key",
  "spec": "§4.7 other unknown keys are reported once; §8 item 24",
  "note": "optional: why the case is shaped the way it is",
  "op": "parse",
  "tree": { ".knowledge/records/k3v9q2x7m1ab-replicas.md": "---\nknowledge: 1\n…" },
  "expect": { "valid": true, "unknown_keys": ["owner"] }
}
```

| field | meaning |
|-|-|
| `id` | unique across the corpus; the file is named `<id>.json` |
| `spec` | the design section, row or item the case proves; it starts with `§` |
| `note` | optional prose; a reader ignores it |
| `op` | `parse`, `status`, `context` or `gate` |
| `tree` | the tree the op reads: path → entry |
| `base` | the base tree, for `gate` only |
| `paths` | the changed paths, for `context` only |
| `expect` | the answer, in the op's shape below |

Any other field fails the case, so a typo cannot hide an expectation.

**Tree entries.** A path is a tree path (`/`-separated, no leading `/`). An entry is one of:

- a string: a regular file holding that UTF-8 text;
- `{"base64": "…"}`: a regular file holding those bytes (not UTF-8, or bytes JSON cannot carry);
- `{"exec": "…"}`: an executable regular file holding that text;
- `{"symlink": "<target>"}`: a symbolic link;
- `{"submodule": "<hex>"}`: a submodule entry at that commit.

## `parse`

The tree holds exactly one file, under a `.knowledge/records/` directory. Its scope is the directory above the
first `.knowledge/records/` on its path (`""` for the root folder). `expect` is one of:

- `{"valid": true, "unknown_keys": [...]}` — the record is read, and these unknown keys are reported, in
  file order; a check's unknown key is named `checks[<i>].<key>`, counting from 0;
- `{"invalid": "<code>", "line": n, "col": n}` — the record is invalid with that reason code
  (`SEMANTICS.md` §Reasons); `line` and `col` count from 1, a column counts Unicode scalar values, and they
  are omitted where the position depends on the reader's YAML parser;
- `{"newer": n}` — the record's `knowledge` is `n`, newer than schema 1.

## `status`

Every folder in the tree, every record in it, and each record's state (`SEMANTICS.md` §States). `expect` has
these sections; one left out is empty, so a case states everything the reader reports:

| section | shape |
|-|-|
| `records` | path → `{"state", "reason"?, "by"?, "checks"?}` for every record in the tree, none left out |
| `ignored` | the entries under a `records/` directory that are not records, in byte order of path |
| `duplicates` | id → the paths of the records holding it, in byte order |
| `problems` | path → why its `supersedes` has no effect: `itself`, `not-in-folder` or `duplicated` |
| `cycles` | each supersession cycle as its ids, from the smallest, the cycles in order of that id |
| `folder_wide` | the paths of the records that are context for their whole folder |

A record's `state` is `holds`, `broken`, `stale`, `unverifiable`, `superseded` or `invalid`. `reason` is the
invalid reason's code, or `newer-schema` for a record of a newer schema; `by` is the superseding record's id.
`checks` lists each check's `{"state", "reason"?}` in file order, a reason given for every state but `holds`;
a superseded, invalid or newer record has none. A record written without `checks` has them left uncompared.

## `context`

The case gives `paths`, and the reader routes them at the cap of 20 (`SEMANTICS.md` §Routing). `expect` is:

| Key | Holds |
|-|-|
| `order` | the shown records' ids in order; a record whose id cannot be read is named by its path |
| `more` | how many routed records the cap hides; left out, 0 |
| `more_broken_or_stale` | of those, how many are `broken` or `stale`; left out, 0 |

## `gate`

`base` is B and `tree` is H; the reader judges the change between them (`SEMANTICS.md` §The gate), computing
the changed paths from the two trees' entries. `expect` has these sections; one left out is empty, and
`broken_before` left out is 0:

| section | shape |
|-|-|
| `findings` | every finding in report order: `{"class", "rule"?, "change"?, "from"?, "to"?, "id", "record", "anchor", "reasons", "same_content"?}` |
| `change_findings` | in report order, each one of `{"duplicate-id": {"id", "paths"}}`, `{"supersedes": {"path", "problem"}}`, `{"cycle": [ids]}`, `{"secret-shape": {"path", "rule", "line", "col"}}` |
| `broken_before` | the count of excused checks the change touched |
| `knowledge` | `{"added", "edited", "removed", "followed", "new_unverifiable"}`, each left out empty: record paths in byte order; `[old, new]` anchor pairs; `{"record", "check", "reason"}` with the check's index from 0 |

`class` is `broken` (with `rule` 1 or 2), `changed` (with `change`, the kind's code, and `from` and `to` for
`anchor-moved` and `expectation-changed`) or `stale`. `record` is the judging record's path (H's under rule 1
and for `stale`, B's under rule 2 and for `changed`), `anchor` the anchored path from the root (`<file>#<key
path>` for a `config_key`, its first layer). `reasons` lists each reason's code, the text before `: `, in the
order the rules give them. `same_content` is given only when there is one. A finding's title and first
paragraph are not compared: they are the record's own.

A reader that does not implement an op yet counts its cases as pending; it never skips an op it claims.

## Running it

`cargo test -p docbrain-knowledge --test conformance` runs every case this crate implements and prints each
failing id. Another reader walks `cases/**/*.json` in byte order of path and compares its own answers.

## What covers each rule

`coverage.tsv` has one line per rule of the design's §4.3, §4.6, §4.7, §5.3, §7.1, §7.2, §7.5 and §7.6,
and one per §8 item (`§8.1` to `§8.87`). Its columns are `ref`, `rule` and `cover`; lines starting `#` are
comments. `cover` is a space-separated list of tokens:

| token | meaning |
|-|-|
| a case id | a case under `cases/` proves the rule |
| `test:<file>::<name>` | a `#[test]` of this crate proves it, for a bound too large for a case |
| `cli:#<issue>`, `server:#<issue>`, `job:#<issue>`, `skill:#<issue>` | the part outside the evaluator, owned by that issue |
| `git` | git's own behaviour; no reader acts |

`cargo test -p docbrain-knowledge --test coverage` fails on a token that names nothing, a line without a
token, or a §8 item without a line.
