// SPDX-License-Identifier: MIT
//! One YAML reader for record front matter (strict) and anchored files (anchors expanded), over
//! yaml-rust2's event parser so the scalar style, anchors, tags and marks are all seen (spec §4.2
//! "Parsing", §7.5, §7.6). Mapping entries are kept in order, duplicates included: detection is the
//! caller's.

use std::collections::HashMap;

use yaml_rust2::parser::{Event, Parser};
use yaml_rust2::scanner::{Marker, TScalarStyle};

use crate::bounds::{ALIAS_EXPANSION_MAX, FRONT_MATTER_DEPTH, RESOLVER_DEPTH};

/// One YAML node with its one-based position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub kind: NodeKind,
    pub line: u32,
    pub col: u32,
    /// The node carries an explicit tag (`!Ref`, `!!str`); only in anchored mode.
    pub tagged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Scalar { text: String, style: Style },
    Map(Vec<(Node, Node)>),
    Seq(Vec<Node>),
}

/// How a scalar was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Plain,
    Single,
    Double,
    /// A literal (`|`) or folded (`>`) block.
    Block,
}

/// Front matter (no anchors, aliases or tags) or an anchored file (aliases expanded, tags marked).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Strict,
    Anchored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YamlErrorKind {
    /// The text is not YAML.
    Syntax,
    /// An anchor, alias or tag in strict mode.
    Feature,
    /// Nesting deeper than the mode's bound.
    Depth,
    /// Copies for anchors and aliases beyond `ALIAS_EXPANSION_MAX` (`Built`'s size).
    AliasBudget,
}

/// A failure with its one-based position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YamlError {
    pub kind: YamlErrorKind,
    pub line: u32,
    pub col: u32,
}

fn pos(m: Marker) -> (u32, u32) {
    let clamp = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
    // yaml-rust2 counts lines from 1 and columns from 0.
    (clamp(m.line()), clamp(m.col()).saturating_add(1))
}

fn err(kind: YamlErrorKind, m: Marker) -> YamlError {
    let (line, col) = pos(m);
    YamlError { kind, line, col }
}

/// A finished node with its size and height (collection levels), for the alias bounds. The size is what a copy
/// of the node costs: one per node, and a scalar also its bytes, so a long text aliased many times is charged
/// for every byte it copies, not as one node.
#[derive(Clone)]
struct Built {
    node: Node,
    count: usize,
    height: usize,
}

enum Frame {
    Map { head: Node, anchor: usize, entries: Vec<(Node, Node)>, key: Option<Node>, count: usize, height: usize },
    Seq { head: Node, anchor: usize, items: Vec<Node>, count: usize, height: usize },
}

struct Reader {
    mode: Mode,
    depth_max: usize,
    stack: Vec<Frame>,
    anchors: HashMap<usize, Built>,
    expanded: usize,
    root: Option<Node>,
}

impl Reader {
    /// Charges a copy of `size` against the budget, before it is made.
    fn charge(&mut self, size: usize, m: Marker) -> Result<(), YamlError> {
        self.expanded = self.expanded.saturating_add(size);
        if self.expanded > ALIAS_EXPANSION_MAX { Err(err(YamlErrorKind::AliasBudget, m)) } else { Ok(()) }
    }

    /// Attaches a finished node to its parent (or makes it the document root) and records its anchor, whose
    /// kept copy is charged: nested anchors copy without a single alias.
    fn finish(&mut self, built: Built, anchor: usize, m: Marker) -> Result<(), YamlError> {
        if anchor != 0 {
            self.charge(built.count, m)?;
            self.anchors.insert(anchor, built.clone());
        }
        let Built { node, count, height } = built;
        match self.stack.last_mut() {
            None => self.root = Some(node),
            Some(Frame::Map { entries, key, count: c, height: h, .. }) => {
                *c += count;
                *h = (*h).max(height + 1);
                match key.take() {
                    None => *key = Some(node),
                    Some(k) => entries.push((k, node)),
                }
            }
            Some(Frame::Seq { items, count: c, height: h, .. }) => {
                *c += count;
                *h = (*h).max(height + 1);
                items.push(node);
            }
        }
        Ok(())
    }

    fn head(&self, m: Marker, tagged: bool) -> Node {
        let (line, col) = pos(m);
        Node { kind: NodeKind::Scalar { text: String::new(), style: Style::Plain }, line, col, tagged }
    }

    /// The strict-mode refusals and the tag flag of an anchored node.
    fn features(&self, anchor: usize, tagged: bool, m: Marker) -> Result<bool, YamlError> {
        if self.mode == Mode::Strict && (anchor != 0 || tagged) {
            return Err(err(YamlErrorKind::Feature, m));
        }
        Ok(tagged)
    }

    fn push(&mut self, frame: Frame, m: Marker) -> Result<(), YamlError> {
        if self.stack.len() + 1 > self.depth_max {
            return Err(err(YamlErrorKind::Depth, m));
        }
        self.stack.push(frame);
        Ok(())
    }

    fn event(&mut self, ev: Event, m: Marker) -> Result<(), YamlError> {
        match ev {
            Event::Scalar(text, style, anchor, tag) => {
                let tagged = self.features(anchor, tag.is_some(), m)?;
                let style = match style {
                    TScalarStyle::Plain => Style::Plain,
                    TScalarStyle::SingleQuoted => Style::Single,
                    TScalarStyle::DoubleQuoted => Style::Double,
                    TScalarStyle::Literal | TScalarStyle::Folded => Style::Block,
                };
                let mut node = self.head(m, tagged);
                let count = text.len().saturating_add(1);
                node.kind = NodeKind::Scalar { text, style };
                self.finish(Built { node, count, height: 0 }, anchor, m)?;
            }
            Event::MappingStart(anchor, tag) => {
                let tagged = self.features(anchor, tag.is_some(), m)?;
                let head = self.head(m, tagged);
                self.push(Frame::Map { head, anchor, entries: Vec::new(), key: None, count: 1, height: 1 }, m)?;
            }
            Event::SequenceStart(anchor, tag) => {
                let tagged = self.features(anchor, tag.is_some(), m)?;
                let head = self.head(m, tagged);
                self.push(Frame::Seq { head, anchor, items: Vec::new(), count: 1, height: 1 }, m)?;
            }
            Event::MappingEnd | Event::SequenceEnd => {
                let built = match self.stack.pop() {
                    Some(Frame::Map { mut head, anchor, entries, count, height, .. }) => {
                        head.kind = NodeKind::Map(entries);
                        (Built { node: head, count, height }, anchor)
                    }
                    Some(Frame::Seq { mut head, anchor, items, count, height }) => {
                        head.kind = NodeKind::Seq(items);
                        (Built { node: head, count, height }, anchor)
                    }
                    None => return Err(err(YamlErrorKind::Syntax, m)),
                };
                self.finish(built.0, built.1, m)?;
            }
            Event::Alias(id) => {
                if self.mode == Mode::Strict {
                    return Err(err(YamlErrorKind::Feature, m));
                }
                // An alias to an unknown or unfinished (recursive) anchor is not YAML we can read.
                let Some(b) = self.anchors.get(&id) else { return Err(err(YamlErrorKind::Syntax, m)) };
                if self.stack.len() + b.height > self.depth_max {
                    return Err(err(YamlErrorKind::Depth, m));
                }
                let size = b.count;
                self.charge(size, m)?;
                let b = self.anchors.get(&id).cloned().ok_or_else(|| err(YamlErrorKind::Syntax, m))?;
                self.finish(b, 0, m)?;
            }
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentStart | Event::DocumentEnd => {}
        }
        Ok(())
    }
}

/// Reads `src` into one `Node` per document.
pub fn read_yaml(src: &str, mode: Mode) -> Result<Vec<Node>, YamlError> {
    read(src, mode).map(|(docs, _)| docs)
}

/// What the copies for anchors and aliases charge an anchored file against `ALIAS_EXPANSION_MAX`: what the
/// bound is measured by (`examples/measure_formats.rs`).
pub fn alias_expansion(src: &str) -> Result<usize, YamlError> {
    read(src, Mode::Anchored).map(|(_, expanded)| expanded)
}

fn read(src: &str, mode: Mode) -> Result<(Vec<Node>, usize), YamlError> {
    let depth_max = match mode {
        Mode::Strict => FRONT_MATTER_DEPTH,
        Mode::Anchored => RESOLVER_DEPTH,
    };
    let mut r = Reader { mode, depth_max, stack: Vec::new(), anchors: HashMap::new(), expanded: 0, root: None };
    let mut parser = Parser::new_from_str(src);
    let mut docs = Vec::new();
    loop {
        let (ev, m) = parser.next_token().map_err(|e| err(YamlErrorKind::Syntax, *e.marker()))?;
        match ev {
            Event::StreamEnd => break,
            Event::DocumentStart => r.root = None,
            Event::DocumentEnd => {
                let root = r.root.take().unwrap_or_else(|| {
                    let (line, col) = pos(m);
                    Node { kind: NodeKind::Scalar { text: String::new(), style: Style::Plain }, line, col, tagged: false }
                });
                docs.push(root);
            }
            ev => r.event(ev, m)?,
        }
    }
    Ok((docs, r.expanded))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(n: &Node) -> &Vec<(Node, Node)> {
        match &n.kind {
            NodeKind::Map(m) => m,
            other => panic!("not a map: {other:?}"),
        }
    }

    fn scalar(n: &Node) -> (&str, Style) {
        match &n.kind {
            NodeKind::Scalar { text, style } => (text, *style),
            other => panic!("not a scalar: {other:?}"),
        }
    }

    #[test]
    fn scalar_style_is_kept() {
        let d = read_yaml("a: 'x'\nb: x\nc: \"x\"\nd: |\n  x\ne: >\n  x\n", Mode::Strict).unwrap();
        let m = map(&d[0]);
        let styles: Vec<Style> = m.iter().map(|(_, v)| scalar(v).1).collect();
        assert_eq!(styles, [Style::Single, Style::Plain, Style::Double, Style::Block, Style::Block]);
        assert_eq!(scalar(&m[0].0), ("a", Style::Plain));
    }

    #[test]
    fn duplicate_keys_survive_to_the_map() {
        let d = read_yaml("a: 1\na: 2\n", Mode::Strict).unwrap();
        let m = map(&d[0]);
        assert_eq!(m.len(), 2);
        assert_eq!(scalar(&m[1].1).0, "2");
    }

    #[test]
    fn strict_rejects_anchor_alias_tag() {
        for s in ["a: &x 'v'\n", "a: &x 'v'\nb: *x\n", "a: !t 'v'\n", "a: !!str 'v'\n", "a: &m {b: 1}\n", "a: !t [1]\n"] {
            assert_eq!(read_yaml(s, Mode::Strict).unwrap_err().kind, YamlErrorKind::Feature, "{s}");
        }
    }

    #[test]
    fn strict_depth_bound() {
        let deep = format!("a: {}1{}\n", "[".repeat(33), "]".repeat(33));
        assert_eq!(read_yaml(&deep, Mode::Strict).unwrap_err().kind, YamlErrorKind::Depth);
        let ok = format!("a: {}1{}\n", "[".repeat(31), "]".repeat(31));
        assert!(read_yaml(&ok, Mode::Strict).is_ok());
    }

    #[test]
    fn anchored_expands_aliases() {
        let d = read_yaml("base: &b {m: 1}\nprod: *b\n", Mode::Anchored).unwrap();
        let m = map(&d[0]);
        let prod = map(&m[1].1);
        assert_eq!(scalar(&prod[0].0).0, "m");
        assert_eq!(scalar(&prod[0].1).0, "1");
    }

    #[test]
    fn anchored_marks_tags() {
        let d = read_yaml("a: !Ref x\nb: y\n", Mode::Anchored).unwrap();
        let m = map(&d[0]);
        assert!(m[0].1.tagged);
        assert!(!m[1].1.tagged);
    }

    #[test]
    fn billion_laughs_hits_budget() {
        let mut s = String::from("a0: &a0 [x, x, x, x, x, x, x, x, x, x]\n");
        for i in 1..9 {
            let refs = vec![format!("*a{}", i - 1); 10].join(", ");
            s.push_str(&format!("a{i}: &a{i} [{refs}]\n"));
        }
        assert_eq!(read_yaml(&s, Mode::Anchored).unwrap_err().kind, YamlErrorKind::AliasBudget);
    }

    #[test]
    fn alias_into_depth_is_bounded() {
        let mut s = format!("base: &b {}1{}\n", "[".repeat(40), "]".repeat(40));
        s.push_str(&format!("deep: {}*b{}\n", "[".repeat(30), "]".repeat(30)));
        assert_eq!(read_yaml(&s, Mode::Anchored).unwrap_err().kind, YamlErrorKind::Depth);
    }

    #[test]
    fn an_alias_may_reach_the_depth_bound_exactly() {
        // A map's height counts the maps nested in it: `n` of them, aliased inside two lists under a key, sit at
        // 1 + 2 + n levels, which may equal the bound but not pass it.
        let doc = |n: usize| {
            let v = (0..n).fold(String::from("1"), |v, _| format!("{{k: {v}}}"));
            format!("base: &b {v}\ndeep: [[*b]]\n")
        };
        assert!(read_yaml(&doc(RESOLVER_DEPTH - 3), Mode::Anchored).is_ok());
        assert_eq!(read_yaml(&doc(RESOLVER_DEPTH - 2), Mode::Anchored).unwrap_err().kind, YamlErrorKind::Depth);
    }

    #[test]
    fn the_alias_budget_is_inclusive() {
        // A 999-byte anchored scalar copies 1,000 when anchored and again per alias: 999 aliases reach the bound.
        let doc = |aliases: usize| format!("a: &a {}\nb: [{}]\n", "x".repeat(999), vec!["*a"; aliases].join(", "));
        assert_eq!(alias_expansion(&doc(999)), Ok(ALIAS_EXPANSION_MAX));
        assert_eq!(alias_expansion(&doc(1000)).unwrap_err().kind, YamlErrorKind::AliasBudget);
    }

    #[test]
    fn alias_expansion_counts_what_the_budget_charges() {
        // `{m: 1}` copies five: the map one, each scalar one and its byte. Charged when anchored and per alias.
        assert_eq!(alias_expansion("base: &b {m: 1}\nprod: *b\nqa: *b\n"), Ok(15));
        assert_eq!(alias_expansion("a: 1\n---\nb: [1, 2]\n"), Ok(0));
        assert_eq!(alias_expansion("a: *nope\n").unwrap_err().kind, YamlErrorKind::Syntax);
    }

    #[test]
    fn alias_budget_charges_the_bytes_an_alias_copies() {
        // A long scalar aliased a few times copies its text each time: few nodes, a budget's worth of bytes.
        let n = ALIAS_EXPANSION_MAX / 4096 + 1;
        let s = format!("a: &a {}\nb: [{}]\n", "x".repeat(4096), vec!["*a"; n].join(", "));
        assert_eq!(read_yaml(&s, Mode::Anchored).err().map(|e| e.kind), Some(YamlErrorKind::AliasBudget));
        let s = format!("a: &a {}\nb: [{}]\n", "x".repeat(4095), vec!["*a"; n - 2].join(", "));
        assert!(read_yaml(&s, Mode::Anchored).is_ok());
    }

    #[test]
    fn alias_budget_charges_the_copy_an_anchor_keeps() {
        // Each anchored level keeps a copy of everything under it: nested anchors copy without one alias.
        let levels = 60;
        let inner = "x".repeat(ALIAS_EXPANSION_MAX / levels + 1);
        let s = (0..levels).fold(inner, |acc, i| format!("&a{i} [{acc}]"));
        assert_eq!(read_yaml(&s, Mode::Anchored).err().map(|e| e.kind), Some(YamlErrorKind::AliasBudget));
    }

    #[test]
    fn recursive_alias_does_not_parse() {
        assert_eq!(read_yaml("a: &a [1, *a]\n", Mode::Anchored).unwrap_err().kind, YamlErrorKind::Syntax);
    }

    #[test]
    fn merge_key_reads_as_an_ordinary_entry() {
        let d = read_yaml("x: &c {image: 'a:1'}\nsvc: {<<: *c, port: 80}\n", Mode::Anchored).unwrap();
        let svc = map(&map(&d[0])[1].1);
        assert_eq!(scalar(&svc[0].0), ("<<", Style::Plain));
        assert_eq!(scalar(&map(&svc[0].1)[0].1).0, "a:1");
    }

    #[test]
    fn multi_document() {
        assert_eq!(read_yaml("a: 1\n---\nb: 2\n", Mode::Anchored).unwrap().len(), 2);
        assert_eq!(read_yaml("", Mode::Anchored).unwrap().len(), 0);
    }

    #[test]
    fn single_quoted_folds_lines() {
        let d = read_yaml("t: 'a\n  b'\n", Mode::Strict).unwrap();
        assert_eq!(scalar(&map(&d[0])[0].1).0, "a b");
        let d = read_yaml("t: 'it''s'\n", Mode::Strict).unwrap();
        assert_eq!(scalar(&map(&d[0])[0].1).0, "it's");
    }

    #[test]
    fn marks_are_one_based() {
        let d = read_yaml("a: 1\nb: 2\nc:  'x'\n", Mode::Strict).unwrap();
        let m = map(&d[0]);
        assert_eq!((m[2].1.line, m[2].1.col), (3, 5));
        assert_eq!((m[0].0.line, m[0].0.col), (1, 1));
        let e = read_yaml("a: 1\nb: 2\nc: [1, 2\n", Mode::Strict).unwrap_err();
        assert_eq!(e.kind, YamlErrorKind::Syntax);
        assert!(e.line >= 3, "{e:?}");
        // An anchor or tag is reported at the node it decorates.
        let e = read_yaml("a: 1\nb: 'x'\nc:  &q 'x'\n", Mode::Strict).unwrap_err();
        assert_eq!((e.kind, e.line, e.col), (YamlErrorKind::Feature, 3, 8));
        // An alias to an anchor never defined does not parse, in either mode.
        assert_eq!(read_yaml("a: 'x'\nb: *q\n", Mode::Strict).unwrap_err().kind, YamlErrorKind::Syntax);
        assert_eq!(read_yaml("a: 'x'\nb: *q\n", Mode::Anchored).unwrap_err().kind, YamlErrorKind::Syntax);
    }
}
