// SPDX-License-Identifier: MIT
//! #603: `docbrain-mcp` reads no `.env` file unless `--docbrain-env` names one (owner rulings
//! 2026-10-23 and 2026-10-24). A `.env` in the directory an editor starts the connector in, or
//! in any parent, used to be loaded, so a cloned repository could set `HTTP_PROXY` and receive
//! the connector's `Authorization: Bearer <key>` through its own listener. A named file may set
//! only DocBrain's own variables that the connector reads: the key and the server.
//!
//! Every test runs the real binary with a cleared environment, a scratch working directory and
//! local listeners: the DocBrain stub the host points at, and a recorder that any proxy variable
//! would send traffic to. All values are synthetic.

mod common;

use common::{ALLOWED_IN_THE_CLI, ALLOWED_IN_THE_CONNECTOR, REFUSED_EVERYWHERE, listener, seen};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_docbrain-mcp");
const ME: &str = r#"{"name":"synthetic","role":"viewer"}"#;

/// The connector as an editor starts it: the given arguments and environment only, stdin
/// closed (so the JSON-RPC loop ends at once after startup validation).
fn run<A: AsRef<std::ffi::OsStr>>(cwd: &Path, args: &[A], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(BIN);
    cmd.args(args).current_dir(cwd).env_clear().stdin(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn docbrain-mcp")
}

/// A `.env` that would capture every request: all three proxy variables at the recorder, a CA
/// file, and the server URL at the recorder too.
fn hostile_dotenv(dir: &Path, recorder: &str, extra: &str) {
    let ca = dir.join("ca.pem");
    std::fs::write(&ca, "-----BEGIN CERTIFICATE-----\nU1lOVEhFVElD\n-----END CERTIFICATE-----\n").unwrap();
    std::fs::write(
        dir.join(".env"),
        format!(
            "HTTP_PROXY={recorder}\nHTTPS_PROXY={recorder}\nALL_PROXY={recorder}\nSSL_CERT_FILE={}\nDOCBRAIN_SERVER_URL={recorder}\n{extra}",
            ca.display()
        ),
    )
    .unwrap();
}

/// A repository with a `.env` at its root, and a folder inside it with none: the connector is
/// started in each, so both "this directory" and "a parent" are covered.
fn repo_with_dotenv(recorder: &str, extra: &str) -> (tempfile::TempDir, Vec<PathBuf>) {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    let sub = repo.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    hostile_dotenv(&repo, recorder, extra);
    (root, vec![repo, sub])
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// The control the sentinels rest on: a proxy variable does send loopback traffic through the
/// proxy, so "the recorder received nothing" below means the variable was not set.
#[test]
fn a_proxy_the_host_sets_does_route_loopback_traffic() {
    let (stub, stub_rx) = listener("200 OK", ME);
    let (proxy, proxy_rx) = listener("200 OK", ME);
    let root = tempfile::tempdir().unwrap();
    let out = run(root.path(), &[] as &[&str], &[("DOCBRAIN_API_KEY", "SYNTH_HOST_KEY"), ("DOCBRAIN_SERVER_URL", &stub), ("HTTP_PROXY", &proxy)]);
    let via = seen(&proxy_rx);
    assert!(via.contains(&format!("GET {stub}/api/v1/auth/me")), "the host's proxy was not used:\n{}\n{via}", stderr(&out));
    assert!(seen(&stub_rx).is_empty());
}

#[test]
fn a_dotenv_here_or_in_a_parent_never_reaches_the_connector() {
    let (stub, stub_rx) = listener("200 OK", ME);
    let (recorder, rec_rx) = listener("502 Bad Gateway", "");
    let (_root, cwds) = repo_with_dotenv(&recorder, "");
    for cwd in &cwds {
        let out = run(cwd, &[] as &[&str], &[("DOCBRAIN_API_KEY", "SYNTH_HOST_KEY"), ("DOCBRAIN_SERVER_URL", &stub)]);
        let to_stub = seen(&stub_rx);
        assert!(
            to_stub.contains("GET /api/v1/auth/me") && to_stub.contains("SYNTH_HOST_KEY"),
            "from {}: the server the host named was not asked (exit {:?}):\n{}\n{to_stub}",
            cwd.display(),
            out.status.code(),
            stderr(&out)
        );
        assert!(out.status.success(), "from {}: {}", cwd.display(), stderr(&out));
    }
    let captured = seen(&rec_rx);
    assert!(captured.is_empty(), "a proxy named only by .env received traffic:\n{captured}");
}

#[test]
fn a_dotenv_never_supplies_the_key() {
    let (stub, stub_rx) = listener("200 OK", ME);
    let (recorder, rec_rx) = listener("502 Bad Gateway", "");
    let (_root, cwds) = repo_with_dotenv(&recorder, "DOCBRAIN_API_KEY=SYNTH_DOTENV_KEY\n");
    for cwd in &cwds {
        let out = run(cwd, &[] as &[&str], &[("DOCBRAIN_SERVER_URL", &stub)]);
        assert_eq!(out.status.code(), Some(1), "from {}: {}", cwd.display(), stderr(&out));
        assert!(stderr(&out).contains("DOCBRAIN_API_KEY is not set"), "from {}: {}", cwd.display(), stderr(&out));
    }
    let sent = format!("{}\n{}", seen(&stub_rx), seen(&rec_rx));
    assert!(!sent.contains("SYNTH_DOTENV_KEY"), "a key from .env was sent:\n{sent}");
}

#[test]
fn docbrain_env_loads_the_named_file_and_only_it() {
    let (stub, stub_rx) = listener("200 OK", ME);
    let (recorder, rec_rx) = listener("502 Bad Gateway", "");
    // The working directory's .env would send everything to the recorder and carries a
    // different key: either one showing up means it was read as well.
    let (root, cwds) = repo_with_dotenv(&recorder, "DOCBRAIN_API_KEY=SYNTH_CWD_KEY\nNO_PROXY=127.0.0.1\n");
    let named = root.path().join("connector.env");
    std::fs::write(&named, format!("DOCBRAIN_API_KEY=SYNTH_NAMED_KEY\nDOCBRAIN_SERVER_URL={stub}\n")).unwrap();
    let named_arg = named.to_str().unwrap().to_string();
    let joined = format!("--docbrain-env={named_arg}");
    for cwd in &cwds {
        for args in [vec!["--docbrain-env", named_arg.as_str()], vec![joined.as_str()]] {
            let out = run(cwd, &args, &[]);
            let to_stub = seen(&stub_rx);
            assert!(
                to_stub.contains("GET /api/v1/auth/me") && to_stub.contains("SYNTH_NAMED_KEY"),
                "{args:?} from {}: the named file was not loaded:\n{}\n{to_stub}",
                cwd.display(),
                stderr(&out)
            );
            assert!(!to_stub.contains("SYNTH_CWD_KEY"), "{args:?}: the working directory's .env was read:\n{to_stub}");
            assert!(out.status.success(), "{args:?} from {}: {}", cwd.display(), stderr(&out));
        }
    }
    let captured = seen(&rec_rx);
    assert!(captured.is_empty(), "the working directory's .env was read as well:\n{captured}");
}

#[test]
fn variables_the_host_set_win_over_the_named_file() {
    let (stub, stub_rx) = listener("200 OK", ME);
    let root = tempfile::tempdir().unwrap();
    let named = root.path().join("connector.env");
    std::fs::write(&named, format!("DOCBRAIN_API_KEY=SYNTH_FILE_KEY\nDOCBRAIN_SERVER_URL={stub}\n")).unwrap();
    let out = run(root.path(), &["--docbrain-env", named.to_str().unwrap()], &[("DOCBRAIN_API_KEY", "SYNTH_HOST_KEY")]);
    assert!(out.status.success(), "{}", stderr(&out));
    let to_stub = seen(&stub_rx);
    assert!(to_stub.contains("SYNTH_HOST_KEY") && !to_stub.contains("SYNTH_FILE_KEY"), "{to_stub}");
}

/// A named file may set only DocBrain's own variables that the connector reads (owner ruling
/// 2026-10-24, an allow-list). Any other variable in it, DocBrain's or not, refuses the whole
/// file before any variable is set or anything is sent, naming the variable as the file spells
/// it, never the value or the path. Among them: the variables the round-2 review found (git's
/// global config, Apple's git shim, xdg-open), proxies in either case, the CLI's own settings
/// (DocBrain's, but the connector does not read them), and other spellings of an allowed name.
#[test]
fn a_named_file_may_set_only_the_connector_s_own_variables() {
    let (stub, stub_rx) = listener("200 OK", ME);
    let (proxy, proxy_rx) = listener("200 OK", ME);
    let root = tempfile::tempdir().unwrap();
    for (i, name) in REFUSED_EVERYWHERE.iter().chain(ALLOWED_IN_THE_CLI).enumerate() {
        let named = root.path().join(format!("connector-{i}.env"));
        std::fs::write(
            &named,
            format!("DOCBRAIN_API_KEY=SYNTH_NAMED_KEY\nDOCBRAIN_SERVER_URL={stub}\n{name}=SYNTH_REFUSED_VALUE\n"),
        )
        .unwrap();
        let out = run(root.path(), &["--docbrain-env", named.to_str().unwrap()], &[("http_proxy", &proxy), ("https_proxy", &proxy)]);
        let err = stderr(&out);
        assert!(!out.status.success(), "{name}: a file that sets it was loaded:\n{err}");
        assert!(err.contains(&format!("sets {name}")) && err.contains("may set only"), "{name}: the refusal should name it:\n{err}");
        assert!(!err.contains("SYNTH_REFUSED_VALUE") && !err.contains(&named.display().to_string()), "{name}: echoed:\n{err}");
    }
    let sent = format!("{}\n{}", seen(&stub_rx), seen(&proxy_rx));
    assert!(sent.trim().is_empty(), "a refused file let a request through:\n{sent}");
}

/// A file holding every variable the connector allows is loaded (the key and the server reach
/// the stub), so the list refuses nothing it should accept.
#[test]
fn every_variable_the_connector_allows_loads() {
    let (stub, stub_rx) = listener("200 OK", ME);
    let root = tempfile::tempdir().unwrap();
    let named = root.path().join("connector.env");
    let body: String = ALLOWED_IN_THE_CONNECTOR
        .iter()
        .map(|n| match *n {
            "DOCBRAIN_API_KEY" => format!("{n}=SYNTH_NAMED_KEY\n"),
            "DOCBRAIN_SERVER_URL" => format!("{n}={stub}\n"),
            other => panic!("{other}: give it a value that shows it was loaded"),
        })
        .collect();
    std::fs::write(&named, body).unwrap();
    let out = run(root.path(), &["--docbrain-env", named.to_str().unwrap()], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(seen(&stub_rx).contains("SYNTH_NAMED_KEY"), "the allowed variables were not applied");
}

/// A value holding a NUL byte cannot be put in the environment. It is refused like a bad line,
/// without the value, never as a panic (whose message would print it).
#[test]
fn a_value_with_a_nul_byte_is_refused_without_showing_it() {
    let (stub, stub_rx) = listener("200 OK", ME);
    let root = tempfile::tempdir().unwrap();
    let named = root.path().join("nul.env");
    std::fs::write(&named, format!("DOCBRAIN_SERVER_URL={stub}\nDOCBRAIN_API_KEY=SYNTH_NUL_KEY\0tail\n")).unwrap();
    let out = run(root.path(), &["--docbrain-env", named.to_str().unwrap()], &[]);
    let err = stderr(&out);
    assert!(!out.status.success() && err.contains("NUL") && !err.contains("panicked"), "{err}");
    assert!(!err.contains("SYNTH_NUL_KEY"), "the value was shown:\n{err}");
    assert!(seen(&stub_rx).is_empty());
}

/// A variable the host set to an empty value counts as unset, so the named file fills it: an
/// editor config with an empty placeholder, or an image that declares the variable empty, must
/// not silently cancel the file the user named.
#[test]
fn an_empty_host_value_does_not_cancel_the_named_file() {
    let (stub, stub_rx) = listener("200 OK", ME);
    let root = tempfile::tempdir().unwrap();
    let named = root.path().join("connector.env");
    std::fs::write(&named, format!("DOCBRAIN_API_KEY=SYNTH_FILE_KEY\nDOCBRAIN_SERVER_URL={stub}\n")).unwrap();
    let out = run(root.path(), &["--docbrain-env", named.to_str().unwrap()], &[("DOCBRAIN_API_KEY", ""), ("DOCBRAIN_SERVER_URL", "")]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(seen(&stub_rx).contains("SYNTH_FILE_KEY"), "the file's key was not used");
}

/// A file that was asked for and cannot be used stops the connector before it reads any
/// variable or sends anything: running without it would be running on settings nobody chose.
/// The options Node acts on itself are refused too, so a direct launch and an npm launch agree.
/// No refusal shows a value given on the command line, a path included.
#[test]
fn docbrain_env_refusals_start_nothing() {
    let (stub, stub_rx) = listener("200 OK", ME);
    let (recorder, rec_rx) = listener("502 Bad Gateway", "");
    let root = tempfile::tempdir().unwrap();
    let malformed = root.path().join("malformed.env");
    std::fs::write(&malformed, "SYNTH_SECRET_VALUE_42 is not an assignment\n").unwrap();
    let missing = root.path().join("SYNTH_MISSING_PATH.env");
    let ok = root.path().join("ok.env");
    std::fs::write(&ok, "DOCBRAIN_API_KEY=SYNTH_NAMED_KEY\n").unwrap();
    std::fs::write(root.path().join("relative.env"), format!("HTTP_PROXY={recorder}\n")).unwrap();
    let ok_s = ok.display().to_string();
    let host = [("DOCBRAIN_API_KEY", "SYNTH_HOST_KEY"), ("DOCBRAIN_SERVER_URL", stub.as_str())];
    let cases: Vec<(Vec<String>, &str)> = vec![
        (vec!["--docbrain-env".into(), "relative.env".into()], "absolute path"),
        (vec!["--docbrain-env=relative.env".into()], "absolute path"),
        (vec!["--docbrain-env".into(), "SYNTH_PASTED_KEY".into()], "absolute path"),
        (vec!["--docbrain-env".into(), missing.display().to_string()], "cannot read"),
        (vec!["--docbrain-env".into(), malformed.display().to_string()], "not NAME=value"),
        (vec!["--docbrain-env".into()], "needs a path"),
        (vec!["--docbrain-env=".into()], "needs a path"),
        (vec!["--docbrain-env".into(), ok_s.clone(), "--docbrain-env".into(), missing.display().to_string()], "more than once"),
        (vec!["--env-file".into(), ok_s.clone()], "--docbrain-env"),
        (vec![format!("--env-file={ok_s}")], "--docbrain-env"),
        (vec!["--env-file-if-exists".into(), ok_s.clone()], "--docbrain-env"),
        (vec!["--experimental-config-file".into(), ok_s.clone()], "--docbrain-env"),
        (vec![format!("--experimental-config-file={ok_s}")], "--docbrain-env"),
        (vec!["--experimental-default-config-file".into()], "--docbrain-env"),
        // Older Node lines match these by prefix, so the binary refuses the prefix.
        (vec!["--env-files=/x.env".into()], "--docbrain-env"),
        (vec!["--experimental-default-config-filex".into()], "--docbrain-env"),
    ];
    for (args, says) in &cases {
        let out = run(root.path(), args, &host);
        let err = stderr(&out);
        assert!(!out.status.success(), "{args:?} started:\n{err}");
        assert!(err.contains(says), "{args:?} should say {says:?}:\n{err}");
        for secret in ["SYNTH_SECRET_VALUE_42", "SYNTH_PASTED_KEY", "SYNTH_MISSING_PATH", "relative.env", "ok.env"] {
            assert!(!err.contains(secret), "{args:?} echoed {secret}:\n{err}");
        }
    }
    let sent = format!("{}\n{}", seen(&stub_rx), seen(&rec_rx));
    assert!(sent.trim().is_empty(), "a refused start sent a request:\n{sent}");
}

/// An argument the connector does not know is counted, never shown: a key typed where a flag
/// was expected (`--api-key <key>`) must not reach the host's log. The connector still starts,
/// because hosts pass arguments it has never heard of. An argument that is not UTF-8 is counted
/// the same way, never a panic.
#[test]
fn unknown_arguments_are_counted_never_shown() {
    let (stub, stub_rx) = listener("200 OK", ME);
    let root = tempfile::tempdir().unwrap();
    let host = [("DOCBRAIN_API_KEY", "SYNTH_HOST_KEY"), ("DOCBRAIN_SERVER_URL", stub.as_str())];
    let mut cases: Vec<Vec<std::ffi::OsString>> = vec![
        vec!["--api-key".into(), "SYNTH_PASTED_KEY_ARG".into()],
        vec!["--api-key=SYNTH_PASTED_KEY_ARG".into()],
    ];
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        cases.push(vec![std::ffi::OsString::from_vec(b"SYNTH_PASTED_KEY_ARG\xff".to_vec())]);
    }
    for args in &cases {
        let out = run(root.path(), args, &host);
        let err = stderr(&out);
        assert!(out.status.success(), "{args:?}: {err}");
        assert!(err.contains("unrecognised argument"), "{args:?}: no warning:\n{err}");
        assert!(!err.contains("SYNTH_PASTED_KEY_ARG") && !err.contains("panicked"), "{args:?}:\n{err}");
        assert!(seen(&stub_rx).contains("SYNTH_HOST_KEY"), "{args:?}: did not start");
    }
}

/// A `--docbrain-env` value that is not UTF-8 is taken as the path it names (here one that does
/// not exist, since some file systems refuse such a name): refused as unreadable, never dropped
/// and never a panic. The joined form cannot carry one and is refused as such.
#[cfg(unix)]
#[test]
fn a_path_that_is_not_utf8_is_a_path_not_a_panic() {
    use std::os::unix::ffi::OsStringExt;
    let (stub, stub_rx) = listener("200 OK", ME);
    let root = tempfile::tempdir().unwrap();
    let mut path = root.path().as_os_str().to_owned().into_vec();
    path.extend_from_slice(b"/named\xff.env");
    let path = std::ffi::OsString::from_vec(path);
    let host = [("DOCBRAIN_API_KEY", "SYNTH_HOST_KEY"), ("DOCBRAIN_SERVER_URL", stub.as_str())];
    let out = run(root.path(), &[std::ffi::OsString::from("--docbrain-env"), path.clone()], &host);
    let err = stderr(&out);
    assert!(!out.status.success() && err.contains("cannot read") && !err.contains("panicked"), "{err}");
    let mut joined = b"--docbrain-env=".to_vec();
    joined.extend_from_slice(&path.into_vec());
    let out = run(root.path(), &[std::ffi::OsString::from_vec(joined)], &host);
    let err = stderr(&out);
    assert!(!out.status.success() && err.contains("UTF-8") && !err.contains("panicked"), "{err}");
    assert!(seen(&stub_rx).is_empty(), "a refused start sent a request");
}

/// A file saved with a UTF-8 byte-order mark (Windows editors, PowerShell) loads like the same
/// file without one, as the old implicit `.env` did.
#[test]
fn a_file_with_a_byte_order_mark_loads() {
    let (stub, stub_rx) = listener("200 OK", ME);
    let root = tempfile::tempdir().unwrap();
    let named = root.path().join("bom.env");
    std::fs::write(&named, format!("\u{feff}DOCBRAIN_API_KEY=SYNTH_BOM_KEY\nDOCBRAIN_SERVER_URL={stub}\n")).unwrap();
    let out = run(root.path(), &["--docbrain-env", named.to_str().unwrap()], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(seen(&stub_rx).contains("SYNTH_BOM_KEY"), "the file was not loaded");
}

/// A `$` in the file is refused: the parser would replace `$NAME` or `${NAME}` with another
/// variable from the environment, so an allowed name could carry a value the file may not set
/// (a host secret into the server URL). The value is not shown.
#[test]
fn a_file_that_would_read_another_variable_is_refused() {
    let (stub, stub_rx) = listener("200 OK", ME);
    let root = tempfile::tempdir().unwrap();
    let host = [("SYNTH_HOST_SECRET", "SYNTHHOSTSECRET42"), ("DOCBRAIN_API_KEY", "SYNTH_HOST_KEY")];
    for (i, line) in [
        format!("DOCBRAIN_SERVER_URL={stub}/x-${{SYNTH_HOST_SECRET}}"),
        format!("DOCBRAIN_SERVER_URL=\"{stub}/x-$SYNTH_HOST_SECRET\""),
        format!("DOCBRAIN_SERVER_URL='{stub}/x-$SYNTH_LITERAL'"),
    ]
    .iter()
    .enumerate()
    {
        let named = root.path().join(format!("subst-{i}.env"));
        std::fs::write(&named, format!("{line}\n")).unwrap();
        let out = run(root.path(), &["--docbrain-env", named.to_str().unwrap()], &host);
        let err = stderr(&out);
        assert!(!out.status.success() && err.contains('$'), "{line}: loaded:\n{err}");
        assert!(!err.contains("SYNTHHOSTSECRET42") && !err.contains("SYNTH_HOST_SECRET") && !err.contains("x-"), "{line}: echoed:\n{err}");
    }
    assert!(seen(&stub_rx).is_empty(), "a refused file let a request through");
}
