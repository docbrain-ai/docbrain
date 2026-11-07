// SPDX-License-Identifier: MIT
//! The change gate, reference form: two rules over two trees (`SEMANTICS.md`
//! §The gate).

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::bounds::{ANCHORED_READ_MAX, RECORD_FILE_MAX, RECORDS_PER_PASS};
use crate::check::{CheckState, check_inputs, eval_check};
use crate::folder::{Folder, RecordFile, SupersedeProblem, folder_scopes_above, is_record, load_folders_counted, records_dir, scope_of};
use crate::path::{RepoGlob, starting_with};
use crate::reason::{Fail, Unverifiable};
use crate::record::{Anchor, Check, Expectation, Id, Parsed, Record, Verb};
use crate::resolve::FileCache;
use crate::tree::{ChangedPaths, Entry, Mode, Oid, Tree, TreeError};

/// What happened to a `must` of B that has no same check live in H and still does not fail (`changed` in `SEMANTICS.md` §The gate).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChangeKind {
    RecordRemoved,
    Superseded,
    Invalid,
    NewerSchema,
    CheckRemoved,
    VerbChanged,
    AnchorMoved { from: String, to: String },
    ExpectationChanged { from: String, to: String },
}

/// A finding's class: `broken` by rule 1 or rule 2, `changed`, or `stale`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Class {
    Broken { rule: u8 },
    Changed(ChangeKind),
    Stale,
}

/// One finding: a record and an anchored file (and key), the worst class of its checks, every reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub class: Class,
    pub id: Id,
    pub record_path: String,
    /// The anchored path from the root; `<file>#<key path>` for `config_key` (its first layer).
    pub anchor: String,
    pub title: String,
    pub first_paragraph: String,
    pub reasons: Vec<String>,
    /// For a check whose file is missing at H: the file the change added with the same content.
    pub same_content: Option<String>,
}

/// A finding on the change itself, never a record state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChangeFinding {
    DuplicateId { id: Id, paths: Vec<String> },
    Supersedes { path: String, problem: SupersedeProblem },
    Cycle { ids: Vec<Id> },
    SecretShape { path: String, rule: &'static str, line: u32, col: u32 },
}

/// Record files added, edited and removed; anchors that followed a move; new `must` checks that cannot be
/// judged at H (record path, check index, reason code).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct KnowledgeChanges {
    pub added: Vec<String>,
    pub edited: Vec<String>,
    pub removed: Vec<String>,
    pub followed: Vec<(String, String)>,
    pub new_unverifiable: Vec<(String, usize, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct GateReport {
    pub findings: Vec<Finding>,
    pub change_findings: Vec<ChangeFinding>,
    /// Checks rule 1 excused whose inputs or folder the change touched.
    pub broken_before: usize,
    pub knowledge: KnowledgeChanges,
}

impl ChangeKind {
    /// The published code.
    pub fn code(&self) -> &'static str {
        match self {
            ChangeKind::RecordRemoved => "record-removed",
            ChangeKind::Superseded => "superseded",
            ChangeKind::Invalid => "invalid",
            ChangeKind::NewerSchema => "newer-schema",
            ChangeKind::CheckRemoved => "check-removed",
            ChangeKind::VerbChanged => "verb-changed",
            ChangeKind::AnchorMoved { .. } => "anchor-moved",
            ChangeKind::ExpectationChanged { .. } => "expectation-changed",
        }
    }

    /// The code, then `: <from> -> <to>` where the kind carries them.
    fn reason(&self) -> String {
        match self {
            ChangeKind::AnchorMoved { from, to } | ChangeKind::ExpectationChanged { from, to } => {
                format!("{}: {from} -> {to}", self.code())
            }
            other => other.code().to_owned(),
        }
    }
}

impl Class {
    /// Broken, then changed, then stale: the worst class of a group wins.
    fn rank(&self) -> u8 {
        match self {
            Class::Broken { .. } => 0,
            Class::Changed(_) => 1,
            Class::Stale => 2,
        }
    }
}

/// `SEMANTICS.md` §The gate: same id, verb and parsed expectation; key paths or `code` texts equal; the same
/// number of layers; and each position's root path equal, or B's path matches nothing in H.
pub fn same_check(h: (&Id, &Check), b: (&Id, &Check), head: &dyn Tree) -> Result<bool, TreeError> {
    if h.0 != b.0 || h.1.verb != b.1.verb || h.1.expect != b.1.expect {
        return Ok(false);
    }
    same_anchor(h.1, b.1, head)
}

/// The anchor part of "the same check": the same kind and keys (`same_kind_and_key`), and each position's
/// root path equal or B's matching nothing in H.
fn same_anchor(h: &Check, b: &Check, head: &dyn Tree) -> Result<bool, TreeError> {
    let Some(pairs) = positions(h, b) else { return Ok(false) };
    for (x, y) in pairs {
        if x.root != y.root && matches_any(head, y)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The paired file patterns of two anchors of one kind with equal key paths (as parsed), `code` texts and
/// layer counts; `None` when they differ in any of those. An unknown check kind pairs with its own key.
fn positions<'a>(h: &'a Check, b: &'a Check) -> Option<Vec<(&'a RepoGlob, &'a RepoGlob)>> {
    match (&h.anchor, &b.anchor) {
        (Anchor::Path(x), Anchor::Path(y)) => Some(vec![(x, y)]),
        (Anchor::Code { file: x, text: tx }, Anchor::Code { file: y, text: ty }) if tx == ty => Some(vec![(x, y)]),
        (Anchor::ConfigKey(x), Anchor::ConfigKey(y)) if x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.key.segs == q.key.segs) => {
            Some(x.iter().zip(y).map(|(p, q)| (&p.file, &q.file)).collect())
        }
        (Anchor::Unknown(x), Anchor::Unknown(y)) if x == y => Some(Vec::new()),
        _ => None,
    }
}

/// Whether any entry of `tree` is at or under a literal path, or matched by a glob; links and submodules
/// count (the matching of a `path` anchor, `SEMANTICS.md` §Three anchors).
fn matches_any(tree: &dyn Tree, g: &RepoGlob) -> Result<bool, TreeError> {
    if g.is_literal() {
        Ok(tree.entry(&g.root)?.is_some() || !tree.list(&g.root)?.is_empty())
    } else {
        Ok(tree.list(g.literal_dir())?.iter().any(|(p, _)| g.matches(p)))
    }
}

/// How a finding names a check's place: the root path of its first position, `<file>#<key path>` for a
/// `config_key`.
fn place(c: &Check, at: usize) -> String {
    match &c.anchor {
        Anchor::Path(g) | Anchor::Code { file: g, .. } => g.root.clone(),
        Anchor::ConfigKey(layers) => layers.get(at).map_or_else(String::new, |l| format!("{}#{}", l.file.root, l.key.text)),
        Anchor::Unknown(k) => k.clone(),
    }
}

/// A check is affected by a change when a record file of its folder changed, or a changed path is
/// matched by one of its inputs, lies under a literal input, or is a directory at or above a literal input or
/// a glob's literal directory.
///
/// Each clause is tried only on the changed paths that can satisfy it, so the work follows the change: the
/// records directory's run for the first, and `touch_candidates` for each input.
pub(crate) fn affected(check: &Check, scope: &str, changed: &ChangedPaths) -> bool {
    let changed = changed.paths();
    let records = format!("{}/", records_dir(scope));
    starting_with(changed, &records).iter().any(|p| p.ends_with(".md") && scope_of(p) == Some(scope))
        || check_inputs(check).iter().any(|g| touch_candidates(g, changed).any(|p| touches(g, p)))
}

/// Every path of `sorted` (byte order) that `touches(g, _)`, and maybe others: the run that starts with the
/// pattern's literal prefix, then each directory above the literal path or, for a glob, at or above its
/// literal directory, found by exact name.
fn touch_candidates<'a>(g: &'a RepoGlob, sorted: &'a [String]) -> impl Iterator<Item = &'a String> + 'a {
    let (base, itself) = if g.is_literal() { (g.root.as_str(), false) } else { (g.literal_dir(), true) };
    let dirs = base.match_indices('/').map(move |(i, _)| base.get(..i).unwrap_or_default());
    let dirs = dirs.chain((itself && !base.is_empty()).then_some(base));
    let exact = dirs.filter_map(move |d| sorted.binary_search_by(|p| p.as_str().cmp(d)).ok().and_then(|i| sorted.get(i)));
    starting_with(sorted, g.literal_prefix()).iter().chain(exact)
}

fn touches(g: &RepoGlob, p: &str) -> bool {
    let below = |dir: &str, p: &str| p.strip_prefix(dir).is_some_and(|r| r.starts_with('/'));
    if g.is_literal() {
        g.root == p || below(&g.root, p) || below(p, &g.root)
    } else {
        let dir = g.literal_dir();
        g.matches(p) || p == dir || below(p, dir)
    }
}

fn id_of(p: &Parsed) -> Option<Id> {
    match p {
        Parsed::Valid(r) => Some(r.id),
        Parsed::Newer { id, .. } => *id,
        Parsed::Invalid { .. } => None,
    }
}

/// One live check of a tree: its record is valid, schema 1, and not superseded.
struct Live<'a> {
    file: &'a RecordFile,
    rec: &'a Record,
    scope: &'a str,
    idx: usize,
}

impl<'a> Live<'a> {
    fn check(&self) -> &'a Check {
        &self.rec.checks[self.idx]
    }
}

fn live_checks(folders: &[Folder]) -> Vec<Live<'_>> {
    let mut out = Vec::new();
    for folder in folders {
        for file in &folder.records {
            let Parsed::Valid(rec) = &file.parsed else { continue };
            if file.superseded_by.is_none() {
                out.extend((0..rec.checks.len()).map(|idx| Live { file, rec, scope: &folder.scope, idx }));
            }
        }
    }
    out
}

/// One tree's live checks' states, each evaluated on first use.
struct States<'t> {
    tree: &'t dyn Tree,
    cache: FileCache,
    memo: Vec<Option<CheckState>>,
}

impl<'t> States<'t> {
    fn new(tree: &'t dyn Tree, live: usize) -> States<'t> {
        States { tree, cache: FileCache::default(), memo: vec![None; live] }
    }

    fn of(&mut self, live: &[Live<'_>], j: usize) -> Result<CheckState, TreeError> {
        if let Some(s) = &self.memo[j] {
            return Ok(s.clone());
        }
        let s = eval_check(self.tree, live[j].check(), &mut self.cache)?;
        self.memo[j] = Some(s.clone());
        Ok(s)
    }

    /// A check of the other tree, as written there, evaluated in this one.
    fn eval(&mut self, c: &Check) -> Result<CheckState, TreeError> {
        eval_check(self.tree, c, &mut self.cache)
    }
}

/// Hints the files the given checks read (the regular files within the anchored-file bound that a `code` or
/// `config_key` input matches), so a partial clone fetches them in one call. Best effort: a lookup that fails
/// hints nothing, and evaluation then reads what it needs itself.
fn prefetch_inputs<'a>(tree: &dyn Tree, checks: impl Iterator<Item = &'a Check>) {
    let mut oids = Vec::new();
    for c in checks.filter(|c| matches!(c.anchor, Anchor::Code { .. } | Anchor::ConfigKey(_))) {
        for g in check_inputs(c) {
            let found: Vec<Entry> = if g.is_literal() {
                tree.entry(&g.root).ok().flatten().into_iter().collect()
            } else {
                tree.list(g.literal_dir()).unwrap_or_default().into_iter().filter(|(p, _)| g.matches(p)).map(|(_, e)| e).collect()
            };
            oids.extend(found.into_iter().filter(|e| matches!(e.mode, Mode::File | Mode::Executable) && e.size <= ANCHORED_READ_MAX).map(|e| e.oid));
        }
    }
    tree.prefetch(&oids);
}

/// The text of a failed check's reason: `false` carries the value and, under an op, the expectation.
fn fail_reason(f: &Fail, c: &Check) -> String {
    match (f, &c.expect) {
        (Fail::False { actual }, Expectation::Exist) => format!("false: {}", actual.as_str()),
        (Fail::False { actual }, e) => format!("false: {}, expected {e}", actual.as_str()),
        (other, _) => other.code().into_owned(),
    }
}

fn no_longer_checkable(u: &Unverifiable) -> String {
    format!("no-longer-checkable: {}", u.code())
}

/// One check's contribution to a finding.
struct Member<'a> {
    class: Class,
    file: &'a RecordFile,
    rec: &'a Record,
    check: &'a Check,
    reason: String,
    missing_file: bool,
}

fn member<'a>(class: Class, l: &Live<'a>, reason: String) -> Member<'a> {
    Member { class, file: l.file, rec: l.rec, check: l.check(), reason, missing_file: false }
}

/// Both trees, what changed between them, and the folders above the changed paths in each.
struct Pass<'a> {
    base: &'a dyn Tree,
    head: &'a dyn Tree,
    changed: &'a ChangedPaths,
    head_folders: &'a [Folder],
}

/// The gate over every check of the folders above a changed path, the changed paths computed from both
/// trees' entries. The reference the footprint form is proven against.
pub fn gate_full(base: &dyn Tree, head: &dyn Tree) -> Result<GateReport, TreeError> {
    run(base, head, &changed_between(base, head)?, true)
}

/// The gate by the footprint rule (`SEMANTICS.md` §Cost): the folders above a changed path, and in
/// them only the checks the change affected. `changed` is every path added, removed, or with another
/// mode or oid between the two trees. Equal to `gate_full` over the same trees.
pub fn gate(base: &dyn Tree, head: &dyn Tree, changed: &ChangedPaths) -> Result<GateReport, TreeError> {
    run(base, head, changed, false)
}

/// Both forms. A pass that leaves a record unread (`RECORDS_PER_PASS`) judges every check: a record the
/// change added can push an untouched one past the bound in one tree only.
fn run(base: &dyn Tree, head: &dyn Tree, changed: &ChangedPaths, every: bool) -> Result<GateReport, TreeError> {
    run_within(base, head, changed, every, RECORDS_PER_PASS)
}

fn run_within(base: &dyn Tree, head: &dyn Tree, changed: &ChangedPaths, every: bool, budget: usize) -> Result<GateReport, TreeError> {
    let paths = changed.paths();
    let (base_folders, base_over) = load_folders_counted(base, &folder_scopes_above(base, paths)?, budget)?;
    let (head_folders, head_over) = load_folders_counted(head, &folder_scopes_above(head, paths)?, budget)?;
    let pass = Pass { base, head, changed, head_folders: &head_folders };
    judge(&pass, &live_checks(&base_folders), &live_checks(&head_folders), every || base_over || head_over)
}

/// Every path added, removed, or with another mode or oid, from full listings of both trees.
fn changed_between(base: &dyn Tree, head: &dyn Tree) -> Result<ChangedPaths, TreeError> {
    let b: BTreeMap<String, Entry> = base.list("")?.into_iter().collect();
    let h: BTreeMap<String, Entry> = head.list("")?.into_iter().collect();
    let mut out: Vec<String> = b
        .iter()
        .filter(|(p, e)| h.get(*p).is_none_or(|x| x.mode != e.mode || x.oid != e.oid))
        .map(|(p, _)| p.clone())
        .collect();
    out.extend(h.keys().filter(|p| !b.contains_key(*p)).cloned());
    Ok(ChangedPaths::new(out))
}

/// Which live checks of B are the same check as each live check of H.
struct Matching {
    /// For each live check of H, the live checks of B that are its same check.
    same: Vec<Vec<usize>>,
    /// Whether each live check of B is some H check's same check.
    matched: Vec<bool>,
    /// The (B path, H path) pairs of same checks whose root paths differ.
    followed: BTreeSet<(String, String)>,
}

/// Same checks with every root path equal pair first: an H check that has one in B is the same check as
/// those alone, and follows nothing. Otherwise a twin elsewhere (same id, its path gone from H) would excuse
/// or condemn a check the change did not touch, and the twin's own removal would never be judged.
fn exact(h: &Check, b: &Check) -> bool {
    h.verb == b.verb && h.expect == b.expect && positions(h, b).is_some_and(|ps| ps.iter().all(|(x, y)| x.root == y.root))
}

fn matching(head: &dyn Tree, in_base: &[Live<'_>], in_head: &[Live<'_>]) -> Result<Matching, TreeError> {
    let mut by_id: BTreeMap<Id, Vec<usize>> = BTreeMap::new();
    for (j, b) in in_base.iter().enumerate() {
        by_id.entry(b.rec.id).or_default().push(j);
    }
    let mut m = Matching { same: Vec::with_capacity(in_head.len()), matched: vec![false; in_base.len()], followed: BTreeSet::new() };
    for h in in_head {
        let ids = by_id.get(&h.rec.id).map(Vec::as_slice).unwrap_or_default();
        let mut same: Vec<usize> = ids.iter().copied().filter(|&j| exact(h.check(), in_base[j].check())).collect();
        if same.is_empty() {
            for &j in ids {
                let b = &in_base[j];
                if same_check((&h.rec.id, h.check()), (&b.rec.id, b.check()), head)? {
                    same.push(j);
                    for (x, y) in positions(h.check(), b.check()).into_iter().flatten() {
                        if x.root != y.root {
                            m.followed.insert((y.root.clone(), x.root.clone()));
                        }
                    }
                }
            }
        }
        for &j in &same {
            m.matched[j] = true;
        }
        m.same.push(same);
    }
    Ok(m)
}

/// The two rules (`SEMANTICS.md` §The gate), over the live checks of both trees. With `every` false, only the checks the change
/// affected are judged: any other yields nothing (`SEMANTICS.md` §Cost).
fn judge(pass: &Pass<'_>, in_base: &[Live<'_>], in_head: &[Live<'_>], every: bool) -> Result<GateReport, TreeError> {
    let need = |l: &Live<'_>| every || affected(l.check(), l.scope, pass.changed);
    let judged: Vec<usize> = (0..in_head.len()).filter(|&i| need(&in_head[i])).collect();
    let Matching { same, matched, followed } = matching(pass.head, in_base, in_head)?;
    // Rule 2's candidates: a live must of B with no same check live in H.
    let dropped: Vec<usize> = (0..in_base.len()).filter(|&j| !matched[j] && in_base[j].check().verb == Verb::Must && need(&in_base[j])).collect();
    let at_base = judged.iter().flat_map(|&i| same[i].iter()).chain(&dropped).map(|&j| in_base[j].check());
    prefetch_inputs(pass.base, at_base);
    let mut base = States::new(pass.base, in_base.len());
    let at_head = judged.iter().map(|&i| in_head[i].check()).chain(dropped.iter().map(|&j| in_base[j].check()));
    prefetch_inputs(pass.head, at_head);
    let mut head = States::new(pass.head, in_head.len());

    let (mut members, mut broken_before, mut new_unverifiable) = (Vec::new(), 0, Vec::new());
    // Rule 1 (H judges H) and the stale notes. An excuse needs every same check of B to qualify, and at least
    // one; an unverifiable state is a finding only after holding at B.
    for &i in &judged {
        let h = &in_head[i];
        let state = head.of(in_head, i)?;
        let at_b = same[i].iter().map(|&j| base.of(in_base, j)).collect::<Result<Vec<_>, _>>()?;
        let all = |f: fn(&CheckState) -> bool| !at_b.is_empty() && at_b.iter().all(f);
        let held = at_b.iter().any(|s| matches!(s, CheckState::Holds(_)));
        match (h.check().verb, &state) {
            (Verb::Must, CheckState::Broken(f)) => {
                if all(|s| matches!(s, CheckState::Broken(_))) {
                    broken_before += usize::from(affected(h.check(), h.scope, pass.changed));
                } else {
                    let mut x = member(Class::Broken { rule: 1 }, h, fail_reason(f, h.check()));
                    x.missing_file = *f == Fail::MissingFile;
                    members.push(x);
                }
            }
            (Verb::Must, CheckState::Unverifiable(u)) if held => members.push(member(Class::Broken { rule: 1 }, h, no_longer_checkable(u))),
            (Verb::Must, CheckState::Unverifiable(u)) if at_b.is_empty() => {
                new_unverifiable.push((h.file.path.clone(), h.idx, u.code().into_owned()));
            }
            (Verb::Assumes, CheckState::Stale(f)) if !all(|s| !matches!(s, CheckState::Holds(_))) => {
                let mut x = member(Class::Stale, h, fail_reason(f, h.check()));
                x.missing_file = *f == Fail::MissingFile;
                members.push(x);
            }
            (Verb::Assumes, CheckState::Unverifiable(u)) if held => members.push(member(Class::Stale, h, no_longer_checkable(u))),
            _ => {}
        }
    }
    // Rule 2 (B is not dropped): not broken at B, evaluated at H as B wrote it. Unverifiable at both trees is
    // never a finding.
    let mut rule_2 = Vec::new();
    for &j in &dropped {
        let b = &in_base[j];
        let at_b = base.of(in_base, j)?;
        if matches!(at_b, CheckState::Broken(_)) {
            continue;
        }
        let at_h = head.eval(b.check())?;
        let x = match (&at_b, &at_h) {
            (_, CheckState::Broken(f)) => {
                let mut x = member(Class::Broken { rule: 2 }, b, fail_reason(f, b.check()));
                x.missing_file = *f == Fail::MissingFile;
                x
            }
            (CheckState::Holds(_), CheckState::Unverifiable(u)) => member(Class::Broken { rule: 2 }, b, no_longer_checkable(u)),
            (CheckState::Unverifiable(_), CheckState::Unverifiable(_)) => continue,
            _ => {
                let kind = change_kind(b, pass)?;
                let reason = kind.reason();
                member(Class::Changed(kind), b, reason)
            }
        };
        rule_2.push(x);
    }
    // Rule 1's members, then rule 2's, then the stale notes.
    let (stale, rule_1): (Vec<_>, Vec<_>) = members.into_iter().partition(|m| m.class == Class::Stale);
    let added = added_files(pass)?;
    let findings = group(pass, rule_1.into_iter().chain(rule_2).chain(stale), &added)?;

    let knowledge = knowledge_changes(pass, followed, new_unverifiable)?;
    let change_findings = change_findings(pass.head, pass.head_folders, &knowledge)?;
    Ok(GateReport { findings, change_findings, broken_before, knowledge })
}

/// What happened to B's record, found by id among H's records that have one (the first live by path,
/// else the first by path), else at B's path when the record there has no readable id.
fn change_kind(b: &Live<'_>, pass: &Pass<'_>) -> Result<ChangeKind, TreeError> {
    let all = || pass.head_folders.iter().flat_map(|f| &f.records);
    let mut with_id: Vec<&RecordFile> = all().filter(|r| id_of(&r.parsed) == Some(b.rec.id)).collect();
    with_id.sort_by(|x, y| x.path.cmp(&y.path));
    let live = |r: &&&RecordFile| matches!(r.parsed, Parsed::Valid(_)) && r.superseded_by.is_none();
    let Some(r) = with_id.iter().find(live).or_else(|| with_id.first()) else {
        return Ok(match all().find(|r| r.path == b.file.path).map(|r| &r.parsed) {
            Some(Parsed::Invalid { .. }) => ChangeKind::Invalid,
            Some(Parsed::Newer { id: None, .. }) => ChangeKind::NewerSchema,
            _ => ChangeKind::RecordRemoved,
        });
    };
    if r.superseded_by.is_some() {
        return Ok(ChangeKind::Superseded);
    }
    let rec = match &r.parsed {
        Parsed::Valid(rec) => rec,
        Parsed::Newer { .. } => return Ok(ChangeKind::NewerSchema),
        Parsed::Invalid { .. } => return Ok(ChangeKind::Invalid),
    };
    let was = b.check();
    for c in &rec.checks {
        if c.verb != was.verb && same_anchor(c, was, pass.head)? {
            return Ok(ChangeKind::VerbChanged);
        }
    }
    for c in &rec.checks {
        if c.expect != was.expect && same_anchor(c, was, pass.head)? {
            return Ok(ChangeKind::ExpectationChanged { from: was.expect.to_string(), to: c.expect.to_string() });
        }
    }
    for c in &rec.checks {
        if let Some(at) = positions(c, was).and_then(|pairs| pairs.iter().position(|(x, y)| x.root != y.root)) {
            return Ok(ChangeKind::AnchorMoved { from: place(was, at), to: place(c, at) });
        }
    }
    Ok(ChangeKind::CheckRemoved)
}

/// The regular files the change added, with their oids, in byte order of path.
fn added_files(pass: &Pass<'_>) -> Result<Vec<(String, Oid)>, TreeError> {
    let mut out = Vec::new();
    for p in pass.changed.paths() {
        if let Some(e) = pass.head.entry(p)?
            && matches!(e.mode, Mode::File | Mode::Executable)
            && pass.base.entry(p)?.is_none()
        {
            out.push((p.clone(), e.oid));
        }
    }
    Ok(out)
}

/// For a check missing its file at H, the first added file whose content is that of one of its literal
/// input paths in B (a regular file there, absent from H).
fn same_content(c: &Check, pass: &Pass<'_>, added: &[(String, Oid)]) -> Result<Option<String>, TreeError> {
    for g in check_inputs(c).iter().filter(|g| g.is_literal()) {
        if pass.head.entry(&g.root)?.is_some() {
            continue;
        }
        let Some(e) = pass.base.entry(&g.root)? else { continue };
        if !matches!(e.mode, Mode::File | Mode::Executable) {
            continue;
        }
        if let Some((p, _)) = added.iter().find(|(_, o)| *o == e.oid) {
            return Ok(Some(p.clone()));
        }
    }
    Ok(None)
}

/// One finding per (id, record path, anchor): the worst class wins, every reason kept in member order.
fn group<'a>(pass: &Pass<'_>, members: impl Iterator<Item = Member<'a>>, added: &[(String, Oid)]) -> Result<Vec<Finding>, TreeError> {
    let mut findings: Vec<Finding> = Vec::new();
    // (id, record path, anchor) -> index in `findings`.
    let mut at: BTreeMap<(Id, String, String), usize> = BTreeMap::new();
    for m in members {
        let anchor = place(m.check, 0);
        let same = if m.missing_file { same_content(m.check, pass, added)? } else { None };
        let k = (m.rec.id, m.file.path.clone(), anchor.clone());
        if let Some(&i) = at.get(&k) {
            let f = &mut findings[i];
            if m.class.rank() < f.class.rank() {
                (f.class, f.title, f.first_paragraph) = (m.class, m.rec.title.clone(), m.rec.first_paragraph().to_owned());
            }
            f.reasons.push(m.reason);
            f.same_content = f.same_content.take().or(same);
            continue;
        }
        at.insert(k, findings.len());
        findings.push(Finding {
            class: m.class,
            id: m.rec.id,
            record_path: m.file.path.clone(),
            anchor,
            title: m.rec.title.clone(),
            first_paragraph: m.rec.first_paragraph().to_owned(),
            reasons: vec![m.reason],
            same_content: same,
        });
    }
    findings.sort_by(|x, y| {
        (x.class.rank(), &x.record_path, &x.anchor, x.id).cmp(&(y.class.rank(), &y.record_path, &y.anchor, y.id))
    });
    Ok(findings)
}

/// Record files added, edited and removed by the change; anchors that followed; new unverifiable musts.
fn knowledge_changes(pass: &Pass<'_>, followed: BTreeSet<(String, String)>, mut new_unverifiable: Vec<(String, usize, String)>) -> Result<KnowledgeChanges, TreeError> {
    let mut k = KnowledgeChanges::default();
    let is_rec = |p: &str, e: &Option<Entry>| e.as_ref().is_some_and(|e| scope_of(p).is_some() && is_record(p, e));
    for p in pass.changed.paths() {
        match (is_rec(p, &pass.base.entry(p)?), is_rec(p, &pass.head.entry(p)?)) {
            (false, true) => k.added.push(p.clone()),
            (true, false) => k.removed.push(p.clone()),
            (true, true) => k.edited.push(p.clone()),
            (false, false) => {}
        }
    }
    k.followed = followed.into_iter().collect();
    new_unverifiable.sort();
    k.new_unverifiable = new_unverifiable;
    Ok(k)
}

/// a duplicate id, a `supersedes` without effect, a cycle and a secret shape (`SEMANTICS.md` §Secrets), only where a record
/// file the change added or edited takes part; by kind, then path.
fn change_findings(head: &dyn Tree, head_folders: &[Folder], k: &KnowledgeChanges) -> Result<Vec<ChangeFinding>, TreeError> {
    let touched: BTreeSet<&str> = k.added.iter().chain(&k.edited).map(String::as_str).collect();
    let (mut dups, mut sups, mut cycles) = (Vec::new(), Vec::new(), Vec::new());
    let screened: Vec<&RecordFile> = head_folders.iter().flat_map(|f| &f.records).filter(|r| touched.contains(r.path.as_str())).collect();
    let oids: Vec<Oid> = screened.iter().map(|r| r.oid.clone()).collect();
    let mut secrets = Vec::new();
    let read = if oids.is_empty() { Vec::new() } else { head.read_many(&oids)? };
    for (r, bytes) in screened.iter().zip(read) {
        // The first `record-file` bytes: past them the record is invalid, and a clone reads a different length.
        let cut = usize::try_from(RECORD_FILE_MAX).unwrap_or(usize::MAX).min(bytes.len());
        for hit in crate::secret::screen(&String::from_utf8_lossy(&bytes[..cut])) {
            secrets.push(ChangeFinding::SecretShape { path: r.path.clone(), rule: hit.rule, line: hit.line, col: hit.col });
        }
    }
    for f in head_folders {
        for (id, paths) in &f.duplicates {
            if paths.iter().any(|p| touched.contains(p.as_str())) {
                dups.push(ChangeFinding::DuplicateId { id: *id, paths: paths.clone() });
            }
        }
        for (path, problem) in &f.supersede_problems {
            if touched.contains(path.as_str()) {
                sups.push(ChangeFinding::Supersedes { path: path.clone(), problem: *problem });
            }
        }
        for ids in &f.cycles {
            let member = |r: &RecordFile| id_of(&r.parsed).is_some_and(|i| ids.contains(&i));
            if f.records.iter().any(|r| member(r) && touched.contains(r.path.as_str())) {
                cycles.push(ChangeFinding::Cycle { ids: ids.clone() });
            }
        }
    }
    let first = |c: &ChangeFinding| match c {
        ChangeFinding::DuplicateId { paths, .. } => paths.first().cloned().unwrap_or_default(),
        ChangeFinding::Supersedes { path, .. } | ChangeFinding::SecretShape { path, .. } => path.clone(),
        ChangeFinding::Cycle { ids } => ids.first().map(ToString::to_string).unwrap_or_default(),
    };
    for v in [&mut dups, &mut sups, &mut cycles, &mut secrets] {
        v.sort_by_key(first);
    }
    Ok(dups.into_iter().chain(sups).chain(cycles).chain(secrets).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::parse_rel;
    use crate::tree::MemTree;
    use proptest::prelude::*;

    const R: &str = ".knowledge/records/";
    const T: &str = "retry(3, backoff::exponential());";

    fn id(n: usize) -> Id {
        Id::parse(&format!("k3v9q2x7m{n:03}")).unwrap()
    }
    fn rec(n: usize, kind: &str, rest: &str) -> String {
        format!("---\nknowledge: 1\nid: 'k3v9q2x7m{n:03}'\nkind: '{kind}'\ntitle: 't{n}'\n{rest}---\nWhy {n}.\n\nMore.\n")
    }
    fn checks(cs: &[String]) -> String {
        format!("checks:\n{}", cs.concat())
    }
    fn key(k: &str, verb: &str, expect: &str) -> String {
        format!("  - config_key: '{k}'\n    {verb}: '{expect}'\n")
    }
    fn path(p: &str, verb: &str) -> String {
        format!("  - path: '{p}'\n    {verb}: 'exist'\n")
    }
    fn code(f: &str, text: &str) -> String {
        format!("  - code: '{f}'\n    text: '{text}'\n    must: 'exist'\n")
    }
    fn tree(entries: &[(&str, &str)]) -> MemTree {
        let mut t = MemTree::new();
        for (p, s) in entries {
            t.file(p, s);
        }
        t
    }
    fn run(b: &MemTree, h: &MemTree) -> GateReport {
        gate_full(b, h).unwrap()
    }
    /// (class, record path, anchor) of each finding.
    fn found(r: &GateReport) -> Vec<(Class, &str, &str)> {
        r.findings.iter().map(|f| (f.class.clone(), f.record_path.as_str(), f.anchor.as_str())).collect()
    }
    fn a(n: &str) -> String {
        format!("{R}{n}")
    }

    fn scope_s() -> impl Strategy<Value = String> {
        prop::sample::select(vec!["", "a", "ab", "a/b"]).prop_map(str::to_owned)
    }
    fn input_s() -> impl Strategy<Value = String> {
        let seg = prop::sample::select(vec!["a", "b", "ab", "a*", "*", "?", "**", "*b", "b?", "c.yaml"]);
        prop::collection::vec(seg, 1..4).prop_map(|s| s.join("/"))
    }
    fn changed_s() -> impl Strategy<Value = Vec<String>> {
        let seg = prop::sample::select(vec!["a", "b", "ab", "ba", "c.yaml", ".knowledge/records", "r.md", "x"]);
        prop::collection::vec(prop::collection::vec(seg, 1..6).prop_map(|s| s.join("/")), 0..16)
    }

    proptest! {
        #[test]
        fn candidates_hold_every_affecting_path(scope in scope_s(), rel in input_s(), changed in changed_s()) {
            // `affected` tries each clause only on its candidates; the full scan over every changed path is the
            // reference, so a path that satisfies a clause must be one of that clause's candidates.
            let changed = ChangedPaths::new(changed);
            let sorted = changed.paths();
            if let Ok(g) = parse_rel(&scope, &rel) {
                let candidates: BTreeSet<&String> = touch_candidates(&g, sorted).collect();
                for p in sorted {
                    prop_assert!(!touches(&g, p) || candidates.contains(p), "{} touches {} unseen", g.root, p);
                }
            }
            let records = starting_with(sorted, &format!("{}/", records_dir(&scope)));
            for p in sorted {
                let own = p.ends_with(".md") && scope_of(p) == Some(scope.as_str());
                prop_assert!(!own || records.contains(p), "a record of {:?} unseen: {}", scope, p);
            }
        }
    }

    #[test]
    fn affected_work_follows_the_change() {
        // A wide change: a check's inputs are tested only against the changed paths that start with their literal
        // part and the directories above it, so the work grows with checks plus paths, never with their product.
        let mut b = MemTree::new();
        for n in 0..1_000 {
            b.file(&format!("d{n}/f.rs"), "x\n");
            if n % 2 == 0 {
                b.file(&format!("d{n}/c.yaml"), "x: 1\n");
                b.file(&format!("d{n}/{R}r.md"), rec(n, "fact", &checks(&[path("*.yaml", "must")])));
            }
        }
        let mut h = MemTree::new();
        for n in 0..1_000 {
            h.file(&format!("d{n}/f.rs"), "y\n");
            if n % 2 == 0 {
                if n > 0 {
                    h.file(&format!("d{n}/c.yaml"), "x: 1\n");
                }
                h.file(&format!("d{n}/{R}r.md"), rec(n, "fact", &checks(&[path("*.yaml", "must")])));
            }
        }
        let changed = b.changed_paths(&h);
        crate::path::MATCHES.with(|m| m.set(0));
        let r = gate(&b, &h, &changed).unwrap();
        let work = crate::path::MATCHES.with(std::cell::Cell::get);
        assert_eq!(found(&r), [(Class::Broken { rule: 1 }, "d0/.knowledge/records/r.md", "d0/*.yaml")]);
        assert!(work <= 3_000, "{work} matches for 500 checks and 1,001 changed paths");
    }

    #[test]
    fn record_never_excuses_its_own_change() {
        // Edge case 66: the new must passes while the old one is violated: B's record judges at H.
        let b = tree(&[(&a("a.md"), &rec(1, "decision", &checks(&[key("v.yaml#replicas", "must", ">= 2")]))), ("v.yaml", "replicas: 3\n")]);
        let h = tree(&[(&a("a.md"), &rec(1, "decision", &checks(&[key("v.yaml#replicas", "must", ">= 1")]))), ("v.yaml", "replicas: 1\n")]);
        let r = run(&b, &h);
        assert_eq!(found(&r), [(Class::Broken { rule: 2 }, a("a.md").as_str(), "v.yaml#replicas")]);
        assert_eq!(r.findings[0].reasons, ["false: 1, expected >= 2"]);
        assert_eq!((r.findings[0].id, r.findings[0].title.as_str(), r.findings[0].first_paragraph.as_str()), (id(1), "t1", "Why 1."));
        assert_eq!(r.knowledge.edited, [a("a.md")]);
    }

    #[test]
    fn relax_only_is_changed() {
        // Edge case 67: only the record is loosened; the value is unchanged.
        let b = tree(&[(&a("a.md"), &rec(1, "decision", &checks(&[key("v.yaml#replicas", "must", ">= 2")]))), ("v.yaml", "replicas: 3\n")]);
        let h = tree(&[(&a("a.md"), &rec(1, "decision", &checks(&[key("v.yaml#replicas", "must", ">= 1")]))), ("v.yaml", "replicas: 3\n")]);
        let r = run(&b, &h);
        let kind = ChangeKind::ExpectationChanged { from: ">= 2".into(), to: ">= 1".into() };
        assert_eq!(found(&r), [(Class::Changed(kind), a("a.md").as_str(), "v.yaml#replicas")]);
        assert_eq!(r.findings[0].reasons, ["expectation-changed: >= 2 -> >= 1"]);
    }

    #[test]
    fn tightened_must_judged_by_rule_1() {
        // Edge case 68: changed, and the new check is judged by rule 1 at H.
        let b = tree(&[(&a("a.md"), &rec(1, "decision", &checks(&[key("v.yaml#replicas", "must", ">= 2")]))), ("v.yaml", "replicas: 3\n")]);
        let h = tree(&[(&a("a.md"), &rec(1, "decision", &checks(&[key("v.yaml#replicas", "must", ">= 4")]))), ("v.yaml", "replicas: 3\n")]);
        let r = run(&b, &h);
        assert_eq!(found(&r), [(Class::Broken { rule: 1 }, a("a.md").as_str(), "v.yaml#replicas")]);
        assert_eq!(r.findings[0].reasons, ["false: 3, expected >= 4", "expectation-changed: >= 2 -> >= 4"]);
        let h = tree(&[(&a("a.md"), &rec(1, "decision", &checks(&[key("v.yaml#replicas", "must", ">= 4")]))), ("v.yaml", "replicas: 5\n")]);
        let kind = ChangeKind::ExpectationChanged { from: ">= 2".into(), to: ">= 4".into() };
        assert_eq!(found(&run(&b, &h)), [(Class::Changed(kind), a("a.md").as_str(), "v.yaml#replicas")]);
    }

    #[test]
    fn moved_file_anchor_follows() {
        // Edge case 6: the anchor names the new path in the same change: the same check, nothing fires.
        let b = tree(&[(&a("a.md"), &rec(1, "fact", &checks(&[path("old/f.txt", "must")]))), ("old/f.txt", "x\n")]);
        let h = tree(&[(&a("a.md"), &rec(1, "fact", &checks(&[path("new/f.txt", "must")]))), ("new/f.txt", "x\n")]);
        let r = run(&b, &h);
        assert_eq!(found(&r), []);
        assert_eq!(r.knowledge.followed, [("old/f.txt".to_owned(), "new/f.txt".to_owned())]);
        assert_eq!((r.knowledge.edited.as_slice(), r.broken_before), ([a("a.md")].as_slice(), 0));
    }

    #[test]
    fn moved_file_anchor_left_behind() {
        // Edge case 7: rule 1 broken (must) or stale (assumes), naming the file added with the same content;
        // a config_key whose only file moved is missing-file too.
        let cs = checks(&[path("old/f.txt", "must"), path("old/f.txt", "assumes")]);
        let k = checks(&[key("old/v.yaml#a", "must", "== 1")]);
        let b = tree(&[(&a("a.md"), &rec(1, "fact", &cs)), (&a("k.md"), &rec(2, "fact", &k)), ("old/f.txt", "x\n"), ("old/v.yaml", "a: 1\n"), ("z.txt", "x\n")]);
        let h = tree(&[(&a("a.md"), &rec(1, "fact", &cs)), (&a("k.md"), &rec(2, "fact", &k)), ("new/f.txt", "x\n"), ("new/v.yaml", "a: 1\n"), ("z.txt", "x\n")]);
        let r = run(&b, &h);
        assert_eq!(
            found(&r),
            [(Class::Broken { rule: 1 }, a("a.md").as_str(), "old/f.txt"), (Class::Broken { rule: 1 }, a("k.md").as_str(), "old/v.yaml#a")]
        );
        assert_eq!(r.findings[0].reasons, ["missing-file", "missing-file"]);
        let same: Vec<_> = r.findings.iter().map(|f| f.same_content.as_deref()).collect();
        // z.txt has the same content but was not added by the change.
        assert_eq!(same, [Some("new/f.txt"), Some("new/v.yaml")]);
        assert_eq!(r.knowledge.followed, []);
        // The assumes alone is a stale note with the same action.
        let s = checks(&[path("old/f.txt", "assumes")]);
        let b = tree(&[(&a("a.md"), &rec(1, "fact", &s)), ("old/f.txt", "x\n")]);
        let h = tree(&[(&a("a.md"), &rec(1, "fact", &s)), ("new/f.txt", "x\n")]);
        let r = run(&b, &h);
        assert_eq!(found(&r), [(Class::Stale, a("a.md").as_str(), "old/f.txt")]);
        assert_eq!(r.findings[0].same_content.as_deref(), Some("new/f.txt"));
    }

    #[test]
    fn code_moved_to_existing_file_is_broken() {
        // Edge case 11: the old file remains, so B's check judges it and finds the text gone.
        let b = tree(&[(&a("a.md"), &rec(1, "fact", &checks(&[code("a.rs", T)]))), ("a.rs", &format!("fn a() {{ {T} }}\n")), ("b.rs", "fn b() {}\n")]);
        let h = tree(&[(&a("a.md"), &rec(1, "fact", &checks(&[code("b.rs", T)]))), ("a.rs", "fn a() {}\n"), ("b.rs", &format!("fn b() {{ {T} }}\n"))]);
        let r = run(&b, &h);
        assert_eq!(found(&r), [(Class::Broken { rule: 2 }, a("a.md").as_str(), "a.rs")]);
        assert_eq!(r.findings[0].reasons, ["false: 0 occurrences"]);
        // The text copied, not moved: the old anchor still holds, so the anchor moved.
        let h = tree(&[(&a("a.md"), &rec(1, "fact", &checks(&[code("b.rs", T)]))), ("a.rs", &format!("fn a() {{ {T} }}\n")), ("b.rs", &format!("fn b() {{ {T} }}\n"))]);
        let kind = ChangeKind::AnchorMoved { from: "a.rs".into(), to: "b.rs".into() };
        assert_eq!(found(&run(&b, &h)), [(Class::Changed(kind), a("a.md").as_str(), "a.rs")]);
    }

    #[test]
    fn decommission_needs_two_changes() {
        // Edge case 14: the service, its folder and records deleted in one change: rule 2 for each must.
        let x = "svc/a/.knowledge/records/x.md";
        let cs = checks(&[path("main.rs", "must"), key("conf.yaml#replicas", "must", ">= 2")]);
        let b = tree(&[(x, &rec(1, "decision", &cs)), ("svc/a/main.rs", "fn main() {}\n"), ("svc/a/conf.yaml", "replicas: 2\n"), ("README", "x\n")]);
        let h = tree(&[("README", "x\n")]);
        let r = run(&b, &h);
        assert_eq!(found(&r), [(Class::Broken { rule: 2 }, x, "svc/a/conf.yaml#replicas"), (Class::Broken { rule: 2 }, x, "svc/a/main.rs")]);
        assert_eq!(r.knowledge.removed, [x]);
        // First change: the records alone are removed: changed, never a block.
        let h1 = tree(&[("svc/a/main.rs", "fn main() {}\n"), ("svc/a/conf.yaml", "replicas: 2\n"), ("README", "x\n")]);
        let r = run(&b, &h1);
        let changed = Class::Changed(ChangeKind::RecordRemoved);
        assert_eq!(found(&r), [(changed.clone(), x, "svc/a/conf.yaml#replicas"), (changed, x, "svc/a/main.rs")]);
        assert_eq!(r.findings[0].reasons, ["record-removed"]);
        // Second change: the service goes; nothing judges it any more.
        assert_eq!(found(&run(&h1, &h)), []);
    }

    #[test]
    fn unknown_is_not_an_excuse() {
        // Edge case 50: unverifiable at B (a template), violated at H: broken; unverifiable at both: nothing.
        let cs = checks(&[key("v.yaml#x", "must", "== 1")]);
        let b = tree(&[(&a("a.md"), &rec(1, "fact", &cs)), ("v.yaml", "x: '{{ v }}'\n")]);
        let h = tree(&[(&a("a.md"), &rec(1, "fact", &cs)), ("v.yaml", "x: 2\n")]);
        let r = run(&b, &h);
        assert_eq!(found(&r), [(Class::Broken { rule: 1 }, a("a.md").as_str(), "v.yaml#x")]);
        let h2 = tree(&[(&a("a.md"), &rec(1, "fact", &cs)), ("v.yaml", "x: '{{ w }}'\n")]);
        let r = run(&b, &h2);
        assert_eq!((found(&r), r.knowledge.new_unverifiable.len(), r.broken_before), (vec![], 0, 0));
    }

    #[test]
    fn no_longer_checkable_is_broken() {
        // Edge case 49: held at B, an expression (here a template) at H: rule 1, "no longer checkable".
        let cs = checks(&[key("v.yaml#x", "must", "== 1")]);
        let b = tree(&[(&a("a.md"), &rec(1, "fact", &cs)), ("v.yaml", "x: 1\n")]);
        let h = tree(&[(&a("a.md"), &rec(1, "fact", &cs)), ("v.yaml", "x: '{{ v }}'\n")]);
        let r = run(&b, &h);
        assert_eq!(found(&r), [(Class::Broken { rule: 1 }, a("a.md").as_str(), "v.yaml#x")]);
        assert_eq!(r.findings[0].reasons, ["no-longer-checkable: template"]);
    }

    #[test]
    fn broken_before_counts_only_touched() {
        // Rule 1 excuses a must broken at both trees; the count holds only checks whose inputs or folder the
        // change touched. svc/c lies above no changed path: it is never read, so its broken must never fires.
        let t = |r: &str| {
            tree(&[
                (&a("root.md"), &rec(1, "fact", &checks(&[path("gone.txt", "must"), key("v.yaml#r", "must", ">= 5")]))),
                ("svc/b/.knowledge/records/b.md", &rec(2, "fact", &checks(&[path("missing", "must")]))),
                ("svc/c/.knowledge/records/c.md", &rec(3, "fact", &checks(&[path("missing", "must"), path("missing", "assumes")]))),
                ("v.yaml", r),
            ])
        };
        let b = t("r: 1\n");
        let mut h = t("r: 2\n");
        h.file("svc/b/other.txt", "x\n");
        let r = run(&b, &h);
        assert_eq!((found(&r), r.broken_before), (vec![], 1));
    }

    #[test]
    fn vendored_twin_does_not_absorb_deletion() {
        // the same check is sought only in folders above a changed path; the untouched twin is not one.
        let rec_a = rec(1, "fact", &checks(&[path("main.rs", "must")]));
        let b = tree(&[
            ("svc/a/.knowledge/records/x.md", &rec_a),
            ("svc/a/main.rs", "fn main() {}\n"),
            ("vendor/v/.knowledge/records/x.md", &rec_a),
            ("vendor/v/main.rs", "fn main() {}\n"),
        ]);
        let h = tree(&[("vendor/v/.knowledge/records/x.md", &rec_a), ("vendor/v/main.rs", "fn main() {}\n")]);
        assert_eq!(found(&run(&b, &h)), [(Class::Broken { rule: 2 }, "svc/a/.knowledge/records/x.md", "svc/a/main.rs")]);
    }

    #[test]
    fn decision_to_question_keeps_judging() {
        // Edge case 69: the kind never changes judging; with the checks kept rule 1 judges, removed rule 2.
        let cs = checks(&[key("v.yaml#replicas", "must", ">= 2")]);
        let b = tree(&[(&a("a.md"), &rec(1, "decision", &cs)), ("v.yaml", "replicas: 3\n")]);
        let h = tree(&[(&a("a.md"), &rec(1, "question", &cs)), ("v.yaml", "replicas: 1\n")]);
        assert_eq!(found(&run(&b, &h)), [(Class::Broken { rule: 1 }, a("a.md").as_str(), "v.yaml#replicas")]);
        let h = tree(&[(&a("a.md"), &rec(1, "question", "")), ("v.yaml", "replicas: 1\n")]);
        assert_eq!(found(&run(&b, &h)), [(Class::Broken { rule: 2 }, a("a.md").as_str(), "v.yaml#replicas")]);
    }

    #[test]
    fn twin_holding_is_not_excused() {
        // H's check is the same check as two of B's (both B paths are gone from H); one held at B, so
        // rule 1 does not excuse it.
        let one = |p: &str| rec(1, "fact", &checks(&[path(p, "must")]));
        let b = tree(&[("a/.knowledge/records/x.md", &one("f")), ("a/f", "x\n"), ("b/.knowledge/records/x.md", &one("f"))]);
        let h = tree(&[("c/.knowledge/records/x.md", &one("f"))]);
        let r = run(&b, &h);
        assert_eq!(found(&r), [(Class::Broken { rule: 1 }, "c/.knowledge/records/x.md", "c/f")]);
        let mut followed = r.knowledge.followed.clone();
        followed.sort();
        assert_eq!(followed, [("a/f".to_owned(), "c/f".to_owned()), ("b/f".to_owned(), "c/f".to_owned())]);
    }

    #[test]
    fn exact_twin_first_elsewhere_removed() {
        // The untouched root check has its own copy in B, so the removed twin of svc/a (same id, a path
        // that matched at B) is not its same check. The removal is judged by rule 2; the root is not judged.
        let one = rec(1, "fact", &checks(&[path("main.rs", "must")]));
        let b = tree(&[(&a("x.md"), &one), ("svc/a/.knowledge/records/x.md", &one), ("svc/a/main.rs", "fn main() {}\n")]);
        let h = tree(&[(&a("x.md"), &one)]);
        let r = run(&b, &h);
        assert_eq!(found(&r), [(Class::Broken { rule: 2 }, "svc/a/.knowledge/records/x.md", "svc/a/main.rs")]);
        assert_eq!((r.broken_before, r.knowledge.followed.as_slice()), (0, [].as_slice()));
        assert_eq!(gate(&b, &h, &b.changed_paths(&h)).unwrap(), r);
    }

    #[test]
    fn exact_twin_first_untouched_layers() {
        // Two untouched twins, one with a layer that exists and overrides (broken), one with a layer that
        // matches nothing (holds). The broken one is judged against its own copy only: no finding.
        // Paths are relative to the folder: both twins read svc/a/v.yaml.
        let k = |v: &str, l: &str| format!("  - config_key:\n      - '{v}#replicas'\n      - '{l}#replicas'\n    must: '>= 2'\n");
        let x = rec(1, "fact", &checks(&[k("svc/a/v.yaml", "svc/a/env/prod.yaml")]));
        let y = rec(1, "fact", &checks(&[k("v.yaml", "prod.yaml")]));
        let files = [(a("x.md"), x), ("svc/a/.knowledge/records/y.md".to_owned(), y), ("svc/a/v.yaml".to_owned(), "replicas: 3\n".to_owned()), ("svc/a/env/prod.yaml".to_owned(), "replicas: 1\n".to_owned())];
        let entries: Vec<(&str, &str)> = files.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
        let b = tree(&entries);
        let mut h = b.clone();
        h.file("svc/a/README", "x\n");
        let r = run(&b, &h);
        assert_eq!((found(&r), r.broken_before), (vec![], 0));
        assert_eq!(gate(&b, &h, &b.changed_paths(&h)).unwrap(), r);
    }

    #[test]
    fn past_the_bound_judges_every_check() {
        // D5: a record added at the root pushes svc/a's record past the bound in H only. That record is
        // untouched, but it is no longer live in H, so rule 2 judges it: the footprint must judge it too.
        let one = rec(1, "fact", &checks(&[path("f", "must")]));
        let b = tree(&[("svc/a/.knowledge/records/x.md", &one), ("svc/a/f", "x\n")]);
        let mut h = b.clone();
        h.file(&a("0.md"), rec(2, "fact", "")).file("svc/a/README", "x\n");
        let changed = b.changed_paths(&h);
        let full = run_within(&b, &h, &changed, true, 1).unwrap();
        assert_eq!(full.findings.iter().map(|f| f.record_path.as_str()).collect::<Vec<_>>(), ["svc/a/.knowledge/records/x.md"]);
        assert_eq!(run_within(&b, &h, &changed, false, 1).unwrap(), full);
    }

    #[test]
    fn never_verifiable_is_silent() {
        // A B must unverifiable at B and at H gives no finding when its record goes; one that can be
        // judged at H is changed.
        let unknown = "  - url: 'https://example.org'\n    must: 'exist'\n";
        let b = tree(&[(&a("a.md"), &rec(1, "fact", &format!("checks:\n{unknown}"))), ("README", "x\n")]);
        let h = tree(&[("README", "x\n")]);
        let r = run(&b, &h);
        assert_eq!((found(&r), r.knowledge.removed.as_slice()), (vec![], [a("a.md")].as_slice()));
        let cs = checks(&[key("v.yaml#x", "must", "== 1")]);
        let b = tree(&[(&a("a.md"), &rec(1, "fact", &cs)), ("v.yaml", "x: '{{ v }}'\n")]);
        let h = tree(&[("v.yaml", "x: 1\n")]);
        assert_eq!(found(&run(&b, &h)), [(Class::Changed(ChangeKind::RecordRemoved), a("a.md").as_str(), "v.yaml#x")]);
    }

    #[test]
    fn new_unverifiable_assumes_is_silent() {
        // Edge case 51: a new check that cannot be judged is not a finding; a new must is listed.
        let unknown = |verb: &str| format!("  - url: 'https://example.org'\n    {verb}: 'exist'\n");
        let b = tree(&[("README", "x\n")]);
        let h = tree(&[("README", "x\n"), (&a("a.md"), &rec(1, "fact", &format!("checks:\n{}{}", unknown("assumes"), unknown("must"))))]);
        let r = run(&b, &h);
        assert_eq!(found(&r), []);
        assert_eq!(r.knowledge.new_unverifiable, [(a("a.md"), 1, "unknown".to_owned())]);
        assert_eq!(r.knowledge.added, [a("a.md")]);
    }

    #[test]
    fn invalidated_record_says_invalid() {
        // A record made invalid has no id; it is found at its own path.
        let cs = checks(&[key("v.yaml#replicas", "must", ">= 2")]);
        let b = tree(&[(&a("a.md"), &rec(1, "fact", &cs)), ("v.yaml", "replicas: 3\n")]);
        let broken = format!("{}<<<<<<< ours\nx\n=======\ny\n>>>>>>> theirs\n", rec(1, "fact", &cs));
        let h = tree(&[(&a("a.md"), &broken), ("v.yaml", "replicas: 3\n")]);
        assert_eq!(found(&run(&b, &h)), [(Class::Changed(ChangeKind::Invalid), a("a.md").as_str(), "v.yaml#replicas")]);
    }

    #[test]
    fn directory_anchor_does_not_follow() {
        // B's directory anchor still has files under it in H, so the rewritten anchor is not the same check.
        let b = tree(&[(&a("a.md"), &rec(1, "fact", &checks(&[path("roles/vault", "must")]))), ("roles/vault/main.yml", "x\n"), ("roles/other/main.yml", "x\n")]);
        let h = tree(&[(&a("a.md"), &rec(1, "fact", &checks(&[path("roles/other", "must")]))), ("roles/vault/main.yml", "x\n"), ("roles/other/main.yml", "x\n")]);
        let kind = ChangeKind::AnchorMoved { from: "roles/vault".into(), to: "roles/other".into() };
        assert_eq!(found(&run(&b, &h)), [(Class::Changed(kind), a("a.md").as_str(), "roles/vault")]);
    }

    #[test]
    fn superseded_and_newer_schema() {
        // Edge cases 18 and 22: superseded or moved to a newer schema in the change: rule 2 judges B's must.
        let cs = checks(&[key("v.yaml#replicas", "must", ">= 2")]);
        let b = tree(&[(&a("a.md"), &rec(1, "decision", &cs)), ("v.yaml", "replicas: 3\n")]);
        let sup = rec(2, "decision", "supersedes: 'k3v9q2x7m001'\n");
        let h = tree(&[(&a("a.md"), &rec(1, "decision", &cs)), (&a("b.md"), &sup), ("v.yaml", "replicas: 3\n")]);
        assert_eq!(found(&run(&b, &h)), [(Class::Changed(ChangeKind::Superseded), a("a.md").as_str(), "v.yaml#replicas")]);
        let h = tree(&[(&a("a.md"), &rec(1, "decision", &cs)), (&a("b.md"), &sup), ("v.yaml", "replicas: 1\n")]);
        assert_eq!(found(&run(&b, &h)), [(Class::Broken { rule: 2 }, a("a.md").as_str(), "v.yaml#replicas")]);
        let newer = rec(1, "decision", &cs).replace("knowledge: 1", "knowledge: 2");
        let h = tree(&[(&a("a.md"), &newer), ("v.yaml", "replicas: 3\n")]);
        assert_eq!(found(&run(&b, &h)), [(Class::Changed(ChangeKind::NewerSchema), a("a.md").as_str(), "v.yaml#replicas")]);
    }

    #[test]
    fn revived_record_judged_without_excuse() {
        // Edge case 19: deleting the superseding record revives a broken must; it was not live in B.
        let cs = checks(&[key("v.yaml#replicas", "must", ">= 2")]);
        let sup = rec(2, "decision", "supersedes: 'k3v9q2x7m001'\n");
        let b = tree(&[(&a("a.md"), &rec(1, "decision", &cs)), (&a("b.md"), &sup), ("v.yaml", "replicas: 1\n")]);
        let h = tree(&[(&a("a.md"), &rec(1, "decision", &cs)), ("v.yaml", "replicas: 1\n")]);
        let r = run(&b, &h);
        assert_eq!((found(&r), r.broken_before), (vec![(Class::Broken { rule: 1 }, a("a.md").as_str(), "v.yaml#replicas")], 0));
    }

    #[test]
    fn change_findings_only_on_touched_records() {
        // a duplicate id, a supersedes without effect and a cycle are findings only on record files the
        // change added or edited; an old duplicate in a folder the change also reads is not.
        let old = [(a("d1.md"), rec(9, "fact", "")), (a("d2.md"), rec(9, "fact", ""))];
        let mut b = tree(&[("svc/.knowledge/records/a.md", &rec(1, "fact", ""))]);
        let mut h = tree(&[
            ("svc/.knowledge/records/a.md", &rec(1, "fact", "")),
            ("svc/.knowledge/records/copy.md", &rec(1, "fact", "")),
            ("svc/.knowledge/records/s.md", &rec(5, "fact", "supersedes: 'k3v9q2x7m777'\n")),
            ("svc/.knowledge/records/c1.md", &rec(6, "fact", "supersedes: 'k3v9q2x7m007'\n")),
            ("svc/.knowledge/records/c2.md", &rec(7, "fact", "supersedes: 'k3v9q2x7m006'\n")),
        ]);
        for (p, s) in &old {
            b.file(p, s);
            h.file(p, s);
        }
        let r = run(&b, &h);
        assert_eq!(
            r.change_findings,
            [
                ChangeFinding::DuplicateId { id: id(1), paths: vec!["svc/.knowledge/records/a.md".into(), "svc/.knowledge/records/copy.md".into()] },
                ChangeFinding::Supersedes { path: "svc/.knowledge/records/s.md".into(), problem: SupersedeProblem::NotInFolder },
                ChangeFinding::Cycle { ids: vec![id(6), id(7)] },
            ]
        );
    }

    #[test]
    fn gate_reports_secret_in_added_record() {
        // `SEMANTICS.md` §Secrets: a secret shape in a record file the change added or edited is a change finding with rule,
        // line and column, by path; never a gate failure, never a record state. Untouched and removed ones are not.
        let tok = format!("db_sk_{}", "q7zx3mk9".repeat(4));
        let with = |n: usize| format!("---\nknowledge: 1\nid: 'k3v9q2x7m{n:03}'\nkind: 'fact'\ntitle: 't{n}'\n---\nUse {tok} here.\n");
        let b = tree(&[(&a("old.md"), &with(1)), (&a("edit.md"), &rec(2, "fact", "")), (&a("gone.md"), &with(3))]);
        let mut h = tree(&[(&a("old.md"), &with(1)), (&a("edit.md"), &with(2)), (&a("new.md"), &with(4))]);
        // A record file past its bound is screened on its first `record-file` bytes only (both clones agree).
        let pad = " ".repeat(crate::bounds::RECORD_FILE_MAX as usize);
        h.file(&a("big.md"), format!("{}{pad}{tok}\n", rec(5, "fact", "")));
        let r = run(&b, &h);
        assert_eq!(found(&r), []);
        let shape = |p: &str| ChangeFinding::SecretShape { path: a(p), rule: "docbrain_api_key", line: 7, col: 5 };
        assert_eq!(r.change_findings, [shape("edit.md"), shape("new.md")]);
        assert_eq!(gate(&b, &h, &b.changed_paths(&h)).unwrap(), r);
        for st in crate::folder::status(&h).unwrap().records {
            assert!(!format!("{st:?}").contains("Secret"), "{st:?}");
        }
    }

    #[test]
    fn anchored_file_past_its_bound_is_broken() {
        // Edge case 65: held at B, past the anchored-file bound at H: rule 1, no longer checkable.
        let cs = checks(&[key("big.toml#v", "must", "== 1")]);
        let b = tree(&[(&a("a.md"), &rec(1, "fact", &cs)), ("big.toml", "v = 1\n")]);
        let pad = crate::bounds::anchored_file_max(crate::format::Format::Toml) as usize;
        let big = format!("v = 1\npad = '{}'\n", "x".repeat(pad));
        let h = tree(&[(&a("a.md"), &rec(1, "fact", &cs)), ("big.toml", &big)]);
        let r = run(&b, &h);
        assert_eq!(found(&r), [(Class::Broken { rule: 1 }, a("a.md").as_str(), "big.toml#v")]);
        assert_eq!(r.findings[0].reasons, ["no-longer-checkable: bound:anchored-file"]);
    }

    #[test]
    fn same_check_compares_parsed_parts() {
        let parse = |s: &str| match crate::record::parse_record("", s.as_bytes()) {
            crate::record::Parsed::Valid(r) => r,
            other => panic!("{other:?}"),
        };
        let t = tree(&[("kept.txt", "x\n")]);
        let r1 = parse(&rec(1, "fact", &checks(&[key("v.yaml#a.\"b\"", "must", ">=2"), path("gone.txt", "must"), path("kept.txt", "must")])));
        let r2 = parse(&rec(1, "fact", &checks(&[key("v.yaml#a.b", "must", ">= 2"), path("new.txt", "must"), path("other.txt", "must")])));
        let same = |i: usize, j: usize| same_check((&r1.id, &r1.checks[i]), (&r2.id, &r2.checks[j]), &t).unwrap();
        // Parsed equal: quoting of a key segment and spacing after the op do not matter.
        assert!(same(0, 0));
        // With r2 as H and r1 as B: a different path is the same check only when B's matches nothing in H.
        let same_hb = |h: usize, b: usize| same_check((&r2.id, &r2.checks[h]), (&r1.id, &r1.checks[b]), &t).unwrap();
        assert!(same_hb(1, 1)); // B's gone.txt matches nothing in H: followed
        assert!(!same_hb(2, 2)); // B's kept.txt is still in H: not the same check
        let r3 = parse(&rec(2, "fact", &checks(&[key("v.yaml#a.b", "must", ">= 2")])));
        assert!(!same_check((&r3.id, &r3.checks[0]), (&r1.id, &r1.checks[0]), &t).unwrap()); // another id
        // A `code` text and an unknown kind's key are part of the check.
        let unknown = |k: &str| format!("  - {k}: 'x'\n    must: 'exist'\n");
        let r4 = parse(&rec(1, "fact", &checks(&[code("f.rs", "a()"), code("f.rs", "b()"), unknown("url"), unknown("ping")])));
        let same4 = |i: usize, j: usize| same_check((&r4.id, &r4.checks[i]), (&r4.id, &r4.checks[j]), &t).unwrap();
        assert_eq!([same4(0, 0), same4(0, 1), same4(2, 2), same4(2, 3)], [true, false, true, false]);
    }

    #[test]
    fn worst_class_first_then_path() {
        // Findings are ordered by class (broken, changed, stale) before record path.
        let recs = [(a("a.md"), rec(1, "fact", &checks(&[path("s.txt", "assumes")]))), (a("b.md"), rec(2, "fact", &checks(&[path("m.txt", "must")])))];
        let mut b = tree(&[("s.txt", "x\n"), ("m.txt", "x\n")]);
        let mut h = MemTree::new();
        for (p, s) in &recs {
            b.file(p, s);
            h.file(p, s);
        }
        assert_eq!(found(&run(&b, &h)), [(Class::Broken { rule: 1 }, a("b.md").as_str(), "m.txt"), (Class::Stale, a("a.md").as_str(), "s.txt")]);
    }

    #[test]
    fn a_group_keeps_its_first_members_title() {
        // Rule 1's member (H's record) and rule 2's (B's) are one finding of one class; the first, H's,
        // names it.
        let b = tree(&[(&a("a.md"), &rec(1, "decision", &checks(&[key("v.yaml#replicas", "must", ">= 2")]))), ("v.yaml", "replicas: 3\n")]);
        let renamed = rec(1, "decision", &checks(&[key("v.yaml#replicas", "must", ">= 4")])).replace("title: 't1'", "title: 'renamed'");
        let h = tree(&[(&a("a.md"), &renamed), ("v.yaml", "replicas: 1\n")]);
        let r = run(&b, &h);
        assert_eq!(found(&r), [(Class::Broken { rule: 1 }, a("a.md").as_str(), "v.yaml#replicas")]);
        assert_eq!((r.findings[0].title.as_str(), r.findings[0].reasons.len()), ("renamed", 2));
    }

    #[test]
    fn a_file_that_is_no_record_is_no_knowledge_change() {
        // A file under records/ that is not `*.md` affects no check of the folder; a `*.md` outside
        // records/ is no record. A record added under records/ is both.
        let b = tree(&[(&a("a.md"), &rec(1, "fact", &checks(&[path("missing", "must")])))]);
        let mut h = b.clone();
        h.file(&a("notes.txt"), "x\n").file("README.md", "x\n");
        let r = run(&b, &h);
        assert_eq!((found(&r), r.broken_before, r.knowledge.added.len(), r.knowledge.edited.len()), (vec![], 0, 0, 0));
        h.file(&a("b.md"), rec(2, "fact", ""));
        let r = run(&b, &h);
        assert_eq!((r.broken_before, r.knowledge.added.clone()), (1, vec![a("b.md")]));
    }

    #[test]
    fn an_old_cycle_is_no_finding_of_an_unrelated_change() {
        // a cycle is a change finding only when a record the change added or edited is in it.
        let c1 = rec(6, "fact", "supersedes: 'k3v9q2x7m007'\n");
        let mut b = tree(&[(&a("c1.md"), &c1), (&a("c2.md"), &rec(7, "fact", "supersedes: 'k3v9q2x7m006'\n"))]);
        b.file(&a("d.md"), rec(8, "fact", ""));
        let mut h = b.clone();
        h.file(&a("d.md"), rec(8, "fact", "").replace("More.", "Edited."));
        assert_eq!(run(&b, &h).change_findings, []);
        h.file(&a("c1.md"), c1.replace("More.", "Edited."));
        assert_eq!(run(&b, &h).change_findings, [ChangeFinding::Cycle { ids: vec![id(6), id(7)] }]);
    }

    #[test]
    fn rule_2_no_longer_checkable_or_moved() {
        // B's must held at B and is unverifiable at H as B wrote it: broken by rule 2, no longer checkable.
        let b = tree(&[(&a("a.md"), &rec(1, "fact", &checks(&[key("v.yaml#x", "must", "== 1")]))), ("v.yaml", "x: 1\n")]);
        let h = tree(&[(&a("a.md"), &rec(1, "fact", "")), ("v.yaml", "x: '{{ v }}'\n")]);
        let r = run(&b, &h);
        assert_eq!(found(&r), [(Class::Broken { rule: 2 }, a("a.md").as_str(), "v.yaml#x")]);
        assert_eq!(r.findings[0].reasons, ["no-longer-checkable: template"]);
        // Its file moved: missing-file, naming the file the change added with the same content.
        let h = tree(&[(&a("a.md"), &rec(1, "fact", "")), ("w.yaml", "x: 1\n")]);
        let r = run(&b, &h);
        assert_eq!(found(&r), [(Class::Broken { rule: 2 }, a("a.md").as_str(), "v.yaml#x")]);
        assert_eq!(r.findings[0].reasons, ["missing-file"]);
        assert_eq!(r.findings[0].same_content.as_deref(), Some("w.yaml"));
    }

    #[test]
    fn assumes_no_longer_checkable_is_stale() {
        // An assumes that held at B and is unverifiable at H is a stale note, no longer checkable.
        let cs = checks(&[key("v.yaml#x", "assumes", "== 1")]);
        let b = tree(&[(&a("a.md"), &rec(1, "fact", &cs)), ("v.yaml", "x: 1\n")]);
        let h = tree(&[(&a("a.md"), &rec(1, "fact", &cs)), ("v.yaml", "x: '{{ v }}'\n")]);
        let r = run(&b, &h);
        assert_eq!(found(&r), [(Class::Stale, a("a.md").as_str(), "v.yaml#x")]);
        assert_eq!(r.findings[0].reasons, ["no-longer-checkable: template"]);
    }

    #[test]
    fn change_kind_reads_the_live_record() {
        // Among H's records with B's id, the first live one by path says what happened (here the verb),
        // though a newer-schema copy with the id sorts first.
        let must = checks(&[key("v.yaml#x", "must", ">= 1")]);
        let b = tree(&[(&a("b.md"), &rec(1, "fact", &must)), ("v.yaml", "x: 1\n")]);
        let assumes = rec(1, "fact", &checks(&[key("v.yaml#x", "assumes", ">= 1")]));
        let newer = rec(1, "fact", &must).replace("knowledge: 1", "knowledge: 2");
        let h = tree(&[(&a("a.md"), &newer), (&a("b.md"), &assumes), ("v.yaml", "x: 1\n")]);
        assert_eq!(found(&run(&b, &h)), [(Class::Changed(ChangeKind::VerbChanged), a("b.md").as_str(), "v.yaml#x")]);
        // At B's path, a newer schema with no readable id: newer schema, not removed.
        let h = tree(&[(&a("b.md"), "---\nknowledge: 2\nkind: 'policy'\n---\nbody\n"), ("v.yaml", "x: 1\n")]);
        assert_eq!(found(&run(&b, &h)), [(Class::Changed(ChangeKind::NewerSchema), a("b.md").as_str(), "v.yaml#x")]);
    }

    #[test]
    fn past_the_bound_in_b_only_judges_every_check() {
        // D5 the other way: a root record the change deletes had pushed svc/a's past the bound in B, so its must,
        // broken at both trees, was never live in B: rule 1 has no excuse for it, in either form.
        let mut b = tree(&[("svc/a/.knowledge/records/x.md", &rec(1, "fact", &checks(&[path("f", "must")])))]);
        b.file(&a("0.md"), rec(2, "fact", ""));
        let mut h = b.clone();
        h.remove(&a("0.md")).file("svc/a/README", "x\n");
        let changed = b.changed_paths(&h);
        let full = run_within(&b, &h, &changed, true, 1).unwrap();
        assert_eq!(found(&full), [(Class::Broken { rule: 1 }, "svc/a/.knowledge/records/x.md", "svc/a/f")]);
        assert_eq!(run_within(&b, &h, &changed, false, 1).unwrap(), full);
    }
}
