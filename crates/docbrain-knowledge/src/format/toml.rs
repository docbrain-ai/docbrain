// SPDX-License-Identifier: MIT
//! TOML through `toml_edit` (`SEMANTICS.md` §Resolving values): numbers exactly from their source text.

use toml_edit::{DocumentMut, Item, Repr, Table, Value as TomlValue};

use crate::bounds::{NUMBER_TEXT_MAX, RESOLVER_DEPTH};
use crate::reason::{Bound, Unverifiable};
use crate::scalar::{Decimal, Scalar};
use crate::value::{MapBuilder, Value};

/// Reads a TOML document. A key or table defined twice is `defined-twice`; any other parse error is `format`.
pub fn read_toml(src: &str) -> Result<Value, Unverifiable> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    // One BOM is skipped (`SEMANTICS.md` §Resolving values). toml_edit would skip a second one itself.
    if src.starts_with('\u{feff}') {
        return Err(Unverifiable::Format);
    }
    let doc: DocumentMut = src.parse().map_err(|e: toml_edit::TomlError| classify(e.message()))?;
    table(doc.as_table(), 1)
}

/// toml_edit reports a key or table defined twice, and its own nesting limit, only in its message, after an
/// optional context line. A message quotes source text only in those two defined-twice forms, as the key, so
/// they are tested first: a key spelled like another message cannot change the answer.
fn classify(message: &str) -> Unverifiable {
    if message.contains("duplicate key `") || message.contains("` attempted to extend non-table type (") {
        Unverifiable::DefinedTwice
    } else if message.contains("recursion limit exceeded") {
        Unverifiable::Bound(Bound::ResolverDepth)
    } else {
        Unverifiable::Format
    }
}

fn deeper(depth: usize) -> Result<usize, Unverifiable> {
    if depth >= RESOLVER_DEPTH { Err(Unverifiable::Bound(Bound::ResolverDepth)) } else { Ok(depth + 1) }
}

/// `depth` counts the maps and lists from the root, the root being 1.
fn table(t: &Table, depth: usize) -> Result<Value, Unverifiable> {
    let mut m = MapBuilder::default();
    for (k, item) in t.iter() {
        let v = match item {
            Item::None => continue,
            Item::Value(v) => value(v, depth)?,
            Item::Table(t) => table(t, deeper(depth)?)?,
            Item::ArrayOfTables(a) => {
                let list = deeper(depth)?;
                let map = deeper(list)?;
                Value::List(a.iter().map(|t| table(t, map)).collect::<Result<_, _>>()?)
            }
        };
        m.insert(k.to_owned(), v);
    }
    Ok(m.finish(false))
}

/// A value's text as written (toml_edit's `ValueRepr` bound cannot be named, so this takes the repr).
fn raw(repr: Option<&Repr>) -> Option<&str> {
    repr.and_then(|r| r.as_raw().as_str())
}

fn number_value(text: &str) -> Value {
    Value::Scalar(Decimal::parse_number(text).map_or(Scalar::Opaque(Unverifiable::NotComparable), Scalar::Number))
}

/// A value inside a table at `depth`.
fn value(v: &TomlValue, depth: usize) -> Result<Value, Unverifiable> {
    let too_long = |text: Option<&str>| text.is_some_and(|t| t.len() > NUMBER_TEXT_MAX);
    let bound = Value::Scalar(Scalar::Opaque(Unverifiable::Bound(Bound::NumberText)));
    Ok(match v {
        TomlValue::String(f) => Value::Scalar(Scalar::Str(f.value().clone())),
        TomlValue::Boolean(f) => Value::Scalar(Scalar::Bool(*f.value())),
        // An integer fits an i64 (toml_edit refuses one that does not), so its value is exact.
        TomlValue::Integer(f) if too_long(raw(f.as_repr())) => bound,
        TomlValue::Integer(f) => number_value(&f.value().to_string()),
        TomlValue::Float(f) => {
            let text = raw(f.as_repr()).ok_or(Unverifiable::Format)?;
            let bare = text.trim_start_matches(['+', '-']);
            if text.len() > NUMBER_TEXT_MAX {
                bound
            } else if bare == "inf" || bare == "nan" {
                Value::Scalar(Scalar::Opaque(Unverifiable::NotComparable))
            } else if f.value().is_infinite() {
                // toml_edit refuses a float beyond f64 when it is positive and reads it as -inf when it is
                // negative: both are `format` here.
                return Err(Unverifiable::Format);
            } else {
                number_value(&text.replace('_', ""))
            }
        }
        TomlValue::Datetime(f) => Value::Scalar(Scalar::Datetime(raw(f.as_repr()).map_or_else(|| f.value().to_string(), str::to_owned))),
        TomlValue::Array(a) => {
            let inner = deeper(depth)?;
            Value::List(a.iter().map(|v| value(v, inner)).collect::<Result<_, _>>()?)
        }
        TomlValue::InlineTable(t) => {
            let inner = deeper(depth)?;
            let mut m = MapBuilder::default();
            for (k, v) in t.iter() {
                m.insert(k.to_owned(), value(v, inner)?);
            }
            m.finish(false)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(s: &str) -> Value {
        Value::Scalar(Scalar::Number(Decimal::parse_number(s).unwrap()))
    }
    fn st(s: &str) -> Value {
        Value::Scalar(Scalar::Str(s.into()))
    }
    fn map(entries: Vec<(&str, Value)>) -> Value {
        Value::Map { entries: entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(), merge_key: false }
    }
    fn get<'a>(v: &'a Value, key: &str) -> &'a Value {
        match v {
            Value::Map { entries, .. } => &entries.iter().find(|(k, _)| k == key).unwrap().1,
            other => panic!("not a map: {other:?}"),
        }
    }

    #[test]
    fn toml_duplicate_is_defined_twice() {
        assert_eq!(read_toml("a = 1\na = 2\n"), Err(Unverifiable::DefinedTwice));
        for t in ["[a]\nx = 1\n[a]\ny = 2\n", "a = 1\n[a]\n", "t = { a = 1, a = 2 }\n", "a = 1\na.b = 2\n", "[a]\nb = 1\n[a.b]\n"] {
            assert_eq!(read_toml(t), Err(Unverifiable::DefinedTwice), "{t:?}");
        }
    }

    #[test]
    fn toml_syntax_error_is_format() {
        for t in ["a = ", "a = 1 2", "[a", "a = 01", "= 1", "a = 1e400", "a = 0x", "a = 9223372036854775808", "a = 'x\n"] {
            assert_eq!(read_toml(t), Err(Unverifiable::Format), "{t:?}");
        }
    }

    #[test]
    fn toml_cargo_lock_shape() {
        let v = read_toml("version = 4\n\n[[package]]\nname = \"a\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"b\"\nversion = \"0.2.1\"\ndependencies = [\"a\"]\n").unwrap();
        assert_eq!(
            v,
            map(vec![
                ("version", num("4")),
                (
                    "package",
                    Value::List(vec![
                        map(vec![("name", st("a")), ("version", st("1.0.0"))]),
                        map(vec![("name", st("b")), ("version", st("0.2.1")), ("dependencies", Value::List(vec![st("a")]))]),
                    ])
                ),
            ])
        );
    }

    #[test]
    fn toml_numbers_and_dates() {
        let v = read_toml(concat!(
            "hex = 0x1F\nunder = 1_000\nf = 3.10\nd = 1979-05-27\noct = 0o17\nbin = 0b101\nneg = -17\n",
            "uf = 1_000.5_5\ntiny = 1e-400\nexp = 6.626e-34\nplus = +2\n",
            "odt = 1979-05-27T07:32:00Z\nspace = 1979-05-27 07:32:00.999-07:00\nldt = 1979-05-27T00:32:00\nlt = 07:32:00\n",
        ))
        .unwrap();
        assert_eq!(get(&v, "hex"), &num("31"));
        assert_eq!(get(&v, "under"), &num("1000"));
        assert_eq!(get(&v, "f"), &num("3.1"));
        assert_eq!(get(&v, "d"), &Value::Scalar(Scalar::Datetime("1979-05-27".into())));
        assert_eq!(get(&v, "oct"), &num("15"));
        assert_eq!(get(&v, "bin"), &num("5"));
        assert_eq!(get(&v, "neg"), &num("-17"));
        assert_eq!(get(&v, "uf"), &num("1000.55"));
        assert_eq!(get(&v, "tiny"), &num("1e-400"));
        assert_eq!(get(&v, "exp"), &num("0.0000000000000000000000000000000006626"));
        assert_eq!(get(&v, "plus"), &num("2"));
        // A datetime is compared as written.
        assert_eq!(get(&v, "odt"), &Value::Scalar(Scalar::Datetime("1979-05-27T07:32:00Z".into())));
        assert_eq!(get(&v, "space"), &Value::Scalar(Scalar::Datetime("1979-05-27 07:32:00.999-07:00".into())));
        assert_eq!(get(&v, "ldt"), &Value::Scalar(Scalar::Datetime("1979-05-27T00:32:00".into())));
        assert_eq!(get(&v, "lt"), &Value::Scalar(Scalar::Datetime("07:32:00".into())));
    }

    #[test]
    fn toml_inf_and_nan_are_not_comparable() {
        for t in ["inf", "+inf", "-inf", "nan", "+nan", "-nan"] {
            assert_eq!(read_toml(&format!("a = {t}\n")).map(|v| get(&v, "a").clone()), Ok(Value::Scalar(Scalar::Opaque(Unverifiable::NotComparable))), "{t}");
        }
    }

    #[test]
    fn toml_float_overflow_is_format_both_signs() {
        // toml_edit refuses 1e400 and accepts -1e400 as -inf; both signs read the same here.
        assert_eq!(read_toml("a = 1e400\n"), Err(Unverifiable::Format));
        assert_eq!(read_toml("a = -1e400\n"), Err(Unverifiable::Format));
    }

    #[test]
    fn toml_long_number_is_a_bound() {
        let at = format!("a = 0x{}1\n", "0".repeat(NUMBER_TEXT_MAX - 3));
        assert_eq!(read_toml(&at).map(|v| get(&v, "a").clone()), Ok(num("1")));
        let over = format!("a = 0x{}1\n", "0".repeat(NUMBER_TEXT_MAX - 2));
        assert_eq!(read_toml(&over).map(|v| get(&v, "a").clone()), Ok(Value::Scalar(Scalar::Opaque(Unverifiable::Bound(Bound::NumberText)))));
        let long_float = format!("a = 0.{}1\n", "0".repeat(NUMBER_TEXT_MAX));
        assert_eq!(read_toml(&long_float).map(|v| get(&v, "a").clone()), Ok(Value::Scalar(Scalar::Opaque(Unverifiable::Bound(Bound::NumberText)))));
        // A float's text may be exactly the bound long.
        let text = format!("1.{}", "0".repeat(NUMBER_TEXT_MAX - 2));
        assert_eq!(read_toml(&format!("a = {text}\n")).map(|v| get(&v, "a").clone()), Ok(num(&text)));
        let over = format!("a = {text}0\n");
        assert_eq!(read_toml(&over).map(|v| get(&v, "a").clone()), Ok(Value::Scalar(Scalar::Opaque(Unverifiable::Bound(Bound::NumberText)))));
    }

    #[test]
    fn toml_scalars_tables_and_order() {
        let v = read_toml("\u{feff}z = true\na = 'lit'\nb = \"\"\"\nml\"\"\"\n[t]\nx.y = { k = [1, 'two'] }\n").unwrap();
        assert_eq!(
            v,
            map(vec![
                ("z", Value::Scalar(Scalar::Bool(true))),
                ("a", st("lit")),
                ("b", st("ml")),
                ("t", map(vec![("x", map(vec![("y", map(vec![("k", Value::List(vec![num("1"), st("two")]))]))]))])),
            ])
        );
        assert_eq!(read_toml(""), Ok(map(vec![])));
        assert_eq!(read_toml("\"a.b\" = 1\n"), Ok(map(vec![("a.b", num("1"))])));
    }

    #[test]
    fn toml_depth_bound() {
        // Root table is level 1, as in the YAML reader: 63 more nested arrays reach the bound.
        let arrays = |n: usize| format!("a = {}{}\n", "[".repeat(n), "]".repeat(n));
        assert!(read_toml(&arrays(RESOLVER_DEPTH - 1)).is_ok());
        assert_eq!(read_toml(&arrays(RESOLVER_DEPTH)), Err(Unverifiable::Bound(Bound::ResolverDepth)));
        // Beyond toml_edit's own recursion limit (80).
        assert_eq!(read_toml(&arrays(500)), Err(Unverifiable::Bound(Bound::ResolverDepth)));
        let dotted = |n: usize| format!("{} = 1\n", vec!["k"; n].join("."));
        // `a.b = 1` is the root and one table: n segments are n levels.
        assert!(read_toml(&dotted(RESOLVER_DEPTH)).is_ok());
        assert_eq!(read_toml(&dotted(RESOLVER_DEPTH + 1)), Err(Unverifiable::Bound(Bound::ResolverDepth)));
        let header = |n: usize| format!("[{}]\nv = 1\n", vec!["h"; n].join("."));
        assert_eq!(read_toml(&header(RESOLVER_DEPTH)), Err(Unverifiable::Bound(Bound::ResolverDepth)));
    }

    #[test]
    fn toml_deep_dotted_keys_cannot_overflow_the_stack() {
        // toml_edit builds dotted keys without recursion and drops them with it: 200,000 levels would abort.
        let n = 200_000;
        assert_eq!(read_toml(&format!("{} = 1\n", vec!["k"; n].join("."))), Err(Unverifiable::Bound(Bound::ResolverDepth)));
        assert_eq!(read_toml(&format!("[{}]\n", vec!["k"; n].join("."))), Err(Unverifiable::Bound(Bound::ResolverDepth)));
        // toml_edit reports an inline table's dotted key of 80 or more segments as a syntax error, not as its
        // recursion limit: `format`, still unverifiable, and still no overflow.
        assert_eq!(read_toml(&format!("t = {{ {} = 1 }}\n", vec!["\"k\""; n].join(" . "))), Err(Unverifiable::Format));
        // Below toml_edit's limit, this reader's bound applies inside an inline table too.
        assert_eq!(read_toml(&format!("t = {{ {} = 1 }}\n", vec!["k"; 70].join("."))), Err(Unverifiable::Bound(Bound::ResolverDepth)));
    }

    #[test]
    fn toml_attack_pass() {
        let v = read_toml("z = -0.0\nmin = -9223372036854775808\nlow = 1979-05-27t07:32:00z\n[e]\n").unwrap();
        assert_eq!(get(&v, "z"), &num("0"));
        assert_eq!(get(&v, "min"), &num("-9223372036854775808"));
        assert_eq!(get(&v, "low"), &Value::Scalar(Scalar::Datetime("1979-05-27t07:32:00z".into())));
        assert_eq!(get(&v, "e"), &map(vec![]));
        assert_eq!(read_toml("a = 1\r\nb = 'x'\r\n"), Ok(map(vec![("a", num("1")), ("b", st("x"))])));
        assert_eq!(read_toml("\u{feff}\u{feff}a = 1\n"), Err(Unverifiable::Format));
        // Arrays of tables inside arrays of tables: each list and each map is a level.
        let v = read_toml("[[a]]\n[[a.b]]\nc = 1\n[[a.b]]\nc = 2\n").unwrap();
        assert_eq!(
            v,
            map(vec![("a", Value::List(vec![map(vec![("b", Value::List(vec![map(vec![("c", num("1"))]), map(vec![("c", num("2"))])]))])]))])
        );
        // `[[k]]`, `[[k.k]]`, ...: n nested arrays of tables, each a list and a map below the root.
        let aot = |n: usize| (1..=n).map(|i| format!("[[{}]]\n", vec!["k"; i].join("."))).collect::<String>();
        assert!(read_toml(&aot(31)).is_ok());
        assert_eq!(read_toml(&aot(32)), Err(Unverifiable::Bound(Bound::ResolverDepth)));
    }

    #[test]
    fn toml_messages_cannot_be_spoofed_by_keys() {
        // A key spelled like the recursion message, defined twice, is still defined twice.
        assert_eq!(read_toml("'recursion limit exceeded' = 1\n'recursion limit exceeded' = 2\n"), Err(Unverifiable::DefinedTwice));
        assert_eq!(read_toml("\"x\\nrecursion limit exceeded\" = 1\n\"x\\nrecursion limit exceeded\".b = 2\n"), Err(Unverifiable::DefinedTwice));
        // A string value spelled like a message is a string.
        assert_eq!(read_toml("a = \"duplicate key `a`\"\n"), Ok(map(vec![("a", st("duplicate key `a`"))])));
        for t in ["[a]\nb = 1\n[a.b]\n", "[[a]]\n[a]\n", "a = [ { b.c = 1, b = 2 } ]\n", "[a.b]\n[a]\nb = 1\n"] {
            assert_eq!(read_toml(t), Err(Unverifiable::DefinedTwice), "{t:?}");
        }
    }
}
