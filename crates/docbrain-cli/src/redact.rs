// SPDX-License-Identifier: MIT
//! `Debug` for types that hold a credential, without the credential.
//!
//! `redacted_debug!(Type { plain, fields } secret { token } url { base_url } map { headers })`
//! implements `Debug` that prints:
//! - each plain field as it is;
//! - each `secret` only as present or absent (`<redacted>`, `Some(<redacted>)`, `None`, `""`);
//! - each `url` with every credential a URL can carry replaced by `<redacted>` ([`redact_url`]);
//! - each `map` (headers, unknown config keys) with its keys and every value `<redacted>` except
//!   those of a short list of headers that never carry a credential ([`HARMLESS_KEYS`]).
//!
//! Every section but the plain one is optional; a field not listed is not printed, so a field
//! added later stays out of `Debug` until someone lists it. `tests/credential_debug_invariant.rs`
//! holds every type in the workspace to it.
//!
//! A copy of `docbrain_core::redact` (this crate does not depend on docbrain-core), without
//! the JSON walk no type here needs.

/// A stand-in printed in place of a value.
pub struct Redacted(pub String);

impl std::fmt::Debug for Redacted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

const HIDDEN: &str = "<redacted>";

/// How a secret field shows in `Debug`: whether it is there, never what it is.
pub trait Redact {
    fn shape(&self) -> String;
}

impl Redact for String {
    fn shape(&self) -> String {
        self.as_str().shape()
    }
}

impl Redact for &str {
    fn shape(&self) -> String {
        if self.is_empty() { "\"\"".into() } else { HIDDEN.into() }
    }
}

impl Redact for Vec<u8> {
    fn shape(&self) -> String {
        if self.is_empty() { "[]".into() } else { HIDDEN.into() }
    }
}

impl<T: Redact> Redact for Option<T> {
    fn shape(&self) -> String {
        match self {
            None => "None".into(),
            Some(v) => format!("Some({})", v.shape()),
        }
    }
}

// ── url scrub start (docbrain-cli carries this block verbatim; credential_debug_invariant holds them equal)
/// Parameter and key names whose value is a credential, in a query string or a `key=value`
/// connection string (`host=h password=…`, `?access_token=…`).
const SECRET_KEYS: &[&str] = &[
    "password", "passwd", "pwd", "pass", "passphrase", "secret", "client_secret", "token", "access_token", "refresh_token", "id_token",
    "auth_token", "api_key", "apikey", "api-key", "key", "sig", "signature", "auth", "authorization", "code", "sslpassword", "sslkey",
    "x-amz-signature", "x-amz-credential", "x-amz-security-token", "sharedaccesssignature", "accountkey",
];
/// Hosts and paths that carry the secret in the path itself (incoming-webhook URLs).
const SECRET_PATHS: &[&str] = &["slack.com/services/", "/api/webhooks/", "webhook.office.com/webhookb2/", "api.telegram.org/bot"];

fn token_end(s: &str, from: usize) -> usize {
    s[from..].find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`')).map_or(s.len(), |i| from + i)
}

/// `s` with every credential a URL or a connection string in it can carry replaced by `<redacted>`:
/// - the userinfo of each URL: everything between `://` and the URL's last `@` (a password may hold an
///   unencoded `/`, `#` or `?`, so a path with an `@` in it is redacted too: over-redaction is the safe side);
/// - a scheme-less `user:password@host`;
/// - the value of each `name=value` whose name is a credential's ([`SECRET_KEYS`]: query strings and
///   key=value connection strings, quoted values included);
/// - the path of a URL that carries its secret there ([`SECRET_PATHS`]).
///
/// Every occurrence in the string is handled. It does not know credentials of any other shape (a
/// bare token in a path of an unknown service, a password in a free sentence).
pub fn redact_url(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    // 1. URLs and scheme-less user:password@host, one whitespace-separated token at a time.
    let mut i = 0;
    while i < s.len() {
        let ch = s[i..].chars().next().unwrap_or(' ');
        if ch.is_whitespace() || matches!(ch, '"' | '\'' | '<' | '>' | '`') {
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let end = token_end(s, i);
        let tok = &s[i..end];
        out.push_str(&scrub_token(tok));
        i = end;
    }
    // 2. name=value pairs whose name is a credential's.
    scrub_pairs(&out)
}

fn scrub_token(tok: &str) -> String {
    const HIDDEN: &str = "<redacted>";
    let (mut head, mut rest) = match tok.find("://") {
        Some(at) => (tok[..at + 3].to_string(), tok[at + 3..].to_string()),
        None => (String::new(), tok.to_string()),
    };
    if let Some(at) = rest.rfind('@') {
        let before = &rest[..at];
        // A URL's userinfo; or, with no scheme, `user:password@host` (an e-mail address or
        // `git@host:repo` has no `:` before its `@`).
        if !head.is_empty() || (before.contains(':') && !before.starts_with("mailto:")) {
            rest = format!("{HIDDEN}{}", &rest[at..]);
        }
    }
    if !head.is_empty() {
        let lower = rest.to_ascii_lowercase();
        for marker in SECRET_PATHS {
            if let Some(m) = lower.find(marker) {
                let cut = m + marker.len();
                let tail_end = rest[cut..].find(['?', '#']).map_or(rest.len(), |x| cut + x);
                if tail_end > cut {
                    rest = format!("{}{HIDDEN}{}", &rest[..cut], &rest[tail_end..]);
                }
                break;
            }
        }
        head.push_str(&rest);
        return head;
    }
    rest
}

fn scrub_pairs(s: &str) -> String {
    const HIDDEN: &str = "<redacted>";
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let Some(eq) = s[i..].find('=').map(|x| i + x) else {
            out.push_str(&s[i..]);
            break;
        };
        // The name: the run of [A-Za-z0-9_-] just before `=`.
        let mut start = eq;
        while start > i && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_' || bytes[start - 1] == b'-') {
            start -= 1;
        }
        let name = s[start..eq].to_ascii_lowercase();
        out.push_str(&s[i..=eq]);
        let mut j = eq + 1;
        if !name.is_empty() && SECRET_KEYS.contains(&name.as_str()) {
            let quote = s[j..].chars().next().filter(|c| *c == '\'' || *c == '"');
            let vend = match quote {
                Some(q) => s[j + 1..].find(q).map_or(s.len(), |x| j + 1 + x + 1),
                None => s[j..].find(|c: char| c.is_whitespace() || matches!(c, '&' | ';' | ',' | '"' | '\'' | ')' | '#')).map_or(s.len(), |x| j + x),
            };
            if vend > j {
                match quote {
                    Some(q) => {
                        out.push(q);
                        out.push_str(HIDDEN);
                        out.push(q);
                    }
                    None => out.push_str(HIDDEN),
                }
                j = vend;
            }
        }
        i = j;
    }
    out
}
// ── url scrub end

/// How a URL field shows in `Debug`: as it is, without its userinfo.
pub trait RedactUrl {
    fn shown(&self) -> String;
}

impl RedactUrl for String {
    fn shown(&self) -> String {
        self.as_str().shown()
    }
}

impl RedactUrl for &str {
    fn shown(&self) -> String {
        format!("{:?}", redact_url(self))
    }
}

impl<T: RedactUrl> RedactUrl for Vec<T> {
    fn shown(&self) -> String {
        format!("[{}]", self.iter().map(RedactUrl::shown).collect::<Vec<_>>().join(", "))
    }
}

impl<T: RedactUrl> RedactUrl for Option<T> {
    fn shown(&self) -> String {
        match self {
            None => "None".into(),
            Some(v) => format!("Some({})", v.shown()),
        }
    }
}

/// Header names whose values never carry a credential; every other value in a `map` field is
/// shown as `<redacted>`.
pub const HARMLESS_KEYS: &[&str] = &["accept", "accept-encoding", "accept-language", "cache-control", "content-type", "user-agent"];

fn harmless(key: &str) -> bool {
    HARMLESS_KEYS.iter().any(|h| h.eq_ignore_ascii_case(key))
}

fn shown_map<'a, V: 'a>(entries: impl Iterator<Item = (&'a String, &'a V)>, show: impl Fn(&V) -> String) -> String {
    let parts: Vec<String> = entries
        .map(|(k, v)| if harmless(k) { format!("{k:?}: {}", show(v)) } else { format!("{k:?}: {HIDDEN}") })
        .collect();
    format!("{{{}}}", parts.join(", "))
}

/// How a map field (headers, unknown keys) shows in `Debug`: its keys, and only harmless values.
pub trait RedactMap {
    fn shown(&self) -> String;
}

impl RedactMap for serde_json::Map<String, serde_json::Value> {
    fn shown(&self) -> String {
        // A JSON value shows as JSON (`"application/json"`, not `String("application/json")`).
        shown_map(self.iter(), |v| v.to_string())
    }
}

impl RedactMap for serde_json::Value {
    fn shown(&self) -> String {
        match self {
            serde_json::Value::Null => "null".into(),
            serde_json::Value::Object(m) => m.shown(),
            _ => HIDDEN.into(),
        }
    }
}

impl<V: std::fmt::Debug> RedactMap for std::collections::HashMap<String, V> {
    fn shown(&self) -> String {
        let mut e: Vec<_> = self.iter().collect();
        e.sort_by(|a, b| a.0.cmp(b.0));
        shown_map(e.into_iter(), |v| format!("{v:?}"))
    }
}

impl<V: std::fmt::Debug> RedactMap for std::collections::BTreeMap<String, V> {
    fn shown(&self) -> String {
        shown_map(self.iter(), |v| format!("{v:?}"))
    }
}

impl<T: RedactMap> RedactMap for Option<T> {
    fn shown(&self) -> String {
        match self {
            None => "None".into(),
            Some(v) => format!("Some({})", v.shown()),
        }
    }
}

/// See the module docs.
macro_rules! redacted_debug {
    ($t:ident { $($f:ident),* $(,)? }
        $(secret { $($s:ident),* $(,)? })?
        $(url { $($u:ident),* $(,)? })?
        $(map { $($m:ident),* $(,)? })?) => {
        impl ::std::fmt::Debug for $t {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                let mut d = f.debug_struct(stringify!($t));
                $( d.field(stringify!($f), &self.$f); )*
                $($( d.field(stringify!($s), &$crate::redact::Redacted($crate::redact::Redact::shape(&self.$s))); )*)?
                $($( d.field(stringify!($u), &$crate::redact::Redacted($crate::redact::RedactUrl::shown(&self.$u))); )*)?
                $($( d.field(stringify!($m), &$crate::redact::Redacted($crate::redact::RedactMap::shown(&self.$m))); )*)?
                d.finish()
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::redact_url;

    struct Holder {
        name: String,
        token: String,
        api_key: Option<String>,
        empty: Option<String>,
        unset: Option<String>,
        url: String,
        headers: serde_json::Value,
    }
    redacted_debug!(Holder { name } secret { token, api_key, empty, unset } url { url } map { headers });

    #[test]
    fn a_secret_shows_as_present_or_absent_never_its_value() {
        let h = Holder {
            name: "docs".into(),
            token: "Bearer s3cr3t".into(),
            api_key: Some("sk-live-123".into()),
            empty: Some(String::new()),
            unset: None,
            url: "postgres://app:pa55word@db.internal:5432/docbrain".into(),
            headers: serde_json::json!({"X-Api-Key": "k-999", "Content-Type": "application/json"}),
        };
        let out = format!("{h:?}");
        assert_eq!(
            out,
            r#"Holder { name: "docs", token: <redacted>, api_key: Some(<redacted>), empty: Some(""), unset: None, url: "postgres://<redacted>@db.internal:5432/docbrain", headers: {"Content-Type": "application/json", "X-Api-Key": <redacted>} }"#
        );
        let pretty = format!("{h:#?}");
        for leak in ["s3cr3t", "sk-live", "pa55word", "app:", "k-999"] {
            assert!(!pretty.contains(leak), "{leak}: {pretty}");
        }
    }

    #[test]
    fn a_url_shows_without_its_credentials() {
        // The scrubber is docbrain-core's, verbatim (credential_debug_invariant holds the two equal); its
        // property tests live there.
        assert_eq!(redact_url("postgres://u:pa/ss@h/db?password=x"), "postgres://<redacted>@h/db?password=<redacted>");
    }

}
