// SPDX-License-Identifier: MIT
//! #603, the CA half of the sentinel: a CA file named by a `.env` here or in a parent is never
//! trusted, and a `--docbrain-env` file may not name one. The stub speaks TLS with a certificate
//! only a CA made for the run vouches for.
//!
//! Whether a CA file can matter at all depends on the platform's verifier, and the control
//! test settles it on the machine it runs on. On Linux, OpenSSL reads `SSL_CERT_FILE` when the
//! CLI builds its client, so the control expects the host's CA to be trusted, and the
//! sentinel would fail if a `.env` were read. On macOS (Security.framework) and Windows
//! (SChannel), native-tls verifies against the system store and reads no such variable, so no
//! file of any kind can add a CA there: the control expects the handshake to fail even with the
//! host's own variable, and the refusal of a named file is what proves the rest.

mod common;

use common::{seen, tls_listener};
use std::path::Path;
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_docbrain-cli");

/// A scratch HOME whose saved sign-in points at the TLS stub.
fn home_for(dir: &Path, url: &str) -> std::path::PathBuf {
    let home = dir.join("home");
    std::fs::create_dir_all(home.join(".docbrain")).unwrap();
    std::fs::write(home.join(".docbrain/config.json"), format!(r#"{{"server_url":"{url}","api_key":"SYNTH_SAVED_KEY"}}"#)).unwrap();
    home
}

fn run(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(BIN);
    cmd.args(args).current_dir(cwd).env_clear().stdin(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn docbrain-cli")
}

#[test]
fn the_control_ssl_cert_file_from_the_host_counts_only_where_the_verifier_reads_it() {
    let dir = tempfile::tempdir().unwrap();
    let (url, rx, ca) = tls_listener(dir.path(), "200 OK", "{}");
    let ca = ca.to_str().unwrap().to_string();
    let home = home_for(dir.path(), &url);
    let out = run(dir.path(), &["version"], &[("HOME", home.to_str().unwrap()), ("SSL_CERT_FILE", &ca)]);
    let got = seen(&rx);
    let err = String::from_utf8_lossy(&out.stderr);
    if cfg!(target_os = "linux") {
        assert!(got.contains("GET /api/v1/config"), "OpenSSL did not trust the host's CA, so the sentinel would prove nothing:\n{err}");
    } else {
        assert!(got.is_empty(), "this platform trusted a CA named by SSL_CERT_FILE; run the sentinel here:\n{err}");
    }
}

#[test]
fn a_ca_named_only_by_a_dotenv_is_never_trusted() {
    let root = tempfile::tempdir().unwrap();
    let (url, rx, ca) = tls_listener(root.path(), "200 OK", "{}");
    let home = home_for(root.path(), &url);
    let repo = root.path().join("repo");
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    std::fs::write(repo.join(".env"), format!("SSL_CERT_FILE={}\nSSL_CERT_DIR={}\n", ca.display(), root.path().display())).unwrap();
    for cwd in [repo.clone(), repo.join("sub")] {
        let out = run(&cwd, &["version"], &[("HOME", home.to_str().unwrap())]);
        assert!(String::from_utf8_lossy(&out.stdout).contains("unreachable"), "from {}: {}", cwd.display(), String::from_utf8_lossy(&out.stdout));
    }
    let got = seen(&rx);
    assert!(got.is_empty(), "a CA named only by .env was trusted:\n{got}");
}

#[test]
fn a_named_file_may_not_name_a_ca() {
    let root = tempfile::tempdir().unwrap();
    let (url, rx, ca) = tls_listener(root.path(), "200 OK", "{}");
    let home = home_for(root.path(), &url);
    for var in ["SSL_CERT_FILE", "SSL_CERT_DIR"] {
        let named = root.path().join(format!("{var}.env"));
        let value = if var == "SSL_CERT_FILE" { ca.display().to_string() } else { root.path().display().to_string() };
        std::fs::write(&named, format!("{var}={value}\n")).unwrap();
        let out = run(root.path(), &["--docbrain-env", named.to_str().unwrap(), "version"], &[("HOME", home.to_str().unwrap())]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success() && err.contains(&format!("sets {var}")) && err.contains("may set only"), "{var}:\n{err}");
    }
    let got = seen(&rx);
    assert!(got.is_empty(), "a CA from a named file was trusted:\n{got}");
}
