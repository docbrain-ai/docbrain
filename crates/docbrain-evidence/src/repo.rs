// SPDX-License-Identifier: MIT
//! Repository names: one forge-qualified name for one repository, whoever asks.
//!
//! A pull-request check, an ingested checkout and the server must agree on
//! which repository a claim is about, so the name is a format, not a feature:
//! `github:owner/repo` or `gitlab:group/…/project` (GitLab nested groups kept
//! whole), lower-cased, host dropped. It is published here, under MIT, so any
//! other product can produce the same names from the same inputs.
//!
//! # Inputs
//!
//! A forge name (`github:o/r`, `gitlab:g/sub/p`) or a clone URL (https, ssh,
//! scp form, `git://`, `git+ssh://`). Web URLs are not names: a GitLab project
//! page and a clone URL without `.git` look alike, so `Form::Typed` input (text
//! a person typed into `--repo`) refuses the web-only shapes, while
//! `Form::Remote` input (a URL read from `git remote get-url`) is a clone URL
//! by construction and is named.
//!
//! # What an error may say
//!
//! Nothing of the input. A clone URL read in CI carries a job token
//! (`https://gitlab-ci-token:<token>@host/g/p.git`); the userinfo, query and
//! fragment are cut before anything else, and no `Unnamed` variant carries
//! input text except the ssh/scp host the CLI needs to resolve an ssh alias.
//! `Display` prints only the reason.
//!
//! Pure, std only, no subprocess: resolving an ssh alias is the CLI's job.
//! The rules are the numbered comments in [`name`], in order; an independent
//! reference implementation's answers over a corpus of inputs pin them in
//! `tests/repo_names.rs`.

use std::fmt;

/// The two forges a repository name can belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Forge {
    GitHub,
    GitLab,
}

impl Forge {
    /// The name prefix, without the colon.
    pub fn prefix(self) -> &'static str {
        match self {
            Forge::GitHub => "github",
            Forge::GitLab => "gitlab",
        }
    }
}

/// Where the input came from. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// Text a person supplied (`--repo`): web-only URL shapes are refused.
    Typed,
    /// A clone URL read from git (ingest; the CLI's `origin` arm).
    Remote,
}

/// A self-hosted forge the caller knows about (ingest passes its configured
/// GitHub web base and GitLab base URL; the CLI passes none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownHost {
    /// Host name; compared after the same fold as the input's host.
    pub host: String,
    pub forge: Forge,
    /// The forge's relative URL root (`/gitlab` in `https://host/gitlab`),
    /// slashes trimmed; empty for none. Stripped from http(s) URLs only —
    /// an ssh URL carries no relative root.
    pub base: String,
}

impl KnownHost {
    pub fn new(host: &str, forge: Forge, base: &str) -> Self {
        Self { host: host.to_string(), forge, base: base.trim_matches('/').to_string() }
    }

    /// From a configured base URL such as `https://code.example.net:8443/gl/`.
    /// `None` when the URL has no host this module could compare.
    pub fn from_base_url(url: &str, forge: Forge) -> Option<Self> {
        let rest = url.trim().split_once("://").map(|(_, r)| r)?;
        let (auth, path) = rest.split_once('/').unwrap_or((rest, ""));
        let auth = auth.rsplit_once('@').map(|(_, h)| h).unwrap_or(auth);
        let host = if auth.starts_with('[') {
            auth.split_once(']').map(|(h, _)| format!("{h}]"))?
        } else {
            auth.rsplit_once(':').map(|(h, _)| h).unwrap_or(auth).to_string()
        };
        if host.is_empty() {
            return None;
        }
        let path = path.split(['?', '#']).next().unwrap_or("");
        Some(Self::new(&host, forge, path))
    }
}

/// A canonical repository name. `Display` is the wire form, `github:o/r`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoName {
    pub forge: Forge,
    /// Lower-cased, `/`-separated, no leading or trailing slash.
    pub path: String,
}

impl fmt::Display for RepoName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.forge.prefix(), self.path)
    }
}

impl RepoName {
    /// Accept only the canonical wire form (what [`name`] prints): the
    /// server's grammar. `None` for anything else — a URL, upper case, a
    /// trailing slash, `local:` — so the server never has to guess.
    pub fn parse_canonical(s: &str) -> Option<Self> {
        let n = name(s, &[], Form::Typed).ok()?;
        (n.to_string() == s).then_some(n)
    }
}

/// Why an input is not a repository name. Carries no input text, except the
/// ssh/scp host that the CLI may resolve as an ssh alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unnamed {
    Empty,
    /// `local:<dir>` — a checkout with no nameable remote.
    Local,
    /// Not a URL form this module reads (a file path, an unknown scheme, a
    /// non-numeric port, a bracketed scp host).
    Unsupported,
    /// The host is neither a fixed forge host nor a known host. `alias` is
    /// set only for ssh, scp and forge-prefixed input (`github:o/r.git` is an
    /// scp remote through an alias named `github`), never for http(s).
    UnknownHost { alias: Option<String> },
    /// The path is not a repository path on that forge.
    BadPath,
}

impl fmt::Display for Unnamed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Unnamed::Empty => "no repository given",
            Unnamed::Local => "a local checkout has no forge name",
            Unnamed::Unsupported => "not a repository name or clone URL",
            Unnamed::UnknownHost { .. } => "host not recognised",
            Unnamed::BadPath => "not a repository path (web URLs are not accepted: give the clone URL or the name)",
        })
    }
}

impl std::error::Error for Unnamed {}

/// The grammar, for messages that must say what IS accepted.
pub const GRAMMAR: &str = "github:owner/repo or gitlab:group/project, or a clone URL";

const SCHEMES: [&str; 6] = ["https", "http", "ssh", "git", "git+ssh", "ssh+git"];

fn fixed_forge(host: &str) -> Option<Forge> {
    match host {
        "github.com" | "ssh.github.com" => Some(Forge::GitHub),
        "gitlab.com" | "altssh.gitlab.com" => Some(Forge::GitLab),
        _ => None,
    }
}

/// docs.gitlab.com/user/reserved_names, "Reserved project names" (retrieved
/// 2026-10-03): single-segment words only. The compound entries
/// (`environments/folders`, `gitlab-lfs/objects`, `info/lfs/objects`) are
/// paths, not words, and are not reserved singly. Applies to the LAST segment.
const RESERVED_PROJECT: [&str; 18] = [
    "-", "badges", "blame", "blob", "builds", "commits", "create", "create_dir", "edit", "files",
    "find_file", "new", "preview", "raw", "refs", "tree", "update", "wikis",
];

/// Same page: "You cannot create subgroups with the following names: `-`".
const RESERVED_SUBGROUP: [&str; 1] = ["-"];

/// Same page, "Reserved group names" (top-level groups). Refused as the first
/// segment of TYPED http(s) input only: such a URL is a web route, not a group.
const RESERVED_TOPLEVEL: [&str; 38] = [
    "-", ".well-known", "404.html", "422.html", "500.html", "502.html", "503.html", "admin", "api",
    "apple-touch-icon.png", "assets", "dashboard", "deploy.html", "explore", "favicon.ico",
    "favicon.png", "files", "groups", "health_check", "help", "import", "jwt", "login", "oauth",
    "profile", "projects", "public", "robots.txt", "s", "search", "sitemap", "sitemap.xml",
    "sitemap.xml.gz", "slash-command-logo.png", "snippets", "unsubscribes", "uploads", "users",
];

/// GitLab: "must not contain consecutive special characters".
const CONSECUTIVE_SPECIALS: [&str; 9] = ["--", "..", "__", ".-", "-.", "_.", "._", "-_", "_-"];

/// ASCII-lower-case and drop one trailing dot (`GitHub.com.` is github.com).
/// ASCII only, deliberately: a Unicode fold would let a homoglyph host match.
fn fold(host: &str) -> String {
    let h = host.to_ascii_lowercase();
    match h.strip_suffix('.') {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => h,
    }
}

fn segment_chars_ok(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn github_owner_ok(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=39).contains(&b.len())
        && b[0].is_ascii_alphanumeric()
        && b[b.len() - 1].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
}

fn ends_with_ci(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len()
        && s.is_char_boundary(s.len() - suffix.len())
        && s[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// Name a repository. See the module docs; the numbered comments are the rules in order.
pub fn name(input: &str, known_hosts: &[KnownHost], form: Form) -> Result<RepoName, Unnamed> {
    // 1. Trim; empty.
    let mut s = input.trim();
    if s.is_empty() {
        return Err(Unnamed::Empty);
    }
    // 2. Cut at the first `?` or `#` before anything else, so nothing after
    //    them (an access token in a query) can reach any output.
    if let Some(i) = s.find(['?', '#']) {
        s = &s[..i];
    }

    // 3. Forge-prefixed input.
    for forge in [Forge::GitHub, Forge::GitLab] {
        let prefix = forge.prefix();
        if s.len() > prefix.len()
            && s.as_bytes()[prefix.len()] == b':'
            && s[..prefix.len()].eq_ignore_ascii_case(prefix)
        {
            let rest = s[prefix.len() + 1..].trim_end_matches('/');
            if ends_with_ci(rest, ".git") {
                // `github:o/r.git` is an scp remote through an ssh alias named
                // `github`, not a name.
                return Err(Unnamed::UnknownHost { alias: Some(prefix.to_string()) });
            }
            return path_rules(forge, rest, "", false, false, form);
        }
    }
    if s.len() >= 6 && s.is_char_boundary(6) && s[..6].eq_ignore_ascii_case("local:") {
        return Err(Unnamed::Local);
    }

    // 4. URL forms.
    let (host, path, http) = if let Some((scheme, rest)) = s.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        if !SCHEMES.contains(&scheme.as_str()) {
            return Err(Unnamed::Unsupported);
        }
        let http = scheme == "http" || scheme == "https";
        let (auth, path) = rest.split_once('/').unwrap_or((rest, ""));
        // The LAST `@`: a password may itself contain `@`.
        let auth = auth.rsplit_once('@').map(|(_, h)| h).unwrap_or(auth);
        let (host, port) = if auth.starts_with('[') {
            let Some((inner, after)) = auth.split_once(']') else {
                return Err(Unnamed::Unsupported);
            };
            (format!("{inner}]"), after.trim_start_matches(':'))
        } else if let Some((h, p)) = auth.rsplit_once(':') {
            (h.to_string(), p)
        } else {
            (auth.to_string(), "")
        };
        if !port.is_empty() && !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Unnamed::Unsupported);
        }
        (host, path, http)
    } else {
        // scp form iff a `:` precedes any `/`.
        let colon = s.find(':');
        let slash = s.find('/');
        let Some(colon) = colon else { return Err(Unnamed::Unsupported) };
        if slash.is_some_and(|sl| sl < colon) {
            return Err(Unnamed::Unsupported);
        }
        let left = &s[..colon];
        let left = left.rsplit_once('@').map(|(_, h)| h).unwrap_or(left);
        if left.starts_with('[') {
            return Err(Unnamed::Unsupported);
        }
        (left.to_string(), &s[colon + 1..], false)
    };

    // 5. Forge from the host: fixed hosts, then the caller's known hosts.
    let host = fold(&host);
    let (forge, base) = match fixed_forge(&host) {
        Some(f) => (f, ""),
        None => match known_hosts.iter().find(|k| fold(&k.host) == host) {
            Some(k) => (k.forge, k.base.as_str()),
            None => {
                return Err(Unnamed::UnknownHost { alias: (!http).then_some(host) });
            }
        },
    };
    let had_git = ends_with_ci(path.trim_end_matches('/'), ".git");
    path_rules(forge, path, if http { base } else { "" }, http, had_git, form)
}

// 6–7. Path rules, then lower-case.
fn path_rules(
    forge: Forge,
    p: &str,
    base: &str,
    http: bool,
    had_git: bool,
    form: Form,
) -> Result<RepoName, Unnamed> {
    let mut p = p.trim_start_matches('/');
    let base = base.trim_matches('/');
    if !base.is_empty() {
        // Segment-wise and case-sensitive: GitLab's relative root is a path.
        if p == base {
            p = "";
        } else if let Some(rest) = p.strip_prefix(base).and_then(|r| r.strip_prefix('/')) {
            p = rest;
        } else {
            return Err(Unnamed::BadPath);
        }
    }
    let mut p = p.trim_end_matches('/');
    if ends_with_ci(p, ".git") {
        p = &p[..p.len() - 4];
    }
    let segs: Vec<&str> = p.split('/').collect();
    let last = segs[segs.len() - 1];
    if [".git", ".wiki", ".atom"].iter().any(|x| ends_with_ci(last, x)) {
        return Err(Unnamed::BadPath);
    }
    if segs.iter().any(|g| *g == "." || *g == ".." || !segment_chars_ok(g)) {
        return Err(Unnamed::BadPath);
    }
    match forge {
        Forge::GitHub => {
            if segs.len() != 2 || !github_owner_ok(segs[0]) || segs[1].len() > 100 {
                return Err(Unnamed::BadPath);
            }
        }
        Forge::GitLab => {
            if segs.len() < 2 {
                return Err(Unnamed::BadPath);
            }
            let alnum_ends = |g: &str| {
                let b = g.as_bytes();
                b[0].is_ascii_alphanumeric() && b[b.len() - 1].is_ascii_alphanumeric()
            };
            if !segs.iter().all(|g| alnum_ends(g)) {
                return Err(Unnamed::BadPath);
            }
            if segs.iter().any(|g| CONSECUTIVE_SPECIALS.iter().any(|c| g.contains(c))) {
                return Err(Unnamed::BadPath);
            }
            if RESERVED_PROJECT.contains(&last.to_ascii_lowercase().as_str()) {
                return Err(Unnamed::BadPath);
            }
            if segs[1..segs.len() - 1].iter().any(|g| RESERVED_SUBGROUP.contains(g)) {
                return Err(Unnamed::BadPath);
            }
            if form == Form::Typed && http {
                if RESERVED_TOPLEVEL.contains(&segs[0].to_ascii_lowercase().as_str()) {
                    return Err(Unnamed::BadPath);
                }
                if segs.len() >= 3 && !had_git {
                    return Err(Unnamed::BadPath);
                }
            }
        }
    }
    Ok(RepoName { forge, path: p.to_ascii_lowercase() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> String {
        match name(s, &[], Form::Typed) {
            Ok(r) => r.to_string(),
            Err(e) => format!("REFUSE:{e:?}"),
        }
    }

    #[test]
    fn canonical_wire_form_only() {
        assert!(RepoName::parse_canonical("github:o/r").is_some());
        assert!(RepoName::parse_canonical("gitlab:g/sub/p").is_some());
        for bad in ["GitHub:o/r", "github:o/r/", "https://github.com/o/r.git", "local:x", "", " github:o/r"] {
            assert!(RepoName::parse_canonical(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn display_never_prints_the_input() {
        for e in [
            Unnamed::Empty,
            Unnamed::Local,
            Unnamed::Unsupported,
            Unnamed::UnknownHost { alias: Some("secret-host".into()) },
            Unnamed::BadPath,
        ] {
            assert!(!e.to_string().contains("secret"), "{e}");
        }
    }

    #[test]
    fn known_host_from_base_url() {
        let k = KnownHost::from_base_url("https://user:tok@Code.Example.net:8443/gl/?x=1", Forge::GitLab).unwrap();
        assert_eq!(k, KnownHost::new("Code.Example.net", Forge::GitLab, "gl"));
        assert_eq!(KnownHost::from_base_url("https://github.example.com", Forge::GitHub).unwrap().base, "");
        assert!(KnownHost::from_base_url("not a url", Forge::GitHub).is_none());
    }

    #[test]
    fn a_few_rows_in_the_open() {
        assert_eq!(n("GitHub:Acme/Platform/"), "github:acme/platform");
        assert_eq!(n("git@GITHUB.COM:o/r.git"), "github:o/r");
        assert_eq!(n("https://gitlab.com/g/sub/p"), "REFUSE:BadPath");
        assert_eq!(name("https://gitlab.com/g/sub/p", &[], Form::Remote).unwrap().to_string(), "gitlab:g/sub/p");
    }
}
