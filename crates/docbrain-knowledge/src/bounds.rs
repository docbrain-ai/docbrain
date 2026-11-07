// SPDX-License-Identifier: MIT
//! Bounds (spec §7.5, §4.3, §4.6). Named constants; an exceeded bound is an `unverifiable` check or an
//! `invalid` record naming it (`reason::Bound`), never an error. Each value moved by measurement is
//! recorded with its measurement in `SEMANTICS.md` §Bounds.

use crate::format::Format;

/// A record file, checked from the tree entry's size before reading (§7.5).
pub const RECORD_FILE_MAX: u64 = 65_536;
/// Front matter, in bytes (§7.5).
pub const FRONT_MATTER_MAX: usize = 8_192;
/// Front matter nesting depth (§7.5).
pub const FRONT_MATTER_DEPTH: usize = 32;
/// Checks per record, the declared-premise cap (§4.2, §7.5).
pub const CHECKS_MAX: usize = 100;
const MIB: u64 = 1_048_576;

/// An anchored file a `config_key` check reads, per format, checked from the tree entry's size before reading
/// and again on the bytes read (§7.5).
pub const fn anchored_file_max(f: Format) -> u64 {
    match f {
        Format::Json => 64 * MIB,
        Format::Yaml => 32 * MIB,
        Format::Toml | Format::Hcl => 16 * MIB,
    }
}
/// A file a `code` check reads, whatever its name, as text (§7.5).
pub const CODE_FILE_MAX: u64 = 64 * MIB;
/// The largest of the bounds above: what any anchor reads. A blob is kept to one byte more, to see it is over.
pub const ANCHORED_READ_MAX: u64 = {
    const fn max(a: u64, b: u64) -> u64 {
        if a > b { a } else { b }
    }
    let formats = max(max(anchored_file_max(Format::Yaml), anchored_file_max(Format::Json)), max(anchored_file_max(Format::Toml), anchored_file_max(Format::Hcl)));
    max(formats, CODE_FILE_MAX)
};
/// What one pass over a tree keeps of the anchored files it has read, counted as the memory each holds once
/// parsed, an estimate (§7.5, `SEMANTICS.md` §Bounds). The least recently used is let go first and read again
/// when asked for, and a file that holds more than this alone is never kept; this changes time, never an answer.
pub const FILES_HELD_MAX: u64 = 256 * MIB;
/// Resolver depth in anchored files (§7.5).
pub const RESOLVER_DEPTH: usize = 64;
/// What one anchored YAML file's anchors and aliases may copy: a node counts one and a scalar also its bytes, so
/// a long text aliased many times is charged for every byte it copies (§7.5, `SEMANTICS.md` §Bounds).
pub const ALIAS_EXPANSION_MAX: usize = 1_000_000;
/// Records read per pass; the excess, in byte order of path, is counted by folder (§7.5).
pub const RECORDS_PER_PASS: usize = 100_000;
/// A `code` text keeps at least this many Unicode scalar values after ASCII whitespace is removed (§4.3).
pub const CODE_TEXT_MIN_SCALARS: usize = 24;
/// A `code` text is at most this many bytes as parsed (§4.3).
pub const CODE_TEXT_MAX_BYTES: usize = 2_048;
/// A number in an anchored file, in bytes as written: converting a hex or octal number to decimal costs the
/// square of its length, so a longer one is `unverifiable: bound:number-text` (`SEMANTICS.md` §Bounds).
pub const NUMBER_TEXT_MAX: usize = 4_096;
/// `=` and `:` characters in one anchored HCL file, counted before parsing. The parser checks each object key
/// against every earlier one, so an object of n items costs the square of n; this count is at least the items of
/// every object, so one file costs at most its square (`SEMANTICS.md` §Bounds).
pub const HCL_ASSIGNMENTS_MAX: usize = 20_000;
/// Records shown by `context` before the overflow line (§4.6).
pub const CONTEXT_CAP: usize = 20;

#[cfg(test)]
mod tests {
    use super::*;

    /// A size as the bounds table writes it.
    fn size(b: u64) -> String {
        if b.is_multiple_of(MIB) {
            format!("{} MiB", b / MIB)
        } else if b.is_multiple_of(1024) {
            format!("{} KiB", b / 1024)
        } else {
            format!("{b} bytes")
        }
    }

    /// A count as the bounds table writes it: thousands separated by commas.
    fn count(n: usize) -> String {
        let s = n.to_string();
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
        }
        out
    }

    /// The published table is the bound: every constant reads as `SEMANTICS.md` states it.
    #[test]
    fn the_published_bounds_are_the_constants() {
        let semantics = include_str!("../SEMANTICS.md");
        let row = |name: &str| semantics.lines().find(|l| l.starts_with(&format!("| `{name}` |"))).unwrap_or_else(|| panic!("no row {name}"));
        let has = |name: &str, text: String| assert!(row(name).contains(&text), "row {name} does not say {text:?}");
        has("record-file", format!("| {},", size(RECORD_FILE_MAX)));
        has("front-matter", format!("| {} |", size(FRONT_MATTER_MAX as u64)));
        has("depth", format!("depth {FRONT_MATTER_DEPTH} |"));
        has("checks", format!("| {CHECKS_MAX} checks"));
        for (f, name) in [(Format::Json, "JSON"), (Format::Yaml, "YAML"), (Format::Toml, "TOML"), (Format::Hcl, "HCL")] {
            has("anchored-file", format!("{name} {},", size(anchored_file_max(f))));
        }
        has("anchored-file", format!("HCL files together {} ", size(anchored_file_max(Format::Hcl))));
        has("anchored-file", format!("whatever its name, {} |", size(CODE_FILE_MAX)));
        has("resolver-depth", format!("| {RESOLVER_DEPTH} |"));
        has("alias-expansion", format!("| {} for", count(ALIAS_EXPANSION_MAX)));
        has("records-per-pass", format!("| {};", count(RECORDS_PER_PASS)));
        has("code-text", format!("at least {CODE_TEXT_MIN_SCALARS} Unicode"));
        has("code-text", format!("at most {} bytes", count(CODE_TEXT_MAX_BYTES)));
        has("number-text", format!("| {} bytes", count(NUMBER_TEXT_MAX)));
        has("hcl-assignments", format!("| {} `=`", count(HCL_ASSIGNMENTS_MAX)));
        let held = format!("up to {}, counted as the memory", size(FILES_HELD_MAX));
        assert!(semantics.contains(&held), "§Bounds does not say {held:?}");
    }
}
