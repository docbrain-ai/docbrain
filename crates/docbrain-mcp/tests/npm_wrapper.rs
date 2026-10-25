// SPDX-License-Identifier: MIT
//! #603, owner ruling 2026-10-24: the npm wrapper (`npm-mcp/run.js`, what `npm install -g
//! docbrain-mcp` and `npx docbrain-mcp` start) never passes on an option Node acts on itself.
//! Node reads `--env-file`, `--env-file-if-exists`, `--experimental-config-file` and
//! `--experimental-default-config-file` anywhere in its own argv, even after the script path,
//! before any line of the wrapper runs; Node 20.6-22.6 and 23.10-24.11 match the first and last
//! by prefix and do not all stop at `--`, so the wrapper refuses any argument that starts with
//! one of the four, wherever it is. The wrapper cannot undo that, but it must never start
//! the connector with one of them. `--docbrain-env`, which Node ignores, passes through.
//!
//! The wrapper starts `bin/docbrain-mcp`; here that is a script that records it was started and
//! then runs the real binary, so "the wrapper refused" is told apart from "the binary refused".

mod common;

use common::{listener, seen};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_docbrain-mcp");
const ME: &str = r#"{"name":"synthetic","role":"viewer"}"#;
const NODE_READS: &[&str] = &["--env-file", "--env-file-if-exists", "--experimental-config-file", "--experimental-default-config-file"];

/// The wrapper's source, or `None` in the public repository, which carries this crate but not
/// the npm packages (they are published from the private tree, where these tests run).
fn wrapper_source() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !root.join("crates/docbrain-server").exists() {
        return None;
    }
    let src = root.join("npm-mcp/run.js");
    assert!(src.exists(), "npm-mcp/run.js is missing from the private tree");
    Some(src)
}

fn node() -> PathBuf {
    std::env::var_os("PATH")
        .and_then(|p| std::env::split_paths(&p).map(|d| d.join("node")).find(|n| n.is_file()))
        .expect("`node` must be on PATH for the npm wrapper tests")
}

/// A copy of the wrapper next to a `bin/docbrain-mcp` that writes `started` to `marker`.
#[cfg(unix)]
fn package(dir: &Path, src: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let pkg = dir.join("pkg");
    std::fs::create_dir_all(pkg.join("bin")).unwrap();
    std::fs::copy(src, pkg.join("run.js")).unwrap();
    let marker = dir.join("started");
    let shim = pkg.join("bin/docbrain-mcp");
    std::fs::write(&shim, format!("#!/bin/sh\necho started >> '{}'\nexec '{BIN}' \"$@\"\n", marker.display())).unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    (pkg.join("run.js"), marker)
}

fn run_wrapper(cwd: &Path, run_js: &Path, args: &[String], env: &[(&str, &str)]) -> Output {
    run_node(cwd, &[run_js.as_os_str()], args, env)
}

/// As `run_wrapper`, but the arguments are put in `process.argv` by a one-line script, so Node
/// itself never sees them: this checks the wrapper's own refusal of each raw form, whatever a
/// given Node version reads or rewrites (Node 26 rewrites one of these options before the
/// wrapper runs).
fn run_wrapper_unseen(cwd: &Path, run_js: &Path, args: &[String], env: &[(&str, &str)]) -> Output {
    let js = serde_json::to_string(run_js.to_str().unwrap()).unwrap();
    let script = format!("process.argv = [process.argv[0], {js}].concat({}); require({js});", serde_json::to_string(args).unwrap());
    run_node(cwd, &[std::ffi::OsStr::new("-e"), std::ffi::OsStr::new(&script)], &[], env)
}

fn run_node(cwd: &Path, first: &[&std::ffi::OsStr], args: &[String], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(node());
    cmd.args(first).args(args).current_dir(cwd).env_clear().stdin(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn node")
}

#[cfg(unix)]
#[test]
fn the_wrapper_never_starts_the_connector_with_an_option_node_acts_on() {
    let Some(src) = wrapper_source() else { return };
    let (stub, stub_rx) = listener("200 OK", ME);
    let dir = tempfile::tempdir().unwrap();
    let (run_js, marker) = package(dir.path(), &src);
    // Empty files, so that Node's own reading succeeds and the wrapper is what decides.
    let cwd = dir.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(cwd.join("node.config.json"), "{}").unwrap();
    let empty_env = dir.path().join("SYNTH_EMPTY.env");
    std::fs::write(&empty_env, "").unwrap();
    let empty_config = dir.path().join("SYNTH_EMPTY.json");
    std::fs::write(&empty_config, "{}").unwrap();
    let host = [("DOCBRAIN_API_KEY", "SYNTH_HOST_KEY"), ("DOCBRAIN_SERVER_URL", stub.as_str())];
    let mut cases: Vec<Vec<String>> = Vec::new();
    for opt in NODE_READS {
        let file = if opt.contains("config") { &empty_config } else { &empty_env };
        if *opt == "--experimental-default-config-file" {
            cases.push(vec![opt.to_string()]);
        } else {
            cases.push(vec![opt.to_string(), file.display().to_string()]);
            cases.push(vec![format!("{opt}={}", file.display())]);
        }
        // After another argument, and as if it were another flag's value: Node still acts.
        // (`--experimental-default-config-file` takes no value; Node refuses one itself.)
        let late = if *opt == "--experimental-default-config-file" { opt.to_string() } else { format!("{opt}={}", file.display()) };
        cases.push(vec!["--frobnicate".into(), late]);
    }
    // Spellings older Node lines act on because they match by prefix.
    for odd in ["--env-files=/x.env", "--env-filex", "--experimental-default-config-filex", "--experimental-config-file-x=/x.json"] {
        cases.push(vec![odd.to_string()]);
        cases.push(vec!["--".to_string(), odd.to_string()]);
    }
    for (args, raw) in cases.iter().flat_map(|a| [(a, false), (a, true)]) {
        let _ = std::fs::remove_file(&marker);
        let out = if raw { run_wrapper_unseen(&cwd, &run_js, args, &host) } else { run_wrapper(&cwd, &run_js, args, &host) };
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        let args = &format!("{args:?} (Node reading them: {})", !raw);
        assert!(!marker.exists(), "{args:?}: the wrapper started the connector:\n{err}");
        assert!(!out.status.success(), "{args:?}: exit 0:\n{err}");
        assert!(err.contains("--docbrain-env"), "{args:?}: the refusal should name --docbrain-env:\n{err}");
        assert!(!err.contains("SYNTH_EMPTY"), "{args:?}: echoed a value:\n{err}");
    }
    assert!(seen(&stub_rx).is_empty(), "a refused start sent a request");
}

#[cfg(unix)]
#[test]
fn the_wrapper_passes_docbrain_env_through_and_echoes_nothing() {
    let Some(src) = wrapper_source() else { return };
    let (stub, stub_rx) = listener("200 OK", ME);
    let dir = tempfile::tempdir().unwrap();
    let (run_js, marker) = package(dir.path(), &src);
    let named = dir.path().join("connector.env");
    std::fs::write(&named, format!("DOCBRAIN_API_KEY=SYNTH_NAMED_KEY\nDOCBRAIN_SERVER_URL={stub}\n")).unwrap();
    for args in [
        vec!["--docbrain-env".to_string(), named.display().to_string()],
        vec![format!("--docbrain-env={}", named.display())],
    ] {
        let _ = std::fs::remove_file(&marker);
        let out = run_wrapper(dir.path(), &run_js, &args, &[]);
        assert!(out.status.success() && marker.exists(), "{args:?}:\n{}", String::from_utf8_lossy(&out.stderr));
        assert!(seen(&stub_rx).contains("SYNTH_NAMED_KEY"), "{args:?}: the named file was not loaded");
    }
    // A key pasted as the value: Node ignores the flag, the connector refuses without echoing.
    let out = run_wrapper(dir.path(), &run_js, &["--docbrain-env".into(), "SYNTH_PASTED_KEY".into()], &[]);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!out.status.success() && !err.contains("SYNTH_PASTED_KEY"), "{err}");
}

/// Node 26 stops reading `--env-file` at `--`, but older Node lines do not, and every line reads
/// the config-file options after it: the wrapper refuses all four wherever they are. What is
/// not one of them still reaches the connector after `--`.
#[cfg(unix)]
#[test]
fn node_options_are_refused_even_after_a_double_dash() {
    let Some(src) = wrapper_source() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (run_js, marker) = package(dir.path(), &src);
    let config = dir.path().join("SYNTH_EMPTY.json");
    std::fs::write(&config, "{}").unwrap();
    std::fs::write(dir.path().join("node.config.json"), "{}").unwrap();
    for late in [
        "--env-file=/nonexistent.env".to_string(),
        format!("--experimental-config-file={}", config.display()),
        "--experimental-default-config-file".to_string(),
    ] {
        let _ = std::fs::remove_file(&marker);
        let out = run_wrapper(dir.path(), &run_js, &["--".into(), late.clone()], &[]);
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(!marker.exists() && err.contains("--docbrain-env"), "{late}: passed on after --:\n{err}");
    }
    let _ = std::fs::remove_file(&marker);
    let out = run_wrapper(dir.path(), &run_js, &["--".into(), "--frobnicate".into()], &[]);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(marker.exists(), "the wrapper refused an argument Node does not read:\n{err}");
}
