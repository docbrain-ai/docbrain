// SPDX-License-Identifier: MIT
//! The `.knowledge/` commands: `record`, `context` and `status`. Offline: no config, no sign-in, no server, no
//! socket. What a record means is the `docbrain-knowledge` crate's (`SEMANTICS.md`); this module owns the path
//! arguments, the trees judged, the output and the exit codes.

mod args;
mod context;
mod exit;
mod paths;
mod readme;
mod record;
mod frame;
mod gate;
mod sarif;
mod show;
mod trees;
mod status;
mod worktree;

#[cfg(test)]
#[path = "../../tests/common/knowledge_repo.rs"]
mod testrepo;

pub(crate) use args::{ContextArgs, RecordArgs, StatusArgs};
pub(crate) use exit::usage_exit;
pub(crate) use paths::Session;
pub(crate) use context::run_context;
pub(crate) use record::run_record;
pub(crate) use status::run_status;
