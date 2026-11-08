// SPDX-License-Identifier: MIT
//! `docbrain record`, `context` and `status` end to end, on real git repositories built per test.

mod common;
#[path = "common/knowledge_repo.rs"]
mod knowledge_repo;

use std::process::{Command, Stdio};
use std::time::Duration;

use knowledge_repo::Repo;

/// One command line of each `.knowledge/` command, as a user types it.
const EVERY_COMMAND: [&[&str]; 6] = [
    &["record", "decision", "Production API runs at least two replicas", "--applies-to", "charts/api/**", "--body", "Why."],
    &["context", "charts/api/values.yaml"],
    &["context", "--brief", "charts/api/values.yaml"],
    &["context", "--json", "charts/api/values.yaml"],
    &["status"],
    &["status", "--base", "main"],
];

fn fixture() -> Repo {
    let r = Repo::new();
    r.write("charts/api/values.yaml", "replicaCount: 2\n");
    r.commit("first");
    r
}

#[test]
fn usage_errors_exit_3() {
    let r = fixture();
    for args in [
        &["status", "--bogus"][..],
        &["status", "--json", "--sarif"],
        &["status", "--base", "main", "--json", "--sarif"],
        &["record", "nonsense", "x"],
        &["--profile", "p", "status", "--bogus"],
        &["--api-key=k", "context", "--bogus"],
    ] {
        let out = r.run(args);
        assert_eq!(out.code, 3, "{args:?}: {out:?}");
        assert!(out.stdout.is_empty(), "{args:?}: {out:?}");
    }
}

#[test]
fn help_exits_0() {
    let r = fixture();
    for args in [&["status", "--help"][..], &["record", "--help"], &["help", "context"], &["context", "-h"]] {
        let out = r.run(args);
        assert_eq!(out.code, 0, "{args:?}: {out:?}");
        assert!(out.stdout.contains("Usage:"), "{args:?}: {out:?}");
    }
}

/// The usage exit of every other command is today's: read from the binary before these commands existed.
#[test]
fn other_commands_keep_their_usage_exit() {
    let r = fixture();
    for args in [&["check-claims", "--bogus"][..], &["ask", "--bogus"], &["--profile", "p", "ask", "--bogus"]] {
        assert_eq!(r.run(args).code, 2, "{args:?}");
    }
}

#[test]
fn outside_a_repository_exits_3_with_one_line() {
    let r = Repo::new();
    let elsewhere = tempfile::tempdir().unwrap();
    for args in EVERY_COMMAND {
        let out = r.run_with(elsewhere.path(), &[], None, args);
        assert_eq!(out.code, 3, "{args:?}: {out:?}");
        assert_eq!(out.stderr.lines().count(), 1, "{args:?}: {out:?}");
        assert!(out.stderr.ends_with('\n'), "{args:?}: {out:?}");
        assert!(out.stdout.is_empty(), "{args:?}: {out:?}");
    }
}

#[test]
fn a_git_failure_is_passed_on_in_one_line() {
    let r = fixture();
    let config = r.dir().join(".git/config");
    let text = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&config, format!("{text}[broken\n")).unwrap();
    for args in EVERY_COMMAND {
        let out = r.run(args);
        assert_eq!(out.code, 3, "{args:?}: {out:?}");
        assert_eq!(out.stderr.lines().count(), 1, "{args:?}: {out:?}");
        assert!(out.stderr.contains("bad config line"), "git's own words: {args:?}: {out:?}");
    }
}

/// Offline: no server is resolved, no sign-in is read, no socket is opened. A server is reachable and named, the
/// sign-in setting is one the online commands refuse, and the file the online commands tidy away is there.
#[test]
fn never_opens_a_socket() {
    let (url, requests) = common::listener("200 OK", "{}");
    let r = fixture();
    let session = r.home().join(".docbrain/session");
    std::fs::create_dir_all(session.parent().unwrap()).unwrap();
    std::fs::write(&session, "kept").unwrap();
    let env = [("DOCBRAIN_SERVER_URL", url.as_str()), ("DOCBRAIN_API_KEY", "fixture-key"), ("DOCBRAIN_PROFILE", "not a profile!")];
    for args in EVERY_COMMAND {
        let out = r.run_with(r.dir(), &env, None, args);
        assert!(matches!(out.code, 0 | 3), "{args:?}: {out:?}");
        assert!(!out.stderr.contains("profile"), "the profile was read: {args:?}: {out:?}");
    }
    assert_eq!(r.run_with(r.dir(), &env, None, &["status"]).code, 0, "status runs offline");
    assert!(requests.recv_timeout(Duration::from_millis(500)).is_err(), "a request reached the server");
    assert_eq!(std::fs::read_to_string(&session).unwrap(), "kept", "~/.docbrain was touched");
}

/// Output into a reader that has already gone (`| head -0`): no panic, and the exit the command gives anyway.
#[test]
fn emit_survives_a_closed_pipe() {
    let r = fixture();
    for args in [&["status"][..], &["context", "charts/api/values.yaml"], &["status", "--base", "main"]] {
        let want = r.run(args).code;
        let mut c = Command::new(knowledge_repo::binary());
        knowledge_repo::hermetic(&mut c, r.home());
        let mut child = c.current_dir(r.dir()).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        drop(child.stdout.take());
        let out = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(want), "{args:?}: {stderr}");
        assert!(!stderr.contains("panicked") && !stderr.contains("Broken pipe"), "{args:?}: {stderr}");
    }
}
