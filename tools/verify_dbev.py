# SPDX-License-Identifier: MIT
"""DocBrain `.dbev` evidence-bundle reference verifier.

This is the OFFLINE, open-source, stdlib-only verifier an auditor runs on a bare
`python3` with NO DocBrain install, NO server, NO network, and NO third-party
packages. Its whole value is that it returns the IDENTICAL verdict (and dominant
finding code) as the normative Rust implementation in
`crates/docbrain-evidence/src/` on every input — so evidence can be trusted
WITHOUT trusting us.

It is a faithful, byte-level re-implementation of the Rust crate:
  * ed25519 `verify_strict` (RFC 8032 math + dalek's strict checks), pinned by
    the frozen cross-language vector suite `vectors/ed25519_pin.json`.
  * DSSE Pre-Authentication Encoding (PAE), domain-separated SHA-256 hashing.
  * A minimal central-directory-only ZIP reader enforcing the `.dbev`
    restricted profile (STORE-only, no zip64, whitelisted paths, local/CD
    agreement) — deliberately NOT Python's `zipfile`, which would accept things
    the profile rejects and rewrite bytes.
  * The key chain, checkpoint chain, manifest, record chain, and the full
    26-row verdict taxonomy with the one-success-exit pipeline.

Usage:
    python3 tools/verify_dbev.py <bundle.dbev> [--json]
    python3 tools/verify_dbev.py --self-test [<vectors.json>]

Exit codes: 0 VALID, 1 TAMPERED, 2 CANNOT_VERIFY (the verdict), 3 a CLI-level
error (file missing/unreadable) — which is NOT a verdict. `--self-test` exits 0
if every ed25519 vector matches, 1 otherwise.

Standard library ONLY (hashlib, json, sys, os, struct-free byte math). No pip.
"""

import hashlib
import json
import math
import os
import sys
from datetime import datetime, timezone

# ============================================================================
# ed25519 verify_strict (mirrors crates/docbrain-evidence/src/ed25519_pin.rs,
# pinned to ed25519-dalek=2.2.0/verify_strict by vectors/ed25519_pin.json).
# ============================================================================

_P = 2**255 - 19
_L = 2**252 + 27742317777372353535851937790883648493
_D = (-121665 * pow(121666, _P - 2, _P)) % _P
_SQRT_M1 = pow(2, (_P - 1) // 4, _P)
_IDENTITY_ENC = (1).to_bytes(32, "little")


def _modp_inv(x):
    return pow(x, _P - 2, _P)


def _recover_x(y, sign):
    y %= _P
    v = (_D * y * y + 1) % _P
    if v == 0:
        return None
    x2 = ((y * y - 1) * _modp_inv(v)) % _P
    if x2 == 0:
        # dalek does not reject signed-zero; conditional negate leaves 0.
        return 0
    x = pow(x2, (_P + 3) // 8, _P)
    if (x * x - x2) % _P != 0:
        x = (x * _SQRT_M1) % _P
    if (x * x - x2) % _P != 0:
        return None
    if (x & 1) != sign:
        x = (_P - x) % _P
    return x


# Extended homogeneous coordinates (X, Y, Z, T); x = X/Z, y = Y/Z, xy = T/Z.
def _point_add(pt, q):
    a = ((pt[1] - pt[0]) * (q[1] - q[0])) % _P
    b = ((pt[1] + pt[0]) * (q[1] + q[0])) % _P
    c = (2 * pt[3] * q[3] * _D) % _P
    dd = (2 * pt[2] * q[2]) % _P
    e = b - a
    f = dd - c
    g = dd + c
    h = b + a
    return (e * f % _P, g * h % _P, f * g % _P, e * h % _P)


def _point_mul(s, pt):
    q = (0, 1, 1, 0)  # neutral element
    while s > 0:
        if s & 1:
            q = _point_add(q, pt)
        pt = _point_add(pt, pt)
        s >>= 1
    return q


def _point_neg(pt):
    return ((-pt[0]) % _P, pt[1], pt[2], (-pt[3]) % _P)


def _point_compress(pt):
    zinv = _modp_inv(pt[2])
    x = (pt[0] * zinv) % _P
    y = (pt[1] * zinv) % _P
    return int(y | ((x & 1) << 255)).to_bytes(32, "little")


def _point_decompress(s):
    if len(s) != 32:
        return None
    y = int.from_bytes(s, "little")
    sign = (y >> 255) & 1
    y &= (1 << 255) - 1
    x = _recover_x(y, sign)
    if x is None:
        return None
    return (x, y, 1, x * y % _P)


def _is_small_order(pt):
    return _point_compress(_point_mul(8, pt)) == _IDENTITY_ENC


_By = (4 * _modp_inv(5)) % _P
_Bx = _recover_x(_By, 0)
_B = (_Bx, _By, 1, _Bx * _By % _P)


def verify_pinned(vk_bytes, msg, sig_bytes):
    """ed25519-dalek 2.2.0 `verify_strict` semantics, exactly.

    Rejects: non-64-byte signatures (checked BEFORE any split), non-canonical
    scalars (s >= L), small-order A or R, and any signature where the
    cofactorless equation R == [s]B - [k]A does not hold (this is where the
    deliberate ZIP215-divergence vectors — small-order/low-order-residue R —
    are caught: a cofactored verifier would wrongly accept them).
    """
    if len(sig_bytes) != 64:
        return False
    a_pt = _point_decompress(vk_bytes)
    if a_pt is None:
        return False
    r_bytes = sig_bytes[0:32]
    s = int.from_bytes(sig_bytes[32:64], "little")
    if s >= _L:
        return False
    r_pt = _point_decompress(r_bytes)
    if r_pt is None:
        return False
    if _is_small_order(r_pt) or _is_small_order(a_pt):
        return False
    k = int.from_bytes(hashlib.sha512(r_bytes + vk_bytes + msg).digest(), "little") % _L
    r_check = _point_add(_point_mul(s, _B), _point_neg(_point_mul(k, a_pt)))
    return _point_compress(r_check) == r_bytes


# ============================================================================
# Strict base64 (mirrors base64::engine::general_purpose::STANDARD in
# base64 0.22: canonical padding REQUIRED, trailing bits MUST be zero, only the
# standard alphabet, no whitespace). Python's base64.b64decode differs (it
# accepts non-zero trailing bits), so we decode by hand to guarantee parity.
# ============================================================================

_B64_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
_B64_LOOKUP = {ord(c): i for i, c in enumerate(_B64_ALPHABET)}


class Base64Error(Exception):
    pass


def b64decode_strict(data):
    """STANDARD (padded, canonical) base64 decode. Raises Base64Error on any
    deviation Rust's STANDARD engine would reject."""
    if isinstance(data, str):
        try:
            data = data.encode("ascii")
        except UnicodeEncodeError:
            raise Base64Error("non-ascii base64")
    n = len(data)
    if n % 4 != 0:
        raise Base64Error("length not a multiple of 4 (non-canonical padding)")
    if n == 0:
        return b""
    pad = 0
    if data[-1] == 0x3D:  # '='
        pad = 1
        if data[-2] == 0x3D:
            pad = 2
    body = data[: n - pad]
    if 0x3D in body:
        raise Base64Error("padding byte not at the end")
    vals = []
    for ch in body:
        v = _B64_LOOKUP.get(ch)
        if v is None:
            raise Base64Error("invalid base64 character")
        vals.append(v)
    m = len(vals)
    rem = m % 4
    if pad == 0:
        if rem != 0:
            raise Base64Error("non-canonical padding")
    elif pad == 1:
        if rem != 3:
            raise Base64Error("non-canonical padding")
    else:  # pad == 2
        if rem != 2:
            raise Base64Error("non-canonical padding")
    out = bytearray()
    i = 0
    full = m // 4
    for _ in range(full):
        b0, b1, b2, b3 = vals[i], vals[i + 1], vals[i + 2], vals[i + 3]
        out.append((b0 << 2) | (b1 >> 4))
        out.append(((b1 & 0x0F) << 4) | (b2 >> 2))
        out.append(((b2 & 0x03) << 6) | b3)
        i += 4
    if rem == 2:
        b0, b1 = vals[i], vals[i + 1]
        out.append((b0 << 2) | (b1 >> 4))
        if (b1 & 0x0F) != 0:
            raise Base64Error("non-zero trailing bits")
    elif rem == 3:
        b0, b1, b2 = vals[i], vals[i + 1], vals[i + 2]
        out.append((b0 << 2) | (b1 >> 4))
        out.append(((b1 & 0x0F) << 4) | (b2 >> 2))
        if (b2 & 0x03) != 0:
            raise Base64Error("non-zero trailing bits")
    return bytes(out)


# ============================================================================
# Strict hex (mirrors the `hex` crate: even length, [0-9a-fA-F] only, no
# whitespace) + 32-byte helper.
# ============================================================================

_HEX_DIGITS = set(b"0123456789abcdefABCDEF")


class HexError(Exception):
    pass


def hex_decode(s):
    if isinstance(s, str):
        try:
            b = s.encode("ascii")
        except UnicodeEncodeError:
            raise HexError("non-ascii hex")
    else:
        b = s
    if len(b) % 2 != 0:
        raise HexError("odd hex length")
    for ch in b:
        if ch not in _HEX_DIGITS:
            raise HexError("invalid hex character")
    return bytes.fromhex(b.decode("ascii"))


def hex_to_32(s):
    b = hex_decode(s)
    if len(b) != 32:
        raise HexError("expected 32 bytes, got %d" % len(b))
    return b


# ============================================================================
# PAE (mirrors pae.rs) + domain-separated hashing (mirrors hash.rs).
# ============================================================================

def pae(payload_type, body):
    t = payload_type.encode("utf-8")
    out = bytearray()
    out += b"DSSEv1 "
    out += str(len(t)).encode("ascii")
    out += b" "
    out += t
    out += b" "
    out += str(len(body)).encode("ascii")
    out += b" "
    out += body
    return bytes(out)


_LEAF_PREFIX = b"\x00"
_HEAD_PREFIX = b"\x01"
_CONTENT_PREFIX = b"\x02"
GENESIS_PREV = b"\x00" * 32


def leaf_hash(envelope_bytes):
    return hashlib.sha256(_LEAF_PREFIX + envelope_bytes).digest()


def head_hash(prev_head, leaf):
    return hashlib.sha256(_HEAD_PREFIX + prev_head + leaf).digest()


def content_hash(salt, content):
    return hashlib.sha256(_CONTENT_PREFIX + salt + content).digest()


# ============================================================================
# Typed JSON helpers. serde is strict about types; Python's json is not, so we
# enforce the same rules (u64 range, string types, closed schemas) at the
# boundary and raise a plain ValueError, which each caller maps to its own
# malformed variant — matching how each Rust module maps a serde error.
# ============================================================================

class JsonSchemaError(ValueError):
    pass


# serde_json validates UTF-8 LAZILY: only strings it actually deserializes into
# a Rust `String`/`Value`, and the object KEYS it reads, are UTF-8-checked;
# strings inside SKIPPED (ignored) fields are byte-scanned without validation.
# Python's json.loads validates the whole buffer eagerly, which diverges on a
# corrupted payload whose bad bytes land in a peek-skipped region (Rust reaches
# the signature check and reports TAMPERED; naive Python bails as malformed).
#
# To mirror serde exactly: decode with surrogateescape (never fails — invalid
# bytes become lone surrogates U+DC80..U+DCFF, which are impossible in real
# UTF-8, so they precisely mark the bad bytes) then validate UTF-8 only at the
# exact spots serde would (via `_ck_str`/`_ck_keys`/`_ck_value`). A byte that
# lands in a STRUCTURAL position becomes a surrogate char json.loads rejects as
# a syntax error, matching serde's invalid-token error.

def _reject_duplicate_keys(pairs):
    """json.loads object_pairs_hook: rebuild each JSON object from its ordered
    (key, value) pairs, raising on the FIRST repeated key. serde's derived
    Deserialize errors on a duplicate of a KNOWN struct field ("duplicate field
    ...") — for WireEnvelope and the tolerant peek structs alike, even without
    deny_unknown_fields — whereas plain json.loads SILENTLY keeps the last value
    for a duplicate key and never errors. That gap let a duplicated `keyid` in
    the outer envelope (the DSSE signature covers only pae(payloadType, payload),
    not the envelope's own keyid/sig) pass this verifier VALID while Rust rejected
    the bundle as malformed — a false-VALID in the public auditor.

    Rejecting ANY duplicate key here, at the single chokepoint every derived-
    struct parse routes through (outer envelope, keyrecord, record, checkpoint,
    manifest payload), restores parity without over-rejecting any reachable
    input: the manifest's outer envelope — the ONLY parse on bytes not first
    gated by a member-hash or a signature — is strict (deny_unknown), so serde
    already rejects unknown keys there identically; every tolerant peek struct
    sits on member bytes the manifest SHA-256-locks, so a duplicate key there is
    caught as a member-hash mismatch (row 12) by BOTH verifiers before json_parse
    is ever reached. object_pairs_hook fires for every object at every nesting
    level in every member, which is exactly the coverage wanted."""
    obj = {}
    for key, value in pairs:
        if key in obj:
            raise JsonSchemaError("duplicate key %r" % (key,))
        obj[key] = value
    return obj


# serde_json is STRICTER than Python's json.loads in two STRUCTURAL ways that
# json.loads is silently lenient about. Both are enforced by serde_json's PARSER
# for every byte it tokenizes — read fields AND ignored fields alike (unlike
# UTF-8, which serde validates only for strings it materializes; that stays a
# per-field check via `_ck_str`/`_ck_value`/`_ck_keys`, mirroring serde's
# laziness). So both are safe to enforce here at the single json.loads chokepoint
# every JSON member routes through: rejecting them everywhere in Python matches
# Rust rejecting them everywhere, with no risk of over-rejecting a member Rust
# accepts (measured empirically against `docbrain-verify`, both directions).

# (1) Non-finite constants. json.loads accepts the barewords `NaN`, `Infinity`
# and `-Infinity` (via parse_constant); serde_json rejects them (RFC 8259 has no
# non-finite numbers). Measured: an anchor whose ignored field is `NaN` is
# VALID | anchor-invalid | row 18 in Rust (the NoDupKeys deserialize_any pre-scan
# errors on the token) but was VALID | valid | row 1 in Python — a false-VALID.
def _reject_json_constant(token):
    raise JsonSchemaError("non-finite JSON constant %r (not valid JSON)" % (token,))


# (2) Recursion depth. serde_json caps container nesting at RECURSION_LIMIT = 128:
# empirically it ACCEPTS a value nested 127 containers deep and REJECTS the 128th
# simultaneously-open `[`/`{`, identically for `serde_json::Value` and the anchor
# NoDupKeys pre-scan (measured directly against serde_json 1.x). Python's
# json.loads allows ~1000, so a member nested 128..~1000 deep parses here yet is
# rejected by Rust — a divergence (measured: an anchor whose ignored field nests
# 127 arrays — 128 open brackets counting the anchor object itself — is
# VALID | anchor-invalid | row 18 in Rust, VALID | valid | row 1 in Python).
# Match the boundary EXACTLY with a pre-scan of the raw text that raises the
# instant the open-bracket count reaches 128 (brackets inside strings excluded).
# An off-by-one here would itself be a divergence, so the accept/reject edge is
# pinned by frozen fuzz vectors at depth 127 (both accept) and 128 (both reject).
_SERDE_RECURSION_LIMIT = 128


def _check_json_depth(text):
    depth = 0
    in_string = False
    escaped = False
    for c in text:
        if in_string:
            if escaped:
                escaped = False
            elif c == "\\":
                escaped = True
            elif c == '"':
                in_string = False
            continue
        if c == '"':
            in_string = True
        elif c == "[" or c == "{":
            depth += 1
            if depth >= _SERDE_RECURSION_LIMIT:
                raise JsonSchemaError(
                    "JSON nesting reaches serde_json's recursion limit (%d)"
                    % _SERDE_RECURSION_LIMIT
                )
        elif c == "]" or c == "}":
            if depth > 0:
                depth -= 1


# (3) Out-of-f64-range NUMBERS. `serde_json::Value` / the Rust `NoDupKeys`
# strict pre-pass EAGERLY materialize every number (ignored fields included), so
# a value outside f64 range errors "number out of range". json.loads is LENIENT:
# `1e400` becomes `inf` WITHOUT firing `parse_constant` (that hook only catches
# the NaN/Infinity/-Infinity BAREWORDS, never numeric TOKENS), and a 345-digit
# integer becomes a Python bigint. This is now enforced GLOBALLY at the json_parse
# chokepoint (parse_int/parse_float hooks below) rather than anchor-locally,
# because the Rust side runs the same strict pre-pass on EVERY JSON member (not
# only anchors), so a giant/overflow number in ANY member — a record `body`, an
# envelope `signatures`, an ignored field — now rejects on both sides. (Pre-pass
# uniformity is what retires the old anchor-local restriction: when Rust's
# ignored-field parse was LAZY, a global reject here over-rejected; now that
# Rust's pre-pass is eager everywhere, matching it requires the global reject.)
#
# The reject boundary is NOT "magnitude > f64::MAX": serde_json 1.0.149 rebuilds a
# number from a u64 significand (the leading digits that fit u64) times 10^exp and
# errors iff THAT product overflows to a non-finite f64 — so it rejects the
# integer floor(f64::MAX) itself and high-precision mantissas just under f64::MAX
# that correct rounding (Python's float()) would keep finite. `_serde_number_out_of_range`
# replicates serde's reconstruction exactly; it was differentially tested against
# `serde_json::from_str::<Value>` over 32k+ tokens (the adversarial ULP band at
# f64::MAX / 2^1024 included) with zero disagreements. Frozen fuzz vectors pin the
# edge (see diff_fuzz.rs *-number-* / *-e400 / *-f64max-ulp).
_U64_MAX = (1 << 64) - 1


def _serde_number_out_of_range(token):
    """True iff serde_json 1.0.149's `deserialize_any` number path (== the strict
    pre-pass `NoDupKeys`/`Value` eager parse) would reject `token` as "number out
    of range". `token` is a syntactically valid JSON number (json.loads has already
    accepted its grammar before calling this). Mirrors serde's significand-times-
    10^exp reconstruction and its overflow check; NOT correct IEEE rounding."""
    s = token
    i = 0
    n = len(s)
    if i < n and s[i] == "-":  # JSON forbids a leading '+' on the number itself
        i += 1
    sig = 0            # leading significant digits captured into a u64
    exp = 0            # base-10 exponent for digits past the u64 significand
    overflowed = False
    has_dot = False
    has_e = False
    while i < n and s[i].isdigit():
        d = ord(s[i]) - 48
        if not overflowed and sig <= (_U64_MAX - d) // 10:
            sig = sig * 10 + d
        else:
            overflowed = True
            exp += 1
        i += 1
    if i < n and s[i] == ".":
        has_dot = True
        i += 1
        while i < n and s[i].isdigit():
            d = ord(s[i]) - 48
            if not overflowed and sig <= (_U64_MAX - d) // 10:
                sig = sig * 10 + d
                exp -= 1
            # once the significand is full, further fraction digits are dropped
            i += 1
    if i < n and (s[i] == "e" or s[i] == "E"):
        has_e = True
        i += 1
        esign = 1
        if i < n and (s[i] == "-" or s[i] == "+"):
            if s[i] == "-":
                esign = -1
            i += 1
        ev = 0
        while i < n and s[i].isdigit():
            ev = ev * 10 + (ord(s[i]) - 48)
            i += 1
        exp += esign * ev
    # A pure integer that fits u64 fits serde's i64/u64 path — always accepted.
    if not has_dot and not has_e and not overflowed:
        return False
    # 0 * 10^anything is 0.0 (finite): serde accepts `0e400`, `0.0e999`, etc.
    if sig == 0:
        return False
    try:
        val = float(sig) * (10.0 ** exp)
    except OverflowError:  # 10.0 ** exp with a large positive exp
        return True
    return not math.isfinite(val)


def _strict_parse_int(token):
    """json.loads parse_int hook: reject an integer token serde_json's eager
    number path would reject as out of range (a >u64 magnitude that overflows
    f64), else return the int. Matches the Rust strict pre-pass on every member."""
    if _serde_number_out_of_range(token):
        raise JsonSchemaError("number %r is outside serde_json's f64 range" % token)
    return int(token)


def _strict_parse_float(token):
    """json.loads parse_float hook: reject a float token serde_json's eager
    number path would reject as out of range (overflows to non-finite f64), else
    return the float. Matches the Rust strict pre-pass on every member."""
    if _serde_number_out_of_range(token):
        raise JsonSchemaError("number %r is outside serde_json's f64 range" % token)
    return float(token)


# (4) Lone/unpaired UTF-16 SURROGATE escapes. serde_json's strings deserialize
# into Rust `String`s, which hold only valid Unicode SCALAR values, so its eager
# `from_slice_strict` pre-pass REJECTS a `\uXXXX` escape that forms a lone
# surrogate (U+D800..U+DFFF) — "unexpected end of hex escape". Python's json.loads
# is LENIENT: it ACCEPTS `\uD800` and returns a `str` holding the lone surrogate
# code point. So a decode of the input BYTES (axis 1 above) is NOT sufficient —
# the bytes `\uD800` are plain ASCII and decode fine; the gap is that the DECODED
# string is not a valid scalar sequence. Measured: a CLOSURE erasure record whose
# `class` is `"\uD800"` is TAMPERED | tampered-scope | row 10 in Rust (the record
# payload is rejected malformed at row 22, so the manifest's withheld_erased count
# no longer matches the surviving withheld records) but was VALID | withheld-erased
# | row 13 in Python — a CRITICAL false-VALID in the public auditor. A valid
# surrogate PAIR (e.g. an emoji `😀`) is COMBINED by json.loads into one
# scalar (U+1F600) and MUST stay accepted; only LONE/unpaired surrogates fail. The
# exact serde test: a `str` is a valid scalar sequence iff `s.encode("utf-8")`
# succeeds (it raises UnicodeEncodeError on a lone surrogate, succeeds on a
# combined pair). Walk EVERY string — keys AND values, at every nesting level —
# because serde validates each string it materializes; run at the json_parse
# chokepoint so it covers every member uniformly (not anchor-locally).
def _reject_lone_surrogates(value):
    if isinstance(value, str):
        try:
            value.encode("utf-8")
        except UnicodeEncodeError:
            raise JsonSchemaError(
                "string contains a lone UTF-16 surrogate (U+D800..U+DFFF is not a "
                "valid Unicode scalar value; serde_json rejects the escape)"
            )
    elif isinstance(value, dict):
        for k, v in value.items():
            _reject_lone_surrogates(k)
            _reject_lone_surrogates(v)
    elif isinstance(value, list):
        for item in value:
            _reject_lone_surrogates(item)


def json_parse(raw_bytes):
    """serde_json::from_slice equivalent, enforcing the uniform strict-JSON
    profile on EVERY parsed member — the Python twin of the Rust
    `strict::from_slice_strict` pre-pass. Parses the WHOLE input as one JSON
    value (trailing bytes or malformed JSON raise JsonSchemaError) and rejects,
    on the SAME bytes both verifiers see:
      * invalid UTF-8 (strict decode — serde_json's eager `deserialize_any`
        materializes every string, ignored fields included, and rejects a bad
        byte; the old lazy surrogateescape path is gone now that Rust's pre-pass
        is eager on every member);
      * lone/unpaired UTF-16 surrogate escapes (`_reject_lone_surrogates`) — a
        `\\uD800`-style escape json.loads accepts but serde_json rejects because
        the decoded string would not be a valid Unicode scalar sequence; valid
        surrogate PAIRS (emoji) are combined by json.loads and stay accepted;
      * duplicate keys at any level (`_reject_duplicate_keys`);
      * NaN/Infinity/-Infinity barewords (`_reject_json_constant`);
      * over-limit nesting at serde_json's 128 cap (`_check_json_depth`);
      * out-of-f64-range numbers (`_strict_parse_int`/`_strict_parse_float`).
    Every rejection maps to JsonSchemaError, which each caller routes to its own
    malformed verdict — matching serde_json rejecting the same member."""
    if isinstance(raw_bytes, (bytes, bytearray)):
        # Strict UTF-8: the whole member must decode. serde_json's eager
        # pre-pass rejects invalid UTF-8 anywhere in the member (string values
        # and structural bytes alike), so Python must too — no surrogateescape.
        try:
            text = bytes(raw_bytes).decode("utf-8")
        except UnicodeDecodeError as e:
            raise JsonSchemaError("invalid UTF-8: %s" % e)
    else:
        text = raw_bytes
    # serde_json rejects over-limit nesting BEFORE it would ever finish parsing,
    # so this pre-scan (which also can't be defeated by json.loads' own, higher
    # recursion cap) runs first, raising at the identical 128-container boundary.
    _check_json_depth(text)
    try:
        parsed = json.loads(
            text,
            object_pairs_hook=_reject_duplicate_keys,
            parse_constant=_reject_json_constant,
            parse_int=_strict_parse_int,
            parse_float=_strict_parse_float,
        )
        # serde_json's strings are Rust `String`s (valid Unicode scalar values
        # only): reject any decoded string carrying a lone UTF-16 surrogate from a
        # `\uXXXX` escape — json.loads accepts it, serde_json rejects it. Run after
        # a successful parse so json.loads has already combined valid surrogate
        # pairs (emoji) into single scalars, which stay accepted.
        _reject_lone_surrogates(parsed)
        return parsed
    except JsonSchemaError:
        raise  # already a mapped malformed reason (duplicate key / non-finite
        # constant / out-of-range number) — do not re-wrap it as a generic
        # "invalid JSON", so the auditor-facing detail stays accurate; it still
        # routes to the same malformed verdict path.
    except RecursionError:
        # Defensive: `_check_json_depth` rejects any member at/over the 128-deep
        # boundary before json.loads runs, so json.loads never sees input deep
        # enough to raise RecursionError for nesting. Kept so that if some other
        # path ever provoked it, a hostile input still maps to a VERDICT (the
        # same malformed path) rather than an uncaught traceback / exit 3.
        raise JsonSchemaError("JSON nesting too deep")
    except ValueError as e:
        raise JsonSchemaError("invalid JSON: %s" % e)


def _bad_utf8(s):
    """True if `s` carries a surrogateescape byte (i.e. the original bytes were
    not valid UTF-8 here) — the mark serde would have rejected."""
    return any(0xDC80 <= ord(c) <= 0xDCFF for c in s)


def _ck_str(s, what="string"):
    """A string serde deserializes into `String` must be valid UTF-8."""
    if _bad_utf8(s):
        raise JsonSchemaError("invalid UTF-8 in %s" % what)
    return s


def _ck_keys(obj):
    """serde reads every top-level key of a struct it deserializes (to match or
    skip it), UTF-8-validating each; skipped VALUES are not validated."""
    for k in obj:
        _ck_str(k, "object key")


def _ck_value(v):
    """serde_json::Value validates UTF-8 of EVERY string recursively (keys and
    values) — used for fields typed `serde_json::Value` (body, actor, ...).

    Guard frame: a value nested past the interpreter's recursion limit would make
    the recursive walk below raise RecursionError (a RuntimeError, NOT a
    JsonSchemaError), which would escape every caller's `except JsonSchemaError`
    and crash the verifier on hostile input. Catch it HERE, at the outermost
    frame — by the time it propagates back up the whole recursive walk has
    unwound, so re-raising is safe — and map it to the same malformed path every
    caller already handles. A hostile deeply-nested member must yield a VERDICT,
    never an uncaught traceback. (Rust rejects such a member at serde_json's own
    recursion cap; the exact row may differ across the two, but neither crashes.)"""
    try:
        _ck_value_rec(v)
    except RecursionError:
        raise JsonSchemaError("value nesting too deep")


def _ck_value_rec(v):
    if isinstance(v, str):
        _ck_str(v, "value string")
    elif isinstance(v, dict):
        for k, val in v.items():
            _ck_str(k, "object key")
            _ck_value_rec(val)
    elif isinstance(v, list):
        for item in v:
            _ck_value_rec(item)


def _require_object(v):
    if not isinstance(v, dict):
        raise JsonSchemaError("expected a JSON object")
    _ck_keys(v)
    return v


def get_u64(obj, key):
    if key not in obj:
        raise JsonSchemaError("missing field %r" % key)
    v = obj[key]
    if isinstance(v, bool) or not isinstance(v, int):
        raise JsonSchemaError("field %r must be an unsigned integer" % key)
    if v < 0 or v > 0xFFFF_FFFF_FFFF_FFFF:
        raise JsonSchemaError("field %r out of u64 range" % key)
    return v


def get_str(obj, key):
    if key not in obj:
        raise JsonSchemaError("missing field %r" % key)
    v = obj[key]
    if not isinstance(v, str):
        raise JsonSchemaError("field %r must be a string" % key)
    return _ck_str(v, "field %r" % key)


def get_opt_str(obj, key):
    """serde `Option<String>`: missing or null -> None; string -> value."""
    v = obj.get(key)
    if v is None:
        return None
    if not isinstance(v, str):
        raise JsonSchemaError("field %r must be a string or null" % key)
    return _ck_str(v, "field %r" % key)


def deny_unknown(obj, allowed):
    for k in obj:
        _ck_str(k, "object key")
        if k not in allowed:
            raise JsonSchemaError("unknown field %r" % k)


# ============================================================================
# RFC 3339 timestamps (mirrors chrono::DateTime::parse_from_rfc3339). Returns an
# `_Instant` ordered by an integer nanosecond key so comparisons match chrono's
# absolute-instant ordering EXACTLY — including two forms Python's datetime
# cannot represent and that the Task-16 differential fuzzer proved chrono
# accepts (chrono is the authoritative trust core; the fix lives here, never in
# Rust):
#   * sub-microsecond precision — chrono keeps up to 9 fractional digits
#     (nanoseconds); datetime truncates at microseconds. A checkpoint clock
#     step or anchor comparison that turns on the 7th-9th fractional digit
#     therefore used to diverge (a parity-visible dominant-code difference on
#     row 24 / row 23), never a false-VALID.
#   * leap second ':60' — chrono accepts it at any time (representing it as the
#     59th second plus a full extra second of nanoseconds); datetime raises.
# `datetime` is still the validator for every range where the two AGREE (month,
# day-of-month, hour, second 61+, Feb 29 in a non-leap year, offset shape).
# ============================================================================

def _days_from_civil(y, m, d):
    """Days since 1970-01-01 in the proleptic Gregorian calendar, integer-exact
    for any date (Howard Hinnant's algorithm). Used only to build the integer
    nanosecond ordering key below; datetime validates the ranges first."""
    y2 = y - (1 if m <= 2 else 0)
    era = (y2 if y2 >= 0 else y2 - 399) // 400
    yoe = y2 - era * 400
    doy = (153 * (m + (-3 if m > 2 else 9)) + 2) // 5 + d - 1
    doe = yoe * 365 + yoe // 4 - yoe // 100 + doy
    return era * 146097 + doe - 719468


class _Instant:
    """A parsed RFC-3339 instant ordered by an integer nanosecond key so
    comparisons match chrono::DateTime<Utc> exactly (sub-microsecond precision
    and leap seconds included). `isoformat()` delegates to a best-effort
    microsecond datetime for display only — never part of the parity-compared
    verdict."""
    __slots__ = ("_key", "_dt")

    def __init__(self, key_ns, dt):
        self._key = key_ns
        self._dt = dt

    def __eq__(self, other):
        return isinstance(other, _Instant) and self._key == other._key

    def __ne__(self, other):
        return not self.__eq__(other)

    def __lt__(self, other):
        if not isinstance(other, _Instant):
            return NotImplemented
        return self._key < other._key

    def __le__(self, other):
        if not isinstance(other, _Instant):
            return NotImplemented
        return self._key <= other._key

    def __gt__(self, other):
        if not isinstance(other, _Instant):
            return NotImplemented
        return self._key > other._key

    def __ge__(self, other):
        if not isinstance(other, _Instant):
            return NotImplemented
        return self._key >= other._key

    __hash__ = None

    def isoformat(self):
        return self._dt.isoformat()


def parse_rfc3339(s):
    if not isinstance(s, str):
        raise ValueError("timestamp must be a string")
    text = s
    # date 'T'/'t'/' ' time, then zone.
    if len(text) < 20:
        raise ValueError("timestamp too short")
    date_part = text[0:10]
    sep = text[10]
    if sep not in ("T", "t", " "):
        raise ValueError("bad date/time separator")
    rest = text[11:]
    y, mo, d = date_part[0:4], date_part[5:7], date_part[8:10]
    if date_part[4] != "-" or date_part[7] != "-":
        raise ValueError("bad date format")
    # find zone start
    if rest[-1] in ("Z", "z"):
        time_part = rest[:-1]
        tz = timezone.utc
    else:
        # look for +HH:MM or -HH:MM at the end
        if len(rest) < 6 or rest[-3] != ":" or rest[-6] not in ("+", "-"):
            raise ValueError("missing/invalid timezone")
        time_part = rest[:-6]
        sign = 1 if rest[-6] == "+" else -1
        oh_str = rest[-5:-3]
        om_str = rest[-2:]
        # ASCII digits only (int() would accept "+5"/" 5"/unicode digits that
        # chrono rejects), then chrono's actual accepted range: reproduced with
        # DateTime::parse_from_rfc3339 (Task-16 probe), it accepts offset hour
        # 0..=23 and minute 0..=59 (up to +/-23:59) and rejects anything past
        # that as "out of range" (e.g. +00:60, +24:00). Without this bound a
        # `+00:60` silently became +1h here — a false-VALID vs the authoritative
        # Rust core's malformed-reject.
        if not (all("0" <= c <= "9" for c in oh_str) and all("0" <= c <= "9" for c in om_str)):
            raise ValueError("non-numeric timezone offset")
        oh = int(oh_str)
        om = int(om_str)
        if oh > 23 or om > 59:
            raise ValueError("timezone offset out of range")
        from datetime import timedelta
        tz = timezone(sign * timedelta(hours=oh, minutes=om))
    hh = time_part[0:2]
    mm = time_part[3:5]
    ss = time_part[6:8]
    if time_part[2] != ":" or time_part[5] != ":":
        raise ValueError("bad time format")
    micro = 0
    frac_ns = 0
    if len(time_part) > 8:
        if time_part[8] != ".":
            raise ValueError("bad fractional separator")
        frac = time_part[9:]
        # ASCII digits only: chrono rejects non-ASCII digits, but str.isdigit()
        # would accept superscripts / full-width forms.
        if not frac or not all("0" <= c <= "9" for c in frac):
            raise ValueError("bad fractional seconds")
        # chrono keeps up to 9 fractional digits (nanoseconds) and truncates
        # the rest — it does NOT round to microseconds.
        frac_ns = int((frac + "000000000")[:9])
        micro = frac_ns // 1000
    for part in (y, mo, d, hh, mm, ss):
        if not part.isdigit():
            raise ValueError("non-numeric datetime component")
    # chrono accepts a ':60' leap second at any time; datetime rejects second
    # == 60. Clamp to 59 for construction (datetime still validates every other
    # range exactly where chrono does) and carry the leap second in the key.
    ss_int = int(ss)
    leap_ns = 0
    if ss_int == 60:
        ss_for_dt = 59
        leap_ns = 1_000_000_000
    else:
        ss_for_dt = ss_int
    dt = datetime(int(y), int(mo), int(d), int(hh), int(mm), ss_for_dt, micro, tzinfo=tz)
    # Integer ordering key in UTC, matching chrono's absolute-instant comparison
    # at nanosecond resolution (datetime caps at microseconds). The multiplier
    # is 2e9, NOT 1e9: this makes the key LEXICOGRAPHIC on
    # (base_secs_using_ss_59, leap_ns + frac_ns), because `leap_ns + frac_ns`
    # maxes at 1e9 + 999_999_999 < 2e9 and so never carries into the next
    # base-second slot. A flat 1e9 multiplier would collapse a `:60` leap second
    # onto the following whole second (`00:00:60Z` == `00:01:00Z`), but chrono
    # (Task-16 probe: lt=true, eq=FALSE) orders `:60` STRICTLY BEFORE the next
    # second — so `00:00:60Z` = base(00:00:59)*2e9 + 1e9 sorts below
    # `00:01:00Z` = base(00:01:00)*2e9, exactly as chrono does.
    dt_utc = dt.astimezone(timezone.utc)
    base_secs = (_days_from_civil(dt_utc.year, dt_utc.month, dt_utc.day) * 86400
                 + dt_utc.hour * 3600 + dt_utc.minute * 60 + dt_utc.second)
    return _Instant(base_secs * 2_000_000_000 + leap_ns + frac_ns, dt)


# ============================================================================
# Container: central-directory-only ZIP reader enforcing the `.dbev` restricted
# profile. Mirrors container.rs. Every failure raises ContainerError; the
# pipeline maps ALL of them to row 21 (cannot-verify-container-profile).
# ============================================================================

class ContainerError(Exception):
    pass


_LFH_SIG = b"PK\x03\x04"
_CDFH_SIG = b"PK\x01\x02"
_EOCD_SIG = b"PK\x05\x06"
_EOCD64_LOCATOR_SIG = b"PK\x06\x07"
_LFH_FIXED_LEN = 30
_CDFH_FIXED_LEN = 46
_EOCD_FIXED_LEN = 22
_EOCD64_LOCATOR_LEN = 20
_MAX_COMMENT_LEN = 0xFFFF
_MAX_BACKSCAN_WINDOW = _EOCD_FIXED_LEN + _MAX_COMMENT_LEN
_U16_SENTINEL = 0xFFFF
_U32_SENTINEL = 0xFFFF_FFFF
_GPBF_UTF8 = 0x0800
_METHOD_STORE = 0


def _u16(b, off):
    if off + 2 > len(b) or off < 0:
        raise ContainerError("truncated (u16 read)")
    return b[off] | (b[off + 1] << 8)


def _u32(b, off):
    if off + 4 > len(b) or off < 0:
        raise ContainerError("truncated (u32 read)")
    return b[off] | (b[off + 1] << 8) | (b[off + 2] << 16) | (b[off + 3] << 24)


def _is_flat_child(rest):
    return bool(rest) and rest not in (".", "..") and not any(c in rest for c in ("/", "\\", "\0"))


def validate_whitelisted_name(name):
    ok = (
        name == "manifest.json"
        or name == "journal/closure.jsonl"
        or name == "checkpoints.jsonl"
        or name == "trust/keys.jsonl"
    )
    if not ok and name.startswith("journal/epoch-") and name.endswith(".jsonl"):
        mid = name[len("journal/epoch-"):-len(".jsonl")]
        ok = len(mid) > 0 and all(c in "0123456789" for c in mid)
    if not ok and name.startswith("anchors/"):
        ok = _is_flat_child(name[len("anchors/"):])
    if not ok and name.startswith("content/"):
        ok = _is_flat_child(name[len("content/"):])
    if not ok and name.startswith("derived/"):
        ok = _is_flat_child(name[len("derived/"):])
    if not ok:
        raise ContainerError("path not whitelisted: %r" % name)


class ContainerReader:
    def __init__(self, names, index):
        self._names = names
        self._index = index

    def member_names(self):
        return self._names

    def get(self, name):
        """Member bytes or None if absent (mirrors member_bytes -> Result)."""
        return self._index.get(name)

    @staticmethod
    def open(data):
        eocd = _find_and_parse_eocd(data)
        entries = _parse_central_directory(data, eocd)
        names = []
        seen = set()
        index = {}
        cd_offset = eocd["cd_offset"]
        for e in entries:
            if e["name"] in seen:
                raise ContainerError("duplicate member %r" % e["name"])
            seen.add(e["name"])
            member = _read_and_verify_local_header(data, e, cd_offset)
            names.append(e["name"])
            index[e["name"]] = member
        return ContainerReader(names, index)


def _find_and_parse_eocd(data):
    if len(data) < _EOCD_FIXED_LEN:
        raise ContainerError("archive smaller than a minimal EOCD")
    window = min(_MAX_BACKSCAN_WINDOW, len(data))
    search_start = len(data) - window
    last_possible = len(data) - _EOCD_FIXED_LEN
    offset = None
    for off in range(last_possible, search_start - 1, -1):
        if data[off:off + 4] == _EOCD_SIG:
            offset = off
            break
    if offset is None:
        raise ContainerError("no EOCD signature found in the trailing %d bytes" % window)

    if offset >= _EOCD64_LOCATOR_LEN and \
            data[offset - _EOCD64_LOCATOR_LEN: offset - _EOCD64_LOCATOR_LEN + 4] == _EOCD64_LOCATOR_SIG:
        raise ContainerError("zip64 locator present")

    disk_number = _u16(data, offset + 4)
    cd_start_disk = _u16(data, offset + 6)
    cd_entries_this_disk = _u16(data, offset + 8)
    cd_entries_total = _u16(data, offset + 10)
    cd_size = _u32(data, offset + 12)
    cd_offset = _u32(data, offset + 16)
    comment_len = _u16(data, offset + 20)

    if disk_number != 0:
        raise ContainerError("multi-disk (disk_number != 0)")
    if cd_start_disk != 0:
        raise ContainerError("multi-disk (cd_start_disk != 0)")
    if cd_entries_this_disk != cd_entries_total:
        raise ContainerError("multi-disk (entry-count disagreement)")
    if cd_entries_total == _U16_SENTINEL or cd_size == _U32_SENTINEL or cd_offset == _U32_SENTINEL:
        raise ContainerError("zip64 sentinel present")
    if comment_len != 0:
        raise ContainerError("non-empty archive comment")
    if offset + _EOCD_FIXED_LEN + comment_len != len(data):
        raise ContainerError("trailing bytes after EOCD")

    return {
        "offset": offset,
        "cd_entries_total": cd_entries_total,
        "cd_size": cd_size,
        "cd_offset": cd_offset,
    }


def _parse_central_directory(data, eocd):
    cd_start = eocd["cd_offset"]
    cd_len = eocd["cd_size"]
    cd_end = cd_start + cd_len
    if cd_end > eocd["offset"] or cd_start > len(data) or cd_end > len(data):
        raise ContainerError("central directory out of bounds")
    cd = data[cd_start:cd_end]

    entries = []
    cursor = 0
    for index in range(eocd["cd_entries_total"]):
        if cursor + _CDFH_FIXED_LEN > len(cd):
            raise ContainerError("truncated central directory entry %d" % index)
        if cd[cursor:cursor + 4] != _CDFH_SIG:
            raise ContainerError("bad central directory signature at entry %d" % index)
        gp_flag = _u16(cd, cursor + 8)
        method = _u16(cd, cursor + 10)
        compressed_size = _u32(cd, cursor + 20)
        uncompressed_size = _u32(cd, cursor + 24)
        name_len = _u16(cd, cursor + 28)
        extra_len = _u16(cd, cursor + 30)
        comment_len = _u16(cd, cursor + 32)
        disk_start = _u16(cd, cursor + 34)
        local_header_offset = _u32(cd, cursor + 42)

        name_start = cursor + _CDFH_FIXED_LEN
        total_len = _CDFH_FIXED_LEN + name_len + extra_len + comment_len
        if cursor + total_len > len(cd):
            raise ContainerError("truncated central directory entry %d" % index)
        name_bytes = cd[name_start:name_start + name_len]
        try:
            name = name_bytes.decode("utf-8")
        except UnicodeDecodeError:
            raise ContainerError("non-UTF-8 member name at entry %d" % index)

        if compressed_size == _U32_SENTINEL or uncompressed_size == _U32_SENTINEL \
                or local_header_offset == _U32_SENTINEL:
            raise ContainerError("zip64 sentinel present")
        if disk_start != 0:
            raise ContainerError("multi-disk (entry disk_start != 0)")
        if method != _METHOD_STORE:
            raise ContainerError("unsupported method %d for %r" % (method, name))
        if compressed_size != uncompressed_size:
            raise ContainerError("STORE size mismatch for %r" % name)
        if gp_flag != _GPBF_UTF8:
            raise ContainerError("disallowed general-purpose flag 0x%04x for %r" % (gp_flag, name))
        if extra_len != 0:
            raise ContainerError("non-empty extra field for %r" % name)
        if comment_len != 0:
            raise ContainerError("non-empty member comment for %r" % name)
        validate_whitelisted_name(name)

        entries.append({
            "name": name,
            "method": method,
            "compressed_size": compressed_size,
            "uncompressed_size": uncompressed_size,
            "local_header_offset": local_header_offset,
        })
        cursor += total_len

    if cursor != len(cd):
        raise ContainerError("central directory size mismatch")
    return entries


def _read_and_verify_local_header(data, entry, cd_start):
    name = entry["name"]
    lh_off = entry["local_header_offset"]
    if not (lh_off + _LFH_FIXED_LEN <= cd_start and lh_off + _LFH_FIXED_LEN <= len(data)):
        raise ContainerError("local header offset out of bounds for %r" % name)
    if data[lh_off:lh_off + 4] != _LFH_SIG:
        raise ContainerError("bad local header signature for %r" % name)

    method = _u16(data, lh_off + 8)
    compressed_size = _u32(data, lh_off + 18)
    uncompressed_size = _u32(data, lh_off + 22)
    name_len = _u16(data, lh_off + 26)
    extra_len = _u16(data, lh_off + 28)

    name_start = lh_off + _LFH_FIXED_LEN
    name_end = name_start + name_len
    if not (name_end <= cd_start and name_end <= len(data)):
        raise ContainerError("local header name out of bounds for %r" % name)
    name_bytes = data[name_start:name_end]
    if name_bytes != name.encode("utf-8"):
        raise ContainerError("local/CD name mismatch for %r" % name)
    if method != entry["method"]:
        raise ContainerError("local/CD method mismatch for %r" % name)
    if compressed_size != entry["compressed_size"]:
        raise ContainerError("local/CD compressed_size mismatch for %r" % name)
    if uncompressed_size != entry["uncompressed_size"]:
        raise ContainerError("local/CD uncompressed_size mismatch for %r" % name)
    if extra_len != 0:
        raise ContainerError("local/CD extra_len mismatch for %r" % name)

    data_start = name_end
    data_end = data_start + entry["compressed_size"]
    if not (data_end <= cd_start and data_end <= len(data)):
        raise ContainerError("member data out of bounds for %r" % name)
    return data[data_start:data_end]


def split_lines(b):
    return [ln for ln in b.split(b"\n") if ln]


# ============================================================================
# Envelope verify (mirrors envelope.rs). Returns the decoded payload bytes or
# raises EnvelopeError with a `.kind` matching the Rust variant.
# ============================================================================

PT_RECORD = "application/vnd.docbrain.evidence.record.v1+json"
PT_CHECKPOINT = "application/vnd.docbrain.evidence.checkpoint.v1+json"
PT_KEYRECORD = "application/vnd.docbrain.evidence.keyrecord.v1+json"
PT_MANIFEST = "application/vnd.docbrain.evidence.manifest.v1+json"

_ENVELOPE_KEYS = {"payloadType", "payload", "sig", "keyid", "signatures"}


class EnvelopeError(Exception):
    def __init__(self, kind, msg=""):
        super().__init__(msg or kind)
        self.kind = kind  # 'Malformed' | 'Unsupported' | 'WrongPayloadType' | 'SignatureInvalid'


def verify_envelope(env_line, expected_type, vk_bytes):
    # serde deserializes the WHOLE WireEnvelope (deny_unknown_fields) up front:
    # every present field's UTF-8 is validated before any of verify_envelope's
    # own logic runs, so a bad-UTF-8 field is Malformed, not Unsupported.
    try:
        obj = _require_object(json_parse(env_line))
        deny_unknown(obj, _ENVELOPE_KEYS)
        payload_type = get_str(obj, "payloadType")
        payload_b64 = get_str(obj, "payload")
        sig_b64 = get_opt_str(obj, "sig")
        keyid = get_opt_str(obj, "keyid")
        signatures = obj.get("signatures")
        if signatures is not None:
            _ck_value(signatures)  # Option<serde_json::Value>
    except JsonSchemaError as e:
        raise EnvelopeError("Malformed", "envelope JSON: %s" % e)

    if signatures is not None:
        raise EnvelopeError("Unsupported", "multi-signature envelope")
    if payload_type != expected_type:
        raise EnvelopeError("WrongPayloadType",
                            "expected %s, got %s" % (expected_type, payload_type))
    if sig_b64 is None:
        raise EnvelopeError("Malformed", "missing sig")
    if keyid is None:
        raise EnvelopeError("Malformed", "missing keyid")
    try:
        payload = b64decode_strict(payload_b64)
    except Base64Error as e:
        raise EnvelopeError("Malformed", "payload base64: %s" % e)
    try:
        sig_bytes = b64decode_strict(sig_b64)
    except Base64Error as e:
        raise EnvelopeError("Malformed", "sig base64: %s" % e)
    if len(sig_bytes) != 64:
        raise EnvelopeError("Malformed", "sig must be 64 bytes, got %d" % len(sig_bytes))
    msg = pae(payload_type, payload)
    if not verify_pinned(vk_bytes, msg, sig_bytes):
        raise EnvelopeError("SignatureInvalid", "signature invalid")
    return payload


# ============================================================================
# Key chain (mirrors keys.rs). Raises KeyChainError with `.variant`/`.position`.
# ============================================================================

class KeyChainError(Exception):
    def __init__(self, variant, position=None, msg=""):
        super().__init__(msg or variant)
        self.variant = variant
        self.position = position


def _parse_verifying_key(hex_str, field):
    try:
        b = hex_to_32(hex_str)
    except HexError as e:
        raise KeyChainError("Malformed", None, "%s: %s" % (field, e))
    if _point_decompress(b) is None:
        raise KeyChainError("Malformed", None, "%s: invalid ed25519 public key" % field)
    return b


class CompromiseRecord:
    def __init__(self, position, compromised_key, claimed_time):
        self.position = position
        self.compromised_key = compromised_key
        self.claimed_compromise_time = claimed_time


class KeyChain:
    def __init__(self, events, recovery_key, compromise):
        self.events = events  # list of (position, vk_bytes), strictly increasing
        self.recovery_key = recovery_key
        self.compromise = compromise

    def all_signing_keys(self):
        return [vk for (_pos, vk) in self.events]

    def key_at_position(self, position):
        found = None
        for (pos, vk) in self.events:
            if pos <= position:
                found = vk
            else:
                break
        return found


def _decode_payload_bytes_keychain(line, index):
    try:
        obj = _require_object(json_parse(line))
        payload_b64 = get_str(obj, "payload")
    except JsonSchemaError as e:
        raise KeyChainError("Malformed", None, "index %d envelope JSON: %s" % (index, e))
    try:
        return b64decode_strict(payload_b64)
    except Base64Error as e:
        raise KeyChainError("Malformed", None, "index %d payload base64: %s" % (index, e))


def _peek_kind(payload_bytes, index):
    try:
        obj = _require_object(json_parse(payload_bytes))
        return get_str(obj, "kind")
    except JsonSchemaError as e:
        raise KeyChainError("Malformed", None, "index %d kind peek: %s" % (index, e))


def verify_key_chain(lines):
    if not lines:
        raise KeyChainError("MissingGenesis")

    genesis_line = lines[0]
    gpb = _decode_payload_bytes_keychain(genesis_line, 0)
    if _peek_kind(gpb, 0) != "genesis":
        raise KeyChainError("MissingGenesis")
    try:
        g = _require_object(json_parse(gpb))
        deny_unknown(g, {"kind", "position", "signing_key", "recovery_key",
                         "predecessor_genesis", "key_prev"})
        get_str(g, "kind")
        position = get_u64(g, "position")
        signing_key_hex = get_str(g, "signing_key")
        recovery_key_hex = get_opt_str(g, "recovery_key")
        key_prev_hex = get_str(g, "key_prev")
        if g.get("predecessor_genesis") is not None:
            _ck_value(g["predecessor_genesis"])  # Option<serde_json::Value>
    except JsonSchemaError as e:
        raise KeyChainError("Malformed", None, "genesis: %s" % e)
    if position != 0:
        raise KeyChainError("GenesisPositionMismatch", None, "found %d" % position)
    try:
        genesis_key_prev = hex_to_32(key_prev_hex)
    except HexError as e:
        raise KeyChainError("Malformed", None, "key_prev: %s" % e)
    if genesis_key_prev != GENESIS_PREV:
        raise KeyChainError("KeyLinkMismatch", 0)
    signing_vk = _parse_verifying_key(signing_key_hex, "signing_key")
    try:
        verify_envelope(genesis_line, PT_KEYRECORD, signing_vk)
    except EnvelopeError as e:
        if e.kind == "SignatureInvalid":
            raise KeyChainError("GenesisNotSelfSigned")
        raise KeyChainError("Malformed", None, "genesis envelope: %s" % e)
    recovery_vk = None
    if recovery_key_hex is not None:
        recovery_vk = _parse_verifying_key(recovery_key_hex, "recovery_key")

    events = [(0, signing_vk)]
    current_signing_vk = signing_vk
    compromise = None
    last_position = 0
    head = head_hash(GENESIS_PREV, leaf_hash(genesis_line))

    for index in range(1, len(lines)):
        line = lines[index]
        if compromise is not None:
            raise KeyChainError("JournalSealed", compromise.position,
                                "index %d after seal" % index)
        payload_bytes = _decode_payload_bytes_keychain(line, index)
        kind = _peek_kind(payload_bytes, index)

        if kind == "genesis":
            raise KeyChainError("DuplicateGenesis", None, "index %d" % index)
        elif kind == "rotation":
            try:
                r = _require_object(json_parse(payload_bytes))
                deny_unknown(r, {"kind", "position", "new_signing_key", "key_prev"})
                get_str(r, "kind")
                rpos = get_u64(r, "position")
                new_signing_key_hex = get_str(r, "new_signing_key")
                rkey_prev = get_str(r, "key_prev")
            except JsonSchemaError as e:
                raise KeyChainError("Malformed", None, "rotation: %s" % e)
            if rpos <= last_position:
                raise KeyChainError("PositionNotIncreasing", None,
                                    "previous %d found %d" % (last_position, rpos))
            try:
                declared_prev = hex_to_32(rkey_prev)
            except HexError as e:
                raise KeyChainError("Malformed", None, "key_prev: %s" % e)
            if declared_prev != head:
                raise KeyChainError("KeyLinkMismatch", rpos)
            try:
                verify_envelope(line, PT_KEYRECORD, current_signing_vk)
            except EnvelopeError as e:
                if e.kind == "SignatureInvalid":
                    raise KeyChainError("UnauthorizedRotation", rpos)
                raise KeyChainError("Malformed", None, "rotation envelope: %s" % e)
            new_vk = _parse_verifying_key(new_signing_key_hex, "new_signing_key")
            events.append((rpos, new_vk))
            current_signing_vk = new_vk
            last_position = rpos
            head = head_hash(head, leaf_hash(line))
        elif kind == "compromise":
            try:
                c = _require_object(json_parse(payload_bytes))
                deny_unknown(c, {"kind", "position", "compromised_key",
                                 "claimed_compromise_time", "key_prev"})
                get_str(c, "kind")
                cpos = get_u64(c, "position")
                compromised_key_hex = get_str(c, "compromised_key")
                claimed_time_str = get_str(c, "claimed_compromise_time")
                ckey_prev = get_str(c, "key_prev")
            except JsonSchemaError as e:
                raise KeyChainError("Malformed", None, "compromise: %s" % e)
            if cpos <= last_position:
                raise KeyChainError("PositionNotIncreasing", None,
                                    "previous %d found %d" % (last_position, cpos))
            try:
                declared_prev = hex_to_32(ckey_prev)
            except HexError as e:
                raise KeyChainError("Malformed", None, "key_prev: %s" % e)
            if declared_prev != head:
                raise KeyChainError("KeyLinkMismatch", cpos)
            if recovery_vk is None:
                raise KeyChainError("UnauthorizedControlRecord", cpos)
            try:
                verify_envelope(line, PT_KEYRECORD, recovery_vk)
            except EnvelopeError as e:
                if e.kind == "SignatureInvalid":
                    raise KeyChainError("UnauthorizedControlRecord", cpos)
                raise KeyChainError("Malformed", None, "compromise envelope: %s" % e)
            try:
                compromised_key = hex_to_32(compromised_key_hex)
            except HexError as e:
                raise KeyChainError("Malformed", None, "compromised_key: %s" % e)
            try:
                claimed_time = parse_rfc3339(claimed_time_str)
            except ValueError as e:
                raise KeyChainError("InvalidCompromiseTime", cpos, str(e))
            last_position = cpos
            head = head_hash(head, leaf_hash(line))
            compromise = CompromiseRecord(cpos, compromised_key, claimed_time)
        else:
            raise KeyChainError("UnknownKind", None, "index %d kind %r" % (index, kind))

    return KeyChain(events, recovery_vk, compromise)


# compromise classification (keys.rs)
def classify_compromise(chain, record_position, record_keyid, anchored_before_claim):
    if chain.compromise is None:
        return "NotAffected"
    try:
        rb = hex_to_32(record_keyid)
    except HexError:
        return "NotAffected"
    if rb != chain.compromise.compromised_key:
        return "NotAffected"
    if record_position >= chain.compromise.position:
        return "TamperedPostPosition"
    return "ValidPreClaim" if anchored_before_claim else "IndeterminateWindow"


# ============================================================================
# Record chain walk (mirrors chain.rs).
# ============================================================================

class ChainError(Exception):
    def __init__(self, variant, position=None, found=None, msg=""):
        super().__init__(msg or variant)
        self.variant = variant
        self.position = position
        self.found = found


_RECORD_HEADER_KEYS = {"position", "prev_head", "class", "kind", "at", "actor",
                       "content_hash", "body", "backfilled"}


def _parse_record_header(line, index):
    try:
        obj = _require_object(json_parse(line))
        payload_b64 = get_str(obj, "payload")
    except JsonSchemaError as e:
        raise ChainError("Malformed", msg="index %d envelope JSON: %s" % (index, e))
    try:
        payload_bytes = b64decode_strict(payload_b64)
    except Base64Error as e:
        raise ChainError("Malformed", msg="index %d payload base64: %s" % (index, e))
    try:
        p = _require_object(json_parse(payload_bytes))
        deny_unknown(p, _RECORD_HEADER_KEYS)
        position = get_u64(p, "position")
        prev_head = hex_to_32(get_str(p, "prev_head"))
        get_str(p, "class")
        get_str(p, "kind")
        get_str(p, "at")
        if "actor" not in p:
            raise JsonSchemaError("missing field 'actor'")
        _ck_value(p["actor"])  # actor: serde_json::Value (UTF-8 validated)
        if "body" not in p:
            raise JsonSchemaError("missing field 'body'")
        _ck_value(p["body"])  # body: serde_json::Value (UTF-8 validated)
        # content_hash (optional) and backfilled (optional, bool) are read
        # by the closed schema but not needed by the walk itself.
        ch = p.get("content_hash")
        if ch is not None:
            if not isinstance(ch, str):
                raise JsonSchemaError("content_hash must be a string or null")
            hex_to_32(ch)
        if "backfilled" in p and not isinstance(p["backfilled"], bool):
            raise JsonSchemaError("backfilled must be a bool")
    except (JsonSchemaError, HexError) as e:
        raise ChainError("Malformed", msg="index %d record payload: %s" % (index, e))
    return position, prev_head


def chain_heads(start_position, start_head, envelope_lines):
    position = start_position
    head = start_head
    heads = []
    for index, line in enumerate(envelope_lines):
        rec_position, rec_prev_head = _parse_record_header(line, index)
        if position + 1 > 0xFFFF_FFFF_FFFF_FFFF:
            raise ChainError("PositionOverflow", msg="index %d" % index)
        expected_position = position + 1
        if rec_position != expected_position:
            if rec_position == position:
                raise ChainError("PositionDuplicate", position=rec_position)
            raise ChainError("PositionGap", found=rec_position,
                             msg="expected %d found %d" % (expected_position, rec_position))
        if rec_prev_head != head:
            raise ChainError("LinkMismatch", position=rec_position)
        leaf = leaf_hash(line)
        head = head_hash(head, leaf)
        position = rec_position
        heads.append((position, head))
    return heads


def walk_chain(start_position, start_head, envelope_lines):
    heads = chain_heads(start_position, start_head, envelope_lines)
    if heads:
        return heads[-1]
    return (start_position, start_head)


# ============================================================================
# Checkpoint chain (mirrors checkpoint.rs).
# ============================================================================

class CpError(Exception):
    def __init__(self, variant, position=None, msg=""):
        super().__init__(msg or variant)
        self.variant = variant
        self.position = position


class Checkpoint:
    def __init__(self, position, head, count, at):
        self.position = position
        self.head = head
        self.count = count
        self.at = at


class ClockAnomaly:
    def __init__(self, position, previous_position):
        self.position = position
        self.previous_position = previous_position


class CheckpointChain:
    def __init__(self, checkpoints, clock_anomalies):
        self.checkpoints = checkpoints
        self.clock_anomalies = clock_anomalies

    def checkpoint_at(self, position):
        for cp in self.checkpoints:
            if cp.position == position:
                return cp
        return None


_CHECKPOINT_KEYS = {"position", "head", "count", "at", "keyid", "cp_prev"}


def verify_checkpoint_chain(lines, keys):
    if not lines:
        raise CpError("Empty")

    checkpoints = []
    clock_anomalies = []
    head = GENESIS_PREV
    last_position = None
    last_at = None

    for index, line in enumerate(lines):
        try:
            obj = _require_object(json_parse(line))
            payload_b64 = get_str(obj, "payload")
        except JsonSchemaError as e:
            raise CpError("Malformed", None, "index %d envelope JSON: %s" % (index, e))
        try:
            payload_bytes = b64decode_strict(payload_b64)
        except Base64Error as e:
            raise CpError("Malformed", None, "index %d payload base64: %s" % (index, e))
        try:
            p = _require_object(json_parse(payload_bytes))
            deny_unknown(p, _CHECKPOINT_KEYS)
            position = get_u64(p, "position")
            head_hex = get_str(p, "head")
            count = get_u64(p, "count")
            at_str = get_str(p, "at")
            keyid_hex = get_str(p, "keyid")
            cp_prev_hex = get_str(p, "cp_prev")
        except JsonSchemaError as e:
            raise CpError("Malformed", None, "index %d checkpoint payload: %s" % (index, e))

        if last_position is not None and position <= last_position:
            raise CpError("PositionNotIncreasing", position,
                          "previous %d found %d" % (last_position, position))

        try:
            declared_cp_prev = hex_to_32(cp_prev_hex)
        except HexError as e:
            raise CpError("Malformed", None, "index %d cp_prev: %s" % (index, e))
        if declared_cp_prev != head:
            raise CpError("CpLinkMismatch", position)

        expected_vk = keys.key_at_position(position)
        if expected_vk is None:
            raise CpError("UnauthorizedSigner", position)
        try:
            declared_keyid = hex_to_32(keyid_hex)
        except HexError as e:
            raise CpError("Malformed", None, "index %d keyid: %s" % (index, e))
        if declared_keyid != expected_vk:
            raise CpError("UnauthorizedSigner", position)
        try:
            verify_envelope(line, PT_CHECKPOINT, expected_vk)
        except EnvelopeError as e:
            if e.kind == "SignatureInvalid":
                raise CpError("SignatureInvalid", position)
            raise CpError("Malformed", None, "index %d checkpoint envelope: %s" % (index, e))

        try:
            declared_head = hex_to_32(head_hex)
        except HexError as e:
            raise CpError("Malformed", None, "index %d head: %s" % (index, e))
        try:
            at = parse_rfc3339(at_str)
        except ValueError as e:
            raise CpError("InvalidTimestamp", position, str(e))

        if last_at is not None and last_position is not None and at <= last_at:
            clock_anomalies.append(ClockAnomaly(position, last_position))

        checkpoints.append(Checkpoint(position, declared_head, count, at))
        head = head_hash(head, leaf_hash(line))
        last_position = position
        last_at = at

    return CheckpointChain(checkpoints, clock_anomalies)


class RangeBounds:
    def __init__(self, start_position, start_head, end_position, end_head, end_count):
        self.start_position = start_position
        self.start_head = start_head
        self.end_position = end_position
        self.end_head = end_head
        self.end_count = end_count


def range_bounds(chain, range_pair):
    start, end = range_pair
    start_cp = chain.checkpoint_at(start)
    if start_cp is None:
        raise CpError("NotABoundary", start)
    end_cp = chain.checkpoint_at(end)
    if end_cp is None:
        raise CpError("NotABoundary", end)
    return RangeBounds(start_cp.position, start_cp.head, end_cp.position,
                       end_cp.head, end_cp.count)


# ============================================================================
# Manifest (mirrors manifest.rs).
# ============================================================================

MANIFEST_MEMBER_NAME = "manifest.json"


class ManifestError(Exception):
    def __init__(self, variant, msg=""):
        super().__init__(msg or variant)
        self.variant = variant  # 'Missing' | 'Malformed' | 'Signature'


class MemberError(Exception):
    def __init__(self, variant, name):
        super().__init__("%s: %s" % (variant, name))
        self.variant = variant  # 'HashMismatch' | 'Unlisted' | 'Missing'
        self.name = name


class Manifest:
    def __init__(self, scope_range, scope_classes, scope_spaces,
                 counts, ecp_position, ecp_head, ecp_count, tool_exporter, members):
        self.scope_range = scope_range
        self.scope_classes = scope_classes
        self.scope_spaces = scope_spaces
        self.counts = counts  # dict records/closure/withheld_erased
        self.ecp_position = ecp_position
        self.ecp_head = ecp_head
        self.ecp_count = ecp_count
        self.tool_exporter = tool_exporter
        self.members = members  # dict name -> 32-byte hash


def _peek_position(line):
    try:
        obj = _require_object(json_parse(line))
        payload_b64 = get_str(obj, "payload")
    except JsonSchemaError as e:
        raise ManifestError("Malformed", "envelope JSON: %s" % e)
    try:
        payload_bytes = b64decode_strict(payload_b64)
    except Base64Error as e:
        raise ManifestError("Malformed", "payload base64: %s" % e)
    try:
        p = _require_object(json_parse(payload_bytes))
        ecp = p.get("export_checkpoint")
        if not isinstance(ecp, dict):
            raise JsonSchemaError("missing/invalid export_checkpoint")
        _ck_keys(ecp)  # serde descends into export_checkpoint and reads its keys
        return get_u64(ecp, "position")
    except JsonSchemaError as e:
        raise ManifestError("Malformed", "position peek: %s" % e)


def verify_manifest(reader, keys):
    line = reader.get(MANIFEST_MEMBER_NAME)
    if line is None:
        raise ManifestError("Missing")

    position = _peek_position(line)
    vk = keys.key_at_position(position)
    if vk is None:
        raise ManifestError("Signature")
    try:
        payload = verify_envelope(line, PT_MANIFEST, vk)
    except EnvelopeError as e:
        if e.kind == "SignatureInvalid":
            raise ManifestError("Signature")
        raise ManifestError("Malformed", "manifest envelope: %s" % e)

    try:
        m = _require_object(json_parse(payload))
        deny_unknown(m, {"scope", "counts", "export_checkpoint", "tool", "members"})
        scope = _require_object(m["scope"]) if "scope" in m else _missing("scope")
        deny_unknown(scope, {"range", "classes", "spaces"})
        rng = scope.get("range")
        if not isinstance(rng, list) or len(rng) != 2:
            raise JsonSchemaError("scope.range must be a 2-element array")
        r0 = _as_u64(rng[0], "scope.range[0]")
        r1 = _as_u64(rng[1], "scope.range[1]")
        classes = scope.get("classes")
        if not isinstance(classes, list) or not all(isinstance(c, str) for c in classes):
            raise JsonSchemaError("scope.classes must be an array of strings")
        for c in classes:
            _ck_str(c, "scope.classes element")
        spaces = scope.get("spaces")
        if spaces is not None:
            if not isinstance(spaces, list) or not all(isinstance(c, str) for c in spaces):
                raise JsonSchemaError("scope.spaces must be an array of strings or null")
            for c in spaces:
                _ck_str(c, "scope.spaces element")

        counts = _require_object(m["counts"]) if "counts" in m else _missing("counts")
        deny_unknown(counts, {"records", "closure", "withheld_erased"})
        c_records = get_u64(counts, "records")
        c_closure = get_u64(counts, "closure")
        c_withheld = get_u64(counts, "withheld_erased")

        ecp = _require_object(m["export_checkpoint"]) if "export_checkpoint" in m else _missing("export_checkpoint")
        deny_unknown(ecp, {"position", "head", "count"})
        ecp_position = get_u64(ecp, "position")
        ecp_head_hex = get_str(ecp, "head")
        ecp_count = get_u64(ecp, "count")

        tool = _require_object(m["tool"]) if "tool" in m else _missing("tool")
        deny_unknown(tool, {"exporter"})
        tool_exporter = get_str(tool, "exporter")

        members_wire = m.get("members")
        if not isinstance(members_wire, dict):
            raise JsonSchemaError("members must be an object")
    except JsonSchemaError as e:
        raise ManifestError("Malformed", "manifest payload JSON: %s" % e)

    try:
        ecp_head = hex_to_32(ecp_head_hex)
    except HexError as e:
        raise ManifestError("Malformed", "export_checkpoint.head: %s" % e)

    members = {}
    for name, hex_hash in members_wire.items():
        if _bad_utf8(name):  # BTreeMap<String,_> key: serde validates UTF-8
            raise ManifestError("Malformed", "member name has invalid UTF-8")
        if not isinstance(hex_hash, str):
            raise ManifestError("Malformed", "members[%r] must be a string" % name)
        try:
            members[name] = hex_to_32(hex_hash)
        except HexError as e:
            raise ManifestError("Malformed", "members[%r]: %s" % (name, e))

    return Manifest((r0, r1), classes, spaces,
                    {"records": c_records, "closure": c_closure, "withheld_erased": c_withheld},
                    ecp_position, ecp_head, ecp_count, tool_exporter, members)


def _missing(field):
    raise JsonSchemaError("missing field %r" % field)


def _as_u64(v, what):
    if isinstance(v, bool) or not isinstance(v, int):
        raise JsonSchemaError("%s must be an unsigned integer" % what)
    if v < 0 or v > 0xFFFF_FFFF_FFFF_FFFF:
        raise JsonSchemaError("%s out of u64 range" % what)
    return v


def verify_members(reader, manifest):
    for name in reader.member_names():
        if name == MANIFEST_MEMBER_NAME:
            continue
        expected = manifest.members.get(name)
        if expected is None:
            raise MemberError("Unlisted", name)
        blob = reader.get(name)
        if blob is None:
            raise MemberError("Unlisted", name)
        got = hashlib.sha256(blob).digest()
        if got != expected:
            raise MemberError("HashMismatch", name)
    for name in sorted(manifest.members.keys()):
        if reader.get(name) is None:
            raise MemberError("Missing", name)


# ============================================================================
# Verdict taxonomy (mirrors verdict.rs) + finding codes (mirrors verify.rs).
# ============================================================================

CODE_TAMPERED_SIGNATURE = "tampered-signature"
CODE_KEY_EPOCH = "tampered-key-epoch"
CODE_TAMPERED_CHAIN = "tampered-chain"
CODE_INVALID_ROTATION = "tampered-invalid-rotation"
CODE_UNAUTHORIZED_CONTROL = "tampered-unauthorized-control-record"
CODE_POST_COMPROMISE = "tampered-post-compromise-position"
CODE_VALID_PRE_CLAIM = "valid-pre-claim"
CODE_INDETERMINATE = "cannot-verify-compromise-window-indeterminate"
CODE_SCOPE = "tampered-scope"
CODE_TAMPERED_MANIFEST = "tampered-manifest"
CODE_TAMPERED_CONTENT = "tampered-content"
CODE_WITHHELD_ERASED = "withheld-erased"
CODE_BUNDLE_INCOMPLETE = "cannot-verify-bundle-incomplete"
CODE_ERASURE_INCONSISTENT = "cannot-verify-erasure-inconsistent"
CODE_UNKNOWN_KEY = "cannot-verify-unknown-key"
CODE_UNSUPPORTED = "cannot-verify-unsupported-format"
CODE_ANCHOR_INVALID = "anchor-invalid"
CODE_ANCHOR_UNLINKED = "anchor-unlinked"
CODE_CONTAINER_PROFILE = "cannot-verify-container-profile"
CODE_MALFORMED = "cannot-verify-malformed"
CODE_TIME_CLAIM_FALSIFIED = "time-claim-falsified"
CODE_CLOCK_ANOMALY = "clock-anomaly"
CODE_TRIVIAL_RANGE = "trivial-range"

NEGATIVE_SPACE = (
    "Not a runtime agent-action logger; not an IT-general-controls platform; not "
    "field-level redaction (v2, Merkle epochs + salted field commitments); not a compliance "
    "certification; not \"the Art 12 log\" of systems that log elsewhere."
)

VERDICT_VALID = "VALID"
VERDICT_TAMPERED = "TAMPERED"
VERDICT_CANNOT_VERIFY = "CANNOT_VERIFY"

_EXIT = {VERDICT_VALID: 0, VERDICT_TAMPERED: 1, VERDICT_CANNOT_VERIFY: 2}

_BLOCKING_TAMPERED = {2, 3, 4, 5, 6, 7, 10, 11, 12}
_BLOCKING_CANNOT = {9, 14, 15, 16, 17, 21, 22, 26}


class Finding:
    def __init__(self, row, code, detail, position=None):
        self.row = row
        self.code = code
        self.detail = detail
        self.position = position


def classify(findings):
    norm = []
    for f in findings:
        if 1 <= f.row <= 26:
            norm.append(f)
        else:
            norm.append(Finding(26, "unmapped-state",
                                "no taxonomy row maps this finding: %s" % f.detail))
    for f in norm:
        if f.row in _BLOCKING_TAMPERED:
            return ("Blocking", VERDICT_TAMPERED, f, norm)
    for f in norm:
        if f.row in _BLOCKING_CANNOT:
            return ("Blocking", VERDICT_CANNOT_VERIFY, f, norm)
    dominant = norm[0] if norm else Finding(1, "valid", "all checks passed")
    return ("Clean", None, dominant, norm)


class VerdictReport:
    def __init__(self, verdict, dominant, findings, anchor_tier, scope_range,
                 scope_classes, scope_spaces, counts, time_confidence):
        self.verdict = verdict
        self.dominant = dominant
        self.findings = findings
        self.anchor_tier = anchor_tier
        self.scope_range = scope_range
        self.scope_classes = scope_classes
        self.scope_spaces = scope_spaces
        self.counts = counts
        self.time_confidence = time_confidence

    def exit_code(self):
        return _EXIT[self.verdict]

    def to_json(self):
        return {
            "verdict": self.verdict,
            "exit_code": self.exit_code(),
            "dominant": _finding_json(self.dominant),
            "findings": [_finding_json(f) for f in self.findings],
            "anchor_tier": self.anchor_tier,
            "scope": {
                "range": [self.scope_range[0], self.scope_range[1]],
                "classes": self.scope_classes,
                "spaces": self.scope_spaces,
            },
            "counts": {
                "records": self.counts["records"],
                "closure": self.counts["closure"],
                "withheld_erased": self.counts["withheld_erased"],
            },
            "negative_space": NEGATIVE_SPACE,
            "time_confidence": self.time_confidence,
        }

    def render_human(self):
        out = []
        out.append("Verdict: %s (row %d, %s)" % (self.verdict, self.dominant.row, self.dominant.code))
        out.append("Reason: %s" % self.dominant.detail)
        out.append("Scope: range [%d, %d], classes %r" % (
            self.scope_range[0], self.scope_range[1], self.scope_classes))
        out.append("Counts: %d records, %d withheld-erased, %d closure" % (
            self.counts["records"], self.counts["withheld_erased"], self.counts["closure"]))
        out.append("Anchor tier: %s" % self.anchor_tier)
        if len(self.findings) > 1:
            out.append("All findings (%d):" % len(self.findings))
            for f in self.findings:
                pos = " @position %d" % f.position if f.position is not None else ""
                out.append("  - row %d [%s]%s: %s" % (f.row, f.code, pos, f.detail))
        out.append("Negative space (what this does NOT prove):")
        out.append(NEGATIVE_SPACE)
        return "\n".join(out) + "\n"


def _finding_json(f):
    return {"row": f.row, "code": f.code, "detail": f.detail, "position": f.position}


# ============================================================================
# Error -> Finding mappings (mirror verify.rs's *_error_finding functions).
# ============================================================================

def _container_finding(e):
    return Finding(21, CODE_CONTAINER_PROFILE, str(e))


def _key_chain_finding(e):
    v = e.variant
    if v in ("Malformed", "MissingGenesis", "GenesisPositionMismatch",
             "DuplicateGenesis", "UnknownKind", "InvalidCompromiseTime"):
        return Finding(22, CODE_MALFORMED, str(e))
    if v == "GenesisNotSelfSigned":
        return Finding(2, CODE_TAMPERED_SIGNATURE, str(e))
    if v in ("PositionNotIncreasing", "KeyLinkMismatch"):
        return Finding(4, CODE_TAMPERED_CHAIN, str(e))
    if v == "UnauthorizedRotation":
        return Finding(5, CODE_INVALID_ROTATION, str(e), e.position)
    if v == "UnauthorizedControlRecord":
        return Finding(6, CODE_UNAUTHORIZED_CONTROL, str(e), e.position)
    if v == "JournalSealed":
        return Finding(7, CODE_POST_COMPROMISE, str(e), e.position)
    return Finding(22, CODE_MALFORMED, str(e))


def _manifest_finding(e):
    if e.variant == "Missing":
        return Finding(21, CODE_CONTAINER_PROFILE, str(e))
    if e.variant == "Malformed":
        return Finding(22, CODE_MALFORMED, str(e))
    return Finding(11, CODE_TAMPERED_MANIFEST, str(e))  # Signature


def _member_finding(e):
    if e.variant == "HashMismatch":
        return Finding(12, CODE_TAMPERED_CONTENT, str(e))
    return Finding(21, CODE_CONTAINER_PROFILE, str(e))  # Unlisted | Missing


def _cp_finding(e):
    v = e.variant
    if v in ("Empty", "Malformed", "InvalidTimestamp"):
        return Finding(22, CODE_MALFORMED, str(e))
    if v in ("CpLinkMismatch", "PositionNotIncreasing", "UnauthorizedSigner", "SignatureInvalid"):
        return Finding(4, CODE_TAMPERED_CHAIN, str(e), e.position)
    if v == "NotABoundary":
        return Finding(10, CODE_SCOPE, str(e), e.position)
    return Finding(22, CODE_MALFORMED, str(e))


def _chain_finding(e):
    v = e.variant
    if v == "Malformed":
        return Finding(22, CODE_MALFORMED, str(e))
    if v in ("LinkMismatch", "PositionDuplicate"):
        return Finding(4, CODE_TAMPERED_CHAIN, str(e), e.position)
    if v == "PositionGap":
        return Finding(4, CODE_TAMPERED_CHAIN, str(e), e.found)
    if v == "PositionOverflow":
        return Finding(4, CODE_TAMPERED_CHAIN, str(e))
    return Finding(22, CODE_MALFORMED, str(e))


# ============================================================================
# Per-record analysis (mirrors verify.rs analyze_record). Returns a dict on
# success, or raises RecordFinding carrying the Finding to push.
# ============================================================================

class RecordFinding(Exception):
    def __init__(self, finding):
        self.finding = finding


_ENV_PEEK_KEYS = None  # EnvPeek is NOT deny_unknown_fields (tolerant)


def _analyze_record(line, keys):
    # EnvPeek is tolerant, but serde still deserializes payloadType/payload/
    # keyid/signatures (all validated) up front; skipped envelope keys are not.
    try:
        obj = _require_object(json_parse(line))
        payload_type = get_str(obj, "payloadType")
        payload_b64 = get_str(obj, "payload")
        keyid_hex = get_opt_str(obj, "keyid")
        signatures = obj.get("signatures")
        if signatures is not None:
            _ck_value(signatures)
    except JsonSchemaError as e:
        raise RecordFinding(Finding(22, CODE_MALFORMED, "record envelope JSON: %s" % e))

    if signatures is not None:
        raise RecordFinding(Finding(17, CODE_UNSUPPORTED,
                                    "multi-signature record envelope (signatures array)"))
    if payload_type != PT_RECORD:
        raise RecordFinding(Finding(22, CODE_MALFORMED,
                                    "record payloadType mismatch: got %r" % payload_type))
    try:
        payload_bytes = b64decode_strict(payload_b64)
    except Base64Error as e:
        raise RecordFinding(Finding(22, CODE_MALFORMED, "record payload base64: %s" % e))
    try:
        header = _require_object(json_parse(payload_bytes))
        position = get_u64(header, "position")
        kind = get_str(header, "kind")
        # RecordPeek deserializes content_hash (Option<String>) and body
        # (Value): UTF-8 validated here, at parse time — before the signature
        # check, matching serde's eager field deserialization.
        ch = header.get("content_hash")
        if ch is not None and not isinstance(ch, str):
            raise JsonSchemaError("content_hash must be a string or null")
        if isinstance(ch, str):
            _ck_str(ch, "content_hash")
        if "body" in header:
            _ck_value(header["body"])
    except JsonSchemaError as e:
        raise RecordFinding(Finding(22, CODE_MALFORMED, "record payload JSON: %s" % e))

    if keyid_hex is None:
        raise RecordFinding(Finding(22, CODE_MALFORMED, "record envelope missing keyid", position))
    try:
        keyid_bytes = hex_to_32(keyid_hex)
    except HexError:
        raise RecordFinding(Finding(22, CODE_MALFORMED,
                                    "record keyid is not valid 32-byte hex", position))

    matched_vk = None
    for vk in keys.all_signing_keys():
        if vk == keyid_bytes:
            matched_vk = vk
            break
    if matched_vk is None:
        raise RecordFinding(Finding(16, CODE_UNKNOWN_KEY,
                                    "record signed by a key not reachable through the in-band key chain",
                                    position))

    try:
        verify_envelope(line, PT_RECORD, matched_vk)
    except EnvelopeError as e:
        if e.kind == "SignatureInvalid":
            raise RecordFinding(Finding(2, CODE_TAMPERED_SIGNATURE,
                                        "record signature invalid under its declared (recognized) key",
                                        position))
        raise RecordFinding(Finding(22, CODE_MALFORMED, "record envelope: %s" % e, position))

    # content_hash hex decode happens AFTER signature verification (mirrors
    # Rust: the Option<String> was UTF-8 validated at parse; the hex decode is
    # deferred to here).
    declared_content_hash = None
    if isinstance(ch, str):
        try:
            declared_content_hash = hex_to_32(ch)
        except HexError:
            raise RecordFinding(Finding(22, CODE_MALFORMED,
                                        "record content_hash is not valid 32-byte hex", position))

    position_correct = keys.key_at_position(position) == matched_vk
    body = header.get("body")  # serde default = Null if absent
    return {
        "position": position,
        "keyid_hex": keyid_hex,
        "matched_vk_bytes": matched_vk,
        "kind": kind,
        "content_hash": declared_content_hash,
        "body": body,
        "position_correct": position_correct,
    }


def _erasure_target(body):
    """serde_json::from_value::<ErasureBody>(body): body must be an object with a
    u64 `target`. Returns target or None on any failure (mirrors the Err arm)."""
    if not isinstance(body, dict):
        return None
    try:
        return get_u64(body, "target")
    except JsonSchemaError:
        return None


# ============================================================================
# Anchors (mirrors verify.rs process_anchors).
# ============================================================================

def _process_anchors(reader, checkpoints):
    tier_rank = {"none": 0, "witness-file-present": 1, "token-present-unvalidated": 2}
    tier = "none"
    findings = []
    for name in reader.member_names():
        if not name.startswith("anchors/"):
            continue
        blob = reader.get(name)
        if blob is None:
            continue
        try:
            # The anchor is the ONE bundle parse with no `deny_unknown_fields`
            # closed-schema struct to backstop it (anchors are deliberately
            # tolerant — the real exporter's witness_ref/token_b64 fields are
            # simply ignored), so it relies entirely on `json_parse` to enforce
            # the strict-JSON profile — the SAME six axes (valid UTF-8, no dup
            # keys, depth<128, in-range finite numbers, no NaN/Infinity, standard
            # syntax) the Rust `from_slice_strict` pre-pass enforces on the anchor
            # member. This rejects only NON-CONFORMANCE, never an unknown field,
            # so a forward-compat anchor carrying new valid fields still verifies.
            # RecursionError is folded in too (json_parse maps over-deep nesting to
            # JsonSchemaError): a hostile anchor must yield row 18, never a crash.
            a = _require_object(json_parse(blob))
            kind = get_str(a, "kind")
            checkpoint_position = get_u64(a, "checkpoint_position")
            tsa_time = get_opt_str(a, "tsa_time") if "tsa_time" in a else None
        except (JsonSchemaError, UnicodeDecodeError, RecursionError):
            findings.append(Finding(18, CODE_ANCHOR_INVALID, "anchor %s failed to parse" % name))
            continue
        checkpoint = None
        for cp in checkpoints.checkpoints:
            if cp.position == checkpoint_position:
                checkpoint = cp
                break
        if checkpoint is None:
            findings.append(Finding(19, CODE_ANCHOR_UNLINKED,
                                    "anchor %s references checkpoint position %d not in the bundle's chain"
                                    % (name, checkpoint_position)))
            continue
        this_tier = "witness-file-present" if kind == "witness" else "token-present-unvalidated"
        if tier_rank[this_tier] > tier_rank[tier]:
            tier = this_tier
        if tsa_time is not None:
            try:
                tsa = parse_rfc3339(tsa_time)
                falsified = checkpoint.at > tsa
            except ValueError:
                falsified = False
            if falsified:
                findings.append(Finding(23, CODE_TIME_CLAIM_FALSIFIED,
                                        "checkpoint %d wall-clock is later than anchor %s's TSA time"
                                        % (checkpoint.position, name),
                                        checkpoint.position))
    return tier, findings


# ============================================================================
# The pipeline (mirrors verify.rs verify_bundle_with_witness). Phase A fail-fast
# bootstrap, then Phase B accumulation, then classify, then one success exit.
# ============================================================================

def _terminal(finding, manifest=None):
    _disp, verdict, dominant, findings = classify([finding])
    if verdict is None:
        # INVARIANT (must match verify.rs `terminal`): every _terminal() call
        # site feeds a BLOCKING-row finding, so classify() never returns a clean
        # verdict here. Cross-language asymmetry to preserve: if a future edit
        # ever routed an INFORMATIONAL-row finding through _terminal(), this side
        # would fall back to VALID while Rust's `unreachable!()` panics — a
        # silent false-VALID here vs a loud crash there. Keep all _terminal()
        # findings blocking rows so this branch stays unreachable.
        verdict = VERDICT_VALID  # unreachable for a single blocking-row finding
    if manifest is not None:
        scope_range = manifest.scope_range
        scope_classes = manifest.scope_classes
        scope_spaces = manifest.scope_spaces
        counts = manifest.counts
    else:
        scope_range = (0, 0)
        scope_classes = []
        scope_spaces = None
        counts = {"records": 0, "closure": 0, "withheld_erased": 0}
    return VerdictReport(verdict, dominant, findings, "none", scope_range,
                         scope_classes, scope_spaces, counts, [])


def _required_member_lines(reader, name):
    blob = reader.get(name)
    if blob is None:
        raise RecordFinding(Finding(21, CODE_CONTAINER_PROFILE,
                                    "required member %r is absent from the container" % name))
    return split_lines(blob)


def _collect_epoch_lines(reader):
    epoch_files = []
    for n in reader.member_names():
        if n.startswith("journal/epoch-") and n.endswith(".jsonl"):
            mid = n[len("journal/epoch-"):-len(".jsonl")]
            if mid and all(c in "0123456789" for c in mid):
                num = int(mid)
                if 0 <= num <= 0xFFFF_FFFF_FFFF_FFFF:
                    epoch_files.append((num, n))
    epoch_files.sort(key=lambda t: t[0])
    lines = []
    for _num, name in epoch_files:
        blob = reader.get(name)
        if blob is None:
            continue
        lines.extend(split_lines(blob))
    return lines


def _collect_member_lines(reader, name):
    blob = reader.get(name)
    if blob is None:
        return []
    return split_lines(blob)


def verify_bundle(data, trusted_witness_times=None):
    if trusted_witness_times is None:
        trusted_witness_times = []
    # ---- Phase A: sequential fail-fast bootstrap ----
    try:
        reader = ContainerReader.open(data)
    except ContainerError as e:
        return _terminal(_container_finding(e))

    try:
        key_lines = _required_member_lines(reader, "trust/keys.jsonl")
    except RecordFinding as rf:
        return _terminal(rf.finding)
    try:
        keys = verify_key_chain(key_lines)
    except KeyChainError as e:
        return _terminal(_key_chain_finding(e))

    try:
        manifest = verify_manifest(reader, keys)
    except ManifestError as e:
        return _terminal(_manifest_finding(e))

    try:
        verify_members(reader, manifest)
    except MemberError as e:
        return _terminal(_member_finding(e), manifest)

    try:
        cp_lines = _required_member_lines(reader, "checkpoints.jsonl")
    except RecordFinding as rf:
        return _terminal(rf.finding, manifest)
    try:
        checkpoints = verify_checkpoint_chain(cp_lines, keys)
    except CpError as e:
        return _terminal(_cp_finding(e), manifest)

    try:
        bounds = range_bounds(checkpoints, manifest.scope_range)
    except CpError as e:
        return _terminal(_cp_finding(e), manifest)

    # ---- Phase B: findings accumulate ----
    findings = []

    if (manifest.ecp_position != bounds.end_position
            or manifest.ecp_head != bounds.end_head
            or manifest.ecp_count != bounds.end_count):
        findings.append(Finding(10, CODE_SCOPE,
                                "manifest export_checkpoint disagrees with the independently verified checkpoint chain"))

    epoch_lines = _collect_epoch_lines(reader)

    # B1: per-record analysis (rows 2,3,16,17,22)
    record_infos = []
    for line in epoch_lines:
        try:
            info = _analyze_record(line, keys)
            if not info["position_correct"]:
                findings.append(Finding(3, CODE_KEY_EPOCH,
                                        "record signed by a real chain key, but not the one authoritative at its declared position",
                                        info["position"]))
            record_infos.append(info)
        except RecordFinding as rf:
            findings.append(rf.finding)

    # B2: chain-link recomputation (row 4)
    try:
        final_pos, final_head = walk_chain(bounds.start_position, bounds.start_head, epoch_lines)
        if final_pos != bounds.end_position or final_head != bounds.end_head:
            findings.append(Finding(4, CODE_TAMPERED_CHAIN,
                                    "final chain head/position does not land on the closing checkpoint"))
    except ChainError as e:
        findings.append(_chain_finding(e))

    # B3: manifest.counts.records vs actual journal entry count (row 10)
    if manifest.counts["records"] != len(epoch_lines):
        findings.append(Finding(10, CODE_SCOPE,
                                "manifest counts.records disagrees with the actual number of journal entries"))

    # B4: compromise classification (rows 7,8,9)
    if keys.compromise is not None:
        comp = keys.compromise
        for info in record_infos:
            if not info["position_correct"]:
                continue
            if info["matched_vk_bytes"] != comp.compromised_key:
                continue
            anchored = _anchored_before_claim_for(info["position"], checkpoints,
                                                  comp.claimed_compromise_time, trusted_witness_times)
            cls = classify_compromise(keys, info["position"], info["keyid_hex"], anchored)
            if cls == "NotAffected":
                pass
            elif cls == "ValidPreClaim":
                findings.append(Finding(8, CODE_VALID_PRE_CLAIM,
                                        "record predates the claimed compromise time under an operator-trusted witness",
                                        info["position"]))
            elif cls == "TamperedPostPosition":
                findings.append(Finding(7, CODE_POST_COMPROMISE,
                                        "record signed by the compromised key at or after its compromise position",
                                        info["position"]))
            elif cls == "IndeterminateWindow":
                findings.append(Finding(9, CODE_INDETERMINATE,
                                        "record's compromise window is indeterminate (unanchored, or anchored within [C, declaration])",
                                        info["position"]))

    # B5: content / erasure closure (rows 12,13,14,15,22)
    in_range_positions = set(info["position"] for info in record_infos)
    erasure_targets = set()
    for info in record_infos:
        if info["kind"] == "erasure":
            target = _erasure_target(info["body"])
            if target is None:
                findings.append(Finding(22, CODE_MALFORMED,
                                        "erasure record body missing/invalid target", info["position"]))
            elif target in in_range_positions:
                erasure_targets.add(target)
            elif target != 0 and target <= bounds.start_position:
                pass  # benign: erasure of a record outside this export's window
            else:
                findings.append(Finding(22, CODE_MALFORMED,
                                        "erasure record targets position %d which is not a valid predecessor position (0, or past the export's own tip)" % target,
                                        info["position"]))

    closure_lines = _collect_member_lines(reader, "journal/closure.jsonl")
    for line in closure_lines:
        try:
            info = _analyze_record(line, keys)
            if not info["position_correct"]:
                findings.append(Finding(3, CODE_KEY_EPOCH,
                                        "closure record signed by a real chain key, but not the one authoritative at its declared position",
                                        info["position"]))
            if info["kind"] != "erasure":
                findings.append(Finding(22, CODE_MALFORMED,
                                        "journal/closure.jsonl may only carry erasure records, found kind %r" % info["kind"],
                                        info["position"]))
                continue
            if bounds.start_position < info["position"] <= bounds.end_position:
                findings.append(Finding(22, CODE_MALFORMED,
                                        "closure record's own position must be strictly outside the exported range",
                                        info["position"]))
                continue
            target = _erasure_target(info["body"])
            if target is not None and target in in_range_positions:
                erasure_targets.add(target)
            elif target is None:
                findings.append(Finding(22, CODE_MALFORMED,
                                        "closure erasure record body missing/invalid target", info["position"]))
            else:
                findings.append(Finding(22, CODE_MALFORMED,
                                        "closure erasure record targets position %d which does not resolve to any record in the exported range" % target,
                                        info["position"]))
        except RecordFinding as rf:
            findings.append(rf.finding)

    if manifest.counts["closure"] != len(closure_lines):
        findings.append(Finding(10, CODE_SCOPE,
                                "manifest closure count disagrees with the actual number of journal/closure.jsonl entries"))

    actual_withheld = 0
    for info in record_infos:
        declared_hash = info["content_hash"]
        if declared_hash is None:
            continue
        member_name = "content/%d" % info["position"]
        blob = reader.get(member_name)
        if blob is not None and len(blob) >= 32:
            salt = blob[:32]
            content = blob[32:]
            got = content_hash(salt, content)
            if got != declared_hash:
                findings.append(Finding(12, CODE_TAMPERED_CONTENT,
                                        "content blob salted-hash mismatch", info["position"]))
            elif info["position"] in erasure_targets:
                findings.append(Finding(15, CODE_ERASURE_INCONSISTENT,
                                        "content present despite a journaled erasure record", info["position"]))
        elif blob is not None:
            findings.append(Finding(22, CODE_MALFORMED,
                                    "content blob shorter than the 32-byte salt", info["position"]))
        else:
            if info["position"] in erasure_targets:
                findings.append(Finding(13, CODE_WITHHELD_ERASED,
                                        "content withheld per a journaled erasure record", info["position"]))
                actual_withheld += 1
            else:
                findings.append(Finding(14, CODE_BUNDLE_INCOMPLETE,
                                        "content missing and no matching erasure record", info["position"]))

    if manifest.counts["withheld_erased"] != actual_withheld:
        findings.append(Finding(10, CODE_SCOPE,
                                "manifest withheld_erased count disagrees with the actual number of withheld records"))

    # B6: clock anomalies (row 24, informational)
    for anomaly in checkpoints.clock_anomalies:
        findings.append(Finding(24, CODE_CLOCK_ANOMALY,
                                "checkpoint at position %d does not advance the wall clock past position %d"
                                % (anomaly.position, anomaly.previous_position),
                                anomaly.position))

    # B7: anchors (rows 18,19,23; tier)
    anchor_tier, anchor_findings = _process_anchors(reader, checkpoints)
    findings.extend(anchor_findings)

    # B8: trivial range (row 25, informational)
    if manifest.counts["records"] == 0:
        findings.append(Finding(25, CODE_TRIVIAL_RANGE, "zero-record range export"))

    _disp, verdict, dominant, findings = classify(findings)
    if verdict is None:
        verdict = VERDICT_VALID  # ONE success exit

    time_confidence = [
        {"label": "checkpoint at position %d" % cp.position,
         "anchored": False,
         "at": cp.at.isoformat()}
        for cp in checkpoints.checkpoints
    ]

    return VerdictReport(verdict, dominant, findings, anchor_tier,
                         manifest.scope_range, manifest.scope_classes, manifest.scope_spaces,
                         manifest.counts, time_confidence)


def _anchored_before_claim_for(position, checkpoints, claim, trusted_witness_times):
    covering = None
    for cp in checkpoints.checkpoints:
        if cp.position >= position:
            covering = cp
            break
    if covering is None:
        return False
    return any(cp_pos == covering.position and t < claim
               for (cp_pos, t) in trusted_witness_times)


# ============================================================================
# CLI
# ============================================================================

def _self_test(vectors_path):
    try:
        with open(vectors_path, "rb") as fh:
            data = json.load(fh)
    except (OSError, ValueError) as e:
        sys.stderr.write("error: cannot load vectors %s: %s\n" % (vectors_path, e))
        return 1
    vectors = data.get("vectors", [])
    import base64 as _b64  # stdlib; only for reading the vector encodings
    passed = 0
    failed = []
    for v in vectors:
        vk = _b64.b64decode(v["vk_b64"])
        msg = _b64.b64decode(v["msg_b64"])
        sig = _b64.b64decode(v["sig_b64"])
        got = verify_pinned(vk, msg, sig)  # verify_pinned itself rejects len != 64
        if got == v["expected"]:
            passed += 1
        else:
            failed.append((v["name"], got, v["expected"]))
    total = len(vectors)
    if failed:
        for name, got, exp in failed:
            sys.stderr.write("FAIL %s: got %s expected %s\n" % (name, got, exp))
        sys.stderr.write("self-test: %d/%d vectors passed\n" % (passed, total))
        return 1
    sys.stdout.write("self-test: %d/%d ed25519 vectors passed (pin=%s)\n"
                     % (passed, total, data.get("pin", "?")))
    return 0


def _default_vectors_path():
    here = os.path.dirname(os.path.abspath(__file__))
    return os.path.join(here, "..", "crates", "docbrain-evidence", "vectors", "ed25519_pin.json")


def main(argv):
    args = argv[1:]
    if "--self-test" in args:
        rest = [a for a in args if a != "--self-test"]
        vectors_path = rest[0] if rest else _default_vectors_path()
        return _self_test(vectors_path)

    as_json = False
    bundle_path = None
    for a in args:
        if a == "--json":
            as_json = True
        elif a.startswith("-"):
            sys.stderr.write("error: unknown flag %s\n" % a)
            return 3
        else:
            bundle_path = a
    if bundle_path is None:
        sys.stderr.write("usage: verify_dbev.py <bundle.dbev> [--json]\n")
        sys.stderr.write("       verify_dbev.py --self-test [<vectors.json>]\n")
        return 3

    try:
        with open(bundle_path, "rb") as fh:
            data = fh.read()
    except OSError as e:
        sys.stderr.write("error: cannot read bundle %s: %s\n" % (bundle_path, e))
        return 3

    report = verify_bundle(data)
    if as_json:
        sys.stdout.write(json.dumps(report.to_json(), indent=2) + "\n")
    else:
        sys.stdout.write(report.render_human())
    sys.stdout.flush()
    return report.exit_code()


if __name__ == "__main__":
    sys.exit(main(sys.argv))
