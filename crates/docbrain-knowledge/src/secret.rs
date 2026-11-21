// SPDX-License-Identifier: MIT
//! The open secret screen (`SEMANTICS.md` §Secrets): the well-known token shapes, under the same ids and the
//! same regular expressions as the server's catalogue, which stays a superset at sync, and with the server's test
//! for a documented shape. A hit is a rule and a position, never
//! the text.

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;

/// One secret-shaped value: its rule and where it starts. Lines and columns count from 1; a column counts
/// Unicode scalar values; a BOM at the very start is not a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretHit {
    pub rule: &'static str,
    pub line: u32,
    pub col: u32,
}

/// (id, regex) of each open rule, in the server catalogue's order (`docbrain-core`
/// `safety/secrets/rules.rs`); a test there holds each string to the catalogue row with its id.
pub const OPEN_RULES: &[(&str, &str)] = &[
    ("aws_access_key_id", "AKIA[0-9A-Z]{16}"),
    ("github_pat_classic", "ghp_[0-9A-Za-z]{36}"),
    ("github_pat_fine_grained", "github_pat_[0-9A-Za-z_]{82}"),
    ("gitlab_pat", "glpat-[0-9A-Za-z_-]{20,}"),
    ("slack_token", "xox[baprs]-[0-9A-Za-z-]{10,72}"),
    ("pem_private_key", "-----BEGIN (RSA |EC |DSA |OPENSSH |ENCRYPTED )?PRIVATE KEY-----"),
    ("jwt_token", r"eyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+"),
    ("url_basic_auth_credentials", r"[a-zA-Z][a-zA-Z0-9+.\-]*://[^/\s:@]+:[^/\s@]+@"),
    ("github_token_families", "gh[oprsu]_[0-9A-Za-z]{36,255}"),
    ("anthropic_api_key", "sk-ant-[0-9A-Za-z_-]{20,}"),
    ("openai_api_key", "sk-(proj-)?[0-9A-Za-z]{20,}"),
    ("docbrain_api_key", "db_sk_[0-9a-z]{32}"),
    ("docbrain_app_token", "db_o[ar]t_[0-9a-z]{64}"),
];

/// Documented shapes (§Secrets): the server's shape test (`docbrain-core` `health/sensitive.rs`, `classify`) for the
/// three open rows it judges; a unit test there holds these values to the server's own. A private-key header with
/// fewer than this many base64 characters after it is an example, not a key.
pub const PEM_KEY_FLOOR: usize = 54;
/// How far past a private-key header its key material is looked for, in bytes.
pub const PEM_TAIL_WINDOW: usize = 8192;
/// Separators a wiki, a table, a JSON string or a quoted email puts into a key's lines, read as a line break, in
/// this order.
pub const PEM_LINE_ARTIFACTS: &[&str] = &[
    "\\\\n", "\\\\r", "\\\\t", "\\n", "\\r", "\\t", "&#10;", "&#xa;", "&#xA;", "&#13;", "&#xd;",
    "&#xD;", "&#32;", "&NewLine;", "&quot;", "<br />", "<br/>", "<br>", "</p>", "<p>", "|",
];
/// Published sample tokens (RFC 7519 §3.1, and the jwt.io demo), compared whole.
pub const SAMPLE_JWTS: &[&str] = &[
    // RFC 7519 §3.1's example token.
    "eyJ0eXAiOiJKV1QiLA0KICJhbGciOiJIUzI1NiJ9.eyJpc3MiOiJqb2UiLA0KICJleHAiOjEzMDA4MTkzODAsDQogImh0dHA6Ly9leGFtcGxlLmNvbS9pc19yb290Ijp0cnVlfQ.dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
    // The jwt.io demo token, republished in a great many how-to pages.
    "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
];
/// Template words: a URL password that is one of these, case-folded, or a bracketed placeholder whose inside is one,
/// is an example (`placeholder_values.txt` `[template_words]`).
pub const TEMPLATE_WORDS: &[&str] =
    &["changeme", "change-me", "example", "placeholder", "redacted", "dummy", "your-password", "your_password", "yourpassword", "mypassword"];
/// Further words a URL password may be and still be an example (`[url_password_words]`). `admin`, `test`, `pass`
/// and `token` are deliberately absent: they are working default passwords.
pub const URL_PASSWORD_WORDS: &[&str] = &["password", "passwd", "pwd", "secret"];

/// What is shown in place of text read from a repository that holds a secret shape: a hit is named, never its text.
pub const WITHHELD_VALUE: &str = "(withheld: secret-shape)";

fn compiled() -> &'static [(&'static str, Regex)] {
    static RULES: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    RULES.get_or_init(|| OPEN_RULES.iter().filter_map(|&(id, re)| Regex::new(re).ok().map(|r| (id, r))).collect())
}

/// Every secret-shaped value in `text` as (rule, byte range), one per secret, by position. A region is one
/// holder's match and what lies wholly inside it (a token in a URL's userinfo, `ghp_` under two rules): one secret.
/// A documented shape is not reported, but holds its region like any match, until a credential inside it takes the
/// region over (the server's `health/sensitive.rs` collapse, so a key pasted into an example is still reported).
/// For a caller that holds the text and must point into it; [`screen`] gives positions.
pub fn spans(text: &str) -> Vec<(&'static str, Range<usize>)> {
    let rules = compiled();
    // (start, end, row) of every match; by start, the longest first, then the catalogue's order.
    let mut found: Vec<(usize, usize, usize)> =
        rules.iter().enumerate().flat_map(|(i, (_, re))| re.find_iter(text).map(move |m| (m.start(), m.end(), i))).collect();
    found.sort_by_key(|&(s, e, i)| (s, std::cmp::Reverse(e), i));
    // Every `-----END` in the text, once: a header's search for its own is then a lookup, not a scan of its window.
    let ends: Vec<usize> = text.match_indices(PEM_END).map(|(i, _)| i).collect();
    let shape = |s: usize, e: usize, i: usize| documented(text, s..e, rules[i].0, &ends);
    // Regions as (start, end, row, documented); `held` ends the open region's holder.
    let mut regions: Vec<(usize, usize, usize, bool)> = Vec::new();
    let mut region: Option<(usize, usize, usize, bool)> = None;
    let mut held: Option<usize> = None;
    for (s, e, i) in found {
        if held.is_some_and(|k| s < k && e <= k) {
            if region.is_some_and(|r| r.3) && !shape(s, e, i) {
                region = Some((s, e, i, false));
                held = Some(e);
            }
            continue;
        }
        regions.extend(region.take());
        held = Some(e);
        region = Some((s, e, i, shape(s, e, i)));
    }
    regions.extend(region);
    regions.into_iter().filter(|r| !r.3).map(|(s, e, i, _)| (rules[i].0, s..e)).collect()
}

const PEM_END: &str = "-----END";

/// Whether the match `at` of `rule` is a documented shape (§Secrets); any other rule's match never is. `ends`: the
/// start of every [`PEM_END`] in `text`, in order.
fn documented(text: &str, at: Range<usize>, rule: &str, ends: &[usize]) -> bool {
    match rule {
        "pem_private_key" => pem_example(text, at.end, ends),
        "url_basic_auth_credentials" => text.get(at).is_some_and(placeholder_url),
        "jwt_token" => text.get(at).is_some_and(|m| SAMPLE_JWTS.contains(&m)),
        _ => false,
    }
}

/// A private-key header with no key material after it: to the first `-----END`, then to the next header, within
/// [`PEM_TAIL_WINDOW`] bytes.
fn pem_example(text: &str, header_end: usize, ends: &[usize]) -> bool {
    let mut limit = header_end.saturating_add(PEM_TAIL_WINDOW).min(text.len());
    while limit > 0 && !text.is_char_boundary(limit) {
        limit -= 1;
    }
    let Some(mut region) = text.get(header_end..limit) else { return false };
    // The first `-----END` wholly inside the window (`region.find(PEM_END)`, from the list).
    if let Some(&p) = ends.get(ends.partition_point(|&p| p < header_end)).filter(|&&p| p + PEM_END.len() <= limit) {
        region = &region[..p - header_end];
    }
    if let Some(m) = compiled().iter().find(|(id, _)| *id == "pem_private_key").and_then(|(_, re)| re.find(region)) {
        region = &region[..m.start()];
    }
    !has_key_material(region)
}

/// At least [`PEM_KEY_FLOOR`] base64 characters, after [`PEM_LINE_ARTIFACTS`] become line breaks: summed over the
/// lines that are base64 alone (trimmed of whitespace and `>`), or in one unbroken run.
fn has_key_material(region: &str) -> bool {
    let mut normalised = region.to_string();
    for artifact in PEM_LINE_ARTIFACTS {
        if normalised.contains(artifact) {
            normalised = normalised.replace(artifact, "\n");
        }
    }
    let lines: usize = normalised
        .lines()
        .map(|line| line.trim_matches(|c: char| c.is_ascii_whitespace() || c == '>'))
        .filter(|line| !line.is_empty() && line.bytes().all(is_base64))
        .map(str::len)
        .sum();
    let (mut run, mut longest) = (0usize, 0usize);
    for b in normalised.bytes() {
        run = if is_base64(b) { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    lines >= PEM_KEY_FLOOR || longest >= PEM_KEY_FLOOR
}

fn is_base64(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='
}

/// A `scheme://user:password@` match whose password, everything after the user's first colon, is a placeholder.
fn placeholder_url(matched: &str) -> bool {
    let userinfo = matched.strip_suffix('@').and_then(|rest| rest.find("://").and_then(|i| rest.get(i + 3..)));
    let password = userinfo.and_then(|u| u.find(':').and_then(|i| u.get(i + 1..)));
    password.is_some_and(|p| {
        let lower = p.to_ascii_lowercase();
        TEMPLATE_WORDS.iter().chain(URL_PASSWORD_WORDS).any(|w| *w == lower) || masking(p) || structured(p)
    })
}

/// A run of one masking character: dots, asterisks, `x` or `X`, or bullets.
fn masking(p: &str) -> bool {
    let all = |f: fn(char) -> bool| !p.is_empty() && p.chars().all(f);
    all(|c| c == '.') || all(|c| c == '*') || all(|c| c == 'x' || c == 'X') || all(|c| c == '\u{2022}')
}

/// A bracketed or `%` placeholder whose inside names something, or `$` and an UPPER_SNAKE name.
fn structured(p: &str) -> bool {
    for (open, close) in [("${", "}"), ("{{", "}}"), ("<", ">"), ("[", "]"), ("{", "}")] {
        if let Some(inner) = p.strip_prefix(open).and_then(|x| x.strip_suffix(close)) {
            return reference_like(inner.trim());
        }
    }
    if let Some(inner) = p.strip_prefix('%').and_then(|x| x.strip_suffix('%')) {
        return reference_like(inner);
    }
    p.strip_prefix('$').is_some_and(upper_snake)
}

/// An UPPER_SNAKE or lower-case name (`_`, `.`, `-`, space between its parts) whose every part is a word, or a
/// template word.
fn reference_like(inner: &str) -> bool {
    let named = upper_name(inner) || lower_name(inner);
    let words = || inner.split(['_', '.', '-', ' ']).filter(|x| !x.is_empty()).all(word_part);
    !inner.is_empty() && (named && words() || TEMPLATE_WORDS.contains(&inner.to_lowercase().as_str()))
}

fn upper_snake(v: &str) -> bool {
    upper_name(v) && v.split('_').filter(|x| !x.is_empty()).all(word_part)
}

/// `[A-Z_][A-Z0-9_]*`
fn upper_name(v: &str) -> bool {
    let mut c = v.chars();
    c.next().is_some_and(|f| f.is_ascii_uppercase() || f == '_') && c.all(|x| x.is_ascii_uppercase() || x.is_ascii_digit() || x == '_')
}

/// `[a-z_][a-z0-9_. -]*`
fn lower_name(v: &str) -> bool {
    let mut c = v.chars();
    c.next().is_some_and(|f| f.is_ascii_lowercase() || f == '_')
        && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || matches!(x, '_' | '.' | ' ' | '-'))
}

/// At least two ASCII letters, then at most three digits, and nothing else.
fn word_part(p: &str) -> bool {
    let letters = p.chars().take_while(char::is_ascii_alphabetic).count();
    let digits = p[letters..].chars().take_while(char::is_ascii_digit).count();
    letters >= 2 && digits <= 3 && letters + digits == p.chars().count()
}

/// [`spans`] as line and column: where each secret starts, never what it is.
pub fn screen(text: &str) -> Vec<SecretHit> {
    let mut chars = text.char_indices().peekable();
    let (mut line, mut col) = (1u32, 1u32);
    spans(text)
        .into_iter()
        .map(|(rule, at)| {
            let s = at.start;
            while let Some(&(j, c)) = chars.peek() {
                if j >= s {
                    break;
                }
                chars.next();
                if c == '\n' {
                    (line, col) = (line.saturating_add(1), 1);
                } else if !(j == 0 && c == '\u{FEFF}') {
                    col = col.saturating_add(1);
                }
            }
            SecretHit { rule, line, col }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const UPPER_DIGIT: &str = "Q7ZX3MK9RT2WPL5N";
    const ALNUM: &str = "q7Zx3mK9rT2wPl5NbV8c";
    const LOWER_DIGIT: &str = "q7zx3mk9rt2wpl5nbv8c";

    /// `n` characters cycled from `alphabet`: a token built at test time, never a committed literal.
    fn fill(alphabet: &str, n: usize) -> String {
        alphabet.chars().cycle().take(n).collect()
    }

    /// One synthetic token per rule.
    fn token(rule: &str) -> String {
        match rule {
            "aws_access_key_id" => format!("AKIA{}", fill(UPPER_DIGIT, 16)),
            "github_pat_classic" => format!("ghp_{}", fill(ALNUM, 36)),
            "github_pat_fine_grained" => format!("github_pat_{}", fill(&format!("{ALNUM}_"), 82)),
            "gitlab_pat" => format!("glpat-{}", fill(ALNUM, 24)),
            "slack_token" => format!("xoxb-{}", fill(ALNUM, 30)),
            "pem_private_key" => format!("-----BEGIN RSA {} KEY-----\n{}", "PRIVATE", fill(ALNUM, 64)),
            "jwt_token" => format!("eyJ{}.eyJ{}.{}", fill(ALNUM, 20), fill(ALNUM, 30), fill(ALNUM, 43)),
            "url_basic_auth_credentials" => format!("https://deploy:{}@db.internal", fill(ALNUM, 16)),
            "github_token_families" => format!("gho_{}", fill(ALNUM, 40)),
            "anthropic_api_key" => format!("sk-ant-{}", fill(ALNUM, 40)),
            "openai_api_key" => format!("sk-{}", fill(ALNUM, 40)),
            "docbrain_api_key" => format!("db_sk_{}", fill(LOWER_DIGIT, 32)),
            "docbrain_app_token" => format!("db_oat_{}", fill(LOWER_DIGIT, 64)),
            other => panic!("no token for {other}"),
        }
    }

    #[test]
    fn each_rule_hits_its_shape() {
        assert_eq!(compiled().len(), OPEN_RULES.len(), "every row compiles");
        for &(rule, _) in OPEN_RULES {
            let t = token(rule);
            assert_eq!(screen(&format!("x {t} y")), [SecretHit { rule, line: 1, col: 3 }], "{rule}");
            // One character short of the body's least length is no hit.
            let cut = match rule {
                "gitlab_pat" | "github_token_families" => 5,
                "slack_token" | "anthropic_api_key" | "openai_api_key" => 21,
                "pem_private_key" | "url_basic_auth_credentials" | "jwt_token" => continue,
                _ => 1,
            };
            assert!(screen(&format!("x {} y", &t[..t.len() - cut])).is_empty(), "{rule} short");
        }
    }

    #[test]
    fn names_of_secrets_are_not_secrets() {
        assert!(screen("rotate AWS_SECRET_ACCESS_KEY then GITHUB_TOKEN").is_empty());
        let names = "SLACK_BOT_TOKEN ANTHROPIC_API_KEY OPENAI_API_KEY DOCBRAIN_API_KEY prefixes: AKIA ghp_ gho_ \
            github_pat_ glpat- xoxb- sk- sk-ant- sk-proj- db_sk_ db_oat_ eyJ -----BEGIN PUBLIC KEY----- \
            -----BEGIN CERTIFICATE----- https://example.com/a:b@c git@example.com:org/repo.git \
            postgres://db.internal:5432/app https://user@host";
        assert_eq!(screen(names), []);
    }

    #[test]
    fn position_not_text() {
        let t = token("aws_access_key_id");
        let text = format!("first line\r\nsecond\ncafé: {t}\n");
        let hits = screen(&text);
        assert_eq!(hits, [SecretHit { rule: "aws_access_key_id", line: 3, col: 7 }]);
        let shown = format!("{hits:?}");
        for part in [&t[..4], &t[4..10], &t[10..]] {
            assert!(!shown.contains(part), "{shown}");
        }
        // A leading BOM is not a column; a CR is.
        assert_eq!(screen(&format!("\u{FEFF}{t}"))[0].col, 1);
        assert_eq!(screen(&format!("\r{t}"))[0].col, 2);
    }

    #[test]
    fn one_finding_per_secret() {
        // `ghp_` under two rules, a token in a URL's userinfo: one region each. Two secrets apart: two.
        let ghp = token("github_pat_classic");
        assert_eq!(screen(&ghp), [SecretHit { rule: "github_pat_classic", line: 1, col: 1 }]);
        let url = format!("https://ci:{ghp}@example.com");
        assert_eq!(screen(&url), [SecretHit { rule: "url_basic_auth_credentials", line: 1, col: 1 }]);
        assert_eq!(spans(&url), [("url_basic_auth_credentials", 0..11 + ghp.len() + 1)]);
        let two = format!("{} and {}", token("docbrain_api_key"), token("slack_token"));
        assert_eq!(
            screen(&two),
            [SecretHit { rule: "docbrain_api_key", line: 1, col: 1 }, SecretHit { rule: "slack_token", line: 1, col: 44 }]
        );
    }

    fn rules_of(text: &str) -> Vec<&'static str> {
        spans(text).into_iter().map(|(rule, _)| rule).collect()
    }

    #[test]
    fn documented_shapes_are_not_secrets() {
        // A private-key header is a key only with key material after it: up to its END, the next header, or the window.
        let header = format!("-----BEGIN {} KEY-----", "PRIVATE");
        let end = format!("-----END {} KEY-----", "PRIVATE");
        let key = |n| fill(ALNUM, n);
        assert_eq!(rules_of(&format!("{header}\n...\n{end}")), [] as [&str; 0]);
        assert_eq!(rules_of(&format!("{header}\n{}\n{end}", key(PEM_KEY_FLOOR - 1))), [] as [&str; 0]);
        assert_eq!(rules_of(&format!("{header}\n{}\n{end}", key(PEM_KEY_FLOOR))), ["pem_private_key"]);
        assert_eq!(rules_of(&format!("{header}\n{}\n{}\n{end}", key(30), key(30))), ["pem_private_key"], "line sum");
        assert_eq!(rules_of(&format!("{header}\n> {}\n> {}", key(30), key(30))), ["pem_private_key"], "quoted");
        assert_eq!(rules_of(&format!("{header}\\n{}\\n{}", key(30), key(30))), ["pem_private_key"], "escaped breaks");
        assert_eq!(rules_of(&format!("{header}<br>{}<br>{}", key(30), key(30))), ["pem_private_key"], "html breaks");
        assert_eq!(rules_of(&format!("{header}\n{end}\n{}", key(60))), [] as [&str; 0], "after END");
        assert_eq!(rules_of(&format!("{header}\n{header}\n{}", key(60))), ["pem_private_key"], "next header owns it");
        let pad = |n| " ".repeat(n);
        assert_eq!(rules_of(&format!("{header}{}{}", pad(PEM_TAIL_WINDOW - PEM_KEY_FLOOR), key(PEM_KEY_FLOOR))), ["pem_private_key"]);
        assert_eq!(rules_of(&format!("{header}{}{}", pad(PEM_TAIL_WINDOW - PEM_KEY_FLOOR + 1), key(PEM_KEY_FLOOR))), [] as [&str; 0]);
        // A URL's password (after the first colon) that is a placeholder is an example; anything else is a credential.
        for pw in [
            "password", "PASSWORD", "Secret", "changeme", "your_password", "...", "***", "xXxX", "\u{2022}\u{2022}",
            "<DB_PASS>", "${db.password}", "{{db_password}}", "[password]", "{changeme}", "%DB_PASS%", "$DB_PASSWORD",
            "$_DB_PASSWORD", "<_db_pass>", "$DB2_PASSWORD", "<db2_pass>",
        ] {
            assert_eq!(rules_of(&format!("postgres://app:{pw}@db.internal/app")), [] as [&str; 0], "{pw}");
        }
        for pw in [
            "admin", "test", "passw0rdR3al", "password1", "$ecret123", "<K3J9X2M7Q8ZT>", "${k8m2q9w4x7zz}", "(password)",
            "<a>", "<abc1234>", "%%", "$", "<>", "x.", "password:extra", "$Q7Z9K2X", "<Db-password>",
        ] {
            assert_eq!(rules_of(&format!("postgres://app:{pw}@db.internal/app")), ["url_basic_auth_credentials"], "{pw}");
        }
        assert_eq!(rules_of("https://password:Xk9mQ2vR7@host"), ["url_basic_auth_credentials"], "the user is not the password");
        // A published sample token, whole, is an example; one character more is not that sample.
        for t in SAMPLE_JWTS {
            assert_eq!(rules_of(&format!("Authorization: {t}")), [] as [&str; 0]);
            assert_eq!(rules_of(&format!("Authorization: {t}x")), ["jwt_token"]);
        }
    }

    #[test]
    fn documented_shape_edges() {
        let header = format!("-----BEGIN {} KEY-----", "PRIVATE");
        let end = format!("-----END {} KEY-----", "PRIVATE");
        let key = |n| fill(ALNUM, n);
        let prose = |n: usize| "word ".repeat(n / 5 + 1)[..n].to_owned();
        // An END right after the header closes it; an example before a key leaves that key's header its own END.
        assert_eq!(rules_of(&format!("{header}{end}\n{}", key(60))), [] as [&str; 0]);
        assert_eq!(rules_of(&format!("{header}\n{end}\n{header}\n{}\n{end}", key(60))), ["pem_private_key"]);
        // An END that starts one byte past the window is not in it.
        assert_eq!(rules_of(&format!("{header}{}{end}", prose(PEM_TAIL_WINDOW + 1))), [] as [&str; 0]);
        // Prose lines are no key material, however long; `/`, `+` and `=` are base64.
        assert_eq!(rules_of(&format!("{header}\nyour key goes here, paste it in\nand keep it out of the repository\n{end}")), [] as [&str; 0]);
        assert_eq!(rules_of(&format!("{header}\n{}\n{end}", "abcdefg/+=".repeat(6))), ["pem_private_key"]);
        // A window that ends inside a character ends before it: the character neither hides the example nor
        // joins the last line.
        assert_eq!(rules_of(&format!("{header}{}\u{e9} tail", prose(PEM_TAIL_WINDOW - 1))), [] as [&str; 0]);
        let lines = format!("{}\n{}\n{}", prose(PEM_TAIL_WINDOW - 1 - 62), key(30), key(30));
        assert_eq!(rules_of(&format!("{header}{lines}\u{e9} tail")), ["pem_private_key"]);
    }

    #[test]
    fn a_secret_overlapping_another_is_its_own() {
        // Slack's shape stops at 72 characters; a key that starts inside it and runs past its end is a second region.
        let text = format!("xoxb-{}AKIA{}", "a".repeat(65), "B".repeat(16));
        assert_eq!(spans(&text), [("slack_token", 0..77), ("aws_access_key_id", 70..90)]);
    }

    #[test]
    fn a_secret_inside_an_example_is_reported() {
        // The URL is an example (its password is a placeholder), the token in its user part is not.
        let ghp = token("github_pat_classic");
        assert_eq!(screen(&format!("https://{ghp}:password@example.com")), [SecretHit { rule: "github_pat_classic", line: 1, col: 9 }]);
        let sk = token("openai_api_key");
        assert_eq!(rules_of(&format!("see https://{sk}:<API_KEY>@api.example.com and {ghp}")), ["openai_api_key", "github_pat_classic"]);
    }

    #[test]
    fn hostile_headers_are_fast() {
        // Linear, judged by growth rather than by a wall-clock budget a loaded machine misses: four times the
        // headers costs well under the sixteen times a quadratic screen would (best of five, interleaved).
        let text = |kib: usize| format!("-----BEGIN {} KEY-----\n", "PRIVATE").repeat(kib * 1024 / 28 + 1);
        let (small, large) = (text(64), text(256));
        let time = |t: &str| {
            let started = std::time::Instant::now();
            let hits = spans(t);
            let took = started.elapsed();
            assert!(hits.is_empty(), "{}", hits.len());
            took
        };
        let (mut s, mut l) = (std::time::Duration::MAX, std::time::Duration::MAX);
        for _ in 0..5 {
            s = s.min(time(&small));
            l = l.min(time(&large));
        }
        assert!(l < s * 8, "64 KiB {s:?}, 256 KiB {l:?}");
        assert!(l < std::time::Duration::from_secs(2), "{l:?}");
    }

    #[test]
    fn hostile_input_is_fast() {
        let unit = "AKIA ghp_ gho_ github_pat_ glpat- xoxb- -----BEGIN eyJa.eyJb. https://a: sk-ant- sk-proj- db_sk_ db_oat_ ";
        let text = unit.repeat(256 * 1024 / unit.len() + 1);
        let started = std::time::Instant::now();
        let hits = screen(&text);
        let took = started.elapsed();
        assert!(took < std::time::Duration::from_millis(200), "{took:?}");
        assert!(hits.is_empty(), "{}", hits.len());
    }
}
