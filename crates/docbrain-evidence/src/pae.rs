// SPDX-License-Identifier: MIT
//! DSSE Pre-Authentication Encoding (v1.0.2). The signed bytes are
//! `PAE(payloadType, payload)`; the payload is treated as opaque bytes and is
//! NEVER re-encoded anywhere in this crate (spec law 1).

/// PAE per https://github.com/secure-systems-lab/dsse/blob/v1.0.2/protocol.md
pub fn pae(payload_type: &str, body: &[u8]) -> Vec<u8> {
    let t = payload_type.as_bytes();
    let mut out = Vec::with_capacity(16 + t.len() + body.len());
    out.extend_from_slice(b"DSSEv1 ");
    out.extend_from_slice(t.len().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(t);
    out.push(b' ');
    out.extend_from_slice(body.len().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pae_matches_dsse_spec_example() {
        // DSSE v1.0.2 protocol.md example: PAE("http://example.com/HelloWorld", "hello world")
        let got = pae("http://example.com/HelloWorld", b"hello world");
        assert_eq!(
            got,
            b"DSSEv1 29 http://example.com/HelloWorld 11 hello world".to_vec()
        );
    }

    #[test]
    fn pae_empty_body_and_type() {
        assert_eq!(pae("", b""), b"DSSEv1 0  0 ".to_vec());
    }

    #[test]
    fn pae_len_is_bytes_not_chars() {
        // multibyte type: LEN must count UTF-8 bytes
        assert_eq!(pae("é", b"x"), b"DSSEv1 2 \xc3\xa9 1 x".to_vec());
    }

    #[test]
    fn pae_body_is_opaque_bytes() {
        let body = [0u8, 255, 10, 13, 34]; // NUL, 0xFF, LF, CR, quote — all opaque
        let got = pae("t", &body);
        assert!(got.ends_with(&[b'5', b' ', 0, 255, 10, 13, 34]));
    }
}
