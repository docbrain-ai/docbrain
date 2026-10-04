// SPDX-License-Identifier: MIT
//! `docbrain-cli check-claims` end to end, against a mock DocBrain server and
//! recorded forge data.
//!
//! DocBrain makes NO forge API call: a repository is named from what the
//! runner already has (`--repo`, the CI system's variables, the `origin`
//! remote). The forge fixtures under `fixtures/forge/` mirror the GitLab and
//! GitHub REST shapes (each file's `_source` names the API section), and are
//! used for what they carry — the names and URLs a CI job or a person would
//! hand the CLI, and the files a merge or pull request changes. Owner ruling:
//! "just use gitlab API and make sure to test with mock data, we don't need to
//! have gitlab instance"; GitHub is proven the same way here, and live by the
//! coordinator (`build-90/proof-steps.md`).
//!
//! The binary runs with a cleared environment, a scratch `HOME`, and
//! `DOCBRAIN_SERVER_URL` pointed at the mock, so nothing reaches a real stack.

use docbrain_evidence::repo::{name, Form, KnownHost, Forge, Unnamed};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

const BIN: &str = env!("CARGO_BIN_EXE_docbrain-cli");

fn fixture(rel: &str) -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/forge").join(rel);
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap()
}

// ---- a mock DocBrain ----

type Responder = dyn Fn(&Value) -> (u16, String) + Send + Sync;

struct Mock {
    url: String,
    /// Every request body POSTed to the check route.
    bodies: Arc<Mutex<Vec<String>>>,
}

fn mock(respond: impl Fn(&Value) -> (u16, String) + Send + Sync + 'static) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let seen = bodies.clone();
    let respond: Arc<Responder> = Arc::new(respond);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let head_end = loop {
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break None;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break Some(i + 4);
                }
            };
            let Some(head_end) = head_end else { continue };
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let len = head
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0);
            while buf.len() < head_end + len {
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
            let (status, out) = if head.starts_with("POST /api/v1/premises/check ") {
                seen.lock().unwrap().push(body.clone());
                respond(&serde_json::from_str(&body).unwrap_or(Value::Null))
            } else {
                (404, String::new())
            };
            let _ = write!(
                s,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                out.len()
            );
        }
    });
    Mock { url, bodies }
}

/// A RepositoryCheck-shaped answer echoing the request's repository.
#[derive(Clone, Default)]
struct Answer {
    known: bool,
    findings: Vec<&'static str>,
    could_not_tell: Vec<&'static str>,
    others: u64,
    scoped: Option<Vec<&'static str>>,
    echo: Option<&'static str>,
    omit_echo: bool,
}

fn claim(path: &str, by: &str) -> Value {
    json!({
        "premise_id": "00000000-0000-0000-0000-000000000001",
        "expression": path, "successor": null,
        "subject": {"kind": "document", "id": "00000000-0000-0000-0000-000000000002"},
        "claimed_by": by, "space": "OPS", "url": format!("https://wiki.example/{by}")
    })
}

fn answer(a: Answer) -> impl Fn(&Value) -> (u16, String) + Send + Sync {
    move |req: &Value| {
        let repo = req["repository"].as_str().unwrap_or("").to_string();
        let paths = req["diff"].as_str().unwrap_or("").lines().filter(|l| l.starts_with('D') || l.starts_with('R')).count();
        let mut v = json!({
            "repository": a.echo.map(str::to_string).unwrap_or(repo),
            "repository_known": a.known,
            "listed_at": if a.known { json!("2026-10-04T12:20:00Z") } else { Value::Null },
            "scoped_to_spaces": a.scoped,
            "paths_checked": paths,
            "findings": a.findings.iter().map(|p| claim(p, "notes/g.md")).collect::<Vec<_>>(),
            "not_checked": {
                "could_not_tell": a.could_not_tell.iter().map(|r| {
                    let mut c = claim("docs/shared.md", &format!("page-{r}"));
                    c["reason"] = json!(r);
                    c
                }).collect::<Vec<_>>(),
                "other_repositories": a.others
            }
        });
        if a.omit_echo {
            v.as_object_mut().unwrap().remove("repository");
        }
        (200, v.to_string())
    }
}

// ---- running the binary ----

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn scratch() -> (tempfile::TempDir, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    (t, home)
}

fn run(cwd: &Path, home: &Path, server: &str, env: &[(&str, &str)], args: &[&str], diff: &str) -> Out {
    let diff_file = home.join("change.diff");
    std::fs::write(&diff_file, diff).unwrap();
    let mut cmd = Command::new(BIN);
    cmd.current_dir(cwd)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", home)
        .env("DOCBRAIN_SERVER_URL", server)
        .args(["check-claims", "--diff", diff_file.to_str().unwrap()])
        .args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().unwrap();
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).to_string(),
        stderr: String::from_utf8_lossy(&o.stderr).to_string(),
    }
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("HOME", dir)
        .env_remove("GIT_DIR")
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn checkout_with_origin(parent: &Path, url: Option<&str>) -> PathBuf {
    let d = parent.join("work");
    std::fs::create_dir_all(&d).unwrap();
    git(&d, &["init", "-q"]);
    if let Some(u) = url {
        git(&d, &["remote", "add", "origin", u]);
    }
    d
}

fn env_of(v: &Value) -> Vec<(String, String)> {
    v.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect()
}

fn sent(m: &Mock) -> Vec<Value> {
    m.bodies.lock().unwrap().iter().map(|b| serde_json::from_str(b).unwrap()).collect()
}

const DIFF: &str = "D\tdocs/probe.md\nD\tdocs/shared.md\n";

// ---- GitLab: API-shaped data ----

/// Projects API: the two clone URLs and the path name one repository; the
/// web URL of a nested project is not a name when typed, and is when git
/// reports it as a remote; the URL-encoded `:id` form is refused, never
/// mis-named.
#[test]
fn gitlab_project_fixtures_name_one_repository() {
    let p = fixture("gitlab/project_nested.json");
    let want = format!("gitlab:{}", p["path_with_namespace"].as_str().unwrap().to_lowercase());
    assert_eq!(want, "gitlab:acme/sub/twin");
    for url in ["http_url_to_repo", "ssh_url_to_repo"] {
        for form in [Form::Typed, Form::Remote] {
            assert_eq!(name(p[url].as_str().unwrap(), &[], form).unwrap().to_string(), want, "{url}");
        }
    }
    assert_eq!(name(&format!("gitlab:{}", p["path_with_namespace"].as_str().unwrap()), &[], Form::Typed).unwrap().to_string(), want);
    assert_eq!(name(p["web_url"].as_str().unwrap(), &[], Form::Typed), Err(Unnamed::BadPath));
    let encoded = p["_request"].as_str().unwrap().rsplit('/').next().unwrap();
    assert_eq!(encoded, "acme%2FSub%2FTwin");
    assert_eq!(name(&format!("gitlab:{encoded}"), &[], Form::Typed), Err(Unnamed::BadPath));

    // A self-managed host under a relative root: named only when configured.
    let sh = fixture("gitlab/project_self_hosted.json");
    let kh = [KnownHost::from_base_url("https://code.example.net:8443/gl", Forge::GitLab).unwrap()];
    for url in ["http_url_to_repo", "ssh_url_to_repo"] {
        assert_eq!(name(sh[url].as_str().unwrap(), &kh, Form::Remote).unwrap().to_string(), "gitlab:grp/sub/proj");
        assert!(matches!(name(sh[url].as_str().unwrap(), &[], Form::Remote), Err(Unnamed::UnknownHost { .. })));
    }

    // A fork and its parent are two repositories.
    let f = fixture("gitlab/project_fork.json");
    let fork = name(f["http_url_to_repo"].as_str().unwrap(), &[], Form::Typed).unwrap().to_string();
    let parent = name(f["forked_from_project"]["http_url_to_repo"].as_str().unwrap(), &[], Form::Typed).unwrap().to_string();
    assert_eq!((fork.as_str(), parent.as_str()), ("gitlab:forker/twin", "gitlab:acme/sub/twin"));

    let d = fixture("gitlab/project_dotted_group.json");
    assert_eq!(name(d["ssh_url_to_repo"].as_str().unwrap(), &[], Form::Typed).unwrap().to_string(), "gitlab:my.group/sub/p");
}

/// MR diffs API → `--name-status` text, run under the recorded merge request
/// pipeline environments. Both the parent-target and the fork pipeline send
/// the TARGET's name; a branch pipeline in the fork sends the fork's.
#[test]
fn gitlab_merge_request_pipelines_send_the_target_repository() {
    let diffs = fixture("gitlab/mr_diffs.json");
    let mut diff = String::new();
    for d in diffs["_body"].as_array().unwrap() {
        let (old, new) = (d["old_path"].as_str().unwrap(), d["new_path"].as_str().unwrap());
        if d["deleted_file"] == true {
            diff.push_str(&format!("D\t{old}\n"));
        } else if d["renamed_file"] == true {
            diff.push_str(&format!("R100\t{old}\t{new}\n"));
        } else if d["new_file"] == true {
            diff.push_str(&format!("A\t{new}\n"));
        } else {
            diff.push_str(&format!("M\t{new}\n"));
        }
    }
    let envs = fixture("gitlab/mr_pipeline_env.json");
    for (which, want) in [
        ("parent_target", "gitlab:acme/sub/twin"),
        ("fork", "gitlab:acme/sub/twin"),
        ("fork_branch_pipeline", "gitlab:forker/twin"),
    ] {
        let m = mock(answer(Answer { known: true, ..Default::default() }));
        let (t, home) = scratch();
        let env = env_of(&envs[which]);
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let out = run(t.path(), &home, &m.url, &env, &[], &diff);
        let bodies = sent(&m);
        assert_eq!(bodies.len(), 1, "{which}: {}", out.stderr);
        assert_eq!(bodies[0]["repository"], want, "{which}");
        assert_eq!(bodies[0]["diff"], diff.as_str(), "{which}: the raw diff, renames intact");
        assert!(out.stderr.contains(&format!("repository: {want} (from GitLab CI)")), "{}", out.stderr);
        assert_eq!(out.code, 0, "{which}: {}{}", out.stdout, out.stderr);
        for s in [&bodies[0].to_string(), &out.stdout, &out.stderr] {
            assert!(!s.contains("FAKEJOBTOKEN"), "{which}: the job token leaked");
        }
    }
}

/// A project that does not exist and one the caller may not see both answer
/// 404 — and a renamed project's pipelines carry the NEW path. Either way the
/// name is one DocBrain does not read: exit 3, the claims listed, never 0.
#[test]
fn gitlab_absent_private_or_renamed_repository_is_exit_3_with_its_reason() {
    let nf = fixture("gitlab/project_not_found.json");
    assert_eq!(nf["_status"], 404);
    let m = mock(answer(Answer { known: false, could_not_tell: vec!["repository_unknown", "repository_unknown"], ..Default::default() }));
    let (t, home) = scratch();
    let out = run(t.path(), &home, &m.url, &[], &["--repo", "gitlab:acme/Sub/Gone"], DIFF);
    assert_eq!(out.code, 3, "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("DocBrain reads no repository named gitlab:acme/sub/gone — connect it, or check --repo"), "{}", out.stdout);
    assert!(out.stdout.contains("page-repository_unknown"), "every claim is named");
    assert!(!out.stdout.contains("within the"), "the unknown-repository line carries no scope suffix");
    assert!(out.stderr.contains("uncheckable"));
    // --warn-only cannot make it green.
    let out = run(t.path(), &home, &m.url, &[], &["--repo", "gitlab:acme/Sub/Gone", "--warn-only"], DIFF);
    assert_eq!(out.code, 3);
}

// ---- GitHub: API-shaped data ----

#[test]
fn github_repository_fixture_names_one_repository_from_every_url() {
    let r = fixture("github/repo.json");
    let want = format!("github:{}", r["full_name"].as_str().unwrap().to_lowercase());
    for url in ["clone_url", "ssh_url", "git_url", "html_url"] {
        assert_eq!(name(r[url].as_str().unwrap(), &[], Form::Typed).unwrap().to_string(), want, "{url}");
    }
    assert_eq!(name(&format!("github:{}", r["full_name"].as_str().unwrap()), &[], Form::Typed).unwrap().to_string(), want);
    // Case, a trailing slash, `.git`: one name.
    for v in ["GitHub:Octo-Org/Docs-Probe/", "https://GITHUB.com/octo-org/docs-probe.git/", "git@github.com:Octo-Org/Docs-Probe"] {
        assert_eq!(name(v, &[], Form::Typed).unwrap().to_string(), want, "{v}");
    }
}

/// A pull request in GitHub Actions: the PR files API → the diff; the name
/// comes from GITHUB_REPOSITORY (mixed case) and is sent lower-cased; the
/// findings fail the step, the could-not-tell groups are printed with their
/// actions, and the other-repositories line prints on exit 2.
#[test]
fn github_actions_pull_request_end_to_end() {
    let files = fixture("github/pr_files.json");
    let mut diff = String::new();
    for f in files["_body"].as_array().unwrap() {
        let name = f["filename"].as_str().unwrap();
        match f["status"].as_str().unwrap() {
            "removed" => diff.push_str(&format!("D\t{name}\n")),
            "renamed" => diff.push_str(&format!("R100\t{}\t{name}\n", f["previous_filename"].as_str().unwrap())),
            _ => diff.push_str(&format!("M\t{name}\n")),
        }
    }
    let env = env_of(&fixture("github/actions_env.json")["pull_request"]);
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let m = mock(answer(Answer {
        known: true,
        findings: vec!["docs/probe.md", "docs/probe.md", "docs/shared.md"],
        could_not_tell: vec!["own_repository_lacks_path", "own_repository_lacks_path", "shared_path", "no_repository_of_its_own"],
        others: 1,
        ..Default::default()
    }));
    let (t, home) = scratch();
    let out = run(t.path(), &home, &m.url, &env, &[], &diff);
    assert_eq!(sent(&m)[0]["repository"], "github:octo-org/docs-probe");
    assert_eq!(out.code, 2, "{}{}", out.stdout, out.stderr);
    let o = &out.stdout;
    assert!(o.starts_with("3 live documentation claim(s) about github:octo-org/docs-probe would be falsified by this change:"), "{o}");
    assert!(o.contains("DocBrain cannot tell whether 4 claim(s) on these paths are about github:octo-org/docs-probe:"));
    assert!(o.contains("2 page(s) cite these paths but their own repository does not have them — fix or retire the page, or move the claim to github:octo-org/docs-probe:"));
    assert!(o.contains("1 page(s) live in a checkout DocBrain cannot name"));
    assert!(o.contains("1 claim(s) come from captures or pages that carry no repository"));
    assert!(o.contains("1 claim(s) about other repositories name these paths."));
    // (f) --warn-only with could-not-tell → 3, everything still printed.
    let out = run(t.path(), &home, &m.url, &env, &["--warn-only"], &diff);
    assert_eq!(out.code, 3);
    assert!(out.stdout.contains("3 live documentation claim(s)"));
    // (g) --json carries the lower-cased name.
    let out = run(t.path(), &home, &m.url, &env, &["--json"], &diff);
    let v: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["repository"], "github:octo-org/docs-probe");
    assert_eq!(out.code, 2);
}

/// Renamed (301), absent or private (404 for both), forbidden (403): the
/// name the runner holds is not one DocBrain reads → exit 3 with the reason.
#[test]
fn github_moved_absent_private_or_forbidden_is_exit_3_with_its_reason() {
    let moved = fixture("github/repo_moved.json");
    assert_eq!(moved["_status"], 301);
    // After a rename, GITHUB_REPOSITORY carries the new name; DocBrain read the old one.
    let new = moved["_followed"]["full_name"].as_str().unwrap();
    for (fixture_name, repo) in [
        ("github/repo_moved.json", format!("github:{new}")),
        ("github/repo_not_found.json", "github:octo-org/Docs-Private".to_string()),
        ("github/repo_forbidden.json", "github:octo-org/Docs-Blocked".to_string()),
    ] {
        let status = fixture(fixture_name)["_status"].as_i64().unwrap();
        assert!([301, 403, 404].contains(&status));
        let m = mock(answer(Answer { known: false, could_not_tell: vec!["repository_unknown"], ..Default::default() }));
        let (t, home) = scratch();
        let out = run(t.path(), &home, &m.url, &[], &["--repo", &repo], DIFF);
        assert_eq!(out.code, 3, "{fixture_name}");
        assert!(out.stdout.contains("DocBrain reads no repository named"), "{fixture_name}: {}", out.stdout);
    }
}

#[test]
fn gitea_or_forgejo_actions_never_take_the_github_arm() {
    for marker in ["GITEA_ACTIONS", "FORGEJO_ACTIONS"] {
        let m = mock(answer(Answer { known: true, ..Default::default() }));
        let (t, home) = scratch();
        let env = [("GITHUB_ACTIONS", "true"), ("GITHUB_REPOSITORY", "octo-org/Docs-Probe"), (marker, "true")];
        let out = run(t.path(), &home, &m.url, &env, &[], DIFF);
        assert_eq!(out.code, 3, "{marker}");
        assert!(sent(&m).is_empty(), "{marker}: nothing may be sent without a name");
    }
}

// ---- the origin arm, tokens, and no name at all ----

/// An `origin` carrying a token, in three forms: the name is made on the
/// runner and the token reaches no request, stdout or stderr.
#[test]
fn an_origin_token_never_leaves_the_runner() {
    for url in [
        "https://gitlab-ci-token:FAKEJOBTOKEN0123@gitlab.com/acme/Sub/Twin.git",
        "https://gitlab.com/acme/Sub/Twin.git?private_token=FAKEJOBTOKEN0123",
        "https://user:p@ssFAKEJOBTOKEN0123@gitlab.com/acme/Sub/Twin.git",
    ] {
        let m = mock(answer(Answer { known: true, ..Default::default() }));
        let (t, home) = scratch();
        let work = checkout_with_origin(t.path(), Some(url));
        let out = run(&work, &home, &m.url, &[], &[], DIFF);
        let bodies = m.bodies.lock().unwrap().clone();
        assert_eq!(bodies.len(), 1, "{}", out.stderr);
        assert!(bodies[0].contains("\"gitlab:acme/sub/twin\""));
        for s in [&bodies[0], &out.stdout, &out.stderr] {
            assert!(!s.contains("FAKEJOBTOKEN"), "leaked: {s}");
        }
        assert!(out.stderr.contains("(from origin)"));
    }
    // A self-hosted host the CLI does not know: exit 3, nothing sent, no host printed.
    let m = mock(answer(Answer::default()));
    let (t, home) = scratch();
    let work = checkout_with_origin(t.path(), Some("https://gitlab-ci-token:FAKEJOBTOKEN0123@code.example.net/gl/g/p.git"));
    let out = run(&work, &home, &m.url, &[], &[], DIFF);
    assert_eq!(out.code, 3);
    assert!(sent(&m).is_empty());
    assert!(!out.stderr.contains("FAKEJOBTOKEN") && !out.stderr.contains("code.example.net"), "{}", out.stderr);
}

#[test]
fn no_name_anywhere_is_exit_3_and_nothing_is_sent() {
    let m = mock(answer(Answer::default()));
    let (t, home) = scratch();
    let out = run(t.path(), &home, &m.url, &[], &[], DIFF);
    assert_eq!(out.code, 3);
    assert!(out.stderr.contains("cannot tell which repository this is"), "{}", out.stderr);
    assert!(sent(&m).is_empty());
    let work = checkout_with_origin(t.path(), None);
    let out = run(&work, &home, &m.url, &[], &[], DIFF);
    assert_eq!(out.code, 3);
    assert!(sent(&m).is_empty());
    // A typed web URL: refused, with the remedy, and nothing sent.
    let out = run(t.path(), &home, &m.url, &[], &["--repo", "https://gitlab.com/acme/Sub/Twin/-/merge_requests/3"], DIFF);
    assert_eq!(out.code, 3);
    assert!(out.stderr.contains("give the clone URL or the name"), "{}", out.stderr);
    assert!(sent(&m).is_empty());
}

// ---- what the answer says ----

/// An answer without `repository`, or naming another, is an old or
/// unscoped server: exit 3, never its findings.
#[test]
fn the_echo_check_refuses_an_unscoped_answer() {
    for a in [
        Answer { known: true, findings: vec!["docs/shared.md"], omit_echo: true, ..Default::default() },
        Answer { known: true, findings: vec!["docs/shared.md"], echo: Some("github:other/repo"), ..Default::default() },
        Answer { known: true, echo: Some("GitHub:octo-org/docs-probe"), ..Default::default() },
    ] {
        let m = mock(answer(a));
        let (t, home) = scratch();
        let out = run(t.path(), &home, &m.url, &[], &["--repo", "github:octo-org/docs-probe"], DIFF);
        assert_eq!(out.code, 3);
        assert!(out.stderr.contains("does not scope the check to a repository"), "{}", out.stderr);
    }
}

#[test]
fn every_exit_row() {
    let repo = ["--repo", "github:octo-org/docs-probe"];
    let (t, home) = scratch();
    let go = |a: Answer, extra: &[&str], diff: &str| {
        let m = mock(answer(a));
        let mut args = repo.to_vec();
        args.extend_from_slice(extra);
        run(t.path(), &home, &m.url, &[], &args, diff)
    };
    // nothing removed
    let o = go(Answer { known: false, ..Default::default() }, &[], "M\tREADME.md\n");
    assert_eq!((o.code, o.stdout.trim()), (0, "No removed or renamed paths in this change — nothing to check."));
    // clean, with the other-repositories line on exit 0
    let o = go(Answer { known: true, others: 2, ..Default::default() }, &[], DIFF);
    assert_eq!(o.code, 0);
    assert!(o.stdout.starts_with("Checked 2 removed path(s) about github:octo-org/docs-probe (listing from 2026-10-04 12:20 UTC); no live documentation claims affected."), "{}", o.stdout);
    assert!(o.stdout.contains("2 claim(s) about other repositories name these paths."));
    // findings only → 2; --warn-only → 0
    let a = Answer { known: true, findings: vec!["docs/shared.md"], ..Default::default() };
    assert_eq!(go(a.clone(), &[], DIFF).code, 2);
    assert_eq!(go(a, &["--warn-only"], DIFF).code, 0);
    // could not tell only → 3, with --warn-only too
    let a = Answer { known: true, could_not_tell: vec!["not_in_listing"], ..Default::default() };
    let o = go(a.clone(), &[], DIFF);
    assert_eq!(o.code, 3);
    assert!(o.stdout.contains("A listing of github:octo-org/docs-probe that DocBrain holds (the oldest is from 2026-10-04 12:20 UTC) does not include these paths"), "{}", o.stdout);
    assert_eq!(go(a, &["--warn-only"], DIFF).code, 3);
    // a reason this CLI does not know is still said, and still 3
    let o = go(Answer { known: true, could_not_tell: vec!["some_future_reason"], ..Default::default() }, &[], DIFF);
    assert_eq!(o.code, 3);
    assert!(o.stdout.contains("some_future_reason"));
}

/// A key that may see no spaces checked nothing (3); a scoped key's
/// four count lines carry the suffix, and only those.
#[test]
fn scope_is_said_on_the_count_lines_and_an_empty_scope_is_exit_3() {
    let repo = ["--repo", "github:octo-org/docs-probe"];
    let (t, home) = scratch();
    let m = mock(answer(Answer { known: true, scoped: Some(vec![]), ..Default::default() }));
    let o = run(t.path(), &home, &m.url, &[], &repo, DIFF);
    assert_eq!(o.code, 3);
    assert!(o.stderr.contains("this key may see no spaces; nothing was checked"));

    let m = mock(answer(Answer {
        known: true,
        scoped: Some(vec!["TWIN"]),
        findings: vec!["docs/shared.md"],
        could_not_tell: vec!["not_in_listing"],
        others: 1,
        ..Default::default()
    }));
    let o = run(t.path(), &home, &m.url, &[], &repo, DIFF);
    let s = " — within the 1 space this key may see";
    assert!(o.stdout.contains(&format!("would be falsified by this change{s}:")), "{}", o.stdout);
    assert!(o.stdout.contains(&format!("are about github:octo-org/docs-probe{s}:")));
    assert!(o.stdout.contains(&format!("1 claim(s) about other repositories name these paths{s}.")));
    assert_eq!(o.stdout.matches("within the").count(), 3, "group lines carry no suffix:\n{}", o.stdout);

    let m = mock(answer(Answer { known: true, scoped: Some(vec!["A", "B"]), ..Default::default() }));
    let o = run(t.path(), &home, &m.url, &[], &repo, DIFF);
    assert!(o.stdout.contains("no live documentation claims affected — within the 2 spaces this key may see."), "{}", o.stdout);
}

#[test]
fn a_422_is_exit_3_with_the_servers_line() {
    let m = mock(|_: &Value| (422, "this check needs \"repository\" (github:owner/repo or gitlab:group/project); upgrade the DocBrain CLI or pass --repo".to_string()));
    let (t, home) = scratch();
    let o = run(t.path(), &home, &m.url, &[], &["--repo", "github:o/r"], DIFF);
    assert_eq!(o.code, 3);
    assert!(o.stderr.contains("DocBrain returned 422"), "{}", o.stderr);
}

/// A failed `git diff` in a pipe without pipefail hands the CLI an empty stdin.
/// That is never "nothing to check": exit 3, nothing sent. An empty FILE is a
/// change that removes nothing (exit 0), and a stdin diff with lines is read.
#[test]
fn an_empty_stdin_diff_is_exit_3_but_an_empty_file_is_not() {
    let m = mock(answer(Answer { known: true, ..Default::default() }));
    let (t, home) = scratch();
    let stdin_run = |input: &str| {
        let mut child = Command::new(BIN)
            .current_dir(t.path())
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", &home)
            .env("DOCBRAIN_SERVER_URL", &m.url)
            .args(["check-claims", "--diff", "-", "--repo", "github:o/r"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        let o = child.wait_with_output().unwrap();
        (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stderr).to_string())
    };
    let (code, err) = stdin_run("");
    assert_eq!(code, 3);
    assert!(err.contains("the diff read from stdin is empty"), "{err}");
    assert_eq!(stdin_run(" \n\n").0, 3);
    assert!(sent(&m).is_empty(), "an unread diff must never be sent");

    let (code, _) = stdin_run("M\tREADME.md\n");
    assert_eq!(code, 0);
    let o = run(t.path(), &home, &m.url, &[], &["--repo", "github:o/r"], "");
    assert_eq!(o.code, 0, "an empty file is a change that removes nothing");
    assert_eq!(o.stdout.trim(), "No removed or renamed paths in this change — nothing to check.");
}
