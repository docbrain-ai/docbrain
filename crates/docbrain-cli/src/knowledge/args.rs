// SPDX-License-Identifier: MIT
//! The command lines of `record`, `context` and `status`.

use clap::{ArgAction, Args, ValueEnum};

/// What a record is (`SEMANTICS.md` §The record).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum KindArg {
    Decision,
    Fact,
    Procedure,
    Question,
}

/// `--must-code` and `--assumes-code` hold their values flat: clap takes exactly two per flag, so they read in
/// (file, text) pairs.
#[derive(Debug, Args)]
pub(crate) struct RecordArgs {
    /// What the record is
    #[arg(value_enum)]
    pub kind: KindArg,
    /// One line that says what is true
    pub title: String,
    /// A file glob the record is context for, relative to the current directory; repeatable. Without it, the
    /// record applies to the files its checks name
    #[arg(long = "applies-to", value_name = "GLOB")]
    pub applies_to: Vec<String>,
    /// A value in a YAML, JSON, TOML or HCL file that must hold: '<file>#<key path> <op> <value>'; a change that
    /// breaks it fails the gate. Repeatable
    #[arg(long, value_name = "EXPR")]
    pub must: Vec<String>,
    /// As --must, but a change that breaks it marks the record stale instead
    #[arg(long, value_name = "EXPR")]
    pub assumes: Vec<String>,
    /// A file or directory that must exist. Repeatable
    #[arg(long = "must-exist", value_name = "PATH")]
    pub must_exist: Vec<String>,
    /// As --must-exist, but a change that removes it marks the record stale instead
    #[arg(long = "assumes-exist", value_name = "PATH")]
    pub assumes_exist: Vec<String>,
    /// A text that must be found once in a file. Repeatable
    #[arg(long = "must-code", num_args = 2, value_names = ["FILE", "TEXT"], action = ArgAction::Append)]
    pub must_code: Vec<String>,
    /// As --must-code, but a change that breaks it marks the record stale instead
    #[arg(long = "assumes-code", num_args = 2, value_names = ["FILE", "TEXT"], action = ArgAction::Append)]
    pub assumes_code: Vec<String>,
    /// The body: its first paragraph says why. '-' reads it from standard input; without it, your editor opens
    #[arg(long, value_name = "TEXT")]
    pub body: Option<String>,
    /// The id of the record this one replaces
    #[arg(long, value_name = "ID")]
    pub supersedes: Option<String>,
    /// The directory whose .knowledge/ folder holds the record; by default the nearest above every anchor
    #[arg(long, value_name = "DIR")]
    pub folder: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ContextArgs {
    /// The files, relative to the current directory
    #[arg(required = true, value_name = "PATH")]
    pub paths: Vec<String>,
    /// Every record that applies, not only the first 20
    #[arg(long)]
    pub all: bool,
    /// One line per record
    #[arg(long, conflicts_with = "json")]
    pub brief: bool,
    /// JSON on standard output
    #[arg(long)]
    pub json: bool,
    /// The branch a record must be on to count as reviewed (default: origin's default branch)
    #[arg(long, value_name = "BASE")]
    pub base: Option<String>,
}

/// What `--fail-on` fails on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum FailOn {
    /// A `must` that does not hold
    Broken,
}

#[derive(Debug, Args)]
pub(crate) struct StatusArgs {
    /// A commit to read instead of the working tree
    #[arg(value_name = "REV")]
    pub rev: Option<String>,
    /// Judge the change from the merge base with BASE to the working tree (or REV)
    #[arg(long, value_name = "BASE")]
    pub base: Option<String>,
    /// JSON on standard output
    #[arg(long, conflicts_with = "sarif")]
    pub json: bool,
    /// SARIF 2.1.0 on standard output
    #[arg(long, requires = "base")]
    pub sarif: bool,
    /// Exit 2 when a finding is of this kind
    #[arg(long = "fail-on", value_enum, value_name = "KIND")]
    pub fail_on: Option<FailOn>,
    /// Only findings on these paths fail (after --)
    #[arg(last = true, value_name = "PATHSPEC")]
    pub pathspec: Vec<String>,
}
