// SPDX-License-Identifier: MIT
//! An anchored YAML file as values (`SEMANTICS.md` §Resolving values): aliases expanded, plain scalars read by
//! both YAML versions, keys compared by text (C10), tags marked (C13).

use crate::reason::{Bound, Unverifiable};
use crate::scalar::{Scalar, yaml_plain};
use crate::value::{MapBuilder, Value};
use crate::yaml::{Mode, Node, NodeKind, Style, YamlErrorKind, read_yaml};

/// One value per document. A syntax error is `format`; nesting beyond `RESOLVER_DEPTH` is
/// `bound:resolver-depth`; alias expansion beyond its budget is `bound:alias-expansion`.
pub fn read_yaml_file(src: &str) -> Result<Vec<Value>, Unverifiable> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    // C21 skips one BOM.
    if src.starts_with('\u{feff}') {
        return Err(Unverifiable::Format);
    }
    let docs = read_yaml(src, Mode::Anchored).map_err(|e| match e.kind {
        YamlErrorKind::Depth => Unverifiable::Bound(Bound::ResolverDepth),
        YamlErrorKind::AliasBudget => Unverifiable::Bound(Bound::AliasExpansion),
        // A feature error is strict mode's only.
        YamlErrorKind::Syntax | YamlErrorKind::Feature => Unverifiable::Format,
    })?;
    Ok(docs.iter().map(value).collect())
}

/// Recursion follows nesting, which the reader holds to `RESOLVER_DEPTH`.
fn value(n: &Node) -> Value {
    if n.tagged {
        return Value::Opaque(Unverifiable::Tag);
    }
    match &n.kind {
        NodeKind::Scalar { text, style: Style::Plain } => Value::Scalar(yaml_plain(text)),
        NodeKind::Scalar { text, .. } => Value::Scalar(Scalar::Str(text.clone())),
        NodeKind::Seq(items) => Value::List(items.iter().map(value).collect()),
        NodeKind::Map(entries) => {
            let mut m = MapBuilder::default();
            let mut merge_key = false;
            for (k, v) in entries {
                // C10: a key is its scalar text, whatever its style or type; a non-scalar key never matches.
                let NodeKind::Scalar { text, style } = &k.kind else { continue };
                if text == "<<" && *style == Style::Plain {
                    merge_key = true;
                } else {
                    m.insert(text.clone(), value(v));
                }
            }
            m.finish(merge_key)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bounds::{ALIAS_EXPANSION_MAX, RESOLVER_DEPTH};
    use crate::reason::Bound;
    use crate::scalar::{Decimal, Scalar};

    fn one(src: &str) -> Value {
        let mut docs = read_yaml_file(src).unwrap();
        assert_eq!(docs.len(), 1, "{src:?}");
        docs.remove(0)
    }
    fn entries(v: &Value) -> Vec<(&str, &Value)> {
        match v {
            Value::Map { entries, .. } => entries.iter().map(|(k, v)| (k.as_str(), v)).collect(),
            other => panic!("not a map: {other:?}"),
        }
    }
    fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
        entries(v).into_iter().find(|(key, _)| *key == k).unwrap_or_else(|| panic!("no {k}")).1
    }
    fn st(s: &str) -> Value {
        Value::Scalar(Scalar::Str(s.into()))
    }
    fn num(s: &str) -> Value {
        Value::Scalar(Scalar::Number(Decimal::parse_number(s).unwrap()))
    }

    #[test]
    fn plain_scalars_read_by_both_versions() {
        let v = one("a: 1\nb: 0755\nc: on\nd: '0755'\ne: \"on\"\nf: |\n  x\ng: ~\nh: true\ni: 1.50\nj: text\n");
        assert_eq!(get(&v, "a"), &num("1"));
        assert_eq!(get(&v, "b"), &Value::Scalar(Scalar::Opaque(Unverifiable::Yaml11)));
        assert_eq!(get(&v, "c"), &Value::Scalar(Scalar::Opaque(Unverifiable::Yaml11)));
        assert_eq!(get(&v, "d"), &st("0755"));
        assert_eq!(get(&v, "e"), &st("on"));
        assert_eq!(get(&v, "f"), &st("x\n"));
        assert_eq!(get(&v, "g"), &Value::Scalar(Scalar::Null));
        assert_eq!(get(&v, "h"), &Value::Scalar(Scalar::Bool(true)));
        assert_eq!(get(&v, "i"), &num("1.5"));
        assert_eq!(get(&v, "j"), &st("text"));
    }

    #[test]
    fn keys_by_text() {
        // C10: `on` is the key `on`, `1` and `'1'` are one key, and a key defined twice is stored once.
        let v = one("on: a\n1: b\n'1': c\nx: d\n");
        assert_eq!(entries(&v), vec![("on", &st("a")), ("1", &Value::Opaque(Unverifiable::DefinedTwice)), ("x", &st("d"))]);
        // A non-scalar key never matches, and is not an entry.
        let v = one("? [a]\n: 1\n? {b: 1}\n: 2\nc: 3\n");
        assert_eq!(entries(&v), vec![("c", &num("3"))]);
    }

    #[test]
    fn merge_key_sets_the_flag() {
        let v = one("base: &b\n  a: 1\nsvc:\n  <<: *b\n  c: 2\n");
        match get(&v, "svc") {
            Value::Map { entries, merge_key } => {
                assert!(merge_key);
                assert_eq!(entries, &vec![("c".to_owned(), num("2"))]);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(get(&v, "base"), Value::Map { merge_key: false, .. }));
        // Only a plain `<<` is the merge key; a quoted one is an ordinary key.
        let v = one("svc:\n  '<<': 1\n");
        assert!(matches!(get(&v, "svc"), Value::Map { merge_key: false, entries } if entries.len() == 1));
    }

    #[test]
    fn aliases_expand() {
        let v = one("x-common: &c\n  image: 'a:1'\nsvc: *c\nlist: [*c, *c]\n");
        assert_eq!(get(get(&v, "svc"), "image"), &st("a:1"));
        assert!(matches!(get(&v, "list"), Value::List(items) if items.len() == 2));
    }

    #[test]
    fn tags_are_opaque() {
        let v = one("a: !Ref x\nb: !!str 1\nc: !vault |\n  secret\nd: !Sub [a, b]\ne: plain\n");
        for k in ["a", "b", "c", "d"] {
            assert_eq!(get(&v, k), &Value::Opaque(Unverifiable::Tag), "{k}");
        }
        assert_eq!(get(&v, "e"), &st("plain"));
        assert_eq!(read_yaml_file("--- !Foo\na: 1\n").unwrap(), vec![Value::Opaque(Unverifiable::Tag)]);
    }

    #[test]
    fn documents() {
        assert_eq!(read_yaml_file("a: 1\n---\nb: 2\n").unwrap().len(), 2);
        assert_eq!(read_yaml_file("").unwrap(), Vec::<Value>::new());
        assert_eq!(read_yaml_file("--- 5\n").unwrap(), vec![num("5")]);
    }

    #[test]
    fn errors() {
        for t in ["a: [", "a: 'x", "a: b: c\n", "\u{feff}\u{feff}a: 1\n"] {
            assert_eq!(read_yaml_file(t), Err(Unverifiable::Format), "{t:?}");
        }
        // C21: one BOM is skipped.
        assert_eq!(get(&one("\u{feff}a: 1\n"), "a"), &num("1"));
        let deep = format!("{}1{}", "[".repeat(RESOLVER_DEPTH + 1), "]".repeat(RESOLVER_DEPTH + 1));
        assert_eq!(read_yaml_file(&deep), Err(Unverifiable::Bound(Bound::ResolverDepth)));
        let ok = format!("{}1{}", "[".repeat(RESOLVER_DEPTH), "]".repeat(RESOLVER_DEPTH));
        assert!(read_yaml_file(&ok).is_ok());
        // Billion laughs: each level doubles.
        let mut laughs = String::from("a0: &a0 [x, x]\n");
        let levels = (ALIAS_EXPANSION_MAX as f64).log2() as usize + 2;
        for i in 1..=levels {
            laughs.push_str(&format!("a{i}: &a{i} [*a{}, *a{}]\n", i - 1, i - 1));
        }
        assert_eq!(read_yaml_file(&laughs), Err(Unverifiable::Bound(Bound::AliasExpansion)));
    }

    #[test]
    fn crlf_and_unicode() {
        let v = one("名前:\r\n  \"my key\": 太郎\r\n");
        assert_eq!(get(get(&v, "名前"), "my key"), &st("太郎"));
    }
}
