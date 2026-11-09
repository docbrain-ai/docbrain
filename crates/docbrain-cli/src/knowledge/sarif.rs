// SPDX-License-Identifier: MIT
//! `docbrain status --base --sarif`: the gate's report as one SARIF 2.1.0 run. Every finding, every finding on
//! the change and every knowledge change is a result; none is capped (edge case 35).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use docbrain_knowledge::gate::{ChangeFinding, Class, GateReport};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::frame::esc;
use super::gate::{ON_PURPOSE, change_text};
use super::show::WITHHELD_VALUE;

const SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";
const SRCROOT: &str = "%SRCROOT%";
const FINGERPRINT: &str = "docbrainKnowledge/v1";

/// Each rule: id, level, one line.
const RULES: [(&str, &str, &str); 8] = [
    ("knowledge/broken", "warning", "A constraint a record states is false after this change."),
    (
        "knowledge/changed",
        "warning",
        "This change removed, superseded, relaxed or unread a constraint a record states.",
    ),
    ("knowledge/stale", "note", "An assumption a record rests on no longer holds."),
    (
        "knowledge/knowledge-change",
        "note",
        "This change added, edited or removed a record, or a check it adds cannot be judged.",
    ),
    ("knowledge/duplicate-id", "warning", "Two record files carry the same id."),
    ("knowledge/supersedes", "warning", "A record supersedes an id it cannot."),
    ("knowledge/cycle", "warning", "Records supersede one another in a cycle."),
    ("knowledge/secret-shape", "warning", "A record file holds a value shaped like a secret."),
];

/// What SARIF needs that the report does not carry, read from the trees: each finding's anchored file, by record
/// path and anchor, and the record file of each id in a cycle.
#[derive(Default)]
pub(crate) struct Files {
    pub anchored: BTreeMap<(String, String), String>,
    pub of_id: BTreeMap<String, String>,
}

pub(crate) fn sarif(r: &GateReport, files: &Files, version: &str) -> Value {
    let mut results = Vec::new();
    for f in &r.findings {
        let (rule, word, number) = match &f.class {
            Class::Broken { rule } => (0, "broken", format!(" (rule {rule})")),
            Class::Changed(_) => (1, "changed", String::new()),
            Class::Stale => (2, "stale", String::new()),
        };
        let file = if let Some(shown) = f.anchor.strip_suffix(WITHHELD_VALUE) {
            // A withheld anchor is located at the file it still names, else at its record: never at the secret.
            shown.strip_suffix('#').unwrap_or(&f.record_path)
        } else {
            match files.anchored.get(&(f.record_path.clone(), f.anchor.clone())) {
                Some(file) => file.as_str(),
                None => f.anchor.split_once('#').map_or(f.anchor.as_str(), |(file, _)| file),
            }
        };
        let mut lines = vec![format!("{word} {}{number}", esc(&f.anchor))];
        if !f.title.is_empty() {
            lines.push(esc(&f.title).into_owned());
        }
        lines.extend(f.reasons.iter().map(|reason| format!("reason: {}", esc(reason))));
        if let Some(p) = &f.same_content {
            lines.push(format!("same content at {}", esc(p)));
        }
        let at = Place { file, related: vec![&f.record_path], region: None };
        results.push(result(rule, lines.join("\n"), at, f.id.as_str(), &f.anchor));
    }
    for c in &r.change_findings {
        let text = change_text(c);
        let (rule, at, id) = match c {
            ChangeFinding::DuplicateId { id, paths } => (4, Place::of(paths.iter().map(String::as_str)), id.as_str()),
            ChangeFinding::Supersedes { path, .. } => (5, Place::of([path.as_str()]), ""),
            ChangeFinding::Cycle { ids } => {
                let paths = ids.iter().filter_map(|id| files.of_id.get(id.as_str()).map(String::as_str));
                (6, Place::of(paths), ids.first().map_or("", |id| id.as_str()))
            }
            ChangeFinding::SecretShape { path, line, col, .. } => {
                (7, Place { region: Some((*line, *col)), ..Place::of([path.as_str()]) }, "")
            }
        };
        let place = at.file;
        results.push(result(rule, text, at, id, place));
    }
    let k = &r.knowledge;
    let changes = [("added", &k.added), ("edited", &k.edited), ("removed", &k.removed)];
    for (word, paths) in changes {
        for p in paths {
            results.push(result(3, format!("knowledge {word} {}", esc(p)), Place::of([p.as_str()]), "", p));
        }
    }
    for (from, to) in &k.followed {
        let text = format!("knowledge followed {} -> {}", esc(from), esc(to));
        let at = Place::of(Some(to.as_str()).filter(|to| !to.ends_with(WITHHELD_VALUE)));
        results.push(result(3, text, at, "", to));
    }
    for (p, i, code) in &k.new_unverifiable {
        let text = format!("knowledge not judged {} check {}: {}", esc(p), i + 1, esc(code));
        results.push(result(3, text, Place::of([p.as_str()]), "", &format!("{p}\0{}", i + 1)));
    }
    // Two results with one fingerprint would be one alert to a code scanner (two withheld anchors on one record, two
    // secret shapes in one file): the n-th, from the second on, hashes `<the fingerprint>\0<n>` instead. Never the
    // anchor as written: a hash of a short secret can be reversed.
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for v in &mut results {
        let fingerprint = &mut v["partialFingerprints"][FINGERPRINT];
        let first = fingerprint.as_str().unwrap_or_default().to_owned();
        let n = seen.entry(first.clone()).or_default();
        *n += 1;
        if *n > 1 {
            *fingerprint = json!(hex::encode(Sha256::digest(format!("{first}\0{n}"))));
        }
    }
    let rules: Vec<Value> = RULES
        .iter()
        .map(|(id, level, text)| {
            let mut rule =
                json!({"id": id, "shortDescription": {"text": text}, "defaultConfiguration": {"level": level}});
            if *id == "knowledge/broken" {
                rule["help"] = json!({"text": ON_PURPOSE});
            }
            rule
        })
        .collect();
    json!({
        "$schema": SCHEMA,
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {"name": "docbrain", "version": version, "rules": rules}},
            "columnKind": "unicodeCodePoints",
            "results": results,
        }],
    })
}

/// Where a result is: its file, the files related to it, and a line and column when it has one.
struct Place<'a> {
    file: &'a str,
    related: Vec<&'a str>,
    region: Option<(u32, u32)>,
}

impl<'a> Place<'a> {
    /// The first of `paths`, the rest related; none at all when `paths` is empty.
    fn of(paths: impl IntoIterator<Item = &'a str>) -> Place<'a> {
        let mut paths = paths.into_iter();
        Place { file: paths.next().unwrap_or_default(), related: paths.collect(), region: None }
    }
}

fn location(path: &str) -> Value {
    json!({"physicalLocation": {"artifactLocation": {"uri": uri(path), "uriBaseId": SRCROOT}}})
}

/// `<rule id>\0<record id or "">\0<anchor or file>`, hashed.
fn result(rule: usize, text: String, at: Place<'_>, record_id: &str, place: &str) -> Value {
    let (id, level, _) = RULES[rule];
    let fingerprint = hex::encode(Sha256::digest(format!("{id}\0{record_id}\0{place}")));
    let mut v = json!({
        "ruleId": id,
        "ruleIndex": rule,
        "level": level,
        "message": {"text": text},
        "partialFingerprints": {FINGERPRINT: fingerprint},
    });
    if !at.file.is_empty() {
        let mut here = location(at.file);
        if let Some((line, col)) = at.region {
            here["physicalLocation"]["region"] = json!({"startLine": line, "startColumn": col});
        }
        v["locations"] = json!([here]);
    }
    if !at.related.is_empty() {
        v["relatedLocations"] = at.related.iter().map(|p| location(p)).collect();
    }
    v
}

/// `path` as a URI reference: RFC 3986 unreserved characters and `/` kept, every other byte percent-encoded.
fn uri(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use docbrain_knowledge::folder::SupersedeProblem;
    use docbrain_knowledge::gate::{ChangeFinding, ChangeKind, Class, Finding, KnowledgeChanges};
    use docbrain_knowledge::record::Id;
    use serde_json::json;
    use sha2::{Digest, Sha256};

    fn id(s: &str) -> Id {
        Id::parse(s).unwrap()
    }

    fn finding(class: Class, path: &str, anchor: &str) -> Finding {
        Finding {
            class,
            id: id("7k2m9q4r8t1v"),
            record_path: path.to_owned(),
            anchor: anchor.to_owned(),
            title: "Title".to_owned(),
            first_paragraph: "Why.".to_owned(),
            reasons: vec!["missing-file".to_owned()],
            same_content: None,
            read_at_head: Default::default(),
        }
    }

    const RECORD: &str = "a/.knowledge/records/r1.md";

    /// One result of every kind, in the order SARIF lists them.
    fn every_kind() -> (GateReport, Files) {
        let r = GateReport {
            findings: vec![
                finding(Class::Broken { rule: 1 }, RECORD, "a/v.yaml#replicas"),
                finding(Class::Changed(ChangeKind::Superseded), "r2.md", "b.md"),
                finding(Class::Stale, "r3.md", "c.md"),
            ],
            change_findings: vec![
                ChangeFinding::DuplicateId { id: id("7k2m9q4r8t1v"), paths: vec!["d1.md".into(), "d2.md".into()] },
                ChangeFinding::Supersedes { path: "s.md".into(), problem: SupersedeProblem::NotInFolder },
                ChangeFinding::Cycle { ids: vec![id("2a3b4c5d6e7f"), id("3b4c5d6e7f8g")] },
                ChangeFinding::SecretShape { path: "k.md".into(), rule: "aws_access_key_id", line: 8, col: 19 },
            ],
            broken_before: 0,
            knowledge: KnowledgeChanges {
                added: vec!["n.md".into()],
                edited: vec!["e.md".into()],
                removed: vec!["x.md".into()],
                followed: vec![("old.yaml".into(), "new.yaml".into())],
                new_unverifiable: vec![("n.md".into(), 1, "expression".into())],
            },
        };
        let mut files = Files::default();
        files.anchored.insert((RECORD.into(), "a/v.yaml#replicas".into()), "a/v.yaml".into());
        files.of_id.insert("2a3b4c5d6e7f".into(), "c1.md".into());
        files.of_id.insert("3b4c5d6e7f8g".into(), "c2.md".into());
        (r, files)
    }

    fn results(v: &Value) -> &Vec<Value> {
        v["runs"][0]["results"].as_array().unwrap()
    }

    #[test]
    fn rule_ids_and_levels() {
        let (r, files) = every_kind();
        let v = sarif(&r, &files, "9.9.9");
        assert_eq!(
            (&v["$schema"], &v["version"]),
            (&json!("https://json.schemastore.org/sarif-2.1.0.json"), &json!("2.1.0"))
        );
        assert_eq!(v["runs"].as_array().unwrap().len(), 1);
        assert_eq!(v["runs"][0]["columnKind"], "unicodeCodePoints", "a secret's column counts characters");
        let driver = &v["runs"][0]["tool"]["driver"];
        assert_eq!((&driver["name"], &driver["version"]), (&json!("docbrain"), &json!("9.9.9")));
        let rules: Vec<&str> = driver["rules"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
        let ids = [
            "knowledge/broken",
            "knowledge/changed",
            "knowledge/stale",
            "knowledge/knowledge-change",
            "knowledge/duplicate-id",
            "knowledge/supersedes",
            "knowledge/cycle",
            "knowledge/secret-shape",
        ];
        assert_eq!(rules, ids);
        for rule in driver["rules"].as_array().unwrap() {
            let text = rule["shortDescription"]["text"].as_str().unwrap();
            assert!(!text.is_empty() && !text.contains('\n'), "{rule}");
        }
        let got: Vec<(&str, &str)> =
            results(&v).iter().map(|x| (x["ruleId"].as_str().unwrap(), x["level"].as_str().unwrap())).collect();
        let mut want = vec![
            ("knowledge/broken", "warning"),
            ("knowledge/changed", "warning"),
            ("knowledge/stale", "note"),
            ("knowledge/duplicate-id", "warning"),
            ("knowledge/supersedes", "warning"),
            ("knowledge/cycle", "warning"),
            ("knowledge/secret-shape", "warning"),
        ];
        want.extend([("knowledge/knowledge-change", "note"); 5]);
        assert_eq!(got, want);
        for x in results(&v) {
            let i = x["ruleIndex"].as_u64().unwrap() as usize;
            assert_eq!(ids[i], x["ruleId"], "{x}");
        }
    }

    #[test]
    fn every_result_is_located() {
        let (r, files) = every_kind();
        let v = sarif(&r, &files, "1");
        let uri = |x: &Value| {
            assert_eq!(x["physicalLocation"]["artifactLocation"]["uriBaseId"], "%SRCROOT%", "{x}");
            x["physicalLocation"]["artifactLocation"]["uri"].as_str().unwrap().to_owned()
        };
        let at: Vec<(String, Vec<String>)> = results(&v)
            .iter()
            .map(|x| {
                let related = x["relatedLocations"].as_array().map(|a| a.iter().map(uri).collect()).unwrap_or_default();
                (uri(&x["locations"][0]), related)
            })
            .collect();
        let one =
            |u: &str, related: &[&str]| (u.to_owned(), related.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        assert_eq!(
            at,
            [
                one("a/v.yaml", &[RECORD]),
                one("b.md", &["r2.md"]),
                one("c.md", &["r3.md"]),
                one("d1.md", &["d2.md"]),
                one("s.md", &[]),
                one("c1.md", &["c2.md"]),
                one("k.md", &[]),
                one("n.md", &[]),
                one("e.md", &[]),
                one("x.md", &[]),
                one("new.yaml", &[]),
                one("n.md", &[]),
            ]
        );
        let secret = &results(&v)[6]["locations"][0]["physicalLocation"]["region"];
        assert_eq!(secret, &json!({"startLine": 8, "startColumn": 19}));
        assert!(
            results(&v)
                .iter()
                .enumerate()
                .all(|(i, x)| i == 6 || x["locations"][0]["physicalLocation"].get("region").is_none())
        );
    }

    #[test]
    fn a_withheld_anchor_is_located_at_the_file_it_still_names() {
        let r = GateReport {
            findings: vec![
                finding(Class::Broken { rule: 1 }, RECORD, "a/v.yaml#(withheld: secret-shape)"),
                finding(Class::Broken { rule: 1 }, RECORD, "(withheld: secret-shape)"),
            ],
            knowledge: KnowledgeChanges {
                followed: vec![("(withheld: secret-shape)".into(), "(withheld: secret-shape)".into())],
                ..Default::default()
            },
            ..Default::default()
        };
        let mut files = Files::default();
        for anchor in ["a/v.yaml#(withheld: secret-shape)", "(withheld: secret-shape)"] {
            files.anchored.insert((RECORD.into(), anchor.into()), "keys/AKIAQ7ZK3M9W2X4V8B6N".into());
        }
        let v = sarif(&r, &files, "1");
        let at = |i: usize| results(&v)[i]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"].clone();
        assert_eq!((at(0), at(1)), (json!("a/v.yaml"), json!(uri(RECORD))), "never a file the anchor withholds");
        assert_eq!(results(&v)[2].get("locations"), None, "a withheld followed path is no location");
        assert!(!v.to_string().contains("AKIA"));
    }

    #[test]
    fn an_anchor_whose_file_is_not_known_is_cut_at_its_first_hash() {
        let r = GateReport { findings: vec![finding(Class::Stale, "r.md", "v.yaml#a.b")], ..Default::default() };
        let v = sarif(&r, &Files::default(), "1");
        assert_eq!(results(&v)[0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"], "v.yaml");
    }

    #[test]
    fn uris_keep_unreserved_characters_and_slashes() {
        assert_eq!(uri("a/b-c_d.e~f/G9"), "a/b-c_d.e~f/G9");
        assert_eq!(uri("a b#c%d?e&f+g:h@i"), "a%20b%23c%25d%3Fe%26f%2Bg%3Ah%40i");
        assert_eq!(uri("\u{fc}ber/\u{65e5}"), "%C3%BCber/%E6%97%A5");
        assert_eq!(uri("\u{1b}[2J"), "%1B%5B2J");
    }

    #[test]
    fn fingerprints_hash_rule_record_and_place() {
        let (r, files) = every_kind();
        let v = sarif(&r, &files, "1");
        let fp: Vec<&str> =
            results(&v).iter().map(|x| x["partialFingerprints"]["docbrainKnowledge/v1"].as_str().unwrap()).collect();
        let sha = |s: &str| hex::encode(Sha256::digest(s));
        assert_eq!(fp[0], sha("knowledge/broken\u{0}7k2m9q4r8t1v\u{0}a/v.yaml#replicas"));
        assert_eq!(fp[3], sha("knowledge/duplicate-id\u{0}7k2m9q4r8t1v\u{0}d1.md"));
        assert_eq!(fp[6], sha("knowledge/secret-shape\u{0}\u{0}k.md"));
        assert_eq!(fp[7], sha("knowledge/knowledge-change\u{0}\u{0}n.md"));
        assert_eq!(fp[11], sha("knowledge/knowledge-change\u{0}\u{0}n.md\u{0}2"), "a check not judged keeps its own");
        assert_eq!(fp.iter().collect::<std::collections::BTreeSet<_>>().len(), fp.len(), "{fp:?}");
    }

    #[test]
    fn results_alike_but_for_what_is_withheld_keep_fingerprints_apart() {
        // Two anchors withheld alike on one record, two secret shapes in one file: an alert each, from the second on
        // by its occurrence.
        let withheld = "a/v.yaml#(withheld: secret-shape)";
        let shape = |line| ChangeFinding::SecretShape { path: "k.md".into(), rule: "aws_access_key_id", line, col: 19 };
        let broken = || finding(Class::Broken { rule: 1 }, RECORD, withheld);
        let r = GateReport {
            findings: vec![broken(), broken(), broken()],
            change_findings: vec![shape(8), shape(9)],
            ..Default::default()
        };
        let v = sarif(&r, &Files::default(), "1");
        let fp: Vec<&str> =
            results(&v).iter().map(|x| x["partialFingerprints"]["docbrainKnowledge/v1"].as_str().unwrap()).collect();
        let sha = |s: &str| hex::encode(Sha256::digest(s));
        let first = sha(&format!("knowledge/broken\u{0}7k2m9q4r8t1v\u{0}{withheld}"));
        let nth = |n: usize| sha(&format!("{first}\u{0}{n}"));
        assert_eq!(fp[..3], [first.clone(), nth(2), nth(3)]);
        let first = sha("knowledge/secret-shape\u{0}\u{0}k.md");
        assert_eq!(fp[3..], [first.clone(), sha(&format!("{first}\u{0}2"))]);
    }

    #[test]
    fn messages_name_the_result_and_escape_controls() {
        let (mut r, files) = every_kind();
        r.findings[0].title = "Two \u{1b}[2Jreplicas".into();
        r.findings[0].same_content = Some("a/w.yaml".into());
        let v = sarif(&r, &files, "1");
        let text = |i: usize| results(&v)[i]["message"]["text"].as_str().unwrap().to_owned();
        assert_eq!(
            text(0),
            "broken a/v.yaml#replicas (rule 1)\nTwo \\x{1b}[2Jreplicas\nreason: missing-file\nsame content at a/w.yaml"
        );
        assert_eq!(text(1), "changed b.md\nTitle\nreason: missing-file");
        assert_eq!(text(3), "duplicate-id 7k2m9q4r8t1v: d1.md, d2.md");
        assert_eq!(text(4), "supersedes s.md: not-in-folder");
        assert_eq!(text(5), "cycle 2a3b4c5d6e7f, 3b4c5d6e7f8g");
        assert_eq!(text(6), "secret-shape k.md 8:19 (aws_access_key_id)");
        assert_eq!(text(7), "knowledge added n.md");
        assert_eq!(text(10), "knowledge followed old.yaml -> new.yaml");
        assert_eq!(text(11), "knowledge not judged n.md check 2: expression");
        let mut withheld = r.clone();
        withheld.findings[0].title.clear();
        assert_eq!(
            results(&sarif(&withheld, &files, "1"))[0]["message"]["text"].as_str().unwrap().lines().nth(1),
            Some("reason: missing-file")
        );
    }

    #[test]
    fn the_broken_rule_says_how_to_change_the_value_on_purpose() {
        let v = sarif(&GateReport::default(), &Files::default(), "9.9.9");
        for rule in v["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap() {
            let help = if rule["id"] == "knowledge/broken" { json!({"text": ON_PURPOSE}) } else { Value::Null };
            assert_eq!(rule["help"], help, "{rule}");
        }
    }
}
