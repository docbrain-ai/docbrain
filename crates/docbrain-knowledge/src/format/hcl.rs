// SPDX-License-Identifier: MIT
//! HCL attribute literals and block merging (`SEMANTICS.md` §Resolving values).

use std::collections::HashMap;

use hcl_edit::Span;
use hcl_edit::expr::{Expression, Object, ObjectKey};
use hcl_edit::structure::{BlockLabel, Body, Structure};
use hcl_edit::template::{Element, Template};

use crate::bounds::{HCL_ASSIGNMENTS_MAX, NUMBER_TEXT_MAX, RESOLVER_DEPTH};
use crate::reason::{Bound, Unverifiable};
use crate::scalar::{Decimal, Scalar};
use crate::value::{MapBuilder, Value};

/// One HCL file's body: its attributes (name, value, line) and its blocks, in file order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HclBody {
    pub attrs: Vec<(String, Value, usize)>,
    pub blocks: Vec<HclBlock>,
}

impl HclBody {
    /// An estimate of the heap this body holds, as `Value::heap`. Recursion follows the block nesting, which
    /// the parser bounds.
    pub(crate) fn heap(&self) -> usize {
        let attrs = self.attrs.iter().fold(self.attrs.capacity().saturating_mul(std::mem::size_of::<(String, Value, usize)>()), |n, (k, v, _)| {
            n.saturating_add(k.capacity()).saturating_add(v.heap())
        });
        self.blocks.iter().fold(attrs.saturating_add(self.blocks.capacity().saturating_mul(std::mem::size_of::<HclBlock>())), |n, b| {
            let labels = b.labels.iter().fold(b.labels.capacity().saturating_mul(std::mem::size_of::<String>()), |m, l| m.saturating_add(l.capacity()));
            n.saturating_add(b.ty.capacity()).saturating_add(labels).saturating_add(b.body.heap())
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HclBlock {
    pub ty: String,
    pub labels: Vec<String>,
    pub body: HclBody,
}

/// Reads one HCL file. A syntax error is `format`; an attribute or object key defined twice in one body is
/// `defined-twice`; nesting beyond the parser's limits (64 levels, 128 chained binary operators) is
/// `bound:resolver-depth`; more than `HCL_ASSIGNMENTS_MAX` `=` and `:` characters is `bound:hcl-assignments`.
/// An attribute's line is 1-based.
pub fn read_hcl(src: &str) -> Result<HclBody, Unverifiable> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    // One BOM is skipped (`SEMANTICS.md` §Resolving values).
    if src.starts_with('\u{feff}') {
        return Err(Unverifiable::Format);
    }
    if src.bytes().filter(|b| matches!(b, b'=' | b':')).count() > HCL_ASSIGNMENTS_MAX {
        return Err(Unverifiable::Bound(Bound::HclAssignments));
    }
    let body = hcl_edit::parser::parse_body(src).map_err(|e| classify(e.message()))?;
    let newlines: Vec<usize> = src.match_indices('\n').map(|(i, _)| i).collect();
    Ok(read_body(&body, &newlines))
}

/// hcl-edit's messages are built from fixed labels only, never from source text.
fn classify(message: &str) -> Unverifiable {
    if message.contains("found redefined attribute") || message.contains("found redefined object key") {
        Unverifiable::DefinedTwice
    } else if message.contains("at most 64 nested levels") || message.contains("chained binary operators") {
        Unverifiable::Bound(Bound::ResolverDepth)
    } else {
        Unverifiable::Format
    }
}

/// Recursion follows block nesting, which the parser holds to 64 levels.
fn read_body(body: &Body, newlines: &[usize]) -> HclBody {
    let mut out = HclBody::default();
    for s in body.iter() {
        match s {
            Structure::Attribute(a) => {
                let line = a.span().map_or(0, |r| newlines.partition_point(|&nl| nl < r.start) + 1);
                out.attrs.push((a.key.as_str().to_owned(), value(&a.value), line));
            }
            Structure::Block(b) => out.blocks.push(HclBlock {
                ty: b.ident.as_str().to_owned(),
                labels: b
                    .labels
                    .iter()
                    .map(|l| match l {
                        BlockLabel::Ident(i) => i.as_str().to_owned(),
                        BlockLabel::String(s) => s.as_str().to_owned(),
                    })
                    .collect(),
                body: read_body(&b.body, newlines),
            }),
        }
    }
    out
}

/// Recursion follows expression nesting, which the parser holds to 64 levels.
fn value(e: &Expression) -> Value {
    match e {
        Expression::Null(_) => Value::Scalar(Scalar::Null),
        Expression::Bool(b) => Value::Scalar(Scalar::Bool(*b.value())),
        Expression::Number(n) => number(n.as_repr().map(|r| &**r)),
        Expression::String(s) => Value::Scalar(Scalar::Str(s.as_str().to_owned())),
        Expression::Array(a) => Value::List(a.iter().map(value).collect()),
        Expression::Object(o) => object(o),
        Expression::StringTemplate(t) => literal(t),
        Expression::HeredocTemplate(h) => literal(&h.template),
        _ => Value::Opaque(Unverifiable::Expression),
    }
}

/// A number literal as written; hcl-edit writes a negative one as `-`, spacing or inline comments, the literal.
fn number(repr: Option<&str>) -> Value {
    let Some(text) = repr else { return Value::Opaque(Unverifiable::NotComparable) };
    let text = match text.strip_prefix('-') {
        Some(rest) => {
            let lead = rest.trim_end_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'));
            format!("-{}", &rest[lead.len()..])
        }
        None => text.to_owned(),
    };
    if text.len() > NUMBER_TEXT_MAX {
        return Value::Scalar(Scalar::Opaque(Unverifiable::Bound(Bound::NumberText)));
    }
    match Decimal::parse_number(&text) {
        Some(d) => Value::Scalar(Scalar::Number(d)),
        None => Value::Scalar(Scalar::Opaque(Unverifiable::NotComparable)),
    }
}

/// An object whose key is an expression has keys nobody can read from the file: the whole object is one.
fn object(o: &Object) -> Value {
    let mut m = MapBuilder::default();
    for (k, v) in o.iter() {
        let key = match k {
            ObjectKey::Ident(i) => i.as_str().to_owned(),
            ObjectKey::Expression(Expression::String(s)) => s.as_str().to_owned(),
            ObjectKey::Expression(_) => return Value::Opaque(Unverifiable::Expression),
        };
        m.insert(key, value(v.expr()));
    }
    m.finish(false)
}

/// a template with any interpolation or directive is an expression.
fn literal(t: &Template) -> Value {
    let mut s = String::new();
    for e in t.iter() {
        match e {
            Element::Literal(l) => s.push_str(l),
            _ => return Value::Opaque(Unverifiable::Expression),
        }
    }
    Value::Scalar(Scalar::Str(s))
}

/// the files of one glob as one value. Top-level blocks key by type then labels and merge by attribute;
/// an attribute set twice is `defined-twice`. A nested block type (with its labels) is its body when it occurs
/// once and a list when it repeats. A `dynamic "x"` block makes `x` an `expression`. Depth counts every map
/// and list from the root (root = 1); beyond `RESOLVER_DEPTH` anywhere, the whole value is
/// `bound:resolver-depth`.
pub fn merge_files(bodies: &[HclBody]) -> Value {
    let too_deep = Value::Opaque(Unverifiable::Bound(Bound::ResolverDepth));
    let mut root = Tree::default();
    for body in bodies {
        if fill(&mut root, body, 1).is_err() {
            return too_deep;
        }
    }
    finish(root, 1).unwrap_or(too_deep)
}

struct TooDeep;

/// A map being merged. The slot rules commute, so the answer does not depend on file or block order.
#[derive(Default)]
struct Tree {
    slots: Vec<(String, Slot)>,
    index: HashMap<String, usize>,
}

enum Slot {
    Attr(Value),
    /// A top-level block type, a label, or a nested block type that has labels.
    Tree(Tree),
    /// The bodies of a nested block type (and labels), in order.
    Nested(Vec<Tree>),
    /// A `dynamic` block makes blocks nobody can read from the file.
    Expression,
    Twice,
}

impl Tree {
    fn find(&mut self, name: &str) -> Option<&mut Slot> {
        let i = *self.index.get(name)?;
        self.slots.get_mut(i).map(|(_, s)| s)
    }

    fn get_or(&mut self, name: &str, new: impl FnOnce() -> Slot) -> Option<&mut Slot> {
        if !self.index.contains_key(name) {
            self.index.insert(name.to_owned(), self.slots.len());
            self.slots.push((name.to_owned(), new()));
        }
        self.find(name)
    }

    fn attr(&mut self, name: &str, v: &Value) {
        match self.find(name) {
            Some(s) => *s = Slot::Twice,
            None => {
                self.get_or(name, || Slot::Attr(v.clone()));
            }
        }
    }

    fn dynamic(&mut self, name: &str) {
        match self.get_or(name, || Slot::Expression) {
            Some(s @ (Slot::Tree(_) | Slot::Nested(_))) => *s = Slot::Expression,
            Some(s @ Slot::Attr(_)) => *s = Slot::Twice,
            _ => {}
        }
    }

    fn tree(&mut self, name: &str) -> Option<&mut Tree> {
        match self.get_or(name, || Slot::Tree(Tree::default()))? {
            Slot::Tree(t) => Some(t),
            Slot::Expression | Slot::Twice => None,
            s => {
                *s = Slot::Twice;
                None
            }
        }
    }

    fn nested(&mut self, name: &str) -> Option<&mut Vec<Tree>> {
        match self.get_or(name, || Slot::Nested(Vec::new()))? {
            Slot::Nested(list) => Some(list),
            Slot::Expression | Slot::Twice => None,
            s => {
                *s = Slot::Twice;
                None
            }
        }
    }
}

/// Adds `body` to `tree`, a map at `depth`. Top-level blocks (at the root) descend by type and labels and merge;
/// nested ones are grouped. A label chain longer than the depth allows is refused before it is built, so no
/// tree is deeper than `RESOLVER_DEPTH` and recursion follows the parser's block nesting.
fn fill(tree: &mut Tree, body: &HclBody, depth: usize) -> Result<(), TooDeep> {
    for (k, v, _) in &body.attrs {
        tree.attr(k, v);
    }
    for b in &body.blocks {
        if let ("dynamic", [label]) = (b.ty.as_str(), b.labels.as_slice()) {
            tree.dynamic(label);
            continue;
        }
        // The body's map is at least one level below the type, and one more per label.
        let body_depth = depth + 1 + b.labels.len();
        if body_depth > RESOLVER_DEPTH {
            return Err(TooDeep);
        }
        if depth == 1 {
            let mut t = tree.tree(&b.ty);
            for l in &b.labels {
                t = t.and_then(|t| t.tree(l));
            }
            if let Some(t) = t {
                fill(t, &b.body, body_depth)?;
            }
        } else {
            let list = match b.labels.split_last() {
                None => tree.nested(&b.ty),
                Some((last, chain)) => {
                    let mut t = tree.tree(&b.ty);
                    for l in chain {
                        t = t.and_then(|t| t.tree(l));
                    }
                    t.and_then(|t| t.nested(last))
                }
            };
            if let Some(list) = list {
                let mut t = Tree::default();
                fill(&mut t, &b.body, body_depth)?;
                list.push(t);
            }
        }
    }
    Ok(())
}

/// `tree` as a map at `depth`, checked exactly: a repeated nested block's bodies sit one level deeper, in a list.
fn finish(tree: Tree, depth: usize) -> Result<Value, TooDeep> {
    if depth > RESOLVER_DEPTH {
        return Err(TooDeep);
    }
    let mut entries = Vec::with_capacity(tree.slots.len());
    for (k, slot) in tree.slots {
        let v = match slot {
            Slot::Attr(v) => {
                fits(&v, depth + 1)?;
                v
            }
            Slot::Tree(t) => finish(t, depth + 1)?,
            Slot::Nested(mut list) => {
                if list.len() == 1
                    && let Some(one) = list.pop()
                {
                    finish(one, depth + 1)?
                } else {
                    // The list's own level is checked by its bodies', one deeper.
                    Value::List(list.into_iter().map(|t| finish(t, depth + 2)).collect::<Result<_, _>>()?)
                }
            }
            Slot::Expression => Value::Opaque(Unverifiable::Expression),
            Slot::Twice => Value::Opaque(Unverifiable::DefinedTwice),
        };
        entries.push((k, v));
    }
    Ok(Value::Map { entries, merge_key: false })
}

/// An attribute's value, a map or list at `depth` when it is one.
fn fits(v: &Value, depth: usize) -> Result<(), TooDeep> {
    let children: Box<dyn Iterator<Item = &Value>> = match v {
        Value::Map { entries, .. } => Box::new(entries.iter().map(|(_, v)| v)),
        Value::List(items) => Box::new(items.iter()),
        _ => return Ok(()),
    };
    if depth > RESOLVER_DEPTH {
        return Err(TooDeep);
    }
    children.into_iter().try_for_each(|c| fits(c, depth + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bounds::{HCL_ASSIGNMENTS_MAX, NUMBER_TEXT_MAX, RESOLVER_DEPTH};
    use crate::reason::Bound;
    use crate::scalar::{Decimal, Scalar};

    fn num(s: &str) -> Value {
        Value::Scalar(Scalar::Number(Decimal::parse_number(s).unwrap()))
    }
    fn st(s: &str) -> Value {
        Value::Scalar(Scalar::Str(s.into()))
    }
    fn map(entries: Vec<(&str, Value)>) -> Value {
        Value::Map { entries: entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(), merge_key: false }
    }
    fn at<'a>(v: &'a Value, path: &[&str]) -> &'a Value {
        let mut v = v;
        for key in path {
            v = match v {
                Value::Map { entries, .. } => &entries.iter().find(|(k, _)| k == key).unwrap_or_else(|| panic!("no {key} in {v:?}")).1,
                other => panic!("not a map at {key}: {other:?}"),
            };
        }
        v
    }
    fn merged(files: &[&str]) -> Value {
        merge_files(&files.iter().map(|f| read_hcl(f).unwrap()).collect::<Vec<_>>())
    }
    /// The value of attribute `a` in a one-attribute file.
    fn attr(value: &str) -> Value {
        at(&merged(&[&format!("a = {value}\n")]), &["a"]).clone()
    }
    const EXPRESSION: Value = Value::Opaque(Unverifiable::Expression);

    #[test]
    fn resource_attribute_literal() {
        let v = merged(&["resource \"aws_db_instance\" \"prod\" {\n  multi_az = true\n  engine = \"postgres\"\n}\n"]);
        assert_eq!(at(&v, &["resource", "aws_db_instance", "prod", "multi_az"]), &Value::Scalar(Scalar::Bool(true)));
        assert_eq!(at(&v, &["resource", "aws_db_instance", "prod", "engine"]), &st("postgres"));
    }

    #[test]
    fn terraform_blocks_merge_across_files() {
        let v = merged(&["terraform {\n  required_version = \">= 1.5\"\n}\n", "terraform {\n  backend \"s3\" {\n    bucket = \"b\"\n  }\n}\n"]);
        assert_eq!(
            at(&v, &["terraform"]),
            &map(vec![("required_version", st(">= 1.5")), ("backend", map(vec![("s3", map(vec![("bucket", st("b"))]))]))])
        );
        // The same within one file.
        let v = merged(&["locals {\n  a = 1\n}\nlocals {\n  b = 2\n}\n"]);
        assert_eq!(at(&v, &["locals"]), &map(vec![("a", num("1")), ("b", num("2"))]));
    }

    #[test]
    fn attribute_in_two_files_defined_twice() {
        let v = merged(&["terraform {\n  required_version = \">= 1.5\"\n}\n", "terraform {\n  required_version = \">= 1.6\"\n}\n"]);
        assert_eq!(at(&v, &["terraform", "required_version"]), &Value::Opaque(Unverifiable::DefinedTwice));
        // Top-level attributes (a .tfvars chain is layers, not a glob; one glob merges).
        let v = merged(&["region = \"a\"\n", "region = \"b\"\nzone = 1\n"]);
        assert_eq!(v, map(vec![("region", Value::Opaque(Unverifiable::DefinedTwice)), ("zone", num("1"))]));
        // Within one body the parser refuses it.
        assert_eq!(read_hcl("a = 1\na = 2\n"), Err(Unverifiable::DefinedTwice));
        assert_eq!(read_hcl("b {\n  a = 1\n  a = 2\n}\n"), Err(Unverifiable::DefinedTwice));
    }

    #[test]
    fn repeated_nested_block_is_list() {
        let sg = "resource \"aws_security_group\" \"web\" {\n  ingress {\n    from_port = 443\n  }\n  ingress {\n    from_port = 80\n  }\n  egress {\n    from_port = 0\n  }\n}\n";
        let v = merged(&[sg]);
        let web = at(&v, &["resource", "aws_security_group", "web"]);
        assert_eq!(at(web, &["ingress"]), &Value::List(vec![map(vec![("from_port", num("443"))]), map(vec![("from_port", num("80"))])]));
        assert_eq!(at(web, &["egress"]), &map(vec![("from_port", num("0"))]));
        // Nested blocks never merge, also across files: two `backend "s3"` are a list.
        let v = merged(&["terraform {\n  backend \"s3\" {\n    a = 1\n  }\n}\n", "terraform {\n  backend \"s3\" {\n    b = 2\n  }\n}\n"]);
        assert_eq!(at(&v, &["terraform", "backend", "s3"]), &Value::List(vec![map(vec![("a", num("1"))]), map(vec![("b", num("2"))])]));
    }

    #[test]
    fn expressions_are_opaque() {
        for e in ["var.x", "local.y", "max(1,2)", "\"${var.x}\"", "a ? 1 : 2", "\"%{if true}x%{endif}\"", "[for x in var.l : x]", "(1)", "!true", "1 + 2", "- -1", "a.b[0]", "<<EOT\n${x}\nEOT"] {
            assert_eq!(attr(e), EXPRESSION, "{e}");
        }
        // A list or map is read element by element.
        assert_eq!(attr("[1, var.x]"), Value::List(vec![num("1"), EXPRESSION]));
        // An object whose key is an expression has unknown keys: the whole object is an expression.
        assert_eq!(attr("{ (var.k) = 1, b = 2 }"), EXPRESSION);
        // `$${` is an escape, not an interpolation.
        assert_eq!(attr("\"$${x}\""), st("${x}"));
    }

    #[test]
    fn dynamic_block_is_an_expression() {
        let v = merged(&["resource \"r\" \"n\" {\n  dynamic \"ingress\" {\n    for_each = var.l\n    content {\n      port = ingress.value\n    }\n  }\n  ingress {\n    port = 1\n  }\n  name = \"x\"\n}\n"]);
        // A body's attributes come before its blocks in the merged map.
        assert_eq!(at(&v, &["resource", "r", "n"]), &map(vec![("name", st("x")), ("ingress", EXPRESSION)]));
    }

    #[test]
    fn negative_and_exact_numbers() {
        assert_eq!(attr("-1"), num("-1"));
        assert_eq!(attr("-  5"), num("-5"));
        assert_eq!(attr("0.10"), num("0.1"));
        assert_eq!(attr("1.5e3"), num("1500"));
        assert_eq!(attr("007"), num("7"));
        assert_eq!(attr("123456789012345678901234567890"), num("123456789012345678901234567890"));
        assert_eq!(attr("null"), Value::Scalar(Scalar::Null));
        assert_eq!(attr("false"), Value::Scalar(Scalar::Bool(false)));
        assert_eq!(read_hcl("a = 1e400\n"), Err(Unverifiable::Format));
    }

    #[test]
    fn object_literal_traversable() {
        let v = merged(&["terraform {\n  required_providers {\n    aws = {\n      source = \"hashicorp/aws\"\n      version = \"~> 5.0\"\n    }\n  }\n}\n"]);
        assert_eq!(at(&v, &["terraform", "required_providers", "aws", "version"]), &st("~> 5.0"));
        assert_eq!(attr("{ \"a.b\" = 1, c: [2] }"), map(vec![("a.b", num("1")), ("c", Value::List(vec![num("2")]))]));
        // The parser refuses `k` twice; `k` and `"k"` are the same key by text.
        assert_eq!(read_hcl("a = { k = 1, k = 2 }\n"), Err(Unverifiable::DefinedTwice));
        assert_eq!(attr("{ k = 1, \"k\" = 2 }"), map(vec![("k", Value::Opaque(Unverifiable::DefinedTwice))]));
    }

    #[test]
    fn heap_counts_attributes_and_nested_bodies() {
        let body = read_hcl(&format!("a = \"{}\"\nb {{\n  c = \"{}\"\n}}\n", "x".repeat(1000), "y".repeat(1000))).unwrap();
        assert!(body.heap() >= 2000, "{}", body.heap());
    }

    #[test]
    fn heredoc_without_interpolation_is_string() {
        assert_eq!(attr("<<EOT\nhi\n  there\nEOT"), st("hi\n  there\n"));
        assert_eq!(attr("<<-EOT\n    hi\n      there\n    EOT"), st("hi\n  there\n"));
    }

    #[test]
    fn syntax_error_is_format() {
        for t in ["a = ", "a = (", "a = \"x", "block {", "a = 1 2\n", "= 1\n", "\u{feff}\u{feff}a = 1\n"] {
            assert_eq!(read_hcl(t), Err(Unverifiable::Format), "{t:?}");
        }
        // one BOM is skipped.
        assert_eq!(merge_files(&[read_hcl("\u{feff}a = 1\n").unwrap()]), map(vec![("a", num("1"))]));
        assert_eq!(merge_files(&[read_hcl("").unwrap()]), map(vec![]));
    }

    #[test]
    fn attribute_lines() {
        let b = read_hcl("# c\na = 1\n\nblk {\n  b = 2\n}\n").unwrap();
        assert_eq!(b.attrs.iter().map(|(k, _, l)| (k.as_str(), *l)).collect::<Vec<_>>(), vec![("a", 2)]);
        assert_eq!(b.blocks[0].body.attrs[0].2, 5);
        assert_eq!((b.blocks[0].ty.as_str(), b.blocks[0].labels.len()), ("blk", 0));
    }

    #[test]
    fn collisions_are_defined_twice() {
        // An attribute and a block of the same name in one body.
        let v = merged(&["r {\n  tags = {}\n  tags {\n  }\n}\n"]);
        assert_eq!(at(&v, &["r", "tags"]), &Value::Opaque(Unverifiable::DefinedTwice));
        // A top-level attribute and a top-level block type.
        let v = merged(&["x = 1\n", "x {\n}\n"]);
        assert_eq!(at(&v, &["x"]), &Value::Opaque(Unverifiable::DefinedTwice));
    }

    #[test]
    fn depth_bound() {
        // Labels nest: root, the type, then one map per label; the body is the last of them.
        let labelled = |n: usize| format!("a {} {{\n}}\n", vec!["\"l\""; n].join(" "));
        assert!(matches!(merged(&[&labelled(RESOLVER_DEPTH - 2)]), Value::Map { .. }));
        assert_eq!(merged(&[&labelled(RESOLVER_DEPTH - 1)]), Value::Opaque(Unverifiable::Bound(Bound::ResolverDepth)));
        // Far beyond: no deep value is built, so nothing overflows.
        assert_eq!(merged(&[&labelled(200_000)]), Value::Opaque(Unverifiable::Bound(Bound::ResolverDepth)));
        // The parser's own limit.
        assert_eq!(read_hcl(&format!("a = {}1{}\n", "[".repeat(70), "]".repeat(70))), Err(Unverifiable::Bound(Bound::ResolverDepth)));
        // An attribute's own nesting counts on top of the blocks around it: n labels put the body at n + 2.
        let nested_list = |n: usize| merged(&[&format!("a {} {{\n  x = [[1]]\n}}\n", vec!["\"l\""; n].join(" "))]);
        assert!(matches!(nested_list(RESOLVER_DEPTH - 4), Value::Map { .. }));
        assert_eq!(nested_list(RESOLVER_DEPTH - 3), Value::Opaque(Unverifiable::Bound(Bound::ResolverDepth)));
        // So does an object's: a map in a map counts like a list in a list.
        let nested_map = |n: usize| merged(&[&format!("a {} {{\n  x = {{ m = {{ k = 1 }} }}\n}}\n", vec!["\"l\""; n].join(" "))]);
        assert!(matches!(nested_map(RESOLVER_DEPTH - 4), Value::Map { .. }));
        assert_eq!(nested_map(RESOLVER_DEPTH - 3), Value::Opaque(Unverifiable::Bound(Bound::ResolverDepth)));
        // A repeated nested block's bodies sit in a list, one level deeper than a single body.
        let nested_block = |n: usize, times: usize| {
            merged(&[&format!("a {} {{\n{}}}\n", vec!["\"l\""; n].join(" "), "  b {\n  }\n".repeat(times))])
        };
        assert!(matches!(nested_block(RESOLVER_DEPTH - 3, 1), Value::Map { .. }));
        assert_eq!(nested_block(RESOLVER_DEPTH - 3, 2), Value::Opaque(Unverifiable::Bound(Bound::ResolverDepth)));
        assert!(matches!(nested_block(RESOLVER_DEPTH - 4, 2), Value::Map { .. }));
        // A single nested body is as deep as a repeated one's list: one label more reaches the bound.
        assert_eq!(nested_block(RESOLVER_DEPTH - 2, 1), Value::Opaque(Unverifiable::Bound(Bound::ResolverDepth)));
        // What a single nested body holds counts from that body's own level, one below its parent's.
        let inside_nested = |n: usize| merged(&[&format!("a {} {{\n  b {{\n    x = [[1]]\n  }}\n}}\n", vec!["\"l\""; n].join(" "))]);
        assert!(matches!(inside_nested(RESOLVER_DEPTH - 5), Value::Map { .. }));
        assert_eq!(inside_nested(RESOLVER_DEPTH - 4), Value::Opaque(Unverifiable::Bound(Bound::ResolverDepth)));
        // A nested block's labels count too, and a label chain far beyond is refused before it is built.
        let nested_labels = |n: usize| merged(&[&format!("a {{\n  b {} {{\n  }}\n}}\n", vec!["\"l\""; n].join(" "))]);
        assert!(matches!(nested_labels(RESOLVER_DEPTH - 3), Value::Map { .. }));
        assert_eq!(nested_labels(RESOLVER_DEPTH - 2), Value::Opaque(Unverifiable::Bound(Bound::ResolverDepth)));
        assert_eq!(nested_labels(200_000), Value::Opaque(Unverifiable::Bound(Bound::ResolverDepth)));
        // The parser's other limit: 128 chained binary operators.
        assert_eq!(read_hcl(&format!("a = 1{}\n", " + 1".repeat(200))), Err(Unverifiable::Bound(Bound::ResolverDepth)));
        // One file too deep makes the whole glob's answer.
        let deep = read_hcl(&labelled(RESOLVER_DEPTH)).unwrap();
        let fine = read_hcl("b = 1\n").unwrap();
        assert_eq!(merge_files(&[fine, deep]), Value::Opaque(Unverifiable::Bound(Bound::ResolverDepth)));
    }

    #[test]
    fn attack_pass() {
        // A long number is the number-text bound, like every format's.
        let long = format!("0.{}1", "0".repeat(NUMBER_TEXT_MAX));
        assert_eq!(attr(&long), Value::Scalar(Scalar::Opaque(Unverifiable::Bound(Bound::NumberText))));
        assert_eq!(attr(&format!("-{}", &long[..NUMBER_TEXT_MAX - 1])), num(&format!("-{}", &long[..NUMBER_TEXT_MAX - 1])));
        // `-`, an inline comment, then the literal.
        assert_eq!(attr("- /* 9 */ 1e-2"), num("-0.01"));
        // Messages never quote source text, so a value spelled like one changes nothing.
        assert_eq!(read_hcl("a = \"found redefined attribute\"\nb = (\n"), Err(Unverifiable::Format));
        assert_eq!(read_hcl("a = \"at most 64 nested levels\"\nb = (\n"), Err(Unverifiable::Format));
        // The same quoted key twice.
        assert_eq!(read_hcl("a = { \"k\" = 1, \"k\" = 2 }\n"), Err(Unverifiable::DefinedTwice));
        // Order does not change the answer: a dynamic block before or after the static ones.
        let before = merged(&["r {\n  dynamic \"i\" {\n  }\n  i {\n  }\n}\n"]);
        let after = merged(&["r {\n  i {\n  }\n  dynamic \"i\" {\n  }\n}\n"]);
        assert_eq!(before, after);
        assert_eq!(at(&before, &["r", "i"]), &EXPRESSION);
        // A dynamic block for a name that is also an attribute is defined twice, in either order.
        for f in ["r {\n  i = 1\n  dynamic \"i\" {\n  }\n}\n", "r {\n  dynamic \"i\" {\n  }\n  i = 1\n}\n"] {
            assert_eq!(at(&merged(&[f]), &["r", "i"]), &Value::Opaque(Unverifiable::DefinedTwice), "{f:?}");
        }
        // A dynamic block for a labelled nested type covers every label.
        let v = merged(&["r {\n  backend \"s3\" {\n  }\n  dynamic \"backend\" {\n  }\n}\n"]);
        assert_eq!(at(&v, &["r", "backend"]), &EXPRESSION);
        // A nested type with and without labels in one body.
        let v = merged(&["r {\n  b {\n  }\n  b \"l\" {\n  }\n}\n"]);
        assert_eq!(at(&v, &["r", "b"]), &Value::Opaque(Unverifiable::DefinedTwice));
        // A top-level type with and without labels keeps both, keyed apart; a body attribute named like a label
        // collides with it.
        let v = merged(&["x {\n  a = 1\n}\n", "x \"l\" {\n  b = 2\n}\n"]);
        assert_eq!(at(&v, &["x"]), &map(vec![("a", num("1")), ("l", map(vec![("b", num("2"))]))]));
        let v = merged(&["x {\n  l = 1\n}\n", "x \"l\" {\n}\n"]);
        assert_eq!(at(&v, &["x", "l"]), &Value::Opaque(Unverifiable::DefinedTwice));
        // Three files, one attribute in all three: still defined twice, once.
        let v = merged(&["a = 1\n", "a = 1\n", "a = 1\n"]);
        assert_eq!(v, map(vec![("a", Value::Opaque(Unverifiable::DefinedTwice))]));
        // An identifier label and a quoted one with the same text are the same key.
        let v = merged(&["r x {\n  a = 1\n}\n", "r \"x\" {\n  b = 2\n}\n"]);
        assert_eq!(at(&v, &["r", "x"]), &map(vec![("a", num("1")), ("b", num("2"))]));
        // Multibyte text survives, in a value, a key and a label.
        let v = merged(&["r \"café\" {\n  a = { \"ключ\" = \"日本\" }\n}\n"]);
        assert_eq!(at(&v, &["r", "café", "a", "ключ"]), &st("日本"));
        // CRLF line ends.
        let b = read_hcl("a = 1\r\nb = \"x\"\r\n").unwrap();
        assert_eq!(b.attrs.iter().map(|(k, v, l)| (k.as_str(), v.clone(), *l)).collect::<Vec<_>>(), vec![("a", num("1"), 1), ("b", st("x"), 2)]);
        // No bodies is an empty map.
        assert_eq!(merge_files(&[]), map(vec![]));
    }

    proptest::proptest! {
        #[test]
        fn never_panics(s in "[a-z \"=:{}\\[\\]()\\n<>$%#./*+-1e,?!]{0,80}") {
            if let Ok(b) = read_hcl(&s) {
                merge_files(&[b.clone(), b]);
            }
        }
    }

    #[test]
    fn assignment_bound() {
        let at_bound = (0..HCL_ASSIGNMENTS_MAX).map(|i| format!("k{i} = 1\n")).collect::<String>();
        assert!(read_hcl(&at_bound).is_ok());
        // `:` counts like `=`, wherever it is.
        let over = format!("{at_bound}# :\n");
        assert_eq!(read_hcl(&over), Err(Unverifiable::Bound(Bound::HclAssignments)));
        // The case the bound exists for: one object with many keys costs the square of its size to parse.
        let object = format!("a = {{ {} }}\n", (0..HCL_ASSIGNMENTS_MAX + 1).map(|i| format!("k{i} = 1")).collect::<Vec<_>>().join(", "));
        assert_eq!(read_hcl(&object), Err(Unverifiable::Bound(Bound::HclAssignments)));
    }
}
