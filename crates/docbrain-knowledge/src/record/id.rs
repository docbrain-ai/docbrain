// SPDX-License-Identifier: MIT
//! Record identity (spec §4.4): 12 characters of lower-case Crockford base32, 60 random bits.

use std::fmt;

const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// A record id. Ids beginning `0000` are reserved for documentation.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id([u8; 12]);

impl Id {
    /// Exactly 12 characters of `0123456789abcdefghjkmnpqrstvwxyz`.
    pub fn parse(s: &str) -> Option<Id> {
        let b: [u8; 12] = s.as_bytes().try_into().ok()?;
        b.iter().all(|c| ALPHABET.contains(c)).then_some(Id(b))
    }

    /// Reserved for documentation: every reader treats such a record as invalid.
    pub fn is_example(&self) -> bool {
        self.0.starts_with(b"0000")
    }

    /// 60 bits from the operating system's secure random source, never a seeded generator; an id that
    /// would begin `0000` is drawn again.
    pub fn mint() -> Result<Id, getrandom::Error> {
        loop {
            let mut buf = [0u8; 8];
            getrandom::fill(&mut buf)?;
            let bits = u64::from_le_bytes(buf) & ((1 << 60) - 1);
            let mut out = [0u8; 12];
            for (k, slot) in out.iter_mut().enumerate() {
                let digit = (bits >> (5 * (11 - k))) & 31;
                *slot = ALPHABET[digit as usize];
            }
            let id = Id(out);
            if !id.is_example() {
                return Ok(id);
            }
        }
    }

    pub fn as_str(&self) -> &str {
        // Always ASCII: every byte is from ALPHABET.
        std::str::from_utf8(&self.0).unwrap_or_default()
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An id serialises as its 12-character text.
impl serde::Serialize for Id {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_never_example_and_parses() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..10_000 {
            let id = Id::mint().unwrap();
            assert!(!id.is_example());
            assert_eq!(Id::parse(&id.to_string()), Some(id));
            seen.insert(id);
        }
        // Sixty random bits each: ten thousand ids never collide.
        assert_eq!(seen.len(), 10_000);
    }

    #[test]
    fn debug_shows_the_text() {
        assert_eq!(format!("{:?}", Id::parse("k3v9q2x7m1ab").unwrap()), "Id(k3v9q2x7m1ab)");
    }

    #[test]
    fn parse_is_lower_crockford_twelve() {
        assert!(Id::parse("k3v9q2x7m1ab").is_some());
        assert!(Id::parse("0000000000a1").is_some_and(|i| i.is_example()));
        for bad in ["K3v9q2x7m1ab", "k3v9q2x7m1a", "k3v9q2x7m1abc", "i3v9q2x7m1ab", "l3v9q2x7m1ab", "o3v9q2x7m1ab", "u3v9q2x7m1ab", "", "k3v9q2x7m1a "] {
            assert_eq!(Id::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Id::parse("0755abcd1234").map(|i| i.to_string()), Some("0755abcd1234".to_owned()));
    }
}
