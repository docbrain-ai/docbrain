// SPDX-License-Identifier: MIT
//! `--profile` through the real binary: every refusal happens before any network or
//! credential prompt, and the offline commands and `version` keep working when the
//! server/key pair cannot be resolved. A scratch HOME and cwd, so no real config or `.env`.

use std::path::Path;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_docbrain-cli");

/// The CLI's identity tag (server + key), mirrored here so a test can place a conversation
/// where exactly one identity will find it.
fn tag(url: &str, key: &str) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(url.trim().trim_end_matches('/').as_bytes());
    h.update(b"\n");
    h.update(key.as_bytes());
    hex::encode(&h.finalize()[..8])
}

fn two_identities(url: &str) -> String {
    format!(
        r#"{{"server_url":"{url}","api_key":"SAVED_DEFAULT_KEY",
            "profiles":{{"reader":{{"server_url":"{url}","api_key":"SAVED_READER_KEY"}}}}}}"#
    )
}

const CONFIG: &str = r#"{"server_url":"http://127.0.0.1:9","api_key":"SAVED_DEFAULT_KEY",
  "profiles":{"reader":{"server_url":"http://127.0.0.1:9","api_key":"SAVED_READER_KEY"}}}"#;

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn home_with(config: Option<&str>) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    if let Some(c) = config {
        std::fs::create_dir(home.path().join(".docbrain")).unwrap();
        std::fs::write(home.path().join(".docbrain/config.json"), c).unwrap();
    }
    home
}

fn run(home: &Path, args: &[&str], env: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .current_dir(home)
        .env("HOME", home)
        .env_remove("DOCBRAIN_API_KEY")
        .env_remove("DOCBRAIN_SERVER_URL")
        .env_remove("DOCBRAIN_PROFILE")
        .stdin(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn docbrain-cli");
    Run {
        code: out.status.code().expect("exited normally"),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn assert_no_key_printed(r: &Run) {
    for k in ["SAVED_DEFAULT_KEY", "SAVED_READER_KEY", "ENV_SECRET"] {
        assert!(!r.stdout.contains(k) && !r.stderr.contains(k), "{k} printed:\n{}\n{}", r.stdout, r.stderr);
    }
}

#[test]
fn an_unknown_profile_is_refused_naming_the_known_ones() {
    let home = home_with(Some(CONFIG));
    let r = run(home.path(), &["--profile", "editor", "ask", "hi"], &[]);
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("unknown profile \"editor\"") && r.stderr.contains("reader"), "{}", r.stderr);
    assert_no_key_printed(&r);
}

#[test]
fn docbrain_profile_env_selects_the_same_way() {
    let home = home_with(Some(CONFIG));
    let r = run(home.path(), &["ask", "hi"], &[("DOCBRAIN_PROFILE", "editor")]);
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("unknown profile \"editor\""), "{}", r.stderr);
}

#[test]
fn an_invalid_profile_name_is_refused() {
    let home = home_with(Some(CONFIG));
    let r = run(home.path(), &["--profile", "../x", "ask", "hi"], &[]);
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("invalid profile name"), "{}", r.stderr);
}

#[test]
fn the_saved_key_is_not_sent_to_an_env_named_server() {
    let home = home_with(Some(CONFIG));
    let r = run(home.path(), &["ask", "hi"], &[("DOCBRAIN_SERVER_URL", "http://127.0.0.1:8")]);
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("the saved key belongs to"), "{}", r.stderr);
    assert_no_key_printed(&r);
    let r = run(home.path(), &["--profile", "reader", "ask", "hi"], &[("DOCBRAIN_SERVER_URL", "http://127.0.0.1:8")]);
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("DOCBRAIN_SERVER_URL"), "{}", r.stderr);
    assert_no_key_printed(&r);
}

#[test]
fn api_key_and_profile_together_are_refused() {
    let home = home_with(Some(CONFIG));
    let r = run(home.path(), &["--profile", "reader", "--api-key", "ENV_SECRET", "ask", "hi"], &[]);
    assert_ne!(r.code, 0);
    assert_no_key_printed(&r);
}

#[test]
fn version_still_answers_when_the_pair_cannot_be_resolved() {
    let home = home_with(Some(CONFIG));
    let r = run(home.path(), &["--profile", "editor", "version"], &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("docbrain CLI") && r.stderr.contains("unknown profile"), "{}\n{}", r.stdout, r.stderr);
}

#[test]
fn offline_commands_ignore_an_unresolvable_pair() {
    let home = home_with(Some(CONFIG));
    std::fs::write(home.path().join("kf.json"), "{}").unwrap();
    std::fs::write(home.path().join("schema.json"), "{}").unwrap();
    for args in [
        &["--profile", "editor", "evidence", "verify", "missing.dbev"][..],
        &["--profile", "editor", "health", "validate", "kf.json", "--schema", "schema.json"][..],
        &["evidence", "verify", "missing.dbev"][..],
    ] {
        let r = run(home.path(), args, &[("DOCBRAIN_SERVER_URL", "http://127.0.0.1:8")]);
        assert!(
            !r.stderr.contains("unknown profile") && !r.stderr.contains("saved key belongs"),
            "{args:?} was stopped by server resolution:\n{}", r.stderr
        );
    }
}

#[test]
fn a_new_profile_login_without_a_server_is_refused_before_any_prompt() {
    let home = home_with(Some(CONFIG));
    let r = run(home.path(), &["--profile", "editor", "login", "--email", "a@b.c"], &[]);
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("--server") && !r.stderr.contains("Password"), "{}", r.stderr);
}

#[test]
fn a_signed_in_profile_is_not_moved_to_another_server() {
    let home = home_with(Some(CONFIG));
    let r = run(home.path(), &["--profile", "reader", "login", "--server", "http://127.0.0.1:8", "--email", "a@b.c"], &[]);
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("logout"), "{}", r.stderr);
}

#[test]
fn a_corrupt_config_is_never_overwritten_by_login() {
    let home = home_with(Some("{not json"));
    let r = run(home.path(), &["login", "--server", "http://127.0.0.1:9", "--email", "a@b.c"], &[]);
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("not valid DocBrain config"), "{}", r.stderr);
    assert_eq!(std::fs::read_to_string(home.path().join(".docbrain/config.json")).unwrap(), "{not json");
}

#[test]
fn a_corrupt_config_stops_commands_instead_of_falling_back_to_env() {
    let home = home_with(Some("{not json"));
    let r = run(home.path(), &["ask", "hi"], &[("DOCBRAIN_API_KEY", "ENV_SECRET")]);
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("not valid DocBrain config"), "{}", r.stderr);
    assert_no_key_printed(&r);
}

#[test]
fn logout_of_an_unknown_profile_changes_nothing() {
    let home = home_with(Some(CONFIG));
    let r = run(home.path(), &["--profile", "editor", "logout"], &[]);
    assert_ne!(r.code, 0);
    assert_eq!(std::fs::read_to_string(home.path().join(".docbrain/config.json")).unwrap(), CONFIG);
}

/// A local listener that records every request and answers 500, so the CLI's real
/// requests can be inspected without a server.
fn recording_server() -> (String, std::sync::mpsc::Receiver<String>) {
    recording_server_answering("500 Internal Server Error", "")
}

fn recording_server_answering(status: &'static str, body: &'static str) -> (String, std::sync::mpsc::Receiver<String>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            stream.set_read_timeout(Some(std::time::Duration::from_millis(500))).ok();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n) = stream.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
            let _ = stream.write_all(
                format!("HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes(),
            );
        }
    });
    (url, rx)
}

#[test]
fn continue_resumes_only_the_selected_profiles_conversation_with_its_own_key() {
    let (url, rx) = recording_server();
    let home = home_with(Some(&two_identities(&url)));
    let d = home.path().join(".docbrain");
    std::fs::write(d.join(format!("session-{}", tag(&url, "SAVED_DEFAULT_KEY"))), "conv-of-default").unwrap();
    std::fs::write(d.join(format!("session-{}", tag(&url, "SAVED_READER_KEY"))), "conv-of-reader").unwrap();

    let _ = run(home.path(), &["--profile", "reader", "ask", "hi", "--continue"], &[]);
    let sent: String = rx.try_iter().collect::<Vec<_>>().join("\n");
    assert!(sent.contains("conv-of-reader") && sent.contains("SAVED_READER_KEY"), "reader's request:\n{sent}");
    assert!(!sent.contains("conv-of-default") && !sent.contains("SAVED_DEFAULT_KEY"), "default leaked:\n{sent}");

    let _ = run(home.path(), &["ask", "hi", "--continue"], &[]);
    let sent: String = rx.try_iter().collect::<Vec<_>>().join("\n");
    assert!(sent.contains("conv-of-default") && sent.contains("SAVED_DEFAULT_KEY"), "default's request:\n{sent}");
    assert!(!sent.contains("conv-of-reader") && !sent.contains("SAVED_READER_KEY"), "reader leaked:\n{sent}");
}

#[test]
fn a_key_from_the_environment_or_the_flag_never_continues_the_saved_identitys_conversation() {
    let (url, rx) = recording_server();
    let (other, rx_other) = recording_server();
    let home = home_with(Some(&two_identities(&url)));
    std::fs::write(
        home.path().join(".docbrain").join(format!("session-{}", tag(&url, "SAVED_DEFAULT_KEY"))),
        "conv-of-default",
    ).unwrap();

    let _ = run(home.path(), &["ask", "hi", "--continue"], &[("DOCBRAIN_SERVER_URL", &other), ("DOCBRAIN_API_KEY", "ENV_KEY")]);
    let _ = run(home.path(), &["--api-key", "FLAG_KEY", "ask", "hi", "--continue"], &[]);
    let sent: String = rx.try_iter().chain(rx_other.try_iter()).collect::<Vec<_>>().join("\n");
    assert!(sent.contains("ENV_KEY") && sent.contains("FLAG_KEY"), "both runs reached a server:\n{sent}");
    assert!(!sent.contains("conv-of-default") && !sent.contains("SAVED_DEFAULT_KEY"), "{sent}");
}

#[test]
fn a_profile_without_a_saved_conversation_does_not_borrow_the_default_one() {
    let (url, rx) = recording_server();
    let home = home_with(Some(&two_identities(&url)));
    std::fs::write(
        home.path().join(".docbrain").join(format!("session-{}", tag(&url, "SAVED_DEFAULT_KEY"))),
        "conv-of-default",
    ).unwrap();
    std::fs::write(home.path().join(".docbrain/session"), "conv-from-an-older-cli").unwrap();
    let _ = run(home.path(), &["--profile", "reader", "ask", "hi", "--continue"], &[]);
    let sent: String = rx.try_iter().collect::<Vec<_>>().join("\n");
    assert!(sent.contains("SAVED_READER_KEY"), "{sent}");
    assert!(!sent.contains("conv-of-default") && !sent.contains("conv-from-an-older-cli"), "{sent}");
}

#[test]
fn a_dotenv_file_cannot_choose_the_identity() {
    let (url, rx) = recording_server();
    let home = home_with(Some(&two_identities(&url)));
    std::fs::write(home.path().join(".env"), "DOCBRAIN_PROFILE=reader\n").unwrap();
    let _ = run(home.path(), &["ask", "hi"], &[]);
    let sent: String = rx.try_iter().collect::<Vec<_>>().join("\n");
    assert!(sent.contains("SAVED_DEFAULT_KEY") && !sent.contains("SAVED_READER_KEY"), "{sent}");
}

#[test]
fn a_dotenv_file_cannot_choose_where_a_password_goes() {
    let home = home_with(Some(CONFIG));
    std::fs::write(home.path().join(".env"), "DOCBRAIN_SERVER_URL=http://127.0.0.1:7\n").unwrap();
    let r = run(home.path(), &["login", "--email", "a@b.c"], &[]);
    assert!(r.stderr.contains("Signing in to http://127.0.0.1:9"), "{}", r.stderr);
    assert!(!r.stderr.contains("127.0.0.1:7"), "{}", r.stderr);
}

#[test]
fn a_config_error_never_prints_the_value_it_choked_on() {
    let home = home_with(Some(r#"{"profiles":{"reader":"SECRET_MISPLACED_KEY"}}"#));
    for args in [&["version"][..], &["ask", "hi"][..], &["--profile", "reader", "ask", "hi"][..], &["login", "--server", "http://127.0.0.1:9"][..]] {
        let r = run(home.path(), args, &[]);
        assert!(r.stderr.contains("line 1, column"), "{args:?}: {}", r.stderr);
        assert!(!r.stdout.contains("SECRET") && !r.stderr.contains("SECRET"), "{args:?}:\n{}\n{}", r.stdout, r.stderr);
    }
}

#[cfg(unix)]
#[test]
fn a_world_readable_config_is_made_private_on_first_use() {
    use std::os::unix::fs::PermissionsExt;
    let home = home_with(Some(CONFIG));
    let path = home.path().join(".docbrain/config.json");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let _ = run(home.path(), &["--profile", "editor", "ask", "hi"], &[]);
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn logout_removes_a_key_the_server_no_longer_knows_and_keeps_one_it_could_not_revoke() {
    let (dead, _rx) = recording_server_answering("401 Unauthorized", "Invalid or expired API key");
    let home = home_with(Some(&two_identities(&dead)));
    let r = run(home.path(), &["--profile", "reader", "logout"], &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let after = std::fs::read_to_string(home.path().join(".docbrain/config.json")).unwrap();
    assert!(!after.contains("SAVED_READER_KEY") && after.contains("SAVED_DEFAULT_KEY"), "{after}");

    let (broken, _rx) = recording_server();
    let home = home_with(Some(&two_identities(&broken)));
    let r = run(home.path(), &["--profile", "reader", "logout"], &[]);
    assert_ne!(r.code, 0);
    let after = std::fs::read_to_string(home.path().join(".docbrain/config.json")).unwrap();
    assert!(after.contains("SAVED_READER_KEY"), "a key the server may still accept is not forgotten: {after}");
}

#[test]
fn logout_of_one_profile_leaves_every_other_sign_in_alone() {
    let (ok, rx) = recording_server_answering("200 OK", "");
    let home = home_with(Some(&two_identities(&ok)));
    let r = run(home.path(), &["--profile", "reader", "logout"], &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let sent: String = rx.try_iter().collect::<Vec<_>>().join("\n");
    assert!(sent.contains("/api/v1/auth/logout") && sent.contains("SAVED_READER_KEY") && !sent.contains("SAVED_DEFAULT_KEY"), "{sent}");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(home.path().join(".docbrain/config.json")).unwrap()).unwrap();
    assert_eq!(v["api_key"], "SAVED_DEFAULT_KEY");
    assert!(v["profiles"]["reader"]["api_key"].is_null());
    assert_eq!(v["profiles"]["reader"]["server_url"], ok.as_str());
}

#[test]
fn a_401_from_something_other_than_docbrain_does_not_forget_the_key() {
    let (proxy, _rx) = recording_server_answering("401 Unauthorized", "<html>proxy login required</html>");
    let home = home_with(Some(&two_identities(&proxy)));
    let r = run(home.path(), &["--profile", "reader", "logout"], &[]);
    assert_ne!(r.code, 0);
    let after = std::fs::read_to_string(home.path().join(".docbrain/config.json")).unwrap();
    assert!(after.contains("SAVED_READER_KEY"), "{after}");
}

fn run_in(cwd: &Path, home: &Path, args: &[&str], env: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env_remove("DOCBRAIN_API_KEY")
        .env_remove("DOCBRAIN_SERVER_URL")
        .env_remove("DOCBRAIN_PROFILE")
        .stdin(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn docbrain-cli");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Whatever a `.env` (here or in a parent directory) says about the server, the key or the
/// profile, the CLI speaks as the shell and the saved config say.
#[test]
fn a_dotenv_file_never_steers_the_server_the_key_or_the_profile() {
    let (saved, rx_saved) = recording_server();
    let (repo, rx_repo) = recording_server();
    let home = home_with(Some(&two_identities(&saved)));
    let nested = home.path().join("repo/sub");
    std::fs::create_dir_all(&nested).unwrap();
    for (dir, body) in [
        (home.path().to_path_buf(), format!("DOCBRAIN_SERVER_URL={repo}\n")),
        (home.path().to_path_buf(), format!("DOCBRAIN_SERVER_URL={repo}\nDOCBRAIN_API_KEY=DOTENV_KEY\n")),
        (home.path().to_path_buf(), "DOCBRAIN_API_KEY=DOTENV_KEY\n".to_string()),
        (home.path().join("repo"), format!("DOCBRAIN_SERVER_URL={repo}\nDOCBRAIN_PROFILE=reader\n")),
    ] {
        std::fs::write(dir.join(".env"), &body).unwrap();
        for (args, env) in [
            (&["ask", "hi"][..], &[][..]),
            (&["ask", "hi"][..], &[("DOCBRAIN_API_KEY", "SHELL_KEY")][..]),
            (&["--api-key", "FLAG_KEY", "ask", "hi"][..], &[][..]),
            (&["--profile", "reader", "ask", "hi"][..], &[][..]),
        ] {
            let _ = run_in(&nested, home.path(), args, env);
        }
        std::fs::remove_file(dir.join(".env")).unwrap();
    }
    let to_repo: String = rx_repo.try_iter().collect::<Vec<_>>().join("\n");
    assert!(to_repo.is_empty(), "a server named only by .env was contacted:\n{to_repo}");
    let to_saved: String = rx_saved.try_iter().collect::<Vec<_>>().join("\n");
    assert!(!to_saved.contains("DOTENV_KEY"), "a key from .env was used:\n{to_saved}");
    assert!(to_saved.contains("SAVED_DEFAULT_KEY"), "the default sign-in was not used:\n{to_saved}");
}

#[test]
fn a_dotenv_key_never_reaches_a_logged_out_sign_ins_server() {
    let (saved, rx) = recording_server();
    let home = home_with(Some(&format!(r#"{{"server_url":"{saved}"}}"#)));
    std::fs::write(home.path().join(".env"), "DOCBRAIN_API_KEY=DOTENV_KEY\n").unwrap();
    let _ = run(home.path(), &["ask", "hi"], &[]);
    let sent: String = rx.try_iter().collect::<Vec<_>>().join("\n");
    assert!(!sent.contains("DOTENV_KEY"), "{sent}");
}

#[test]
fn a_held_lock_makes_logout_fail_fast_before_anything_is_revoked() {
    let (srv, rx) = recording_server_answering("200 OK", "");
    let home = home_with(Some(&two_identities(&srv)));
    let lock = std::fs::OpenOptions::new()
        .read(true).write(true).create(true).truncate(false)
        .open(home.path().join(".docbrain/.config.lock")).unwrap();
    lock.lock().unwrap();
    let started = std::time::Instant::now();
    let r = run(home.path(), &["--profile", "reader", "logout"], &[]);
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "logout waited on the lock");
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("in progress"), "{}", r.stderr);
    let sent: String = rx.try_iter().collect::<Vec<_>>().join("\n");
    assert!(sent.is_empty(), "revoked while the config could not be written:\n{sent}");
    drop(lock);
}

#[test]
fn files_from_older_clis_are_removed() {
    let (url, _rx) = recording_server();
    let home = home_with(Some(&two_identities(&url)));
    let d = home.path().join(".docbrain");
    std::fs::write(d.join("session"), "old-conv").unwrap();
    std::fs::write(d.join("last_answer.json"), "{}").unwrap();
    let _ = run(home.path(), &["ask", "hi"], &[]);
    assert!(!d.join("session").exists() && !d.join("last_answer.json").exists());
}

#[cfg(unix)]
#[test]
fn login_stops_before_any_credential_when_the_config_cannot_be_locked() {
    use std::os::unix::fs::PermissionsExt;
    let home = home_with(Some(CONFIG));
    let d = home.path().join(".docbrain");
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o500)).unwrap();
    let r = run(home.path(), &["login", "--email", "a@b.c"], &[]);
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_ne!(r.code, 0);
    assert!(!r.stderr.contains("Signing in") && !r.stderr.contains("Password"), "{}", r.stderr);
}

#[test]
fn profile_after_the_command_selects_the_same_sign_in() {
    let (url, rx) = recording_server();
    let home = home_with(Some(&two_identities(&url)));
    let _ = run(home.path(), &["ask", "hi", "--profile", "reader"], &[]);
    let sent: String = rx.try_iter().collect::<Vec<_>>().join("\n");
    assert!(sent.contains("SAVED_READER_KEY") && !sent.contains("SAVED_DEFAULT_KEY"), "{sent}");
}

#[test]
fn two_different_profiles_on_one_command_line_are_refused_before_any_request() {
    let (url, rx) = recording_server();
    let home = home_with(Some(&two_identities(&url)));
    for args in [
        &["--profile", "reader", "ask", "--profile", "default", "hi"][..],
        &["ask", "hi", "--profile", "reader", "--profile=default"][..],
    ] {
        let r = run(home.path(), args, &[]);
        assert_ne!(r.code, 0, "{args:?}");
        assert!(r.stderr.contains("more than once"), "{args:?}: {}", r.stderr);
    }
    assert_eq!(rx.try_iter().count(), 0);
}

#[test]
fn evidence_export_takes_the_framework_as_framework_and_sends_it_as_the_api_field() {
    let (url, rx) = recording_server();
    let home = home_with(Some(&two_identities(&url)));
    let _ = run(home.path(), &["evidence", "export", "--framework", "soc2", "-o", "out.dbev", "--profile", "reader"], &[]);
    let sent: String = rx.try_iter().collect::<Vec<_>>().join("\n");
    assert!(sent.contains("/api/v1/evidence/export"), "{sent}");
    assert!(sent.contains(r#""profile":"soc2""#) && sent.contains("SAVED_READER_KEY"), "{sent}");
}

#[test]
fn the_old_evidence_profile_spelling_explains_itself_and_sends_nothing() {
    let (url, rx) = recording_server();
    let home = home_with(Some(&two_identities(&url)));
    let r = run(home.path(), &["evidence", "export", "--profile", "eu-ai-act", "-o", "out.dbev"], &[]);
    assert_ne!(r.code, 0);
    assert!(r.stderr.contains("--framework") && r.stderr.contains("unknown profile"), "{}", r.stderr);
    assert_eq!(rx.try_iter().count(), 0);
}
