// SPDX-License-Identifier: MIT
//! Writing a record (`SEMANTICS.md` §The record; edge case 33): LF line ends, no BOM, every string single-quoted with `'` written
//! `''`, `applies_to` always written, lists in block style. `render` writes what it is given; the caller
//! checks a draft first (a title with a line break, hidden text or an empty body do not parse back).

use super::{Id, Kind, Verb};

/// A check as the writer gives it: paths and key paths as written, the expectation as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftAnchor {
    Path(String),
    /// `<file>#<key path>` per layer.
    ConfigKey(Vec<String>),
    Code { file: String, text: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftCheck {
    pub anchor: DraftAnchor,
    pub verb: Verb,
    pub expect: String,
}

/// A record to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub kind: Kind,
    pub title: String,
    pub applies_to: Vec<String>,
    pub checks: Vec<DraftCheck>,
    pub supersedes: Option<Id>,
    pub body: String,
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The record file for `d` under `id`, ending with one LF.
pub fn render(id: &Id, d: &Draft) -> String {
    let mut out = String::from("---\nknowledge: 1\n");
    out += &format!("id: {}\nkind: {}\ntitle: {}\n", quote(id.as_str()), quote(d.kind.as_str()), quote(&d.title));
    if d.applies_to.is_empty() {
        out += "applies_to: []\n";
    } else {
        out += "applies_to:\n";
        for g in &d.applies_to {
            out += &format!("  - {}\n", quote(g));
        }
    }
    if !d.checks.is_empty() {
        out += "checks:\n";
        for c in &d.checks {
            match &c.anchor {
                DraftAnchor::Path(p) => out += &format!("  - path: {}\n", quote(p)),
                DraftAnchor::ConfigKey(layers) => match layers.as_slice() {
                    [one] => out += &format!("  - config_key: {}\n", quote(one)),
                    _ => {
                        out += "  - config_key:\n";
                        for l in layers {
                            out += &format!("      - {}\n", quote(l));
                        }
                    }
                },
                DraftAnchor::Code { file, text } => out += &format!("  - code: {}\n    text: {}\n", quote(file), quote(text)),
            }
            out += &format!("    {}: {}\n", c.verb.as_str(), quote(&c.expect));
        }
    }
    if let Some(s) = &d.supersedes {
        out += &format!("supersedes: {}\n", quote(s.as_str()));
    }
    out += "---\n";
    out += &d.body;
    if !d.body.ends_with('\n') {
        out.push('\n');
    }
    out
}
