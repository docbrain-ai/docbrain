// SPDX-License-Identifier: MIT
//! `check-claims`: which repository the change is in, and what to say about
//! the answer.
//!
//! The name is made HERE, on the runner, and only the canonical name is sent:
//! a GitLab CI checkout's `origin` is `https://gitlab-ci-token:<job token>@…`,
//! and a URL in a request body is a credential in a server log.
//!
//! Order, first that yields wins:
//! 1. `--repo <name or clone URL>`;
//! 2. GitHub Actions — `GITHUB_ACTIONS=true` and `GITHUB_REPOSITORY` — unless
//!    `GITEA_ACTIONS` or `FORGEJO_ACTIONS` is set: those runners export
//!    GitHub-compatible variables for repositories that are not on GitHub;
//! 3. GitLab CI — `GITLAB_CI=true`: `CI_MERGE_REQUEST_PROJECT_PATH` (the
//!    project the merge request targets; a fork MR's pipeline runs in the
//!    fork, where `CI_PROJECT_PATH` names the fork), else `CI_PROJECT_PATH`;
//! 4. the `origin` remote, read with `git remote get-url origin` — the same
//!    command ingest runs, so the two apply `url.*.insteadOf` alike;
//! 5. nothing: exit 3, nothing sent.
//!
//! An ssh/scp remote through an alias (`github-work:o/r.git`) is retried once
//! through `ssh -G`, which reads the user's own ssh config on the user's own
//! machine — the same thing `git fetch` does there.

use docbrain_evidence::repo::{name, Form, RepoName, Unnamed, GRAMMAR};

/// Where the name came from, printed so a CI log shows what was checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Flag,
    GitHubActions,
    GitLabCi,
    Remote,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Flag => "--repo",
            Origin::GitHubActions => "GitHub Actions",
            Origin::GitLabCi => "GitLab CI",
            Origin::Remote => "origin",
        }
    }
}

/// What the runner offers. Functions, not the process, so every arm is
/// testable without touching the real environment, git or ssh.
pub struct Runner<'a> {
    pub env: &'a dyn Fn(&str) -> Option<String>,
    /// `git remote get-url origin` in the working directory, or `None`.
    pub origin: &'a dyn Fn() -> Option<String>,
    /// `ssh -G -- <alias>`'s `hostname`, or `None`.
    pub ssh_hostname: &'a dyn Fn(&str) -> Option<String>,
}

/// Why no name could be made. `Display` never carries the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoName {
    /// An arm yielded a value that is not a repository name.
    Unnameable { from: Origin, why: Unnamed },
    /// No arm yielded anything.
    Nothing,
}

impl std::fmt::Display for NoName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoName::Unnameable { from, why } => write!(
                f,
                "uncheckable: the repository from {} is not a GitHub or GitLab repository name ({why}); expected {GRAMMAR}",
                from.label()
            ),
            NoName::Nothing => f.write_str(
                "uncheckable: cannot tell which repository this is (no --repo, no GitHub Actions or GitLab CI variables, no origin remote DocBrain can name); pass --repo github:owner/repo or --repo gitlab:group/project",
            ),
        }
    }
}

fn set(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Name `input`, retrying once through `ssh -G` when the host is an ssh alias.
fn name_via_alias(input: &str, form: Form, run: &Runner) -> Result<RepoName, Unnamed> {
    match name(input, &[], form) {
        Err(Unnamed::UnknownHost { alias: Some(alias) }) => {
            // An alias that looks like an option is never handed to ssh.
            if alias.starts_with('-') {
                return Err(Unnamed::UnknownHost { alias: None });
            }
            let Some(host) = (run.ssh_hostname)(&alias).filter(|h| !h.eq_ignore_ascii_case(&alias)) else {
                return Err(Unnamed::UnknownHost { alias: None });
            };
            let Some(path) = path_after_host(input) else {
                return Err(Unnamed::UnknownHost { alias: None });
            };
            name(&format!("ssh://git@{host}/{path}"), &[], form)
                .map_err(|e| match e {
                    // One retry only: never chase a second alias.
                    Unnamed::UnknownHost { .. } => Unnamed::UnknownHost { alias: None },
                    other => other,
                })
        }
        other => other,
    }
}

/// The path part of an ssh URL, an scp remote or a forge-prefixed alias form.
fn path_after_host(input: &str) -> Option<String> {
    let s = input.trim();
    let s = s.split(['?', '#']).next().unwrap_or(s);
    let path = match s.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?.1,
        None => s.split_once(':')?.1,
    };
    let path = path.trim_start_matches('/');
    (!path.is_empty()).then(|| path.to_string())
}

/// Derive the repository a change is in. See the module docs for the order.
pub fn derive(flag: Option<&str>, run: &Runner) -> Result<(RepoName, Origin), NoName> {
    let fail = |from: Origin| move |why: Unnamed| NoName::Unnameable { from, why };

    if let Some(v) = flag {
        return name_via_alias(v, Form::Typed, run).map(|n| (n, Origin::Flag)).map_err(fail(Origin::Flag));
    }

    let env = |k: &str| set((run.env)(k));
    let is_true = |k: &str| env(k).is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let foreign_actions = env("GITEA_ACTIONS").is_some() || env("FORGEJO_ACTIONS").is_some();
    if is_true("GITHUB_ACTIONS")
        && !foreign_actions
        && let Some(repo) = env("GITHUB_REPOSITORY")
    {
        return name(&format!("github:{repo}"), &[], Form::Typed)
            .map(|n| (n, Origin::GitHubActions))
            .map_err(fail(Origin::GitHubActions));
    }
    if is_true("GITLAB_CI")
        && let Some(path) = env("CI_MERGE_REQUEST_PROJECT_PATH").or_else(|| env("CI_PROJECT_PATH"))
    {
        return name(&format!("gitlab:{path}"), &[], Form::Typed)
            .map(|n| (n, Origin::GitLabCi))
            .map_err(fail(Origin::GitLabCi));
    }
    if let Some(url) = set((run.origin)()) {
        return name_via_alias(&url, Form::Remote, run).map(|n| (n, Origin::Remote)).map_err(fail(Origin::Remote));
    }
    Err(NoName::Nothing)
}

/// `git remote get-url origin` in the working directory.
pub fn git_origin() -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `ssh -G -- <alias>`'s `hostname` line.
pub fn ssh_hostname(alias: &str) -> Option<String> {
    let out = std::process::Command::new("ssh")
        .args(["-G", "--", alias])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("hostname ").map(|h| h.trim().to_string()))
}

// ---- what the answer says ----

/// The exit code for a parsed answer: 2 on findings unless
/// `--warn-only`; 3 when anything could not be told or the repository is not
/// one DocBrain reads; 0 otherwise. `--warn-only` never turns a 3 into a 0.
pub fn exit_code(findings: usize, could_not_tell: usize, repository_known: bool, paths_checked: u64, warn_only: bool) -> i32 {
    if paths_checked == 0 {
        return 0;
    }
    if findings > 0 && !warn_only {
        return 2;
    }
    if !repository_known || could_not_tell > 0 {
        return 3;
    }
    0
}

/// The scope suffix, for the four count-bearing lines only.
pub fn scope_suffix(scoped_to_spaces: Option<usize>) -> String {
    match scoped_to_spaces {
        None => String::new(),
        Some(1) => " — within the 1 space this key may see".to_string(),
        Some(n) => format!(" — within the {n} spaces this key may see"),
    }
}

/// The reasons in the order the CLI prints their groups.
pub const REASON_ORDER: [&str; 5] = [
    "own_repository_lacks_path",
    "shared_path",
    "no_repository_of_its_own",
    "not_in_listing",
    "repository_unknown",
];

/// One group's header line: what is recorded, then the one action.
pub fn group_line(reason: &str, n: usize, repository: &str, listed_at: &str) -> String {
    match reason {
        "own_repository_lacks_path" => format!(
            "{n} page(s) cite these paths but their own repository does not have them — fix or retire the page, or move the claim to {repository}:"
        ),
        "shared_path" => format!(
            "{n} page(s) live in a checkout DocBrain cannot name, on paths that {repository} also holds — give that checkout a remote DocBrain can read (github.com, gitlab.com or a configured host):"
        ),
        "no_repository_of_its_own" => format!(
            "{n} claim(s) come from captures or pages that carry no repository, on paths that {repository} and another repository hold; DocBrain cannot tell which one they mean — state the repository in the page or capture, or accept exit 3 for these:"
        ),
        "not_in_listing" => format!(
            "A listing of {repository} that DocBrain holds (the oldest is from {listed_at}) does not include these paths — re-run ingest; a path excluded by .docbrainignore, or on a branch other than the ingested one, cannot be checked:"
        ),
        "repository_unknown" => format!(
            "DocBrain reads no repository named {repository} — connect it, or check --repo; if this was a web URL, give the clone URL or the name:"
        ),
        // A reason this CLI does not know (a newer server): still said, never dropped.
        other => format!("{n} claim(s) could not be checked ({other}):"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn derive_env(flag: Option<&str>, vars: &[(&str, &str)], origin: Option<&str>) -> Result<(String, Origin), NoName> {
        let map: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let e = move |k: &str| map.get(k).cloned();
        let o = origin.map(str::to_string);
        let origin_fn = move || o.clone();
        let ssh = |a: &str| (a == "github-work").then(|| "github.com".to_string());
        let run = Runner { env: &e, origin: &origin_fn, ssh_hostname: &ssh };
        derive(flag, &run).map(|(n, o)| (n.to_string(), o))
    }

    const GH: [(&str, &str); 2] = [("GITHUB_ACTIONS", "true"), ("GITHUB_REPOSITORY", "Acme/Platform")];

    #[test]
    fn the_flag_wins_over_every_other_arm() {
        let vars = [GH[0], GH[1], ("GITLAB_CI", "true"), ("CI_PROJECT_PATH", "g/p")];
        assert_eq!(
            derive_env(Some("gitlab:x/y"), &vars, Some("git@github.com:o/r.git")).unwrap(),
            ("gitlab:x/y".into(), Origin::Flag)
        );
    }

    /// GitHub Actions before GitLab CI before origin.
    #[test]
    fn github_actions_then_gitlab_ci_then_origin() {
        let both = [GH[0], GH[1], ("GITLAB_CI", "true"), ("CI_PROJECT_PATH", "g/p")];
        assert_eq!(derive_env(None, &both, None).unwrap(), ("github:acme/platform".into(), Origin::GitHubActions));
        let gl = [("GITLAB_CI", "true"), ("CI_PROJECT_PATH", "g/p")];
        assert_eq!(derive_env(None, &gl, Some("git@github.com:o/r.git")).unwrap(), ("gitlab:g/p".into(), Origin::GitLabCi));
        assert_eq!(derive_env(None, &[], Some("git@github.com:o/r.git")).unwrap(), ("github:o/r".into(), Origin::Remote));
    }

    /// A Gitea or Forgejo runner exports GITHUB_* for a repository that is
    /// not on GitHub; the GitHub arm must yield.
    #[test]
    fn gitea_and_forgejo_markers_make_the_github_arm_yield() {
        for marker in ["GITEA_ACTIONS", "FORGEJO_ACTIONS"] {
            let vars = [GH[0], GH[1], (marker, "true")];
            assert_eq!(derive_env(None, &vars, Some("https://gitlab.com/g/p.git")).unwrap().1, Origin::Remote);
            assert_eq!(derive_env(None, &vars, None), Err(NoName::Nothing));
        }
    }

    /// A fork MR runs in the fork; the merge request's TARGET is what the
    /// merge would break.
    #[test]
    fn the_merge_request_target_wins_over_the_project_path() {
        let fork = [
            ("GITLAB_CI", "true"),
            ("CI_PROJECT_PATH", "forker/Twin"),
            ("CI_MERGE_REQUEST_PROJECT_PATH", "acme/Sub/Twin"),
        ];
        assert_eq!(derive_env(None, &fork, None).unwrap().0, "gitlab:acme/sub/twin");
        let branch = [("GITLAB_CI", "true"), ("CI_PROJECT_PATH", "forker/Twin")];
        assert_eq!(derive_env(None, &branch, None).unwrap().0, "gitlab:forker/twin");
    }

    #[test]
    fn ci_variables_that_are_not_true_or_are_empty_do_not_count() {
        let vars = [("GITHUB_ACTIONS", "1"), ("GITHUB_REPOSITORY", "o/r"), ("GITLAB_CI", "true"), ("CI_PROJECT_PATH", " ")];
        assert_eq!(derive_env(None, &vars, None), Err(NoName::Nothing));
    }

    #[test]
    fn an_ssh_alias_is_retried_once_and_an_option_shaped_alias_never_reaches_ssh() {
        assert_eq!(derive_env(None, &[], Some("github-work:Acme/X.git")).unwrap().0, "github:acme/x");
        assert_eq!(derive_env(Some("github-work:o/r.git"), &[], None).unwrap().0, "github:o/r");
        // An option-shaped alias never reaches ssh.
        let hit = std::cell::Cell::new(false);
        let ssh = |_: &str| {
            hit.set(true);
            Some("github.com".to_string())
        };
        let e = |_: &str| None;
        let o = || Some("-oProxyCommand=x:o/r.git".to_string());
        let run = Runner { env: &e, origin: &o, ssh_hostname: &ssh };
        assert!(derive(None, &run).is_err());
        assert!(!hit.get(), "an alias starting with '-' was handed to ssh");
    }

    #[test]
    fn an_unknown_https_host_is_refused_and_the_message_carries_none_of_it() {
        let err = derive_env(None, &[], Some("https://ci-token:S3CRET@git.unknown.example/g/p.git")).unwrap_err();
        let text = err.to_string();
        assert!(!text.contains("S3CRET") && !text.contains("unknown.example"), "{text}");
        assert!(text.starts_with("uncheckable:"));
    }

    #[test]
    fn exit_codes() {
        // findings → 2; --warn-only → 0 only when nothing is uncertain
        assert_eq!(exit_code(3, 0, true, 2, false), 2);
        assert_eq!(exit_code(3, 0, true, 2, true), 0);
        // --warn-only never downgrades a 3
        assert_eq!(exit_code(3, 4, true, 2, true), 3);
        assert_eq!(exit_code(3, 4, true, 2, false), 2);
        // could not tell is never 0
        assert_eq!(exit_code(0, 1, true, 1, false), 3);
        // an unknown repository is never 0
        assert_eq!(exit_code(0, 0, false, 1, false), 3);
        assert_eq!(exit_code(0, 0, false, 1, true), 3);
        assert_eq!(exit_code(0, 0, true, 1, false), 0);
        assert_eq!(exit_code(0, 0, false, 0, false), 0, "nothing removed, nothing to check");
    }

    #[test]
    fn the_suffix_counts_spaces() {
        assert_eq!(scope_suffix(None), "");
        assert_eq!(scope_suffix(Some(1)), " — within the 1 space this key may see");
        assert_eq!(scope_suffix(Some(3)), " — within the 3 spaces this key may see");
    }

    /// A claimant with no repository is never told to fix a checkout.
    #[test]
    fn each_reason_has_its_own_action() {
        let line = |r: &str| group_line(r, 1, "github:o/r", "2026-10-04");
        assert!(!line("no_repository_of_its_own").contains("checkout"));
        assert!(line("shared_path").contains("give that checkout a remote"));
        assert!(line("own_repository_lacks_path").contains("move the claim to github:o/r"));
        assert!(line("not_in_listing").contains("(the oldest is from 2026-10-04)"));
        assert!(line("repository_unknown").contains("connect it"));
        assert!(line("from_a_newer_server").contains("from_a_newer_server"));
        let all: std::collections::BTreeSet<String> = REASON_ORDER.iter().map(|r| line(r)).collect();
        assert_eq!(all.len(), REASON_ORDER.len(), "two reasons share a line");
    }

    #[test]
    fn path_after_host_reads_every_alias_form() {
        assert_eq!(path_after_host("github-work:o/r.git").as_deref(), Some("o/r.git"));
        assert_eq!(path_after_host("ssh://git@alias:22/g/p.git").as_deref(), Some("g/p.git"));
        assert_eq!(path_after_host("alias:/g/p?x#y").as_deref(), Some("g/p"));
        assert_eq!(path_after_host("alias:"), None);
    }
}
