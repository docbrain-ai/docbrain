// SPDX-License-Identifier: MIT
//! The value tree a resolver reads an anchored file into (`SEMANTICS.md` §Values and comparison,
//! §Resolving values). Mapping keys keep file order; a key defined twice is stored once, as
//! `Value::Opaque(DefinedTwice)`.

use std::collections::HashMap;

use crate::reason::Unverifiable;
use crate::scalar::Scalar;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Scalar(Scalar),
    /// `merge_key`: the mapping has a YAML `<<` key, so an absent key is `unverifiable: merge-key`.
    Map { entries: Vec<(String, Value)>, merge_key: bool },
    List(Vec<Value>),
    Opaque(Unverifiable),
}

impl Value {
    /// An estimate of the heap this value holds below itself: every node, map entry and text, without the
    /// allocator's own overhead. Walked with one iterator per level, so it takes no memory per node.
    pub(crate) fn heap(&self) -> usize {
        enum Kids<'a> {
            List(std::slice::Iter<'a, Value>),
            Map(std::slice::Iter<'a, (String, Value)>),
        }
        fn kids(v: &Value) -> Option<Kids<'_>> {
            match v {
                Value::List(items) => Some(Kids::List(items.iter())),
                Value::Map { entries, .. } => Some(Kids::Map(entries.iter())),
                Value::Scalar(_) | Value::Opaque(_) => None,
            }
        }
        fn own(v: &Value) -> usize {
            match v {
                Value::Scalar(s) => s.heap(),
                Value::Map { entries, .. } => entries
                    .iter()
                    .fold(entries.capacity().saturating_mul(std::mem::size_of::<(String, Value)>()), |n, (k, _)| n.saturating_add(k.capacity())),
                Value::List(items) => items.capacity().saturating_mul(std::mem::size_of::<Value>()),
                Value::Opaque(_) => 0,
            }
        }
        let mut n = own(self);
        let mut stack: Vec<Kids<'_>> = kids(self).into_iter().collect();
        while let Some(top) = stack.last_mut() {
            let next = match top {
                Kids::List(i) => i.next(),
                Kids::Map(i) => i.next().map(|(_, v)| v),
            };
            match next {
                Some(v) => {
                    n = n.saturating_add(own(v));
                    stack.extend(kids(v));
                }
                None => {
                    stack.pop();
                }
            }
        }
        n
    }
}

/// Builds a mapping in file order, in time linear in its keys.
#[derive(Debug, Default)]
pub struct MapBuilder {
    entries: Vec<(String, Value)>,
    index: HashMap<String, usize>,
}

impl MapBuilder {
    /// Adds `key`; a key already present keeps its first position and becomes `Opaque(DefinedTwice)`.
    pub fn insert(&mut self, key: String, value: Value) {
        match self.index.get(&key) {
            Some(&i) => {
                if let Some(e) = self.entries.get_mut(i) {
                    e.1 = Value::Opaque(Unverifiable::DefinedTwice);
                }
            }
            None => {
                self.index.insert(key.clone(), self.entries.len());
                self.entries.push((key, value));
            }
        }
    }

    pub fn finish(self, merge_key: bool) -> Value {
        Value::Map { entries: self.entries, merge_key }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn str_value(s: &str) -> Value {
        Value::Scalar(Scalar::Str(s.to_owned()))
    }

    #[test]
    fn heap_counts_every_node_key_and_text() {
        // {"k": <1,000 bytes>, "l": ["ab", "cd"]}: the map's entries and keys, the text, the list and its texts.
        let list = Value::List(vec![str_value("ab"), str_value("cd")]);
        let v = Value::Map { entries: vec![("k".to_owned(), str_value(&"x".repeat(1000))), ("l".to_owned(), list)], merge_key: false };
        let want = 2 * std::mem::size_of::<(String, Value)>() + 2 + 1000 + 2 * std::mem::size_of::<Value>() + 4;
        assert_eq!(v.heap(), want);
    }

    #[test]
    fn a_key_defined_twice_is_stored_once() {
        let mut m = MapBuilder::default();
        m.insert("a".into(), str_value("1"));
        m.insert("b".into(), str_value("2"));
        m.insert("a".into(), str_value("3"));
        m.insert("a".into(), str_value("4"));
        let want = Value::Map {
            entries: vec![("a".into(), Value::Opaque(Unverifiable::DefinedTwice)), ("b".into(), str_value("2"))],
            merge_key: false,
        };
        assert_eq!(m.finish(false), want);
    }

    #[test]
    fn keys_keep_file_order_and_merge_flag() {
        let mut m = MapBuilder::default();
        for k in ["z", "a", "m"] {
            m.insert(k.into(), str_value(k));
        }
        let Value::Map { entries, merge_key } = m.finish(true) else { panic!("a map") };
        assert!(merge_key);
        assert_eq!(entries.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["z", "a", "m"]);
    }

    #[test]
    fn many_keys_build_in_linear_time() {
        let mut m = MapBuilder::default();
        for i in 0..200_000 {
            m.insert(format!("k{i}"), Value::Scalar(Scalar::Null));
        }
        m.insert("k0".into(), Value::Scalar(Scalar::Null));
        let Value::Map { entries, .. } = m.finish(false) else { panic!("a map") };
        assert_eq!(entries.len(), 200_000);
        assert_eq!(entries[0].1, Value::Opaque(Unverifiable::DefinedTwice));
    }
}
