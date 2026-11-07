// SPDX-License-Identifier: MIT
//! `docbrain-knowledge`: the open evaluator of `.knowledge/` records.
//!
//! It parses records, computes every check's and record's state from a tree, routes records to paths,
//! judges a change with the two rules, and screens for well-known secret shapes. The meaning is
//! published in `SEMANTICS.md` beside this crate; code that disagrees with it is the bug. Every later
//! surface (the CLI, the hook, MCP, the server's sync) calls this crate, and nothing else, for meaning.
//!
//! The evaluator is synchronous and pure over a [`Tree`]. Async callers wrap calls in
//! `tokio::task::spawn_blocking` (or their runtime's equivalent).

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

// The corpus runner's tree builder, for unit tests that run the corpus under internal settings.
#[cfg(test)]
extern crate self as docbrain_knowledge;
#[cfg(test)]
#[allow(dead_code)]
#[path = "../tests/common/mod.rs"]
mod test_common;

pub mod bounds;
pub mod charset;
pub mod check;
pub mod folder;
pub mod format;
pub mod gate;
pub mod git;
pub mod path;
pub mod reason;
pub mod record;
pub mod resolve;
pub mod route;
pub mod scalar;
pub mod secret;
pub mod tree;
pub mod value;
pub mod yaml;

pub use reason::{Bound, Fail, Invalid, Shown, Unverifiable};
pub use tree::{ChangedPaths, Entry, MemTree, Mode, Oid, Tree, TreeError};
