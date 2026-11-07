// SPDX-License-Identifier: MIT
//! The reason vocabulary: why a check is `unverifiable` or `broken`/`stale`, and why a record is `invalid`.
//!
//! `SEMANTICS.md` §States, §Bounds. Every code here is stable published text
//! (`SEMANTICS.md` §Reasons): kebab-case, never reworded. A code is unique within its state; a reader
//! always reports the state beside the code.

use std::borrow::Cow;

/// A named bound (`SEMANTICS.md` §Bounds). An exceeded bound is an `unverifiable` check or an `invalid` record naming it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Bound {
    RecordFile,
    FrontMatter,
    Depth,
    Checks,
    AnchoredFile,
    ResolverDepth,
    AliasExpansion,
    RecordsPerPass,
    CodeText,
    NumberText,
    HclAssignments,
}

impl Bound {
    /// Every bound, in declaration order.
    pub const ALL: [Bound; 11] = [
        Bound::RecordFile,
        Bound::FrontMatter,
        Bound::Depth,
        Bound::Checks,
        Bound::AnchoredFile,
        Bound::ResolverDepth,
        Bound::AliasExpansion,
        Bound::RecordsPerPass,
        Bound::CodeText,
        Bound::NumberText,
        Bound::HclAssignments,
    ];

    /// The bound's published name, used after `bound:` in a reason code.
    pub fn name(self) -> &'static str {
        match self {
            Bound::RecordFile => "record-file",
            Bound::FrontMatter => "front-matter",
            Bound::Depth => "depth",
            Bound::Checks => "checks",
            Bound::AnchoredFile => "anchored-file",
            Bound::ResolverDepth => "resolver-depth",
            Bound::AliasExpansion => "alias-expansion",
            Bound::RecordsPerPass => "records-per-pass",
            Bound::CodeText => "code-text",
            Bound::NumberText => "number-text",
            Bound::HclAssignments => "hcl-assignments",
        }
    }

    fn code(self) -> Cow<'static, str> {
        Cow::Owned(format!("bound:{}", self.name()))
    }
}

/// Why a check is `unverifiable` (`SEMANTICS.md` §Reasons): the value cannot be judged. Never reported as `holds`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Unverifiable {
    Format,
    Template,
    Expression,
    MergeKey,
    LfsPointer,
    Submodule,
    SymbolicLink,
    Encoding,
    NotAScalar,
    NotComparable,
    NumberOrVersion,
    Range,
    Ambiguous,
    DefinedTwice,
    Unknown,
    NewerSchema,
    Yaml11,
    Tag,
    Bound(Bound),
}

impl Unverifiable {
    /// The published reason code.
    pub fn code(&self) -> Cow<'static, str> {
        let c = match self {
            Unverifiable::Format => "format",
            Unverifiable::Template => "template",
            Unverifiable::Expression => "expression",
            Unverifiable::MergeKey => "merge-key",
            Unverifiable::LfsPointer => "lfs-pointer",
            Unverifiable::Submodule => "submodule",
            Unverifiable::SymbolicLink => "symbolic-link",
            Unverifiable::Encoding => "encoding",
            Unverifiable::NotAScalar => "not-a-scalar",
            Unverifiable::NotComparable => "not-comparable",
            Unverifiable::NumberOrVersion => "number-or-version",
            Unverifiable::Range => "range",
            Unverifiable::Ambiguous => "ambiguous",
            Unverifiable::DefinedTwice => "defined-twice",
            Unverifiable::Unknown => "unknown",
            Unverifiable::NewerSchema => "newer-schema",
            Unverifiable::Yaml11 => "yaml-1.1",
            Unverifiable::Tag => "tag",
            Unverifiable::Bound(b) => return b.code(),
        };
        Cow::Borrowed(c)
    }
}

/// A display of a resolved value, cut at 200 bytes on a character boundary.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Shown(String);

impl Shown {
    /// The longest display kept, in bytes.
    pub const MAX_BYTES: usize = 200;

    pub fn new(text: &str) -> Shown {
        let mut end = text.len().min(Self::MAX_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        Shown(text[..end].to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why a check does not hold although its value was judged (`SEMANTICS.md` §Reasons): `broken` under `must`, `stale` under
/// `assumes`. A missing file or key is never `unverifiable`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Fail {
    MissingFile,
    MissingKey,
    False { actual: Shown },
}

impl Fail {
    /// The published reason code.
    pub fn code(&self) -> Cow<'static, str> {
        Cow::Borrowed(match self {
            Fail::MissingFile => "missing-file",
            Fail::MissingKey => "missing-key",
            Fail::False { .. } => "false",
        })
    }
}

/// Why a record is `invalid` (`SEMANTICS.md` §Reasons). `MissingKey` and `Type` name the
/// front-matter key; their codes are `missing-key:<key>` and `type:<key>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Invalid {
    Parse,
    Quotes,
    SchemaInteger,
    Id,
    ExampleId,
    Kind,
    Title,
    EmptyBody,
    HiddenText,
    ConflictMarkers,
    Confinement,
    SymlinkRecord,
    Verbs,
    Anchor,
    KeyPath,
    GlobDir,
    DuplicateKey,
    YamlFeature,
    MissingKey(&'static str),
    Type(&'static str),
    Bound(Bound),
}

impl Invalid {
    /// The published reason code.
    pub fn code(&self) -> Cow<'static, str> {
        let c = match self {
            Invalid::Parse => "parse",
            Invalid::Quotes => "quotes",
            Invalid::SchemaInteger => "schema-integer",
            Invalid::Id => "id",
            Invalid::ExampleId => "example-id",
            Invalid::Kind => "kind",
            Invalid::Title => "title",
            Invalid::EmptyBody => "empty-body",
            Invalid::HiddenText => "hidden-text",
            Invalid::ConflictMarkers => "conflict-markers",
            Invalid::Confinement => "confinement",
            Invalid::SymlinkRecord => "symlink-record",
            Invalid::Verbs => "verbs",
            Invalid::Anchor => "anchor",
            Invalid::KeyPath => "key-path",
            Invalid::GlobDir => "glob-dir",
            Invalid::DuplicateKey => "duplicate-key",
            Invalid::YamlFeature => "yaml-feature",
            Invalid::MissingKey(k) => return Cow::Owned(format!("missing-key:{k}")),
            Invalid::Type(k) => return Cow::Owned(format!("type:{k}")),
            Invalid::Bound(b) => return b.code(),
        };
        Cow::Borrowed(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Every `Unverifiable` value. The match below fails to compile when a variant is added, so the
    /// list cannot silently fall behind the enum.
    fn every_unverifiable() -> Vec<Unverifiable> {
        let simple = [
            Unverifiable::Format,
            Unverifiable::Template,
            Unverifiable::Expression,
            Unverifiable::MergeKey,
            Unverifiable::LfsPointer,
            Unverifiable::Submodule,
            Unverifiable::SymbolicLink,
            Unverifiable::Encoding,
            Unverifiable::NotAScalar,
            Unverifiable::NotComparable,
            Unverifiable::NumberOrVersion,
            Unverifiable::Range,
            Unverifiable::Ambiguous,
            Unverifiable::DefinedTwice,
            Unverifiable::Unknown,
            Unverifiable::NewerSchema,
            Unverifiable::Yaml11,
            Unverifiable::Tag,
        ];
        for u in &simple {
            match u {
                Unverifiable::Format
                | Unverifiable::Template
                | Unverifiable::Expression
                | Unverifiable::MergeKey
                | Unverifiable::LfsPointer
                | Unverifiable::Submodule
                | Unverifiable::SymbolicLink
                | Unverifiable::Encoding
                | Unverifiable::NotAScalar
                | Unverifiable::NotComparable
                | Unverifiable::NumberOrVersion
                | Unverifiable::Range
                | Unverifiable::Ambiguous
                | Unverifiable::DefinedTwice
                | Unverifiable::Unknown
                | Unverifiable::NewerSchema
                | Unverifiable::Yaml11
                | Unverifiable::Tag
                | Unverifiable::Bound(_) => {}
            }
        }
        let mut all = simple.to_vec();
        all.extend(Bound::ALL.iter().map(|b| Unverifiable::Bound(*b)));
        all
    }

    fn every_invalid() -> Vec<Invalid> {
        let simple = [
            Invalid::Parse,
            Invalid::Quotes,
            Invalid::SchemaInteger,
            Invalid::Id,
            Invalid::ExampleId,
            Invalid::Kind,
            Invalid::Title,
            Invalid::EmptyBody,
            Invalid::HiddenText,
            Invalid::ConflictMarkers,
            Invalid::Confinement,
            Invalid::SymlinkRecord,
            Invalid::Verbs,
            Invalid::Anchor,
            Invalid::KeyPath,
            Invalid::GlobDir,
            Invalid::DuplicateKey,
            Invalid::YamlFeature,
            Invalid::MissingKey("text"),
            Invalid::Type("applies_to"),
        ];
        for i in &simple {
            match i {
                Invalid::Parse
                | Invalid::Quotes
                | Invalid::SchemaInteger
                | Invalid::Id
                | Invalid::ExampleId
                | Invalid::Kind
                | Invalid::Title
                | Invalid::EmptyBody
                | Invalid::HiddenText
                | Invalid::ConflictMarkers
                | Invalid::Confinement
                | Invalid::SymlinkRecord
                | Invalid::Verbs
                | Invalid::Anchor
                | Invalid::KeyPath
                | Invalid::GlobDir
                | Invalid::DuplicateKey
                | Invalid::YamlFeature
                | Invalid::MissingKey(_)
                | Invalid::Type(_)
                | Invalid::Bound(_) => {}
            }
        }
        let mut all = simple.to_vec();
        all.extend(Bound::ALL.iter().map(|b| Invalid::Bound(*b)));
        all
    }

    fn every_fail() -> Vec<Fail> {
        vec![Fail::MissingFile, Fail::MissingKey, Fail::False { actual: Shown::new("1") }]
    }

    /// Kebab before an optional `:`; after it, a front-matter key or bound name verbatim (ledger, pre-flight).
    fn well_formed(code: &str) -> bool {
        let (head, tail) = match code.split_once(':') {
            Some((h, t)) => (h, Some(t)),
            None => (code, None),
        };
        let kebab = |s: &str, extra: &[char]| {
            !s.is_empty()
                && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.' || extra.contains(&c))
                && !s.starts_with('-')
                && !s.ends_with('-')
        };
        kebab(head, &[]) && tail.is_none_or(|t| kebab(t, &['_']))
    }

    #[test]
    fn reason_codes_are_the_published_vocabulary() {
        assert_eq!(Unverifiable::NumberOrVersion.code(), "number-or-version");
        assert_eq!(Unverifiable::Yaml11.code(), "yaml-1.1");
        assert_eq!(Unverifiable::Bound(Bound::AnchoredFile).code(), "bound:anchored-file");
        assert_eq!(Fail::MissingKey.code(), "missing-key");
        assert_eq!(Invalid::Quotes.code(), "quotes");
        assert_eq!(Invalid::MissingKey("text").code(), "missing-key:text");
        assert_eq!(Invalid::Type("applies_to").code(), "type:applies_to");
        assert_eq!(Invalid::Bound(Bound::FrontMatter).code(), "bound:front-matter");
    }

    #[test]
    fn every_code_is_unique_and_kebab() {
        let groups: [(&str, Vec<String>); 3] = [
            ("unverifiable", every_unverifiable().iter().map(|u| u.code().into_owned()).collect()),
            ("invalid", every_invalid().iter().map(|u| u.code().into_owned()).collect()),
            ("fail", every_fail().iter().map(|u| u.code().into_owned()).collect()),
        ];
        for (state, codes) in &groups {
            let mut seen = HashSet::new();
            for c in codes {
                assert!(well_formed(c), "{state}: {c} is not a published code shape");
                assert!(seen.insert(c.clone()), "{state}: {c} twice");
            }
        }
        assert_eq!(groups[0].1.len(), 18 + Bound::ALL.len());
    }

    #[test]
    fn every_code_is_in_the_published_semantics() {
        let semantics = include_str!("../SEMANTICS.md");
        let mut codes: Vec<String> = Vec::new();
        codes.extend(every_unverifiable().iter().map(|u| u.code().into_owned()));
        codes.extend(every_invalid().iter().map(|u| u.code().into_owned()));
        codes.extend(every_fail().iter().map(|u| u.code().into_owned()));
        for c in codes {
            // A code with a key part is published as its pattern: `missing-key:<key>`, `type:<key>`.
            // A bound code is published as `bound:<name>` plus the name's row in the bounds table.
            let published = match c.split_once(':') {
                Some(("bound", name)) => {
                    assert!(semantics.contains(&format!("| `{name}` |")), "SEMANTICS.md has no bound row {name}");
                    "`bound:<name>`".to_owned()
                }
                Some((head, _)) => format!("`{head}:<key>`"),
                None => format!("`{c}`"),
            };
            assert!(semantics.contains(&published), "SEMANTICS.md does not list {published}");
        }
    }

    #[test]
    fn shown_cuts_on_a_char_boundary() {
        assert_eq!(Shown::new("abc").as_str(), "abc");
        let long = "é".repeat(150); // 300 bytes
        let s = Shown::new(&long);
        assert_eq!(s.as_str().len(), 200);
        let odd = format!("a{}", "é".repeat(150)); // a boundary falls inside a char at 200
        assert_eq!(Shown::new(&odd).as_str().len(), 199);
    }
}
