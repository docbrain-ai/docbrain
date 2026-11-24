# `.knowledge/` semantics, schema 1

**Status:** schema 1 (`knowledge: 1`), complete. Every edge case and every rule that `conformance/coverage.tsv`
lists is proven by the conformance corpus or names its owner outside this crate (§Conformance).

**Changelog.** 2026-11-07: schema 1, the first complete edition.

This is the published meaning of a `.knowledge/` folder: what a record is, what each check's and record's
state is in a tree, which records route to a path, and how a change is judged. It is the whole
meaning: a second reader needs nothing else to agree with the first. Two readers that implement this document give the same answer on every case of the conformance
corpus (`conformance/`). Where the code of this crate disagrees with this document, the code is the bug.

## Sections

1. Reasons
2. Bounds
3. Paths and globs
4. Hidden text
5. The record
6. Values and comparison
7. Resolving values
8. States
9. Routing
10. The gate
11. Cost
12. Secrets
13. Edge cases
14. Conformance

## Reasons

Every reason is a stable code: kebab-case, never reworded. A code is unique within its state, and a reader
always reports the state beside it. Where a code carries a key, the part after `:` is the front-matter key
or the bound's name exactly as written.

### `unverifiable` (a check whose value cannot be judged; never reported as `holds`)

| code | meaning |
|-|-|
| `format` | the anchored file is in no format with a resolver, or does not parse |
| `template` | the anchored scalar contains `{{` or `{%` |
| `expression` | the anchored HCL value is an expression, not a literal |
| `merge-key` | the key is absent from a YAML mapping that has a `<<` merge key |
| `lfs-pointer` | the anchored file is a Git LFS pointer |
| `submodule` | the anchored path is, or lies inside, a submodule entry |
| `symbolic-link` | the anchored path is, or lies through, a symbolic link |
| `encoding` | the anchored file is not UTF-8 |
| `not-a-scalar` | an ordered or equality comparison met a map or a list |
| `not-comparable` | no reading applies to the value and the literal |
| `number-or-version` | the decimal and version readings disagree |
| `range` | an ordered comparison on a string that is a version range |
| `ambiguous` | a selector, document or `code` text matches more than once |
| `defined-twice` | the key is defined twice (a duplicate key, or two merged files) |
| `unknown` | an unknown check kind or op |
| `newer-schema` | the record's schema is newer than this reader |
| `yaml-1.1` | a plain YAML scalar that YAML 1.1 and YAML 1.2 read differently |
| `tag` | the value is resolved onto or through an explicitly tagged YAML node |
| `bound:<name>` | a bound was exceeded; the name is one of the bounds below |

### `broken` or `stale` (a check judged false; `broken` under `must`, `stale` under `assumes`)

| code | meaning |
|-|-|
| `missing-file` | no entry matches the anchored file or path |
| `missing-key` | no layer defines the key |
| `false` | the expectation is false; the value is shown, cut at 200 bytes, or as `(withheld: secret-shape)` when the whole value holds a secret shape (§Secrets), wherever it lies |

A missing file or key is never `unverifiable`.

### `invalid` (a record that is not read)

| code | meaning |
|-|-|
| `parse` | not UTF-8, no front matter, or YAML that does not parse |
| `quotes` | a string that is not single-quoted |
| `schema-integer` | `knowledge` is not a plain positive integer |
| `id` | an `id` or `supersedes` that is not 12 Crockford base32 characters |
| `example-id` | an id that begins `0000` (reserved for documentation) |
| `kind` | `kind` is not one of the four |
| `title` | `title` is empty or holds a line break |
| `empty-body` | the body is empty |
| `hidden-text` | a hidden-text character anywhere in the file |
| `conflict-markers` | merge conflict markers in the file |
| `confinement` | a path that leaves the folder's parent directory, or names `.git` |
| `symlink-record` | the record file is a symbolic link |
| `verbs` | a check without exactly one of `must`, `assumes` |
| `anchor` | a check with no anchor (and no unknown kind), two anchors, or `text` without `code` |
| `key-path` | a `config_key` key path that does not parse |
| `glob-dir` | a `config_key` file glob with a wildcard before its last segment |
| `duplicate-key` | a front-matter key written twice |
| `yaml-feature` | an anchor, alias, tag or merge key in front matter |
| `missing-key:<key>` | a required key is absent |
| `type:<key>` | a key holds the wrong type |
| `bound:<name>` | a bound was exceeded |

## Bounds

An exceeded bound is an `unverifiable` check (or an `invalid` record) naming it, never an error.

| name | value |
|-|-|
| `record-file` | 64 KiB, from the tree entry's size before reading, and again on the bytes read |
| `front-matter` | 8 KiB |
| `depth` | front-matter nesting depth 32 |
| `checks` | 100 checks per record |
| `anchored-file` | per format, from the tree entry's size before reading and again on the bytes read: JSON 64 MiB, YAML 32 MiB, TOML 16 MiB, HCL 16 MiB, and a glob's HCL files together 16 MiB (they merge into one value); the file a `code` check reads, whatever its name, 64 MiB |
| `resolver-depth` | 64 |
| `alias-expansion` | 1,000,000 for what one anchored YAML file's anchors and aliases copy: each anchor keeps a copy of its node and each alias makes one, and a copy is charged its size, one per node and a scalar also its bytes |
| `records-per-pass` | 100,000; the excess, in byte order of path, is counted by folder |
| `code-text` | a `code` text keeps at least 24 Unicode scalar values after ASCII whitespace is removed, and is at most 2,048 bytes |
| `number-text` | 4,096 bytes for a number in an anchored file, as written, when either YAML version or the format reads it as a number (converting a hex or octal number costs the square of its length) |
| `hcl-assignments` | 20,000 `=` and `:` characters in one anchored HCL file, counted before it is parsed (the parser's cost for an object grows with the square of its items; 20,000 items in one object parse in 0.7 s) |

`context` shows at most 20 records before its overflow line (§Routing); that cap is not a reason.

**Measured** (2026-11-07, release build, one laptop; each moved value is recorded with its
measurement). The real files are every file of each format in twelve infrastructure repositories (4,437 HCL,
244 JSON and 9,478 YAML files) and in this repository (431 JSON, 16 TOML, 88 YAML). The time is `status` on a
tree holding one record whose check reads a generated file of exactly the bound, shaped like that format's
largest real files (lockfiles), best of three:

| bound | largest real file | at the bound | memory at the bound |
|-|-|-|-|
| JSON 64 MiB | 2.32 MiB | 0.29 s (`package-lock.json`) | 570 MB |
| YAML 32 MiB | 0.24 MiB | 0.89 s (`pnpm-lock.yaml`) | 738 MB |
| TOML 16 MiB | 0.16 MiB | 0.37 s (`Cargo.lock`) | 501 MB |
| HCL 16 MiB | 0.11 MiB | 0.30 s (4,800 resources) | 201 MB |
| a `code` file 64 MiB | 6.80 MiB (any other file) | 0.10 s | 382 MB |

The memory column is the lockfile shape. The worst shape is a list of one-digit numbers: each element is held as
a 40-byte node, 20 times its two bytes, so a JSON file of 64 MiB at the bound holds 1,344 MB while it is read
(2.9 s). A pass keeps at most 256 MiB of parsed files beside the one being read: 24 such files of 8 MiB under
one glob, read by three checks, peaked at 328 MB of heap (4,032 MB before the pass had a budget, one more file
held for each file added) and took 26 s instead of 10 s, each file parsed once per check. These are the heap
the evaluator holds (2026-11-07, release build, a counting allocator). macOS's allocator keeps freed large
blocks in a cache that counts in the process's footprint (3.9 GB in the run above,
330 MB with `MallocLargeCache=0`): the program that embeds this crate decides how its allocator returns memory.

The old single bound of 32 MiB made a 50 MiB `package-lock.json` `unverifiable`. The alias budget was 100,000
nodes, charging a copy one node whatever its text, so a 72 KiB file aliasing one 64 KiB scalar 2,000 times
used 267 MB to read while the count stood at 2,000; it now stops that file at 2.3 MB. The heaviest real charge
is 2,322. At the bound the worst cases are 58 ms and 67 MB (an anchored list of 500 one-letter scalars aliased
998 times) and 1.5 ms and 4.5 MB (a 1,000-byte scalar aliased 998 times), under what the YAML file bound
already admits.

## Paths and globs

**Paths.** A folder's scope is its parent directory (`""` for the root folder). Every path in a record, in
`applies_to` and in every anchor, is written relative to the scope and is joined with it before it is looked
up, so a record applies to and anchors only entries of its own scope. Paths use `/` and are compared byte for
byte and case-sensitively. A `.` segment names the directory it is in and is dropped first, so `./a` and
`a/./b` are `a` and `a/b`, and a path of nothing but `.` segments is empty. A written path is rejected, making
the record `invalid: confinement`, when it starts with `/`, `~` or a drive letter (`[A-Za-z]:`), contains `\`,
or has a segment that is empty, `..` or `.git`. An empty path is `invalid: parse`. Paths name entries of a tree, never files on disk; a directory is
not an entry.

**Globs**, in `applies_to`, `path` and the file part of `config_key`:

- `*` matches any run of characters, the empty run included, except `/`;
- `?` matches exactly one character (one Unicode scalar value) except `/`;
- `**` as a whole segment matches zero or more whole segments; as the **last** segment it matches one or
  more, because a path names an entry and `docs/**` names the entries under `docs/`, not an entry `docs`;
- `**` inside a segment (`a**`) is two `*`;
- names that start with a dot match like any other name;
- there are no character classes, braces or escapes: `[`, `]`, `{` and `}` are literal.

A path without `*` or `?` is literal and matches only itself.

**`config_key` file globs.** A wildcard may appear only in the last segment of a `config_key` file part
(`infra/prod/*.tf`); a wildcard in an earlier segment makes the record `invalid: glob-dir`. A glob that
matches no entry is a layer that defines nothing.

**Default `applies_to`.** A record without `applies_to` applies to each `path` anchor's path (and, when
that path is literal, everything under it, `<path>/**`), each `config_key` layer's file part, and each `code`
file. `applies_to: []` applies to nothing.

## Hidden text

A hidden-text character anywhere in a record file makes the record `invalid: hidden-text`, with
the line and column of the first one (lines and columns from 1; a column counts Unicode scalar values; a
BOM at the very start of the file is not a column). This is a charset rule, not a content scanner: a body in
any language parses. The set is named against **Unicode 17.0.0**; a later Unicode version changes it only in
a new format revision.

A scalar value is hidden text when it is:

1. a C0 or C1 control character (General_Category `Cc`), except TAB (U+0009), LF (U+000A), and CR (U+000D)
   immediately followed by LF;
2. a format character (General_Category `Cf`: the zero-width characters, the bidirectional embeddings,
   overrides and isolates U+202A to U+202E and U+2066 to U+2069, the tag characters, the soft hyphen, the
   BOM), except:
   - a BOM (U+FEFF) at the very start of the file;
   - U+200C or U+200D whose left and right neighbours are both non-ASCII scalar values that are letters
     (`L*`), marks (`M*`) or have `Emoji=Yes` — Persian, the Indic scripts and emoji sequences need
     them;
   - the implicit direction marks U+200E, U+200F and U+061C;
3. a variation selector U+E0100 to U+E01EF;
4. a variation selector (the `Variation_Selector` property: U+180B to U+180D, U+180F, U+FE00 to U+FE0F,
   U+E0100 to U+E01EF) immediately after another one; the position reported is the second.

The tables are generated by `tools/gen_unicode.py` into `src/charset/tables.rs` from these Unicode 17.0.0
files, whose sha256 the generated file records:

| file | sha256 |
|-|-|
| `ucd/extracted/DerivedGeneralCategory.txt` | `d62e5bab70ca74f099343f71224fa051cb1fdd61a1ab45c0488c44cfc0b6102e` |
| `ucd/PropList.txt` | `130dcddcaadaf071008bdfce1e7743e04fdfbc910886f017d9f9ac931d8c64dd` |
| `ucd/emoji/emoji-data.txt` | `2cb2bb9455cda83e8481541ecf5b6dfda66a3bb89efa3fa7c5297eccf607b72b` |

## The record

Conformance cases: `conformance/cases/record`, `checks`, `identity`,
`schema`, `edge-writing`, `edge-paths`. `schema/knowledge-1.schema.json` describes the front matter's shape only; this
section is the whole rule.

**Record files.** The first `.knowledge/records/` from the root on a path names a folder; the folder's
scope is its parent directory. A record file is a `*.md` entry anywhere under `records/`; subdirectories carry
no meaning, and the name `<id>-<slug>.md` is cosmetic (the front matter's `id` is the identity). A
non-`.md` entry under `records/` is not a record and is reported once.

**Keys, schema 1.** Seven top-level keys, four required:

| key | required | value |
|-|-|-|
| `knowledge` | yes | the schema, a plain integer |
| `id` | yes | 12 characters of lower-case Crockford base32 (`0-9a-hjkmnp-tv-z`); one that begins `0000` is reserved for documentation and makes the record `invalid: example-id` |
| `kind` | yes | `decision`, `fact`, `procedure` or `question`, exactly |
| `title` | yes | not empty, and no line break after folding; spaces alone are a title |
| `applies_to` | no | a list of path globs; `[]` routes nowhere |
| `checks` | no | a list of at most 100 checks |
| `supersedes` | no | an id, as for `id` (the `0000` reservation applies to `id` only) |

A key that starts `x-` is ignored silently. Any other unknown key is reported once, by name, in file order; a
check's unknown key is named `checks[<i>].<key>`, counting checks from 0.

**A check.** Known keys: `path`, `config_key`, `code`, `text`, `must`, `assumes`.

- Exactly one verb, `must` or `assumes`, else `invalid: verbs`.
- At most one anchor (`path`, `config_key`, `code`); two or three are `invalid: anchor`. `code` needs `text`
  (`invalid: missing-key:text`); `text` beside `path` or `config_key` is `invalid: anchor`.
- No anchor but another key that is neither known nor `x-`: an unknown check kind, read and judged
  `unverifiable: unknown`. No anchor and no such key: `invalid: anchor`. An unknown kind's other keys,
  `text` included, are not checked and are not reported as unknown.
- `path`, `code`, `text` and the verb are strings. `code` is a literal path: a `*` or `?` in it is
  `invalid: type:code`. `config_key` is a string or a non-empty list of strings (layers, read in order);
  each is `<file>#<key path>`, split at the first `#`; no `#`, or a key path that does not parse, is
  `invalid: key-path`.

**Expectations.** A verb's value is read as: `exist`, exactly; else an op (`==`, `!=`, `>=`, `>`, `<=`,
`<`, the longest that matches) followed by a literal, the rest of the value with spaces and tabs trimmed
from both ends (an empty literal is a literal); an op followed at once by another `=`, `!`, `<` or `>` is
not an op. A literal that starts and ends with the same quote, `'` or `"`, is the text between that one pair,
spaces kept and nothing unescaped: `== '1.97'`, `== "1.97"` and `== 1.97` are one expectation, and `== "'a'"`
is the text `'a'` with its quotes. Anything else is an unknown op, judged `unverifiable: unknown`; the record stays valid. On `path`
and `code` the only expectation is `exist`: a known op there is `unverifiable: not-comparable`.

**Key paths.** A key path is segments separated by `.`. A segment is a bare key (any run of characters other
than `.`, `/`, `[`, `=`, `"`, and inside a selector also `,` and `]`), a quoted key (`"…"`, where only
`\"` and `\\` are escapes), or a bracket after a key or at the start: `[n]`, an index with no leading zero,
or `[k=v,k2.k3=v2]`, a selector whose keys are bare or quoted segments joined by `.` and whose values are
quoted or run to the next `,` or `]`. An empty segment, an unterminated quote or bracket, or anything after a bracket other than `.`
or `[` is `invalid: key-path`. What a key path names in each format is §Resolving values.

**Reading a record.** These steps run in order; the first that fails decides, at its position; within a step,
the first failure in file order wins.

1. A file over 64 KiB: `bound:record-file`.
2. Not UTF-8: `parse`, at the first byte that is not.
3. Hidden text anywhere (§Hidden text): `hidden-text`. A CR not followed by LF is hidden text.
4. A BOM at the start is skipped and CRLF reads as LF. Positions count lines of the text as read.
5. Conflict markers: a line starting `<<<<<<<` that a later line starting `>>>>>>>` follows:
   `conflict-markers`, at the first. A `=======` line alone is a setext heading, not a marker.
6. Front matter: the first line is exactly `---`, and the front matter ends at the next line that is exactly
   `---`; else `parse`. Everything after that line is the body.
7. Front matter over 8 KiB: `bound:front-matter`.
8. YAML 1.2, core schema. Nesting deeper than 32: `bound:depth`. An anchor, an alias or a tag:
   `yaml-feature`. A syntax error on a line `key: value` or `- value` whose value does not start with `'`,
   `[`, `{` or `#`: `quotes` at the value (YAML cannot read `must: >= 2`: `>` opens a folded block scalar,
   and single quotes are the fix); any other syntax error: `parse`. Front matter that is not exactly one
   document holding a mapping, or a mapping key that is not a scalar: `parse`.
9. A key written twice in one mapping, at any depth: `duplicate-key`, at the second. A key `<<`:
   `yaml-feature`.
10. Every value is single-quoted, except the top-level value of `knowledge`; a key is plain or
    single-quoted. Anything else (plain, double-quoted, block): `quotes`.
11. `knowledge` is a plain scalar `[1-9][0-9]*`, else `schema-integer`. Above 1, the record is of a
    newer schema and reading stops here (**A newer schema**, below).
12. `missing-key:<key>` for the first of `knowledge`, `id`, `kind`, `title` that is absent; then `id`
    (`type:id`, `id`, `example-id`), `kind` (`type:kind`, `kind`) and `title` (`type:title`, `title`).
13. `applies_to` that is not a list of strings: `type:applies_to`.
14. `checks` that is not a list: `type:checks`; a 101st check: `bound:checks`, at it; then each check as
    above (`type:checks` for a check that is not a mapping).
15. Every `config_key` key path (`key-path`).
16. Every written path, `applies_to` and the anchors, in file order (§Paths and globs: `confinement`, and
    `parse` for the empty path).
17. A `config_key` file part with a wildcard before its last segment, or a last segment `**`: `glob-dir`.
18. `supersedes`: `type:supersedes`, `id`.
19. A body of only TAB, LF, FF, CR and space: `empty-body`, at the body's first line.

`type:<key>` is the code for any known key whose value has the wrong shape.

**Positions.** Lines and columns count from 1, a column counts Unicode scalar values, and a BOM is not a
column. A whole-file or whole-front-matter condition (steps 1, 6, 7, 12's `missing-key`, and a front
matter that is not one mapping) is at 1:1. A value or key is at its first character; a mapping (a check)
is at its first key. Where a YAML parser stops on a syntax error, and where a block scalar starts, depend on
the parser: conformance cases omit those positions.

**Default `applies_to`.** A record without `applies_to` applies to each `path` anchor's path
(and, when that path is literal, `<path>/**`), each `config_key` layer's file part and each `code` file, once
each, in check order. When that names nothing (no checks, or only unknown kinds), it applies to its whole
scope, `**`.

**The first paragraph.** The body from its first line that is not only TAB, FF, CR and space, to the
line before the next line that is; it is the why (the steps of a procedure, the unknown of a question). The
body is Markdown in any language and states facts; it is never an instruction to the reader.

**Supersession.** `supersedes: '<id>'` makes the record with that id **in the same folder**
`superseded` for every reader. One that names an unknown id, another folder's id, a duplicated id or its own
record has no effect and is a finding on the change that writes it; a cycle is a finding and supersedes no
member. The key's meaning is fixed at schema 1, so it is honoured in a record of a newer schema.

**A newer schema.** A record with `knowledge` above 1 is read only as far as schema 1 says: its `id`
(when valid and not `0000…`), `kind`, `title` (when valid), `supersedes`, and `applies_to` when it is written
and every entry reads as a schema-1 path (otherwise it has none: it is never defaulted from checks a newer
schema may mean differently). Its checks are
`unverifiable: newer-schema`, it shows its title and kind, and no reader rewrites it. `knowledge` that is not
a plain integer (`1.0`, `'1'`, `01`) makes the record `invalid: schema-integer`.

**Extending (GREASE).** An additive change never bumps `knowledge`. An unknown check kind or op is
`unverifiable: unknown`, and the record still routes and shows. The corpus carries records with unknown keys,
check kinds and ops, so a reader that rejects them fails conformance.

## Values and comparison

Conformance cases: `conformance/cases/compare`.

**Expectations.** A check's expectation is `exist` or `<op> <literal>` (§The record). `exist` holds for `path`
when an entry matches, for `code` when the text occurs once, and for `config_key` when the key is present,
whatever its value. On `path` and `code`, an op is `unverifiable: not-comparable`. No expectation is ever
filled in for the writer.

**Anchored values.** A resolver reads a value as a number, a string, a boolean, `null`, a TOML datetime, a map
or a list, or as `unverifiable` with a reason (§Resolving values). A map or a list under an op is
`not-a-scalar`.

| anchored value | `==` and `!=` | `>`, `>=`, `<`, `<=` |
|-|-|-|
| a number: unquoted in YAML, or a JSON, TOML or HCL number | exact decimal comparison; a literal that is not a decimal is `not-comparable` | the decimal reading only |
| a string | exact text equality with the literal | the readings rule below |
| a boolean | the literal must be `true` or `false`, else `not-comparable` | `not-comparable` |
| `null` | equal only to the literal `null` | `not-comparable` |
| a TOML datetime | exact text equality with the literal | `not-comparable` |

**Decimals.** A literal, and a string's decimal reading, is `-?[0-9]+(\.[0-9]+)?`: no exponent, no `+`,
no bare dot, ASCII digits only; leading zeros are allowed (`007` is 7). A number in a file (a YAML 1.2 int or
float, JSON, TOML, HCL) converts exactly, exponent, hex and octal included. Comparison is exact, arbitrary
precision: `3.10` equals `3.1`, `-0` equals `0`, `1e5` equals `100000`, and no integer loses digits. A number
whose exponent does not fit a signed 64-bit integer is `not-comparable`; a number written in more than 4,096
bytes is `bound:number-text`.

**Versions.** A string is a version when it is an optional `v`, then `N(.N){0,3}` (`N` is `[0-9]+`),
then an optional `-<pre>` and an optional `+<build>`, each dot-separated identifiers `[0-9A-Za-z-]+`. The
grammar is the same for a value and a literal. Versions order by SemVer 2.0 precedence: numeric parts
numerically, missing parts 0 (`1.2` equals `1.2.0.0`); a version with a prerelease is below the same version
without one; prerelease identifiers compare left to right, numeric ones numerically, alphanumeric ones in
ASCII order, numeric below alphanumeric, and a longer list above its prefix; the build is ignored.

**Ranges.** A string is a range when it contains `^`, `~`, `||`, `,` or a space, starts with `<`, `>`, `=`
or `!`, or has a dot-separated component equal to `*`, `x` or `X` (so `1.0.0-alpha.x` is a range too). An
ordered comparison on a string value that is a range is `unverifiable: range` (anchor the lockfile); `==` and
`!=` still compare it as text.

**The readings rule.** An ordered comparison of a string value has two readings: as decimals (both sides parse
as decimals) and as versions (both sides parse as versions). A number in the file is never a version, and a
string with a leading `v`, in the file or in the literal, is a version only. The range test comes first.

- No reading applies: `unverifiable: not-comparable`.
- One applies, or both agree: that is the answer.
- Both apply and disagree: `unverifiable: number-or-version`, never a silent pass; `record` asks for a `v`
  literal when a version is meant, or the literal with as many decimals as the value when a decimal is.

| value in the file | expectation | result |
|-|-|-|
| `'-1'` (a string) | `'== -1'` | holds: the text is equal |
| `'0.8'` | `'< 0.9'` | holds: both readings agree |
| `'0.9.1'` | `'< 0.9'` | broken: only the version reading applies |
| `'3.9'` | `'>= 3.10'` | `unverifiable: number-or-version` (3.9 ≥ 3.1 as decimals, 3.9 < 3.10 as versions) |
| `'3.9'` | `'>= v3.10'` | broken: a `v` literal is a version only |
| `'0.5'` (a string) | `'<= 0.25'` | `unverifiable: number-or-version` |
| `'0.125'` | `'<= 0.250'` | holds: both readings agree |
| `2` | `'>= 2'` | holds |
| `0.5` | `'>= 0.25'` | holds: a number is a decimal only |
| `'abc'` | `'> 1.0'` | `unverifiable: not-comparable` |

**YAML plain scalars.** A single-quoted, double-quoted or block scalar is a string. A plain scalar is
read under the YAML 1.2 core schema; it is `unverifiable: yaml-1.1` when YAML 1.1 reads it differently (a
different type, or a different number), or when it is `y`, `n`, `yes`, `no`, `on` or `off` in any case.
Numbers are compared by exact value, so `00` (0 in both) and `0x1F` (31 in both) are numbers, while `0755`
(755 against octal 493) and `08` (8 against text) are `yaml-1.1`. `.inf`, `-.inf` and `.nan` are
`not-comparable`. Timestamps are text under both, so a date never differs. A plain scalar of more than 4,096
bytes that either version reads as a number is `bound:number-text`.

The YAML 1.2 core schema, each a full match:

| type | regex |
|-|-|
| null | `null\|Null\|NULL\|~` or empty |
| bool | `true\|True\|TRUE\|false\|False\|FALSE` |
| int | `[-+]?[0-9]+`, `0o[0-7]+`, `0x[0-9a-fA-F]+` |
| float | `[-+]?(\.[0-9]+\|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?` |
| infinity | `[-+]?\.(inf\|Inf\|INF)` |
| not a number | `\.nan\|\.NaN\|\.NAN` |

YAML 1.1 is PyYAML's resolver, the deployed 1.1 behaviour (the 1.1 specification's own float pattern would make
every plain `1.2.3` a float), each a full match:

| type | regex |
|-|-|
| bool | `yes\|Yes\|YES\|no\|No\|NO\|true\|True\|TRUE\|false\|False\|FALSE\|on\|On\|ON\|off\|Off\|OFF` |
| int | `[-+]?0b[0-1_]+`, `[-+]?0[0-7_]+`, `[-+]?(0\|[1-9][0-9_]*)`, `[-+]?0x[0-9a-fA-F_]+`, `[-+]?[1-9][0-9_]*(:[0-5]?[0-9])+` |
| float | `[-+]?([0-9][0-9_]*)\.[0-9_]*([eE][-+][0-9]+)?`, `\.[0-9][0-9_]*([eE][-+][0-9]+)?`, `[-+]?[0-9][0-9_]*(:[0-5]?[0-9])+\.[0-9_]*`, `[-+]?\.(inf\|Inf\|INF)`, `\.(nan\|NaN\|NAN)` |
| null | `~\|null\|Null\|NULL` or empty |

A 1.1 int with `_`, a binary int and a sexagesimal number are numbers that 1.2 reads as text, so they always
differ. `tools/gen_yaml_plain.py` records PyYAML's reading of 6,749 plain scalars against these 1.2 regexes in
`tests/fixtures/yaml-plain.jsonl`, and the crate's tests hold `yaml_plain` to it.

## Resolving values

Edge cases 39, 44, 45, 54 to 57 and 62. Conformance cases: `conformance/cases/edge-resolving`,
`conformance/cases/config-key`.

A `config_key` anchor is `<file>#<key path>`, or a list of them read as layers. Resolving it gives one of
three answers: the value at the key, the key missing (with whether any layer's file exists), or
`unverifiable` with a reason. §States turns the answer into a check's state.

**Layers.** Layers are read from the last to the first, and the first that defines the key wins. A layer
defines the key when it holds a value there, or when it cannot be read: an unreadable layer may hold the
key, so its reason wins over an earlier layer's value. A layer with no file defines nothing, and neither
does a layer whose file lacks the key. Missing in every layer is a missing key, unless no layer has a
file at all: then the file is missing.

**Files of a layer.** The file part is a literal path, or a glob with wildcards in the last segment only
(else `invalid: glob-dir`); a glob that matches nothing is a layer that defines nothing. Each layer is
judged in this order, and the first rule that applies decides:

1. A symbolic-link or submodule entry as any directory above the file, or above the glob's directory, gives
   `symbolic-link` or `submodule`. So does the literal path itself, or any match of the glob.
2. Any match that a resolver reads (**Formats**, below) larger than `anchored-file`, or a glob's HCL files larger
   together than the HCL bound, is `bound:anchored-file`: from the tree entries' sizes before anything is
   read, and again on the bytes read, before any file's own reason.
3. A file whose name has no resolver is `format`, whatever its size. It is never read.
4. A blob under 1,024 bytes starting `version https://git-lfs.github.com/spec/` is `lfs-pointer`.
5. A blob that is not UTF-8 is `encoding`.
6. One UTF-8 byte-order mark is skipped; a second is `format`. A file that does not parse is
   `format`.

Conflict markers are judged in records only. An anchored file holding them is read like any other.

**Formats.** By the last path segment, case-sensitive:

| reader | names |
|-|-|
| YAML 1.2, core schema | `*.yaml`, `*.yml` |
| JSON (RFC 8259: no comments, no trailing commas) | `*.json`, `Pipfile.lock`, `flake.lock`, `composer.lock` |
| TOML 1.0 | `*.toml`, `Cargo.lock`, `poetry.lock`, `uv.lock`, `Pipfile` |
| HCL attribute literals | `*.tf`, `*.tfvars`, `*.hcl` |

Any other name is `format`. A later revision may only add names.

**One glob, several files.** Each matching file is resolved alone, except that all the glob's HCL files
merge into one value (below), which stands where the first HCL file does in path order. Then the key must be
in exactly one place: in two, it is `defined-twice`. Otherwise a file that cannot be read may hold the key,
so the first reason in path order wins over one value. With no value and no reason, the key is missing. Two
files with the same content are still two files.

**YAML.** Aliases are expanded, within the `alias-expansion` budget per file. A plain scalar is read by both YAML
versions (§Values and comparison), so a value YAML 1.1 reads differently is `yaml-1.1` (edge case 57). A
quoted or block scalar is a string. A mapping key is its scalar text, whatever its style or type: `on:`
is the key `on`, and `1:` and `'1':` are one key. A key defined twice by text is `defined-twice`. A
non-scalar key never matches. A plain `<<` key is a merge key: it is not an entry, and a key absent from its
mapping is `merge-key`, never missing (edge case 54), including a selector key. A key present beside `<<` is
read as written. A node with an explicit tag (`!Ref`, `!!str`, `!vault`) is `tag`, whether the key path ends
on it or goes through it.

A file with two or more documents is the list of its documents, and a file with one document is that
document. A file with no document holds no key. With two or more documents, a leading index or
selector picks among them. Any other key path must be in exactly one document: in two, it is `ambiguous`;
a document that cannot be read (a tagged one) may hold it, so its reason wins over one value.

**JSON and TOML.** A JSON number and a TOML integer or float are exact decimals; a TOML datetime is its text as
written. A JSON key defined twice in one object is `defined-twice`. TOML forbids a key defined twice, so such
a file is `defined-twice` as a whole. An array of tables is a list.

**HCL.** A value is read only when it is a literal: a number (with a leading `-`), a string, a heredoc
without interpolation or directives, `true`, `false`, `null`, or an object or tuple literal of them, read like
a map and a list. Anything else is `expression` (`var.x`, `local.y`, a function call, a string template with
an interpolation or a directive, an object with a computed key); so is a `dynamic` block. A key path starts at
the block type, then its labels, then nested block types; an unlabelled block is its type alone.

Top-level blocks with the same type and labels merge by attribute, within a file and across a glob's files,
as Terraform does. An attribute set twice is `defined-twice`, and so is a name used both as an attribute and as
a block. Nested blocks never merge: a nested block type (with its labels) that occurs once is that block's
body, and one that occurs twice is a list that needs a selector, else `ambiguous`.

A file with more than `hcl-assignments` `=` and `:` characters is `bound:hcl-assignments`, counted before it
is parsed. Nesting beyond the parser's own limits is `bound:resolver-depth`.

**Nesting.** Every map and list counts, the file's root value at depth 1; depth 64 (`resolver-depth`) is
allowed and depth 65 is `bound:resolver-depth`, in every format. An HCL block's body sits one level below
its block, and its labels add one each.

**The key path.** Segments are read in order:

| segment | on a map | on a list | on a scalar |
|-|-|-|-|
| a key | its value; absent: missing, or `merge-key` beside `<<` | `ambiguous` when not empty (a selector is needed), else missing | missing |
| `[n]` | the map itself as a list of one: `[0]` is the map, any other index missing | the element, or missing past the end | missing |
| `[k=v,...]` | the map as a list of one | the one matching element (below) | missing |

A map is a list of one under an index or a selector because a block or a document that occurs once is that
block or document, so a key path written for the repeated case still reads the single one.

Going into a value that cannot be read is that value's reason (`tag`, `defined-twice`, `expression`, a bound).
A string containing `{{` or `{%` is a template that may render to anything, in every format: the key path
ending on it, or going through it, is `template` (edge case 56). A sibling of a templated value resolves
as usual. A plain scalar that is unverifiable as a value (`yaml-1.1`, `bound:number-text`) is still a scalar,
so a key below it is missing, and the key path ending on it is that reason. These are unverifiable whatever
the expectation. A number no decimal holds (`.inf`, `.nan`, an exponent beyond any decimal) is read the
same by both versions: it is a value, so `exist` holds on it, and an op on it is `not-comparable`. A key path ending on a map or a list is a value: `exist` holds on it, and an op on it
is `not-a-scalar` (§Values and comparison).

**Selectors.** `[k=v,k2.sub=v2]` picks the one element whose keys all equal those values, as `==`
compares (§Values and comparison): the text `443` equals the number 443. Each criterion's key is a dot path
inside the element. A criterion is false when its key is absent or its value is a map or a list; it is
unverifiable when its key lies beside a `<<` merge key, goes through a non-empty list or a value that cannot
be read, or its value is a template, or when `==` itself is unverifiable (`on` against `1`). An element
matches when every criterion is true; one false criterion decides, whatever the reasons of the others;
otherwise the element's reason is its first criterion's reason. Two matching elements are `ambiguous`.
Otherwise an element whose criteria are unverifiable may match, so the first such reason in element order
wins over one match. No match, and no reason, is a missing key.

**Bounds.** A pass keeps what it has read of anchored files (parsed documents, HCL bodies, a glob's merged
HCL files, `code` texts) up to 256 MiB, counted as the memory each holds once parsed (an estimate); the least
recently used is let go first and read again when asked for, and a file that holds more than that alone is
never kept. So a blob may be read more than once in a pass, and the answer
depends neither on the order checks are resolved in nor on what the pass holds: every case of the corpus
answers alike whether a pass keeps everything, nothing, or 64 bytes. Each file is read by itself, after the
files a layer reads are named to the tree together, so a partial clone fetches them in one call. A file is
judged too large from its tree entry before it is read, and again from the bytes once read: a partial clone
cannot know the size of a blob it does not hold, and both clones must give one answer. A glob's HCL files
merge into one value, so they are bounded together as one HCL file, and reading stops at the file that takes
them past it.

## States

A check's state and a record's state are recomputed from one tree, never stored.

### Three anchors

**Before the tree is read.** A check kind this reader does not know, or an expectation that is neither
`exist` nor an op it knows, is `unknown`. On `path` and `code` the only expectation is `exist`: a known op
there is `not-comparable`.

**Links and submodules above the anchor.** For every anchor, a symbolic-link or submodule entry at any
directory above a literal path, or at a glob's literal directory or any directory above it, gives that reason
(`symbolic-link`, `submodule`) before any match is looked for. Entries are judged by their mode and never
followed; the tools never descend into a submodule (edge cases 39, 44).

**`path`** (`exist`):

- A literal path holds when its entry is a regular or executable file, and gives the reason when the entry is
  a link or a submodule. With no entry at that path, it holds while any entry lies under it, because it names
  a directory (`ansible/roles/vault`, edge case 42); otherwise it is missing (`missing-file`).
- A glob matches every entry under its literal directory (§Paths and globs: `docs/**` is every file under
  `docs/`, never a file named `docs`; brackets are literal). Any regular-file match holds. With none, the
  first link or submodule match in byte order of path gives its reason; with no match at all, it is missing.
- A path that is ignored by git or untracked is not in the tree, so it is missing (edge case 43). The CLI's
  wording for that case is its own.

**`config_key`**: resolved as §Resolving values says. A scalar that is present holds under `exist`, with its
value shown; under an op it is compared (§Values and comparison): true holds, false is `false` with the value
shown, and a comparison that cannot be made is its reason. A value is shown cut at 200 bytes, or as
`(withheld: secret-shape)` when the whole value holds a secret shape (§Secrets), wherever it lies: it is
screened whole before it is cut, so no cut shows a part of a secret. A map or a list holds under `exist` and is
`not-a-scalar` under an op. A missing file or key is `missing-file` or `missing-key`.

**`code`.** The file is a literal path (a glob there makes the record invalid). In order:

1. a link or submodule above the file, then no entry (`missing-file`), then a link or submodule entry;
2. the text: more than 2,048 bytes as parsed, or fewer than 24 Unicode scalar values once whitespace is
   removed, is `bound:code-text`;
3. the file's entry larger than the anchored-file bound is `bound:anchored-file`, before it is read, and
   again on the bytes read;
4. an LFS pointer is `lfs-pointer`, a file that is not UTF-8 is `encoding`;
5. U+0009, U+000A, U+000C, U+000D and U+0020 are removed from the text and from the file. Nothing else
   is: U+000B and a no-break space are characters like any other (edge case 62).
6. The text's occurrences are counted at every start position, overlaps included: none is `false`,
   shown as `0 occurrences` (the line it guarded is gone, edge case 48); one holds; two or more are
   `ambiguous`.

So a re-wrapped or re-indented text holds, CRLF line ends included, and so does a text left in a comment
(edge cases 63, 64): `code` proves the text is present, not that it runs.

### States of one check

| state | when |
|-|-|
| `holds` | resolved, and the expectation is true |
| `broken` | a `must` that is false, or whose file or key is missing |
| `stale` | an `assumes` that is false, or whose file or key is missing |
| `unverifiable` | the value cannot be judged; the reason is always given (§Reasons) |

A missing file or key is never `unverifiable`, and it is judged before any bound on the check's own text: a
`code` check on a missing file is missing even when its text is under the floor. `unverifiable` is never
reported as `holds`.

The value a check shows is a scalar's text: a string as read, `true` or `false`, `null`, a datetime as
written, a number in plain decimal notation while that takes at most 64 characters, and otherwise as
`d.ddde<n>` (`1e400`, never expanded). A number no decimal holds (`.inf`) shows nothing.

### Record state

`invalid` (§The record), else `superseded` (its checks are not evaluated), else the worst of its checks:
`broken` over `stale` over `unverifiable` over `holds`. A record without checks holds and is context only.

A record of a newer schema (§The record) is shown with its id, kind and title as schema 1 reads them, and is
`unverifiable: newer-schema` as a whole; its checks are not read, and it is never rewritten. It can be
superseded, and its `supersedes` is honoured.

### Folders and record files

- An entry belongs to the folder named by the first `.knowledge/records/` on its path; the folder's scope is
  the directory above it (`""` at the root). Everything under that `records/`, at any depth, is that
  folder's, a nested `.knowledge/records/` included.
- The folders on the way up from a path are the root's, each directory's above it, and the path's own (a
  directory names its own folder).
- A record file is an entry whose name ends `.md`, case-sensitive, that is not a submodule. Any other entry
  under `records/` is ignored and reported once (edge case 29). A binary `.md` file is a record, `invalid:
  parse`, because it is not UTF-8.
- A record file is judged from its tree entry before it is read: a symbolic link is `invalid:
  symlink-record` and an entry over 64 KiB is `invalid: bound:record-file`, both at line 1, column 1; the
  size is judged again on the bytes read.
- One pass reads at most 100,000 records, in byte order of path across every folder; the rest are not read
  and are counted by folder (edge case 81).

### Identity and supersession

- An id is the front matter's, never the file name's (edge case 4). Two or more records with one id in one
  folder are duplicates: each is still read and evaluated (edge case 20). The same id in two folders is two
  records.
- `supersedes` is honoured from a valid record and from a newer schema's record. It has no effect, and is
  reported, when it names its own record (`itself`), an id no record of its folder holds, another folder's
  included (`not-in-folder`), or a duplicated id (`duplicated`). A newer record whose id cannot be read
  supersedes nothing.
- A superseded record's own `supersedes` still applies, so a chain supersedes every record but its last.
- A cycle is reported once, from its smallest id, and the cycles in order of that id. No member of a cycle is
  superseded, even by a record outside it that points in (edge case 17).
- A record superseded by several records is superseded; it names the first of them in byte order of path.

### Folder-wide records

A record whose `applies_to` covers its whole folder is context for every file there: no checks and no
`applies_to`, or an `applies_to` entry that is `'**'`. A superseded record is context for nothing, so it is
never folder-wide. A newer schema's record is folder-wide when the `applies_to` it writes holds `'**'`.

## Routing

The records that are context for a set of paths.

- **Where.** Only the folders on each path's way up are read (the root, every directory above the path, and
  the path itself), each with its records' front matter in one read. Records past the per-pass bound, in
  byte order of path across those folders, are not read.
- **Which.** A record routes when it is valid or of a newer schema, is not superseded, and an `applies_to`
  glob matches one of the paths. Invalid and superseded records never route; the members of a supersession
  cycle and both holders of a duplicated id do. Without `applies_to`, a valid record applies to the files its
  checks anchor: each `path` anchor's path, and `<path>/**` too when it is literal; each `config_key`
  layer's file part; each `code` file. A record with neither checks nor `applies_to` applies to `**`. An
  `applies_to` written empty applies to nothing. Only routed records are evaluated, sharing one pass's reads.
- **Order.** Each key decides only when every key before it ties:
  1. `broken` and `stale` before the rest, as one group;
  2. the nearer folder: more segments in its scope (the root has none);
  3. the narrowest glob that matches one of the paths: fewest `**` segments, then the longest literal prefix
     (the bytes of the glob joined with its scope before its first `*` or `?`);
  4. a record with a `must` before one without (a newer schema's checks are not read, so it has none);
  5. kind: `decision`, `procedure`, `fact`, `question`, then a record whose kind cannot be read;
  6. title in byte order (`Zeta` before `alpha`), a record without one last;
  7. id in byte order, a record without one last;
  8. path in byte order, so the order is total.
- **The cap.** The first 20 are shown. Of the rest, the count and how many are `broken` or `stale` are
  given, so nothing the cap hides goes unannounced.

## The gate

How a change is judged: two trees, B before the change and H after it.

### The two trees and the change

- **H** is the tree being judged and **B** the tree it is judged against. Which commits they are (a
  revision's tree, the working-tree view, the merge base under `--base`) is the CLI's; this
  document starts from the two trees. Both are read by exact byte path, never
  from a disk walk.
- **The changed paths** are every path added, removed, or with another mode or object id between B and H:
  `git diff-tree -r --no-renames` between them. There is no rename detection, and no local git setting
  changes the set (edge case 77).

### Live, candidates and the same check

- **Live.** A check is live in a tree when its record there is valid, of schema 1 and not superseded.
- **Candidates.** The same check is sought only among the records of the folders above a changed
  path (§Folders and record files), in B and in H. Only those folders are judged: an anchor stays inside its
  folder's parent directory, so no check of another folder can change state.
- **The same check.** A check of H is the same check as a check of B when their records have the same id,
  the verbs are equal, the expectations are equal as parsed (op and literal, so spacing and quoting do not
  matter), the key paths (as parsed) or `code` texts are equal, they have the same number of layers, and in
  each position H's path, from the repository root, equals B's, or B's path matches nothing in H. A path
  matches nothing in H when no entry is at it or under it (a literal path) or matched by it (a glob), links
  and submodules counting: the matching of a `path` anchor (§Three anchors). So a directory anchor follows
  only when nothing is left under the old directory. A check of an unknown kind keeps only its kind's name
  (§The record), so two of one kind with equal verbs and expectations are the same check.
- **Exact pairs first.** A check of H that has a same check in B with every path equal is the same
  check as those alone; only a check of H with no such pair follows. Without this, a twin elsewhere (same
  id, a path that matches nothing in H) would excuse or condemn a check the change did not touch, and the
  twin's own removal would never reach rule 2. It is stricter than following alone in one case, and fails
  closed there: a change that moves one of two twins onto the other's path, and deletes the old path in the
  same change, reports the moved twin under rule 2; removing it first, in its own change, passes.
- **Followed.** Each same-check pair whose paths differ in a position is an anchor that followed: (B's path,
  H's path).

### The two rules

Each rule reads the check's state at a tree (§States of one check). Where several checks of B are the same
check as one of H (twins that share an id), an excuse needs every one of them to qualify, and at least one:
adding a twin never hides a finding.

1. **Rule 1, H judges H.** A live `must` of H:
   - `broken` at H: excused when it has a same check in B and every one was `broken` there; otherwise
     `broken`, rule 1, with the H state's reason.
   - `unverifiable` at H: `broken`, rule 1, reason `no-longer-checkable: <code>`, when a same check held at
     B. With no same check in B, it is a new `must` that cannot be judged: listed under knowledge changes with
     its record, its index among the record's checks (from 0) and its reason, never a finding (edge case 51).
     When no same check held at B, nothing: unknown is not an excuse, and not a finding either (edge case 50).
2. **Rule 2, B is not dropped.** A live `must` of B that was not `broken` at B and has no same check live in
   H is evaluated at H as B wrote it:
   - `broken` at H, or `unverifiable` at H after holding at B: `broken`, rule 2.
   - `unverifiable` at B and at H: nothing. A check that was never verifiable is never a finding.
   - Otherwise `changed`, saying what happened, decided in this order:
     1. The record: found by id among H's candidate records that have a readable id (valid or of a newer
        schema), the first live one in byte order of path, else the first; failing that, the record at B's
        path when it has no readable id. None: `record-removed`. Superseded: `superseded`. Invalid:
        `invalid`. Of a newer schema: `newer-schema`.
     2. A check of it on the same anchor (same kind, keys and layer count, each path equal or following) with
        another verb: `verb-changed`.
     3. One on the same anchor with another expectation: `expectation-changed: <B's> -> <H's>`.
     4. One of the same kind and keys whose path differs where B's still matches something in H:
        `anchor-moved: <B's anchor> -> <H's anchor>`, at the first position that differs.
     5. Else `check-removed`.
3. **Stale.** A live `assumes` of H that fails at H is `stale` unless it has a same check in B and none
   held there. One `unverifiable` at H is `stale`, reason `no-longer-checkable: <code>`, only when a same
   check held at B: a new `assumes` that cannot be judged is silent.

**Broken before** counts the checks rule 1 excused that the change touched: a record file of their folder
changed, or a changed path is matched by one of their inputs, lies under a literal input, or is a directory
at or above a literal input or a glob's literal directory. Never a finding.

**Same content.** A finding whose check is `missing-file` at H names the first path, in byte order, that the
change added as a regular file with the object id that one of the check's literal input paths had in B,
where that path was a regular file in B and is absent in H. It is the action "update the anchor" (edge case 7).

### Findings on the change

Only where a record file the change added or edited takes part, in H: a duplicate id in a folder
(`duplicate-id`, the id and every path holding it); a `supersedes` without effect (`supersedes`, the path and
the problem: `itself`, `not-in-folder`, `duplicated`); a supersession cycle (`cycle`, its ids); a secret-shaped
value (`secret-shape`, the path, the rule id, the line and column; §Secrets). A record the change did not touch
is never a finding, even in a folder the change reads. Findings come by kind in that order, then by path, and a
file's secret shapes by position. None of them fails the gate, and none is a record state.

### Knowledge changes

Record files added, edited (another mode or object id) and removed, in byte order of path; the anchors that
followed, deduplicated, in byte order; the new `must` checks that cannot be judged, in byte order.

### Output

- **Grouping.** One finding per record id, judging record path and anchor. The judging record is H's under
  rule 1 and for `stale`, B's under rule 2 and for `changed`. The anchor is the root path of the check's
  first position, `<file>#<key path as written>` for a `config_key`. The worst class wins (`broken`, then
  `changed`, then `stale`); at an equal class the first member wins. The title and first paragraph are the
  winning record's. Every reason is kept, in the order the rules give them: rule 1's, then rule 2's, then the
  stale notes', each in check order.
- **Order.** Findings by class (`broken`, `changed`, `stale`), then record path, anchor and id. Findings on
  the change by kind (`duplicate-id`, `supersedes`, `cycle`), then the first path (a cycle's
  smallest id).
- **Reasons.** A stable code, then `: <detail>` where there is one:

| code | when | detail |
|-|-|-|
| `false` | the check is false at the tree that judges it | `<value shown>` (as §States shows a value: cut at 200 bytes, or `(withheld: secret-shape)`), then `, expected <op> <literal>` unless the expectation is `exist` |
| `missing-file`, `missing-key` | as in §Reasons | none |
| an `unverifiable` code | as in §Reasons | as there |
| `no-longer-checkable` | rule 1 or 2 (or a stale note): `unverifiable` at H after holding at B | the `unverifiable` code |
| `record-removed`, `superseded`, `invalid`, `newer-schema`, `verb-changed`, `check-removed` | `changed` | none |
| `anchor-moved`, `expectation-changed` | `changed` | `<from> -> <to>` |

### What the two rules mean in practice

- **A record never excuses its own change.** Removing, relaxing, superseding, invalidating or reclassifying
  the record that holds a constraint leaves rule 2 judging B's check (edge cases 13, 18, 22, 66). The kind
  never changes judging (edge case 69).
- **A constraint comes down in a change of its own.** Remove or relax it first (`changed`); change the value
  in the next change (edge cases 11, 12, 14, 67).
- **A moved file takes its anchor along in the same change**, and nothing fires (edge cases 6, 8, 9, 10). Left
  behind, rule 1 reports `broken` naming the added file with the same content (edge case 7).
- **A constraint that comes back judges at once.** A record revived by deleting its superseder, or repaired
  from `invalid`, was not live in B, so rule 1 judges it with no excuse (edge case 19).
- **Unknown is not an excuse.** Rule 1 excuses only a check `broken` at B (edge case 50).
- **A new check that cannot be judged is not a finding**: a new `must` is listed under knowledge changes.

## Cost

`gate(B, H, changed)` gives the report of the reference, `gate_full(B, H)`, which lists both trees to find
the change. A property over random trees and edits (folders moved, files moved with or without
rewritten anchors, records edited, copied, superseded or deleted, directories turned into links) and every
gate case of the corpus check the two are equal. What the footprint form reads, per tree:

- the folders above a changed path, as the reference does, and their record files, named to the tree
  together (a partial clone fetches them in one call) and read a folder at a time;
- the checks the change affected: a record file of the check's folder changed, or a changed path is
  matched by an input, is under a literal input, is a directory above a literal input, or is at or above a
  glob's literal directory (a link there makes the check unverifiable while nothing it matches changes);
- the files those checks read, named to the tree together, so a partial clone fetches them in one call;
  nothing is held ahead, and each is read when evaluation needs it. Best effort: a failed lookup names
  nothing, and evaluation reads what it needs itself.
- the record files the change added or edited, again in one read, for the secret screen (§Secrets); none when
  the change touches no record file.

The same-check matching still covers every live check of those folders. It compares records, and lists
paths only for pairs with one id whose paths differ.

An unaffected check yields nothing, so skipping it changes nothing. Its record and inputs are unchanged, so
its copy in the other tree is live and pairs with it exactly. An exact pair has equal states, because it has
the same inputs. With exact pairs first (§The gate), an H check is judged against its exact pairs alone,
so rule 1 excuses or ignores it, and its B copy is matched, so rule 2 never sees it. More is read in two
cases:

- a judged check whose exact twin sits in an untouched folder of B reads that twin's files (the same paths);
- a pass that leaves a record unread (`RECORDS_PER_PASS`) judges every check, because a record the change
  added can push an untouched one past the bound in one tree only.

## Secrets

The screen reads a whole record file, front matter included, and reports each
secret-shaped value: the well-known token shapes below, under the ids and the regular expressions (Rust
`regex` syntax) of the DocBrain server's catalogue, whose screen stays a superset. The name of a secret
(`AWS_SECRET_ACCESS_KEY` in a rotation procedure) is never a secret: no row matches a name or a bare prefix,
and a false finding is a bug. A hit is the rule id and the line and column where the value starts (as in
§Hidden text), never the text.

| id | regex | source |
|-|-|-|
| `aws_access_key_id` | `AKIA[0-9A-Z]{16}` | AWS |
| `github_pat_classic` | `ghp_[0-9A-Za-z]{36}` | github.blog |
| `github_pat_fine_grained` | `github_pat_[0-9A-Za-z_]{82}` | github.blog |
| `gitlab_pat` | `glpat-[0-9A-Za-z_-]{20,}` | gitlab.com |
| `slack_token` | `xox[baprs]-[0-9A-Za-z-]{10,72}` | api.slack.com |
| `pem_private_key` | `-----BEGIN (RSA \|EC \|DSA \|OPENSSH \|ENCRYPTED )?PRIVATE KEY-----` | RFC 7468 |
| `jwt_token` | `eyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+` | RFC 7519 |
| `url_basic_auth_credentials` | `[a-zA-Z][a-zA-Z0-9+.\-]*://[^/\s:@]+:[^/\s@]+@` | RFC 3986 userinfo |
| `github_token_families` | `gh[oprsu]_[0-9A-Za-z]{36,255}` | github.blog |
| `anthropic_api_key` | `sk-ant-[0-9A-Za-z_-]{20,}` | anthropic.com |
| `openai_api_key` | `sk-(proj-)?[0-9A-Za-z]{20,}` | openai.com |
| `docbrain_api_key` | `db_sk_[0-9a-z]{32}` | DocBrain API key |
| `docbrain_app_token` | `db_o[ar]t_[0-9a-z]{64}` | DocBrain app access and refresh token |

A **documented shape** is an example in documentation, not a working credential, and is not reported. Three
rows have one; the server's screen judges them the same way, and no other row's
match is ever a shape:

- `pem_private_key`: the header has no key material after it. What follows the header is read up to the first
  `-----END` wholly within the next 8,192 bytes (or those bytes, cut back to a character boundary), and then up
  to the next match of this row's own regex. In it, each of these separators becomes a line break, in this
  order: `\\n` `\\r` `\\t` `\n` `\r` `\t` `&#10;` `&#xa;` `&#xA;` `&#13;` `&#xd;` `&#xD;` `&#32;` `&NewLine;`
  `&quot;` `<br />` `<br/>` `<br>` `</p>` `<p>` `|` (each written here as its characters, a backslash and a
  letter for the first six). Key material is at least 54 base64 characters (`A-Z a-z 0-9 + / =`), summed over
  the lines that hold nothing else once trimmed of ASCII whitespace and `>`, or in one unbroken run.
- `url_basic_auth_credentials`: the password, everything after the first `:` of the userinfo, is a placeholder.
  It is one, compared after ASCII case folding, of `changeme` `change-me` `example` `placeholder` `redacted`
  `dummy` `your-password` `your_password` `yourpassword` `mypassword` (the template words) or `password`
  `passwd` `pwd` `secret`. Or it is a run of one masking character (`.`, `*`, `x` or `X`, `•`). Or it is
  `${…}`, `{{…}}`, `<…>`, `[…]` or `{…}` (the first that fits) whose trimmed inside names something, `%…%`
  whose inside names something, or `$` and a name `[A-Z_][A-Z0-9_]*`. An inside names something when it is a
  template word (case-folded), or matches `[A-Z_][A-Z0-9_]*` or `[a-z_][a-z0-9_. -]*` and every part between
  `_` `.` `-` and spaces is a word: two or more ASCII letters, then at most three digits. `admin`, `test`,
  `pass` and `token` are not placeholders: they are working default passwords.
- `jwt_token`: the match is, whole, the RFC 7519 §3.1 example token or the jwt.io demo token
  (`SAMPLE_JWTS`).

One secret is one hit. Matches are taken by start, the longest first, then in table order. A match wholly
inside the region's holder (a token in a URL's userinfo, `ghp_` under two rows) is the same secret and is not
reported again, unless the holder is a shape and the match is not: then the match is reported and holds the
region from there, so a real token in an example URL's user part is still found. A match that runs past the
end of the holder is its own hit. A record file past `record-file` is screened on its first `record-file`
bytes only: the record is already `invalid` there, and a partial clone may read a different length past it.
Bytes that are not UTF-8 read as U+FFFD.

In the gate, each hit in a record file the change added or edited is a `secret-shape` finding on the
change. It never fails the gate and is never a record state.

## Edge cases

Each is an answer the sections above give, numbered for reference. `conformance/coverage.tsv` names the cases
that prove each one, or the owner outside this crate that carries it.

1. two branches create records concurrently: different ids, different files, no conflict
2. two branches edit one record: conflict markers make it invalid until resolved
3. one branch removes a record, another edits it: the person merging decides
4. a record file renamed or moved into a subdirectory: identity is the id; the name is cosmetic
5. squash merges and rebases: identity by id; sync is idempotent
6. an anchored file moved with its anchor in the same change: the same check at the new path
7. an anchored file moved, the anchor left behind: `broken` under `must`, naming the added file with the same
    content; `stale` under `assumes`
8. a whole directory moves with its folder and records: every anchor follows
9. a file moves out of its folder's parent directory: the record moves with it in the same change, its anchor
    rewritten, and it is the same check; left behind, its anchor is missing (edge case 7)
10. a record moved to another folder with its anchors rewritten: the same check
11. code moved from a file that remains: not the same check
12. a guarded key restructured in its file: the old key path no longer names a scalar, `broken` by rule 1 or
    rule 2; a key path changed in the same change is not the same check
13. a record deleted: rule 2 judges its `must` checks at H
14. a service decommissioned in one change is `broken`; in two changes `changed`, then nothing
15. removing a body for privacy: the next sync drops it
16. `supersedes` naming an unknown id, another folder's record or its own: a finding, no effect
17. a supersession cycle: a finding; no member is superseded
18. a change supersedes a `must` and breaks it: rule 2
19. the superseding record deleted, or a record repaired from invalid: rule 1 with no excuse
20. two records with one id in one folder: both evaluate; a finding on the change; the server indexes the first
21. `knowledge: 2` is `unverifiable: newer-schema`, never rewritten; `1.0` or `'1'` is `invalid`
22. a record changed to `knowledge: 2`: rule 2 judges B's checks
23. a schema-2 record that supersedes a schema-1 record: honoured
24. an unknown key is reported once; an `x-` key is ignored
25. anchors, aliases, tags, merge keys or duplicate keys in front matter: invalid
26. a plain or double-quoted string in front matter: `invalid: quotes`; `knowledge: 1` is the one integer
27. a body above 64 KiB is refused; the context output shows the first paragraph and the path
28. an empty body: `record` refuses; a hand-written one is invalid
29. a binary or non-`.md` file under `records/`: ignored, reported once
30. an id starting `0000` is invalid
31. a hidden-text character, an ANSI escape or a run of variation selectors: invalid with line and column
32. a body in any language with the direction marks U+200E, U+200F and U+061C is valid; the bidirectional
    embeddings, overrides and isolates stay `invalid`
33. CRLF or a BOM accepted on read; the CLI writes LF without a BOM
34. two slugs that differ only by case: two ids, two files
35. an agent writes 500 records in one change: listed under knowledge changes; no cap
36. an agent pins a value it just chose under `must`: listed for review; the skill's eval suite catches it
37. a record about another repository's path: no cross-repository anchor in schema 1
38. a leading `/`, `..`, a drive letter, `~`, a backslash, or under `.git/`: invalid; on Windows `record` writes `/`
39. a symbolic link as an anchored file or a component of its path: `unverifiable: symbolic-link`; as a record
    file: `invalid: symlink-record`
40. `app/[slug]/page.tsx` is literal
41. `docs/**` is every file under `docs/`, never a file named `docs`
42. a directory path under `exist` holds while a file lies under it
43. an anchor on an ignored or untracked file: not in H, so missing: `broken` or `stale` by its verb; `record`
    refuses it
44. an anchor inside a git submodule: `unverifiable: submodule`; the tools never descend into it
45. an anchored file stored in Git LFS: `unverifiable: lfs-pointer`; `record` refuses it
46. a root record and a nested record both match: both shown, nearest first
47. `applies_to` matching nothing: context for nothing; `status` says so
48. the guarded line deleted, or the key missing: `broken` under `must`, `stale` under `assumes`
49. a guarded value replaced by an expression, a template or a range: rule 1, no longer checkable
50. `unverifiable` at B then violated at H is `broken`; unverifiable at both is never a finding
51. a new `must` that is `unverifiable` at H: listed under knowledge changes; `record` refuses it
52. a layer whose file is missing defines nothing
53. `terraform {}` in two files: blocks with the same type and labels merge by attribute; an attribute set in
    both is `unverifiable: defined-twice`
54. a key absent from a YAML mapping that has a `<<` merge key: `unverifiable: merge-key`
55. a multi-document YAML file with the key in two documents: `ambiguous`
56. an Ansible vars file with one Jinja value: that value is `template`; the other keys resolve
57. `on:` is a string key; `on`, `off`, `yes`, `no`, `y`, `n`, `0755`, `1:30` are unverifiable
58. TOML datetimes are text under `==`, not comparable when ordered; `.inf`, `.nan` not comparable; `0x1F` is 31
59. a selector value is compared as `==` compares
60. an ordered comparison between a version and a non-version: not comparable
61. a decimal string whose two readings disagree: `unverifiable: number-or-version`, never a silent pass;
    `record` refuses it
62. a `code` file not UTF-8: `encoding`; only ASCII whitespace removed; the floor counts scalar values
63. a formatter re-wraps or re-indents guarded code: holds
64. the guarded text left in a comment: holds
65. an anchored file grows past its bound: `unverifiable` with the bound; held at B, so `broken`
66. a change edits a `must` so the new one passes while the old one is violated: rule 2
67. a change only removes, loosens or supersedes a `must`: `changed`; the next change may alter the value
68. a tightened `must`: `changed`, reason `expectation-changed`; the new check is judged by rule 1
69. a decision turned into a question: the kind never changes judging
70. a dependency bump that trips an `assumes`: `stale`; one that trips a `must`: `broken`
71. a shallow CI clone without the base commit: exit 3 with the fetch hint
72. several merge bases: exit 3, "pass the base commit"; the reference job passes `M^1`
73. a pull request into a release branch, or a retargeted stacked one: merge commit against its first parent
74. two changes that pass alone and break together: caught only with up-to-date branches or a merge queue, else by
    the scheduled status
75. a change edits the job file or its flags: `pull_request_target`, or `CODEOWNERS` and protected configuration
76. a proposed record with a workflow-command line: printed between stop-commands tokens
77. `diff.renames`, `diff.renameLimit` or `merge.renames` set locally: no effect
78. a sparse checkout or sparse index: H takes paths outside the cone from the index; anchors read from objects
79. a blobless partial clone: one batch of fetches; offline with a missing object, exit 3
80. context for 5,000 paths: routing through the cache, then the cap
81. more than 100,000 records: the excess, in byte order of path, listed in one line by folder
82. more than 1,000 files in one directory under `records/`: subdirectories carry no meaning, and
    `.knowledge/README.md` suggests one per year
83. a subagent edits a file its parent already saw records for: the hook's dedupe is keyed by the receiving context
84. offline, air-gapped or the server down: everything local works; sync resumes
85. records on a non-default branch: local tools read H; the server reads the default branch only
86. the folder inside a Docker context, a Helm package or an npm package ships with it; `.knowledge/README.md`
    gives the ignore lines
87. root records filling every edit's context: local and narrow first, the cap announced; `status` names every
    folder-wide record

## Conformance

The corpus is `conformance/cases/`: one JSON file per case, each naming the section of this document or the edge
case it tests, in the format of `conformance/README.md`. A case is a tree (and, for the gate, a second tree) and an op, `parse`,
`status`, `context` or `gate`, with its whole expected answer: states, reason codes, order and counts.
`conformance/coverage.tsv` maps rules of this document, keyed by section, and every edge case to the cases
that prove them. A bound too large for a case (an anchored file past 32 MiB, 100,000 records)
is proven by a named test of this crate; a rule that lives outside the evaluator (the CLI's exit codes and
output, the reference job, the server's sync, the skill) names its owner, and a test of this crate where its git
adapter does part of the work. The rows keyed `Surfaces` are such duties, which this document does not state; a row
keyed by a section is a rule of the evaluator, and a case or a test proves it.

This crate runs the corpus with `cargo test -p docbrain-knowledge --test conformance`, and checks the map
with `--test coverage`: every token in it names a case, a test or an owner that exists, every edge case
has its line, and that line states it in the words of §Edge cases.

A reader passes when, for each op it implements, every case of that op gives exactly the expected answer;
the title and first paragraph are a record's own text and are not compared. Cases with unknown keys, `x-`
keys, unknown check kinds and ops, and a newer schema that supersedes a schema-1 record (`schema-grease-*`) are
part of the corpus on purpose: a reader that rejects an extension point fails. A reader that disagrees with
a case has found a bug in the reader, the case or this document, and the three are fixed together.
