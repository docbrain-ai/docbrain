// SPDX-License-Identifier: MIT
//! Routing: the records that are context for a set of paths, in the order `SEMANTICS.md`
//! §Routing gives.

use std::cmp::{Ordering, Reverse};

use crate::bounds::RECORDS_PER_PASS;
use crate::check::RecordState;
use crate::folder::{RecordFile, RecordStatus, folder_scopes_above, load_folders, record_status};
use crate::path::starting_with;
use crate::record::{Kind, Parsed, Verb};
use crate::resolve::FileCache;
use crate::tree::{Tree, TreeError};

/// The routed records, ordered and capped, and what the cap hides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextReport {
    pub records: Vec<RecordStatus>,
    /// Routed records past the cap.
    pub more: usize,
    /// Of those, the `broken` or `stale` ones.
    pub more_broken_or_stale: usize,
}

/// The records from every folder on each path's way up whose `applies_to` matches one of `paths`, evaluated,
/// ordered (`SEMANTICS.md` §Routing), the first `cap` kept.
pub fn context(tree: &dyn Tree, paths: &[String], cap: usize) -> Result<ContextReport, TreeError> {
    context_within(tree, paths, cap, RECORDS_PER_PASS)
}

fn context_within(tree: &dyn Tree, paths: &[String], cap: usize, budget: usize) -> Result<ContextReport, TreeError> {
    let folders = load_folders(tree, &folder_scopes_above(tree, paths)?, budget)?;
    let mut cache = FileCache::default();
    let mut sorted = paths.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut routed = Vec::new();
    for f in &folders {
        let depth = if f.scope.is_empty() { 0 } else { f.scope.split('/').count() };
        for r in &f.records {
            // Front matter decides; only a routed record is evaluated.
            let Some(rank) = route(r, &sorted) else { continue };
            let status = record_status(tree, r, &mut cache)?;
            let calm = !matches!(status.state, RecordState::Broken | RecordState::Stale);
            routed.push((Rank { calm, depth: Reverse(depth), ..rank }, status));
        }
    }
    routed.sort_by(|(a, x), (b, y)| order(a, x, b, y));
    let hidden = routed.split_off(cap.min(routed.len()));
    Ok(ContextReport {
        records: routed.into_iter().map(|(_, s)| s).collect(),
        more: hidden.len(),
        more_broken_or_stale: hidden.iter().filter(|(r, _)| !r.calm).count(),
    })
}

/// The keys of `SEMANTICS.md` §Routing that come before title and id, each ascending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Rank {
    /// Neither `broken` nor `stale`: one group, after them.
    calm: bool,
    /// The folder's depth in segments: nearer is deeper.
    depth: Reverse<usize>,
    /// The narrowest glob that matches: fewest `**`, then the longest literal prefix.
    narrow: (usize, Reverse<usize>),
    no_must: bool,
    kind: u8,
}

/// The record's routing keys when it is live and one of its `applies_to` globs matches one of `sorted` (byte
/// order). A glob is tried only on the paths that start with its literal prefix.
fn route(r: &RecordFile, sorted: &[String]) -> Option<Rank> {
    if r.superseded_by.is_some() {
        return None;
    }
    let (globs, must, kind) = match &r.parsed {
        Parsed::Valid(rec) => (&rec.applies_to, rec.checks.iter().any(|c| c.verb == Verb::Must), Some(rec.kind)),
        Parsed::Newer { applies_to, kind, .. } => (applies_to, false, *kind),
        Parsed::Invalid { .. } => return None,
    };
    let narrow = globs
        .iter()
        .filter(|g| starting_with(sorted, g.literal_prefix()).iter().any(|p| g.matches(p)))
        .map(|g| (g.star_count(), Reverse(g.literal_prefix_len())))
        .min()?;
    let kind = match kind {
        Some(Kind::Decision) => 0,
        Some(Kind::Procedure) => 1,
        Some(Kind::Fact) => 2,
        Some(Kind::Question) => 3,
        None => 4,
    };
    Some(Rank { calm: true, depth: Reverse(0), narrow, no_must: !must, kind })
}

/// `SEMANTICS.md` §Routing: the rank, then title in byte order, then id, a record without either after those with one; then path,
/// so the order is total.
fn order(a: &Rank, x: &RecordStatus, b: &Rank, y: &RecordStatus) -> Ordering {
    a.cmp(b)
        .then_with(|| (x.title.is_none(), &x.title).cmp(&(y.title.is_none(), &y.title)))
        .then_with(|| (x.id.is_none(), x.id).cmp(&(y.id.is_none(), y.id)))
        .then_with(|| x.path.cmp(&y.path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bounds::CONTEXT_CAP;
    use crate::check::RecordState;
    use crate::tree::{Entry, MemTree, Oid};
    use std::cell::RefCell;
    use std::sync::Arc;

    const R: &str = ".knowledge/records/";
    const P: &str = "svc/src/a/main.rs";

    fn id(n: usize) -> String {
        format!("k3v9q2x7m{n:03}")
    }
    fn rec(id: &str, kind: &str, title: &str, rest: &str) -> String {
        format!("---\nknowledge: 1\nid: '{id}'\nkind: '{kind}'\ntitle: '{title}'\n{rest}---\nWhy this matters.\n")
    }
    fn applies(globs: &[&str]) -> String {
        format!("applies_to: [{}]\n", globs.iter().map(|g| format!("'{g}'")).collect::<Vec<_>>().join(", "))
    }
    fn check(verb: &str, p: &str) -> String {
        format!("checks:\n  - path: '{p}'\n    {verb}: 'exist'\n")
    }
    fn paths(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| (*s).to_owned()).collect()
    }
    fn ids(r: &ContextReport) -> Vec<String> {
        r.records.iter().map(|s| s.id.map_or_else(|| s.path.clone(), |i| i.to_string())).collect()
    }

    /// Records the directories listed and the size of each read.
    struct Seen<'a> {
        inner: &'a MemTree,
        lists: RefCell<Vec<String>>,
        reads: RefCell<Vec<usize>>,
    }
    impl Tree for Seen<'_> {
        fn entry(&self, path: &str) -> Result<Option<Entry>, TreeError> {
            self.inner.entry(path)
        }
        fn list(&self, dir: &str) -> Result<Vec<(String, Entry)>, TreeError> {
            self.lists.borrow_mut().push(dir.to_owned());
            self.inner.list(dir)
        }
        fn read_many(&self, oids: &[Oid]) -> Result<Vec<Arc<[u8]>>, TreeError> {
            self.reads.borrow_mut().push(oids.len());
            self.inner.read_many(oids)
        }
    }
    fn seen(t: &MemTree) -> Seen<'_> {
        Seen { inner: t, lists: RefCell::default(), reads: RefCell::default() }
    }

    #[test]
    fn reads_only_folders_above() {
        let mut t = MemTree::new();
        t.file(&format!("{R}a.md"), rec(&id(1), "fact", "root", &applies(&["**"])))
            .file(&format!("svc/{R}b.md"), rec(&id(2), "fact", "svc", &applies(&["src/**"])))
            .file(&format!("other/{R}c.md"), rec(&id(3), "fact", "sibling", &applies(&["**"])))
            .file(P, "x\n")
            .file("other/x.rs", "x\n");
        let s = seen(&t);
        let r = context(&s, &paths(&[P]), CONTEXT_CAP).unwrap();
        // Edge case 46: both shown, nearest first.
        assert_eq!(ids(&r), [id(2), id(1)]);
        let lists = s.lists.borrow();
        assert!(lists.iter().all(|d| !d.is_empty() && !d.starts_with("other")), "{lists:?}");
        drop(lists);
        // No paths: nothing listed, nothing read.
        let s = seen(&t);
        assert_eq!(context(&s, &[], CONTEXT_CAP).unwrap(), ContextReport { records: Vec::new(), more: 0, more_broken_or_stale: 0 });
        assert!(s.lists.borrow().is_empty() && s.reads.borrow().is_empty());
    }

    #[test]
    fn record_bound_spans_the_folders_read() {
        // One pass's bound counts across every folder routing reads, in byte order of path.
        let mut t = MemTree::new();
        for (dir, n) in [("", 1), ("", 2), ("svc/", 3), ("svc/", 4)] {
            t.file(&format!("{dir}{R}{n}.md"), rec(&id(n), "fact", "t", &applies(&["**"])));
        }
        let r = context_within(&t, &paths(&["svc/x.rs"]), CONTEXT_CAP, 3).unwrap();
        assert_eq!(ids(&r), [id(3), id(1), id(2)]);
    }

    #[test]
    fn unreadable_keys_sort_last() {
        // A newer schema's record may lack a readable kind, title or id; each sorts after one that has it.
        let newer = |rest: &str| format!("---\nknowledge: 2\n{rest}applies_to: ['**']\n---\nb\n");
        let (a, b) = (id(1), id(2));
        let pairs = [
            (newer(&format!("id: '{b}'\nkind: 'question'\ntitle: 't'\n")), newer(&format!("id: '{a}'\nkind: 'policy'\ntitle: 't'\n"))),
            (newer(&format!("id: '{b}'\nkind: 'fact'\ntitle: 't'\n")), newer(&format!("id: '{a}'\nkind: 'fact'\n"))),
            (newer(&format!("id: '{b}'\nkind: 'fact'\ntitle: 't'\n")), newer("kind: 'fact'\ntitle: 't'\n")),
        ];
        for (first, second) in pairs {
            let mut t = MemTree::new();
            t.file(&format!("{R}b.md"), &first).file(&format!("{R}a.md"), &second);
            let r = context(&t, &paths(&["x.rs"]), CONTEXT_CAP).unwrap();
            assert_eq!(r.records.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), [format!("{R}b.md"), format!("{R}a.md")], "{first}");
        }
    }

    #[test]
    fn only_routed_records_are_evaluated() {
        let mut t = MemTree::new();
        let elsewhere = "checks:\n  - config_key: 'docs/conf.yaml#x'\n    must: 'exist'\n  - code: 'docs/lib.rs'\n    text: \
                         'retry(3, backoff::exponential());'\n    must: 'exist'\n";
        t.file(&format!("{R}a.md"), rec(&id(1), "fact", "routed", &format!("{}{}", applies(&["src/**"]), check("must", "src/a.rs"))))
            .file(&format!("{R}b.md"), rec(&id(2), "fact", "elsewhere", elsewhere))
            .file("src/a.rs", "x\n")
            .file("docs/conf.yaml", "x: 1\n")
            .file("docs/lib.rs", "retry(3, backoff::exponential());\n");
        let s = seen(&t);
        let r = context(&s, &paths(&["src/a.rs"]), CONTEXT_CAP).unwrap();
        assert_eq!(ids(&r), [id(1)]);
        assert_eq!(r.records[0].state, RecordState::Holds);
        // The folder's records once, and no file the unrouted record anchors.
        assert_eq!(*s.reads.borrow(), vec![2]);
    }

    /// A record and the scope prefix of its folder (`""` or `"svc/"`).
    type Side<'a> = (&'a str, String);

    /// The order of two records written in reverse path order (`first` at `b.md`, `second` at `a.md`).
    fn order_of(first: Side, second: Side, path: &str) -> Vec<String> {
        let mut t = MemTree::new();
        t.file(path, "x\n").file(&format!("{}{R}b.md", first.0), first.1).file(&format!("{}{R}a.md", second.0), second.1);
        ids(&context(&t, &paths(&[path]), CONTEXT_CAP).unwrap())
    }

    #[test]
    fn ordering_keys_in_turn() {
        // Each pair: the first wins on one key while every later key favours the second.
        let (hi, lo) = (id(9), id(1));
        let best = |glob: &str| format!("{}{}", applies(&[glob]), check("must", P));
        let svc_best = format!("{}{}", applies(&["src/a/main.rs"]), check("must", "src/a/main.rs"));
        let cases: Vec<(&str, Side, Side)> = vec![
            ("broken", ("", rec(&hi, "question", "z", &format!("{}{}", applies(&["**"]), check("must", "gone")))), ("svc/", rec(&lo, "decision", "a", &svc_best))),
            ("stale", ("", rec(&hi, "question", "z", &format!("{}{}", applies(&["**"]), check("assumes", "gone")))), ("svc/", rec(&lo, "decision", "a", &svc_best))),
            ("nearer folder", ("svc/", rec(&hi, "question", "z", &applies(&["**"]))), ("", rec(&lo, "decision", "a", &best(P)))),
            ("fewer **", ("", rec(&hi, "question", "z", &applies(&["svc/*/a/main.rs"]))), ("", rec(&lo, "decision", "a", &best("svc/src/a/**")))),
            ("longer prefix", ("", rec(&hi, "question", "z", &applies(&["svc/src/**"]))), ("", rec(&lo, "decision", "a", &best("svc/**")))),
            ("best of its globs", ("", rec(&hi, "question", "z", &applies(&["**", P]))), ("", rec(&lo, "decision", "a", &best("svc/src/**")))),
            ("must", ("", rec(&hi, "question", "z", &best("**"))), ("", rec(&lo, "decision", "a", &format!("{}{}", applies(&["**"]), check("assumes", P))))),
            ("decision", ("", rec(&hi, "decision", "z", &applies(&["**"]))), ("", rec(&lo, "procedure", "a", &applies(&["**"])))),
            ("procedure", ("", rec(&hi, "procedure", "z", &applies(&["**"]))), ("", rec(&lo, "fact", "a", &applies(&["**"])))),
            ("fact", ("", rec(&hi, "fact", "z", &applies(&["**"]))), ("", rec(&lo, "question", "a", &applies(&["**"])))),
            ("title bytes", ("", rec(&hi, "fact", "Zeta", &applies(&["**"]))), ("", rec(&lo, "fact", "alpha", &applies(&["**"])))),
            ("id", ("", rec(&lo, "fact", "t", &applies(&["**"]))), ("", rec(&hi, "fact", "t", &applies(&["**"])))),
        ];
        for (key, first, second) in cases {
            let want = [first.1.split('\'').nth(1).unwrap_or_default().to_owned(), second.1.split('\'').nth(1).unwrap_or_default().to_owned()];
            assert_eq!(order_of(first, second, P), want, "{key}");
        }
        // Broken and stale are one group: the later keys order them.
        let stale_near = ("svc/", rec(&hi, "question", "z", &format!("{}{}", applies(&["**"]), check("assumes", "gone"))));
        let broken_far = ("", rec(&lo, "decision", "a", &format!("{}{}", applies(&["**"]), check("must", "gone"))));
        assert_eq!(order_of(stale_near, broken_far, P), [hi.clone(), lo.clone()]);
        // Nearer is deeper in segments, not longer in bytes.
        let mut t = MemTree::new();
        t.file(&format!("a/b/c/{R}x.md"), rec(&hi, "fact", "t", &applies(&["**"])))
            .file(&format!("services/{R}x.md"), rec(&lo, "fact", "t", &applies(&["**"])));
        assert_eq!(ids(&context(&t, &paths(&["a/b/c/x.rs", "services/y.rs"]), CONTEXT_CAP).unwrap()), [hi.clone(), lo.clone()]);
        // Records alike in every key, the same id included, are in path order.
        let mut t = MemTree::new();
        t.file(&format!("{R}b.md"), rec(&lo, "fact", "t", &applies(&["**"]))).file(&format!("{R}a.md"), rec(&lo, "fact", "t", &applies(&["**"])));
        let r = context(&t, &paths(&["x.rs"]), CONTEXT_CAP).unwrap();
        assert_eq!(r.records.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), [format!("{R}a.md"), format!("{R}b.md")]);
        // Across folders of one depth too, where scope order and path order differ (`a` < `a-b`, `a-b/` < `a/`);
        // the globs `a/bcd/**` and `a-b/c/**` tie on every key.
        let mut t = MemTree::new();
        t.file(&format!("a/{R}x.md"), rec(&lo, "fact", "t", &applies(&["bcd/**"])))
            .file(&format!("a-b/{R}x.md"), rec(&lo, "fact", "t", &applies(&["c/**"])));
        let r = context(&t, &paths(&["a/bcd/x.rs", "a-b/c/x.rs"]), CONTEXT_CAP).unwrap();
        assert_eq!(r.records.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), [format!("a-b/{R}x.md"), format!("a/{R}x.md")]);
    }

    #[test]
    fn cap_and_overflow_counts() {
        let mut t = MemTree::new();
        t.file("src/a.rs", "x\n");
        for n in 0..25 {
            let c = match n {
                0..13 => check("must", "gone"),
                13..23 => check("assumes", "gone"),
                _ => check("must", "src/a.rs"),
            };
            t.file(&format!("{R}{n:02}.md"), rec(&id(n), "fact", "t", &format!("{}{c}", applies(&["src/**"]))));
        }
        let p = paths(&["src/a.rs"]);
        let r = context(&t, &p, 20).unwrap();
        assert_eq!((r.records.len(), r.more, r.more_broken_or_stale), (20, 5, 3));
        let all = context(&t, &p, usize::MAX).unwrap();
        assert_eq!((all.records.len(), all.more, all.more_broken_or_stale), (25, 0, 0));
        assert_eq!(r.records, all.records[..20]);
        let none = context(&t, &p, 0).unwrap();
        assert_eq!((none.records.len(), none.more, none.more_broken_or_stale), (0, 25, 23));
    }

    #[test]
    fn superseded_and_invalid_never_route() {
        let mut t = MemTree::new();
        let wide = applies(&["**"]);
        let newer = |n: usize, title: &str, rest: &str| rec(&id(n), "fact", title, rest).replace("knowledge: 1", "knowledge: 2");
        t.file("src/a.rs", "x\n")
            .file(&format!("{R}a.md"), rec(&id(1), "fact", "old", &wide))
            .file(&format!("{R}b.md"), rec(&id(2), "fact", "new", &format!("{}supersedes: '{}'\n", applies(&["docs/**"]), id(1))))
            .file(&format!("{R}c.md"), "---\nknowledge: 1\nid: 'bad'\n---\nx\n")
            .file(&format!("{R}d.md"), newer(4, "newer", &wide))
            .file(&format!("{R}e.md"), newer(5, "newer without applies_to", ""))
            .file(&format!("{R}f.md"), newer(6, "newer, superseded", &wide))
            .file(&format!("{R}g.md"), rec(&id(7), "fact", "supersedes the newer", &format!("{}supersedes: '{}'\n", applies(&["docs/**"]), id(6))));
        let r = context(&t, &paths(&["src/a.rs"]), CONTEXT_CAP).unwrap();
        // A newer record routes by the applies_to it writes; it is shown and never judged.
        assert_eq!(ids(&r), [id(4)]);
        assert_eq!(r.records[0].state, RecordState::Unverifiable);
        assert_eq!((r.more, r.more_broken_or_stale), (0, 0));
    }

    #[test]
    fn routing_work_follows_the_paths() {
        // Edge case 80 at scale: a record's globs are matched only against the paths that start with their literal
        // part, so the matches grow with records plus paths, never with their product.
        let mut t = MemTree::new();
        let ps: Vec<String> = (0..1_000).map(|n| format!("d{n}/f.rs")).collect();
        for p in &ps {
            t.file(p, "x\n");
        }
        for n in (0..1_000).step_by(2) {
            t.file(&format!("d{n}/{R}r.md"), rec(&id(n), "fact", "t", &applies(&["**"])));
        }
        crate::path::MATCHES.with(|m| m.set(0));
        let r = context(&t, &ps, CONTEXT_CAP).unwrap();
        let work = crate::path::MATCHES.with(std::cell::Cell::get);
        assert_eq!((r.records.len(), r.more), (20, 480));
        assert!(work <= 1_500, "{work} matches for 500 records and 1,000 paths");
    }

    #[test]
    fn many_paths_read_once() {
        // Edge case 80: 5,000 paths; the folder's front matter read once, each anchored file once, then the cap.
        let mut t = MemTree::new();
        let ps: Vec<String> = (0..5_000).map(|n| format!("src/m{}/f{n}.rs", n % 50)).collect();
        for p in &ps {
            t.file(p, "x\n");
        }
        t.file("conf.yaml", "x: 1\n");
        for n in 0..30 {
            let rest = format!("{}checks:\n  - config_key: 'conf.yaml#x'\n    must: '== 1'\n", applies(&[&format!("src/m{n}/**")]));
            t.file(&format!("{R}{n:02}.md"), rec(&id(n), "fact", "t", &rest));
        }
        let s = seen(&t);
        let r = context(&s, &ps, CONTEXT_CAP).unwrap();
        assert_eq!((r.records.len(), r.more, r.more_broken_or_stale), (20, 10, 0));
        assert_eq!(*s.reads.borrow(), vec![30, 1]);
    }
}
