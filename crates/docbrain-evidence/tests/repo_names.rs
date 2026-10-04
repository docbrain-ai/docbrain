// SPDX-License-Identifier: MIT
//! `repo::name` against an independent reference implementation.
//!
//! `fixtures/repo/reference_corpus.json` is the output of a separately written
//! reference normaliser over its whole corpus — 388 distinct inputs, each run
//! as `Typed` and as `Remote` — so the rules are pinned by 776 expectations
//! nobody wrote by hand. Regenerate only from the reference, never from this
//! crate.

use docbrain_evidence::repo::{name, Forge, Form, KnownHost, RepoName, Unnamed};
use serde_json::Value;

fn corpus() -> Value {
    serde_json::from_str(include_str!("fixtures/repo/reference_corpus.json")).unwrap()
}

fn known_hosts(c: &Value) -> Vec<KnownHost> {
    c["known_hosts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| {
            let forge = match k[1].as_str().unwrap() {
                "github" => Forge::GitHub,
                _ => Forge::GitLab,
            };
            KnownHost::new(k[0].as_str().unwrap(), forge, k[2].as_str().unwrap())
        })
        .collect()
}

/// The reference's own spelling of a result.
fn got(input: &str, kh: &[KnownHost], form: Form) -> String {
    match name(input, kh, form) {
        Ok(n) => n.to_string(),
        Err(Unnamed::UnknownHost { alias }) => {
            format!("REFUSE:UnknownHost[host={}]", alias.as_deref().unwrap_or("None"))
        }
        Err(e) => format!("REFUSE:{e:?}"),
    }
}

#[test]
fn every_corpus_input_matches_the_reference_in_both_forms() {
    let c = corpus();
    let kh = known_hosts(&c);
    let mut wrong = Vec::new();
    let cases = c["cases"].as_array().unwrap();
    assert!(cases.len() >= 388, "the corpus shrank: {}", cases.len());
    for case in cases {
        let input = case["input"].as_str().unwrap();
        for (form, key) in [(Form::Typed, "typed"), (Form::Remote, "remote")] {
            let want = case[key].as_str().unwrap();
            let have = got(input, &kh, form);
            if have != want {
                wrong.push(format!("{input:?} {key}: want {want} have {have}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{} disagreements:\n{}", wrong.len(), wrong.join("\n"));
}

/// No output a caller can print — a name, an error's `Display` or `Debug` —
/// carries the corpus secret (planted as userinfo, query, fragment, path and
/// host), and an http(s) input never yields a host at all.
#[test]
fn no_output_carries_a_secret_and_http_errors_carry_no_host() {
    let c = corpus();
    let kh = known_hosts(&c);
    let secret = c["secret"].as_str().unwrap().to_ascii_lowercase();
    let mut leaks = Vec::new();
    for case in c["cases"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        // An http(s) URL as the parser sees it: scheme, then `://`. (`https:/h/o/r`
        // has one slash, so it is scp form and its "host" is the word `https`.)
        let low = input.trim_start().to_ascii_lowercase();
        let http = low.starts_with("http://") || low.starts_with("https://");
        for form in [Form::Typed, Form::Remote] {
            let out = match name(input, &kh, form) {
                Ok(n) => format!("{n} {n:?}"),
                Err(e) => {
                    if http && matches!(e, Unnamed::UnknownHost { alias: Some(_) }) {
                        leaks.push(format!("{input:?}: http input carried a host"));
                    }
                    // Display only: Debug of an ssh alias carries the host by design.
                    e.to_string()
                }
            };
            if out.to_ascii_lowercase().contains(&secret) {
                leaks.push(format!("{input:?} -> {out}"));
            }
        }
    }
    assert!(leaks.is_empty(), "{}", leaks.join("\n"));
}

/// The vector table, in the open, so a reader can see the
/// rules without opening the corpus.
#[test]
fn the_r4_vector_table() {
    let kh = [KnownHost::new("code.example.net", Forge::GitLab, "/gl")];
    let t = |s: &str| got(s, &kh, Form::Typed);
    let r = |s: &str| got(s, &kh, Form::Remote);
    assert_eq!(t("GitHub:Acme/Platform/"), "github:acme/platform");
    assert_eq!(t("github:o/r.git"), "REFUSE:UnknownHost[host=github]");
    assert_eq!(t("https://GitHub.com./O/R.git"), "github:o/r");
    assert_eq!(t("git@GITHUB.COM:o/r.git"), "github:o/r");
    assert_eq!(t("git+ssh://git@github.com/o/r.git"), "github:o/r");
    assert_eq!(t("https://github.com/o/r?access_token=TOK"), "github:o/r");
    assert_eq!(t("https://user:p@ssTOK@github.com/o/r"), "github:o/r");
    assert_eq!(t("https://TOK/x@github.com/o/r"), "REFUSE:UnknownHost[host=None]");
    assert_eq!(t("https://github.com.evil.example/o/r"), "REFUSE:UnknownHost[host=None]");
    assert_eq!(t("https://github.com:evil/o/r"), "REFUSE:Unsupported");
    assert_eq!(t("git@[::1]:g/p.git"), "REFUSE:Unsupported");
    assert_eq!(t("/srv/x.git"), "REFUSE:Unsupported");
    for web in [
        "https://gitlab.com/g/p/tree/main",
        "https://gitlab.com/g/p/merge_requests/3",
        "https://gitlab.com/g/p/blob/main/README.md",
        "https://gitlab.com/g/sub/p",
        "https://gitlab.com/groups/g",
    ] {
        assert_eq!(t(web), "REFUSE:BadPath", "{web}");
    }
    assert_eq!(r("https://gitlab.com/g/sub/p"), "gitlab:g/sub/p");
    assert_eq!(r("https://gitlab-ci-token:TOK@gitlab.com/g/sub/p"), "gitlab:g/sub/p");
    assert_eq!(r("https://code.example.net/gl/g/sub/p"), "gitlab:g/sub/p");
    assert_eq!(r("https://gitlab.com/groups/g"), "gitlab:groups/g");
    assert_eq!(t("https://gitlab.com/g/sub/p.git"), "gitlab:g/sub/p");
    assert_eq!(t("ssh://git@gitlab.com/g/sub/p"), "gitlab:g/sub/p");
    assert_eq!(t("https://gitlab.com/G/Sub/Deep/P.git"), "gitlab:g/sub/deep/p");
    assert_eq!(t("https://gitlab.com/g/p"), "gitlab:g/p");
    assert_eq!(t("https://gitlab.com/g/tree/p.git"), "gitlab:g/tree/p");
    assert_eq!(t("https://gitlab.com/g/info.git"), "gitlab:g/info");
    for bad in [
        "https://gitlab.com/g/new.git",
        "https://gitlab.com/g/-/p.git",
        "https://gitlab.com/g/p--x.git",
        "https://gitlab.com/g/p.git.git",
        "https://github.com/o/r.wiki.git",
        "https://code.example.net/glx/p.git",
        "https://code.example.net/GL/g/p.git",
    ] {
        assert_eq!(t(bad), "REFUSE:BadPath", "{bad}");
    }
    assert_eq!(t("https://code.example.net:8443/gl/Grp/Sub/Proj.git"), "gitlab:grp/sub/proj");
    assert_eq!(t("ssh://git@code.example.net:2222/Grp/Sub/Proj.git"), "gitlab:grp/sub/proj");
    assert_eq!(t("https://github.com/orgs/acme"), "github:orgs/acme");
    assert_eq!(t("local:x"), "REFUSE:Local");
    assert_eq!(t(""), "REFUSE:Empty");
}

/// Adversarial inputs the corpus does not hold (build-90/self-test.md).
#[test]
fn adversarial_inputs() {
    let t = |s: &str| got(s, &[], Form::Typed);
    // empty and whitespace
    assert_eq!(t("   \t\n "), "REFUSE:Empty");
    assert_eq!(t("\u{3000}github:o/r\u{3000}"), "github:o/r", "Unicode whitespace is trimmed");
    // a newline inside a segment is not a name (a regex `$` would accept `a\n`)
    assert_eq!(t("gitlab:a\n/b"), "REFUSE:BadPath");
    // unicode in a path or a host never matches
    assert_eq!(t("github:ö/r"), "REFUSE:BadPath");
    assert_eq!(t("https://gıthub.com/o/r.git"), "REFUSE:UnknownHost[host=None]");
    assert_eq!(t("https://GİTHUB.com/o/r.git"), "REFUSE:UnknownHost[host=None]");
    assert_eq!(t("git@\u{212A}nown.example:o/r.git"), "REFUSE:UnknownHost[host=\u{212A}nown.example]");
    // deep nesting and mixed case
    assert_eq!(t("gitlab:A/b/C/d/E/f/G/h"), "gitlab:a/b/c/d/e/f/g/h");
    assert_eq!(t("GITLAB:G/P"), "gitlab:g/p");
    assert_eq!(t("gitlab:g//p"), "REFUSE:BadPath");
    // only one trailing `.git`, trailing slashes, and the scp form
    assert_eq!(t("git@gitlab.com:g/sub/p.git/"), "gitlab:g/sub/p");
    assert_eq!(t("github:o/r//"), "github:o/r");
    // the prefix alone, a colon alone, a multibyte character where the prefix colon would be
    assert_eq!(t("github:"), "REFUSE:BadPath");
    assert_eq!(t(":"), "REFUSE:UnknownHost[host=]");
    assert_eq!(t("github\u{e9}o/r"), "REFUSE:Unsupported");
    assert_eq!(t("loca\u{e9}x"), "REFUSE:Unsupported");
    // GitHub owner limits: 39 characters, no leading hyphen; repository ≤ 100
    assert_eq!(t(&format!("github:{}/r", "a".repeat(39))), format!("github:{}/r", "a".repeat(39)));
    assert_eq!(t(&format!("github:{}/r", "a".repeat(40))), "REFUSE:BadPath");
    assert_eq!(t(&format!("github:o/{}", "r".repeat(101))), "REFUSE:BadPath");
    assert_eq!(t("github:-o/r"), "REFUSE:BadPath");
}

#[test]
fn the_server_accepts_only_what_name_prints() {
    for ok in ["github:o/r", "gitlab:g/sub/p", "gitlab:g/info"] {
        assert_eq!(RepoName::parse_canonical(ok).unwrap().to_string(), ok);
    }
    for bad in ["GitHub:o/r", "github:o/r/", "github:o/r.git", "https://github.com/o/r", "local:x", "gitlab:g", "github:o/r/x"] {
        assert!(RepoName::parse_canonical(bad).is_none(), "{bad}");
    }
}
