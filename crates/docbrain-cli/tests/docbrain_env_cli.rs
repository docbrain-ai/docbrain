// SPDX-License-Identifier: MIT
//! #603: the CLI reads no `.env` file unless `--docbrain-env` names one (owner rulings
//! 2026-10-23 and 2026-10-24). A `.env` in the directory a command runs in, or in any parent,
//! used to be loaded after the identity variables were captured, so it could not pick the
//! server or the key, but it could still set `HTTP_PROXY` and receive `Authorization: Bearer
//! <key>` (and a `login` password) through its own listener. A named file may set only
//! DocBrain's own variables that the CLI reads, never the server, the key or the profile.
//!
//! Every test runs the real binary with a cleared environment (HOME only), a scratch HOME
//! whose saved sign-in points at a local stub, and a recorder that any proxy variable would
//! send traffic to. All values are synthetic.

mod common;

use common::{ALLOWED_IN_THE_CLI, ALLOWED_IN_THE_CONNECTOR, REFUSED_EVERYWHERE, listener, seen};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_docbrain-cli");
/// What the stub answers on every path: for `/api/v1/instance` it is an instance holding 800
/// documents, which the seeded record says held 1000.
const INSTANCE: &str = r#"{"install_id":"SYNTH_INSTALL","documents":800}"#;
const DROP_WARNING: &str = "down from 1000";

/// A scratch HOME whose default sign-in and `reader` profile both point at `stub`, with a
/// repository folder (and a folder inside it) to run commands from.
struct Home {
    dir: tempfile::TempDir,
    stub: String,
}

impl Home {
    fn new(stub: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".docbrain")).unwrap();
        std::fs::write(
            dir.path().join(".docbrain/config.json"),
            format!(
                r#"{{"server_url":"{stub}","api_key":"SYNTH_SAVED_DEFAULT_KEY",
                    "profiles":{{"reader":{{"server_url":"{stub}","api_key":"SYNTH_SAVED_READER_KEY"}}}}}}"#
            ),
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("repo/sub")).unwrap();
        Home { dir, stub: stub.to_string() }
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn repo(&self) -> PathBuf {
        self.path().join("repo")
    }
    /// The repository root (holds the `.env`) and a folder inside it (holds none).
    fn cwds(&self) -> [PathBuf; 2] {
        [self.repo(), self.repo().join("sub")]
    }
    /// The record says the stub's instance held 1000 documents when last seen.
    fn seed_instance_record(&self) {
        std::fs::write(
            self.path().join(".docbrain/instances.json"),
            format!(r#"{{"{}":{{"install_id":"SYNTH_INSTALL","documents":1000}}}}"#, self.stub),
        )
        .unwrap();
    }
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.run_with(cwd, args, &[])
    }
    /// As `run`, with these variables set in the shell.
    fn run_with(&self, cwd: &Path, args: &[&str], shell: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(BIN);
        cmd.args(args).current_dir(cwd).env_clear().env("HOME", self.path()).stdin(Stdio::null());
        for (k, v) in shell {
            cmd.env(k, v);
        }
        cmd.output().expect("spawn docbrain-cli")
    }
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn all(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// Every proxy variable at the recorder, a CA file, and the server URL at the recorder too.
fn hostile_dotenv(dir: &Path, recorder: &str) {
    let ca = dir.join("ca.pem");
    std::fs::write(&ca, "-----BEGIN CERTIFICATE-----\nU1lOVEhFVElD\n-----END CERTIFICATE-----\n").unwrap();
    std::fs::write(
        dir.join(".env"),
        format!(
            "HTTP_PROXY={recorder}\nHTTPS_PROXY={recorder}\nALL_PROXY={recorder}\nSSL_CERT_FILE={}\nDOCBRAIN_SERVER_URL={recorder}\n",
            ca.display()
        ),
    )
    .unwrap();
}

/// The control the sentinel rests on: a proxy variable does send loopback traffic through the
/// proxy, so "the recorder received nothing" below means the variable was not set.
#[test]
fn a_proxy_the_shell_sets_does_route_loopback_traffic() {
    let (stub, stub_rx) = listener("500 Internal Server Error", "");
    let (proxy, proxy_rx) = listener("502 Bad Gateway", "");
    let home = Home::new(&stub);
    let out = home.run_with(&home.repo(), &["version"], &[("HTTP_PROXY", &proxy)]);
    let via = seen(&proxy_rx);
    assert!(via.contains(&format!("GET {stub}/api/v1/config")), "the shell's proxy was not used:\n{}\n{via}", stderr(&out));
    assert!(seen(&stub_rx).is_empty());
}

#[test]
fn a_dotenv_here_or_in_a_parent_never_reaches_the_cli() {
    let (stub, stub_rx) = listener("500 Internal Server Error", "");
    let (recorder, rec_rx) = listener("502 Bad Gateway", "");
    let home = Home::new(&stub);
    hostile_dotenv(&home.repo(), &recorder);
    for cwd in home.cwds() {
        let out = home.run(&cwd, &["version"]);
        let to_stub = seen(&stub_rx);
        assert!(
            to_stub.contains("GET /api/v1/config"),
            "version from {}: the saved server was not asked:\n{}\n{to_stub}",
            cwd.display(),
            stderr(&out)
        );
        let out = home.run(&cwd, &["ask", "hi"]);
        let to_stub = seen(&stub_rx);
        assert!(
            to_stub.contains("POST /api/v1/ask") && to_stub.contains("SYNTH_SAVED_DEFAULT_KEY"),
            "ask from {}: the saved server was not asked with the saved key:\n{}\n{to_stub}",
            cwd.display(),
            stderr(&out)
        );
    }
    let captured = seen(&rec_rx);
    assert!(captured.is_empty(), "a proxy named only by .env received traffic:\n{captured}");
}

/// The named file is loaded and nothing else is read. The variable that shows it is
/// `DOCBRAIN_CORPUS_DROP_ALERT`: at 0.5 the CLI stays quiet about a corpus that fell from
/// 1000 to 800 documents, at the default 0.10 it warns. The flag is global.
#[test]
fn docbrain_env_loads_the_named_file_and_only_it() {
    let (stub, _stub_rx) = listener("200 OK", INSTANCE);
    let home = Home::new(&stub);
    let quiet = home.path().join("quiet.env");
    std::fs::write(&quiet, "DOCBRAIN_CORPUS_DROP_ALERT=0.5\n").unwrap();
    let other = home.path().join("other.env");
    std::fs::write(&other, "DOCBRAIN_GENERATE_TIMEOUT_SECS=60\n").unwrap();
    let (quiet, other) = (quiet.to_str().unwrap(), other.to_str().unwrap());
    let warns = |args: &[&str], cwd: &Path| {
        home.seed_instance_record();
        let out = home.run(cwd, args);
        all(&out).contains(DROP_WARNING)
    };
    // The control: without the file, the fall is reported.
    assert!(warns(&["version"], &home.repo()), "the control did not warn");
    let joined = format!("--docbrain-env={quiet}");
    for args in [vec!["--docbrain-env", quiet, "version"], vec!["version", "--docbrain-env", quiet], vec![joined.as_str(), "version"]] {
        assert!(!warns(&args, &home.repo()), "{args:?}: the named file was not loaded");
    }
    // A .env here or in a parent says the same thing, and is not read, with or without a named file.
    std::fs::write(home.repo().join(".env"), "DOCBRAIN_CORPUS_DROP_ALERT=0.5\n").unwrap();
    for cwd in home.cwds() {
        assert!(warns(&["version"], &cwd), "from {}: a .env was read", cwd.display());
        assert!(warns(&["--docbrain-env", other, "version"], &cwd), "from {}: a .env was read beside the named file", cwd.display());
    }
}

/// A variable already set in the shell wins over the same variable in the named file.
#[test]
fn variables_the_shell_set_win_over_the_named_file() {
    let (stub, _stub_rx) = listener("200 OK", INSTANCE);
    let home = Home::new(&stub);
    let named = home.path().join("cli.env");
    std::fs::write(&named, "DOCBRAIN_CORPUS_DROP_ALERT=0.5\n").unwrap();
    home.seed_instance_record();
    let out = home.run_with(&home.repo(), &["--docbrain-env", named.to_str().unwrap(), "version"], &[("DOCBRAIN_CORPUS_DROP_ALERT", "0.05")]);
    assert!(all(&out).contains(DROP_WARNING), "the file overrode the shell:\n{}", all(&out));
}

/// A named file may set only DocBrain's own variables that the CLI reads (owner ruling
/// 2026-10-24, an allow-list), and never the server, the key or the profile. Any other variable
/// in it refuses the whole file before any variable is set or anything is sent, for `ask` and for
/// `login`, naming the variable as the file spells it, never the value or the path.
#[test]
fn a_named_file_may_set_only_the_cli_s_own_variables() {
    let (stub, stub_rx) = listener("500 Internal Server Error", "");
    let (proxy, proxy_rx) = listener("502 Bad Gateway", "");
    let home = Home::new(&stub);
    for (i, name) in REFUSED_EVERYWHERE.iter().chain(ALLOWED_IN_THE_CONNECTOR).enumerate() {
        let named = home.path().join(format!("cli-{i}.env"));
        std::fs::write(&named, format!("DOCBRAIN_CORPUS_DROP_ALERT=0.5\n{name}=SYNTH_REFUSED_VALUE\n")).unwrap();
        for cmd in [&["ask", "hi"][..], &["login", "--email", "a@b.c"][..]] {
            let mut args = vec!["--docbrain-env", named.to_str().unwrap()];
            args.extend_from_slice(cmd);
            let out = home.run_with(&home.repo(), &args, &[("http_proxy", &proxy), ("https_proxy", &proxy)]);
            let err = stderr(&out);
            assert!(!out.status.success(), "{name} {cmd:?}: a file that sets it was loaded:\n{err}");
            assert!(err.contains(&format!("sets {name}")) && err.contains("may set only"), "{name} {cmd:?}: the refusal should name it:\n{err}");
            assert!(!err.contains("Signing in") && !err.contains("Password"), "{name}: login went ahead:\n{err}");
            assert!(!err.contains("SYNTH_REFUSED_VALUE") && !err.contains(&named.display().to_string()), "{name}: echoed:\n{err}");
        }
    }
    let sent = format!("{}\n{}", seen(&stub_rx), seen(&proxy_rx));
    assert!(sent.trim().is_empty(), "a refused file let a request through:\n{sent}");
}

/// A file holding every variable the CLI allows is accepted, and applied (the corpus warning is
/// silenced by the file's threshold), so the list refuses nothing it should accept.
#[test]
fn every_variable_the_cli_allows_is_accepted() {
    let (stub, _stub_rx) = listener("200 OK", INSTANCE);
    let home = Home::new(&stub);
    let named = home.path().join("cli.env");
    let body: String = ALLOWED_IN_THE_CLI
        .iter()
        .map(|n| match *n {
            "DOCBRAIN_CORPUS_DROP_ALERT" => format!("{n}=0.5\n"),
            "DOCBRAIN_GENERATE_TIMEOUT_SECS" => format!("{n}=60\n"),
            other => panic!("{other}: give it a value"),
        })
        .collect();
    std::fs::write(&named, body).unwrap();
    home.seed_instance_record();
    let out = home.run(&home.repo(), &["--docbrain-env", named.to_str().unwrap(), "version"]);
    assert!(out.status.success(), "{}", all(&out));
    assert!(!all(&out).contains(DROP_WARNING), "the file was not applied:\n{}", all(&out));
}

/// A value holding a NUL byte cannot be put in the environment. It is refused like a bad line,
/// without the value, never as a panic (whose message would print it).
#[test]
fn a_value_with_a_nul_byte_is_refused_without_showing_it() {
    let (stub, stub_rx) = listener("500 Internal Server Error", "");
    let home = Home::new(&stub);
    let named = home.path().join("nul.env");
    std::fs::write(&named, "DOCBRAIN_CORPUS_DROP_ALERT=SYNTH_NUL_VALUE\0tail\n").unwrap();
    let out = home.run(&home.repo(), &["--docbrain-env", named.to_str().unwrap(), "ask", "hi"]);
    let err = stderr(&out);
    assert!(!out.status.success() && err.contains("NUL") && !err.contains("panicked"), "{err}");
    assert!(!err.contains("SYNTH_NUL_VALUE"), "the value was shown:\n{err}");
    assert!(seen(&stub_rx).is_empty());
}

/// A variable the shell set to an empty value counts as unset, so the named file fills it.
#[test]
fn an_empty_shell_value_does_not_cancel_the_named_file() {
    let (stub, _stub_rx) = listener("200 OK", INSTANCE);
    let home = Home::new(&stub);
    let named = home.path().join("cli.env");
    std::fs::write(&named, "DOCBRAIN_CORPUS_DROP_ALERT=0.5\n").unwrap();
    home.seed_instance_record();
    let out = home.run_with(&home.repo(), &["--docbrain-env", named.to_str().unwrap(), "version"], &[("DOCBRAIN_CORPUS_DROP_ALERT", "")]);
    assert!(!all(&out).contains(DROP_WARNING), "the empty shell value cancelled the file:\n{}", all(&out));
}

/// With no HOME in the shell, a named file that sets one would pick the saved sign-in (and the
/// server a `login` password goes to): it is refused like any other denied name.
///
/// Without HOME the CLI falls back to the account's real home directory, so a broken build
/// could read the real saved sign-in. Every request is therefore sent through a local recorder
/// by the shell's own proxy variables, and what it records is never printed.
#[test]
fn a_named_file_may_not_choose_home_when_the_shell_has_none() {
    let (theirs, theirs_rx) = listener("500 Internal Server Error", "");
    let (guard, guard_rx) = listener("502 Bad Gateway", "");
    let scratch = tempfile::tempdir().unwrap();
    let fake = Home::new(&theirs);
    let named = scratch.path().join("home.env");
    std::fs::write(&named, format!("HOME={}\n", fake.path().display())).unwrap();
    for cmd in [&["ask", "hi"][..], &["login", "--email", "a@b.c"][..]] {
        let mut args = vec!["--docbrain-env", named.to_str().unwrap()];
        args.extend_from_slice(cmd);
        let out = Command::new(BIN)
            .args(&args)
            .current_dir(scratch.path())
            .env_clear()
            .env("HTTP_PROXY", &guard)
            .env("HTTPS_PROXY", &guard)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let err = stderr(&out);
        assert!(!out.status.success() && err.contains("sets HOME") && err.contains("may set only"), "{cmd:?}:\n{err}");
        assert!(!err.contains("Signing in") && !err.contains("Password"), "{cmd:?}: login went ahead");
    }
    let to_theirs = seen(&theirs_rx);
    let to_guard = seen(&guard_rx);
    assert!(to_theirs.is_empty() && to_guard.is_empty(), "a request left the CLI (its content is not printed)");
}

/// A file that was asked for and cannot be used stops the command before anything is sent:
/// running without it would be running on settings nobody chose. No refusal shows a value given
/// on the command line, a path included.
#[test]
fn docbrain_env_refusals_send_nothing() {
    let (stub, stub_rx) = listener("500 Internal Server Error", "");
    let (recorder, rec_rx) = listener("502 Bad Gateway", "");
    let home = Home::new(&stub);
    let dir = home.repo();
    std::fs::write(dir.join("relative.env"), format!("HTTP_PROXY={recorder}\n")).unwrap();
    let malformed = home.path().join("malformed.env");
    std::fs::write(&malformed, "SYNTH_SECRET_VALUE_42 is not an assignment\n").unwrap();
    let missing = home.path().join("SYNTH_MISSING_PATH.env").display().to_string();
    let malformed = malformed.display().to_string();
    let other = home.path().join("other.env");
    std::fs::write(&other, "\n").unwrap();
    let other = other.display().to_string();
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (vec!["--docbrain-env", "relative.env", "ask", "hi"], "absolute path"),
        (vec!["--docbrain-env", "relative.env", "version"], "absolute path"),
        (vec!["--docbrain-env", "SYNTH_PASTED_KEY", "ask", "hi"], "absolute path"),
        (vec!["--docbrain-env", &missing, "ask", "hi"], "cannot read"),
        (vec!["--docbrain-env", &malformed, "ask", "hi"], "not NAME=value"),
        (vec!["--docbrain-env", &other, "ask", "hi", "--docbrain-env", &missing], "more than once"),
    ];
    for (args, says) in &cases {
        let out = home.run(&dir, args);
        let err = stderr(&out);
        assert!(!out.status.success(), "{args:?} ran:\n{err}");
        assert!(err.contains(says), "{args:?} should say {says:?}:\n{err}");
        for secret in ["SYNTH_SECRET_VALUE_42", "SYNTH_PASTED_KEY", "SYNTH_MISSING_PATH", "relative.env", "other.env"] {
            assert!(!err.contains(secret), "{args:?} echoed {secret}:\n{err}");
        }
    }
    let sent = format!("{}\n{}", seen(&stub_rx), seen(&rec_rx));
    assert!(sent.trim().is_empty(), "a refused command sent a request:\n{sent}");
}

/// A command line the CLI cannot parse is refused without showing what it could not parse: a
/// key typed in the wrong place (a stray argument, a value for a flag that takes a number, a
/// mistyped flag with `=`) must not reach a terminal log or a CI log. A flag's own name is
/// shown, since it is what the reader needs to fix the command.
#[test]
fn an_argument_the_cli_cannot_parse_is_never_shown() {
    let (stub, stub_rx) = listener("500 Internal Server Error", "");
    let home = Home::new(&stub);
    for args in [
        &["version", "SYNTH_PASTED_KEY_ARG"][..],
        &["--api-kye=SYNTH_PASTED_KEY_ARG", "version"][..],
        // One argument holding a flag and its value, as an exec-form list writes it.
        &["--api-key SYNTH_PASTED_KEY_ARG", "version"][..],
        &["thumbsdown", "--source", "SYNTH_PASTED_KEY_ARG"][..],
        &["--env-file", "/SYNTH_PASTED_KEY_ARG.env", "version"][..],
        &["--env-file=/SYNTH_PASTED_KEY_ARG.env", "version"][..],
    ] {
        let out = home.run(&home.repo(), args);
        let err = all(&out);
        assert_eq!(out.status.code(), Some(2), "{args:?}:\n{err}");
        assert!(!err.contains("SYNTH_PASTED_KEY_ARG"), "{args:?} echoed the value:\n{err}");
    }
    // A value the CLI checks itself (a profile name, a file it is asked to read) is not shown
    // either when it is refused.
    for args in [
        &["--profile", "SYNTH_PASTED_KEY_ARG_THAT_IS_NOT_A_NAME", "ask", "hi"][..],
        &["--profile", "synthpastedkeyarg", "ask", "hi"][..],
        &["health", "validate", "/SYNTH_PASTED_KEY_ARG.json"][..],
        &["evidence", "verify", "/SYNTH_PASTED_KEY_ARG.dbev"][..],
        &["check-claims", "--diff", "/SYNTH_PASTED_KEY_ARG.patch"][..],
        &["generate", "a runbook", "--source", "/SYNTH_PASTED_KEY_ARG.md"][..],
    ] {
        let out = home.run(&home.repo(), args);
        let err = all(&out);
        assert!(!out.status.success(), "{args:?}:\n{err}");
        assert!(!err.contains("SYNTH_PASTED_KEY_ARG") && !err.contains("synthpastedkeyarg"), "{args:?} echoed the value:\n{err}");
    }
    let out = home.run(&home.repo(), &["--env-file", "/x.env", "version"]);
    assert!(all(&out).contains("--docbrain-env"), "the refusal should point at --docbrain-env:\n{}", all(&out));
    let out = home.run(&home.repo(), &["--api-kye=SYNTH_PASTED_KEY_ARG", "version"]);
    assert!(all(&out).contains("--api-kye"), "a mistyped flag's name should be shown:\n{}", all(&out));
    assert!(seen(&stub_rx).is_empty());
}

/// `login --github` with this environment, in a scratch HOME whose sign-in points at a dead
/// server. It never opens a browser: on macOS the run is sandboxed so `/usr/bin/open` cannot
/// start; elsewhere PATH is empty, so no opener is found. The login waits for the browser, so
/// it is killed once its start URL is read (or once it exits). Returns (exit status if it
/// exited by itself, the start URL line if printed, stderr).
fn sso_login(env: &[(&str, &str)]) -> (Option<std::process::ExitStatus>, Option<String>, String) {
    use std::io::{BufRead, BufReader, Read};
    let home = Home::new("http://127.0.0.1:9");
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("/usr/bin/sandbox-exec");
        c.args(["-p", "(version 1)(allow default)(deny process-exec (literal \"/usr/bin/open\"))", BIN]);
        c
    } else {
        Command::new(BIN)
    };
    cmd.args(["login", "--github"]).current_dir(home.path()).env_clear().env("HOME", home.path()).env("PATH", home.path());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn login");
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut url = None;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if line.contains("cli_callback=") {
            url = Some(line.trim().to_string());
            break;
        }
    }
    let exited = child.try_wait().unwrap();
    if exited.is_none() {
        let _ = child.kill();
    }
    let status = child.wait().unwrap();
    let mut err = String::new();
    let _ = child.stderr.take().unwrap().read_to_string(&mut err);
    (if url.is_some() { exited } else { Some(status) }, url, err)
}

/// The port in a start URL's `cli_callback=http://HOST:PORT`.
fn callback_port(url: &str) -> u16 {
    let cb = url.split("cli_callback=").nth(1).unwrap();
    cb.rsplit(':').next().unwrap().trim().parse().unwrap()
}

/// The login callback host comes only from the shell, is only a bare loopback IP, and the CLI
/// listens on exactly that address (owner ruling 2026-10-24, review round 3): the one-time code
/// the browser brings back can only reach the CLI's own listener.
#[test]
fn the_login_callback_is_a_loopback_ip_the_cli_listens_on() {
    use std::net::TcpStream;
    // The default: 127.0.0.1, listened on there.
    let (_, url, err) = sso_login(&[]);
    let url = url.unwrap_or_else(|| panic!("no start URL:\n{err}"));
    assert!(url.contains("cli_callback=http://127.0.0.1:"), "{url}");
    // IPv6 loopback, where the machine has it: the URL names [::1], and the CLI listens there
    // and not on 127.0.0.1. Probed while that login is still waiting.
    if std::net::TcpListener::bind(("::1", 0)).is_ok() {
        use std::io::{BufRead, BufReader};
        let home = Home::new("http://127.0.0.1:9");
        let mut cmd = if cfg!(target_os = "macos") {
            let mut c = Command::new("/usr/bin/sandbox-exec");
            c.args(["-p", "(version 1)(allow default)(deny process-exec (literal \"/usr/bin/open\"))", BIN]);
            c
        } else {
            Command::new(BIN)
        };
        let mut child = cmd
            .args(["login", "--github"])
            .current_dir(home.path())
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", home.path())
            .env("DOCBRAIN_CLI_CALLBACK_HOST", "::1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let mut url = None;
        let mut line = String::new();
        for _ in 0..40 {
            line.clear();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            if line.contains("cli_callback=") {
                url = Some(line.trim().to_string());
                break;
            }
        }
        let url = url.expect("no start URL for ::1");
        let port = callback_port(&url);
        let on_v6 = TcpStream::connect(("::1", port)).is_ok();
        let on_v4 = TcpStream::connect(("127.0.0.1", port)).is_ok();
        let _ = child.kill();
        let _ = child.wait();
        // The brackets travel percent-encoded in the query; the server decodes them back to
        // `http://[::1]:<port>`.
        assert!(url.contains("cli_callback=http://%5B::1%5D:"), "{url}");
        assert!(on_v6, "the CLI does not listen on the address it advertised");
        assert!(!on_v4, "the CLI also listens on 127.0.0.1, which it did not advertise");
    }
    // Anything but a bare loopback IP is refused before a start URL is printed, unshown.
    for bad in [
        "127.0.0.1:8080?", "127.0.0.1:8080/x", "[::1]", "localhost", "0.0.0.0", "10.0.0.1", "127.1",
        "SYNTHcollector.example", "evil.example@127.0.0.1", " 127.0.0.1", "::ffff:127.0.0.1",
    ] {
        let (status, url, err) = sso_login(&[("DOCBRAIN_CLI_CALLBACK_HOST", bad)]);
        assert!(url.is_none(), "{bad:?}: a start URL was printed: {url:?}");
        assert!(status.is_some_and(|s| !s.success()), "{bad:?}: not refused:\n{err}");
        assert!(err.contains("DOCBRAIN_CLI_CALLBACK_HOST") && err.contains("loopback"), "{bad:?}:\n{err}");
        if !["127.0.0.1", "::1"].contains(&bad.trim()) {
            assert!(!err.contains(bad.trim()), "{bad:?}: echoed:\n{err}");
        }
    }
}

/// A file saved with a UTF-8 byte-order mark loads like the same file without one.
#[test]
fn a_file_with_a_byte_order_mark_loads() {
    let (stub, _stub_rx) = listener("200 OK", INSTANCE);
    let home = Home::new(&stub);
    let named = home.path().join("bom.env");
    std::fs::write(&named, "\u{feff}DOCBRAIN_CORPUS_DROP_ALERT=0.5\n").unwrap();
    home.seed_instance_record();
    let out = home.run(&home.repo(), &["--docbrain-env", named.to_str().unwrap(), "version"]);
    assert!(out.status.success(), "{}", all(&out));
    assert!(!all(&out).contains(DROP_WARNING), "the file was not applied:\n{}", all(&out));
}

/// A `$` in the file is refused: the parser would replace `$NAME` or `${NAME}` with another
/// variable from the environment (the shell's key, say), so an allowed name could carry it.
#[test]
fn a_file_that_would_read_another_variable_is_refused() {
    let (stub, stub_rx) = listener("500 Internal Server Error", "");
    let home = Home::new(&stub);
    let shell = [("DOCBRAIN_API_KEY", "SYNTH_SHELL_KEY_77")];
    for (i, line) in ["DOCBRAIN_CORPUS_DROP_ALERT=${DOCBRAIN_API_KEY}", "DOCBRAIN_CORPUS_DROP_ALERT=\"$DOCBRAIN_API_KEY\"", "DOCBRAIN_CORPUS_DROP_ALERT='$SYNTH_LITERAL'"].iter().enumerate() {
        let named = home.path().join(format!("subst-{i}.env"));
        std::fs::write(&named, format!("{line}\n")).unwrap();
        let out = home.run_with(&home.repo(), &["--docbrain-env", named.to_str().unwrap(), "ask", "hi"], &shell);
        let err = stderr(&out);
        assert!(!out.status.success() && err.contains('$'), "{line}: loaded:\n{err}");
        assert!(!err.contains("SYNTH_SHELL_KEY_77") && !err.contains("DOCBRAIN_API_KEY}"), "{line}: echoed:\n{err}");
    }
    assert!(seen(&stub_rx).is_empty(), "a refused file let a request through");
}

/// `DOCBRAIN_GENERATE_TIMEOUT_SECS` from the named file takes effect: with 30 (its floor) a
/// server that never answers the generate request stops the command after about 30 seconds,
/// where the default would wait 900.
#[test]
fn the_named_file_sets_the_generate_timeout() {
    use std::io::Write;
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let stub = format!("http://{}", l.local_addr().unwrap());
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in l.incoming() {
            let Ok(mut s) = stream else { break };
            s.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
            let req = common::read_request(&mut s);
            if req.contains("/api/v1/generate") {
                held.push(s); // never answered
            } else {
                let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
            }
        }
    });
    let home = Home::new(&stub);
    let named = home.path().join("cli.env");
    std::fs::write(&named, "DOCBRAIN_GENERATE_TIMEOUT_SECS=30\n").unwrap();
    let started = std::time::Instant::now();
    let mut child = Command::new(BIN)
        .args(["--docbrain-env", named.to_str().unwrap(), "generate", "a runbook"])
        .current_dir(home.repo())
        .env_clear()
        .env("HOME", home.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break Some(st);
        }
        if started.elapsed() > std::time::Duration::from_secs(90) {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    let mut err = String::new();
    let _ = std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut err);
    let status = status.unwrap_or_else(|| panic!("still waiting after 90 s: the file's timeout was not applied\n{err}"));
    assert!(!status.success() && err.contains("timed out"), "{err}");
    assert!(started.elapsed() >= std::time::Duration::from_secs(25), "stopped before the timeout: {err}");
}

/// A path the CLI was asked to read or write, a profile name a login was asked to create, and
/// a flag with a value glued to it are not shown when the command fails: any of them may be a
/// key typed in the wrong place.
#[test]
fn what_was_typed_is_not_shown_when_a_command_fails() {
    use std::os::unix::ffi::OsStrExt;
    let attest = r#"{"export":"SYNTH_EXPORT","computed_at":"t","install_id":"i"}"#;
    let artifact = r##"{"markdown":"# Title","doc_type":"runbook","quality":{"score":0.9,"violations":[],"has_error_severity":false}}"##;
    let (attest_stub, _a) = listener("200 OK", attest);
    let (generate_stub, _g) = listener("200 OK", artifact);
    let (dead_stub, _d) = listener("500 Internal Server Error", "");
    let a = Home::new(&attest_stub);
    let g = Home::new(&generate_stub);
    let d = Home::new(&dead_stub);
    let not_utf8 = g.path().join("SYNTH_PASTED_KEY_ARG.md");
    std::fs::write(&not_utf8, b"\xff\xfe not text").unwrap();
    let not_utf8 = std::ffi::OsStr::from_bytes(not_utf8.as_os_str().as_bytes()).to_str().unwrap().to_string();
    for (home, args) in [
        (&a, vec!["license", "attest", "--export", "-o", "/nonexistent-dir/SYNTH_PASTED_KEY_ARG.json"]),
        (&g, vec!["generate", "a runbook", "--out", "/nonexistent-dir/SYNTH_PASTED_KEY_ARG.md"]),
        (&g, vec!["generate", "a runbook", "--target", not_utf8.as_str()]),
        (&d, vec!["--profile", "synthpastedkeyarg", "login", "--email", "a@b.c"]),
        (&d, vec!["--api-keySYNTH_PASTED_KEY_ARG", "ask", "hi"]),
        // Lower case only, so only the "glued onto a real flag" rule hides it.
        (&d, vec!["--api-keysynthpastedkeyarg", "ask", "hi"]),
        // Not glued onto any flag, but not a flag name either.
        (&d, vec!["--SYNTH_PASTED_KEY_ARG", "ask", "hi"]),
    ] {
        let out = home.run(&home.repo(), &args);
        let err = all(&out);
        assert!(!out.status.success(), "{args:?}:\n{err}");
        assert!(!err.contains("SYNTH_PASTED_KEY_ARG") && !err.contains("synthpastedkeyarg"), "{args:?} echoed the value:\n{err}");
    }
}

/// R4-F1: clap's tips print a whole argument unquoted ("to pass '<arg>' as a value, use
/// '-- <arg>'"), so a key glued onto a flag after a subcommand reached the tip line. Every clap
/// error a command line can produce, with a key-shaped synthetic value in every position: before
/// the subcommand, after it, after its positional, glued onto a real flag, glued onto a mistyped
/// one, after `=`, as a flag's value, and as the command itself. No part of the key may appear.
#[test]
fn a_key_typed_anywhere_never_appears_in_a_command_line_error() {
    use std::os::unix::ffi::OsStrExt;
    let (dead_stub, _d) = listener("500 Internal Server Error", "");
    let home = Home::new(&dead_stub);
    // Key-shaped (a prefix and 32 characters): one with upper case, one lower case only, so no
    // rule about what a flag name looks like can be what hides it.
    let mixed = "db_sk_SynthKey0123456789abcdefSYNTHk";
    let lower = "db_sk_synthkeyzyxwvutsrqponmlkjihgfe";
    for key in [mixed, lower] {
        let dashed = format!("--{key}");
        let short = format!("-{key}");
        let out_glued = format!("--out{key}");
        let typo_glued = format!("--api-kye{key}");
        let key_glued = format!("--api-key{key}");
        let typo_eq = format!("--api-kye={key}");
        let json_eq = format!("--json={key}");
        let profile_eq = format!("--profile={key}");
        // One argument holding a flag and its value, as an exec-form list writes it.
        let spaced = format!("--api-key {key}");
        let cases: Vec<Vec<&str>> = vec![
            // unexpected argument, before and after the subcommand and its positional
            vec![&dashed, "ask", "hi"],
            vec!["ask", &dashed, "hi"],
            vec!["ask", "hi", &dashed],
            vec!["ask", "hi", &short],
            vec!["ask", "hi", key],
            vec!["ask", "hi", "--", key],
            // glued onto a real flag and onto a mistyped one, before and after the subcommand
            vec!["generate", "a runbook", &out_glued],
            vec![&key_glued, "ask", "hi"],
            vec!["ask", "hi", &key_glued],
            vec!["ask", "hi", &typo_glued],
            vec![&typo_glued, "ask", "hi"],
            vec!["ask", "hi", &typo_eq],
            vec![&spaced, "ask", "hi"],
            vec!["ask", "hi", &spaced],
            // an unknown command
            vec![key, "ask"],
            vec!["health", key],
            // an invalid value, a value that does not parse, a value for a flag that takes none
            vec!["health", "findings", "--format", key],
            vec!["generate", "a runbook", "--max-regen-rounds", key],
            vec!["ask", "hi", &json_eq],
            // a conflict, a repeat and a missing value, with the key as a flag's value
            vec!["--api-key", key, "--profile", "reader", "ask", "hi"],
            vec![&profile_eq, "--api-key", key, "ask", "hi"],
            vec!["ask", "hi", "--session", key, "--session", key],
            vec!["--api-key", key, "ask"],
        ];
        for args in &cases {
            let out = home.run(&home.repo(), args);
            let err = all(&out);
            assert_eq!(out.status.code(), Some(2), "{args:?} is a command-line error:\n{err}");
            assert!(err.contains("error:"), "{args:?}:\n{err}");
            for part in [key, &key[6..], &key[..20], &key[16..]] {
                assert!(!err.contains(part), "{args:?} echoed {part:?}:\n{err}");
            }
        }
        // Not UTF-8: clap's own refusal, still without the key.
        let mut bytes = b"\xff".to_vec();
        bytes.extend_from_slice(key.as_bytes());
        let out = Command::new(BIN)
            .args([std::ffi::OsStr::new("ask"), std::ffi::OsStr::new("hi"), std::ffi::OsStr::from_bytes(&bytes)])
            .current_dir(home.repo())
            .env_clear()
            .env("HOME", home.path())
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let err = all(&out);
        assert!(!out.status.success() && !err.contains(&key[6..]), "not UTF-8:\n{err}");
    }

    // What the reader needs survives: the flag a typo was close to, and the pointer away from
    // Node's own file option, neither naming what was typed.
    let out = home.run(&home.repo(), &["ask", "hi", "--api-kye", mixed]);
    let err = all(&out);
    assert!(err.contains("'--api-key'"), "the similar-flag tip names the real flag:\n{err}");
    assert!(!err.contains(&mixed[6..]), "{err}");
    let out = home.run(&home.repo(), &["--env-file", "/nonexistent/SYNTHKEYPATH", "ask", "hi"]);
    let err = all(&out);
    assert!(err.contains("the CLI's file option is --docbrain-env"), "{err}");
    assert!(!err.contains("SYNTHKEYPATH"), "{err}");
}
