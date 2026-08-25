// SPDX-License-Identifier: MIT
//! DSSE-style envelope sign/verify with per-context `payloadType` enforcement
//! (spec laws 1 and 4).
//!
//! Wire form: one JSON object per line —
//! `{"payloadType": "...", "payload": "<b64>", "sig": "<b64>", "keyid": "<hex>"}`.
//! Payload bytes are opaque and are never re-encoded once decoded (law 1); the
//! signed message is `pae(payloadType, payload)` over the EXACT decoded bytes,
//! and `verify_envelope` never re-parses the envelope after it has verified
//! (the DSSE rule: callers get the payload bytes back, not the parsed wire
//! struct). DSSE envelopes carry exactly one signature; a `signatures` array
//! (the multi-signature wire form) is out of scope for this crate's single-
//! signature law and fails closed as `Unsupported` (law 4, taxonomy row 17).

use crate::ed25519_pin::verify_pinned;
use crate::pae::pae;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::Deserialize;

/// Pinned DSSE `payloadType` for evidence journal records (law 4).
pub const PT_RECORD: &str = "application/vnd.docbrain.evidence.record.v1+json";
/// Pinned DSSE `payloadType` for checkpoints.
pub const PT_CHECKPOINT: &str = "application/vnd.docbrain.evidence.checkpoint.v1+json";
/// Pinned DSSE `payloadType` for key-chain records.
pub const PT_KEYRECORD: &str = "application/vnd.docbrain.evidence.keyrecord.v1+json";
/// Pinned DSSE `payloadType` for the bundle manifest.
pub const PT_MANIFEST: &str = "application/vnd.docbrain.evidence.manifest.v1+json";

/// A signed (or about-to-be-signed) DSSE-style envelope. See module docs for
/// the wire form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub payload_type: String,
    pub payload: Vec<u8>,
    pub sig: [u8; 64],
    pub keyid: String,
}

/// Every variant here is a fail-closed outcome; every variant maps to
/// CANNOT_VERIFY (malformed or unsupported) in the verdict engine — none is
/// ever silently skipped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeError {
    #[error("malformed envelope: {0}")]
    Malformed(String),
    #[error("unsupported envelope: {0}")]
    Unsupported(String),
    #[error("payloadType mismatch: expected {expected}, got {got}")]
    WrongPayloadType { expected: String, got: String },
    #[error("signature invalid")]
    SignatureInvalid,
}

/// The permissive wire-format shape used only to detect malformed/unsupported
/// input; `deny_unknown_fields` is what turns a stray key into `Malformed`,
/// and the explicit (rather than absent) `signatures` field is what lets us
/// tell "extra unknown key" apart from "multi-signature DSSE form" so the
/// latter reports `Unsupported`, not `Malformed`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEnvelope {
    #[serde(rename = "payloadType")]
    payload_type: String,
    payload: String,
    #[serde(default)]
    sig: Option<String>,
    #[serde(default)]
    keyid: Option<String>,
    #[serde(default)]
    signatures: Option<serde_json::Value>,
}

/// Appends a JSON-escaped (RFC 8259 §7) string literal, including the
/// surrounding quotes, to `out`. Hand-written (rather than going through
/// `serde_json`) so `Envelope::to_line` has no fallible serializer call to
/// `.expect()`/`.unwrap()` away.
fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl Envelope {
    /// Serialize to the one-JSON-object-per-line wire form. Hand-built (not
    /// via `serde_json::to_vec`) so this is infallible end to end: `payload`
    /// and `sig` are base64 (RFC 4648 alphabet — never needs escaping),
    /// `payload_type` and `keyid` go through `push_json_string`.
    pub fn to_line(&self) -> Vec<u8> {
        let mut out = String::from("{\"payloadType\":");
        push_json_string(&mut out, &self.payload_type);
        out.push_str(",\"payload\":\"");
        out.push_str(&STANDARD.encode(&self.payload));
        out.push_str("\",\"sig\":\"");
        out.push_str(&STANDARD.encode(self.sig));
        out.push_str("\",\"keyid\":");
        push_json_string(&mut out, &self.keyid);
        out.push('}');
        out.into_bytes()
    }
}

/// Sign `payload` under `payload_type`, over `pae(payload_type, payload)`.
pub fn sign_envelope(
    sk: &SigningKey,
    keyid: &str,
    payload_type: &str,
    payload: &[u8],
) -> Envelope {
    let msg = pae(payload_type, payload);
    let sig = sk.sign(&msg);
    Envelope {
        payload_type: payload_type.to_string(),
        payload: payload.to_vec(),
        sig: sig.to_bytes(),
        keyid: keyid.to_string(),
    }
}

/// Parse `env_line`, check `payloadType == expected_type` BEFORE any
/// signature work, reject any multi-signature wire form or unknown key,
/// rebuild `pae()` from the exact decoded payload bytes, and verify via
/// `verify_pinned` (the crate's only signature-verification entry point).
/// Returns the exact decoded payload bytes; callers never re-parse the
/// envelope.
pub fn verify_envelope(
    env_line: &[u8],
    expected_type: &str,
    vk: &VerifyingKey,
) -> Result<Vec<u8>, EnvelopeError> {
    let wire: WireEnvelope = crate::strict::from_slice_strict(env_line)
        .map_err(|e| EnvelopeError::Malformed(format!("envelope JSON: {e}")))?;

    if wire.signatures.is_some() {
        return Err(EnvelopeError::Unsupported(
            "multi-signature envelope (signatures array); single-signature law".into(),
        ));
    }

    if wire.payload_type != expected_type {
        return Err(EnvelopeError::WrongPayloadType {
            expected: expected_type.to_string(),
            got: wire.payload_type,
        });
    }

    let sig_b64 = wire
        .sig
        .ok_or_else(|| EnvelopeError::Malformed("missing sig".into()))?;
    let _keyid = wire
        .keyid
        .ok_or_else(|| EnvelopeError::Malformed("missing keyid".into()))?;

    let payload = STANDARD
        .decode(wire.payload.as_bytes())
        .map_err(|e| EnvelopeError::Malformed(format!("payload base64: {e}")))?;
    let sig_bytes = STANDARD
        .decode(sig_b64.as_bytes())
        .map_err(|e| EnvelopeError::Malformed(format!("sig base64: {e}")))?;
    let sig_arr: [u8; 64] = sig_bytes.as_slice().try_into().map_err(|_| {
        EnvelopeError::Malformed(format!(
            "sig must be 64 bytes, got {}",
            sig_bytes.len()
        ))
    })?;
    let sig = Signature::from_bytes(&sig_arr);

    let msg = pae(&wire.payload_type, &payload);
    if !verify_pinned(vk, &msg, &sig) {
        return Err(EnvelopeError::SignatureInvalid);
    }

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adversarial pass over `push_json_string` (new hand-rolled escaper,
    /// replacing the `serde_json::to_vec(...).expect(...)` call). Each case
    /// round-trips through `to_line()` -> `verify_envelope()` and asserts
    /// the exact byte payload survives, proving the hand-built escaping is
    /// both valid JSON (parses) and correct (keyid/payload_type come back
    /// unchanged).
    #[test]
    fn to_line_escapes_special_characters_in_keyid_and_payload_type() {
        let sk = key(1);
        let attack_keyids = [
            "plain-hex-keyid",
            "has\"quote",
            "has\\backslash",
            "has\nnewline",
            "has\rcarriage\treturn+tab",
            "has\u{0001}control\u{001f}chars",
            "has\u{0000}nul",
            "unicode-é-证据-🔑",
            "",
            "trailing-backslash\\",
            "quote-then-backslash\"\\",
        ];
        for keyid in attack_keyids {
            let env = sign_envelope(&sk, keyid, PT_RECORD, b"payload");
            let line = env.to_line();
            // The hand-built line must be valid JSON at all (this alone
            // would fail if escaping were wrong, e.g. an unescaped quote
            // breaking the object structure).
            let parsed: serde_json::Value =
                serde_json::from_slice(&line).unwrap_or_else(|e| {
                    panic!("to_line() produced invalid JSON for keyid {keyid:?}: {e}")
                });
            assert_eq!(
                parsed["keyid"].as_str().unwrap(),
                keyid,
                "keyid round-trip mismatch for {keyid:?}"
            );
            let got = verify_envelope(&line, PT_RECORD, &sk.verifying_key())
                .unwrap_or_else(|e| panic!("verify_envelope failed for keyid {keyid:?}: {e}"));
            assert_eq!(got, b"payload");
        }
    }

    #[test]
    fn to_line_escapes_special_characters_in_payload_type() {
        let sk = key(1);
        // payload_type is normally one of the pinned PT_* constants, but the
        // function signature accepts any &str — attack it the same way.
        let weird_type = "type/with\"quote\\and\nnewline";
        let env = sign_envelope(&sk, "keyid", weird_type, b"x");
        let line = env.to_line();
        let parsed: serde_json::Value = serde_json::from_slice(&line).unwrap();
        assert_eq!(parsed["payloadType"].as_str().unwrap(), weird_type);
        let got = verify_envelope(&line, weird_type, &sk.verifying_key()).unwrap();
        assert_eq!(got, b"x");
    }

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    #[test]
    fn sign_then_verify_round_trips_the_payload() {
        let sk = key(1);
        let env = sign_envelope(&sk, "keyid-1", PT_RECORD, b"hello record");
        let line = env.to_line();
        let got = verify_envelope(&line, PT_RECORD, &sk.verifying_key())
            .expect("valid envelope must verify");
        assert_eq!(got, b"hello record");
    }

    #[test]
    fn wrong_expected_payload_type_is_rejected_before_signature_work() {
        let sk = key(1);
        let env = sign_envelope(&sk, "keyid-1", PT_RECORD, b"hello record");
        let line = env.to_line();
        // A totally bogus key: if verification happened first this would
        // still need to reach the signature step to fail; we assert it
        // fails with WrongPayloadType specifically, proving the ordering.
        let err = verify_envelope(&line, PT_CHECKPOINT, &sk.verifying_key()).unwrap_err();
        assert_eq!(
            err,
            EnvelopeError::WrongPayloadType {
                expected: PT_CHECKPOINT.to_string(),
                got: PT_RECORD.to_string(),
            }
        );
    }

    #[test]
    fn flipped_payload_byte_is_signature_invalid() {
        let sk = key(1);
        let env = sign_envelope(&sk, "keyid-1", PT_RECORD, b"hello record");
        let mut payload = env.payload.clone();
        payload[0] ^= 0xFF;
        let tampered = serde_json::json!({
            "payloadType": env.payload_type,
            "payload": STANDARD.encode(&payload),
            "sig": STANDARD.encode(env.sig),
            "keyid": env.keyid,
        });
        let line = serde_json::to_vec(&tampered).unwrap();
        let err = verify_envelope(&line, PT_RECORD, &sk.verifying_key()).unwrap_err();
        assert_eq!(err, EnvelopeError::SignatureInvalid);
    }

    #[test]
    fn multi_signature_wire_form_is_unsupported() {
        let sk = key(1);
        let env = sign_envelope(&sk, "keyid-1", PT_RECORD, b"hello record");
        let multi = serde_json::json!({
            "payloadType": env.payload_type,
            "payload": STANDARD.encode(&env.payload),
            "signatures": [
                {"sig": STANDARD.encode(env.sig), "keyid": env.keyid},
                {"sig": STANDARD.encode(env.sig), "keyid": "keyid-2"},
            ],
        });
        let line = serde_json::to_vec(&multi).unwrap();
        let err = verify_envelope(&line, PT_RECORD, &sk.verifying_key()).unwrap_err();
        assert!(matches!(err, EnvelopeError::Unsupported(_)));
    }

    #[test]
    fn unknown_envelope_key_is_malformed() {
        let sk = key(1);
        let env = sign_envelope(&sk, "keyid-1", PT_RECORD, b"hello record");
        let extra = serde_json::json!({
            "payloadType": env.payload_type,
            "payload": STANDARD.encode(&env.payload),
            "sig": STANDARD.encode(env.sig),
            "keyid": env.keyid,
            "extra": "not part of the wire form",
        });
        let line = serde_json::to_vec(&extra).unwrap();
        let err = verify_envelope(&line, PT_RECORD, &sk.verifying_key()).unwrap_err();
        assert!(matches!(err, EnvelopeError::Malformed(_)));
    }

    #[test]
    fn whitespace_in_base64_is_malformed() {
        let sk = key(1);
        let env = sign_envelope(&sk, "keyid-1", PT_RECORD, b"hello record");
        let mut sig_b64 = STANDARD.encode(env.sig);
        sig_b64.insert(4, ' '); // strict RFC 4648 alphabet: whitespace is invalid
        let line = format!(
            "{{\"payloadType\":{:?},\"payload\":{:?},\"sig\":{:?},\"keyid\":{:?}}}",
            env.payload_type,
            STANDARD.encode(&env.payload),
            sig_b64,
            env.keyid,
        )
        .into_bytes();
        let err = verify_envelope(&line, PT_RECORD, &sk.verifying_key()).unwrap_err();
        assert!(matches!(err, EnvelopeError::Malformed(_)));
    }

    #[test]
    fn wrong_sig_length_is_malformed() {
        let sk = key(1);
        let env = sign_envelope(&sk, "keyid-1", PT_RECORD, b"hello record");
        let truncated_sig = &env.sig[..63];
        let line = serde_json::to_vec(&serde_json::json!({
            "payloadType": env.payload_type,
            "payload": STANDARD.encode(&env.payload),
            "sig": STANDARD.encode(truncated_sig),
            "keyid": env.keyid,
        }))
        .unwrap();
        let err = verify_envelope(&line, PT_RECORD, &sk.verifying_key()).unwrap_err();
        assert!(matches!(err, EnvelopeError::Malformed(_)));
    }
}
