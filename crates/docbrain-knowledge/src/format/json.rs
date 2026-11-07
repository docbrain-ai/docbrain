// SPDX-License-Identifier: MIT
//! A strict RFC 8259 reader (`SEMANTICS.md` §Resolving values): no comments, no trailing commas, no `NaN`.

use crate::bounds::{NUMBER_TEXT_MAX, RESOLVER_DEPTH};
use crate::reason::{Bound, Unverifiable};
use crate::scalar::{Decimal, Scalar};
use crate::value::{MapBuilder, Value};

/// Reads a JSON text. A syntax error is `format`; nesting deeper than `RESOLVER_DEPTH` (the root map or list
/// is 1) is `bound:resolver-depth`. A key defined twice is stored once as `defined-twice`.
pub fn read_json(src: &str) -> Result<Value, Unverifiable> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let mut r = Reader { src, at: 0 };
    r.space();
    let v = r.value(0)?;
    r.space();
    if r.at != src.len() {
        return Err(Unverifiable::Format);
    }
    Ok(v)
}

struct Reader<'a> {
    src: &'a str,
    at: usize,
}

const FORMAT: Unverifiable = Unverifiable::Format;

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.at).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.at += 1;
        Some(b)
    }

    fn expect(&mut self, b: u8) -> Result<(), Unverifiable> {
        if self.next() == Some(b) { Ok(()) } else { Err(FORMAT) }
    }

    /// RFC 8259 whitespace: space, tab, line feed, carriage return.
    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    /// A value inside `depth` open maps and lists.
    fn value(&mut self, depth: usize) -> Result<Value, Unverifiable> {
        match self.peek().ok_or(FORMAT)? {
            b'{' | b'[' if depth >= RESOLVER_DEPTH => Err(Unverifiable::Bound(Bound::ResolverDepth)),
            b'{' => self.object(depth + 1),
            b'[' => self.array(depth + 1),
            b'"' => Ok(Value::Scalar(Scalar::Str(self.string()?))),
            b't' => self.word("true", Scalar::Bool(true)),
            b'f' => self.word("false", Scalar::Bool(false)),
            b'n' => self.word("null", Scalar::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(FORMAT),
        }
    }

    fn word(&mut self, w: &str, s: Scalar) -> Result<Value, Unverifiable> {
        if self.src.get(self.at..).is_some_and(|rest| rest.starts_with(w)) {
            self.at += w.len();
            Ok(Value::Scalar(s))
        } else {
            Err(FORMAT)
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, Unverifiable> {
        self.expect(b'{')?;
        let mut m = MapBuilder::default();
        self.space();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(m.finish(false));
        }
        loop {
            self.space();
            if self.peek() != Some(b'"') {
                return Err(FORMAT);
            }
            let key = self.string()?;
            self.space();
            self.expect(b':')?;
            self.space();
            let v = self.value(depth)?;
            m.insert(key, v);
            self.space();
            match self.next() {
                Some(b',') => {}
                Some(b'}') => return Ok(m.finish(false)),
                _ => return Err(FORMAT),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, Unverifiable> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.space();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Value::List(items));
        }
        loop {
            self.space();
            items.push(self.value(depth)?);
            self.space();
            match self.next() {
                Some(b',') => {}
                Some(b']') => return Ok(Value::List(items)),
                _ => return Err(FORMAT),
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.at += 1;
        }
        self.at - start
    }

    /// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][-+]?[0-9]+)?`, exactly.
    fn number(&mut self) -> Result<Value, Unverifiable> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(FORMAT),
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if self.digits() == 0 {
                return Err(FORMAT);
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return Err(FORMAT);
            }
        }
        let text = self.src.get(start..self.at).ok_or(FORMAT)?;
        let s = if text.len() > NUMBER_TEXT_MAX {
            Scalar::Opaque(Unverifiable::Bound(Bound::NumberText))
        } else {
            // `None` only when the exponent does not fit an i64: no exact value can be held.
            Decimal::parse_number(text).map_or(Scalar::Opaque(Unverifiable::NotComparable), Scalar::Number)
        };
        Ok(Value::Scalar(s))
    }

    fn hex4(&mut self) -> Result<u32, Unverifiable> {
        let h = self.src.get(self.at..self.at + 4).ok_or(FORMAT)?;
        if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(FORMAT);
        }
        self.at += 4;
        u32::from_str_radix(h, 16).map_err(|_| FORMAT)
    }

    /// A string; a lone surrogate escape is `format` (no Unicode text holds it).
    fn string(&mut self) -> Result<String, Unverifiable> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let run = self.at;
            while self.peek().is_some_and(|b| b != b'"' && b != b'\\' && b >= 0x20) {
                self.at += 1;
            }
            // The run stops at an ASCII byte or the end, so it is whole characters.
            out.push_str(self.src.get(run..self.at).ok_or(FORMAT)?);
            match self.next().ok_or(FORMAT)? {
                b'"' => return Ok(out),
                b'\\' => {}
                _ => return Err(FORMAT),
            }
            let c = match self.next().ok_or(FORMAT)? {
                b'"' => '"',
                b'\\' => '\\',
                b'/' => '/',
                b'b' => '\u{8}',
                b'f' => '\u{c}',
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                b'u' => {
                    let hi = self.hex4()?;
                    let code = match hi {
                        0xD800..=0xDBFF => {
                            self.expect(b'\\')?;
                            self.expect(b'u')?;
                            let lo = self.hex4()?;
                            if !(0xDC00..=0xDFFF).contains(&lo) {
                                return Err(FORMAT);
                            }
                            0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                        }
                        0xDC00..=0xDFFF => return Err(FORMAT),
                        c => c,
                    };
                    char::from_u32(code).ok_or(FORMAT)?
                }
                _ => return Err(FORMAT),
            };
            out.push(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bounds::{NUMBER_TEXT_MAX, RESOLVER_DEPTH};
    use crate::reason::Bound;
    use crate::scalar::{Decimal, Scalar};
    use proptest::prelude::*;

    fn num(s: &str) -> Value {
        Value::Scalar(Scalar::Number(Decimal::parse_number(s).unwrap()))
    }
    fn st(s: &str) -> Value {
        Value::Scalar(Scalar::Str(s.into()))
    }
    fn map(entries: Vec<(&str, Value)>) -> Value {
        Value::Map { entries: entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(), merge_key: false }
    }

    #[test]
    fn json_duplicate_key_defined_twice() {
        assert_eq!(
            read_json(r#"{"a":1,"b":2,"a":3}"#),
            Ok(map(vec![("a", Value::Opaque(Unverifiable::DefinedTwice)), ("b", num("2"))]))
        );
        // The same key spelled with an escape is the same key.
        assert_eq!(read_json(r#"{"a":1,"a":2}"#), Ok(map(vec![("a", Value::Opaque(Unverifiable::DefinedTwice))])));
    }

    #[test]
    fn json_numbers_exact() {
        let v = read_json(r#"[0.1, 1e400, 123456789012345678901234567890, -0, 1E+2, 2.50e-3]"#).unwrap();
        assert_eq!(
            v,
            Value::List(vec![
                num("0.1"),
                num("1e400"),
                num("123456789012345678901234567890"),
                num("0"),
                num("100"),
                num("0.0025"),
            ])
        );
    }

    #[test]
    fn json_number_grammar() {
        for t in ["01", "+1", ".5", "1.", "1e", "1e+", "-", "--1", "NaN", "Infinity", "-Infinity", "0x1F", "1_000", "1.5.2", "١"] {
            assert_eq!(read_json(t), Err(Unverifiable::Format), "{t}");
        }
        assert_eq!(read_json("0"), Ok(num("0")));
        assert_eq!(read_json("-0.0e-0"), Ok(num("0")));
    }

    #[test]
    fn json_exponent_beyond_i64_is_not_comparable() {
        assert_eq!(read_json("1e99999999999999999999"), Ok(Value::Scalar(Scalar::Opaque(Unverifiable::NotComparable))));
    }

    #[test]
    fn json_long_number_is_a_bound() {
        let at = "1".repeat(NUMBER_TEXT_MAX);
        assert!(matches!(read_json(&at), Ok(Value::Scalar(Scalar::Number(_)))));
        let over = "1".repeat(NUMBER_TEXT_MAX + 1);
        assert_eq!(read_json(&over), Ok(Value::Scalar(Scalar::Opaque(Unverifiable::Bound(Bound::NumberText)))));
    }

    #[test]
    fn json_rejects_comments_and_trailing_commas() {
        assert_eq!(read_json("{\"a\":1,}"), Err(Unverifiable::Format));
        assert_eq!(read_json("// c\n{}"), Err(Unverifiable::Format));
        for t in ["[1,]", "{,}", "[,1]", "{'a':1}", "{\"a\" 1}", "{\"a\":}", "{a:1}", "/* c */ 1", "[1 2]", "{\"a\":1 \"b\":2}"] {
            assert_eq!(read_json(t), Err(Unverifiable::Format), "{t}");
        }
    }

    #[test]
    fn json_document_edges() {
        for t in ["", " ", "{} {}", "[] x", "\u{feff}\u{feff}{}", "\u{a0}{}", "\u{c}{}", "nul", "truee", "True"] {
            assert_eq!(read_json(t), Err(Unverifiable::Format), "{t:?}");
        }
        // C21: one BOM is skipped. A top-level scalar is a JSON text (RFC 8259 §2).
        assert_eq!(read_json("\u{feff}{}"), Ok(map(vec![])));
        assert_eq!(read_json(" \t\r\n true \n"), Ok(Value::Scalar(Scalar::Bool(true))));
        assert_eq!(read_json("null"), Ok(Value::Scalar(Scalar::Null)));
        assert_eq!(read_json("\"x\""), Ok(st("x")));
    }

    #[test]
    fn json_strings() {
        assert_eq!(read_json(r#""a\"b\\c\/d\b\f\n\r\t""#), Ok(st("a\"b\\c/d\u{8}\u{c}\n\r\t")));
        assert_eq!(read_json(r#""é😀""#), Ok(st("é😀")));
        assert_eq!(read_json("\"café ✓\""), Ok(st("café ✓")));
        for t in [r#""\uD83D""#, r#""\uDE00""#, r#""\uD83Dx""#, r#""\uD83DA""#, r#""\uD83D\u0041""#, r#""\uD83D\uD83D""#, r#""\x41""#, r#""\u12""#, r#""\u12G4""#, "\"a\u{1}b\"", "\"a\nb\"", "\"abc", r#""\"#] {
            assert_eq!(read_json(t), Err(Unverifiable::Format), "{t:?}");
        }
    }

    #[test]
    fn json_depth_bound() {
        let nest = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(read_json(&nest(RESOLVER_DEPTH)).is_ok());
        assert_eq!(read_json(&nest(RESOLVER_DEPTH + 1)), Err(Unverifiable::Bound(Bound::ResolverDepth)));
        // Deep enough to overflow a recursive reader's stack.
        assert_eq!(read_json(&nest(1_000_000)), Err(Unverifiable::Bound(Bound::ResolverDepth)));
        let objs = format!("{}1{}", "{\"a\":".repeat(RESOLVER_DEPTH + 1), "}".repeat(RESOLVER_DEPTH + 1));
        assert_eq!(read_json(&objs), Err(Unverifiable::Bound(Bound::ResolverDepth)));
    }

    #[test]
    fn json_keeps_key_order() {
        assert_eq!(read_json(r#"{"z":true,"a":false,"m":[]}"#), Ok(map(vec![("z", Value::Scalar(Scalar::Bool(true))), ("a", Value::Scalar(Scalar::Bool(false))), ("m", Value::List(vec![]))])));
    }

    #[test]
    fn json_attack_pass() {
        assert_eq!(read_json(r#""\u0000""#), Ok(st("\0")));
        assert_eq!(read_json(r#""\uDBFF\uDFFF\u00E9""#), Ok(st("\u{10FFFF}é")));
        assert_eq!(read_json("\"\u{7f}\""), Ok(st("\u{7f}")));
        // A \u escape whose four bytes end inside a multibyte character.
        for t in ["\"\\u12é\"", "\"\\uD83D\\uDEé\"", "-01", "1true", "[-]", "{1:2}", "[1,2,", "{\"a\":1", "\"a\" : 1", "[1]]", "{\"a\":1}}"] {
            assert_eq!(read_json(t), Err(Unverifiable::Format), "{t:?}");
        }
        // The bound counts the sign: the number is as written.
        let signed = format!("-{}", "1".repeat(NUMBER_TEXT_MAX));
        assert_eq!(read_json(&signed), Ok(Value::Scalar(Scalar::Opaque(Unverifiable::Bound(Bound::NumberText)))));
        // A depth failure deep inside stops the read wherever it is.
        let mixed = format!("{{\"a\":{}{}}}", "[".repeat(RESOLVER_DEPTH), "]".repeat(RESOLVER_DEPTH));
        assert_eq!(read_json(&mixed), Err(Unverifiable::Bound(Bound::ResolverDepth)));
    }

    fn json_value() -> impl Strategy<Value = serde_json::Value> {
        let leaf = prop_oneof![
            Just(serde_json::Value::Null),
            any::<bool>().prop_map(serde_json::Value::Bool),
            any::<i64>().prop_map(|n| serde_json::json!(n)),
            any::<u64>().prop_map(|n| serde_json::json!(n)),
            any::<f64>().prop_filter("finite", |f| f.is_finite()).prop_map(|f| serde_json::json!(f)),
            any::<String>().prop_map(serde_json::Value::String),
        ];
        leaf.prop_recursive(6, 64, 8, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..8).prop_map(serde_json::Value::Array),
                prop::collection::vec((any::<String>(), inner), 0..8)
                    .prop_map(|kv| serde_json::Value::Object(kv.into_iter().collect())),
            ]
        })
    }

    /// serde_json's value as ours: numbers by their text, keys in serde_json's order (it has no duplicates).
    fn expected(v: &serde_json::Value) -> Value {
        match v {
            serde_json::Value::Null => Value::Scalar(Scalar::Null),
            serde_json::Value::Bool(b) => Value::Scalar(Scalar::Bool(*b)),
            serde_json::Value::Number(n) => num(&n.to_string()),
            serde_json::Value::String(s) => st(s),
            serde_json::Value::Array(a) => Value::List(a.iter().map(expected).collect()),
            serde_json::Value::Object(o) => {
                Value::Map { entries: o.iter().map(|(k, v)| (k.clone(), expected(v))).collect(), merge_key: false }
            }
        }
    }

    proptest! {
        #[test]
        fn json_agrees_with_serde_json_on_valid_input(v in json_value(), pretty in any::<bool>()) {
            let text = if pretty { serde_json::to_string_pretty(&v).unwrap() } else { serde_json::to_string(&v).unwrap() };
            prop_assert_eq!(read_json(&text), Ok(expected(&v)));
        }

        #[test]
        fn json_rejects_what_serde_json_rejects(s in "[\\[\\]{}\":,0-9a-z.eE+\\- \\\\]{0,12}") {
            let theirs = serde_json::from_str::<serde_json::Value>(&s);
            // serde_json holds numbers as f64 and refuses one beyond it; this reader keeps it exactly.
            if theirs.as_ref().is_err_and(|e| e.to_string().starts_with("number out of range")) {
                return Ok(());
            }
            if theirs.is_err() {
                prop_assert_eq!(read_json(&s), Err(Unverifiable::Format), "{:?}", s);
            } else {
                prop_assert!(read_json(&s).is_ok(), "{:?}", s);
            }
        }
    }
}
