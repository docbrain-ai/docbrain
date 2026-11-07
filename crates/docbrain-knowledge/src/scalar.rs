// SPDX-License-Identifier: MIT
//! Scalars and comparison (`SEMANTICS.md` §Values and comparison, §Resolving values):
//! exact decimals, versions, ranges, YAML plain-scalar resolution
//! under 1.2 and 1.1, and `compare`.

use std::cmp::Ordering;
use std::fmt::Write as _;

use crate::bounds::NUMBER_TEXT_MAX;
use crate::reason::{Bound, Unverifiable};
use crate::record::Op;

/// An exact decimal: `digits` × 10^`exp`, `digits` without leading or trailing zeros. Zero has empty
/// digits, no sign and `exp` 0, so derived equality is numeric equality.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Decimal {
    neg: bool,
    digits: Box<str>,
    exp: i64,
}

/// Plain decimal notation while it stays within 64 characters, else `d.ddde<n>`: a value is shown short, never
/// expanded from a large exponent.
impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const PLAIN_MAX: i64 = 64;
        if self.digits.is_empty() {
            return f.write_str("0");
        }
        if self.neg {
            f.write_str("-")?;
        }
        let d = &*self.digits;
        let len = i64::try_from(d.len()).unwrap_or(i64::MAX);
        // The position of the decimal point, counted from the first digit.
        let point = len.saturating_add(self.exp);
        if self.exp >= 0 && point <= PLAIN_MAX {
            write!(f, "{d}{}", "0".repeat(usize::try_from(self.exp).unwrap_or(0)))
        } else if self.exp < 0 && point > 0 {
            let at = usize::try_from(point).unwrap_or(0);
            write!(f, "{}.{}", &d[..at], &d[at..])
        } else if self.exp < 0 && len.saturating_sub(point).saturating_add(2) <= PLAIN_MAX {
            write!(f, "0.{}{d}", "0".repeat(usize::try_from(-point).unwrap_or(0)))
        } else {
            let (first, rest) = d.split_at(1);
            let dot = if rest.is_empty() { "" } else { "." };
            write!(f, "{first}{dot}{rest}e{}", point.saturating_sub(1))
        }
    }
}

/// Non-empty and every byte an ASCII digit.
fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// One leading `+` or `-` removed; `true` for `-`.
fn sign(s: &str) -> (bool, &str) {
    match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    }
}

/// A number's parts under `[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?`: sign, integer digits,
/// fraction digits, and the exponent as (negative, digits).
struct NumberParts<'a> {
    neg: bool,
    int: &'a str,
    frac: &'a str,
    exp: Option<(bool, &'a str)>,
}

fn number_parts(s: &str) -> Option<NumberParts<'_>> {
    let (neg, rest) = sign(s);
    let (mant, exp) = match rest.find(['e', 'E']) {
        Some(i) => (&rest[..i], Some(sign(&rest[i + 1..]))),
        None => (rest, None),
    };
    let (int, frac, point) = match mant.split_once('.') {
        Some((i, f)) => (i, f, true),
        None => (mant, "", false),
    };
    let mant_ok = if int.is_empty() { point && all_digits(frac) } else { all_digits(int) && (frac.is_empty() || all_digits(frac)) };
    (mant_ok && exp.is_none_or(|(_, d)| all_digits(d))).then_some(NumberParts { neg, int, frac, exp })
}

impl Decimal {
    fn zero() -> Decimal {
        Decimal { neg: false, digits: "".into(), exp: 0 }
    }

    /// `int` and `frac` are ASCII digits; the value is `int.frac` × 10^`exp`.
    fn scaled(neg: bool, int: &str, frac: &str, exp: i64) -> Option<Decimal> {
        let all = format!("{int}{frac}");
        let lead = all.trim_start_matches('0');
        let body = lead.trim_end_matches('0');
        if body.is_empty() {
            return Some(Decimal::zero());
        }
        let trailing = i64::try_from(lead.len() - body.len()).ok()?;
        let exp = exp.checked_sub(i64::try_from(frac.len()).ok()?)?.checked_add(trailing)?;
        Some(Decimal { neg, digits: body.into(), exp })
    }

    /// a literal, or a string's decimal reading: `-?[0-9]+(\.[0-9]+)?`.
    pub fn parse_plain(s: &str) -> Option<Decimal> {
        let (neg, rest) = s.strip_prefix('-').map_or((false, s), |r| (true, r));
        let (int, frac) = rest.split_once('.').map_or((rest, None), |(i, f)| (i, Some(f)));
        if !all_digits(int) || frac.is_some_and(|f| !all_digits(f)) {
            return None;
        }
        Decimal::scaled(neg, int, frac.unwrap_or(""), 0)
    }

    /// A number in a file: `[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?`, exactly. `None` also when
    /// the exponent does not fit an `i64`.
    pub fn parse_number(s: &str) -> Option<Decimal> {
        let NumberParts { neg, int, frac, exp } = number_parts(s)?;
        let exp = match exp {
            None => 0,
            Some((eneg, d)) => {
                let v: i64 = d.parse().ok()?;
                if eneg { -v } else { v }
            }
        };
        Decimal::scaled(neg, int, frac, exp)
    }

    /// Digits in `radix` (2 to 16, either case, no sign, prefix or separator), exactly, at any length. The
    /// cost is the square of the length: callers bound the text (`NUMBER_TEXT_MAX`).
    pub fn from_radix(s: &str, radix: u32) -> Option<Decimal> {
        const BASE: u64 = 1_000_000_000;
        if s.is_empty() || !(2..=16).contains(&radix) {
            return None;
        }
        let mut limbs: Vec<u64> = vec![0]; // base 10^9, least significant first
        for c in s.chars() {
            let mut carry = u64::from(c.to_digit(radix)?);
            for l in &mut limbs {
                let v = *l * u64::from(radix) + carry;
                *l = v % BASE;
                carry = v / BASE;
            }
            if carry > 0 {
                limbs.push(carry);
            }
        }
        let mut digits = String::new();
        let mut it = limbs.iter().rev();
        if let Some(top) = it.next() {
            let _ = write!(digits, "{top}");
        }
        for l in it {
            let _ = write!(digits, "{l:09}");
        }
        Decimal::scaled(false, &digits, "", 0)
    }

    fn negated(self) -> Decimal {
        if self.digits.is_empty() { self } else { Decimal { neg: !self.neg, ..self } }
    }

    fn signum(&self) -> i8 {
        match (self.digits.is_empty(), self.neg) {
            (true, _) => 0,
            (false, true) => -1,
            (false, false) => 1,
        }
    }
}

impl Ord for Decimal {
    fn cmp(&self, o: &Self) -> Ordering {
        let (a, b) = (self.signum(), o.signum());
        if a != b || a == 0 {
            return a.cmp(&b);
        }
        // The position of the leading digit decides; then the digits, which have no trailing zeros.
        let lead = |d: &Decimal| i128::from(d.exp) + i128::try_from(d.digits.len()).unwrap_or(i128::MAX);
        let mag = lead(self).cmp(&lead(o)).then_with(|| self.digits.as_bytes().cmp(o.digits.as_bytes()));
        if a < 0 { mag.reverse() } else { mag }
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// A version (the same for a value and a literal): an optional `v`, `N(.N){0,3}`, an optional
/// `-<pre>` and an optional `+<build>`. Ordered by SemVer 2.0 precedence, the build ignored, missing parts 0.
#[derive(Debug, Clone)]
pub struct Version {
    /// Four numeric parts, leading zeros removed (`""` is 0).
    parts: [Box<str>; 4],
    pre: Vec<Box<str>>,
}

/// Dot-separated identifiers, each `[0-9A-Za-z-]+`.
fn identifiers_ok(s: &str) -> bool {
    s.split('.').all(|i| !i.is_empty() && i.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
}

/// Two runs of ASCII digits with leading zeros removed, compared as numbers.
fn cmp_digits(a: &str, b: &str) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.strip_prefix('v').unwrap_or(s);
        let (s, build) = s.split_once('+').map_or((s, None), |(a, b)| (a, Some(b)));
        let (core, pre) = s.split_once('-').map_or((s, None), |(a, p)| (a, Some(p)));
        if build.is_some_and(|b| !identifiers_ok(b)) || pre.is_some_and(|p| !identifiers_ok(p)) {
            return None;
        }
        let nums: Vec<&str> = core.split('.').collect();
        if nums.len() > 4 || !nums.iter().all(|n| all_digits(n)) {
            return None;
        }
        let mut parts: [Box<str>; 4] = Default::default();
        for (p, n) in parts.iter_mut().zip(&nums) {
            *p = n.trim_start_matches('0').into();
        }
        Some(Version { parts, pre: pre.map(|p| p.split('.').map(Into::into).collect()).unwrap_or_default() })
    }
}

impl Ord for Version {
    fn cmp(&self, o: &Self) -> Ordering {
        let core = self.parts.iter().zip(&o.parts).map(|(a, b)| cmp_digits(a, b)).find(|c| c.is_ne()).unwrap_or(Ordering::Equal);
        core.then_with(|| match (self.pre.is_empty(), o.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                let ident = |a: &str, b: &str| match (all_digits(a), all_digits(b)) {
                    (true, true) => cmp_digits(a.trim_start_matches('0'), b.trim_start_matches('0')),
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    (false, false) => a.cmp(b),
                };
                self.pre.iter().zip(&o.pre).map(|(a, b)| ident(a, b)).find(|c| c.is_ne()).unwrap_or_else(|| self.pre.len().cmp(&o.pre.len()))
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl PartialEq for Version {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}

impl Eq for Version {}

/// A version range: contains `^`, `~`, `||`, `,` or a space; starts with `<`, `>`, `=` or `!`; or has a
/// dot-separated component that is `*`, `x` or `X`.
pub fn is_range(s: &str) -> bool {
    s.contains(['^', '~', ',', ' '])
        || s.contains("||")
        || s.starts_with(['<', '>', '=', '!'])
        || s.split('.').any(|c| matches!(c, "*" | "x" | "X"))
}

/// An anchored scalar, as a resolver reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scalar {
    Number(Decimal),
    Str(String),
    Bool(bool),
    Null,
    Datetime(String),
    Opaque(Unverifiable),
}

impl Scalar {
    /// The bytes this scalar holds on the heap.
    pub(crate) fn heap(&self) -> usize {
        match self {
            Scalar::Number(d) => d.digits.len(),
            Scalar::Str(s) | Scalar::Datetime(s) => s.capacity(),
            Scalar::Bool(_) | Scalar::Null | Scalar::Opaque(_) => 0,
        }
    }
}

/// One YAML version's reading of a plain scalar.
#[derive(Debug, PartialEq)]
enum Reading {
    Null,
    Bool(bool),
    Num(Decimal),
    Inf(bool),
    Nan,
    /// A number whose exponent does not fit an `i64`.
    Unheld,
    /// A 1.1 number that 1.2 never reads as a number (`_`, `0b`, sexagesimal), so not converted.
    Uncounted,
    /// A number written longer than `NUMBER_TEXT_MAX`, not converted.
    Long,
    Str,
}

fn inf(t: &str) -> Option<Reading> {
    let (neg, rest) = sign(t);
    matches!(rest, ".inf" | ".Inf" | ".INF").then_some(Reading::Inf(neg))
}

fn num(long: bool, d: Option<Decimal>) -> Reading {
    if long {
        return Reading::Long;
    }
    d.map_or(Reading::Unheld, Reading::Num)
}

/// YAML 1.2 core schema.
fn read_1_2(t: &str, long: bool) -> Reading {
    match t {
        "" | "~" | "null" | "Null" | "NULL" => return Reading::Null,
        "true" | "True" | "TRUE" => return Reading::Bool(true),
        "false" | "False" | "FALSE" => return Reading::Bool(false),
        ".nan" | ".NaN" | ".NAN" => return Reading::Nan,
        _ => {}
    }
    if let Some(r) = inf(t) {
        return r;
    }
    if let Some(h) = t.strip_prefix("0x").filter(|h| !h.is_empty() && h.bytes().all(|b| b.is_ascii_hexdigit())) {
        return num(long, Decimal::from_radix(h, 16));
    }
    if let Some(o) = t.strip_prefix("0o").filter(|o| !o.is_empty() && o.bytes().all(|b| (b'0'..=b'7').contains(&b))) {
        return num(long, Decimal::from_radix(o, 8));
    }
    if number_parts(t).is_some() {
        return num(long, Decimal::parse_number(t));
    }
    Reading::Str
}

/// `[0-9_]*` with at least `min` bytes, and whether it holds a `_`.
fn digits_us(s: &str, min: usize) -> bool {
    s.len() >= min && s.bytes().all(|b| b.is_ascii_digit() || b == b'_')
}

/// `(:[0-5]?[0-9])+` after the first group.
fn sexagesimal_groups(groups: &[&str]) -> bool {
    !groups.is_empty()
        && groups.iter().all(|g| match g.as_bytes() {
            [d] => d.is_ascii_digit(),
            [a, d] => (b'0'..=b'5').contains(a) && d.is_ascii_digit(),
            _ => false,
        })
}

/// `[eE][-+][0-9]+`, or empty.
fn exp_1_1(e: &str) -> bool {
    e.is_empty() || (e.len() >= 3 && matches!(e.as_bytes()[0], b'e' | b'E') && matches!(e.as_bytes()[1], b'-' | b'+') && all_digits(&e[2..]))
}

/// YAML 1.1 as PyYAML resolves it; the regexes are in `SEMANTICS.md` §Values and comparison.
fn read_1_1(t: &str, long: bool) -> Reading {
    match t {
        "yes" | "Yes" | "YES" | "true" | "True" | "TRUE" | "on" | "On" | "ON" => return Reading::Bool(true),
        "no" | "No" | "NO" | "false" | "False" | "FALSE" | "off" | "Off" | "OFF" => return Reading::Bool(false),
        "" | "~" | "null" | "Null" | "NULL" => return Reading::Null,
        ".nan" | ".NaN" | ".NAN" => return Reading::Nan,
        _ => {}
    }
    if let Some(r) = inf(t) {
        return r;
    }
    let uncounted = if long { Reading::Long } else { Reading::Uncounted };
    let (neg, body) = sign(t);
    let signed = |d: Option<Decimal>| num(long, d.map(|d| if neg { d.negated() } else { d }));
    // int: [-+]?0b[0-1_]+
    if body.strip_prefix("0b").is_some_and(|b| !b.is_empty() && b.bytes().all(|c| matches!(c, b'0' | b'1' | b'_'))) {
        return uncounted;
    }
    // int: [-+]?0x[0-9a-fA-F_]+
    if let Some(h) = body.strip_prefix("0x").filter(|h| !h.is_empty() && h.bytes().all(|c| c.is_ascii_hexdigit() || c == b'_')) {
        return if h.contains('_') { uncounted } else { signed(Decimal::from_radix(h, 16)) };
    }
    // int: [-+]?0[0-7_]+
    if let Some(o) = body.strip_prefix('0').filter(|o| !o.is_empty() && o.bytes().all(|c| matches!(c, b'0'..=b'7' | b'_'))) {
        return if o.contains('_') { uncounted } else { signed(Decimal::from_radix(o, 8)) };
    }
    // int: [-+]?(0|[1-9][0-9_]*)
    if body == "0" || (body.as_bytes().first().is_some_and(|b| (b'1'..=b'9').contains(b)) && digits_us(body, 1)) {
        return if body.contains('_') { uncounted } else { num(long, Decimal::parse_number(t)) };
    }
    // int: [-+]?[1-9][0-9_]*(:[0-5]?[0-9])+
    let groups: Vec<&str> = body.split(':').collect();
    if let [first, rest @ ..] = groups.as_slice()
        && first.as_bytes().first().is_some_and(|b| (b'1'..=b'9').contains(b))
        && digits_us(first, 1)
        && sexagesimal_groups(rest)
    {
        return uncounted;
    }
    // float: [-+]?([0-9][0-9_]*)\.[0-9_]*([eE][-+][0-9]+)?  and  \.[0-9][0-9_]*([eE][-+][0-9]+)?
    if let Some((int, rest)) = body.split_once('.') {
        let e = rest.find(['e', 'E']).unwrap_or(rest.len());
        let (frac, exp) = rest.split_at(e);
        let main = int.as_bytes().first().is_some_and(u8::is_ascii_digit) && digits_us(int, 1) && digits_us(frac, 0);
        let bare = t.starts_with('.') && frac.as_bytes().first().is_some_and(u8::is_ascii_digit) && digits_us(frac, 1);
        if (main || bare) && exp_1_1(exp) {
            return if t.contains('_') { uncounted } else { num(long, Decimal::parse_number(t)) };
        }
        // float: [-+]?[0-9][0-9_]*(:[0-5]?[0-9])+\.[0-9_]*
        let groups: Vec<&str> = int.split(':').collect();
        if let [first, rest @ ..] = groups.as_slice()
            && first.as_bytes().first().is_some_and(u8::is_ascii_digit)
            && digits_us(first, 1)
            && sexagesimal_groups(rest)
            && digits_us(&body[int.len() + 1..], 0)
        {
            return uncounted;
        }
    }
    Reading::Str
}

/// A plain YAML scalar: the 1.2 core reading, unless YAML 1.1 reads it differently or it is `y`, `n`,
/// `yes`, `no`, `on` or `off` in any case (`unverifiable: yaml-1.1`). A number written in more than
/// `NUMBER_TEXT_MAX` bytes is `unverifiable: bound:number-text`. Quoted and block scalars are strings and
/// never come here.
pub fn yaml_plain(text: &str) -> Scalar {
    let long = text.len() > NUMBER_TEXT_MAX;
    let (a, b) = (read_1_2(text, long), read_1_1(text, long));
    if a == Reading::Long || b == Reading::Long {
        return Scalar::Opaque(Unverifiable::Bound(Bound::NumberText));
    }
    if ["y", "n", "yes", "no", "on", "off"].iter().any(|w| text.eq_ignore_ascii_case(w)) || a != b {
        return Scalar::Opaque(Unverifiable::Yaml11);
    }
    match a {
        Reading::Null => Scalar::Null,
        Reading::Bool(v) => Scalar::Bool(v),
        Reading::Num(d) => Scalar::Number(d),
        Reading::Str => Scalar::Str(text.to_owned()),
        Reading::Inf(_) | Reading::Nan | Reading::Unheld => Scalar::Opaque(Unverifiable::NotComparable),
        Reading::Uncounted | Reading::Long => Scalar::Opaque(Unverifiable::Yaml11),
    }
}

fn holds(op: Op, o: Ordering) -> bool {
    match op {
        Op::Eq => o.is_eq(),
        Op::Ne => o.is_ne(),
        Op::Ge => o.is_ge(),
        Op::Gt => o.is_gt(),
        Op::Le => o.is_le(),
        Op::Lt => o.is_lt(),
    }
}

/// `op literal` against an anchored scalar (`SEMANTICS.md` §Values and comparison and its table; the readings rule).
pub fn compare(value: &Scalar, op: Op, literal: &str) -> Result<bool, Unverifiable> {
    let ordered = !matches!(op, Op::Eq | Op::Ne);
    let equal = |e: bool| if matches!(op, Op::Ne) { !e } else { e };
    match value {
        Scalar::Opaque(u) => Err(*u),
        Scalar::Number(d) => Decimal::parse_plain(literal).map(|l| holds(op, d.cmp(&l))).ok_or(Unverifiable::NotComparable),
        Scalar::Str(s) if ordered => readings(s, op, literal),
        Scalar::Str(s) => Ok(equal(s == literal)),
        Scalar::Bool(_) | Scalar::Null | Scalar::Datetime(_) if ordered => Err(Unverifiable::NotComparable),
        Scalar::Bool(b) => match literal {
            "true" => Ok(equal(*b)),
            "false" => Ok(equal(!*b)),
            _ => Err(Unverifiable::NotComparable),
        },
        Scalar::Null => Ok(equal(literal == "null")),
        Scalar::Datetime(t) => Ok(equal(t == literal)),
    }
}

/// The readings rule: an ordered comparison of a string, as decimals and as versions.
fn readings(s: &str, op: Op, literal: &str) -> Result<bool, Unverifiable> {
    if is_range(s) {
        return Err(Unverifiable::Range);
    }
    let as_decimal = Decimal::parse_plain(s).zip(Decimal::parse_plain(literal)).map(|(a, b)| holds(op, a.cmp(&b)));
    let as_version = Version::parse(s).zip(Version::parse(literal)).map(|(a, b)| holds(op, a.cmp(&b)));
    match (as_decimal, as_version) {
        (None, None) => Err(Unverifiable::NotComparable),
        (Some(d), Some(v)) if d != v => Err(Unverifiable::NumberOrVersion),
        (Some(r), _) | (None, Some(r)) => Ok(r),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reason::Bound;
    use proptest::prelude::*;

    #[test]
    fn decimal_display() {
        let shown = |t: &str| Decimal::parse_number(t).unwrap().to_string();
        let table = [
            ("0", "0"),
            ("-0", "0"),
            ("0.000", "0"),
            ("1.50", "1.5"),
            ("-0.25", "-0.25"),
            ("100", "100"),
            ("0.001", "0.001"),
            ("123456.789", "123456.789"),
            ("1e400", "1e400"),
            ("-1.5e-70", "-1.5e-70"),
            ("1e63", "1000000000000000000000000000000000000000000000000000000000000000"),
            ("1e64", "1e64"),
            ("1e-62", "0.00000000000000000000000000000000000000000000000000000000000001"),
            ("1e-63", "1e-63"),
            ("12e-1", "1.2"),
        ];
        for (input, want) in table {
            assert_eq!(shown(input), want, "{input}");
            // What is shown reads back as the same number, whatever form it takes.
            assert_eq!(Decimal::parse_number(want), Decimal::parse_number(input), "{input}");
        }
        // 65 digits with no exponent are past the plain form.
        let ones = "1".repeat(65);
        assert_eq!(shown(&ones), format!("1.{}e64", "1".repeat(64)));
        assert_eq!(Decimal::parse_number(&shown(&ones)), Decimal::parse_number(&ones));
    }

    fn dec(s: &str) -> Decimal {
        Decimal::parse_number(s).unwrap_or_else(|| panic!("not a number: {s}"))
    }
    fn n(s: &str) -> Scalar {
        Scalar::Number(dec(s))
    }
    fn s(t: &str) -> Scalar {
        Scalar::Str(t.to_owned())
    }
    fn ver(t: &str) -> Version {
        Version::parse(t).unwrap_or_else(|| panic!("not a version: {t}"))
    }

    #[test]
    fn spec_table_rows() {
        assert_eq!(compare(&s("-1"), Op::Eq, "-1"), Ok(true));
        assert_eq!(compare(&s("0.8"), Op::Lt, "0.9"), Ok(true));
        assert_eq!(compare(&s("0.9.1"), Op::Lt, "0.9"), Ok(false));
        assert_eq!(compare(&s("3.9"), Op::Ge, "3.10"), Err(Unverifiable::NumberOrVersion));
        assert_eq!(compare(&s("3.9"), Op::Ge, "v3.10"), Ok(false));
        assert_eq!(compare(&s("0.5"), Op::Le, "0.25"), Err(Unverifiable::NumberOrVersion));
        assert_eq!(compare(&n("2"), Op::Ge, "2"), Ok(true));
        assert_eq!(compare(&n("0.5"), Op::Ge, "0.25"), Ok(true));
        assert_eq!(compare(&Scalar::Bool(true), Op::Eq, "yes"), Err(Unverifiable::NotComparable));
    }

    #[test]
    fn the_expectation_table() {
        // A number: exact decimal under == and !=; a literal that is not a decimal is not comparable.
        assert_eq!(compare(&n("2"), Op::Ne, "3"), Ok(true));
        assert_eq!(compare(&n("2"), Op::Eq, "abc"), Err(Unverifiable::NotComparable));
        assert_eq!(compare(&n("2"), Op::Ge, "v1"), Err(Unverifiable::NotComparable)); // never a version
        assert_eq!(compare(&n("-0"), Op::Eq, "0"), Ok(true));
        assert_eq!(compare(&n("7"), Op::Eq, "007"), Ok(true));
        // A string: exact text under == and !=.
        assert_eq!(compare(&s("0.50"), Op::Eq, "0.5"), Ok(false));
        assert_eq!(compare(&s("abc"), Op::Ne, "abd"), Ok(true));
        // A boolean: the literal is true or false; ordered is not comparable.
        assert_eq!(compare(&Scalar::Bool(true), Op::Eq, "true"), Ok(true));
        assert_eq!(compare(&Scalar::Bool(false), Op::Ne, "true"), Ok(true));
        assert_eq!(compare(&Scalar::Bool(true), Op::Eq, "True"), Err(Unverifiable::NotComparable));
        assert_eq!(compare(&Scalar::Bool(true), Op::Ge, "true"), Err(Unverifiable::NotComparable));
        // null: equal only to null; ordered is not comparable.
        assert_eq!(compare(&Scalar::Null, Op::Ne, "null"), Ok(false));
        assert_eq!(compare(&Scalar::Null, Op::Ne, "~"), Ok(true));
        assert_eq!(compare(&Scalar::Null, Op::Lt, "1"), Err(Unverifiable::NotComparable));
        // An opaque value keeps its reason.
        assert_eq!(compare(&Scalar::Opaque(Unverifiable::Yaml11), Op::Eq, "yes"), Err(Unverifiable::Yaml11));
    }

    #[test]
    fn exact_decimals() {
        assert_eq!(compare(&n("3.10"), Op::Eq, "3.1"), Ok(true));
        assert_eq!(compare(&n("12345678901234567890123"), Op::Gt, "12345678901234567890122"), Ok(true));
        assert_eq!(compare(&n("1e5"), Op::Eq, "100000"), Ok(true));
        assert_eq!(compare(&n("1.5E-3"), Op::Eq, "0.0015"), Ok(true));
        assert_eq!(compare(&n("-.5"), Op::Lt, "-0.4"), Ok(true));
        assert_eq!(compare(&n("+1."), Op::Eq, "1"), Ok(true));
        assert_eq!(compare(&n("0.000"), Op::Eq, "-0.0"), Ok(true));
        assert_eq!(compare(&n("1e400"), Op::Gt, "999999999999999999999999999999"), Ok(true));
        assert!(Decimal::parse_number("1e99999999999999999999").is_none(), "an exponent past i64 is not held");
    }

    #[test]
    fn decimal_grammars() {
        for ok in ["0", "-0", "007", "1.50", "-12.0", "123456789012345678901234567890"] {
            assert!(Decimal::parse_plain(ok).is_some(), "{ok}");
        }
        for bad in ["", "-", "+1", "1.", ".5", "1e5", "1.2.3", " 1", "1 ", "0x1F", "١", "1_000", "--1"] {
            assert!(Decimal::parse_plain(bad).is_none(), "{bad:?}");
        }
        for ok in ["1", "+1", "-1", "1.", ".5", "1.5e3", "1E+3", "1e-3", "-0.0e0"] {
            assert!(Decimal::parse_number(ok).is_some(), "{ok}");
        }
        for bad in ["", ".", "e3", "1e", "1e+", "0x1F", "1_0", "1.2.3", "inf", ".inf"] {
            assert!(Decimal::parse_number(bad).is_none(), "{bad:?}");
        }
        assert_eq!(dec("1.50"), dec("15e-1"));
        assert_eq!(dec("0"), dec("-0.000"));
    }

    #[test]
    fn radix_numbers_convert_exactly() {
        assert_eq!(Decimal::from_radix("1F", 16), Some(dec("31")));
        assert_eq!(Decimal::from_radix("1f", 16), Some(dec("31")));
        assert_eq!(Decimal::from_radix("17", 8), Some(dec("15")));
        assert_eq!(Decimal::from_radix("101", 2), Some(dec("5")));
        assert_eq!(Decimal::from_radix("0000", 16), Some(dec("0")));
        assert_eq!(Decimal::from_radix("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF", 16), Some(dec("340282366920938463463374607431768211455")));
        assert_eq!(Decimal::from_radix("1000000000000000000000", 16), Some(dec("19342813113834066795298816")));
        for bad in [("", 16), ("8", 8), ("2", 2), ("g", 16), ("-1", 16), ("1_0", 16)] {
            assert_eq!(Decimal::from_radix(bad.0, bad.1), None, "{bad:?}");
        }
    }

    #[test]
    fn literal_not_decimal_on_number() {
        assert_eq!(compare(&n("5"), Op::Eq, "1e5"), Err(Unverifiable::NotComparable));
    }

    #[test]
    fn ranges() {
        for v in ["^1.2", "~1.2", ">=1.2", "1.x", "1.2.*", "1 - 2", "1,2", "1 || 2", "<2", "=1.2", "!1", "X", "*"] {
            assert!(is_range(v), "{v}");
            assert_eq!(compare(&s(v), Op::Ge, "1.0"), Err(Unverifiable::Range), "{v}");
        }
        for v in ["1.2", "v1.2.3", "1.0.0-rc.1", "abc", "1.2x", "x1"] {
            assert!(!is_range(v), "{v}");
        }
        // == and != are text, so a range compares as text.
        assert_eq!(compare(&s("^1.2"), Op::Eq, "^1.2"), Ok(true));
    }

    #[test]
    fn version_grammar() {
        for ok in ["1", "v1", "1.2.3.4", "01.2", "1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-0.3.7", "1.0.0-x-y-z.--", "1.0.0+20130313144700", "1.0.0-beta+exp.sha.5114f85", "v1.29.3"] {
            assert!(Version::parse(ok).is_some(), "{ok}");
        }
        for bad in ["", "v", "V1", "1.2.3.4.5", "1.", ".1", "1..2", "1.2.3-", "1.2.3+", "1.2.3-a..b", "1.2.3-a_b", "-1", "1.2a", "vv1", " 1", "١.٢"] {
            assert!(Version::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn semver_precedence() {
        assert_eq!(compare(&s("1.0.0-alpha"), Op::Lt, "v1.0.0"), Ok(true));
        assert_eq!(compare(&s("1.2"), Op::Eq, "1.2.0"), Ok(false)); // == is text on a string
        assert_eq!(compare(&s("1.2"), Op::Ge, "v1.2.0"), Ok(true));
        // SemVer 2.0 §11: 1.0.0-alpha < -alpha.1 < -alpha.beta < -beta < -beta.2 < -beta.11 < -rc.1 < 1.0.0.
        let chain = ["1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2", "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0"];
        for w in chain.windows(2) {
            assert!(ver(w[0]) < ver(w[1]), "{} < {}", w[0], w[1]);
        }
        assert_eq!(ver("1.2"), ver("1.2.0.0"));
        assert_ne!(ver("1.2"), ver("1.3"));
        assert_ne!(ver("1.0.0-alpha"), ver("1.0.0"));
        assert_eq!(ver("1.0.0+a"), ver("1.0.0+b")); // build ignored
        assert_eq!(ver("01.2"), ver("1.2"));
        assert!(ver("1.99999999999999999999999") > ver("1.99999999999999999999998"));
        assert!(ver("1.0.0-2") < ver("1.0.0-10")); // numeric identifiers compare numerically
        assert!(ver("1.0.0-999") < ver("1.0.0-a")); // numeric before alphanumeric
    }

    #[test]
    fn v_values_are_versions() {
        assert_eq!(compare(&s("v1.29.3"), Op::Ge, "v1.28"), Ok(true));
        assert_eq!(compare(&s("v1.29.3"), Op::Ge, "1.28"), Ok(true));
        assert_eq!(compare(&s("v3.9"), Op::Ge, "3.10"), Ok(false));
        assert_eq!(compare(&s("v1.29.3"), Op::Eq, "1.29.3"), Ok(false)); // == is text
    }

    #[test]
    fn version_vs_non_version() {
        assert_eq!(compare(&s("abc"), Op::Gt, "1.0"), Err(Unverifiable::NotComparable));
        assert_eq!(compare(&s("1.0"), Op::Gt, "abc"), Err(Unverifiable::NotComparable));
        assert_eq!(compare(&s(""), Op::Gt, "1"), Err(Unverifiable::NotComparable));
        assert_eq!(compare(&s("-1"), Op::Lt, "0"), Ok(true)); // only the decimal reading applies
        assert_eq!(compare(&s("1.0-rc"), Op::Lt, "1.0"), Ok(true)); // only the version reading applies
    }

    #[test]
    fn null_and_datetime() {
        assert_eq!(compare(&Scalar::Null, Op::Eq, "null"), Ok(true));
        assert_eq!(compare(&Scalar::Null, Op::Eq, "0"), Ok(false));
        assert_eq!(compare(&Scalar::Datetime("1979-05-27T07:32:00Z".into()), Op::Lt, "2000"), Err(Unverifiable::NotComparable));
        assert_eq!(compare(&Scalar::Datetime("1979-05-27T07:32:00Z".into()), Op::Eq, "1979-05-27T07:32:00Z"), Ok(true));
        assert_eq!(compare(&Scalar::Datetime("1979-05-27".into()), Op::Ne, "1979-05-28"), Ok(true));
    }

    #[test]
    fn yaml_1_1_differences() {
        for t in ["yes", "No", "ON", "y", "yEs", "0755", "010", "08", "1:30", "0b101", "1_000", "0o17", "1e5", "-0x1F", "off", "N", "+.5", "1.5e3", "190:20:30.15", "+0x1F", "1__0"] {
            assert_eq!(yaml_plain(t), Scalar::Opaque(Unverifiable::Yaml11), "{t}");
        }
        assert_eq!(yaml_plain("0x1F"), Scalar::Number(dec("31"))); // edge case 58
        assert_eq!(yaml_plain("1.2.3"), Scalar::Str("1.2.3".into())); // a plain chart version is text, not yaml-1.1
        assert_eq!(yaml_plain("true"), Scalar::Bool(true));
        assert_eq!(yaml_plain("FALSE"), Scalar::Bool(false));
        assert_eq!(yaml_plain("00"), Scalar::Number(dec("0"))); // same value in both
        assert_eq!(yaml_plain("07"), Scalar::Number(dec("7")));
        assert_eq!(yaml_plain(".inf"), Scalar::Opaque(Unverifiable::NotComparable));
        assert_eq!(yaml_plain("-.INF"), Scalar::Opaque(Unverifiable::NotComparable));
        assert_eq!(yaml_plain(".NaN"), Scalar::Opaque(Unverifiable::NotComparable));
        assert_eq!(yaml_plain("2001-12-14"), Scalar::Str("2001-12-14".into()));
        assert_eq!(yaml_plain("2001-12-14t21:59:43.10-05:00"), Scalar::Str("2001-12-14t21:59:43.10-05:00".into()));
    }

    #[test]
    fn yaml_edges_from_the_attack_pass() {
        let num = |t: &str, v: &str| assert_eq!(yaml_plain(t), Scalar::Number(dec(v)), "{t}");
        let y11 = |t: &str| assert_eq!(yaml_plain(t), Scalar::Opaque(Unverifiable::Yaml11), "{t}");
        let text = |t: &str| assert_eq!(yaml_plain(t), Scalar::Str(t.to_owned()), "{t}");
        // Both versions read the same number.
        for (t, v) in [("+0", "0"), ("0.0", "0"), ("00.5", "0.5"), ("1.e+5", "100000"), ("1.0e+5", "100000"), ("+1", "1"), ("-12", "-12"), ("0x0", "0"), ("0xff", "255")] {
            num(t, v);
        }
        // Only 1.1 reads a number, or the two read different values.
        for t in ["12:30", "-1:30", "0_", "0x_", "0x1_F", "1_0.5", "+0755", "1.0e5", "0b", "0b_"] {
            if t == "0b" { text(t) } else { y11(t) }
        }
        // Text in both.
        for t in ["0:30", "1.5e", "._5", "1.2.3.4", "0b2", "e5", "1e+", "1:60", "1:30:"] {
            text(t);
        }
        // An exponent past i64 is not held; with a 1.1-style signed exponent both versions agree it is a float.
        assert_eq!(yaml_plain("1.0e+99999999999999999999"), Scalar::Opaque(Unverifiable::NotComparable));
        assert_eq!(yaml_plain("1e9223372036854775807"), Scalar::Opaque(Unverifiable::Yaml11));
        assert!(matches!(compare(&n("1e9223372036854775807"), Op::Gt, "1"), Ok(true)));
        // A range rule quirk `SEMANTICS.md` §Values and comparison states literally: a whole component `x`, even in a prerelease.
        assert!(is_range("1.0.0-alpha.x"));
    }

    /// `tools/gen_yaml_plain.py`: PyYAML's own 1.1 resolver against the 1.2 core regexes, over 6,749 plain
    /// scalars. A disagreement here is a fault in the hand-written 1.1 or 1.2 matcher.
    #[test]
    fn yaml_plain_agrees_with_pyyaml() {
        let fixture = include_str!("../tests/fixtures/yaml-plain.jsonl");
        let show = |sc: &Scalar| match sc {
            Scalar::Null => "Null".to_owned(),
            Scalar::Bool(b) => format!("Bool:{b}"),
            Scalar::Str(_) => "Str".to_owned(),
            Scalar::Number(d) => format!("Number:{}{}e{}", if d.neg { "-" } else { "" }, d.digits, d.exp),
            Scalar::Opaque(Unverifiable::Yaml11) => "Yaml11".to_owned(),
            Scalar::Opaque(Unverifiable::NotComparable) => "NotComparable".to_owned(),
            other => format!("{other:?}"),
        };
        let mut bad = Vec::new();
        let mut total = 0;
        for line in fixture.lines() {
            let row: serde_json::Value = serde_json::from_str(line).unwrap();
            let (text, want) = (row["text"].as_str().unwrap(), row["expect"].as_str().unwrap());
            total += 1;
            let got = show(&yaml_plain(text));
            if got != want {
                bad.push(format!("{text:?}: want {want}, got {got}"));
            }
        }
        assert!(total > 6000, "the fixture is cut short: {total} rows");
        assert!(bad.is_empty(), "{} of {total} disagree:\n{}", bad.len(), bad.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
    }

    #[test]
    fn yaml_1_2_core_resolution() {
        for t in ["", "~", "null", "Null", "NULL"] {
            assert_eq!(yaml_plain(t), Scalar::Null, "{t:?}");
        }
        for (t, v) in [("0", "0"), ("-0", "0"), ("+12", "12"), ("1.5", "1.5"), ("1.", "1"), (".5", "0.5"), ("1.0e+5", "100000"), ("-1", "-1"), ("12345678901234567890123", "12345678901234567890123")] {
            assert_eq!(yaml_plain(t), Scalar::Number(dec(v)), "{t}");
        }
        for t in ["nULL", "tRUE", "0x", "0o", "0o8", "0X1F", "1.2.3", "abc", "0:30", "١٢", "1e", "v1.2", ". 5"] {
            assert_eq!(yaml_plain(t), Scalar::Str(t.to_owned()), "{t}");
        }
    }

    #[test]
    fn a_long_number_is_a_bound() {
        let long_int = "1".repeat(crate::bounds::NUMBER_TEXT_MAX + 1);
        let long_hex = format!("0x{}", "f".repeat(crate::bounds::NUMBER_TEXT_MAX));
        // A number in YAML 1.2 only (`0o`), written long: the bound, not the version difference.
        let long_oct = format!("0o{}", "7".repeat(crate::bounds::NUMBER_TEXT_MAX));
        for t in [long_int.as_str(), long_hex.as_str(), long_oct.as_str()] {
            assert_eq!(yaml_plain(t), Scalar::Opaque(Unverifiable::Bound(Bound::NumberText)), "{}", t.len());
        }
        let at_cap = "1".repeat(crate::bounds::NUMBER_TEXT_MAX);
        assert!(matches!(yaml_plain(&at_cap), Scalar::Number(_)));
        let long_text = "a".repeat(crate::bounds::NUMBER_TEXT_MAX + 1);
        assert_eq!(yaml_plain(&long_text), Scalar::Str(long_text.clone())); // not a number: no bound
    }

    fn plain(a: i64, scale: u32) -> String {
        let digits = a.unsigned_abs().to_string();
        let sign = if a < 0 { "-" } else { "" };
        if scale == 0 {
            return format!("{sign}{digits}");
        }
        let padded = format!("{digits:0>width$}", width = scale as usize + 1);
        let (int, frac) = padded.split_at(padded.len() - scale as usize);
        format!("{sign}{int}.{frac}")
    }

    proptest! {
        #[test]
        fn decimal_order_matches_f64_where_exact(a in -1_000_000i64..1_000_000, b in -1_000_000i64..1_000_000, sa in 0u32..4, sb in 0u32..4) {
            let (da, db) = (Decimal::parse_plain(&plain(a, sa)).unwrap(), Decimal::parse_plain(&plain(b, sb)).unwrap());
            let want = (i128::from(a) * 10i128.pow(sb)).cmp(&(i128::from(b) * 10i128.pow(sa)));
            prop_assert_eq!(da.cmp(&db), want, "{} vs {}", plain(a, sa), plain(b, sb));
            prop_assert_eq!(da.partial_cmp(&db), Some(want));
            prop_assert_eq!(da == db, want == Ordering::Equal);
        }

        #[test]
        fn readings_never_pass_silently(v in "[0-9]{1,2}\\.[0-9]{1,3}", l in "[0-9]{1,2}\\.[0-9]{1,3}", op in 0usize..4) {
            let op = [Op::Ge, Op::Gt, Op::Le, Op::Lt][op];
            let holds = |o: Ordering| match op { Op::Ge => o.is_ge(), Op::Gt => o.is_gt(), Op::Le => o.is_le(), _ => o.is_lt() };
            let as_dec = holds(Decimal::parse_plain(&v).unwrap().cmp(&Decimal::parse_plain(&l).unwrap()));
            let as_ver = holds(ver(&v).cmp(&ver(&l)));
            let want = if as_dec == as_ver { Ok(as_dec) } else { Err(Unverifiable::NumberOrVersion) };
            prop_assert_eq!(compare(&s(&v), op, &l), want);
        }

        #[test]
        fn radix_round_trip(x in any::<u128>()) {
            let d = Decimal::parse_plain(&x.to_string()).unwrap();
            prop_assert_eq!(Decimal::from_radix(&format!("{x:x}"), 16), Some(d.clone()));
            prop_assert_eq!(Decimal::from_radix(&format!("{x:o}"), 8), Some(d.clone()));
            prop_assert_eq!(Decimal::from_radix(&format!("{x:b}"), 2), Some(d));
        }

        #[test]
        fn number_and_plain_agree(a in any::<i64>(), scale in 0u32..19, e in -40i64..40) {
            // One value, written with a fractional mantissa and with an integer one, is one Decimal.
            let frac = Decimal::parse_number(&format!("{}e{e}", plain(a, scale))).unwrap();
            let int = Decimal::parse_number(&format!("{a}e{}", e - i64::from(scale))).unwrap();
            prop_assert_eq!(&frac, &int);
            prop_assert_eq!(Decimal::parse_number(&plain(a, scale)), Decimal::parse_plain(&plain(a, scale)));
        }
    }
}
