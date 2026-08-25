// SPDX-License-Identifier: MIT
//! DocBrain evidence bundle: DSSE-signed, verifiable evidence for claims.

pub mod builder;
pub mod chain;
pub mod checkpoint;
pub mod container;
pub mod ed25519_pin;
pub mod envelope;
#[cfg(all(test, feature = "gen-vectors"))]
mod gen_vectors;
pub mod hash;
pub mod keys;
pub mod manifest;
pub mod pae;
mod strict;
pub mod verdict;
pub mod verify;

pub use builder::BundleBuilder;
pub use chain::{chain_heads, parse_record, walk_chain, ChainError, RecordHeader};
pub use checkpoint::{
    range_bounds, verify_checkpoint_chain, Checkpoint, CheckpointChain, ClockAnomaly, CpError,
    RangeBounds,
};
pub use container::{ContainerError, ContainerReader, ContainerWriter};
pub use ed25519_pin::{verify_pinned, PIN};
pub use envelope::{
    sign_envelope, verify_envelope, Envelope, EnvelopeError, PT_CHECKPOINT, PT_KEYRECORD,
    PT_MANIFEST, PT_RECORD,
};
pub use hash::{content_hash, head_hash, leaf_hash, GENESIS_PREV};
pub use keys::{
    classify_compromise, key_at_position, verify_key_chain, CompromiseClass, CompromiseRecord,
    KeyChain, KeyChainError,
};
pub use manifest::{
    verify_manifest, verify_members, Counts, ExportCheckpointRef, Manifest, ManifestError,
    MemberError, Scope, MANIFEST_MEMBER_NAME,
};
pub use pae::pae;
pub use verify::{
    chain_heads_for_bundle, read_records, verify_bundle, verify_bundle_with_witness, BundleHeads,
    ReadError,
};
pub use verdict::{
    aggregate, classify, AnchorTier, CountsSummary, Disposition, Finding, ScopeSummary, TimeSpan,
    Verdict, VerdictReport, NEGATIVE_SPACE,
};
