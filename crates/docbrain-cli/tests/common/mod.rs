// SPDX-License-Identifier: MIT
//! Local listeners for the `--docbrain-env` tests (#603): a plain HTTP stub or recorder, and a
//! TLS stub whose certificate is signed by a CA made for the run (so no key is ever committed).
//! All values are synthetic.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

/// A listener that records each request it receives (request line, headers, body) and answers
/// it with `status` and `body`. The record is sent before the answer, so it is in the channel
/// by the time the client has its response.
pub fn listener(status: &'static str, body: &'static str) -> (String, Receiver<String>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for stream in l.incoming() {
            let Ok(mut s) = stream else { break };
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let _ = tx.send(read_request(&mut s));
            let _ = s.write_all(answer(status, body).as_bytes());
        }
    });
    (url, rx)
}

fn answer(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

pub fn read_request<S: Read>(s: &mut S) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
            let len = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() >= end + 4 + len {
                break;
            }
        }
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

pub fn seen(rx: &Receiver<String>) -> String {
    rx.try_iter().collect::<Vec<_>>().join("\n")
}

/// A CA, and a certificate for 127.0.0.1 signed by it, made with the `openssl` command in `dir`.
/// Returns the CA certificate, the server certificate and the server key (PKCS#8).
pub fn make_ca(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let run = |args: &[&str]| {
        let out = Command::new("openssl")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("the `openssl` command is needed to make the test CA");
        assert!(out.status.success(), "openssl {args:?}:\n{}", String::from_utf8_lossy(&out.stderr));
    };
    std::fs::write(dir.join("server.ext"), "subjectAltName=IP:127.0.0.1\nbasicConstraints=CA:FALSE\n").unwrap();
    run(&[
        "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "ca.key", "-out", "ca.pem", "-days", "2",
        "-subj", "/CN=DocBrain test CA", "-addext", "basicConstraints=critical,CA:TRUE",
        "-addext", "keyUsage=critical,keyCertSign,cRLSign",
    ]);
    run(&["req", "-newkey", "rsa:2048", "-nodes", "-keyout", "server.key", "-out", "server.csr", "-subj", "/CN=127.0.0.1"]);
    run(&[
        "x509", "-req", "-in", "server.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial",
        "-out", "server.pem", "-days", "2", "-extfile", "server.ext",
    ]);
    (dir.join("ca.pem"), dir.join("server.pem"), dir.join("server.key"))
}

/// An HTTPS stub on 127.0.0.1 whose certificate only the CA in `dir` vouches for. It records
/// each request that completes the TLS handshake; a client that does not trust the CA aborts the
/// handshake and nothing is recorded. Returns the `https://` URL, the record and the CA file.
pub fn tls_listener(dir: &Path, status: &'static str, body: &'static str) -> (String, Receiver<String>, PathBuf) {
    let (ca, cert, key) = make_ca(dir);
    let identity = native_tls::Identity::from_pkcs8(&std::fs::read(cert).unwrap(), &std::fs::read(key).unwrap())
        .expect("load the test server identity");
    let acceptor = native_tls::TlsAcceptor::new(identity).expect("build the test TLS acceptor");
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("https://127.0.0.1:{}", l.local_addr().unwrap().port());
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for stream in l.incoming() {
            let Ok(s) = stream else { break };
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let Ok(mut tls) = acceptor.accept(s) else { continue };
            let _ = tx.send(read_request(&mut tls));
            let _ = tls.write_all(answer(status, body).as_bytes());
        }
    });
    (url, rx, ca)
}

/// What a `--docbrain-env` file may set (#603, owner ruling 2026-10-24: an allow-list of
/// DocBrain's own variables that the binary reads), written out here independently of the code,
/// so that a change to the code's list fails a test.
pub const ALLOWED_IN_THE_CLI: &[&str] = &["DOCBRAIN_CORPUS_DROP_ALERT", "DOCBRAIN_GENERATE_TIMEOUT_SECS"];
pub const ALLOWED_IN_THE_CONNECTOR: &[&str] = &["DOCBRAIN_API_KEY", "DOCBRAIN_SERVER_URL"];

/// Variables a file may set in neither binary: every one the round-2 review found a use for
/// (git's config and Apple's git shim, xdg-open's desktop files), the old deny-list's classes,
/// variables the binaries do read but that are not DocBrain's, DocBrain's identity variable,
/// one DocBrain-looking name nothing reads, other spellings of an allowed name, and one random.
/// `DOCBRAIN_CLI_CALLBACK_HOST` is the CLI's, but only from the shell (owner ruling 2026-10-24,
/// review round 3): a file could send the one-time login code to a listener it chose.
pub const REFUSED_EVERYWHERE: &[&str] = &[
    "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_DATA_DIRS", "XDG_CONFIG_DIRS", "DEVELOPER_DIR",
    "GIT_CONFIG_GLOBAL", "GIT_SSH_COMMAND", "HTTP_PROXY", "https_proxy", "ALL_PROXY", "NO_PROXY",
    "REQUEST_METHOD", "SSL_CERT_FILE", "SSL_CERT_DIR", "HOME", "PATH", "LD_PRELOAD",
    "DYLD_INSERT_LIBRARIES", "BROWSER", "NO_COLOR", "TERM", "CI", "RUST_LOG", "NODE_OPTIONS",
    "DOCBRAIN_PROFILE", "DOCBRAIN_CLI_CALLBACK_HOST", "DOCBRAIN_NOT_A_SETTING", "docbrain_api_key",
    "Docbrain_Corpus_Drop_Alert",
    "SYNTH_RANDOM_NAME",
];
