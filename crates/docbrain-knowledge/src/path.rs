// SPDX-License-Identifier: MIT
//! Paths and the glob dialect (`SEMANTICS.md` §Paths and globs).
//!
//! A path in a record is relative to its folder's parent directory (the scope) and never leaves it. Globs:
//! `*` is any run of characters except `/`; `?` is one character except `/`; `**` as a whole segment is zero
//! or more segments (one or more when it is the last segment, since a path names an entry, not a
//! directory); `**` inside a segment is two `*`. No classes, braces or escapes.

use crate::reason::Invalid;

/// Why a written path is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    /// The empty string.
    Empty,
    /// A leading `/`, `~` or drive letter, a `\`, a `..`, `.git` or empty segment (`SEMANTICS.md` §Paths and globs).
    Confinement,
}

impl PathError {
    /// The record's `invalid` reason for this error.
    pub fn invalid(self) -> Invalid {
        match self {
            PathError::Empty => Invalid::Parse,
            PathError::Confinement => Invalid::Confinement,
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Pattern matches run on this thread: tests bound the work of routing and of the gate by it.
    pub(crate) static MATCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A path or glob written in a record, joined with its folder's scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoGlob {
    /// As written, relative to the scope, with its `.` segments dropped.
    pub rel: String,
    /// The scope joined with `rel`: a path or glob over the whole tree.
    pub root: String,
    literal: bool,
}

/// Parses a path written in a record of the folder whose parent directory is `scope` (`""` at the root).
pub fn parse_rel(scope: &str, s: &str) -> Result<RepoGlob, PathError> {
    if s.is_empty() {
        return Err(PathError::Empty);
    }
    let b = s.as_bytes();
    let drive = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    if s.starts_with('/') || s.starts_with('~') || drive || s.contains('\\') {
        return Err(PathError::Confinement);
    }
    // A `.` segment names the directory it is in: dropped, so `./a` and `a/./b` are `a` and `a/b`.
    let segs: Vec<&str> = s.split('/').filter(|seg| *seg != ".").collect();
    if segs.is_empty() {
        return Err(PathError::Empty);
    }
    if segs.iter().any(|seg| seg.is_empty() || *seg == ".." || *seg == ".git") {
        return Err(PathError::Confinement);
    }
    let rel = segs.join("/");
    let root = if scope.is_empty() { rel.clone() } else { format!("{scope}/{rel}") };
    let literal = !rel.contains(['*', '?']);
    Ok(RepoGlob { rel, root, literal })
}

impl RepoGlob {
    /// Whether the tree path `path` matches (exact equality for a literal).
    pub fn matches(&self, path: &str) -> bool {
        #[cfg(test)]
        MATCHES.with(|m| m.set(m.get() + 1));
        if self.literal { self.root == path } else { glob_match(&self.root, path) }
    }

    /// No `*` or `?` anywhere.
    pub fn is_literal(&self) -> bool {
        self.literal
    }

    /// The directory to list for this pattern: the whole segments of `root` before the first segment
    /// with a wildcard, the last segment excluded (`""` is the tree root).
    pub fn literal_dir(&self) -> &str {
        let mut end = 0;
        let mut pos = 0;
        for seg in self.root.split('/') {
            let next = pos + seg.len();
            if next == self.root.len() || seg.contains(['*', '?']) {
                break;
            }
            end = next;
            pos = next + 1;
        }
        &self.root[..end]
    }

    /// The number of `**` segments (routing: fewer is narrower, `SEMANTICS.md` §Routing).
    pub fn star_count(&self) -> usize {
        self.root.split('/').filter(|s| *s == "**").count()
    }

    /// Bytes of `root` before its first `*` or `?` (routing: longer is narrower, `SEMANTICS.md` §Routing).
    pub fn literal_prefix_len(&self) -> usize {
        self.root.find(['*', '?']).unwrap_or(self.root.len())
    }

    /// `root` before its first `*` or `?`: every path the pattern matches starts with it.
    pub fn literal_prefix(&self) -> &str {
        self.root.get(..self.literal_prefix_len()).unwrap_or_default()
    }
}

/// The paths of `sorted` (byte order) that start with `prefix`: in that order they are one run.
pub(crate) fn starting_with<'a>(sorted: &'a [String], prefix: &str) -> &'a [String] {
    let run = sorted.get(sorted.partition_point(|p| p.as_str() < prefix)..).unwrap_or_default();
    run.get(..run.partition_point(|p| p.starts_with(prefix))).unwrap_or_default()
}

/// Whether `path` matches `pattern` in the published glob dialect.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').collect();
    let segs: Vec<&str> = path.split('/').collect();
    // done[i][j]: pattern segments i.. match path segments j.. (computed back to front).
    let (n, m) = (pat.len(), segs.len());
    let mut next = vec![false; m + 1];
    next[m] = true; // no pattern left matches no path left
    for i in (0..n).rev() {
        let mut cur = vec![false; m + 1];
        if pat[i] == "**" {
            let last = i + 1 == n;
            // Zero segments (not as the last segment), or one segment then `**` again.
            for j in (0..=m).rev() {
                let zero = !last && next[j];
                let more = j < m && (cur[j + 1] || (last && j + 1 == m));
                cur[j] = zero || more;
            }
        } else {
            for j in 0..m {
                cur[j] = next[j + 1] && segment_match(pat[i], segs[j]);
            }
        }
        next = cur;
    }
    next[0]
}

/// `*` and `?` inside one segment; `?` is one Unicode scalar value.
fn segment_match(pat: &str, seg: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let s: Vec<char> = seg.chars().collect();
    let (mut pi, mut si) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, si));
            pi += 1;
        } else if let Some((sp, ss)) = star {
            pi = sp + 1;
            si = ss + 1;
            star = Some((sp, ss + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_trailing_star_matches_nothing_more() {
        for (pat, seg) in [("a*", "a"), ("?*", "a"), ("a**", "a"), ("values*", "values")] {
            assert!(segment_match(pat, seg), "{pat} {seg}");
        }
        assert!(glob_match("cfg/values*.yaml", "cfg/values.yaml"));
    }

    #[test]
    fn star_stops_at_slash() {
        assert!(glob_match("a/*.tf", "a/x.tf"));
        assert!(!glob_match("a/*.tf", "a/b/x.tf"));
    }

    #[test]
    fn double_star_is_whole_segment() {
        assert!(glob_match("docs/**", "docs/a/b.md"));
        assert!(!glob_match("docs/**", "docs"));
        assert!(glob_match("a/**/x", "a/x"));
        assert!(glob_match("**/x.yaml", "x.yaml"));
        assert!(glob_match("a**/x", "ab/x"));
        assert!(!glob_match("a**/x", "a/b/x")); // `**` inside a segment is two `*`
    }

    #[test]
    fn question_mark_is_one_char_never_slash() {
        assert!(glob_match("a?c", "abc"));
        assert!(glob_match("a?c", "aéc"));
        assert!(!glob_match("a?c", "a/c"));
        assert!(!glob_match("a?c", "ac"));
    }

    #[test]
    fn brackets_braces_are_literal() {
        assert!(glob_match("app/[slug]/page.tsx", "app/[slug]/page.tsx"));
        assert!(!glob_match("app/[slug]/page.tsx", "app/s/page.tsx"));
        assert!(glob_match("{a,b}.md", "{a,b}.md"));
        assert!(!glob_match("{a,b}.md", "a.md"));
    }

    #[test]
    fn dot_names_match_like_others() {
        assert!(glob_match("*", ".env"));
        assert!(glob_match("**", ".github/workflows/ci.yml"));
    }

    #[test]
    fn case_and_bytes_exact() {
        assert!(!glob_match("A.yaml", "a.yaml"));
    }

    #[test]
    fn many_double_stars_stay_polynomial() {
        let pat = "**/**/**/**/**/**/**/**/**/**/z";
        let path = (0..40).map(|_| "a").collect::<Vec<_>>().join("/");
        assert!(!glob_match(pat, &path));
        assert!(glob_match(pat, &format!("{path}/z")));
        let stars = "*a*a*a*a*a*a*a*a*a*a*b";
        assert!(!glob_match(stars, &"a".repeat(200)));
    }

    #[test]
    fn confinement_rejects() {
        for s in ["/x", "a/../b", "..", "a\\b", "C:/x", "c:x", "~/x", "~", ".git/config", "a/.git/x", "a//b", "a/"] {
            assert_eq!(parse_rel("", s).unwrap_err(), PathError::Confinement, "{s}");
        }
        assert_eq!(parse_rel("", "").unwrap_err(), PathError::Empty);
        assert_eq!(PathError::Confinement.invalid(), crate::Invalid::Confinement);
        assert_eq!(PathError::Empty.invalid(), crate::Invalid::Parse);
        for s in ["a/.gitignore", ".github/x", "a..b/c", "a/b~", "C/x"] {
            assert!(parse_rel("", s).is_ok(), "{s}");
        }
    }

    #[test]
    fn dot_segments_are_dropped() {
        for (s, rel) in [("./a", "a"), ("a/./b", "a/b"), ("./././a/.", "a"), ("./src/**", "src/**"), ("*/./x", "*/x"), ("...", "...")] {
            let g = parse_rel("svc", s).unwrap();
            assert_eq!((g.rel.as_str(), g.root.clone()), (rel, format!("svc/{rel}")), "{s}");
        }
        assert!(parse_rel("", "./src/**").unwrap().matches("src/a.rs"));
        assert!(parse_rel("", "./*.rs").unwrap().literal_prefix().is_empty());
        for s in [".", "./.", "./"] {
            assert!(parse_rel("", s).is_err(), "{s}");
        }
        assert_eq!(parse_rel("", "./.").unwrap_err(), PathError::Empty);
        for s in ["./../x", "./.git/x", "a/.//b"] {
            assert_eq!(parse_rel("", s).unwrap_err(), PathError::Confinement, "{s}");
        }
    }

    #[test]
    fn scope_joins_root() {
        assert_eq!(parse_rel("charts", "api/values.yaml").unwrap().root, "charts/api/values.yaml");
        assert_eq!(parse_rel("", "api/values.yaml").unwrap().root, "api/values.yaml");
        let g = parse_rel("charts", "api/*.yaml").unwrap();
        assert_eq!(g.rel, "api/*.yaml");
        assert!(g.matches("charts/api/values.yaml"));
        assert!(!g.matches("api/values.yaml"));
    }

    #[test]
    fn literal_and_its_directory() {
        let l = parse_rel("svc", "docs/a.md").unwrap();
        assert!(l.is_literal());
        assert_eq!(l.literal_dir(), "svc/docs");
        let g = parse_rel("svc", "infra/prod/*.tf").unwrap();
        assert!(!g.is_literal());
        assert_eq!(g.literal_dir(), "svc/infra/prod");
        assert_eq!(parse_rel("", "**/x.yaml").unwrap().literal_dir(), "");
        assert_eq!(parse_rel("", "x.yaml").unwrap().literal_dir(), "");
        assert_eq!(parse_rel("", "a/b?/c").unwrap().literal_dir(), "a");
    }

    #[test]
    fn narrowness_measures() {
        assert_eq!(parse_rel("", "a/**/b/**").unwrap().star_count(), 2);
        assert_eq!(parse_rel("", "a/*.tf").unwrap().star_count(), 0);
        assert_eq!(parse_rel("s", "a/b*.tf").unwrap().literal_prefix_len(), "s/a/b".len());
        assert_eq!(parse_rel("", "a/b.tf").unwrap().literal_prefix_len(), "a/b.tf".len());
    }

    fn seg() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9._\\[\\]{}~-]{1,5}".prop_filter("not .. or .git", |s| s != ".." && s != ".git")
    }

    fn path() -> impl Strategy<Value = String> {
        prop::collection::vec(seg(), 1..5).prop_map(|v| v.join("/"))
    }

    proptest! {
        #[test]
        fn glob_props(p in path(), q in path()) {
            prop_assert!(glob_match(&p, &p));
            prop_assert!(glob_match("**", &p));
            prop_assert_eq!(glob_match(&p, &q), p == q);
        }

        #[test]
        fn every_match_starts_with_the_literal_prefix(scope in scope(), rel in pattern(), paths in paths()) {
            // Routing tries a glob only on the run of sorted paths that start with its literal prefix (edge case 80).
            let Ok(g) = parse_rel(&scope, &rel) else { return Ok(()) };
            let mut sorted = paths;
            sorted.sort();
            sorted.dedup();
            let run = starting_with(&sorted, g.literal_prefix());
            for p in &sorted {
                prop_assert!(!g.matches(p) || run.contains(p), "{} matches {} outside the run", g.root, p);
            }
            prop_assert!(run.iter().all(|p| p.starts_with(g.literal_prefix())));
        }
    }

    fn scope() -> impl Strategy<Value = String> {
        prop::sample::select(vec!["", "a", "ab", "a/b"]).prop_map(str::to_owned)
    }
    fn pattern() -> impl Strategy<Value = String> {
        let seg = prop::sample::select(vec!["a", "b", "ab", "a*", "*", "?", "**", "*b", "b?"]);
        prop::collection::vec(seg, 1..4).prop_map(|s| s.join("/"))
    }
    fn paths() -> impl Strategy<Value = Vec<String>> {
        let seg = prop::sample::select(vec!["a", "b", "ab", "ba", "aa", "a*", "b.md"]);
        prop::collection::vec(prop::collection::vec(seg, 1..6).prop_map(|s| s.join("/")), 0..16)
    }
}
