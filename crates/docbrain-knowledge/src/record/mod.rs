// SPDX-License-Identifier: MIT
//! Records: one Markdown file with YAML front matter (`SEMANTICS.md` §The record, §States).

mod id;
mod keypath;
mod parse;
mod render;

use std::fmt;

pub use id::Id;
pub use keypath::{KeyPath, Seg, parse_key_path};
pub use parse::parse_record;
pub use render::{Draft, DraftAnchor, DraftCheck, render};

use crate::path::RepoGlob;
use crate::reason::Invalid;

/// The four kinds (`SEMANTICS.md` §The record).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Decision,
    Fact,
    Procedure,
    Question,
}

impl Kind {
    /// The kind written exactly, in lower case.
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "decision" => Some(Kind::Decision),
            "fact" => Some(Kind::Fact),
            "procedure" => Some(Kind::Procedure),
            "question" => Some(Kind::Question),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Decision => "decision",
            Kind::Fact => "fact",
            Kind::Procedure => "procedure",
            Kind::Question => "question",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A check's verb (`SEMANTICS.md` §States): `must` judges changes; `assumes` makes the record stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verb {
    Must,
    Assumes,
}

impl Verb {
    pub fn as_str(self) -> &'static str {
        match self {
            Verb::Must => "must",
            Verb::Assumes => "assumes",
        }
    }
}

impl fmt::Display for Verb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A comparison op (`SEMANTICS.md` §States).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    Eq,
    Ne,
    Ge,
    Gt,
    Le,
    Lt,
}

impl Op {
    /// The two-character ops first, so `>=` is never read as `>` then `=`.
    const BY_LENGTH: [Op; 6] = [Op::Eq, Op::Ne, Op::Ge, Op::Le, Op::Gt, Op::Lt];

    pub fn as_str(self) -> &'static str {
        match self {
            Op::Eq => "==",
            Op::Ne => "!=",
            Op::Ge => ">=",
            Op::Gt => ">",
            Op::Le => "<=",
            Op::Lt => "<",
        }
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A check's expectation (`SEMANTICS.md` §States): `exist`, or an op and a literal. Anything else is kept as written and is
/// judged `unverifiable: unknown` (`SEMANTICS.md` §The record).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Expectation {
    Exist,
    Compare { op: Op, literal: String },
    Unknown(String),
}

impl Expectation {
    /// `exist` exactly; else an op at the start, not followed by another of `=`, `!`, `<`, `>`, with the
    /// rest, spaces and tabs around it removed, as the literal; else unknown.
    pub fn parse(s: &str) -> Expectation {
        if s == "exist" {
            return Expectation::Exist;
        }
        for op in Op::BY_LENGTH {
            if let Some(rest) = s.strip_prefix(op.as_str()) {
                if rest.starts_with(['=', '!', '<', '>']) {
                    break;
                }
                return Expectation::Compare { op, literal: rest.trim_matches([' ', '\t']).to_owned() };
            }
        }
        Expectation::Unknown(s.to_owned())
    }
}

impl fmt::Display for Expectation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expectation::Exist => f.write_str("exist"),
            Expectation::Compare { op, literal } => write!(f, "{op} {literal}"),
            Expectation::Unknown(s) => f.write_str(s),
        }
    }
}

/// One `config_key` layer: a file (or a glob within one directory) and a key path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    pub file: RepoGlob,
    pub key: KeyPath,
}

/// What a check is anchored to (`SEMANTICS.md` §States). `Unknown` keeps an unknown check kind's first key (`SEMANTICS.md` §The record).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    Path(RepoGlob),
    ConfigKey(Vec<Layer>),
    Code { file: RepoGlob, text: String },
    Unknown(String),
}

/// One check, with the line its entry starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub anchor: Anchor,
    pub verb: Verb,
    pub expect: Expectation,
    pub line: u32,
}

/// A valid schema-1 record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub id: Id,
    pub kind: Kind,
    pub title: String,
    /// As written when `applies_written`; else the default, or the whole folder when the checks
    /// name no file.
    pub applies_to: Vec<RepoGlob>,
    pub applies_written: bool,
    pub checks: Vec<Check>,
    pub supersedes: Option<Id>,
    /// Everything after the closing `---` line, LF line ends.
    pub body: String,
    /// Unknown keys not starting `x-`, in file order; a check's as `checks[<n>].<key>`.
    pub unknown_keys: Vec<String>,
}

impl Record {
    /// The body from its first non-blank line to the line before the next line of only ASCII whitespace,
    /// without the final line end.
    pub fn first_paragraph(&self) -> &str {
        let blank = |l: &str| l.chars().all(|c| matches!(c, '\t' | '\x0C' | '\r' | ' '));
        let (mut start, mut end, mut off) = (None, self.body.len(), 0);
        for line in self.body.split_inclusive('\n') {
            let content = line.strip_suffix('\n').unwrap_or(line);
            match start {
                None if !blank(content) => start = Some(off),
                Some(_) if blank(content) => {
                    end = off;
                    break;
                }
                _ => {}
            }
            off += line.len();
        }
        start.map_or("", |s| self.body[s..end].trim_end_matches('\n'))
    }
}

/// The reading of one record file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Valid(Record),
    /// `knowledge` above 1: the record is shown, never judged or rewritten (`SEMANTICS.md` §The record). Each field is filled
    /// when it reads as schema 1 reads it; `applies_to` only as written, all or nothing (it routes by it).
    Newer {
        schema: u64,
        id: Option<Id>,
        kind: Option<Kind>,
        title: Option<String>,
        supersedes: Option<Id>,
        applies_to: Vec<RepoGlob>,
    },
    /// Line and column from 1; the column counts Unicode scalar values.
    Invalid { reason: Invalid, line: u32, col: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_verbs_show_as_written() {
        for (k, s) in [(Kind::Decision, "decision"), (Kind::Fact, "fact"), (Kind::Procedure, "procedure"), (Kind::Question, "question")] {
            assert_eq!((k.as_str(), k.to_string()), (s, s.to_owned()));
        }
        for (v, s) in [(Verb::Must, "must"), (Verb::Assumes, "assumes")] {
            assert_eq!((v.as_str(), v.to_string()), (s, s.to_owned()));
        }
    }
}
