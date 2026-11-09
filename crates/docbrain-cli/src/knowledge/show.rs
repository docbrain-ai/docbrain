// SPDX-License-Identifier: MIT
//! The words every command shares: an anchor, a check line, a record's state, the hint that names the fix.

use std::borrow::Cow;

use docbrain_knowledge::check::{CheckState, RecordState};
use docbrain_knowledge::record::{Anchor, Check};
use docbrain_knowledge::secret;
use docbrain_knowledge::{Fail, Shown};

use super::frame::esc;

/// A check's anchor in one string, as a command prints it (`shown_anchor`): a `path`'s root glob; a `config_key`'s
/// first file and key path, with how many layers follow; a `code` check's file.
pub(crate) fn anchor_text(c: &Check) -> String {
    let at = esc(&shown_anchor(&anchor_path(c))).into_owned();
    match &c.anchor {
        Anchor::Path(_) => at,
        Anchor::ConfigKey(layers) => match layers.len() {
            0 | 1 => at,
            2 => format!("{at} (+1 layer)"),
            n => format!("{at} (+{} layers)", n - 1),
        },
        Anchor::Code { .. } => format!("{at} (code)"),
        Anchor::Unknown(_) => format!("{at} (unknown)"),
    }
}

/// An anchor in one string (`anchor_path`, a finding's anchor, a followed path) as a command prints it: as written;
/// when each secret shape in it starts after its first `#`, the file before it and the rest withheld
/// (`api/values.yaml#(withheld: secret-shape)`), so the reader still knows which file to open; else withheld whole.
pub(crate) fn shown_anchor(a: &str) -> Cow<'_, str> {
    let hits = secret::screen(a);
    if hits.is_empty() {
        return Cow::Borrowed(a);
    }
    if let Some((file, _)) = a.split_once('#') {
        // Where the `#` stands, counted as the screen counts a hit's line and column.
        let (mut line, mut col) = (1u32, 1u32);
        for (j, c) in file.char_indices() {
            if c == '\n' {
                (line, col) = (line.saturating_add(1), 1);
            } else if !(j == 0 && c == '\u{FEFF}') {
                col = col.saturating_add(1);
            }
        }
        if hits.iter().all(|h| (h.line, h.col) > (line, col)) {
            return Cow::Owned(format!("{file}#{WITHHELD_VALUE}"));
        }
    }
    Cow::Borrowed(WITHHELD_VALUE)
}

/// A check's anchor as JSON and the pathspec read it, unescaped: a `path`'s or a `code` check's root path, a
/// `config_key`'s `<file>#<key path>` (its first layer), an unknown check kind's key.
pub(crate) fn anchor_path(c: &Check) -> String {
    match &c.anchor {
        Anchor::Path(g) | Anchor::Code { file: g, .. } => g.root.clone(),
        Anchor::ConfigKey(layers) => layers.first().map_or_else(String::new, |l| format!("{}#{}", l.file.root, l.key.text)),
        Anchor::Unknown(kind) => kind.clone(),
    }
}

/// The files a check names, from the root, its first layer first; a glob stays a glob.
pub(crate) fn anchored_files(c: &Check) -> Vec<&str> {
    match &c.anchor {
        Anchor::Path(g) | Anchor::Code { file: g, .. } => vec![g.root.as_str()],
        Anchor::ConfigKey(layers) => layers.iter().map(|l| l.file.root.as_str()).collect(),
        Anchor::Unknown(_) => Vec::new(),
    }
}

/// What a command prints in place of text read from the repository that holds a secret shape (`SEMANTICS.md`
/// §Secrets: a hit is named, never its text).
pub(crate) const WITHHELD_VALUE: &str = "(withheld: secret-shape)";

/// Text read from the repository (a value, a title, an expectation as written) as a command prints it.
pub(crate) fn clean(text: &str) -> &str {
    if secret::screen(text).is_empty() { text } else { WITHHELD_VALUE }
}

pub(crate) fn shown(v: &Shown) -> &str {
    clean(v.as_str())
}

/// `<verb> <anchor> <expectation as written>  <state>[  <reason or shown value>]`. `untracked` says whether a file
/// (or glob) the check names is in the working tree without being tracked: a missing file then says to add it.
pub(crate) fn check_line(c: &Check, st: &CheckState, untracked: &dyn Fn(&str) -> bool) -> String {
    let expect = c.expect.to_string();
    let expect = clean(&expect);
    let head = format!("{} {} {}", c.verb, anchor_text(c), esc(expect));
    let fail = |f: &Fail| match f {
        Fail::False { actual } => format!("false: {}, expected {}", esc(shown(actual)), esc(expect)),
        Fail::MissingFile if anchored_files(c).into_iter().any(untracked) => {
            "missing-file (not tracked: git add it first)".to_owned()
        }
        other => other.code().into_owned(),
    };
    let word = check_word(st);
    match st {
        CheckState::Holds(Some(v)) if !v.as_str().is_empty() => format!("{head}  {word}  {}", esc(shown(v))),
        CheckState::Holds(_) => format!("{head}  {word}"),
        CheckState::Broken(f) | CheckState::Stale(f) => format!("{head}  {word}  {}", fail(f)),
        CheckState::Unverifiable(u) => {
            let code = u.code();
            match hint(&code) {
                Some(h) => format!("{head}  {word}  {code}: {h}"),
                None => format!("{head}  {word}  {code}"),
            }
        }
    }
}

/// A check's state as one word.
pub(crate) fn check_word(st: &CheckState) -> &'static str {
    match st {
        CheckState::Holds(_) => "holds",
        CheckState::Broken(_) => "broken",
        CheckState::Stale(_) => "stale",
        CheckState::Unverifiable(_) => "unverifiable",
    }
}

/// A record's state as one word.
pub(crate) fn state_word(st: &RecordState) -> &'static str {
    match st {
        RecordState::Invalid { .. } => "invalid",
        RecordState::Superseded { .. } => "superseded",
        RecordState::Broken => "broken",
        RecordState::Stale => "stale",
        RecordState::Unverifiable => "unverifiable",
        RecordState::Holds => "holds",
    }
}

/// What to do about a reason, where one thing fixes it.
pub(crate) fn hint(code: &str) -> Option<&'static str> {
    Some(match code {
        "number-or-version" => "write a leading v for a version (v1.29)",
        "lfs-pointer" => "the file is stored in Git LFS; anchor another file",
        "template" => "rendered values cannot be checked; anchor the values file",
        "ambiguous" => "the key matches more than one value; add a selector",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use docbrain_knowledge::check::eval_check;
    use docbrain_knowledge::path::parse_rel;
    use docbrain_knowledge::record::{Expectation, Id, Parsed, Record, Verb, parse_record};
    use docbrain_knowledge::resolve::FileCache;
    use docbrain_knowledge::{Fail, Invalid, MemTree, Shown, Unverifiable};

    fn record(scope: &str, check: &str) -> Record {
        let text = format!("---\nknowledge: 1\nid: '7k2m9q4r8t1v'\nkind: 'decision'\ntitle: 't'\nchecks:\n{check}---\nWhy.\n");
        match parse_record(scope, text.as_bytes()) {
            Parsed::Valid(r) => r,
            other => panic!("{other:?}"),
        }
    }

    /// Each check in its own record, judged in `tree`.
    fn lines(tree: &MemTree, checks: &[&str], untracked: &dyn Fn(&str) -> bool) -> Vec<String> {
        let mut cache = FileCache::default();
        checks
            .iter()
            .map(|c| {
                let c = &record("", c).checks[0];
                check_line(c, &eval_check(tree, c, &mut cache).unwrap(), untracked)
            })
            .collect()
    }

    const REPLICAS: &str = "  - config_key: 'charts/api/values.yaml#replicaCount'\n    must: '>= 2'\n";
    const RUNBOOK: &str = "  - path: 'docs/runbooks/api-rollout.md'\n    must: 'exist'\n";
    const TAG: &str = "  - config_key: 'charts/api/values.yaml#image.tag'\n    assumes: '>= 1.29'\n";
    const TYPO: &str = "  - path: 'docs/runbooks/api-rolout.md'\n    must: 'exist'\n";

    #[test]
    fn check_lines_match_the_transcripts() {
        let mut t = MemTree::new();
        t.file("charts/api/values.yaml", "replicaCount: 1\nimage:\n  tag: '1.3'\n");
        t.file("docs/runbooks/api-rollout.md", "# Rollout\n");
        let untracked = |p: &str| p == "docs/runbooks/api-rolout.md";
        assert_eq!(
            lines(&t, &[REPLICAS, RUNBOOK, TAG, TYPO], &untracked),
            [
                "must charts/api/values.yaml#replicaCount >= 2  broken  false: 1, expected >= 2",
                "must docs/runbooks/api-rollout.md exist  holds",
                "assumes charts/api/values.yaml#image.tag >= 1.29  unverifiable  number-or-version: write a leading v for a version (v1.29)",
                "must docs/runbooks/api-rolout.md exist  broken  missing-file (not tracked: git add it first)",
            ]
        );
        t.file("charts/api/values.yaml", "replicaCount: 2\n");
        assert_eq!(lines(&t, &[REPLICAS], &untracked), ["must charts/api/values.yaml#replicaCount >= 2  holds  2"]);
        // Not in the working tree at all: no note.
        assert_eq!(lines(&t, &[TYPO], &|_| false), ["must docs/runbooks/api-rolout.md exist  broken  missing-file"]);
    }

    #[test]
    fn check_lines_for_every_state() {
        let mut t = MemTree::new();
        t.file("v.yaml", "a: 1\n");
        let stale = "  - path: 'gone.md'\n    assumes: 'exist'\n";
        let key = "  - config_key: 'v.yaml#b'\n    must: '== 1'\n";
        let format = "  - config_key: 'v.yaml#a'\n    must: 'between 1 and 2'\n";
        assert_eq!(
            lines(&t, &[stale, key, format], &|_| false),
            [
                "assumes gone.md exist  stale  missing-file",
                "must v.yaml#b == 1  broken  missing-key",
                "must v.yaml#a between 1 and 2  unverifiable  unknown",
            ]
        );
        let c = Check { anchor: Anchor::Unknown("regex".into()), verb: Verb::Must, expect: Expectation::Exist, line: 3 };
        let st = |s| check_line(&c, &s, &|_| true);
        assert_eq!(st(CheckState::Unverifiable(Unverifiable::Unknown)), "must regex (unknown) exist  unverifiable  unknown");
        assert_eq!(st(CheckState::Unverifiable(Unverifiable::Template)), "must regex (unknown) exist  unverifiable  template: rendered values cannot be checked; anchor the values file");
        assert_eq!(st(CheckState::Holds(None)), "must regex (unknown) exist  holds");
        assert_eq!(st(CheckState::Holds(Some(Shown::new("")))), "must regex (unknown) exist  holds", "no trailing spaces");
        let ctl = CheckState::Broken(Fail::False { actual: Shown::new("a\u{1b}[2Jb") });
        let c = Check { expect: Expectation::Unknown("== \u{7}".into()), ..c };
        assert_eq!(check_line(&c, &ctl, &|_| false), "must regex (unknown) == \\x{7}  broken  false: a\\x{1b}[2Jb, expected == \\x{7}");
    }

    #[test]
    fn a_secret_shaped_value_is_withheld() {
        let mut t = MemTree::new();
        let values = "db:\n  password: AKIAQ7ZK3M9W2X4V8B6N\n  url: postgres://app:changeme@db/app\n  key: xAKIAQ7ZK3M9W2X4V8B6N\n";
        t.file("v.yaml", values);
        let set = "  - config_key: 'v.yaml#db.password'\n    must: 'exist'\n";
        let documented = "  - config_key: 'v.yaml#db.url'\n    must: 'exist'\n";
        let compared = "  - config_key: 'v.yaml#db.key'\n    must: '== x'\n";
        assert_eq!(
            lines(&t, &[set, documented, compared], &|_| false),
            [
                "must v.yaml#db.password exist  holds  (withheld: secret-shape)",
                "must v.yaml#db.url exist  holds  postgres://app:changeme@db/app",
                "must v.yaml#db.key == x  broken  false: (withheld: secret-shape), expected == x",
            ]
        );
        // The record's own expectation, as written, is text read from the repository too.
        let literal = "  - config_key: 'v.yaml#db.url'\n    must: '== AKIAQ7ZK3M9W2X4V8B6N'\n";
        let line = &lines(&t, &[literal], &|_| false)[0];
        assert!(line.starts_with("must v.yaml#db.url (withheld: secret-shape)  ") && !line.contains("AKIA"), "{line}");
    }

    #[test]
    fn anchor_text_names_the_first_file() {
        let one = |scope: &str, check: &str| anchor_text(&record(scope, check).checks[0]);
        assert_eq!(one("", "  - config_key:\n      - 'v.yaml#a.b'\n      - 'c.json#a.b'\n    must: '>= 2'\n"), "v.yaml#a.b (+1 layer)");
        let three = "  - config_key:\n      - 'v.yaml#a'\n      - 'c.json#a'\n      - 'd/*.yaml#a'\n    must: '>= 2'\n";
        assert_eq!(one("svc", three), "svc/v.yaml#a (+2 layers)");
        assert_eq!(one("", "  - code: 'main.rs'\n    text: 'retry(3, backoff::exponential());'\n    must: 'exist'\n"), "main.rs (code)");
        assert_eq!(one("charts", "  - path: 'd/*.txt'\n    must: 'exist'\n"), "charts/d/*.txt");
        let c = Check {
            anchor: Anchor::Path(parse_rel("", "a\u{1b}]8;;x\u{7}b").unwrap()),
            verb: Verb::Assumes,
            expect: Expectation::Exist,
            line: 1,
        };
        assert_eq!(anchor_text(&c), "a\\x{1b}]8;;x\\x{7}b");
    }

    #[test]
    fn an_anchor_shows_only_what_precedes_a_secret_shape() {
        const KEY: &str = "AKIAQ7ZK3M9W2X4V8B6N";
        let shown = |a: &str| shown_anchor(a).into_owned();
        assert_eq!(shown("v.yaml#db.password"), "v.yaml#db.password");
        assert_eq!(shown(""), "");
        // Every shape after the first `#`: the file stays named.
        assert_eq!(shown(&format!("api/values.yaml#{KEY}")), "api/values.yaml#(withheld: secret-shape)");
        assert_eq!(shown(&format!("v.yaml#tokens.{KEY}.owner")), "v.yaml#(withheld: secret-shape)");
        assert_eq!(shown(&format!("v.yaml#a#{KEY}")), "v.yaml#(withheld: secret-shape)", "the key path's own `#`");
        assert_eq!(shown(&format!("a\nb.yaml#{KEY}")), "a\nb.yaml#(withheld: secret-shape)", "a line break before the `#`");
        let bom = format!("\u{FEFF}v.yaml#{KEY}");
        assert_eq!(shown(&bom), "\u{FEFF}v.yaml#(withheld: secret-shape)", "a leading byte order mark takes no column");
        // A shape that starts in the file part, or spans the `#`: withheld whole.
        assert_eq!(shown(&format!("keys/{KEY}.yaml#a")), WITHHELD_VALUE);
        assert_eq!(shown(&format!("keys/{KEY}")), WITHHELD_VALUE);
        assert_eq!(shown(&format!("keys/{KEY}#{KEY}")), WITHHELD_VALUE);
        assert_eq!(shown(&format!("a\nkeys/{KEY}.yaml#x")), WITHHELD_VALUE, "on the `#`'s own line, before it");
        let across = "s://user:pa#ss@host/v.yaml";
        assert!(secret::screen("s://user:pa").is_empty() && !secret::screen(across).is_empty(), "the shape spans the `#`");
        assert_eq!(shown(across), WITHHELD_VALUE);
    }

    #[test]
    fn a_secret_shaped_anchor_is_printed_withheld_and_read_as_written() {
        let c = |scope: &str, check: &str| record(scope, check).checks[0].clone();
        let key = c("svc", "  - config_key:\n      - 'v.yaml#tokens.AKIAQ7ZK3M9W2X4V8B6N'\n      - 'c.json#a'\n    must: 'exist'\n");
        assert_eq!(anchor_text(&key), "svc/v.yaml#(withheld: secret-shape) (+1 layer)");
        assert_eq!(anchor_path(&key), "svc/v.yaml#tokens.AKIAQ7ZK3M9W2X4V8B6N", "the anchor a command reads by");
        let code = c("", "  - code: 'AKIAQ7ZK3M9W2X4V8B6N.rs'\n    text: 'x'\n    must: 'exist'\n");
        assert_eq!(anchor_text(&code), "(withheld: secret-shape) (code)");
        let path = c("", "  - path: 'keys/AKIAQ7ZK3M9W2X4V8B6N'\n    must: 'exist'\n");
        assert_eq!(check_line(&path, &CheckState::Broken(Fail::MissingFile), &|_| false), "must (withheld: secret-shape) exist  broken  missing-file");
    }

    #[test]
    fn an_empty_layer_list_is_never_a_record() {
        let text = "---\nknowledge: 1\nid: '7k2m9q4r8t1v'\nkind: 'decision'\ntitle: 't'\nchecks:\n  - config_key: []\n    must: 'exist'\n---\nWhy.\n";
        assert!(matches!(parse_record("", text.as_bytes()), Parsed::Invalid { .. }));
    }

    #[test]
    fn state_words() {
        let id = Id::parse("7k2m9q4r8t1v").unwrap();
        let words = [
            (RecordState::Invalid { reason: Invalid::Parse, line: 1, col: 1 }, "invalid"),
            (RecordState::Superseded { by: id }, "superseded"),
            (RecordState::Broken, "broken"),
            (RecordState::Stale, "stale"),
            (RecordState::Unverifiable, "unverifiable"),
            (RecordState::Holds, "holds"),
        ];
        for (st, w) in words {
            assert_eq!(state_word(&st), w);
        }
    }

    #[test]
    fn hints_name_the_fix() {
        assert_eq!(hint("number-or-version"), Some("write a leading v for a version (v1.29)"));
        assert_eq!(hint("lfs-pointer"), Some("the file is stored in Git LFS; anchor another file"));
        assert_eq!(hint("template"), Some("rendered values cannot be checked; anchor the values file"));
        assert_eq!(hint("ambiguous"), Some("the key matches more than one value; add a selector"));
        for none in ["format", "unknown", "bound:depth", "missing-file", ""] {
            assert_eq!(hint(none), None, "{none}");
        }
    }
}
