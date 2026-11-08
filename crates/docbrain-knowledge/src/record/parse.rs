// SPDX-License-Identifier: MIT
//! `parse_record`: the strict reading of one record file (`SEMANTICS.md` §The record, §States,
//! §Hidden text, §Paths and globs, §Bounds). The steps run in a fixed order and
//! the first that fails is the reason; within one step, the first in the file.

use std::collections::HashSet;

use crate::bounds::{CHECKS_MAX, FRONT_MATTER_MAX, RECORD_FILE_MAX};
use crate::charset::hidden_text;
use crate::path::{RepoGlob, parse_rel};
use crate::reason::{Bound, Invalid};
use crate::yaml::{Mode, Node, NodeKind, Style, YamlErrorKind, read_yaml};

use super::{Anchor, Check, Expectation, Id, KeyPath, Kind, Layer, Parsed, Record, Verb, parse_key_path};

/// Line and column from 1, the column in Unicode scalar values.
type Pos = (u32, u32);

/// Conditions of the whole file or of the whole front matter are reported here.
const START: Pos = (1, 1);

struct Fault(Invalid, Pos);

type Res<T> = Result<T, Fault>;

const TOP_KEYS: [&str; 7] = ["knowledge", "id", "kind", "title", "applies_to", "checks", "supersedes"];
const REQUIRED: [&str; 4] = ["knowledge", "id", "kind", "title"];
const CHECK_KEYS: [&str; 6] = ["path", "config_key", "code", "text", "must", "assumes"];

/// Reads one record file of the folder whose parent directory is `scope` (`""` at the root).
pub fn parse_record(scope: &str, bytes: &[u8]) -> Parsed {
    match read(scope, bytes) {
        Ok(p) => p,
        Err(Fault(reason, (line, col))) => Parsed::Invalid { reason, line, col },
    }
}

fn read(scope: &str, bytes: &[u8]) -> Res<Parsed> {
    // 1-4: the file.
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > RECORD_FILE_MAX {
        return Err(Fault(Invalid::Bound(Bound::RecordFile), START));
    }
    let raw = std::str::from_utf8(bytes).map_err(|e| Fault(Invalid::Parse, utf8_pos(bytes, e.valid_up_to())))?;
    if let Some((line, col, _)) = hidden_text(raw) {
        return Err(Fault(Invalid::HiddenText, (line, col)));
    }
    // No lone CR is left (it is hidden text), so this removes every CR.
    let text = raw.strip_prefix('\u{FEFF}').unwrap_or(raw).replace("\r\n", "\n");
    if let Some(line) = conflict_marker(&text) {
        return Err(Fault(Invalid::ConflictMarkers, (line, 1)));
    }
    // 5-7: the front matter.
    let (fm, body, body_line) = split(&text).ok_or(Fault(Invalid::Parse, START))?;
    if fm.len() > FRONT_MATTER_MAX {
        return Err(Fault(Invalid::Bound(Bound::FrontMatter), START));
    }
    let docs = read_yaml(fm, Mode::Strict).map_err(|e| match e.kind {
        YamlErrorKind::Syntax => syntax_fault(fm, e.line, e.col),
        YamlErrorKind::Depth => Fault(Invalid::Bound(Bound::Depth), (e.line.saturating_add(1), e.col)),
        YamlErrorKind::Feature | YamlErrorKind::AliasBudget => Fault(Invalid::YamlFeature, (e.line.saturating_add(1), e.col)),
    })?;
    let [root] = docs.as_slice() else { return Err(Fault(Invalid::Parse, START)) };
    let NodeKind::Map(top) = &root.kind else { return Err(Fault(Invalid::Parse, START)) };
    if let Some(p) = first_key(root, &|k| !matches!(k.kind, NodeKind::Scalar { .. })) {
        return Err(Fault(Invalid::Parse, p));
    }
    // 8-10: duplicates, merge keys, quoting.
    if let Some(p) = first_duplicate(root) {
        return Err(Fault(Invalid::DuplicateKey, p));
    }
    if let Some(p) = first_key(root, &|k| text_of(k) == "<<") {
        return Err(Fault(Invalid::YamlFeature, p));
    }
    if let Some(p) = first_unquoted(root, true) {
        return Err(Fault(Invalid::Quotes, p));
    }
    // 11: the schema.
    if let Some(v) = lookup(top, "knowledge") {
        let schema = schema_integer(v).ok_or(Fault(Invalid::SchemaInteger, at(v)))?;
        if schema > 1 {
            return Ok(newer(scope, schema, top));
        }
    }
    // 12-14: the required keys, then their values.
    for name in REQUIRED {
        if lookup(top, name).is_none() {
            return Err(Fault(Invalid::MissingKey(name), START));
        }
    }
    let id_node = need(top, "id")?;
    let id = Id::parse(string(id_node, "id")?).ok_or(Fault(Invalid::Id, at(id_node)))?;
    if id.is_example() {
        return Err(Fault(Invalid::ExampleId, at(id_node)));
    }
    let kind_node = need(top, "kind")?;
    let kind = Kind::parse(string(kind_node, "kind")?).ok_or(Fault(Invalid::Kind, at(kind_node)))?;
    let title_node = need(top, "title")?;
    let title = string(title_node, "title")?;
    if !title_ok(title) {
        return Err(Fault(Invalid::Title, at(title_node)));
    }
    // 15: applies_to is a list of strings.
    let applies = match lookup(top, "applies_to") {
        None => None,
        Some(v) => Some(strings(v).map_err(|p| Fault(Invalid::Type("applies_to"), p))?),
    };
    let mut unknown: Vec<(Pos, String)> = top
        .iter()
        .map(|(k, _)| k)
        .filter(|k| !TOP_KEYS.contains(&text_of(k)) && !text_of(k).starts_with("x-"))
        .map(|k| (at(k), text_of(k).to_owned()))
        .collect();
    // 16-17: checks, then each check's keys.
    let raw = match lookup(top, "checks") {
        None => Vec::new(),
        Some(v) => {
            let NodeKind::Seq(items) = &v.kind else { return Err(Fault(Invalid::Type("checks"), at(v))) };
            if let Some(over) = items.get(CHECKS_MAX) {
                return Err(Fault(Invalid::Bound(Bound::Checks), at(over)));
            }
            items.iter().enumerate().map(|(i, c)| raw_check(i, c, &mut unknown)).collect::<Res<Vec<_>>>()?
        }
    };
    // 18: key paths.
    let mut keys = Vec::with_capacity(raw.len());
    for c in &raw {
        let mut layer_keys = Vec::new();
        if let RawAnchor::ConfigKey(layers) = &c.anchor {
            for &(s, p) in layers {
                let (_, key) = s.split_once('#').ok_or(Fault(Invalid::KeyPath, p))?;
                layer_keys.push(parse_key_path(key).map_err(|_| Fault(Invalid::KeyPath, p))?);
            }
        }
        keys.push(layer_keys);
    }
    // 19: every path stays in the scope, in file order.
    let mut paths: Vec<(Pos, &str)> = applies.iter().flatten().map(|&(s, p)| (p, s)).collect();
    for c in &raw {
        match &c.anchor {
            RawAnchor::Path(s, p) | RawAnchor::Code(s, p, _) => paths.push((*p, s)),
            RawAnchor::ConfigKey(layers) => paths.extend(layers.iter().map(|&(s, p)| (p, file_part(s)))),
            RawAnchor::Unknown(_) => {}
        }
    }
    paths.sort_by_key(|&(p, _)| p);
    for &(p, s) in &paths {
        rel(scope, s, p)?;
    }
    // 20: a config_key file glob stays within one directory.
    for c in &raw {
        if let RawAnchor::ConfigKey(layers) = &c.anchor
            && let Some(&(_, p)) = layers.iter().find(|&&(s, _)| glob_dir(file_part(s)))
        {
            return Err(Fault(Invalid::GlobDir, p));
        }
    }
    // 21: supersedes.
    let supersedes = match lookup(top, "supersedes") {
        None => None,
        Some(v) => Some(Id::parse(string(v, "supersedes")?).ok_or(Fault(Invalid::Id, at(v)))?),
    };
    // 22: the body.
    if body.chars().all(|c| matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' ')) {
        return Err(Fault(Invalid::EmptyBody, (body_line, 1)));
    }
    // Every step passed: build the record.
    let mut checks = Vec::with_capacity(raw.len());
    for (c, layer_keys) in raw.iter().zip(keys) {
        let anchor = match &c.anchor {
            RawAnchor::Path(s, p) => Anchor::Path(rel(scope, s, *p)?),
            RawAnchor::ConfigKey(layers) => Anchor::ConfigKey(
                layers
                    .iter()
                    .zip(layer_keys)
                    .map(|(&(s, p), key): (&(&str, Pos), KeyPath)| Ok(Layer { file: rel(scope, file_part(s), p)?, key }))
                    .collect::<Res<_>>()?,
            ),
            RawAnchor::Code(f, p, t) => Anchor::Code { file: rel(scope, f, *p)?, text: (*t).to_owned() },
            RawAnchor::Unknown(k) => Anchor::Unknown(k.clone()),
        };
        checks.push(Check { anchor, verb: c.verb, expect: Expectation::parse(c.expect), line: c.pos.0 });
    }
    let (applies_to, applies_written) = match &applies {
        Some(list) => (list.iter().map(|&(s, p)| rel(scope, s, p)).collect::<Res<_>>()?, true),
        None => (default_applies_to(scope, &checks), false),
    };
    unknown.sort_by_key(|(p, _)| *p);
    Ok(Parsed::Valid(Record {
        id,
        kind,
        title: title.to_owned(),
        applies_to,
        applies_written,
        checks,
        supersedes,
        body: body.to_owned(),
        unknown_keys: unknown.into_iter().map(|(_, k)| k).collect(),
    }))
}

/// One check after its keys are read, before its paths and key paths are.
struct RawCheck<'a> {
    pos: Pos,
    verb: Verb,
    expect: &'a str,
    anchor: RawAnchor<'a>,
}

enum RawAnchor<'a> {
    Path(&'a str, Pos),
    ConfigKey(Vec<(&'a str, Pos)>),
    Code(&'a str, Pos, &'a str),
    Unknown(String),
}

/// exactly one verb; at most one anchor; `text` with `code` only, and `code` needs `text`; no anchor
/// but another non-`x-` key is an unknown check kind; then each known key's value has its type.
fn raw_check<'a>(i: usize, c: &'a Node, unknown: &mut Vec<(Pos, String)>) -> Res<RawCheck<'a>> {
    let pos = at(c);
    let NodeKind::Map(es) = &c.kind else { return Err(Fault(Invalid::Type("checks"), pos)) };
    let verbs: Vec<(Verb, &Node)> =
        [("must", Verb::Must), ("assumes", Verb::Assumes)].into_iter().filter_map(|(n, v)| lookup(es, n).map(|node| (v, node))).collect();
    let [(verb, verb_node)] = verbs.as_slice() else { return Err(Fault(Invalid::Verbs, pos)) };
    let anchors: Vec<&str> = ["path", "config_key", "code"].into_iter().filter(|n| lookup(es, n).is_some()).collect();
    let extra: Vec<&Node> = es.iter().map(|(k, _)| k).filter(|k| known_check_key(text_of(k)).is_none() && !text_of(k).starts_with("x-")).collect();
    let text = lookup(es, "text");
    match (anchors.as_slice(), text.is_some()) {
        ([], _) => {
            let Some(k) = extra.first() else { return Err(Fault(Invalid::Anchor, pos)) };
            let expect = string(verb_node, verb.as_str())?;
            return Ok(RawCheck { pos, verb: *verb, expect, anchor: RawAnchor::Unknown(text_of(k).to_owned()) });
        }
        ([_, _, ..], _) => return Err(Fault(Invalid::Anchor, pos)),
        (["code"], false) => return Err(Fault(Invalid::MissingKey("text"), pos)),
        ([a], true) if *a != "code" => return Err(Fault(Invalid::Anchor, pos)),
        _ => {}
    }
    for (k, v) in es {
        let Some(name) = known_check_key(text_of(k)) else { continue };
        match name {
            "config_key" => {
                let ok = match &v.kind {
                    NodeKind::Scalar { .. } => true,
                    NodeKind::Seq(items) => !items.is_empty() && items.iter().all(|n| matches!(n.kind, NodeKind::Scalar { .. })),
                    NodeKind::Map(_) => false,
                };
                if !ok {
                    return Err(Fault(Invalid::Type("config_key"), at(v)));
                }
            }
            "code" if string(v, name)?.contains(['*', '?']) => return Err(Fault(Invalid::Type("code"), at(v))),
            _ => {
                string(v, name)?;
            }
        }
    }
    for k in &extra {
        unknown.push((at(k), format!("checks[{i}].{}", text_of(k))));
    }
    let anchor = match anchors.as_slice() {
        ["path"] => {
            let v = need(es, "path")?;
            RawAnchor::Path(string(v, "path")?, at(v))
        }
        ["code"] => {
            let v = need(es, "code")?;
            RawAnchor::Code(string(v, "code")?, at(v), string(need(es, "text")?, "text")?)
        }
        _ => {
            let v = need(es, "config_key")?;
            match &v.kind {
                NodeKind::Seq(items) => RawAnchor::ConfigKey(items.iter().map(|n| (text_of(n), at(n))).collect()),
                _ => RawAnchor::ConfigKey(vec![(text_of(v), at(v))]),
            }
        }
    };
    Ok(RawCheck { pos, verb: *verb, expect: string(verb_node, verb.as_str())?, anchor })
}

fn known_check_key(t: &str) -> Option<&'static str> {
    CHECK_KEYS.iter().find(|k| **k == t).copied()
}

/// `knowledge` per a plain scalar `[1-9][0-9]*`; a value past `u64::MAX` reads as `u64::MAX`.
fn schema_integer(v: &Node) -> Option<u64> {
    let NodeKind::Scalar { text, style: Style::Plain } = &v.kind else { return None };
    let b = text.as_bytes();
    let ok = matches!(b.first(), Some(b'1'..=b'9')) && b.iter().all(u8::is_ascii_digit);
    ok.then(|| text.parse().unwrap_or(u64::MAX))
}

/// A record of a newer schema: what reads as schema 1 reads it (`SEMANTICS.md` §The record). `applies_to` is taken only as written and
/// only whole, never defaulted from checks a newer schema may mean differently.
fn newer(scope: &str, schema: u64, top: &[(Node, Node)]) -> Parsed {
    let s = |name: &str| lookup(top, name).and_then(scalar);
    Parsed::Newer {
        schema,
        id: s("id").and_then(Id::parse).filter(|i| !i.is_example()),
        kind: s("kind").and_then(Kind::parse),
        title: s("title").filter(|t| title_ok(t)).map(str::to_owned),
        supersedes: s("supersedes").and_then(Id::parse),
        applies_to: lookup(top, "applies_to")
            .and_then(|v| strings(v).ok())
            .and_then(|l| l.iter().map(|&(g, _)| parse_rel(scope, g).ok()).collect::<Option<Vec<_>>>())
            .unwrap_or_default(),
    }
}

/// non-empty, and no line break after folding.
fn title_ok(t: &str) -> bool {
    !t.is_empty() && !t.contains('\n')
}

fn lookup<'a>(es: &'a [(Node, Node)], name: &str) -> Option<&'a Node> {
    es.iter().find(|(k, _)| text_of(k) == name).map(|(_, v)| v)
}

/// A key the required-keys step has already found.
fn need<'a>(es: &'a [(Node, Node)], name: &'static str) -> Res<&'a Node> {
    lookup(es, name).ok_or(Fault(Invalid::MissingKey(name), START))
}

fn scalar(n: &Node) -> Option<&str> {
    match &n.kind {
        NodeKind::Scalar { text, .. } => Some(text),
        _ => None,
    }
}

/// The text of a key (every key is a scalar once the structure step has passed).
fn text_of(n: &Node) -> &str {
    scalar(n).unwrap_or_default()
}

/// A string value, else `type:<key>` at the value.
fn string<'a>(n: &'a Node, key: &'static str) -> Res<&'a str> {
    scalar(n).ok_or(Fault(Invalid::Type(key), at(n)))
}

/// A list of strings with their positions; the error is where the list or an element is not one.
fn strings(n: &Node) -> Result<Vec<(&str, Pos)>, Pos> {
    let NodeKind::Seq(items) = &n.kind else { return Err(at(n)) };
    items.iter().map(|i| scalar(i).map(|s| (s, at(i))).ok_or(at(i))).collect()
}

/// A node's position in the file (the front matter starts on line 2); a mapping is at its first key.
fn at(n: &Node) -> Pos {
    match &n.kind {
        NodeKind::Map(es) if !es.is_empty() => es.first().map_or((0, 0), |(k, _)| at(k)),
        _ => (n.line.saturating_add(1), n.col),
    }
}

/// Where YAML stops reading: `quotes` at the value when the line is `key: value` or `- value` and the
/// value does not start with `'`, `[`, `{` or `#` (quoting it is the fix: `must: >= 2` opens a folded
/// block scalar in YAML); else `parse` where the reader stopped. `line` counts front-matter lines from 1.
fn syntax_fault(fm: &str, line: u32, col: u32) -> Fault {
    let at_line = (line.saturating_add(1), col);
    let Some(text) = fm.split('\n').nth(line.saturating_sub(1) as usize) else { return Fault(Invalid::Parse, at_line) };
    let indent = text.len() - text.trim_start_matches(' ').len();
    let mut rest = &text[indent..];
    let mut start = indent;
    if let Some(r) = rest.strip_prefix('-').filter(|r| r.starts_with(' ')) {
        let gap = r.len() - r.trim_start_matches(' ').len();
        start += 1 + gap;
        rest = &r[gap..];
    } else if rest.starts_with('-') {
        return Fault(Invalid::Parse, at_line);
    }
    if let Some(k) = rest.find(": ") {
        let after = &rest[k + 2..];
        let gap = after.len() - after.trim_start_matches(' ').len();
        start += k + 2 + gap;
        rest = &after[gap..];
    } else if start == indent {
        return Fault(Invalid::Parse, at_line); // neither a key with a value nor a list entry
    }
    if rest.is_empty() || rest.starts_with(['\'', '[', '{', '#']) {
        return Fault(Invalid::Parse, at_line);
    }
    let col = text[..start].chars().count() + 1;
    Fault(Invalid::Quotes, (line.saturating_add(1), u32::try_from(col).unwrap_or(u32::MAX)))
}

fn rel(scope: &str, s: &str, p: Pos) -> Res<RepoGlob> {
    parse_rel(scope, s).map_err(|e| Fault(e.invalid(), p))
}

/// The file part of a `config_key` layer (the text before its first `#`).
fn file_part(s: &str) -> &str {
    s.split_once('#').map_or(s, |(f, _)| f)
}

/// a wildcard before the last segment, or a last segment `**`, leaves one directory.
fn glob_dir(file: &str) -> bool {
    let (dirs, last) = file.rsplit_once('/').unwrap_or(("", file));
    dirs.contains(['*', '?']) || last == "**"
}

/// The default `applies_to` (`SEMANTICS.md` §The record, Default `applies_to`): each `path` anchor's path (and
/// `<path>/**` when literal), each `config_key` layer's file, each `code` file, once each in check order; the
/// whole folder when that names nothing.
pub fn default_applies_to(scope: &str, checks: &[Check]) -> Vec<RepoGlob> {
    let mut out: Vec<RepoGlob> = Vec::new();
    {
        let mut push = |g: RepoGlob| {
            if !out.contains(&g) {
                out.push(g);
            }
        };
        for c in checks {
            match &c.anchor {
                Anchor::Path(g) => {
                    push(g.clone());
                    if g.is_literal()
                        && let Ok(under) = parse_rel(scope, &format!("{}/**", g.rel))
                    {
                        push(under);
                    }
                }
                Anchor::ConfigKey(layers) => layers.iter().for_each(|l| push(l.file.clone())),
                Anchor::Code { file, .. } => push(file.clone()),
                Anchor::Unknown(_) => {}
            }
        }
    }
    if out.is_empty()
        && let Ok(all) = parse_rel(scope, "**")
    {
        out.push(all);
    }
    out
}

/// The front matter, the body and the body's first line: the first line is exactly `---`, and the
/// front matter ends at the next line that is exactly `---`.
fn split(text: &str) -> Option<(&str, &str, u32)> {
    let rest = text.strip_prefix("---\n")?;
    let mut off = 0;
    for line in rest.split_inclusive('\n') {
        if line.strip_suffix('\n').unwrap_or(line) == "---" {
            let fm = &rest[..off];
            let lines = u32::try_from(fm.matches('\n').count()).unwrap_or(u32::MAX);
            return Some((fm, &rest[off + line.len()..], lines.saturating_add(3)));
        }
        off += line.len();
    }
    None
}

/// the line of a `<<<<<<<` line that a later `>>>>>>>` line follows.
fn conflict_marker(text: &str) -> Option<u32> {
    let mut open = None;
    for (i, line) in (1u32..).zip(text.split('\n')) {
        match open {
            None if line.starts_with("<<<<<<<") => open = Some(i),
            Some(o) if line.starts_with(">>>>>>>") => return Some(o),
            _ => {}
        }
    }
    None
}

/// The position of the first byte that is not UTF-8.
fn utf8_pos(bytes: &[u8], valid_up_to: usize) -> Pos {
    let prefix = std::str::from_utf8(&bytes[..valid_up_to]).unwrap_or_default();
    let prefix = prefix.strip_prefix('\u{FEFF}').unwrap_or(prefix);
    let line = prefix.matches('\n').count() + 1;
    let col = prefix.rsplit('\n').next().unwrap_or_default().chars().count() + 1;
    (u32::try_from(line).unwrap_or(u32::MAX), u32::try_from(col).unwrap_or(u32::MAX))
}

/// The first mapping key, in file order, that `bad` rejects.
fn first_key(n: &Node, bad: &dyn Fn(&Node) -> bool) -> Option<Pos> {
    match &n.kind {
        NodeKind::Scalar { .. } => None,
        NodeKind::Seq(items) => items.iter().find_map(|i| first_key(i, bad)),
        NodeKind::Map(es) => es.iter().find_map(|(k, v)| if bad(k) { Some(at(k)) } else { first_key(k, bad).or_else(|| first_key(v, bad)) }),
    }
}

/// The second occurrence of a key in one mapping, at any depth, in file order.
fn first_duplicate(n: &Node) -> Option<Pos> {
    match &n.kind {
        NodeKind::Scalar { .. } => None,
        NodeKind::Seq(items) => items.iter().find_map(first_duplicate),
        NodeKind::Map(es) => {
            let mut seen = HashSet::new();
            es.iter().find_map(|(k, v)| if seen.insert(text_of(k)) { first_duplicate(v) } else { Some(at(k)) })
        }
    }
}

/// The first value that is not single-quoted (the top-level `knowledge` value excepted) or key that is
/// neither plain nor single-quoted.
fn first_unquoted(n: &Node, top: bool) -> Option<Pos> {
    match &n.kind {
        NodeKind::Scalar { style, .. } => (*style != Style::Single).then(|| at(n)),
        NodeKind::Seq(items) => items.iter().find_map(|i| first_unquoted(i, false)),
        NodeKind::Map(es) => es.iter().find_map(|(k, v)| match &k.kind {
            NodeKind::Scalar { style: Style::Plain | Style::Single, text } if top && text == "knowledge" => None,
            NodeKind::Scalar { style: Style::Plain | Style::Single, .. } => first_unquoted(v, false),
            _ => Some(at(k)),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reason::{Bound, Invalid};
    use crate::record::render::{render, Draft, DraftAnchor, DraftCheck};
    use crate::record::{Anchor, Expectation, Id, Kind, Op, Parsed, Record, Seg, Verb};
    use proptest::prelude::*;

    const HEAD: &str = "---
knowledge: 1
id: 'k3v9q2x7m1ab'
kind: 'decision'
title: 'Production API runs at least two replicas'
applies_to: ['charts/api/**', 'envs/prod/**']
";
    const CHECKS: &str = "checks:
  - config_key: 'charts/api/values.yaml#replicaCount'
    must: '>= 2'
  - path: 'docs/runbooks/api-rollout.md'
    must: 'exist'
";
    const BODY: &str = "A single replica took the API down for 11 minutes on 2026-09-02 during a node drain:
the pod disruption budget cannot protect one pod. Two replicas with the PDB at
`minAvailable: 1` survive a drain. Cost was reviewed and accepted; a cost change goes
through a new decision that supersedes this one.
";

    fn spec() -> String {
        format!("{HEAD}{CHECKS}---\n{BODY}")
    }

    /// The full example record with `old` (which must occur) replaced once by `new`.
    fn swap(old: &str, new: &str) -> Vec<u8> {
        let s = spec();
        assert!(s.contains(old), "{old:?}");
        s.replacen(old, new, 1).into_bytes()
    }

    /// The full example record with its `checks` block replaced by `block` (under `checks:`).
    fn with_checks(block: &str) -> Vec<u8> {
        format!("{HEAD}checks:\n{block}---\n{BODY}").into_bytes()
    }

    fn valid(bytes: &[u8]) -> Record {
        match parse_record("", bytes) {
            Parsed::Valid(r) => r,
            other => panic!("expected valid, got {other:?}"),
        }
    }

    fn invalid(bytes: &[u8]) -> (Invalid, u32, u32) {
        match parse_record("", bytes) {
            Parsed::Invalid { reason, line, col } => (reason, line, col),
            other => panic!("expected invalid, got {other:?}"),
        }
    }

    fn reason(bytes: &[u8]) -> Invalid {
        invalid(bytes).0
    }

    #[test]
    fn spec_example_parses() {
        assert_eq!(invalid(&swap("'k3v9q2x7m1ab'", "'0000000000a1'")), (Invalid::ExampleId, 3, 5));
        let r = valid(spec().as_bytes());
        assert_eq!(r.id.to_string(), "k3v9q2x7m1ab");
        assert_eq!(r.kind, Kind::Decision);
        assert_eq!(r.title, "Production API runs at least two replicas");
        assert!(r.applies_written);
        assert_eq!(r.applies_to.iter().map(|g| g.rel.as_str()).collect::<Vec<_>>(), ["charts/api/**", "envs/prod/**"]);
        assert_eq!(r.checks.len(), 2);
        let Anchor::ConfigKey(layers) = &r.checks[0].anchor else { panic!("{:?}", r.checks[0].anchor) };
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].file.rel, "charts/api/values.yaml");
        assert_eq!(layers[0].key.segs, vec![Seg::Key("replicaCount".to_owned())]);
        assert_eq!(r.checks[0].verb, Verb::Must);
        assert_eq!(r.checks[0].expect, Expectation::Compare { op: Op::Ge, literal: "2".to_owned() });
        assert_eq!(r.checks[0].line, 8);
        assert!(matches!(&r.checks[1].anchor, Anchor::Path(g) if g.rel == "docs/runbooks/api-rollout.md"));
        assert_eq!(r.checks[1].expect, Expectation::Exist);
        assert_eq!(r.checks[1].line, 10);
        assert_eq!(r.supersedes, None);
        assert!(r.unknown_keys.is_empty());
        assert_eq!(r.body, BODY);
        assert_eq!(r.first_paragraph(), BODY.trim_end_matches('\n'));
    }

    #[test]
    fn scope_joins_every_path() {
        let Parsed::Valid(r) = parse_record("services/api", spec().as_bytes()) else { panic!() };
        assert_eq!(r.applies_to[0].root, "services/api/charts/api/**");
        let Anchor::ConfigKey(layers) = &r.checks[0].anchor else { panic!() };
        assert_eq!(layers[0].file.root, "services/api/charts/api/values.yaml");
    }

    #[test]
    fn unquoted_and_double_quoted_strings_invalid() {
        assert_eq!(reason(&swap("must: '>= 2'", "must: >= 2")), Invalid::Quotes);
        assert_eq!(reason(&swap("kind: 'decision'", "kind: \"decision\"")), Invalid::Quotes);
        assert_eq!(reason(&swap("title: 'Production API runs at least two replicas'", "title: Production")), Invalid::Quotes);
        assert_eq!(reason(&swap("title: 'Production API runs at least two replicas'", "title: |\n  Production")), Invalid::Quotes);
        assert_eq!(reason(&swap("kind: 'decision'", "\"kind\": 'decision'")), Invalid::Quotes);
        assert_eq!(reason(&swap("kind: 'decision'", "kind: 'decision'\nsupersedes:")), Invalid::Quotes);
        // A single-quoted key is a key like any other.
        assert_eq!(valid(&swap("kind: 'decision'", "'kind': 'decision'")).kind, Kind::Decision);
    }

    #[test]
    fn error_positions_point_at_token() {
        assert_eq!(invalid(&swap("must: '>= 2'", "must: >= 2")), (Invalid::Quotes, 9, 11));
        assert_eq!(invalid(&swap("kind: 'decision'", "kind: \"decision\"")), (Invalid::Quotes, 4, 7));
        let (r, line, _) = invalid(&swap("applies_to: ['charts/api/**', 'envs/prod/**']", "applies_to:\n\t- 'a'"));
        assert_eq!((r, line), (Invalid::Parse, 7));
        assert_eq!(invalid(&swap("'Production API runs at least two replicas'", "Production: API")), (Invalid::Quotes, 5, 8));
        assert_eq!(invalid(&swap("must: 'exist'", "must: @x")), (Invalid::Quotes, 11, 11));
        // The fault points at the value: past a list entry's dash and its gap, and past every space after `: `.
        assert_eq!(invalid(&with_checks("  -   @x\n")), (Invalid::Quotes, 8, 7));
        assert_eq!(invalid(&swap("must: 'exist'", "must:   @x")), (Invalid::Quotes, 11, 13));
        // A mapping is at its first key; an empty one is where it starts.
        assert_eq!(invalid(&swap("'Production API runs at least two replicas'", "{}")), (Invalid::Type("title"), 5, 8));
        // Mis-indented: YAML stops at a line whose value is quoted, so the fix is not quoting.
        assert_eq!(invalid(&swap("- path: 'docs", "-   path: 'docs")).0, Invalid::Parse);
        assert_eq!(reason(&swap("'Production API runs at least two replicas'", "'It's'")), Invalid::Parse);
        assert_eq!(reason(&swap("'Production API runs at least two replicas'", "\"bad \\q escape\"")), Invalid::Quotes);
        assert_eq!(invalid(b"no front matter\n"), (Invalid::Parse, 1, 1));
        assert_eq!(invalid(&format!("{HEAD}{CHECKS}{BODY}").into_bytes()), (Invalid::Parse, 1, 1)); // no closing line
    }

    #[test]
    fn not_utf8_points_at_the_byte() {
        let mut b = spec().into_bytes();
        let at = b.iter().position(|&c| c == b'P').unwrap(); // `Production`, line 5 col 9
        b[at] = 0xFF;
        assert_eq!(invalid(&b), (Invalid::Parse, 5, 9));
    }

    #[test]
    fn knowledge_must_be_plain_positive_integer() {
        for v in ["1.0", "'1'", "01", "+1", "0", "-1", "1e0", "0x1", "[1]", "''", ""] {
            let r = invalid(&swap("knowledge: 1", &format!("knowledge: {v}")));
            assert_eq!(r.0, Invalid::SchemaInteger, "{v:?}");
            if !v.is_empty() {
                assert_eq!((r.1, r.2), (2, 12), "{v:?}");
            }
        }
    }

    #[test]
    fn knowledge_comment_is_not_the_value() {
        assert_eq!(valid(&swap("knowledge: 1", "knowledge: 1 # schema")).kind, Kind::Decision);
    }

    #[test]
    fn newer_applies_to_reads_as_schema_1() {
        let globs = |scope: &str, rest: &str| match parse_record(scope, format!("---\nknowledge: 2\nid: 'k3v9q2x7m1ab'\n{rest}---\nb\n").as_bytes()) {
            Parsed::Newer { applies_to, .. } => applies_to.into_iter().map(|g| g.root).collect::<Vec<_>>(),
            p => panic!("{p:?}"),
        };
        assert_eq!(globs("svc", "applies_to: ['src/**', 'a.rs']\n"), ["svc/src/**", "svc/a.rs"]);
        // All or nothing: one entry schema 1 cannot read, or another shape, and the record routes nowhere.
        assert!(globs("", "applies_to: ['src/**', '../x']\n").is_empty());
        assert!(globs("", "applies_to: {include: 'src/**'}\n").is_empty());
        assert!(globs("", "applies_to: 'src/**'\n").is_empty());
        // Never defaulted from checks, whose meaning a newer schema may change.
        assert!(globs("", "checks:\n  - path: 'src'\n    must: 'exist'\n").is_empty());
    }

    #[test]
    fn newer_schema_fills_what_reads() {
        let p = parse_record("", &swap("knowledge: 1", "knowledge: 2"));
        assert_eq!(
            p,
            Parsed::Newer {
                schema: 2,
                id: Id::parse("k3v9q2x7m1ab"),
                kind: Some(Kind::Decision),
                title: Some("Production API runs at least two replicas".to_owned()),
                supersedes: None,
                applies_to: vec![parse_rel("", "charts/api/**").unwrap(), parse_rel("", "envs/prod/**").unwrap()],
            }
        );
        // A newer schema skips schema 1's key rules, but never the format's rules.
        let odd = "---\nknowledge: 7\nkind: 'policy'\nchecks: 'anything'\n---\nbody\n";
        assert!(matches!(
            parse_record("", odd.as_bytes()),
            Parsed::Newer { schema: 7, id: None, kind: None, title: None, supersedes: None, .. }
        ));
        assert_eq!(reason(&swap("knowledge: 1", "knowledge: 2\nnote: plain")), Invalid::Quotes);
        let huge = parse_record("", &swap("knowledge: 1", "knowledge: 99999999999999999999999"));
        assert!(matches!(huge, Parsed::Newer { schema: u64::MAX, .. }));
    }

    #[test]
    fn yaml_traps_stay_text() {
        assert_eq!(valid(&swap("'k3v9q2x7m1ab'", "'0755abcd1234'")).id.to_string(), "0755abcd1234");
        let r = valid(&swap("['charts/api/**', 'envs/prod/**']", "['*.tf', 'app/[slug]/page.tsx', 'a,b']"));
        assert_eq!(r.applies_to.iter().map(|g| g.rel.as_str()).collect::<Vec<_>>(), ["*.tf", "app/[slug]/page.tsx", "a,b"]);
        // Unquoted, `*.tf` is an alias to no anchor: YAML cannot read it, and quoting is the fix.
        assert_eq!(invalid(&swap("['charts/api/**', 'envs/prod/**']", "*.tf")), (Invalid::Quotes, 6, 13));
        assert_eq!(reason(&swap("['charts/api/**', 'envs/prod/**']", "[*.tf]")), Invalid::Parse);
        for t in ["NO", "yes", "1e5", "0755", "~", "null", "2026-11-04", "a: b # c", "It''s"] {
            let r = valid(&swap("'Production API runs at least two replicas'", &format!("'{t}'")));
            assert_eq!(r.title, t.replace("''", "'"), "{t}");
        }
        let multi = valid(&swap("'Production API runs at least two replicas'", "'Production API\n  runs at least two replicas'"));
        assert_eq!(multi.title, "Production API runs at least two replicas");
    }

    #[test]
    fn yaml_features_and_duplicates_invalid() {
        assert_eq!(invalid(&swap("kind: 'decision'", "kind: &k 'decision'")), (Invalid::YamlFeature, 4, 10));
        assert_eq!(reason(&swap("kind: 'decision'", "kind: !!str 'decision'")), Invalid::YamlFeature);
        assert_eq!(reason(&swap("kind: 'decision'", "kind: 'decision'\nx-a: &a 'v'\nx-b: *a")), Invalid::YamlFeature);
        assert_eq!(invalid(&swap("kind: 'decision'", "kind: 'decision'\n<<: 'x'")), (Invalid::YamlFeature, 5, 1));
        assert_eq!(invalid(&swap("kind: 'decision'", "kind: 'decision'\nkind: 'fact'")), (Invalid::DuplicateKey, 5, 1));
        assert_eq!(invalid(&swap("    must: 'exist'", "    must: 'exist'\n    must: 'exist'")), (Invalid::DuplicateKey, 12, 5));
        // Duplicate before quotes, quotes before the id: the first step that fails wins.
        assert_eq!(reason(&swap("kind: 'decision'", "kind: decision\nkind: 'fact'")), Invalid::DuplicateKey);
        assert_eq!(reason(&swap("'k3v9q2x7m1ab'", "'bad'\nnote: plain")), Invalid::Quotes);
        assert_eq!(reason(&swap("kind: 'decision'", "? ['a']\n: 'x'")), Invalid::Parse);
        assert_eq!(reason(&swap("kind: 'decision'", "kind: 'decision'\n--- 'b'")), Invalid::Parse); // two documents
    }

    #[test]
    fn front_matter_may_be_exactly_the_bound() {
        let fm = format!("{}{CHECKS}", &HEAD[4..]);
        // `x-pad: '…'` and its line end add ten bytes to the padding.
        let with_pad = |n: usize| format!("---\n{fm}x-pad: '{}'\n---\n{BODY}", "a".repeat(n));
        let n = FRONT_MATTER_MAX - fm.len() - 10;
        valid(with_pad(n).as_bytes());
        assert_eq!(invalid(with_pad(n + 1).as_bytes()), (Invalid::Bound(Bound::FrontMatter), 1, 1));
    }

    #[test]
    fn render_writes_the_spec_shape() {
        // A one-layer config_key is one line; several layers are a list.
        let check = |layers: &[&str]| DraftCheck {
            anchor: DraftAnchor::ConfigKey(layers.iter().map(|l| (*l).to_owned()).collect()),
            verb: Verb::Must,
            expect: ">= 2".to_owned(),
        };
        let d = Draft {
            kind: Kind::Decision,
            title: "Production API runs at least two replicas".to_owned(),
            applies_to: vec!["charts/api/**".to_owned()],
            checks: vec![check(&["charts/api/values.yaml#replicaCount"]), check(&["charts/api/values.yaml#replicaCount", "envs/prod/api.yaml#replicaCount"])],
            supersedes: None,
            body: "Why.".to_owned(),
        };
        let want = "---\nknowledge: 1\nid: 'k3v9q2x7m1ab'\nkind: 'decision'\ntitle: 'Production API runs at least two replicas'\n\
                    applies_to:\n  - 'charts/api/**'\nchecks:\n  - config_key: 'charts/api/values.yaml#replicaCount'\n    must: '>= 2'\n\
                    \x20 - config_key:\n      - 'charts/api/values.yaml#replicaCount'\n      - 'envs/prod/api.yaml#replicaCount'\n    must: '>= 2'\n\
                    ---\nWhy.\n";
        assert_eq!(render(&Id::parse("k3v9q2x7m1ab").unwrap(), &d), want);
        valid(want.as_bytes());
    }

    #[test]
    fn front_matter_shape_and_bounds() {
        assert_eq!(invalid(b"---\n- 'a'\n---\nbody\n"), (Invalid::Parse, 1, 1));
        assert_eq!(invalid(b"---\n---\nbody\n"), (Invalid::Parse, 1, 1));
        let big = format!("x-pad: '{}'", "a".repeat(8_200));
        assert_eq!(invalid(&swap("kind: 'decision'", &format!("kind: 'decision'\n{big}"))), (Invalid::Bound(Bound::FrontMatter), 1, 1));
        let deep = format!("x-deep: {}'v'{}", "[".repeat(40), "]".repeat(40));
        assert_eq!(reason(&swap("kind: 'decision'", &format!("kind: 'decision'\n{deep}"))), Invalid::Bound(Bound::Depth));
        let mut huge = spec();
        huge.push_str(&"word ".repeat(14_000));
        assert_eq!(invalid(huge.as_bytes()), (Invalid::Bound(Bound::RecordFile), 1, 1));
    }

    #[test]
    fn required_keys_and_values() {
        assert_eq!(invalid(&swap("kind: 'decision'\n", "")), (Invalid::MissingKey("kind"), 1, 1));
        assert_eq!(reason(&swap("knowledge: 1\n", "")), Invalid::MissingKey("knowledge"));
        // Every required key is looked for before any value is read.
        assert_eq!(reason(&swap("id: 'k3v9q2x7m1ab'\nkind: 'decision'\n", "id: 'bad'\n")), Invalid::MissingKey("kind"));
        assert_eq!(invalid(&swap("'k3v9q2x7m1ab'", "'k3v9q2x7m1aI'")), (Invalid::Id, 3, 5));
        assert_eq!(reason(&swap("'k3v9q2x7m1ab'", "['k3v9q2x7m1ab']")), Invalid::Type("id"));
        assert_eq!(invalid(&swap("'decision'", "'Decision'")), (Invalid::Kind, 4, 7));
        assert_eq!(reason(&swap("'decision'", "'policy'")), Invalid::Kind);
        for k in ["decision", "fact", "procedure", "question"] {
            assert_eq!(valid(&swap("'decision'", &format!("'{k}'"))).kind.as_str(), k);
        }
        assert_eq!(reason(&swap("'Production API runs at least two replicas'", "''")), Invalid::Title);
        assert_eq!(reason(&swap("'Production API runs at least two replicas'", "'a\n\n  b'")), Invalid::Title);
        assert_eq!(reason(&swap("'Production API runs at least two replicas'", "['a']")), Invalid::Type("title"));
    }

    #[test]
    fn crlf_bom_record_equals_lf() {
        let lf = valid(spec().as_bytes());
        let crlf = format!("\u{FEFF}{}", spec().replace('\n', "\r\n"));
        assert_eq!(valid(crlf.as_bytes()), lf);
        // A position after a BOM and CRLF is the same as in the LF file.
        let bad = format!("\u{FEFF}{}", String::from_utf8(swap("must: '>= 2'", "must: >= 2")).unwrap().replace('\n', "\r\n"));
        assert_eq!(invalid(bad.as_bytes()), (Invalid::Quotes, 9, 11));
        assert_eq!(reason(format!("\u{FEFF}\u{FEFF}{}", spec()).as_bytes()), Invalid::HiddenText);
    }

    #[test]
    fn hidden_text_conflict_markers_empty_body_invalid() {
        assert_eq!(invalid(&swap("'Production", "'Pro\u{200B}duction")), (Invalid::HiddenText, 5, 12));
        assert_eq!(invalid(format!("{}\u{202E}x\n", spec()).as_bytes()), (Invalid::HiddenText, 17, 1));
        assert_eq!(reason(format!("{}a\u{1B}[31m\n", spec()).as_bytes()), Invalid::HiddenText);
        assert_eq!(reason(format!("{}a\rb\n", spec()).as_bytes()), Invalid::HiddenText);
        let conflict = format!("{}<<<<<<< ours\nA\n=======\nB\n>>>>>>> theirs\n", spec());
        assert_eq!(invalid(conflict.as_bytes()), (Invalid::ConflictMarkers, 17, 1));
        valid(format!("{}Heading\n=======\n", spec()).as_bytes());
        valid(format!("{}<<<<<<< alone\n", spec()).as_bytes());
        valid(format!("{}>>>>>>> before\n<<<<<<< after\n", spec()).as_bytes());
        assert_eq!(invalid(format!("{HEAD}{CHECKS}---\n").as_bytes()), (Invalid::EmptyBody, 13, 1));
        assert_eq!(invalid(format!("{HEAD}{CHECKS}---").as_bytes()), (Invalid::EmptyBody, 13, 1));
        assert_eq!(invalid(format!("{HEAD}{CHECKS}---\n\n  \n\t\n").as_bytes()), (Invalid::EmptyBody, 13, 1));
        // Hebrew and Arabic with the implicit direction marks are text.
        valid(format!("{HEAD}---\nמחיר\u{200F} 5 ושלום\n\u{061C}٣ مرحبا\n").as_bytes());
    }

    #[test]
    fn unknown_and_x_keys() {
        let r = valid(&swap("kind: 'decision'", "kind: 'decision'\nowner: 'x'\nx-tool: 'y'"));
        assert_eq!(r.unknown_keys, ["owner"]);
        let r = valid(&swap("    must: 'exist'", "    must: 'exist'\n    owner: 'x'\n    x-n: 'y'"));
        assert_eq!(r.unknown_keys, ["checks[1].owner"]);
        let r = valid(&format!("{}zeta: 'z'\n{CHECKS}---\n{BODY}", HEAD.replace("kind: 'decision'", "alpha: 'a'\nkind: 'decision'")).into_bytes());
        assert_eq!(r.unknown_keys, ["alpha", "zeta"]);
        let r = valid(&swap("kind: 'decision'", "kind: 'decision'\nx-tool: {nested: ['deep']}"));
        assert!(r.unknown_keys.is_empty());
    }

    #[test]
    fn unknown_check_kind_and_op_parse() {
        let r = valid(&swap("    must: 'exist'\n", "    must: 'exist'\n  - package: 'sqlx'\n    must: '>= 0.8'\n"));
        assert_eq!(r.checks[2].anchor, Anchor::Unknown("package".to_owned()));
        assert_eq!(r.checks[2].expect, Expectation::Compare { op: Op::Ge, literal: "0.8".to_owned() });
        assert!(r.unknown_keys.is_empty());
        let unknown_shape = valid(&swap("    must: 'exist'\n", "    must: 'exist'\n  - sql: {db: 'x'}\n    text: 'y'\n    assumes: 'exist'\n"));
        assert_eq!(unknown_shape.checks[2].anchor, Anchor::Unknown("sql".to_owned()));
        assert_eq!(unknown_shape.checks[2].verb, Verb::Assumes);
        for (e, want) in [
            ("~= 2", Expectation::Unknown("~= 2".to_owned())),
            ("=== 2", Expectation::Unknown("=== 2".to_owned())),
            ("<> 2", Expectation::Unknown("<> 2".to_owned())),
            ("=> 2", Expectation::Unknown("=> 2".to_owned())),
            ("exists", Expectation::Unknown("exists".to_owned())),
            ("Exist", Expectation::Unknown("Exist".to_owned())),
            (">=2", Expectation::Compare { op: Op::Ge, literal: "2".to_owned() }),
            ("== -1", Expectation::Compare { op: Op::Eq, literal: "-1".to_owned() }),
            ("!=  a b ", Expectation::Compare { op: Op::Ne, literal: "a b".to_owned() }),
            ("== ", Expectation::Compare { op: Op::Eq, literal: String::new() }),
            ("< 0.9", Expectation::Compare { op: Op::Lt, literal: "0.9".to_owned() }),
            ("> v1", Expectation::Compare { op: Op::Gt, literal: "v1".to_owned() }),
            ("<= 3", Expectation::Compare { op: Op::Le, literal: "3".to_owned() }),
        ] {
            let r = valid(&swap("must: '>= 2'", &format!("must: '{e}'")));
            assert_eq!(r.checks[0].expect, want, "{e:?}");
        }
    }

    /// `SEMANTICS.md` §The record, expectations: one pair of the same quote around a literal is quoting; the text
    /// between is the literal, spaces and all, and nothing inside is unescaped.
    #[test]
    fn a_quoted_literal_is_the_text_between_one_pair_of_quotes() {
        for (e, literal) in [
            ("== '1.97'", "1.97"),
            ("== \"1.97\"", "1.97"),
            ("!=  ' a b '  ", " a b "),
            ("== ''", ""),
            ("== \"\"", ""),
            ("== ''a''", "'a'"),
            ("== \"'a'\"", "'a'"),
            ("== 'it's'", "it's"),
            ("== \"a\\\"b\"", "a\\\"b"),
            ("== '", "'"),
            ("== 'a\"", "'a\""),
            ("== a'", "a'"),
            (">= 'v3.10'", "v3.10"),
        ] {
            let r = valid(&swap("must: '>= 2'", &format!("must: '{}'", e.replace('\'', "''"))));
            let Expectation::Compare { literal: got, .. } = &r.checks[0].expect else { panic!("{e:?}: {:?}", r.checks[0].expect) };
            assert_eq!(got, literal, "{e:?}");
        }
    }

    proptest! {
        /// What an expectation shows reads back as the same expectation: a literal a bare form would change is
        /// shown quoted.
        #[test]
        fn an_expectation_reads_back_from_what_it_shows(
            op in prop::sample::select(vec![Op::Eq, Op::Ne, Op::Ge, Op::Le, Op::Gt, Op::Lt]),
            literal in "[ \t'\"a=<.]{0,6}",
        ) {
            let e = Expectation::Compare { op, literal };
            prop_assert_eq!(Expectation::parse(&e.to_string()), e);
        }
    }

    #[test]
    fn check_key_rules() {
        let c = |block: &str| invalid(&with_checks(block));
        assert_eq!(c("  - path: 'a'\n    must: 'exist'\n    assumes: 'exist'\n"), (Invalid::Verbs, 8, 5));
        assert_eq!(c("  - path: 'a'\n"), (Invalid::Verbs, 8, 5));
        assert_eq!(c("  - path: 'a'\n    code: 'b'\n    text: 't'\n    must: 'exist'\n").0, Invalid::Anchor);
        assert_eq!(c("  - code: 'b'\n    must: 'exist'\n"), (Invalid::MissingKey("text"), 8, 5));
        assert_eq!(c("  - path: 'a'\n    text: 't'\n    must: 'exist'\n").0, Invalid::Anchor);
        assert_eq!(c("  - text: 't'\n    must: 'exist'\n").0, Invalid::Anchor);
        assert_eq!(c("  - must: 'exist'\n").0, Invalid::Anchor);
        assert_eq!(c("  - x-mine: 'a'\n    must: 'exist'\n").0, Invalid::Anchor);
        assert_eq!(c("  - path: ['a']\n    must: 'exist'\n"), (Invalid::Type("path"), 8, 11));
        assert_eq!(c("  - path: 'a'\n    must: ['exist']\n").0, Invalid::Type("must"));
        assert_eq!(c("  - code: 'src/*.rs'\n    text: 't'\n    must: 'exist'\n").0, Invalid::Type("code"));
        assert_eq!(c("  - config_key: []\n    must: 'exist'\n").0, Invalid::Type("config_key"));
        assert_eq!(c("  - config_key: [['a#b']]\n    must: 'exist'\n").0, Invalid::Type("config_key"));
        assert_eq!(c("  - config_key: 'a.yaml'\n    must: 'exist'\n"), (Invalid::KeyPath, 8, 17));
        assert_eq!(c("  - config_key: 'a.yaml#a..b'\n    must: 'exist'\n").0, Invalid::KeyPath);
        assert_eq!(c("  - config_key: 'a.yaml#'\n    must: 'exist'\n").0, Invalid::KeyPath);
        assert_eq!(c("  - config_key: 'infra/*/x.tf#a'\n    must: 'exist'\n").0, Invalid::GlobDir);
        assert_eq!(c("  - config_key: 'infra/**#a'\n    must: 'exist'\n").0, Invalid::GlobDir);
        assert_eq!(c("  - config_key: '**/x.tf#a'\n    must: 'exist'\n").0, Invalid::GlobDir);
        assert_eq!(c("  - path: '../x'\n    must: 'exist'\n"), (Invalid::Confinement, 8, 11));
        assert_eq!(c("  - config_key: '#a'\n    must: 'exist'\n").0, Invalid::Parse);
        assert_eq!(c("  - 'a'\n").0, Invalid::Type("checks"));
        assert_eq!(c("  - config_key:\n      - 'a.yaml#a'\n      - '/etc/x.yaml#a'\n    must: 'exist'\n"), (Invalid::Confinement, 10, 9));
        let ok = valid(&with_checks("  - config_key: ['infra/prod/*.tf#a', 'b.yaml#\"x.y\"[0]']\n    must: 'exist'\n  - code: 'src/lib.rs'\n    text: 'fn main() { retry(3) }'\n    assumes: 'exist'\n"));
        let Anchor::ConfigKey(layers) = &ok.checks[0].anchor else { panic!() };
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[1].key.segs, vec![Seg::Key("x.y".to_owned()), Seg::Index(0)]);
        assert!(matches!(&ok.checks[1].anchor, Anchor::Code { file, text } if file.rel == "src/lib.rs" && text == "fn main() { retry(3) }"));
    }

    #[test]
    fn other_key_types_and_order() {
        assert_eq!(invalid(&swap("['charts/api/**', 'envs/prod/**']", "'charts/**'")), (Invalid::Type("applies_to"), 6, 13));
        assert_eq!(invalid(&swap("['charts/api/**', 'envs/prod/**']", "['a', ['b']]")), (Invalid::Type("applies_to"), 6, 19));
        assert_eq!(invalid(&swap("['charts/api/**', 'envs/prod/**']", "['/abs']")), (Invalid::Confinement, 6, 14));
        assert_eq!(reason(&swap("['charts/api/**', 'envs/prod/**']", "['']")), Invalid::Parse);
        assert_eq!(reason(&swap("checks:\n", "checks: 'x'\nx-old:\n")), Invalid::Type("checks"));
        let many: String = (0..101).map(|i| format!("  - path: 'p{i}'\n    must: 'exist'\n")).collect();
        assert_eq!(invalid(&with_checks(&many)), (Invalid::Bound(Bound::Checks), 8 + 2 * 100, 5));
        let hundred: String = (0..100).map(|i| format!("  - path: 'p{i}'\n    must: 'exist'\n")).collect();
        assert_eq!(valid(&with_checks(&hundred)).checks.len(), 100);
        assert_eq!(invalid(&swap("kind: 'decision'", "kind: 'decision'\nsupersedes: 'nope'")), (Invalid::Id, 5, 13));
        assert_eq!(reason(&swap("kind: 'decision'", "kind: 'decision'\nsupersedes: ['k3v9q2x7m1ab']")), Invalid::Type("supersedes"));
        assert_eq!(valid(&swap("kind: 'decision'", "kind: 'decision'\nsupersedes: 'a1b2c3d4e5f6'")).supersedes, Id::parse("a1b2c3d4e5f6"));
        // Within one step, the first in the file: an applies_to entry before a check's path.
        let both = String::from_utf8(swap("['charts/api/**', 'envs/prod/**']", "['../a']")).unwrap().replace("'docs/runbooks/api-rollout.md'", "'../b'");
        assert_eq!(invalid(both.as_bytes()), (Invalid::Confinement, 6, 14));
        // Steps in their order: a key path (step 16) before a path (step 17), whatever the file order.
        let order = String::from_utf8(swap("['charts/api/**', 'envs/prod/**']", "['../a']")).unwrap().replace("values.yaml#replicaCount", "values.yaml#a..b");
        assert_eq!(reason(order.as_bytes()), Invalid::KeyPath);
    }

    #[test]
    fn default_applies_to_follows_the_checks() {
        let r = valid(&swap("applies_to: ['charts/api/**', 'envs/prod/**']\n", ""));
        assert!(!r.applies_written);
        assert_eq!(
            r.applies_to.iter().map(|g| g.rel.as_str()).collect::<Vec<_>>(),
            ["charts/api/values.yaml", "docs/runbooks/api-rollout.md", "docs/runbooks/api-rollout.md/**"]
        );
        let none = valid(format!("{}---\n{BODY}", HEAD.replace("applies_to: ['charts/api/**', 'envs/prod/**']\n", "")).as_bytes());
        assert_eq!(none.applies_to.iter().map(|g| g.rel.as_str()).collect::<Vec<_>>(), ["**"]);
        let empty = valid(&swap("['charts/api/**', 'envs/prod/**']", "[]"));
        assert!(empty.applies_to.is_empty() && empty.applies_written);
        let globbed = valid(&format!("{}checks:\n  - path: 'docs/*.md'\n    must: 'exist'\n  - path: 'docs/*.md'\n    assumes: 'exist'\n---\n{BODY}", HEAD.replace("applies_to: ['charts/api/**', 'envs/prod/**']\n", "")).into_bytes());
        assert_eq!(globbed.applies_to.iter().map(|g| g.rel.as_str()).collect::<Vec<_>>(), ["docs/*.md"]);
    }

    #[test]
    fn first_paragraph_is_c20() {
        let r = valid(format!("{HEAD}---\n\n  \nWhy one.\nWhy two.\n \nNext.\n").as_bytes());
        assert_eq!(r.first_paragraph(), "Why one.\nWhy two.");
    }

    fn rel_path() -> impl Strategy<Value = String> {
        prop::collection::vec(prop_oneof!["[a-z0-9_-]{1,6}", "[a-z]{0,2}\\*[a-z]{0,2}", Just("**".to_owned())], 1..4).prop_map(|v| v.join("/"))
    }

    fn literal_path() -> impl Strategy<Value = String> {
        prop::collection::vec("[a-z0-9_.-]{1,6}".prop_filter("not a dot or .git segment", |s| s != "." && s != ".git" && !s.contains("..")), 1..4)
            .prop_map(|v| v.join("/"))
    }

    fn id() -> impl Strategy<Value = Id> {
        "[0-9abcdefghjkmnpqrstvwxyz]{12}".prop_filter_map("not an example", |s| Id::parse(&s).filter(|i| !i.is_example()))
    }

    fn draft_check() -> impl Strategy<Value = DraftCheck> {
        let anchor = prop_oneof![
            rel_path().prop_map(DraftAnchor::Path),
            prop::collection::vec((literal_path(), "[a-z]{1,5}(\\.[a-z]{1,5}){0,2}").prop_map(|(f, k)| format!("{f}#{k}")), 1..3).prop_map(DraftAnchor::ConfigKey),
            (literal_path(), "[a-z ;(){}'é]{0,30}").prop_map(|(file, text)| DraftAnchor::Code { file, text }),
        ];
        let expect = prop_oneof![Just("exist".to_owned()), (prop::sample::select(vec!["==", "!=", ">=", ">", "<=", "<"]), "[a-z0-9.]{1,6}").prop_map(|(o, l)| format!("{o} {l}"))];
        (anchor, prop_oneof![Just(Verb::Must), Just(Verb::Assumes)], expect).prop_map(|(anchor, verb, expect)| DraftCheck { anchor, verb, expect })
    }

    fn draft() -> impl Strategy<Value = Draft> {
        (
            prop::sample::select(vec![Kind::Decision, Kind::Fact, Kind::Procedure, Kind::Question]),
            "[a-zA-Z0-9 ',.:#é日本-]{0,40}[a-zA-Z'é]",
            prop::collection::vec(rel_path(), 0..4),
            prop::collection::vec(draft_check(), 0..4),
            prop::option::of(id()),
            "[a-z][a-zA-Z0-9 .,'#*-]{0,60}\n(\n[a-z][a-z ]{0,20}\n)?",
        )
            .prop_map(|(kind, title, applies_to, checks, supersedes, body)| Draft { kind, title, applies_to, checks, supersedes, body })
    }

    proptest! {
        #[test]
        fn render_parse_round_trip(d in draft(), id in id()) {
            let text = render(&id, &d);
            let (cr, bom) = ('\r', '\u{FEFF}');
            prop_assert!(!text.contains(cr) && !text.starts_with(bom));
            let r = match parse_record("", text.as_bytes()) {
                Parsed::Valid(r) => r,
                other => return Err(TestCaseError::fail(format!("{other:?}\n{text}"))),
            };
            prop_assert_eq!(r.id, id);
            prop_assert_eq!(r.kind, d.kind);
            prop_assert_eq!(&r.title, &d.title);
            prop_assert!(r.applies_written);
            prop_assert_eq!(r.applies_to.iter().map(|g| g.rel.clone()).collect::<Vec<_>>(), d.applies_to.clone());
            prop_assert_eq!(r.checks.len(), d.checks.len());
            for (c, want) in r.checks.iter().zip(&d.checks) {
                prop_assert_eq!(c.verb, want.verb);
                prop_assert_eq!(c.expect.to_string(), want.expect.clone());
                match (&c.anchor, &want.anchor) {
                    (Anchor::Path(g), DraftAnchor::Path(p)) => prop_assert_eq!(&g.rel, p),
                    (Anchor::ConfigKey(ls), DraftAnchor::ConfigKey(ws)) => {
                        prop_assert_eq!(ls.iter().map(|l| format!("{}#{}", l.file.rel, l.key.text)).collect::<Vec<_>>(), ws.clone());
                    }
                    (Anchor::Code { file, text }, DraftAnchor::Code { file: f, text: t }) => {
                        prop_assert_eq!(&file.rel, f);
                        prop_assert_eq!(text, t);
                    }
                    (got, want) => return Err(TestCaseError::fail(format!("{got:?} vs {want:?}"))),
                }
            }
            prop_assert_eq!(r.supersedes, d.supersedes);
            prop_assert_eq!(&r.body, &d.body);
            prop_assert!(r.unknown_keys.is_empty());
        }
    }
}
