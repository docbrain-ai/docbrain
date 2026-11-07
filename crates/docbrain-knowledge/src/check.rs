// SPDX-License-Identifier: MIT
//! The state of one check and of one record (`SEMANTICS.md`
//! §States).

use std::sync::Arc;

use crate::bounds::{CODE_FILE_MAX, CODE_TEXT_MAX_BYTES, CODE_TEXT_MIN_SCALARS};
use crate::path::RepoGlob;
use crate::reason::{Bound, Fail, Invalid, Shown, Unverifiable};
use crate::record::{Anchor, Check, Expectation, Id, Layer, Verb};
use crate::resolve::{FileCache, Resolved, link_above, link_reason, parent, resolve};
use crate::scalar::{Scalar, compare};
use crate::tree::{Tree, TreeError};
use crate::value::Value;

/// One check's state in a tree, recomputed, never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckState {
    /// The value shown when it is a scalar.
    Holds(Option<Shown>),
    Broken(Fail),
    Stale(Fail),
    Unverifiable(Unverifiable),
}

/// One record's state: `invalid`, else `superseded`, else the worst of its checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordState {
    Invalid { reason: Invalid, line: u32, col: u32 },
    Superseded { by: Id },
    Broken,
    Stale,
    Unverifiable,
    Holds,
}

/// What one anchor gives before the verb decides between `broken` and `stale`.
enum Outcome {
    Holds(Option<Shown>),
    Fail(Fail),
    Unverifiable(Unverifiable),
}

/// One check in `tree`. An unknown anchor or expectation is `unknown`; an op on a `path` or a `code` is
/// `not-comparable`, judged before the tree is read.
pub fn eval_check(tree: &dyn Tree, check: &Check, cache: &mut FileCache) -> Result<CheckState, TreeError> {
    let outcome = match (&check.anchor, &check.expect) {
        (Anchor::Unknown(_), _) | (_, Expectation::Unknown(_)) => Outcome::Unverifiable(Unverifiable::Unknown),
        (Anchor::Path(_) | Anchor::Code { .. }, Expectation::Compare { .. }) => {
            Outcome::Unverifiable(Unverifiable::NotComparable)
        }
        (Anchor::Path(g), Expectation::Exist) => path(tree, g)?,
        (Anchor::Code { file, text }, Expectation::Exist) => code(tree, file, text, cache)?,
        (Anchor::ConfigKey(layers), e) => config_key(tree, layers, e, cache)?,
    };
    Ok(match outcome {
        Outcome::Holds(shown) => CheckState::Holds(shown),
        Outcome::Fail(f) => match check.verb {
            Verb::Must => CheckState::Broken(f),
            Verb::Assumes => CheckState::Stale(f),
        },
        Outcome::Unverifiable(u) => CheckState::Unverifiable(u),
    })
}

/// A match holds; a literal naming a directory holds while an entry lies under it (edge case 42).
fn path(tree: &dyn Tree, g: &RepoGlob) -> Result<Outcome, TreeError> {
    let base = if g.is_literal() { parent(&g.root) } else { g.literal_dir() };
    if let Some(u) = link_above(tree, base)? {
        return Ok(Outcome::Unverifiable(u));
    }
    if g.is_literal() {
        return Ok(match tree.entry(&g.root)? {
            Some(e) => link_reason(e.mode).map_or(Outcome::Holds(None), Outcome::Unverifiable),
            None if !tree.list(&g.root)?.is_empty() => Outcome::Holds(None),
            None => Outcome::Fail(Fail::MissingFile),
        });
    }
    // any regular match holds; only link or submodule matches give their reason.
    let mut reason = None;
    for (p, e) in tree.list(g.literal_dir())? {
        if !g.matches(&p) {
            continue;
        }
        match link_reason(e.mode) {
            None => return Ok(Outcome::Holds(None)),
            Some(u) => {
                reason.get_or_insert(u);
            }
        }
    }
    Ok(reason.map_or(Outcome::Fail(Fail::MissingFile), Outcome::Unverifiable))
}

fn config_key(tree: &dyn Tree, layers: &[Layer], expect: &Expectation, cache: &mut FileCache) -> Result<Outcome, TreeError> {
    Ok(match resolve(tree, layers, cache)? {
        Resolved::Missing(f) => Outcome::Fail(f),
        Resolved::Unverifiable(u) => Outcome::Unverifiable(u),
        Resolved::Present(Value::Scalar(s)) => match expect {
            Expectation::Compare { op, literal } => match compare(&s, *op, literal) {
                Ok(true) => Outcome::Holds(shown(&s)),
                Ok(false) => Outcome::Fail(Fail::False { actual: shown(&s).unwrap_or_else(|| Shown::new("")) }),
                Err(u) => Outcome::Unverifiable(u),
            },
            Expectation::Exist | Expectation::Unknown(_) => Outcome::Holds(shown(&s)),
        },
        Resolved::Present(_) => match expect {
            Expectation::Compare { .. } => Outcome::Unverifiable(Unverifiable::NotAScalar),
            Expectation::Exist | Expectation::Unknown(_) => Outcome::Holds(None),
        },
    })
}

/// A scalar as read; a value no op compares (`.inf`) has no text to show.
fn shown(s: &Scalar) -> Option<Shown> {
    match s {
        Scalar::Number(d) => Some(Shown::new(&d.to_string())),
        Scalar::Str(t) | Scalar::Datetime(t) => Some(Shown::new(t)),
        Scalar::Bool(b) => Some(Shown::new(if *b { "true" } else { "false" })),
        Scalar::Null => Some(Shown::new("null")),
        Scalar::Opaque(_) => None,
    }
}

/// the whitespace removed from a `code` text and from the file before the text is looked for.
fn squeeze(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, '\u{9}' | '\u{a}' | '\u{c}' | '\u{d}' | ' ')).collect()
}

/// The file is looked up by its literal path: a `code` file is never a glob (the record is invalid).
fn code(tree: &dyn Tree, file: &RepoGlob, text: &str, cache: &mut FileCache) -> Result<Outcome, TreeError> {
    if let Some(u) = link_above(tree, parent(&file.root))? {
        return Ok(Outcome::Unverifiable(u));
    }
    let Some(e) = tree.entry(&file.root)? else { return Ok(Outcome::Fail(Fail::MissingFile)) };
    if let Some(u) = link_reason(e.mode) {
        return Ok(Outcome::Unverifiable(u));
    }
    // the cap is on the text as parsed, the floor on scalar values after removal.
    let needle = squeeze(text);
    if text.len() > CODE_TEXT_MAX_BYTES || needle.chars().count() < CODE_TEXT_MIN_SCALARS {
        return Ok(Outcome::Unverifiable(Unverifiable::Bound(Bound::CodeText)));
    }
    if e.size > CODE_FILE_MAX {
        return Ok(Outcome::Unverifiable(Unverifiable::Bound(Bound::AnchoredFile)));
    }
    let hay = match cache.code(&e.oid) {
        Some(h) => h,
        None => {
            let blobs = tree.read_many(std::slice::from_ref(&e.oid))?;
            let Some(bytes) = blobs.first() else { return Err(TreeError::MissingObject(e.oid)) };
            // Again on the bytes: a partial clone reports size 0 for a blob it does not hold.
            let h = if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > CODE_FILE_MAX {
                Err(Unverifiable::Bound(Bound::AnchoredFile))
            } else {
                crate::resolve::text(bytes).map(|t| Arc::from(squeeze(t)))
            };
            cache.put_code(e.oid.clone(), h.clone());
            h
        }
    };
    let hay = match hay {
        Ok(h) => h,
        Err(u) => return Ok(Outcome::Unverifiable(u)),
    };
    Ok(match occurrences(&hay, &needle) {
        0 => Outcome::Fail(Fail::False { actual: Shown::new("0 occurrences") }),
        1 => Outcome::Holds(None),
        _ => Outcome::Unverifiable(Unverifiable::Ambiguous),
    })
}

/// occurrences at every start position, overlaps included, counted to 2.
fn occurrences(hay: &str, needle: &str) -> usize {
    let step = needle.chars().next().map_or(1, char::len_utf8);
    let (mut n, mut from) = (0, 0);
    while n < 2
        && let Some(i) = hay.get(from..).and_then(|h| h.find(needle))
    {
        n += 1;
        from += i + step;
    }
    n
}

/// The patterns whose matching paths are a check's inputs.
pub fn check_inputs(check: &Check) -> Vec<RepoGlob> {
    match &check.anchor {
        Anchor::Path(g) => vec![g.clone()],
        Anchor::ConfigKey(layers) => layers.iter().map(|l| l.file.clone()).collect(),
        Anchor::Code { file, .. } => vec![file.clone()],
        Anchor::Unknown(_) => Vec::new(),
    }
}

/// `broken` over `stale` over `unverifiable` over `holds`; no checks hold.
pub fn worst(checks: &[CheckState]) -> RecordState {
    let any = |f: fn(&CheckState) -> bool| checks.iter().any(f);
    if any(|c| matches!(c, CheckState::Broken(_))) {
        RecordState::Broken
    } else if any(|c| matches!(c, CheckState::Stale(_))) {
        RecordState::Stale
    } else if any(|c| matches!(c, CheckState::Unverifiable(_))) {
        RecordState::Unverifiable
    } else {
        RecordState::Holds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{Parsed, Record, parse_record};
    use crate::tree::MemTree;

    fn record(checks: &str) -> Record {
        let src = format!("---\nknowledge: 1\nid: 'k3v9q2x7m1ab'\nkind: 'fact'\ntitle: 't'\nchecks:\n{checks}---\nWhy.\n");
        match parse_record("", src.as_bytes()) {
            Parsed::Valid(r) => r,
            other => panic!("{other:?}\n{src}"),
        }
    }
    fn one(t: &MemTree, check: &str) -> CheckState {
        eval_check(t, &record(check).checks[0], &mut FileCache::default()).unwrap()
    }
    fn path(p: &str, expect: &str) -> String {
        format!("  - path: '{p}'\n    must: '{expect}'\n")
    }
    fn key(k: &str, verb: &str, expect: &str) -> String {
        format!("  - config_key: '{k}'\n    {verb}: '{expect}'\n")
    }
    fn code(f: &str, text: &str) -> String {
        format!("  - code: '{f}'\n    text: '{}'\n    must: 'exist'\n", text.replace('\'', "''"))
    }
    fn holds(s: &str) -> CheckState {
        CheckState::Holds(Some(Shown::new(s)))
    }
    fn unv(u: Unverifiable) -> CheckState {
        CheckState::Unverifiable(u)
    }
    const RETRY: &str = "retry(3, backoff::exponential());";

    #[test]
    fn missing_is_never_unverifiable() {
        let mut t = MemTree::new();
        assert_eq!(one(&t, &path("a.yaml", "exist")), CheckState::Broken(Fail::MissingFile));
        assert_eq!(one(&t, "  - path: 'a.yaml'\n    assumes: 'exist'\n"), CheckState::Stale(Fail::MissingFile));
        assert_eq!(one(&t, &key("a.yaml#x", "must", "exist")), CheckState::Broken(Fail::MissingFile));
        assert_eq!(one(&t, &code("src/lib.rs", RETRY)), CheckState::Broken(Fail::MissingFile));
        t.file("a.yaml", "x: 1\n");
        assert_eq!(one(&t, &key("a.yaml#y", "must", "== 1")), CheckState::Broken(Fail::MissingKey));
        assert_eq!(one(&t, &key("a.yaml#y", "assumes", "== 1")), CheckState::Stale(Fail::MissingKey));
        assert_eq!(one(&t, &key("a.yaml#x", "must", "== 2")), CheckState::Broken(Fail::False { actual: Shown::new("1") }));
        assert_eq!(one(&t, &key("a.yaml#x", "assumes", "== 2")), CheckState::Stale(Fail::False { actual: Shown::new("1") }));
        assert_eq!(one(&t, &key("a.yaml#x", "must", "== 1")), holds("1"));
        assert_eq!(one(&t, &key("a.yaml#x", "must", "exist")), holds("1"));
    }

    #[test]
    fn config_key_values() {
        let mut t = MemTree::new();
        t.file("v.yaml", "m:\n  a: 1\nl: [1]\ns: 'x'\nb: true\nn: null\nf: 1.50\ni: .inf\nbig: 1.0e+400\nneg: -0.25\n");
        // A map or a list is present for `exist` and not a scalar under an op.
        assert_eq!(one(&t, &key("v.yaml#m", "must", "exist")), CheckState::Holds(None));
        assert_eq!(one(&t, &key("v.yaml#l", "must", "exist")), CheckState::Holds(None));
        assert_eq!(one(&t, &key("v.yaml#m", "must", "== 1")), unv(Unverifiable::NotAScalar));
        assert_eq!(one(&t, &key("v.yaml#l", "must", "!= 1")), unv(Unverifiable::NotAScalar));
        // Values shown as read.
        assert_eq!(one(&t, &key("v.yaml#s", "must", "== x")), holds("x"));
        assert_eq!(one(&t, &key("v.yaml#b", "must", "== false")), CheckState::Broken(Fail::False { actual: Shown::new("true") }));
        assert_eq!(one(&t, &key("v.yaml#n", "must", "== 1")), CheckState::Broken(Fail::False { actual: Shown::new("null") }));
        assert_eq!(one(&t, &key("v.yaml#f", "must", "== 2")), CheckState::Broken(Fail::False { actual: Shown::new("1.5") }));
        assert_eq!(one(&t, &key("v.yaml#neg", "must", "== 2")), CheckState::Broken(Fail::False { actual: Shown::new("-0.25") }));
        t.file("v.toml", "d = 1979-05-27T07:32:00Z\n");
        assert_eq!(
            one(&t, &key("v.toml#d", "must", "== 1979-05-28T07:32:00Z")),
            CheckState::Broken(Fail::False { actual: Shown::new("1979-05-27T07:32:00Z") })
        );
        assert_eq!(one(&t, &key("v.toml#d", "must", "exist")), holds("1979-05-27T07:32:00Z"));
        // `.inf` is present, and no op compares it.
        assert_eq!(one(&t, &key("v.yaml#i", "must", "exist")), CheckState::Holds(None));
        assert_eq!(one(&t, &key("v.yaml#i", "must", "> 1")), unv(Unverifiable::NotComparable));
        // A huge exponent is shown short, never expanded.
        match one(&t, &key("v.yaml#big", "must", "== 1")) {
            CheckState::Broken(Fail::False { actual }) => assert_eq!(actual.as_str(), "1e400"),
            other => panic!("{other:?}"),
        }
        // An unknown op or check kind is unknown.
        assert_eq!(one(&t, &key("v.yaml#s", "must", "~= x")), unv(Unverifiable::Unknown));
        assert_eq!(one(&t, "  - service: 'api'\n    must: 'exist'\n"), unv(Unverifiable::Unknown));
    }

    #[test]
    fn directory_literal_exists_while_file_under_it() {
        let mut t = MemTree::new();
        t.file("ansible/roles/vault/tasks/main.yml", "- name: x\n");
        assert_eq!(one(&t, &path("ansible/roles/vault", "exist")), CheckState::Holds(None));
        assert_eq!(one(&t, &path("ansible/roles/vault/tasks/main.yml", "exist")), CheckState::Holds(None));
        assert_eq!(one(&t, &path("ansible/roles/vaul", "exist")), CheckState::Broken(Fail::MissingFile));
        t.remove("ansible/roles/vault/tasks/main.yml");
        assert_eq!(one(&t, &path("ansible/roles/vault", "exist")), CheckState::Broken(Fail::MissingFile));
        // Edge case 41: `docs/**` is every file under `docs/`, never a file named `docs`.
        t.file("docs", "x");
        assert_eq!(one(&t, &path("docs/**", "exist")), CheckState::Broken(Fail::MissingFile));
        t.remove("docs").file("docs/a/b.md", "x");
        assert_eq!(one(&t, &path("docs/**", "exist")), CheckState::Holds(None));
        // Edge case 40: brackets are literal.
        t.file("app/[slug]/page.tsx", "x");
        assert_eq!(one(&t, &path("app/[slug]/page.tsx", "exist")), CheckState::Holds(None));
        assert_eq!(one(&t, &path("app/s/page.tsx", "exist")), CheckState::Broken(Fail::MissingFile));
        // only `exist` on a path.
        assert_eq!(one(&t, &path("docs/**", "== 1")), unv(Unverifiable::NotComparable));
        assert_eq!(one(&t, &path("docs/**", "~= 1")), unv(Unverifiable::Unknown));
    }

    #[test]
    fn links_and_submodules() {
        let mut t = MemTree::new();
        t.file("real.yaml", "a: 1\n").symlink("link.yaml", "real.yaml");
        assert_eq!(one(&t, &path("link.yaml", "exist")), unv(Unverifiable::SymbolicLink));
        t.symlink("deploy", "elsewhere");
        assert_eq!(one(&t, &path("deploy/values.yaml", "exist")), unv(Unverifiable::SymbolicLink));
        assert_eq!(one(&t, &path("deploy/*.yaml", "exist")), unv(Unverifiable::SymbolicLink));
        // for a `path` glob, any regular match holds; only link matches give the reason.
        t.file("g/a.yaml", "x").symlink("g/b.yaml", "a.yaml");
        assert_eq!(one(&t, &path("g/*.yaml", "exist")), CheckState::Holds(None));
        t.remove("g/a.yaml");
        assert_eq!(one(&t, &path("g/*.yaml", "exist")), unv(Unverifiable::SymbolicLink));
        // Only links and submodules match: the first in path order gives the reason.
        t.symlink("m/a.yaml", "x").submodule("m/b.yaml", "4d7a2c1f00000000000000000000000000000000");
        assert_eq!(one(&t, &path("m/*.yaml", "exist")), unv(Unverifiable::SymbolicLink));
        t.remove("m/a.yaml").symlink("m/c.yaml", "x");
        assert_eq!(one(&t, &path("m/*.yaml", "exist")), unv(Unverifiable::Submodule));
        t.submodule("vendor/lib", "4d7a2c1f00000000000000000000000000000000");
        assert_eq!(one(&t, &path("vendor/lib", "exist")), unv(Unverifiable::Submodule));
        assert_eq!(one(&t, &path("vendor/lib/x.c", "exist")), unv(Unverifiable::Submodule));
        assert_eq!(one(&t, &key("vendor/lib/x.yaml#a", "must", "exist")), unv(Unverifiable::Submodule));
        assert_eq!(one(&t, &code("vendor/lib/x.c", RETRY)), unv(Unverifiable::Submodule));
        assert_eq!(one(&t, &code("link.yaml", RETRY)), unv(Unverifiable::SymbolicLink));
        // A directory whose only entry is a submodule still lies in the tree.
        assert_eq!(one(&t, &path("vendor", "exist")), CheckState::Holds(None));
        // An executable file is a regular file.
        t.exec("bin/run.sh", "x");
        assert_eq!(one(&t, &path("bin/*.sh", "exist")), CheckState::Holds(None));
    }

    #[test]
    fn code_check_rules() {
        let mut t = MemTree::new();
        let c = |f: &str| code(f, RETRY);
        t.file("a.rs", format!("fn main() {{\n    {RETRY}\n}}\n"));
        assert_eq!(one(&t, &c("a.rs")), CheckState::Holds(None));
        // Edge case 63: re-wrapped and re-indented.
        t.file("a.rs", "fn main() {\n\tretry(\n\t\t3,\n\t\tbackoff::exponential()\n\t);\n}\n");
        assert_eq!(one(&t, &c("a.rs")), CheckState::Holds(None));
        // CRLF and form feed are whitespace.
        t.file("a.rs", "retry(3,\r\n\x0Cbackoff::exponential());\r\n");
        assert_eq!(one(&t, &c("a.rs")), CheckState::Holds(None));
        // Edge case 64: left in a comment.
        t.file("a.rs", format!("// {RETRY}\n"));
        assert_eq!(one(&t, &c("a.rs")), CheckState::Holds(None));
        // Edge case 48: the guarded line is gone, or one character changed.
        t.file("a.rs", "retry(4, backoff::exponential());\n");
        assert_eq!(one(&t, &c("a.rs")), CheckState::Broken(Fail::False { actual: Shown::new("0 occurrences") }));
        // U+000B is not removed; edge case 62: a no-break space is a character.
        t.file("a.rs", "retry(3,\x0Bbackoff::exponential());\n");
        assert_eq!(one(&t, &c("a.rs")), CheckState::Broken(Fail::False { actual: Shown::new("0 occurrences") }));
        t.file("a.rs", "retry(3,\u{a0}backoff::exponential());\n");
        assert_eq!(one(&t, &code("a.rs", "retry(3, backoff::exponential());")), CheckState::Broken(Fail::False { actual: Shown::new("0 occurrences") }));
        assert_eq!(one(&t, &code("a.rs", "retry(3,\u{a0}backoff::exponential());")), CheckState::Holds(None));
        // Twice, and overlapping occurrences count.
        t.file("a.rs", format!("{RETRY}\n{RETRY}\n"));
        assert_eq!(one(&t, &c("a.rs")), unv(Unverifiable::Ambiguous));
        let run = |n: usize| "a".repeat(n);
        t.file("b.rs", run(CODE_TEXT_MIN_SCALARS + 1));
        assert_eq!(one(&t, &code("b.rs", &run(CODE_TEXT_MIN_SCALARS))), unv(Unverifiable::Ambiguous));
        assert_eq!(one(&t, &code("b.rs", &run(CODE_TEXT_MIN_SCALARS + 1))), CheckState::Holds(None));
        // The floor counts scalar values after whitespace is removed; the cap is bytes as written.
        let e = |n: usize| "é".repeat(n);
        t.file("c.rs", e(40));
        assert_eq!(one(&t, &code("c.rs", &e(CODE_TEXT_MIN_SCALARS - 1))), unv(Unverifiable::Bound(Bound::CodeText)));
        assert_eq!(one(&t, &code("c.rs", &format!("{} {}", e(12), e(11)))), unv(Unverifiable::Bound(Bound::CodeText)));
        assert_eq!(one(&t, &code("c.rs", &e(CODE_TEXT_MIN_SCALARS))), unv(Unverifiable::Ambiguous));
        let long = format!("{}{}", run(CODE_TEXT_MAX_BYTES - 1), " ");
        t.file("d.rs", run(CODE_TEXT_MAX_BYTES - 1));
        assert_eq!(one(&t, &code("d.rs", &long)), CheckState::Holds(None));
        assert_eq!(one(&t, &code("d.rs", &format!("{long} "))), unv(Unverifiable::Bound(Bound::CodeText)));
        // Not UTF-8; an LFS pointer.
        t.file("e.rs", b"retry(3, backoff::exponential());\xff\n");
        assert_eq!(one(&t, &c("e.rs")), unv(Unverifiable::Encoding));
        t.file("e.rs", "version https://git-lfs.github.com/spec/v1\noid sha256:4d7a\nsize 1\n");
        assert_eq!(one(&t, &c("e.rs")), unv(Unverifiable::LfsPointer));
        // only `exist` on `code`.
        let op = format!("  - code: 'a.rs'\n    text: '{RETRY}'\n    must: '== 1'\n");
        assert_eq!(one(&t, &op), unv(Unverifiable::NotComparable));
        // A missing file is never unverifiable, even with a text under the floor.
        assert_eq!(one(&t, &code("none.rs", "short")), CheckState::Broken(Fail::MissingFile));
    }

    /// Reports `huge.rs` one byte over the anchored-file bound and `edge.rs` at it, and counts blob reads.
    struct Sizes<'a> {
        inner: &'a MemTree,
        reads: std::cell::Cell<usize>,
    }
    impl Tree for Sizes<'_> {
        fn entry(&self, path: &str) -> Result<Option<crate::tree::Entry>, TreeError> {
            Ok(self.inner.entry(path)?.map(|mut e| {
                match path {
                    "huge.rs" => e.size = CODE_FILE_MAX + 1,
                    "edge.rs" => e.size = CODE_FILE_MAX,
                    _ => {}
                }
                e
            }))
        }
        fn list(&self, dir: &str) -> Result<Vec<(String, crate::tree::Entry)>, TreeError> {
            self.inner.list(dir)
        }
        fn read_many(&self, oids: &[crate::tree::Oid]) -> Result<Vec<std::sync::Arc<[u8]>>, TreeError> {
            self.reads.set(self.reads.get() + 1);
            self.inner.read_many(oids)
        }
    }

    #[test]
    fn code_file_bound_on_the_bytes_read() {
        // A blobless clone reports size 0 for a blob it does not hold: the bound is judged again once read.
        let mut t = MemTree::new();
        t.file("huge.rs", RETRY).file("small.rs", format!("{RETRY}\n"));
        // Exactly at the bound once read: read.
        t.file("edge.rs", format!("{RETRY}{}", " ".repeat(usize::try_from(CODE_FILE_MAX).unwrap() - RETRY.len())));
        let big = t.entry("huge.rs").unwrap().unwrap().oid;
        let tree = crate::tree::Unsized { inner: &t, inflate: vec![big] };
        let eval = |c: &str| eval_check(&tree, &record(c).checks[0], &mut FileCache::default()).unwrap();
        assert_eq!(eval(&code("huge.rs", RETRY)), unv(Unverifiable::Bound(Bound::AnchoredFile)));
        assert_eq!(eval(&code("small.rs", RETRY)), CheckState::Holds(None));
        assert_eq!(eval(&code("edge.rs", RETRY)), CheckState::Holds(None));
    }

    #[test]
    fn code_file_bound_from_the_entry() {
        let mut t = MemTree::new();
        t.file("huge.rs", RETRY).file("edge.rs", RETRY);
        let tree = Sizes { inner: &t, reads: std::cell::Cell::new(0) };
        let eval = |c: &str| eval_check(&tree, &record(c).checks[0], &mut FileCache::default()).unwrap();
        assert_eq!(eval(&code("huge.rs", RETRY)), unv(Unverifiable::Bound(Bound::AnchoredFile)));
        assert_eq!(tree.reads.get(), 0);
        assert_eq!(eval(&code("edge.rs", RETRY)), CheckState::Holds(None));
        assert_eq!(tree.reads.get(), 1);
    }

    #[test]
    fn record_state_worst_wins() {
        use CheckState as C;
        let h = C::Holds(None);
        let b = C::Broken(Fail::MissingKey);
        let s = C::Stale(Fail::MissingKey);
        let u = C::Unverifiable(Unverifiable::Format);
        assert_eq!(worst(&[]), RecordState::Holds);
        assert_eq!(worst(std::slice::from_ref(&h)), RecordState::Holds);
        assert_eq!(worst(&[h.clone(), u.clone()]), RecordState::Unverifiable);
        assert_eq!(worst(&[u.clone(), s.clone(), h.clone()]), RecordState::Stale);
        assert_eq!(worst(&[s.clone(), b.clone(), u.clone()]), RecordState::Broken);
        assert_eq!(worst(&[b, s, u, h]), RecordState::Broken);
    }

    #[test]
    fn inputs_of_each_anchor() {
        let r = record(&format!(
            "{}{}{}  - service: 'x'\n    must: 'exist'\n",
            path("docs/**", "exist"),
            "  - config_key:\n      - 'a/base.yaml#x'\n      - 'a/prod/*.yaml#x'\n    must: 'exist'\n",
            code("src/lib.rs", RETRY)
        ));
        let roots: Vec<Vec<String>> = r.checks.iter().map(|c| check_inputs(c).into_iter().map(|g| g.root).collect()).collect();
        assert_eq!(roots, vec![vec!["docs/**".to_owned()], vec!["a/base.yaml".into(), "a/prod/*.yaml".into()], vec!["src/lib.rs".into()], vec![]]);
    }
}
