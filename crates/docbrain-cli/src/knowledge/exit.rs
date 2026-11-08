// SPDX-License-Identifier: MIT
//! One exit contract for the three commands: 0 nothing failed, 2 the command's own failure (a gate that fails,
//! a record refused), 3 it could not run (a usage error, no repository, an object not in the local history).

use std::ffi::OsString;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exit {
    Ok,
    Fail,
    Usage,
}

impl Exit {
    pub(crate) fn code(self) -> i32 {
        match self {
            Exit::Ok => 0,
            Exit::Fail => 2,
            Exit::Usage => 3,
        }
    }
}

const COMMANDS: [&str; 3] = ["record", "context", "status"];

/// The global flags that take a value as the next argument.
const GLOBAL_WITH_VALUE: [&str; 3] = ["--api-key", "--profile", "--docbrain-env"];

/// The exit code of a command-line error when the command is one of these three: 3, because clap's own 2 is what a
/// failing gate exits with. `None` for every other command, which keeps clap's. The command is the first argument
/// that is neither a global flag nor a global flag's value.
pub(crate) fn usage_exit(argv: &[OsString]) -> Option<i32> {
    let mut args = argv.iter().skip(1).map(|a| a.to_string_lossy());
    while let Some(a) = args.next() {
        if GLOBAL_WITH_VALUE.contains(&a.as_ref()) {
            args.next();
        } else if !GLOBAL_WITH_VALUE.iter().any(|g| a.strip_prefix(g).is_some_and(|v| v.starts_with('='))) {
            return COMMANDS.contains(&a.as_ref()).then_some(Exit::Usage.code());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<OsString> {
        std::iter::once("docbrain").chain(args.iter().copied()).map(Into::into).collect()
    }

    #[test]
    fn the_command_is_found_past_global_flags() {
        for args in [&["status", "--bogus"][..], &["--profile", "p", "record"], &["--api-key=k", "--docbrain-env", "/e", "context", "x"]] {
            assert_eq!(usage_exit(&argv(args)), Some(3), "{args:?}");
        }
        for args in [&["ask", "--bogus"][..], &["--profile", "status", "ask"], &["--bogus", "status"], &[], &["--api-key"], &["--profile=status", "ask"]] {
            assert_eq!(usage_exit(&argv(args)), None, "{args:?}");
        }
    }
}
