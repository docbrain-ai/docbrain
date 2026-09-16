//! Did the instance behind this URL change since the last time this client looked?
//!
//! `~/.docbrain/config.json` carries the server URL and the key together, so anything that
//! rewrites it redirects every later command. Printing the URL catches a move to a different
//! host. It cannot catch the quieter half: the same URL in front of a different corpus — a
//! restored snapshot, a re-seeded dev stack, a container rebuilt against an empty database.
//! That instance answers 200 with a plausible answer drawn from documents the reader has
//! never seen.
//!
//! A number printed once catches nothing, because nobody remembers that it was 39,903
//! yesterday. What catches it is the comparison: this client records what it last saw at
//! each URL and says something when the identity changes or the corpus falls away.
//!
//! Two rules, deliberately asymmetric:
//!
//!   * A changed `install_id` is unconditional. The id is minted once per database, so a
//!     different id at the same URL is a different database, full stop.
//!   * A changed count is only worth saying when it FALLS, and by more than the configured
//!     fraction. Corpora grow constantly and shrink a little (deletions, retention); a
//!     client that alarms on drift is a client people learn to ignore.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The fraction of a corpus that must disappear between two observations before this client
/// says anything, when nothing configures it. Used only as the fallback: the value comes
/// from `corpus_drop_alert_fraction` in `~/.docbrain/config.json`, or from
/// `DOCBRAIN_CORPUS_DROP_ALERT`, so a deployment whose corpus legitimately churns can raise
/// it and one that never loses a document can lower it to near zero.
pub const DEFAULT_DROP_FRACTION: f64 = 0.10;

/// What the server said this time.
#[derive(Debug, Clone, PartialEq)]
pub struct Observed {
    /// `None` when the server has not minted an identity yet — an instance that has never
    /// completed a boot. Not a mismatch: nothing to compare.
    pub install_id: Option<String>,
    pub documents: i64,
    /// What this deployment cannot currently use to answer, one sentence each, already
    /// worded by the server. The CLI does not compose its own: a command line, a Slack
    /// message and a web page that word one fact differently teach three vocabularies for
    /// it.
    pub unavailable: Vec<String>,
}

/// What this client recorded at the same URL last time.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Seen {
    #[serde(default)]
    pub install_id: Option<String>,
    #[serde(default)]
    pub documents: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// A different database now answers at this URL.
    Instance { previous: String, current: String },
    /// The same database holds materially fewer documents than it did.
    CorpusFell { previous: i64, current: i64 },
}

impl Change {
    /// One line, addressed to what the reader should do about it.
    pub fn message(&self) -> String {
        match self {
            Change::Instance { previous, current } => format!(
                "this URL now answers as a different instance ({} — it was {}); \
                 answers come from a different corpus than the one you saw before",
                short(current),
                short(previous),
            ),
            Change::CorpusFell { previous, current } => format!(
                "this instance holds {} documents, down from {} when you last looked; \
                 answers may be missing sources that used to be there",
                current, previous,
            ),
        }
    }
}

/// First 8 characters of an id — enough to tell two apart in one line of terminal.
fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

/// The comparison itself. Pure, so it can be tested without a server or a home directory.
///
/// Returns `None` on first sight of a URL: there is nothing to compare against, and warning
/// about a baseline would make the first run of every new instance look like a fault.
pub fn compare(previous: Option<&Seen>, now: &Observed, drop_fraction: f64) -> Option<Change> {
    let previous = previous?;

    // Identity first: a different database makes any count comparison meaningless.
    if let (Some(before), Some(current)) = (previous.install_id.as_deref(), now.install_id.as_deref())
        && before != current
    {
        return Some(Change::Instance {
            previous: before.to_string(),
            current: current.to_string(),
        });
    }

    // A corpus that was empty before has no fraction to compute against.
    if previous.documents <= 0 {
        return None;
    }
    // A corpus that grew or held steady says nothing — including when the threshold is
    // configured to zero, where "no documents lost" would otherwise read as a loss.
    let lost = previous.documents - now.documents;
    if lost <= 0 {
        return None;
    }
    if lost as f64 / previous.documents as f64 >= drop_fraction {
        return Some(Change::CorpusFell {
            previous: previous.documents,
            current: now.documents,
        });
    }
    None
}

// ── The record this client keeps ────────────────────────────────────────────

/// `~/.docbrain/instances.json`, keyed by server URL. Separate from `config.json` on
/// purpose: that file holds the key, and a bookkeeping write must never be able to damage
/// the one file that makes the CLI work at all.
fn state_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".docbrain").join("instances.json"))
}

/// Trailing slashes are cosmetic; two spellings of one URL must not read as two instances.
pub fn normalise_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn load_all() -> BTreeMap<String, Seen> {
    let Some(path) = state_path() else {
        return BTreeMap::new();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn last_seen(server_url: &str) -> Option<Seen> {
    load_all().remove(&normalise_url(server_url))
}

/// Record what this URL answered. Best-effort by design: a client that cannot write its own
/// bookkeeping must still answer questions, so every failure here is silent.
pub fn record(server_url: &str, now: &Observed) {
    let Some(path) = state_path() else { return };
    let mut all = load_all();
    all.insert(
        normalise_url(server_url),
        Seen { install_id: now.install_id.clone(), documents: now.documents },
    );
    let Ok(body) = serde_json::to_string_pretty(&all) else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Write-then-rename: an interrupted write must not leave a half-file that reads as
    // "this URL was never seen" — which would silently disable the comparison.
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// The alert threshold: `DOCBRAIN_CORPUS_DROP_ALERT` beats the config file, which beats the
/// documented default. Values outside `0.0..=1.0` are ignored rather than obeyed — a typo
/// that reads as `10` would otherwise mean "never alarm".
pub fn drop_fraction(configured: Option<f64>) -> f64 {
    let from_env = std::env::var("DOCBRAIN_CORPUS_DROP_ALERT")
        .ok()
        .and_then(|v| v.parse::<f64>().ok());
    resolve_drop_fraction(from_env, configured)
}

/// The precedence rule itself, with the environment passed in rather than read. Tests that
/// read the process environment in a threaded runner test whatever another test set last;
/// this crate has been bitten by that class three times.
pub fn resolve_drop_fraction(from_env: Option<f64>, configured: Option<f64>) -> f64 {
    from_env
        .or(configured)
        .filter(|f| (0.0..=1.0).contains(f))
        .unwrap_or(DEFAULT_DROP_FRACTION)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(id: &str, docs: i64) -> Seen {
        Seen { install_id: Some(id.to_string()), documents: docs }
    }
    fn now(id: &str, docs: i64) -> Observed {
        Observed { install_id: Some(id.to_string()), documents: docs, unavailable: Vec::new() }
    }

    #[test]
    fn first_sight_of_a_url_is_not_a_warning() {
        assert_eq!(compare(None, &now("a", 100), 0.10), None);
    }

    #[test]
    fn the_same_instance_with_the_same_corpus_says_nothing() {
        assert_eq!(compare(Some(&seen("a", 100)), &now("a", 100), 0.10), None);
    }

    #[test]
    fn a_different_install_id_is_reported_however_small_the_corpus_change() {
        let change = compare(Some(&seen("aaaa1111", 100)), &now("bbbb2222", 100), 0.10);
        assert_eq!(
            change,
            Some(Change::Instance {
                previous: "aaaa1111".into(),
                current: "bbbb2222".into()
            })
        );
        // The message must carry enough of both ids to tell them apart.
        let msg = change.unwrap().message();
        assert!(msg.contains("bbbb2222"), "current id missing: {msg}");
        assert!(msg.contains("aaaa1111"), "previous id missing: {msg}");
    }

    #[test]
    fn a_corpus_that_grew_says_nothing() {
        assert_eq!(compare(Some(&seen("a", 100)), &now("a", 4000), 0.10), None);
    }

    #[test]
    fn a_small_shrink_is_normal_churn_and_stays_quiet() {
        // 100 → 95 is 5%, below the 10% default: deletions and retention do this.
        assert_eq!(compare(Some(&seen("a", 100)), &now("a", 95), 0.10), None);
    }

    #[test]
    fn a_corpus_that_fell_past_the_threshold_is_reported() {
        assert_eq!(
            compare(Some(&seen("a", 100)), &now("a", 89), 0.10),
            Some(Change::CorpusFell { previous: 100, current: 89 })
        );
    }

    #[test]
    fn the_threshold_is_the_boundary_it_claims_to_be() {
        // Exactly at the fraction alarms; one document less than it does not.
        assert!(compare(Some(&seen("a", 100)), &now("a", 90), 0.10).is_some());
        assert!(compare(Some(&seen("a", 100)), &now("a", 91), 0.10).is_none());
    }

    #[test]
    fn at_a_zero_threshold_a_steady_corpus_is_still_silent() {
        // A deployment that never loses a document can set the threshold to zero and ask to
        // hear about a single missing one. That must not turn "nothing changed" into a
        // warning on every run — the difference between the two is the `lost <= 0` guard,
        // and a threshold of zero is the only setting under which it is load-bearing.
        assert_eq!(compare(Some(&seen("a", 100)), &now("a", 100), 0.0), None);
        assert_eq!(compare(Some(&seen("a", 100)), &now("a", 140), 0.0), None);
        assert_eq!(
            compare(Some(&seen("a", 100)), &now("a", 99), 0.0),
            Some(Change::CorpusFell { previous: 100, current: 99 })
        );
    }

    #[test]
    fn an_emptied_corpus_is_reported_at_any_threshold() {
        assert_eq!(
            compare(Some(&seen("a", 39903)), &now("a", 0), 1.0),
            Some(Change::CorpusFell { previous: 39903, current: 0 })
        );
    }

    #[test]
    fn an_instance_that_has_not_minted_an_id_is_not_a_mismatch() {
        // Nothing to compare against: report on the corpus alone, never on a missing id.
        let observed = Observed { install_id: None, documents: 100, unavailable: Vec::new() };
        assert_eq!(compare(Some(&seen("a", 100)), &observed, 0.10), None);
        let previous = Seen { install_id: None, documents: 100 };
        assert_eq!(compare(Some(&previous), &now("a", 100), 0.10), None);
    }

    #[test]
    fn a_url_that_differs_only_by_a_trailing_slash_is_one_url() {
        assert_eq!(normalise_url("http://h:3000/"), normalise_url("http://h:3000"));
    }

    #[test]
    fn a_nonsense_threshold_falls_back_to_the_default_rather_than_disabling_the_check() {
        assert_eq!(resolve_drop_fraction(None, Some(10.0)), DEFAULT_DROP_FRACTION);
        assert_eq!(resolve_drop_fraction(None, Some(-1.0)), DEFAULT_DROP_FRACTION);
        assert_eq!(resolve_drop_fraction(None, Some(0.5)), 0.5);
        assert_eq!(resolve_drop_fraction(None, None), DEFAULT_DROP_FRACTION);
    }

    #[test]
    fn the_environment_overrides_the_config_file() {
        assert_eq!(resolve_drop_fraction(Some(0.4), Some(0.9)), 0.4);
    }
}
