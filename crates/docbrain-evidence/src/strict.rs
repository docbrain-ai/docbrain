// SPDX-License-Identifier: MIT
//! The `.dbev` **strict-JSON profile** — the single deterministic rule every
//! JSON member of a bundle must satisfy, enforced IDENTICALLY here and in the
//! stdlib Python reference verifier (`tools/verify_dbev.py`'s `json_parse`).
//!
//! ## Why this exists
//! The two verifiers diverged repeatedly on hostile self-signed bundles because
//! Rust parsed per-type (strict `deny_unknown_fields` structs, tolerant peeks
//! that lazily skip ignored fields, and `serde_json::Value` fields whose map
//! decode silently KEEPS-LAST on a duplicate key) while Python used global
//! `json.loads` hooks. The fix is to run ONE strict pre-validation on every
//! JSON member's bytes BEFORE the type-specific parse, on both sides. Same
//! deterministic rule + same bytes ⇒ the two verifiers cannot diverge on
//! JSON-parse ACCEPTANCE — a proof, not "the gate happened to find nothing".
//!
//! ## The profile (a conforming member)
//! 1. **Valid UTF-8** — the whole member decodes as strict UTF-8.
//! 2. **No duplicate keys** at ANY nesting level.
//! 3. **Nesting depth < 128** — serde_json's `RECURSION_LIMIT` (127 open
//!    brackets accept, the 128th rejects).
//! 4. **All numbers finite and in serde_json's representable range.**
//! 5. **No bareword constants** — `NaN`/`Infinity`/`-Infinity` rejected.
//! 6. **Standard JSON syntax** — no trailing data, comments, leading zeros,
//!    `+`, bad escapes, or raw control chars in strings.
//!
//! A single `serde_json::from_slice::<NoDupKeys>` (a `deserialize_any` visitor)
//! delivers axes 1,3,4,5,6 for free (it eagerly materializes every value with
//! `serde_json::Value` semantics — ignored fields included) PLUS axis 2 (the
//! `Value` map decode keeps-last on a dup; this visitor instead sees the second
//! `next_key` on serde_json's non-deduplicating streaming `MapAccess` and
//! errors). It is the Rust twin of Python's `json_parse` strict hooks.

use serde::de::DeserializeOwned;
use serde::Deserialize;

/// Deserialize-and-discard any JSON value, failing on the FIRST duplicate key
/// at ANY nesting level, and (via `deserialize_any`'s eager `serde_json::Value`
/// materialization) on invalid UTF-8, over-limit nesting, non-finite/out-of-
/// range numbers, and `NaN`/`Infinity` barewords. This IS the strict-JSON
/// profile above; [`from_slice_strict`] runs it as a pre-pass before the typed
/// parse. Recursion matches Python's `object_pairs_hook`: the map visitor fires
/// at EVERY object level, so a nested `{"x":{"a":1,"a":2}}` is rejected too, not
/// just top-level repeats.
pub(crate) struct NoDupKeys;

impl<'de> Deserialize<'de> for NoDupKeys {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct AnyValue;
        impl<'de> serde::de::Visitor<'de> for AnyValue {
            type Value = ();

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("any JSON value with no duplicate object keys")
            }

            fn visit_bool<E>(self, _: bool) -> Result<(), E> {
                Ok(())
            }
            fn visit_i64<E>(self, _: i64) -> Result<(), E> {
                Ok(())
            }
            fn visit_u64<E>(self, _: u64) -> Result<(), E> {
                Ok(())
            }
            fn visit_f64<E>(self, _: f64) -> Result<(), E> {
                Ok(())
            }
            fn visit_str<E>(self, _: &str) -> Result<(), E> {
                Ok(())
            }
            fn visit_unit<E>(self) -> Result<(), E> {
                Ok(())
            }
            fn visit_none<E>(self) -> Result<(), E> {
                Ok(())
            }

            fn visit_some<D>(self, deserializer: D) -> Result<(), D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                deserializer.deserialize_any(AnyValue)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<(), A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                // Recurse into every element so a dup nested in an array is
                // caught too (parity with the Python hook firing at every level).
                while seq.next_element::<NoDupKeys>()?.is_some() {}
                Ok(())
            }

            fn visit_map<A>(self, mut map: A) -> Result<(), A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                // serde_json's streaming `MapAccess` yields keys in document
                // order WITHOUT deduplication (dedup happens only when building a
                // `Value`/`Map`), so a repeated key surfaces here as a second
                // `next_key`. Recurse into each value to catch nested dups.
                let mut seen = std::collections::HashSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key.clone()) {
                        return Err(serde::de::Error::custom(format!("duplicate key {key:?}")));
                    }
                    map.next_value::<NoDupKeys>()?;
                }
                Ok(())
            }
        }

        deserializer.deserialize_any(AnyValue).map(|()| NoDupKeys)
    }
}

/// Strict-profile pre-validate `bytes`, then parse them into `T`. Runs the
/// [`NoDupKeys`] `deserialize_any` pre-pass (the six-axis strict profile) BEFORE
/// the type-specific `serde_json::from_slice::<T>`, on the SAME bytes, so a
/// non-conformant member is rejected uniformly no matter what `T` is — a strict
/// `deny_unknown_fields` struct, a tolerant peek that would lazily skip an
/// ignored field, or a struct carrying a `serde_json::Value` field (whose
/// keep-last map decode would otherwise silently accept a duplicate key). Both
/// the pre-pass and the typed parse yield a `serde_json::Error`, so existing
/// `.map_err(..Malformed..)` call sites need no change: pass the bytes through
/// this instead of `serde_json::from_slice` and the same error mapping applies.
pub(crate) fn from_slice_strict<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    // Axes 1-6 of the strict profile. A non-conformant member is rejected here,
    // before `T`'s own decode is ever attempted.
    let _: NoDupKeys = serde_json::from_slice(bytes)?;
    serde_json::from_slice(bytes)
}
