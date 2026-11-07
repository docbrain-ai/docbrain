// SPDX-License-Identifier: MIT
//! The `config_key` key path grammar (spec §4.3, clarification C16; `SEMANTICS.md` §The record).
//!
//! Dot-separated segments. A segment containing `.`, `/`, `[`, `=` or `"` is double-quoted, with `\"` and
//! `\\` the only escapes. `[n]` indexes a list (`0` or `[1-9][0-9]*`); `[k=v,k2.sub=v2]` selects the one
//! element whose keys equal those values. Inside a selector a bare key segment also ends at `,` and `]`;
//! a value is the raw text up to `,` or `]`, or a double-quoted string. Nothing is trimmed.

/// One step of a key path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Seg {
    Key(String),
    Index(usize),
    /// Each entry is a key path (its segments) and the value its element must equal.
    Select(Vec<(Vec<String>, String)>),
}

/// A parsed key path and the text it was parsed from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyPath {
    pub segs: Vec<Seg>,
    pub text: String,
}

/// Parses a key path; the error is the byte offset where it stops parsing.
pub fn parse_key_path(s: &str) -> Result<KeyPath, usize> {
    let b = s.as_bytes();
    let mut segs = Vec::new();
    let mut i = if b.first() == Some(&b'[') {
        let (seg, j) = bracket(s, 0)?;
        segs.push(seg);
        j
    } else {
        let (k, j) = segment(s, 0, false)?;
        segs.push(Seg::Key(k));
        j
    };
    while let Some(&c) = b.get(i) {
        let (seg, j) = match c {
            b'.' => segment(s, i + 1, false).map(|(k, j)| (Seg::Key(k), j))?,
            b'[' => bracket(s, i)?,
            _ => return Err(i),
        };
        segs.push(seg);
        i = j;
    }
    Ok(KeyPath { segs, text: s.to_owned() })
}

/// A quoted or bare segment starting at `i`, and the offset after it.
fn segment(s: &str, i: usize, in_select: bool) -> Result<(String, usize), usize> {
    if s.as_bytes().get(i) == Some(&b'"') {
        return quoted(s, i);
    }
    let stop = |c: char| matches!(c, '.' | '/' | '[' | '=' | '"') || (in_select && matches!(c, ',' | ']'));
    let end = s.get(i..).and_then(|t| t.find(stop)).map_or(s.len(), |k| i + k);
    if end == i {
        return Err(i);
    }
    Ok((s[i..end].to_owned(), end))
}

/// A double-quoted string starting at `i` (the `"`), and the offset after its closing quote.
fn quoted(s: &str, i: usize) -> Result<(String, usize), usize> {
    let mut out = String::new();
    let mut chars = s[i + 1..].char_indices().map(|(k, c)| (i + 1 + k, c));
    while let Some((k, c)) = chars.next() {
        match c {
            '"' => return Ok((out, k + 1)),
            '\\' => match chars.next() {
                Some((_, e @ ('"' | '\\'))) => out.push(e),
                _ => return Err(k),
            },
            c => out.push(c),
        }
    }
    Err(s.len())
}

/// An index or a selector starting at `i` (the `[`), and the offset after its `]`.
fn bracket(s: &str, i: usize) -> Result<(Seg, usize), usize> {
    let b = s.as_bytes();
    let digits = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && b.get(i + 1 + digits) == Some(&b']') {
        let n = &s[i + 1..i + 1 + digits];
        if n.len() > 1 && n.starts_with('0') {
            return Err(i + 1);
        }
        let n = n.parse::<usize>().map_err(|_| i + 1)?;
        return Ok((Seg::Index(n), i + 2 + digits));
    }
    let mut entries = Vec::new();
    let mut j = i + 1;
    loop {
        let (k, nj) = segment(s, j, true)?;
        let mut key = vec![k];
        j = nj;
        while b.get(j) == Some(&b'.') {
            let (k, nj) = segment(s, j + 1, true)?;
            key.push(k);
            j = nj;
        }
        if b.get(j) != Some(&b'=') {
            return Err(j);
        }
        j += 1;
        let (value, nj) = if b.get(j) == Some(&b'"') {
            quoted(s, j)?
        } else {
            let end = s[j..].find([',', ']']).map_or(s.len(), |k| j + k);
            (s[j..end].to_owned(), end)
        };
        j = nj;
        entries.push((key, value));
        match b.get(j) {
            Some(b',') => j += 1,
            Some(b']') => return Ok((Seg::Select(entries), j + 1)),
            _ => return Err(j),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> Seg {
        Seg::Key(s.to_owned())
    }

    #[test]
    fn key_path_grammar() {
        let k = parse_key_path(r#"[kind=Job,metadata.name=api-migrate].metadata.annotations."argocd.argoproj.io/sync-wave""#).unwrap();
        assert_eq!(k.segs.len(), 4);
        assert_eq!(
            k.segs[0],
            Seg::Select(vec![(vec!["kind".to_owned()], "Job".to_owned()), (vec!["metadata".to_owned(), "name".to_owned()], "api-migrate".to_owned())])
        );
        assert_eq!(k.segs[3], key("argocd.argoproj.io/sync-wave"));
        assert!(parse_key_path("package[name=sqlx].version").is_ok());
        assert!(parse_key_path("a.\"b").is_err());
    }

    #[test]
    fn indexes_and_selectors() {
        assert_eq!(parse_key_path("a[0][12].b").unwrap().segs, vec![key("a"), Seg::Index(0), Seg::Index(12), key("b")]);
        assert_eq!(parse_key_path("[3]").unwrap().segs, vec![Seg::Index(3)]);
        assert_eq!(parse_key_path("ingress[from_port=443].to_port").unwrap().segs.len(), 3);
        let q = parse_key_path(r#"a[k="x,y]\"z"]"#).unwrap();
        assert_eq!(q.segs[1], Seg::Select(vec![(vec!["k".to_owned()], "x,y]\"z".to_owned())]));
        let raw = parse_key_path(r#"a[k=a"b c]"#).unwrap();
        assert_eq!(raw.segs[1], Seg::Select(vec![(vec!["k".to_owned()], "a\"b c".to_owned())]));
        assert_eq!(parse_key_path(r#"a["x.y"=1]"#).unwrap().segs[1], Seg::Select(vec![(vec!["x.y".to_owned()], "1".to_owned())]));
        assert_eq!(parse_key_path("a[k=]").unwrap().segs[1], Seg::Select(vec![(vec!["k".to_owned()], String::new())]));
        assert_eq!(parse_key_path("a[0=x]").unwrap().segs[1], Seg::Select(vec![(vec!["0".to_owned()], "x".to_owned())]));
    }

    #[test]
    fn quoting_and_bare_segments() {
        assert_eq!(parse_key_path(r#""a\"b\\c".d"#).unwrap().segs, vec![key("a\"b\\c"), key("d")]);
        assert_eq!(parse_key_path(r#""""#).unwrap().segs, vec![key("")]);
        assert_eq!(parse_key_path("on").unwrap().segs, vec![key("on")]);
        assert_eq!(parse_key_path("a b.c]d").unwrap().segs, vec![key("a b"), key("c]d")]);
        assert_eq!(parse_key_path("ünï.日本").unwrap().segs, vec![key("ünï"), key("日本")]);
        assert_eq!(parse_key_path("x.y").unwrap().text, "x.y");
    }

    #[test]
    fn rejects_with_offset() {
        for (s, at) in [
            ("", 0),
            (".a", 0),
            ("a.", 2),
            ("a..b", 2),
            ("a/b", 1),
            ("a=b", 1),
            ("a\"b", 1),
            ("a[01]", 2),
            ("a[]", 2),
            ("a[0]b", 4),
            ("a[k=v", 5),
            ("a[k]", 3),
            ("a[k=v]x", 6),
            ("a[k=\"v\"x]", 7),
            ("\"a\\x\"", 2),
            ("a.\"b", 4),
            ("a[x[0]=1]", 3),
            ("a[99999999999999999999999]", 2),
            ("a.[0]", 2),
            ("[k=v,]", 5),
        ] {
            assert_eq!(parse_key_path(s).map(|k| k.segs), Err(at), "{s:?}");
        }
    }
}
