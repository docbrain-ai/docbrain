// SPDX-License-Identifier: MIT
//! Restricted STORE-only ZIP container (`.dbev` normative profile).
//!
//! `.dbev` is ZIP-compatible but conforms to a profile the reader enforces
//! byte-first, before anything in the container is interpreted (design doc
//! "Container: normative restricted profile", finding 3): STORE (method 0)
//! only, no zip64, no encryption, no data descriptors, UTF-8 names, no
//! interpreted extra fields, an empty archive comment, and a fixed member
//! path whitelist. This module is the NORMATIVE spec: Task 14's Python
//! verifier must reproduce every check here byte-for-byte, so precision
//! matters more than cleverness — the notes below record the handful of
//! judgment calls made where the brief's prose left a choice open.
//!
//! ## Design notes (read before touching this file)
//!
//! - **No comment ⇒ EOCD is provably the last 22 bytes.** The profile bans
//!   every archive comment except the empty one. Once the located EOCD's
//!   `comment_len` is required to be 0, "no bytes may follow the EOCD" and
//!   "the archive comment is empty" collapse into ONE check
//!   (`eocd_offset + 22 == file_len`), which is exactly what rejects
//!   garbage appended after a genuine EOCD (test-plan 4.12) without any
//!   special-casing.
//! - **EOCD back-scan picks the RIGHTMOST signature match and never falls
//!   back to an earlier one.** Falling back to an earlier match on
//!   rejection is the exact ambiguity that lets two "compliant" ZIP
//!   parsers disagree on which EOCD is real (test-plan 4.12's point), so a
//!   failed validation of the rightmost match is a hard reject, not a
//!   retry against an earlier candidate.
//! - **General-purpose flag is checked by exact equality to `0x0800`
//!   (UTF-8 names, nothing else), not per-bit.** The brief pins three
//!   specific bits (no encryption / no data descriptor / UTF-8 names) and
//!   is silent on every other bit — but `ContainerWriter::finish` only
//!   ever emits `0x0800`, so an honest bundle can NEVER carry any other
//!   value, and rejecting anything else costs zero false-rejections.
//!   Exact equality is the tighter, safer reading: it collapses 12+
//!   unspecified-bit axes a faithful-but-independent Python reader (Task
//!   14) could otherwise interpret differently, into one pinned constant
//!   both implementations copy verbatim.
//! - **Local-header cross-check covers name, method, and both sizes** (the
//!   brief's literal enumeration) **plus `extra_len`**, because
//!   `extra_len` is not just "checked" — it is unavoidably CONSUMED by any
//!   reader to locate where member data starts, so a CD/local mismatch on
//!   it is a real smuggling vector, not an optional extra. It is checked
//!   in exactly the same byte-agreement style as the rest, so this does
//!   not add a new axis a faithful Python implementation could reasonably
//!   omit — any reader that correctly parses a local header already has
//!   this value in hand.
//! - **Per-member comments and per-member `disk_start` are also required
//!   empty/zero.** Purely restrictive (our own writer never emits either),
//!   so this cannot reject anything genuinely needed and removes
//!   uninterpreted surface, consistent with "no extra fields interpreted."
//! - **Local-header validation is eager, not lazy.** Every member's local
//!   header is cross-checked against the CD during `open()`, not only when
//!   `member_bytes` is later called for that name. `open()` succeeding is
//!   therefore the single point of truth "this container is profile-valid"
//!   — a strengthening (fail-fast for every entry) that cannot itself
//!   cause a Rust/Python divergence, since a full bundle verify always
//!   reads every member anyway.
//! - **STORE requires `compressed_size == uncompressed_size`.** Not an
//!   invented rule: it is what STORE (no compression) *means*, so a CD
//!   entry claiming method 0 with mismatched sizes is self-contradictory
//!   and rejected as malformed rather than accepted with an ambiguous
//!   "true" size.

use std::collections::{HashMap, HashSet};

/// Local file header signature, `PK\x03\x04` (APPNOTE §4.3.7). `pub(crate)`
/// (widened from private for Task 7's `BundleBuilder`, which needs to hand-
/// craft adversarial raw ZIP bytes — e.g. a duplicate member name — that
/// `ContainerWriter` refuses to produce by construction; reusing these
/// constants instead of re-declaring magic numbers in `builder.rs` keeps
/// the two in lockstep if the profile's field values ever change.
pub(crate) const LFH_SIG: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];
/// Central directory file header signature, `PK\x01\x02` (APPNOTE §4.3.12).
pub(crate) const CDFH_SIG: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
/// End of central directory record signature, `PK\x05\x06` (APPNOTE §4.3.16).
pub(crate) const EOCD_SIG: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
/// zip64 end of central directory locator signature, `PK\x06\x07`
/// (APPNOTE §4.3.15) — its mere presence means zip64, so we only ever
/// check for it, never parse it.
const EOCD64_LOCATOR_SIG: [u8; 4] = [0x50, 0x4b, 0x06, 0x07];

const LFH_FIXED_LEN: usize = 30;
const CDFH_FIXED_LEN: usize = 46;
const EOCD_FIXED_LEN: usize = 22;
const EOCD64_LOCATOR_LEN: usize = 20;
const MAX_COMMENT_LEN: usize = 0xFFFF;
/// Bounded EOCD back-scan window (brief: "bounded back-scan 66KB") — the
/// max possible EOCD record (fixed part + max comment), so locating the
/// EOCD is O(64KB) regardless of file size, never O(file size). This is
/// the primary OOM defense: we never allocate or scan proportional to an
/// attacker-controlled declared size before validating it against the
/// actual remaining bytes.
const MAX_BACKSCAN_WINDOW: usize = EOCD_FIXED_LEN + MAX_COMMENT_LEN;

/// zip64/oversize sentinel shared by the u16 entry-count fields and the
/// u32 size/offset fields (APPNOTE §4.4.21-24): its presence means "the
/// real value lives in a zip64 extra field," which this profile forbids
/// outright, so seeing the sentinel anywhere is itself a zip64 signal.
const U16_SENTINEL: u16 = 0xFFFF;
const U32_SENTINEL: u32 = 0xFFFF_FFFF;

/// General-purpose bit flag value this profile requires, exactly (APPNOTE
/// §4.4.4 bit 11, UTF-8 names) — the ONLY value `ContainerReader::open`
/// accepts and the ONLY value `ContainerWriter::finish` ever emits (see
/// module docs on why this is exact equality, not a per-bit check).
pub(crate) const GPBF_UTF8: u16 = 0x0800;

pub(crate) const METHOD_STORE: u16 = 0;

/// Every variant is a fail-closed rejection of the whole container: none of
/// this is ever partially trusted or silently downgraded. The verdict
/// engine (Task 7) maps every variant to `CANNOT_VERIFY(container-profile)`
/// — never `TAMPERED`, never skipped (design doc row 21).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContainerError {
    #[error("archive is smaller than a minimal end-of-central-directory record")]
    TooSmall,
    #[error("no end-of-central-directory signature found in the trailing {window} bytes")]
    EocdNotFound { window: usize },
    #[error("zip64 marker present (locator or sentinel value); zip64 is not supported")]
    Zip64Present,
    #[error("multi-disk archives are not supported ({field} = {value}, must be 0)")]
    MultiDisk { field: &'static str, value: u16 },
    #[error("archive comment must be empty, declared length {len}")]
    NonEmptyArchiveComment { len: u16 },
    #[error("{trailing} trailing byte(s) after the end-of-central-directory record")]
    TrailingBytes { trailing: u64 },
    #[error("central directory bounds [{offset}, {offset}+{size}) exceed the archive")]
    CentralDirectoryOutOfBounds { offset: u64, size: u64 },
    #[error("central directory entry {index} is truncated")]
    TruncatedCentralDirectoryEntry { index: usize },
    #[error("central directory entry {index} has a bad signature")]
    BadCentralDirectorySignature { index: usize },
    #[error("central directory declares {declared} bytes but entries consumed {consumed}")]
    CentralDirectorySizeMismatch { declared: u64, consumed: u64 },
    #[error("member {name:?} uses unsupported compression method {method} (STORE=0 only)")]
    UnsupportedMethod { name: String, method: u16 },
    #[error("member {name:?} has STORE method but compressed_size != uncompressed_size")]
    StoreSizeMismatch { name: String },
    #[error("member {name:?} has a disallowed general-purpose flag (0x{flags:04x})")]
    UnsupportedFlags { name: String, flags: u16 },
    #[error("member {name:?} declares a non-empty extra field ({len} bytes)")]
    NonEmptyExtraField { name: String, len: u16 },
    #[error("member {name:?} declares a non-empty comment ({len} bytes)")]
    NonEmptyMemberComment { name: String, len: u16 },
    #[error("member name at central directory entry {index} is not valid UTF-8")]
    NonUtf8Name { index: usize },
    #[error("member path {name:?} is not in the fixed whitelist")]
    PathNotWhitelisted { name: String },
    #[error("duplicate member name {name:?}")]
    DuplicateMember { name: String },
    #[error("member {name:?} local header offset is out of bounds")]
    LocalHeaderOutOfBounds { name: String },
    #[error("member {name:?} local header has a bad signature")]
    BadLocalHeaderSignature { name: String },
    #[error("member {name:?} local header disagrees with the central directory: {field}")]
    LocalHeaderMismatch { name: String, field: &'static str },
    #[error("member {name:?} data extends past the end of the archive")]
    MemberDataOutOfBounds { name: String },
    #[error("member {name:?} not found in the container")]
    NotFound { name: String },
}

/// Reads a little-endian `u16` at `off`, bounds-checked against `b` rather
/// than `.unwrap()`ing a slice-to-array conversion — every declared size
/// or offset in this module is attacker-controlled, so every read must
/// fail closed instead of panicking (no-unwrap-in-prod bar).
fn read_u16(b: &[u8], off: usize) -> Result<u16, ContainerError> {
    let s = b.get(off..off + 2).ok_or(ContainerError::TooSmall)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

fn read_u32(b: &[u8], off: usize) -> Result<u32, ContainerError> {
    let s = b.get(off..off + 4).ok_or(ContainerError::TooSmall)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// True if `rest` is a single flat path segment: non-empty, contains no
/// `/` or `\`, is not `.` or `..`, and has no embedded NUL. Shared by the
/// three prefixed whitelist categories (`anchors/`, `content/`,
/// `derived/`), which the design doc writes as `anchors/*` etc.
fn is_flat_child(rest: &str) -> bool {
    !rest.is_empty() && rest != "." && rest != ".." && !rest.contains(['/', '\\', '\0'])
}

/// The fixed member path whitelist (design doc "Container: normative
/// restricted profile"). Anything not matched here is rejected regardless
/// of what the manifest later says about it.
pub fn validate_whitelisted_name(name: &str) -> Result<(), ContainerError> {
    let ok = name == "manifest.json"
        || name == "journal/closure.jsonl"
        || name == "checkpoints.jsonl"
        || name == "trust/keys.jsonl"
        || name
            .strip_prefix("journal/epoch-")
            .and_then(|rest| rest.strip_suffix(".jsonl"))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        || name.strip_prefix("anchors/").is_some_and(is_flat_child)
        || name.strip_prefix("content/").is_some_and(is_flat_child)
        || name.strip_prefix("derived/").is_some_and(is_flat_child);
    if ok {
        Ok(())
    } else {
        Err(ContainerError::PathNotWhitelisted {
            name: name.to_string(),
        })
    }
}

struct Eocd {
    offset: usize,
    cd_entries_total: u16,
    cd_size: u32,
    cd_offset: u32,
}

/// Locates and validates the EOCD record: bounded back-scan (see module
/// docs), zip64 rejection (locator presence OR any sentinel field), no
/// multi-disk, no archive comment, no trailing bytes.
fn find_and_parse_eocd(bytes: &[u8]) -> Result<Eocd, ContainerError> {
    if bytes.len() < EOCD_FIXED_LEN {
        return Err(ContainerError::TooSmall);
    }
    let window = MAX_BACKSCAN_WINDOW.min(bytes.len());
    let search_start = bytes.len() - window;
    let last_possible = bytes.len() - EOCD_FIXED_LEN;
    let offset = (search_start..=last_possible)
        .rev()
        .find(|&off| bytes[off..off + 4] == EOCD_SIG)
        .ok_or(ContainerError::EocdNotFound { window })?;

    // zip64 locator: the fixed 20-byte record immediately preceding EOCD.
    if offset >= EOCD64_LOCATOR_LEN
        && bytes[offset - EOCD64_LOCATOR_LEN..offset - EOCD64_LOCATOR_LEN + 4]
            == EOCD64_LOCATOR_SIG
    {
        return Err(ContainerError::Zip64Present);
    }

    let disk_number = read_u16(bytes, offset + 4)?;
    let cd_start_disk = read_u16(bytes, offset + 6)?;
    let cd_entries_this_disk = read_u16(bytes, offset + 8)?;
    let cd_entries_total = read_u16(bytes, offset + 10)?;
    let cd_size = read_u32(bytes, offset + 12)?;
    let cd_offset = read_u32(bytes, offset + 16)?;
    let comment_len = read_u16(bytes, offset + 20)?;

    if disk_number != 0 {
        return Err(ContainerError::MultiDisk {
            field: "disk_number",
            value: disk_number,
        });
    }
    if cd_start_disk != 0 {
        return Err(ContainerError::MultiDisk {
            field: "cd_start_disk",
            value: cd_start_disk,
        });
    }
    if cd_entries_this_disk != cd_entries_total {
        return Err(ContainerError::MultiDisk {
            field: "cd_entries_this_disk",
            value: cd_entries_this_disk,
        });
    }
    if cd_entries_total == U16_SENTINEL || cd_size == U32_SENTINEL || cd_offset == U32_SENTINEL {
        return Err(ContainerError::Zip64Present);
    }
    if comment_len != 0 {
        return Err(ContainerError::NonEmptyArchiveComment { len: comment_len });
    }
    let end = offset + EOCD_FIXED_LEN + comment_len as usize;
    if end != bytes.len() {
        return Err(ContainerError::TrailingBytes {
            trailing: (bytes.len() - end) as u64,
        });
    }

    Ok(Eocd {
        offset,
        cd_entries_total,
        cd_size,
        cd_offset,
    })
}

struct CdEntry {
    name: String,
    method: u16,
    compressed_size: u32,
    uncompressed_size: u32,
    local_header_offset: u32,
}

/// Parses every central directory entry, applying every CD-only profile
/// check (method, flags, extra/comment emptiness, whitelist, size
/// self-consistency, zip64 sentinels) — everything EXCEPT the local-header
/// cross-check and duplicate-name check, which `ContainerReader::open`
/// applies afterward (duplicate detection needs the full entry list;
/// local-header agreement needs a second pass over the file).
fn parse_central_directory(bytes: &[u8], eocd: &Eocd) -> Result<Vec<CdEntry>, ContainerError> {
    let cd_start = eocd.cd_offset as usize;
    let cd_len = eocd.cd_size as usize;
    let cd_end = cd_start
        .checked_add(cd_len)
        .filter(|&end| end <= eocd.offset)
        .ok_or(ContainerError::CentralDirectoryOutOfBounds {
            offset: eocd.cd_offset as u64,
            size: eocd.cd_size as u64,
        })?;
    let cd = &bytes[cd_start..cd_end];

    let mut entries = Vec::with_capacity(eocd.cd_entries_total as usize);
    let mut cursor = 0usize;
    for index in 0..eocd.cd_entries_total as usize {
        if cursor + CDFH_FIXED_LEN > cd.len() {
            return Err(ContainerError::TruncatedCentralDirectoryEntry { index });
        }
        if cd[cursor..cursor + 4] != CDFH_SIG {
            return Err(ContainerError::BadCentralDirectorySignature { index });
        }
        let gp_flag = read_u16(cd, cursor + 8)?;
        let method = read_u16(cd, cursor + 10)?;
        let compressed_size = read_u32(cd, cursor + 20)?;
        let uncompressed_size = read_u32(cd, cursor + 24)?;
        let name_len = read_u16(cd, cursor + 28)? as usize;
        let extra_len = read_u16(cd, cursor + 30)? as usize;
        let comment_len = read_u16(cd, cursor + 32)? as usize;
        let disk_start = read_u16(cd, cursor + 34)?;
        let local_header_offset = read_u32(cd, cursor + 42)?;

        let name_start = cursor + CDFH_FIXED_LEN;
        let total_len = CDFH_FIXED_LEN + name_len + extra_len + comment_len;
        if cursor + total_len > cd.len() {
            return Err(ContainerError::TruncatedCentralDirectoryEntry { index });
        }
        let name_bytes = &cd[name_start..name_start + name_len];
        let name = std::str::from_utf8(name_bytes)
            .map_err(|_| ContainerError::NonUtf8Name { index })?
            .to_string();

        if compressed_size == U32_SENTINEL
            || uncompressed_size == U32_SENTINEL
            || local_header_offset == U32_SENTINEL
        {
            return Err(ContainerError::Zip64Present);
        }
        if disk_start != 0 {
            return Err(ContainerError::MultiDisk {
                field: "entry disk_start",
                value: disk_start,
            });
        }
        if method != METHOD_STORE {
            return Err(ContainerError::UnsupportedMethod { name, method });
        }
        if compressed_size != uncompressed_size {
            return Err(ContainerError::StoreSizeMismatch { name });
        }
        // Exact equality, not per-bit (see module docs): the writer only
        // ever emits `GPBF_UTF8` alone, so any other value — including an
        // unspecified/reserved bit our own writer would never set — is
        // rejected. This removes every unspecified-bit axis a faithful
        // but independently-written reader (Task 14) could otherwise
        // diverge on.
        if gp_flag != GPBF_UTF8 {
            return Err(ContainerError::UnsupportedFlags {
                name,
                flags: gp_flag,
            });
        }
        if extra_len != 0 {
            return Err(ContainerError::NonEmptyExtraField {
                name,
                len: extra_len as u16,
            });
        }
        if comment_len != 0 {
            return Err(ContainerError::NonEmptyMemberComment {
                name,
                len: comment_len as u16,
            });
        }
        validate_whitelisted_name(&name)?;

        entries.push(CdEntry {
            name,
            method,
            compressed_size,
            uncompressed_size,
            local_header_offset,
        });

        cursor += total_len;
    }

    if cursor != cd.len() {
        return Err(ContainerError::CentralDirectorySizeMismatch {
            declared: cd.len() as u64,
            consumed: cursor as u64,
        });
    }

    Ok(entries)
}

/// Cross-checks `entry`'s local header (name/method/sizes/extra_len must
/// byte-agree with the central directory — see module docs for why
/// `extra_len` is included alongside the brief's literal name/sizes/
/// method) and returns the member's data slice, bounds-checked against
/// both the archive length and the central directory start (member data
/// must not overlap the central directory).
fn read_and_verify_local_header<'a>(
    bytes: &'a [u8],
    entry: &CdEntry,
    cd_start: usize,
) -> Result<&'a [u8], ContainerError> {
    let lh_off = entry.local_header_offset as usize;
    let oob = || ContainerError::LocalHeaderOutOfBounds {
        name: entry.name.clone(),
    };
    if lh_off.checked_add(LFH_FIXED_LEN).filter(|&e| e <= cd_start && e <= bytes.len()).is_none() {
        return Err(oob());
    }
    if bytes[lh_off..lh_off + 4] != LFH_SIG {
        return Err(ContainerError::BadLocalHeaderSignature {
            name: entry.name.clone(),
        });
    }

    let method = read_u16(bytes, lh_off + 8)?;
    let compressed_size = read_u32(bytes, lh_off + 18)?;
    let uncompressed_size = read_u32(bytes, lh_off + 22)?;
    let name_len = read_u16(bytes, lh_off + 26)? as usize;
    let extra_len = read_u16(bytes, lh_off + 28)? as usize;

    let name_start = lh_off + LFH_FIXED_LEN;
    let name_end = name_start
        .checked_add(name_len)
        .filter(|&e| e <= cd_start && e <= bytes.len())
        .ok_or_else(oob)?;
    let name_bytes = &bytes[name_start..name_end];
    let mismatch = |field| ContainerError::LocalHeaderMismatch {
        name: entry.name.clone(),
        field,
    };
    if name_bytes != entry.name.as_bytes() {
        return Err(mismatch("name"));
    }
    if method != entry.method {
        return Err(mismatch("method"));
    }
    if compressed_size != entry.compressed_size {
        return Err(mismatch("compressed_size"));
    }
    if uncompressed_size != entry.uncompressed_size {
        return Err(mismatch("uncompressed_size"));
    }
    if extra_len != 0 {
        return Err(mismatch("extra_len"));
    }

    let data_start = name_end; // extra_len == 0, just verified
    let data_end = data_start
        .checked_add(entry.compressed_size as usize)
        .filter(|&e| e <= cd_start && e <= bytes.len())
        .ok_or_else(|| ContainerError::MemberDataOutOfBounds {
            name: entry.name.clone(),
        })?;

    Ok(&bytes[data_start..data_end])
}

/// A parsed, fully profile-validated `.dbev` container. Only constructible
/// via [`ContainerReader::open`] — its existence IS the proof every
/// structural check (STORE-only, no zip64, whitelist, duplicate names,
/// local/CD agreement) already passed for EVERY member, not just the ones
/// later accessed.
#[derive(Debug)]
pub struct ContainerReader<'a> {
    names: Vec<String>,
    index: HashMap<String, &'a [u8]>,
}

impl<'a> ContainerReader<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self, ContainerError> {
        let eocd = find_and_parse_eocd(bytes)?;
        let cd_entries = parse_central_directory(bytes, &eocd)?;

        let mut names = Vec::with_capacity(cd_entries.len());
        let mut seen = HashSet::with_capacity(cd_entries.len());
        let mut index = HashMap::with_capacity(cd_entries.len());

        for entry in &cd_entries {
            if !seen.insert(entry.name.clone()) {
                return Err(ContainerError::DuplicateMember {
                    name: entry.name.clone(),
                });
            }
            let data = read_and_verify_local_header(bytes, entry, eocd.cd_offset as usize)?;
            names.push(entry.name.clone());
            index.insert(entry.name.clone(), data);
        }

        Ok(ContainerReader { names, index })
    }

    /// Every member name, in central-directory order.
    pub fn member_names(&self) -> &[String] {
        &self.names
    }

    /// The member's raw (STORE, i.e. literal) bytes.
    pub fn member_bytes(&self, name: &str) -> Result<&[u8], ContainerError> {
        self.index
            .get(name)
            .copied()
            .ok_or_else(|| ContainerError::NotFound {
                name: name.to_string(),
            })
    }
}

/// CRC-32 (ISO-HDLC / zlib polynomial 0xEDB88320), computed bit-by-bit —
/// informational only: `ContainerReader` never reads or checks this field
/// (see module docs; the brief's cross-check enumeration is name/sizes/
/// method, not CRC), so this exists purely so a `.dbev` written by
/// `ContainerWriter` is byte-compatible with real unzip tooling when a
/// human inspects one by hand (the live break-test procedure opens a
/// `.dbev` in a hex editor). `pub(crate)` for the same reason as the
/// signature constants above (Task 7's `BundleBuilder` raw-ZIP mutations).
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Deterministic STORE-only ZIP writer: members are emitted in insertion
/// order (the exporter's job, Task 11, is to insert in manifest order so
/// container layout and manifest order agree), with the exact profile
/// `ContainerReader::open` enforces baked in by construction — the writer
/// literally cannot produce a DEFLATE entry, an extra field, a non-empty
/// comment, or a non-whitelisted path, so a bundle it builds always
/// round-trips through `ContainerReader::open`.
#[derive(Default)]
pub struct ContainerWriter {
    members: Vec<(String, Vec<u8>)>,
}

impl ContainerWriter {
    pub fn new() -> Self {
        ContainerWriter {
            members: Vec::new(),
        }
    }

    /// Adds one member. Rejects a non-whitelisted path or a duplicate name
    /// immediately, so a caller cannot build a bundle the reader would
    /// later refuse.
    pub fn add_member(&mut self, name: &str, data: Vec<u8>) -> Result<(), ContainerError> {
        validate_whitelisted_name(name)?;
        if self.members.iter().any(|(n, _)| n == name) {
            return Err(ContainerError::DuplicateMember {
                name: name.to_string(),
            });
        }
        self.members.push((name.to_string(), data));
        Ok(())
    }

    /// Serializes to STORE-only ZIP bytes: LFH+data per member (insertion
    /// order), then the central directory, then the EOCD — every field set
    /// to exactly what the profile requires (method 0, gp_flag = UTF-8 bit
    /// only, empty extra fields, empty comments, single disk).
    pub fn finish(&self) -> Result<Vec<u8>, ContainerError> {
        let mut out = Vec::new();
        let mut local_offsets = Vec::with_capacity(self.members.len());

        for (name, data) in &self.members {
            let offset: u32 = out.len().try_into().map_err(|_| {
                ContainerError::MemberDataOutOfBounds { name: name.clone() }
            })?;
            local_offsets.push(offset);

            let name_bytes = name.as_bytes();
            let size: u32 = data.len().try_into().map_err(|_| {
                ContainerError::MemberDataOutOfBounds { name: name.clone() }
            })?;
            let crc = crc32(data);

            out.extend_from_slice(&LFH_SIG);
            out.extend_from_slice(&0u16.to_le_bytes()); // version needed
            out.extend_from_slice(&GPBF_UTF8.to_le_bytes()); // gp flag: UTF-8 only
            out.extend_from_slice(&METHOD_STORE.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // mod time
            out.extend_from_slice(&0u16.to_le_bytes()); // mod date
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes()); // compressed size
            out.extend_from_slice(&size.to_le_bytes()); // uncompressed size
            out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra len
            out.extend_from_slice(name_bytes);
            out.extend_from_slice(data);
        }

        let cd_start: u32 = out
            .len()
            .try_into()
            .map_err(|_| ContainerError::CentralDirectoryOutOfBounds {
                offset: out.len() as u64,
                size: 0,
            })?;

        for ((name, data), &local_offset) in self.members.iter().zip(&local_offsets) {
            let name_bytes = name.as_bytes();
            let size = data.len() as u32;
            let crc = crc32(data);

            out.extend_from_slice(&CDFH_SIG);
            out.extend_from_slice(&0u16.to_le_bytes()); // version made by
            out.extend_from_slice(&0u16.to_le_bytes()); // version needed
            out.extend_from_slice(&GPBF_UTF8.to_le_bytes());
            out.extend_from_slice(&METHOD_STORE.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // mod time
            out.extend_from_slice(&0u16.to_le_bytes()); // mod date
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra len
            out.extend_from_slice(&0u16.to_le_bytes()); // comment len
            out.extend_from_slice(&0u16.to_le_bytes()); // disk start
            out.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            out.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            out.extend_from_slice(&local_offset.to_le_bytes());
            out.extend_from_slice(name_bytes);
        }

        let cd_end: u32 = out.len().try_into().map_err(|_| {
            ContainerError::CentralDirectoryOutOfBounds {
                offset: cd_start as u64,
                size: 0,
            }
        })?;
        let cd_size = cd_end - cd_start;
        let entry_count: u16 = self
            .members
            .len()
            .try_into()
            .map_err(|_| ContainerError::Zip64Present)?;

        out.extend_from_slice(&EOCD_SIG);
        out.extend_from_slice(&0u16.to_le_bytes()); // disk number
        out.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
        out.extend_from_slice(&entry_count.to_le_bytes());
        out.extend_from_slice(&entry_count.to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_start.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment len

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Attack-only bit values (APPNOTE §4.4.4) — the production code no
    /// longer names these individually (see `GPBF_UTF8`'s doc comment:
    /// the reader checks exact equality, not per-bit), so tests craft
    /// them directly to prove the exact-equality check still catches
    /// each one.
    const GPBF_ENCRYPTED: u16 = 0x0001;
    const GPBF_DATA_DESCRIPTOR: u16 = 0x0008;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        // The canonical CRC-32/ISO-HDLC check value for the ASCII string
        // "123456789" (used by every CRC-32 implementation's test suite).
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    // ---- whitelist ----

    #[test]
    fn whitelist_accepts_every_documented_path_shape() {
        for name in [
            "manifest.json",
            "journal/epoch-0.jsonl",
            "journal/epoch-42.jsonl",
            "journal/closure.jsonl",
            "checkpoints.jsonl",
            "trust/keys.jsonl",
            "anchors/tsa-1.token",
            "content/rec-abc123",
            "derived/report.pdf",
        ] {
            assert!(
                validate_whitelisted_name(name).is_ok(),
                "{name} should be whitelisted"
            );
        }
    }

    #[test]
    fn whitelist_rejects_path_traversal_and_nesting() {
        for name in [
            "anchors/../../../etc/passwd",
            "anchors/..",
            "anchors/",
            "anchors/sub/dir",
            "content/../secret",
            "derived/a/b",
            "../manifest.json",
            "journal/epoch-.jsonl",
            "journal/epoch-1a.jsonl",
            "journal/epoch-.jsonl",
            "random.txt",
            "",
        ] {
            assert!(
                validate_whitelisted_name(name).is_err(),
                "{name} should be rejected"
            );
        }
    }

    // ---- ContainerWriter / ContainerReader round trip ----

    #[test]
    fn valid_write_then_read_round_trips_every_member() {
        let mut w = ContainerWriter::new();
        w.add_member("manifest.json", b"{\"manifest\":true}".to_vec())
            .unwrap();
        w.add_member("journal/epoch-0.jsonl", b"line1\nline2\n".to_vec())
            .unwrap();
        w.add_member("checkpoints.jsonl", Vec::new()).unwrap(); // empty member
        w.add_member("content/rec-1", vec![0xAB; 500]).unwrap();
        let bytes = w.finish().unwrap();

        let reader = ContainerReader::open(&bytes).expect("valid container must open");
        let mut names = reader.member_names().to_vec();
        names.sort();
        assert_eq!(
            names,
            vec![
                "checkpoints.jsonl",
                "content/rec-1",
                "journal/epoch-0.jsonl",
                "manifest.json",
            ]
        );
        assert_eq!(
            reader.member_bytes("manifest.json").unwrap(),
            b"{\"manifest\":true}"
        );
        assert_eq!(
            reader.member_bytes("journal/epoch-0.jsonl").unwrap(),
            b"line1\nline2\n"
        );
        assert_eq!(reader.member_bytes("checkpoints.jsonl").unwrap(), b"");
        assert_eq!(reader.member_bytes("content/rec-1").unwrap(), &[0xAB; 500][..]);
        assert!(matches!(
            reader.member_bytes("nope").unwrap_err(),
            ContainerError::NotFound { .. }
        ));
    }

    #[test]
    fn writer_rejects_non_whitelisted_and_duplicate_names() {
        let mut w = ContainerWriter::new();
        assert!(w.add_member("not-allowed.txt", vec![]).is_err());
        w.add_member("manifest.json", vec![1]).unwrap();
        assert!(matches!(
            w.add_member("manifest.json", vec![2]),
            Err(ContainerError::DuplicateMember { .. })
        ));
    }

    #[test]
    fn empty_archive_opens_with_zero_members() {
        let w = ContainerWriter::new();
        let bytes = w.finish().unwrap();
        let reader = ContainerReader::open(&bytes).expect("zero-member archive is structurally valid");
        assert!(reader.member_names().is_empty());
    }

    // ---- hand-crafted structural-attack bytes ----
    //
    // These build raw ZIP bytes field-by-field (never through
    // `ContainerWriter`, which cannot produce a profile violation by
    // construction) so every REQUIRED parser-differential case is a
    // literal byte layout, not something inferred from a library.

    /// One local-header + central-directory-entry pair under full field
    /// control, so a test can override exactly one field to attack one
    /// rule at a time.
    struct RawEntry {
        name: &'static [u8],
        method: u16,
        gp_flag: u16,
        extra: Vec<u8>,
        comment: Vec<u8>,
        compressed_size: u32,
        uncompressed_size_override: Option<u32>,
        data: Vec<u8>,
        local_header_offset_override: Option<u32>,
        cd_disk_start: u16,
        // when Some, the LOCAL header's extra/name diverges from the CD's,
        // to attack the local/CD cross-check independently of the CD-only
        // checks above.
        local_extra_override: Option<Vec<u8>>,
        local_name_override: Option<&'static [u8]>,
        local_method_override: Option<u16>,
        local_compressed_size_override: Option<u32>,
        local_uncompressed_size_override: Option<u32>,
    }

    impl RawEntry {
        fn store(name: &'static [u8], data: Vec<u8>) -> Self {
            let size = data.len() as u32;
            RawEntry {
                name,
                method: METHOD_STORE,
                gp_flag: GPBF_UTF8,
                extra: Vec::new(),
                comment: Vec::new(),
                compressed_size: size,
                uncompressed_size_override: None,
                data,
                local_header_offset_override: None,
                cd_disk_start: 0,
                local_extra_override: None,
                local_name_override: None,
                local_method_override: None,
                local_compressed_size_override: None,
                local_uncompressed_size_override: None,
            }
        }
    }

    /// Hand-rolled raw ZIP byte builder: emits exactly what each `RawEntry`
    /// says, with no profile enforcement of its own, so tests can produce
    /// deliberately-malformed containers.
    #[derive(Default)]
    struct RawZipBuilder {
        entries: Vec<RawEntry>,
        eocd_comment: Vec<u8>,
        force_zip64_locator: bool,
        force_zip64_sentinel_total_entries: bool,
        trailing_garbage: Vec<u8>,
    }

    impl RawZipBuilder {
        fn add(&mut self, e: RawEntry) -> &mut Self {
            self.entries.push(e);
            self
        }

        fn build(&self) -> Vec<u8> {
            let mut out = Vec::new();
            let mut local_offsets = Vec::with_capacity(self.entries.len());

            for e in &self.entries {
                let offset = e
                    .local_header_offset_override
                    .unwrap_or(out.len() as u32);
                local_offsets.push(offset);
                // The offset override may not equal the true position (an
                // attack), but we still emit the local header at the
                // TRUE current write position — the override only changes
                // what the CENTRAL DIRECTORY *claims* the offset is.
                let local_name = e.local_name_override.unwrap_or(e.name);
                let local_extra = e.local_extra_override.clone().unwrap_or_default();
                let local_method = e.local_method_override.unwrap_or(e.method);
                let local_compressed = e
                    .local_compressed_size_override
                    .unwrap_or(e.compressed_size);
                let local_uncompressed = e
                    .local_uncompressed_size_override
                    .unwrap_or(e.uncompressed_size_override.unwrap_or(e.compressed_size));

                out.extend_from_slice(&LFH_SIG);
                out.extend_from_slice(&0u16.to_le_bytes());
                out.extend_from_slice(&e.gp_flag.to_le_bytes());
                out.extend_from_slice(&local_method.to_le_bytes());
                out.extend_from_slice(&0u16.to_le_bytes());
                out.extend_from_slice(&0u16.to_le_bytes());
                out.extend_from_slice(&crc32(&e.data).to_le_bytes());
                out.extend_from_slice(&local_compressed.to_le_bytes());
                out.extend_from_slice(&local_uncompressed.to_le_bytes());
                out.extend_from_slice(&(local_name.len() as u16).to_le_bytes());
                out.extend_from_slice(&(local_extra.len() as u16).to_le_bytes());
                out.extend_from_slice(local_name);
                out.extend_from_slice(&local_extra);
                out.extend_from_slice(&e.data);
            }

            let cd_start = out.len() as u32;
            for (e, &local_offset) in self.entries.iter().zip(&local_offsets) {
                let uncompressed = e.uncompressed_size_override.unwrap_or(e.compressed_size);
                out.extend_from_slice(&CDFH_SIG);
                out.extend_from_slice(&0u16.to_le_bytes()); // version made by
                out.extend_from_slice(&0u16.to_le_bytes()); // version needed
                out.extend_from_slice(&e.gp_flag.to_le_bytes());
                out.extend_from_slice(&e.method.to_le_bytes());
                out.extend_from_slice(&0u16.to_le_bytes());
                out.extend_from_slice(&0u16.to_le_bytes());
                out.extend_from_slice(&crc32(&e.data).to_le_bytes());
                out.extend_from_slice(&e.compressed_size.to_le_bytes());
                out.extend_from_slice(&uncompressed.to_le_bytes());
                out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
                out.extend_from_slice(&(e.extra.len() as u16).to_le_bytes());
                out.extend_from_slice(&(e.comment.len() as u16).to_le_bytes());
                out.extend_from_slice(&e.cd_disk_start.to_le_bytes());
                out.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
                out.extend_from_slice(&0u32.to_le_bytes()); // external attrs
                out.extend_from_slice(&local_offset.to_le_bytes());
                out.extend_from_slice(e.name);
                out.extend_from_slice(&e.extra);
                out.extend_from_slice(&e.comment);
            }
            let cd_end = out.len() as u32;
            let cd_size = cd_end - cd_start;

            if self.force_zip64_locator {
                out.extend_from_slice(&EOCD64_LOCATOR_SIG);
                out.extend_from_slice(&[0u8; EOCD64_LOCATOR_LEN - 4]);
            }

            let entry_count = if self.force_zip64_sentinel_total_entries {
                U16_SENTINEL
            } else {
                self.entries.len() as u16
            };

            out.extend_from_slice(&EOCD_SIG);
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&entry_count.to_le_bytes());
            out.extend_from_slice(&entry_count.to_le_bytes());
            out.extend_from_slice(&cd_size.to_le_bytes());
            out.extend_from_slice(&cd_start.to_le_bytes());
            out.extend_from_slice(&(self.eocd_comment.len() as u16).to_le_bytes());
            out.extend_from_slice(&self.eocd_comment);
            out.extend_from_slice(&self.trailing_garbage);

            out
        }
    }

    fn one_valid_entry() -> RawEntry {
        RawEntry::store(b"manifest.json", b"hello".to_vec())
    }

    /// A sane baseline archive (one whitelisted member) must open cleanly
    /// through the RAW builder too — this proves every attack test below
    /// is changing exactly one thing relative to a known-good baseline,
    /// not accidentally testing a builder bug.
    #[test]
    fn raw_builder_baseline_opens_cleanly() {
        let mut b = RawZipBuilder::default();
        b.add(one_valid_entry());
        let bytes = b.build();
        let reader = ContainerReader::open(&bytes).expect("baseline must open");
        assert_eq!(reader.member_bytes("manifest.json").unwrap(), b"hello");
    }

    // 4.1: duplicate member name (two `manifest.json`).
    #[test]
    fn duplicate_member_name_is_rejected() {
        let mut b = RawZipBuilder::default();
        b.add(RawEntry::store(b"manifest.json", b"one".to_vec()));
        b.add(RawEntry::store(b"manifest.json", b"two".to_vec()));
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::DuplicateMember { .. }), "{err:?}");
    }

    // 4.2: central-directory size != local-header size for a member.
    #[test]
    fn cd_size_disagreeing_with_local_header_size_is_rejected() {
        let mut e = one_valid_entry(); // data = 5 bytes, cd compressed_size = 5
        e.local_compressed_size_override = Some(4); // local header LIES: says 4
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(
            matches!(
                err,
                ContainerError::LocalHeaderMismatch {
                    field: "compressed_size",
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn cd_uncompressed_size_disagreeing_with_local_header_is_rejected() {
        let mut e = one_valid_entry();
        e.local_uncompressed_size_override = Some(4);
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(
            matches!(
                err,
                ContainerError::LocalHeaderMismatch {
                    field: "uncompressed_size",
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn cd_name_disagreeing_with_local_header_name_is_rejected() {
        let mut e = one_valid_entry();
        e.local_name_override = Some(b"checkpoints.jsonl");
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(
            matches!(err, ContainerError::LocalHeaderMismatch { field: "name", .. }),
            "{err:?}"
        );
    }

    #[test]
    fn cd_method_disagreeing_with_local_header_method_is_rejected() {
        let mut e = one_valid_entry();
        e.local_method_override = Some(8); // DEFLATE, locally only
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(
            matches!(err, ContainerError::LocalHeaderMismatch { field: "method", .. }),
            "{err:?}"
        );
    }

    #[test]
    fn local_extra_field_disagreeing_with_cd_extra_len_is_rejected() {
        let mut e = one_valid_entry(); // CD extra_len = 0
        e.local_extra_override = Some(vec![0xAA, 0xBB]); // local extra_len = 2
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(
            matches!(
                err,
                ContainerError::LocalHeaderMismatch { field: "extra_len", .. }
            ),
            "{err:?}"
        );
    }

    // 4.3: member compressed with DEFLATE.
    #[test]
    fn deflate_method_is_rejected() {
        let mut e = one_valid_entry();
        e.method = 8; // DEFLATE
        e.local_method_override = Some(8); // keep local/CD agreeing so THIS
                                            // check (not the cross-check) fires
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(
            matches!(err, ContainerError::UnsupportedMethod { method: 8, .. }),
            "{err:?}"
        );
    }

    // 4.4: extra member not in whitelist path set.
    #[test]
    fn non_whitelisted_path_is_rejected() {
        let mut b = RawZipBuilder::default();
        b.add(RawEntry::store(b"secrets.txt", b"x".to_vec()));
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::PathNotWhitelisted { .. }), "{err:?}");
    }

    // 4.8: zip64 EOCD present (locator variant).
    #[test]
    fn zip64_locator_is_rejected() {
        let mut b = RawZipBuilder::default();
        b.add(one_valid_entry());
        b.force_zip64_locator = true;
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::Zip64Present), "{err:?}");
    }

    // 4.8 variant: zip64 signaled via the EOCD entry-count sentinel alone.
    #[test]
    fn zip64_entry_count_sentinel_is_rejected() {
        let mut b = RawZipBuilder::default();
        b.add(one_valid_entry());
        b.force_zip64_sentinel_total_entries = true;
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::Zip64Present), "{err:?}");
    }

    // 4.9: truncate the file at several offsets — every truncation must
    // fail closed (some form of ContainerError), never panic and never
    // silently accept a shorter/wrong container.
    #[test]
    fn truncation_at_many_offsets_always_fails_closed() {
        let mut b = RawZipBuilder::default();
        b.add(RawEntry::store(b"manifest.json", vec![7u8; 40]));
        b.add(RawEntry::store(b"checkpoints.jsonl", vec![9u8; 20]));
        let full = b.build();
        assert!(ContainerReader::open(&full).is_ok(), "full archive must be valid");

        let step = (full.len() / 15).max(1);
        for cut in (0..full.len()).step_by(step) {
            let truncated = &full[..cut];
            // Must not panic (the real assertion — bounds-checked reads
            // throughout) and must never claim success on a truncated
            // container.
            let result = std::panic::catch_unwind(|| ContainerReader::open(truncated));
            match result {
                Ok(Ok(_)) => panic!("truncation at {cut} must not open successfully"),
                Ok(Err(_)) => {} // expected
                Err(_) => panic!("truncation at {cut} panicked instead of returning Err"),
            }
        }
    }

    // 4.10: non-empty extra field on a member (CD-declared).
    #[test]
    fn non_empty_extra_field_is_rejected() {
        let mut e = one_valid_entry();
        e.extra = vec![0x01, 0x02, 0x03, 0x04];
        // keep local header agreeing so the CD-only extra-field check
        // (not the cross-check) is what fires.
        e.local_extra_override = Some(e.extra.clone());
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::NonEmptyExtraField { .. }), "{err:?}");
    }

    // 4.11: data-descriptor bit set.
    #[test]
    fn data_descriptor_bit_is_rejected() {
        let mut e = one_valid_entry();
        e.gp_flag |= GPBF_DATA_DESCRIPTOR;
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::UnsupportedFlags { .. }), "{err:?}");
    }

    #[test]
    fn encryption_bit_is_rejected() {
        let mut e = one_valid_entry();
        e.gp_flag |= GPBF_ENCRYPTED;
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::UnsupportedFlags { .. }), "{err:?}");
    }

    #[test]
    fn missing_utf8_bit_is_rejected() {
        let mut e = one_valid_entry();
        e.gp_flag = 0; // UTF-8 bit clear
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::UnsupportedFlags { .. }), "{err:?}");
    }

    #[test]
    fn an_unrelated_gp_flag_bit_is_rejected() {
        // gp_flag is checked by exact equality to 0x0800 (see module
        // docs): an otherwise-unspecified bit set alongside UTF-8 must
        // still be rejected, since `ContainerWriter` never emits it.
        let mut e = one_valid_entry();
        e.gp_flag |= 0x0002; // an unrelated, unspecified bit
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::UnsupportedFlags { .. }), "{err:?}");
    }

    #[test]
    fn gp_flag_zero_is_rejected() {
        // 0x0000 clears encryption/data-descriptor (fine) but ALSO clears
        // the required UTF-8 bit, so it must fail — pinned separately from
        // `missing_utf8_bit_is_rejected` (same craft) to name the exact
        // value the review asked to check explicitly.
        let mut e = one_valid_entry();
        e.gp_flag = 0x0000;
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::UnsupportedFlags { .. }), "{err:?}");
    }

    #[test]
    fn gp_flag_0x0801_is_rejected() {
        // UTF-8 bit set (0x0800) PLUS the encryption bit (0x0001): would
        // already be caught even under a per-bit check, but pins the
        // exact value the review named.
        let mut e = one_valid_entry();
        e.gp_flag = 0x0801;
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::UnsupportedFlags { .. }), "{err:?}");
    }

    // 4.12: garbage bytes appended after EOCD.
    #[test]
    fn trailing_garbage_after_eocd_is_rejected() {
        let mut b = RawZipBuilder::default();
        b.add(one_valid_entry());
        b.trailing_garbage = b"GARBAGE-NOT-PART-OF-THE-ARCHIVE".to_vec();
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::TrailingBytes { .. }), "{err:?}");
    }

    #[test]
    fn non_empty_archive_comment_is_rejected() {
        let mut b = RawZipBuilder::default();
        b.add(one_valid_entry());
        b.eocd_comment = b"hi".to_vec();
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::NonEmptyArchiveComment { .. }), "{err:?}");
    }

    #[test]
    fn non_empty_member_comment_is_rejected() {
        let mut e = one_valid_entry();
        e.comment = b"nope".to_vec();
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::NonEmptyMemberComment { .. }), "{err:?}");
    }

    #[test]
    fn non_zero_member_disk_start_is_rejected() {
        let mut e = one_valid_entry();
        e.cd_disk_start = 1;
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::MultiDisk { .. }), "{err:?}");
    }

    #[test]
    fn store_method_with_mismatched_sizes_is_rejected() {
        let mut e = one_valid_entry(); // data = 5 bytes
        e.uncompressed_size_override = Some(999); // CD claims a different
                                                    // uncompressed size for a
                                                    // STORE (0-compression) entry
        e.local_uncompressed_size_override = Some(999); // keep local/CD
                                                          // agreeing so the
                                                          // STORE-self-consistency
                                                          // check fires, not the
                                                          // cross-check
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::StoreSizeMismatch { .. }), "{err:?}");
    }

    #[test]
    fn non_utf8_name_bytes_are_rejected() {
        let mut e = one_valid_entry();
        // 0xFF is not valid UTF-8 in any position; use a name that would
        // otherwise (if decodable) not even matter, since decoding fails
        // first.
        e.name = b"content/\xff\xfe";
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::NonUtf8Name { .. }), "{err:?}");
    }

    #[test]
    fn archive_smaller_than_eocd_is_rejected() {
        let err = ContainerReader::open(b"too small").unwrap_err();
        assert!(matches!(err, ContainerError::TooSmall));
    }

    #[test]
    fn bad_local_header_signature_is_rejected() {
        let mut b = RawZipBuilder::default();
        b.add(one_valid_entry());
        let mut bytes = b.build();
        // Corrupt the local header signature's first byte.
        bytes[0] = 0x00;
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(matches!(err, ContainerError::BadLocalHeaderSignature { .. }), "{err:?}");
    }

    #[test]
    fn local_header_offset_pointing_past_cd_start_is_rejected() {
        let mut e = one_valid_entry();
        e.local_header_offset_override = Some(u32::MAX - 4); // wildly out of range
        let mut b = RawZipBuilder::default();
        b.add(e);
        let bytes = b.build();
        let err = ContainerReader::open(&bytes).unwrap_err();
        assert!(
            matches!(err, ContainerError::LocalHeaderOutOfBounds { .. }),
            "{err:?}"
        );
    }
}
