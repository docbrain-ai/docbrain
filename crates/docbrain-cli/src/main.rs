// SPDX-License-Identifier: MIT
use anyhow::Result;
use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use dotenvy::dotenv;
use std::io::Write;
use std::path::PathBuf;

// ═══════════════════════════════════════════════════════════════════════════
// CLI structure
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Parser)]
#[command(name = "docbrain", about = "AI-powered documentation intelligence", version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// API key for authentication.
    /// Precedence: --api-key flag > ~/.docbrain/config.json > DOCBRAIN_API_KEY env
    #[arg(long, global = true, hide_env_values = true)]
    api_key: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Log in and store session key in ~/.docbrain/config.json.
    /// Default: email + password. Use --github/--gitlab/--oidc for SSO via browser.
    Login {
        /// DocBrain server URL (overrides DOCBRAIN_SERVER_URL env and saved config)
        #[arg(long)]
        server: Option<String>,
        /// Email address (prompted interactively if omitted; ignored for SSO login)
        #[arg(long)]
        email: Option<String>,
        /// Log in via GitHub OAuth (opens browser)
        #[arg(long, conflicts_with_all = &["gitlab", "oidc"])]
        github: bool,
        /// Log in via GitLab OIDC (opens browser)
        #[arg(long, conflicts_with_all = &["github", "oidc"])]
        gitlab: bool,
        /// Log in via OIDC/SSO provider (opens browser; for Microsoft, Google, etc.)
        #[arg(long, conflicts_with_all = &["github", "gitlab"])]
        oidc: bool,
    },
    /// Log out — revokes session key server-side and clears local config
    Logout,
    /// Manage long-lived API tokens (admin only)
    Token {
        #[command(subcommand)]
        action: TokenAction,
    },
    /// Ask a question about your documentation
    Ask {
        /// The question to ask
        question: String,
        /// Continue a specific session (pass session_id from a previous response)
        #[arg(long)]
        session: Option<String>,
        /// Force a fresh session (ignore auto-resume)
        #[arg(long, short = 'n')]
        new: bool,
        /// Show raw UUIDs (session/episode IDs)
        #[arg(long, short = 'v')]
        verbose: bool,
    },
    /// Trace the retrieval pipeline for a question (admin only).
    ///
    /// Sends the question to `/api/v1/ask` with `trace: true`, receives the
    /// `pipeline_trace` field in the response, and pretty-prints it as a
    /// table: which retrievers fired, pool size, rerank provider, per-stage
    /// timings, and the final top-k chunks with titles and scores.
    ///
    /// Use when diagnosing "why didn't chunk X surface?" — the table shows
    /// which stage dropped each chunk. See docs/configuration.md for the
    /// trace schema.
    #[command(name = "trace-query")]
    TraceQuery {
        /// The question to trace
        question: String,
        /// Emit the raw JSON trace instead of the table render
        #[arg(long)]
        json: bool,
    },
    /// Thumbs up — mark the last answer as helpful
    #[command(name = "thumbsup")]
    ThumbsUp {
        /// Optional episode ID (full UUID or 8-char prefix). Defaults to latest.
        episode_id: Option<String>,
    },
    /// Thumbs down — mark the last answer as not helpful
    #[command(name = "thumbsdown")]
    ThumbsDown {
        /// Optional episode ID (full UUID or 8-char prefix). Defaults to latest.
        episode_id: Option<String>,
        /// Mark a SPECIFIC source (1-based index from the last answer's printed
        /// Sources list) as not relevant, instead of the whole answer.
        #[arg(long)]
        source: Option<usize>,
    },
    /// Submit feedback on a previous answer
    Feedback {
        /// Episode ID from the ask response
        episode_id: String,
        /// Feedback: "up" (thumbs up) or "down" (thumbs down)
        rating: String,
    },
    /// View freshness report for documentation
    Freshness {
        /// Space key (e.g., "SAAS"). Omit for all spaces.
        #[arg(long)]
        space: Option<String>,
    },
    /// Incident mode — search runbooks and past incidents
    Incident {
        /// Description of the incident or error (reads from stdin if omitted)
        description: Option<String>,
    },
    /// Generate a document grounded in your org's knowledge. Returns the
    /// markdown; does NOT publish.
    ///
    /// stdout carries ONLY the markdown (pipe-clean) so you can redirect it:
    ///   docbrain generate "runbook for cert rotation" --source notes.md > out.md
    ///   docbrain generate "postmortem from this incident" \
    ///     --source-url https://acme.slack.com/archives/C123/p1700000000123 > pm.md
    ///   docbrain generate "API reference for the changed endpoints" \
    ///     --source-url https://github.com/acme/repo/pull/42 --type reference
    /// All diagnostics (doc type, quality score, needs-input, skipped sources,
    /// violations) go to stderr. Exits non-zero on error-severity quality
    /// violations unless --allow-violations (CI-native behaviour).
    Generate {
        /// What to write (the ask). Required.
        ask: String,
        /// Source file(s) to use as PRIMARY material (repeatable). Read verbatim.
        #[arg(long = "source", value_name = "FILE")]
        sources: Vec<PathBuf>,
        /// Source LINK(s) to use as PRIMARY material (repeatable): a Confluence
        /// page, Jira issue, Slack thread, or GitHub PR/file URL. DocBrain FETCHES
        /// each via its connected MCP connector. If a link can't be fetched
        /// (connector not connected, fetch error, unsupported link) generation
        /// HARD-FAILS naming that source — never a doc from a partial set.
        #[arg(long = "source-url", value_name = "URL")]
        source_urls: Vec<String>,
        /// Read PRIMARY material from stdin.
        #[arg(long)]
        stdin: bool,
        /// Existing doc to augment (reference/url).
        #[arg(long)]
        target: Option<String>,
        /// Team template file. Captures section STRUCTURE, block SHAPE (table
        /// columns, checklists, code-block language, header-field names) and TONE;
        /// fills each section from YOUR sources, marks NEEDS INPUT for gaps, and
        /// never copies the template's example rows, commands, or placeholder text.
        /// Cannot affect safety/quality rules.
        #[arg(long)]
        template: Option<PathBuf>,
        /// Doc type hint (runbook|guide|troubleshooting|faq|reference). Else inferred.
        #[arg(long = "type")]
        doc_type: Option<String>,
        /// Confluence space whose quality rules apply (else global floor).
        #[arg(long)]
        space: Option<String>,
        /// Write markdown to this file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Overwrite the --out file if it already exists. Without this, an
        /// INTERACTIVE run (TTY) refuses to clobber an existing file and tells you
        /// to pass --force; a NON-interactive run (CI / pipe) overwrites as before
        /// (CI regenerates the same artifact every run by design).
        #[arg(long)]
        force: bool,
        /// Disable live-MCP tool enrichment.
        #[arg(long)]
        no_enrich: bool,
        /// Exit 0 even if the doc has error-severity quality violations.
        #[arg(long)]
        allow_violations: bool,
        /// Maximum auto-review rounds. After the first draft, if the freshness
        /// review flagged claims your evidence contradicts, DocBrain regenerates
        /// (asking the writer to reconcile each flag) until the flags clear or
        /// this many extra rounds are used. Set 0 to disable auto-review (a single
        /// pass). The loop stops early once a round resolves nothing, never runs
        /// indefinitely, and never silently ships unresolved flags — any that
        /// remain are reported on stderr. stdout carries only the final markdown.
        #[arg(long = "max-regen-rounds", default_value = "2")]
        max_regen_rounds: u32,
    },
    /// View usage analytics
    Analytics {
        /// Number of days to report on (default: 30)
        #[arg(long, default_value = "30")]
        days: i64,
    },
    /// Documentation Autopilot — view gaps, generate drafts, run analysis
    Autopilot {
        #[command(subcommand)]
        action: AutopilotAction,
    },
    /// Admin operations (requires admin role)
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
    /// Subscription status and identity reporting (admin only).
    ///
    /// Reports what your DocBrain server sees: subscription state, organisation
    /// and expiry. DocBrain does not change how it behaves based on any of it —
    /// nothing here can disable or limit the product.
    License {
        #[command(subcommand)]
        action: LicenseAction,
    },
    /// CI/CD pipeline knowledge capture — analyze PRs and deployments
    Ci {
        #[command(subcommand)]
        action: CiAction,
    },
    /// Evidence bundles — verify, explain, tabulate (all OFFLINE), and export.
    ///
    /// `verify`, `why` and `tables` are fully offline: no server, no API key,
    /// no config file needed — they read a `.dbev` file and nothing else. Only
    /// `export` talks to your DocBrain server. For a purpose-built, dependency-
    /// free verifier to hand an auditor, see the standalone `docbrain-verify`
    /// binary (same verdict + exit codes).
    Evidence {
        #[command(subcommand)]
        action: EvidenceAction,
    },
    /// Show CLI and server version
    Version,
}

#[derive(Subcommand)]
enum EvidenceAction {
    /// Verify a `.dbev` bundle OFFLINE and print a verdict.
    ///
    /// Exit code IS the verdict: 0 VALID, 1 TAMPERED, 2 CANNOT_VERIFY, 3 a
    /// CLI-level error (file missing/unreadable). With `--against <earlier>`,
    /// additionally cross-checks the two bundles for a forked journal (see the
    /// printed caveat); a proven fork forces exit 1 even if this bundle alone
    /// is VALID.
    Verify {
        /// Path to the `.dbev` bundle to verify.
        bundle: PathBuf,
        /// An EARLIER `.dbev` of the same journal to cross-check against, for
        /// fork detection over the overlapping position range.
        #[arg(long)]
        against: Option<PathBuf>,
        /// Emit the machine-readable verdict JSON instead of the human report.
        #[arg(long)]
        json: bool,
    },
    /// Explain one record's story from a VALID bundle, OFFLINE.
    ///
    /// Refuses (exit 2) unless the bundle verifies VALID first — never renders
    /// content from an unverified bundle. `record` is a journal position, or an
    /// id carried in a record body.
    Why {
        /// The record to explain: a journal position (e.g. `42`) or a record-
        /// body id (decision/fragment/premise id).
        record: String,
        /// Path to the `.dbev` bundle.
        bundle: PathBuf,
    },
    /// Write a populations CSV (record counts per class/kind) with a
    /// bundle-digest header row, OFFLINE. Refuses unless the bundle is VALID.
    Tables {
        /// Path to the `.dbev` bundle.
        bundle: PathBuf,
        /// Output CSV path.
        out: PathBuf,
    },
    /// Export a signed `.dbev` bundle from your DocBrain server (admin).
    ///
    /// This is the ONLY evidence subcommand that touches the network. Range
    /// and preset are alternatives, not both.
    Export {
        /// Explicit checkpoint-boundary range `START,END` (e.g. `0,1200`).
        /// Mutually exclusive with --preset.
        #[arg(long, value_name = "START,END")]
        range: Option<String>,
        /// Compliance profile id (default: the server's default profile).
        #[arg(long)]
        profile: Option<String>,
        /// Named retention window within the profile (e.g. a "last-N-days"
        /// preset). Mutually exclusive with --range.
        #[arg(long)]
        preset: Option<String>,
        /// Where to write the downloaded `.dbev` bundle.
        #[arg(short = 'o', long = "output", value_name = "FILE")]
        out: PathBuf,
    },
}

#[derive(Subcommand)]
enum AdminAction {
    /// Ingest management
    Ingest {
        #[command(subcommand)]
        action: AdminIngestAction,
    },
}

#[derive(Subcommand)]
enum LicenseAction {
    /// Show current subscription certificate status
    Show,
    /// Identity report — how many people your connected sources account for.
    ///
    /// Breaks the count down per source, and gives a lower and upper bound on
    /// distinct people, because the same person can appear in more than one
    /// source and DocBrain does not try to merge them. Useful for checking your
    /// own figures before a renewal, and with --export it produces the signed
    /// report to send with one.
    ///
    /// Read-only. It reports and never changes anything, including how DocBrain
    /// behaves.
    Attest {
        /// Write the signed report to a file instead of printing it. This is the
        /// file to send with a renewal — it is signed so the figures in it can be
        /// checked without being taken on trust.
        #[arg(long)]
        export: bool,
        /// Where to write the signed report. Required with --export.
        #[arg(short = 'o', long = "output", value_name = "FILE")]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum AdminIngestAction {
    /// Trigger an adhoc ingest run on the server (admin only)
    Trigger {
        /// Comma-separated list of sources to ingest (e.g. "confluence,github").
        /// Omit to use the server's configured INGEST_SOURCES.
        #[arg(long)]
        sources: Option<String>,
    },
}

#[derive(Subcommand)]
enum TokenAction {
    /// Create a new named API token
    Create {
        /// Token name (e.g., "MCP Server Key", "CI/CD Key")
        #[arg(long)]
        name: String,
        /// Role: viewer or admin
        #[arg(long, default_value = "viewer")]
        role: String,
    },
    /// List active API tokens
    List,
    /// Revoke an API token by ID
    Revoke {
        /// Token ID (from `docbrain token list`)
        id: String,
    },
}

#[derive(Subcommand)]
enum AutopilotAction {
    /// Show Autopilot summary (gap counts, draft counts)
    Summary,
    /// List documentation gap clusters
    Gaps {
        /// Maximum number of gaps to show
        #[arg(long, default_value = "20")]
        limit: i64,
    },
    /// Run gap analysis now
    Analyze,
    /// List auto-generated drafts
    Drafts {
        /// Filter by status: pending_review, approved, published, rejected
        #[arg(long)]
        status: Option<String>,
    },
    /// Generate a draft for a specific gap cluster
    Generate {
        /// Gap cluster ID (UUID)
        cluster_id: String,
    },
    /// Dismiss a gap cluster
    Dismiss {
        /// Gap cluster ID (UUID)
        cluster_id: String,
    },
    /// Publish a draft to a target system (Confluence, GitHub, GitLab)
    Publish {
        /// Draft ID (UUID)
        draft_id: String,
        /// Override the target system: "confluence", "github", "gitlab"
        #[arg(long)]
        target: Option<String>,
    },
}

#[derive(Subcommand)]
enum CiAction {
    /// Analyze a merged PR and extract knowledge fragments
    Analyze {
        /// PR number
        #[arg(long)]
        pr_number: u64,
        /// Repository (e.g., "acme/platform")
        #[arg(long)]
        repo: String,
        /// PR title
        #[arg(long)]
        pr_title: String,
        /// PR description/body
        #[arg(long)]
        pr_body: Option<String>,
        /// Git diff stats (e.g., from `git diff --stat`)
        #[arg(long)]
        diff_stat: Option<String>,
        /// Comma-separated list of changed files
        #[arg(long)]
        changed_files: Option<String>,
        /// Comma-separated PR labels
        #[arg(long)]
        labels: Option<String>,
        /// PR author (email or username)
        #[arg(long)]
        author: Option<String>,
    },
    /// Capture deployment context as a knowledge fragment
    DeployCapture {
        /// Service name (e.g., "payment-gateway")
        #[arg(long)]
        service: String,
        /// Version being deployed (e.g., "2.4.1")
        #[arg(long)]
        version: String,
        /// Target environment (e.g., "production")
        #[arg(long)]
        environment: String,
        /// Changelog (e.g., from `git log --oneline v2.4.0..v2.4.1`)
        #[arg(long)]
        changelog: Option<String>,
        /// Config diff (e.g., from `diff prev.yaml current.yaml`)
        #[arg(long)]
        config_diff: Option<String>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// Config file — ~/.docbrain/config.json
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Default, Serialize, Deserialize)]
struct Config {
    server_url: Option<String>,
    api_key: Option<String>,
}

fn config_file_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".docbrain").join("config.json"))
}

fn read_config() -> Config {
    let path = match config_file_path() {
        Some(p) => p,
        None => return Config::default(),
    };
    if !path.exists() {
        return Config::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_config(cfg: &Config) -> Result<()> {
    let path = config_file_path()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(cfg)?)?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Resolution helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Resolve API key: --api-key flag > config file > DOCBRAIN_API_KEY env.
///
/// Config file wins over the env var so that `docbrain login` always takes effect immediately,
/// even when DOCBRAIN_API_KEY is exported in the shell. The env var is still honoured as a
/// last-resort fallback (useful in CI where there is no config file).
fn resolve_api_key(flag: Option<&str>) -> Option<String> {
    if let Some(k) = flag {
        return Some(k.to_string());
    }
    if let Some(k) = read_config().api_key {
        return Some(k);
    }
    std::env::var("DOCBRAIN_API_KEY").ok()
}

/// Resolve server URL: DOCBRAIN_SERVER_URL env → config file → default.
fn resolve_server_url() -> String {
    std::env::var("DOCBRAIN_SERVER_URL")
        .ok()
        .or_else(|| read_config().server_url)
        .unwrap_or_else(|| "http://localhost:3000".to_string())
}

// ═══════════════════════════════════════════════════════════════════════════
// API response types
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
struct LoginResponse {
    key: String,
    #[allow(dead_code)]
    id: String,
}

#[derive(Deserialize)]
struct AskResponse {
    answer: String,
    sources: Vec<SourceResponse>,
    session_id: Option<String>,
    episode_id: Option<String>,
    turn: Option<usize>,
    #[allow(dead_code)]
    intent: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct SourceResponse {
    heading: Option<String>,
    score: f32,
    source_url: String,
    title: String,
    /// Phase A — stable document id, cached after an ask so `thumbsdown --source N`
    /// can send it as source_ref.key.
    #[serde(default)]
    document_id: String,
    #[serde(default)]
    freshness_status: Option<String>,
    #[serde(default)]
    freshness_score: Option<f32>,
}

#[derive(Deserialize)]
struct FeedbackResponse {
    status: String,
}

#[derive(Deserialize)]
struct FreshnessReportResponse {
    space: Option<String>,
    summary: FreshnessSummaryResponse,
    documents: Vec<FreshnessDocResponse>,
}

#[derive(Deserialize)]
struct FreshnessSummaryResponse {
    total_docs: usize,
    fresh: usize,
    review: usize,
    stale: usize,
    outdated: usize,
    avg_score: f32,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct FreshnessDocResponse {
    title: String,
    source_url: String,
    total_score: f32,
    time_decay_score: f32,
    engagement_score: f32,
    content_currency_score: f32,
    last_edited_at: Option<String>,
    status: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct AnalyticsResponse {
    total_queries: i64,
    unique_users: i64,
    avg_feedback: f32,
    positive_feedback_pct: f32,
    top_queries: Vec<TopQueryResponse>,
    top_intents: Vec<IntentCountResponse>,
    doc_gaps: Vec<DocGapResponse>,
    most_retrieved_docs: Vec<RetrievedDocResponse>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct TopQueryResponse {
    query_text: String,
    count: i64,
    avg_feedback: f32,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct IntentCountResponse {
    intent: String,
    count: i64,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct DocGapResponse {
    label: String,
    occurrence_count: i64,
    #[allow(dead_code)]
    severity: String,
    #[allow(dead_code)]
    last_seen_at: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct RetrievedDocResponse {
    title: String,
    source_url: String,
    retrieval_count: i64,
    avg_feedback: f32,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct AutopilotSummaryResponse {
    total_gaps: i64,
    open_gaps: i64,
    critical_gaps: i64,
    drafts_generated: i64,
    drafts_published: i64,
    last_analysis_at: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct GapClusterResponse {
    id: String,
    label: String,
    description: String,
    query_count: i32,
    sample_queries: Vec<String>,
    avg_confidence: f32,
    severity: String,
    status: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct DraftResponse {
    id: String,
    cluster_id: Option<String>,
    title: String,
    content: String,
    content_type: String,
    quality_score: Option<f64>,
    status: String,
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum PipelineEvent {
    Started { phase: String, description: String },
    Completed {
        phase: String,
        duration_ms: u64,
        result_count: Option<usize>,
        // #214 — per-tool detail for the live_tools phase, e.g.
        // "jira_rest.search → ok, confluence_rest.search → truncated".
        #[serde(default)]
        detail: Option<String>,
    },
}

// ────────────────────────────────────────────────────────────────────────────────
// Ad-hoc generation response (AG-T10) — mirrors server `GeneratedArtifact`
// (the server's generate endpoint). Plain snake_case, no
// renames on the wire, so a struct match is exact. We deserialize only the
// fields the CLI surfaces; `#[allow(dead_code)]` covers fields kept for shape
// fidelity / future use.
// ────────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct GeneratedArtifactResponse {
    markdown: String,
    doc_type: String,
    #[serde(default)]
    needs_input: Vec<String>,
    #[serde(default)]
    skipped_sources: Vec<SkippedSourceResponse>,
    quality: QualityReportResponse,
    /// Per-section grounding. Not rendered to the user yet, but deserialized so
    /// the response shape stays faithful to the server artifact.
    #[serde(default)]
    #[allow(dead_code)]
    provenance: Vec<ProvenanceEntryResponse>,
    /// FC-T6/T7 — the freshness critic's per-claim verdict (only present when the
    /// critic ran and flagged something). Rendered to stderr like needs_input.
    #[serde(default)]
    freshness: Option<DraftFreshnessResponse>,
    /// B-T6 — the support critic's GROUNDING verdict: claims no source supports
    /// (fabrication) + spurious NEEDS-INPUT. Rendered to stderr like freshness.
    #[serde(default)]
    grounding: Option<GroundingResponse>,
    /// PRINT-ONLY in-place reconcile proposal for a `--target` generation.
    /// Present ONLY when a target was reconciled (target supplied + reconcile
    /// enabled + fetch ok + ≥1 existing-doc flag). Rendered as a per-section diff
    /// on stderr (stdout stays the generated doc, pipe-clean); the patch is NEVER
    /// applied from the CLI — a human re-fetches + applies (`--apply` is a
    /// fast-follow).
    #[serde(default)]
    reconcile_patch: Option<ReconcilePatchResponse>,
    /// The target page's Confluence `version` at fetch time, shown in the
    /// proposal header so a future `--apply` can version-check. Present exactly
    /// when `reconcile_patch` is.
    #[serde(default)]
    reconcile_base_version: Option<i64>,
    /// The CLEAN full merged document for a `--target` update. When present
    /// THIS is what stdout emits (the copy-paste / CI-safe artifact: unchanged
    /// sections byte-exact, updated replaced, new inserted), NOT the fresh
    /// `markdown`. Present ONLY on a `--target` merge run. The change-map glance is
    /// rendered to STDERR; stdout stays the clean merged doc (pipe-clean).
    #[serde(default)]
    merged_markdown: Option<String>,
    /// The structured per-section change manifest (the glance source).
    /// Present exactly when `merged_markdown` is.
    #[serde(default)]
    merge_manifest: Option<MergeManifestResponse>,
    /// The persisted History row id (server-set). Present when the doc was saved.
    #[serde(default)]
    #[allow(dead_code)]
    document_id: Option<String>,
    /// A shareable "view in browser" deep link, SERVER-composed from its own
    /// web-base config + the id (the CLI never composes/hardcodes it). Present only
    /// when the deployment set a web base URL AND the persist succeeded. Printed to
    /// stderr after a successful generate; absent → the CLI prints no link.
    #[serde(default)]
    view_url: Option<String>,
    /// The per-tool COVERAGE manifest for the live-MCP
    /// fan-out that fed this generation. Mirror of `Vec<ToolCoverage>`; serde-default
    /// + an empty Vec when no live fan-out ran (old server / corpus-only generation).
    /// The trust summary derives `searched_tools = coverage.len()` and
    /// `contributed_tools = coverage.filter(status=="hit" && doc_count>0).count()`.
    #[serde(default)]
    coverage: Vec<ToolCoverageResponse>,
    /// Distinct grounding sources (documents/tools/seeds), the breadth
    /// scalar the headline reads. `0` (serde-default) for an ungrounded doc or an
    /// old server that omits the field.
    #[serde(default)]
    distinct_sources: usize,
    /// `true` iff the doc shipped grounded ONLY in live/low-trust data
    /// (useful, NOT authoritative). serde-default `false` for old servers.
    #[serde(default)]
    unverified_live_only: bool,
    /// The per-claim LIVE cross-check verdicts. serde-default empty Vec
    /// when the cross-check did not run (old server / nothing live-verifiable).
    /// MUST be deserialized here or serde silently drops it (AskResponse precedent).
    #[serde(default)]
    cross_check: Vec<ClaimCrossCheckResponse>,
    /// T6 (CLI parity) — the live-dispatch coverage SLI (the server's authoritative
    /// scalar, never recomputed CLI-side). `None` only for an OLD server that omits the
    /// sibling field (serde-default); a current server ALWAYS sends an honest verdict
    /// (a `rate` or an `unavailable` cause). Printed as a one-liner in the summary,
    /// mirroring the web `dispatchRateLabel` tone so generate + History read the same.
    #[serde(default)]
    dispatch_rate: Option<DispatchRateResponse>,
    /// The post-assess review honesty ledger (loud-stale / unverified /
    /// gap counts). serde-default None when the review gate did not run.
    #[serde(default)]
    review: Option<ReviewVerdictResponse>,
}

/// Mirror of the server's `MergeManifest`. Plain
/// snake_case, no renames, so the struct match is exact.
#[derive(Deserialize)]
struct MergeManifestResponse {
    #[serde(default)]
    ops: Vec<MergeOpResponse>,
    #[serde(default)]
    unchanged_count: usize,
    #[serde(default)]
    updated_count: usize,
    #[serde(default)]
    new_count: usize,
    #[serde(default)]
    skipped_count: usize,
    #[serde(default)]
    existing_total: usize,
}

/// Mirror of `MergeOp` (serde tag = "status", lowercase). The CLI renders
/// the heading + status; the full before/after bodies are not echoed in the glance
/// (the merged doc on stdout carries the applied text). `#[allow(dead_code)]`
/// covers the body fields kept for shape fidelity.
#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
#[allow(dead_code)]
enum MergeOpResponse {
    Unchanged { heading: Option<String>, body: String },
    Updated {
        heading: Option<String>,
        #[serde(default)]
        ordinal: usize,
        before: String,
        after: String,
    },
    New { heading: Option<String>, body: String, after_ordinal: usize },
    Skipped { heading: Option<String>, reason: String },
}

/// Mirror of the server's `ReconcilePatch`. Plain
/// snake_case, no renames, so the struct match is exact.
#[derive(Deserialize)]
struct ReconcilePatchResponse {
    #[serde(default)]
    ops: Vec<ReconcileOpResponse>,
    #[serde(default)]
    skipped_ops: Vec<ReconcileSkippedOpResponse>,
}

/// One section the proposal would replace. The CLI renders the heading + reason
/// only (the storage bodies are apply-time detail the human does not need to read
/// in the diff); `#[allow(dead_code)]` covers the storage fields kept for shape
/// fidelity.
#[derive(Deserialize)]
#[allow(dead_code)]
struct ReconcileOpResponse {
    heading_level: u8,
    heading_ordinal: usize,
    anchor_storage: String,
    new_section_storage: String,
    /// DISPLAY-ONLY readable markdown forms of the before/after section. The CLI
    /// diff renders THESE (prose lines) instead of the storage XHTML wall. Serde
    /// `default` so an older server that omits them deserializes (and the diff
    /// falls back to the storage fields when they're empty).
    #[serde(default)]
    anchor_markdown: String,
    #[serde(default)]
    new_section_markdown: String,
    /// The contradicted claim that triggered this section's replacement — the
    /// human-readable "why" the CLI prints.
    reason: String,
    claim_hash: String,
}

/// One section the proposal SKIPPED (routed to human / kept original). Heading +
/// reason are both rendered.
#[derive(Deserialize)]
struct ReconcileSkippedOpResponse {
    #[serde(default)]
    heading: Option<String>,
    reason: String,
}

#[derive(Deserialize)]
struct DraftFreshnessResponse {
    #[serde(default)]
    flags: Vec<DraftFreshnessFlagResponse>,
    #[serde(default)]
    checked_count: usize,
    #[serde(default)]
    unchecked_count: usize,
}

#[derive(Deserialize)]
struct DraftFreshnessFlagResponse {
    claim: String,
    #[serde(default)]
    evidence_cited: Vec<String>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ProvenanceEntryResponse {
    section: Option<String>,
    #[serde(default)]
    source_ids: Vec<String>,
    /// A-T3 — the subset of `source_ids` from LIVE-TOOL data (not a stored source).
    #[serde(default)]
    live_source_ids: Vec<String>,
}

/// B-T6 — mirror of `support_critic::GroundingReport`. The doc-grounding verdict:
/// claims no source supports (fabrication) + spurious NEEDS-INPUT. Rendered to
/// stderr like the freshness flags.
#[derive(Deserialize)]
struct GroundingResponse {
    #[serde(default)]
    unsupported_claims: Vec<UnsupportedClaimResponse>,
    /// NoFab — ungrounded claims in a PRESERVED section of the user's EXISTING doc
    /// (Mode 2 / `--target`). Left untouched (the user's content), surfaced as a
    /// review-only list. serde-default empty for fresh-doc generation / old server.
    #[serde(default)]
    preexisting_unverified: Vec<UnsupportedClaimResponse>,
    #[serde(default)]
    spurious_needs_input: Vec<String>,
    #[serde(default)]
    checked_count: usize,
    #[serde(default)]
    supported_count: usize,
    /// The CONSERVATIVE verified scalar: ONLY non-quarantined High-tier
    /// supported claims. This (and ONLY this) feeds the "verified N" headline number.
    /// `Option`, NOT serde-default `0`: an old/persisted server that predates the
    /// tier-split OMITS this key, and absent MUST render "verification unavailable",
    /// never a false "0 of N verified" (which would assert all claims were checked and
    /// none reached an authoritative source). `None` = field absent (old server);
    /// `Some(0)` = a real fully-judged pass that found zero High-tier support. The TS
    /// surface already makes this distinction (`number | undefined`); this mirrors it.
    /// NEVER rendered as a number when the report is degraded (see `render_trust_summary`).
    #[serde(default)]
    supported_high_count: Option<usize>,
    /// Total claim-bearing blocks in the doc: the denominator surface the
    /// coverage ratio is measured against. serde-default `0`.
    #[serde(default)]
    total_blocks: usize,
    /// Claims present in the doc but NOT reached by the critic. serde-default
    /// `0`. Deserialized for wire fidelity; the summary derives the unchecked gap from
    /// `total_blocks - checked` (the same remainder), so this field is currently unread.
    #[serde(default)]
    #[allow(dead_code)]
    unchecked_count: usize,
    /// Dependencies unavailable during this grounding pass (LLM down, live
    /// tool failure). NON-EMPTY ⇒ the report is DEGRADED ⇒ verification is UNAVAILABLE
    /// (the headline must NOT emit a verified number). Mirror is loose (raw JSON values)
    /// because the CLI only needs the EMPTY/NON-EMPTY distinction, not the inner shape.
    /// serde-default empty for an old server / a clean pass.
    #[serde(default)]
    degraded: Vec<serde_json::Value>,
}

/// Mirror of the server's `ToolCoverage`: one row of
/// the live-fan-out coverage manifest (tool name, derived status, docs contributed).
/// `status` is deserialized as a plain `String` (the wire is snake_case: "hit",
/// "not_connected", …) rather than mirroring the full enum — the CLI only needs the
/// "hit" comparison, so a loose `String` is robust to future server-side variants
/// without a wire-shape break. `doc_count` serde-defaults to 0.
#[derive(Deserialize)]
struct ToolCoverageResponse {
    /// The tool name. Kept for wire-shape fidelity (and a future per-tool chip
    /// listing); the trust SUMMARY derives only counts, so it is unread today.
    #[allow(dead_code)]
    tool: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    doc_count: usize,
}

/// Mirror of `ClaimCrossCheckDto`: one per-claim LIVE
/// cross-check verdict. serde silently DROPS unknown wire fields, so without this
/// mirror the CLI would lose `cross_check` on deserialize (the AskResponse
/// precedent). `verdict` is `confirmed` | `contradicted` (the stale-flag) |
/// `not_found` (fail-open). Plain snake_case, no renames, so the struct match is exact.
#[derive(Deserialize)]
struct ClaimCrossCheckResponse {
    claim_text: String,
    #[serde(default)]
    verdict: String,
    #[serde(default)]
    reason: Option<String>,
    /// When the check ran (RFC3339). Deserialized for wire fidelity; the summary
    /// derives only the per-verdict counts, so this is unread today.
    #[serde(default)]
    #[allow(dead_code)]
    as_of: Option<String>,
    /// A short live-evidence excerpt. Unread by the summary (kept for fidelity).
    #[serde(default)]
    #[allow(dead_code)]
    evidence: Option<String>,
}

/// T6 (CLI parity) — mirror of the server `DispatchRateWire` (a `#[serde(tag="kind")]`
/// tagged union): the live-dispatch coverage SLI for a generation. A `rate` carries the
/// ATTEMPTED coverage ratio (0.0..=1.0); an `unavailable` carries a snake_case `cause`
/// the CLI maps to a short human label (kept in lockstep with the web
/// `dispatchRateLabel`). serde silently DROPS unknown wire fields, so this mirror is
/// load-bearing — without it the CLI would lose `dispatch_rate` on deserialize (the
/// AskResponse precedent). An unknown `kind` from a newer server → `Unknown` (serde
/// `other`), which the summary renders as a neutral "n/a" rather than a panic.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DispatchRateResponse {
    Rate { rate: f32 },
    Unavailable { cause: String },
    #[serde(other)]
    Unknown,
}

/// Mirror of `ReviewVerdict`: the post-assess review honesty
/// ledger. Same drop-on-deserialize hazard as `cross_check` above. The CLI renders
/// only the doc-level counts; the per-section ledger rides in `markdown` (the body
/// markers). serde silently ignores unknown wire fields, so the unread `emitted`
/// and `sections` are simply NOT mirrored here (no `#[allow(dead_code)]` ceremony).
#[derive(Deserialize)]
struct ReviewVerdictResponse {
    #[serde(default)]
    loud_stale_count: usize,
    #[serde(default)]
    unverified_count: usize,
    #[serde(default)]
    gap_count: usize,
}

#[derive(Deserialize)]
struct UnsupportedClaimResponse {
    claim: String,
    #[serde(default)]
    heading: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Deserialize)]
struct SkippedSourceResponse {
    label: String,
    reason: String,
}

#[derive(Deserialize)]
struct QualityReportResponse {
    /// `None` when the scorer could not run (server serializes null). Distinct from
    /// a genuine 0 — printed as "unavailable", never a misleading 0.00.
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    violations: Vec<QualityViolationResponse>,
    /// Precomputed by the server (`QualityReport::from_violations`). Drives the
    /// CLI exit code; we trust it rather than recompute from the strings.
    #[serde(default)]
    has_error_severity: bool,
}

#[derive(Deserialize)]
struct QualityViolationResponse {
    rule_name: String,
    severity: String,
    message: String,
}


// ═══════════════════════════════════════════════════════════════════════════
// Session persistence (~/.docbrain/session)
// ═══════════════════════════════════════════════════════════════════════════

fn session_file_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".docbrain").join("session"))
}

fn read_local_session() -> Option<String> {
    let path = session_file_path()?;
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn write_local_session(session_id: &str) {
    if let Some(path) = session_file_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, session_id);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Last-answer cache (~/.docbrain/last_answer.json) — Phase A per-source feedback
// ═══════════════════════════════════════════════════════════════════════════
// Caches the most recent answer's episode_id + the printed sources' stable keys
// so `docbrain thumbsdown --source N` can pin the Nth source as not-relevant.

#[derive(Serialize, Deserialize, Default)]
struct LastAnswer {
    episode_id: Option<String>,
    /// (document_id, title) per printed RAG source, in display order (1-based to
    /// the user). Empty document_id → that source isn't pinnable.
    sources: Vec<(String, String)>,
}

fn last_answer_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".docbrain").join("last_answer.json"))
}

fn write_last_answer(result: &AskResponse) {
    let Some(path) = last_answer_path() else { return };
    let la = LastAnswer {
        episode_id: result.episode_id.clone(),
        sources: result.sources.iter()
            .map(|s| (s.document_id.clone(), s.title.clone()))
            .collect(),
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&la) {
        let _ = std::fs::write(path, json);
    }
}

fn read_last_answer() -> Option<LastAnswer> {
    let path = last_answer_path()?;
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

// ═══════════════════════════════════════════════════════════════════════════
// Display helpers
// ═══════════════════════════════════════════════════════════════════════════

fn phase_label(phase: &str) -> &str {
    match phase {
        "understand" => "Understanding",
        "recall"     => "Recalling",
        "search"     => "Searching",
        "live_tools" => "Calling live tools",
        "synthesize" => "Synthesizing",
        "remember"   => "Remembering",
        "cache_hit"  => "Cache hit",
        _            => phase,
    }
}

fn print_response(result: &AskResponse, verbose: bool) {
    println!("{}", result.answer);
    print_response_metadata(result, verbose);
}

fn freshness_tag(status: Option<&str>) -> &'static str {
    match status {
        Some("stale")        => " \x1b[38;5;208m⚠ stale\x1b[0m",
        Some("outdated")     => " \x1b[31m⚠ outdated\x1b[0m",
        Some("needs_review") => " \x1b[33m⚠ needs review\x1b[0m",
        _                   => "",
    }
}

fn print_sources(result: &AskResponse) {
    if result.sources.is_empty() {
        return;
    }
    println!("
---");
    println!("Sources:");
    // 1-based numbering so `docbrain thumbsdown --source N` lines up with the
    // list the user sees. Index counts EVERY source in display order (matching
    // the last-answer cache), even visually-deduped URLs, so N stays stable.
    for (i, source) in result.sources.iter().enumerate() {
        let section = match &source.heading {
            Some(h) => format!(" > {}", h),
            None => String::new(),
        };
        let tag = freshness_tag(source.freshness_status.as_deref());
        println!("  [{}] {}{} (score: {:.2}){}", i + 1, source.title, section, source.score, tag);
        println!("      {}", source.source_url);
    }
}

fn print_verbose_ids(result: &AskResponse) {
    if let Some(ref sid) = result.session_id {
        println!("  Session: {}", sid);
    }
    if let Some(ref eid) = result.episode_id {
        println!("  Episode: {}", eid);
    }
}

fn print_response_metadata(result: &AskResponse, verbose: bool) {
    print_sources(result);
    // Phase A — cache this answer's episode + source keys for `thumbsdown --source N`.
    write_last_answer(result);

    if let Some(ref sid) = result.session_id {
        write_local_session(sid);
    }

    let turn_label = match result.turn {
        Some(n) => format!("[turn {}]", n),
        None => String::new(),
    };

    println!("
  {}  Feedback: docbrain thumbsup | docbrain thumbsdown", turn_label);

    if verbose {
        print_verbose_ids(result);
    }

    println!();
}

fn display_phase_event(event: &PipelineEvent, phase_count: &mut u32) {
    let mut stdout = std::io::stdout();
    match event {
        PipelineEvent::Started { phase, description } => {
            *phase_count += 1;
            print!("  \x1b[36m◆\x1b[0m {}... \x1b[2m{}\x1b[0m", phase_label(phase), description);
            let _ = stdout.flush();
        }
        PipelineEvent::Completed { phase, duration_ms, result_count, detail } => {
            // For live_tools, the per-tool detail ("jira_rest.search → ok") is
            // the useful payload; prefer it over a bare result count.
            let info = match (phase.as_str(), detail) {
                ("live_tools", Some(d)) if !d.is_empty() => format!(" — {}", d),
                _ => match result_count {
                    Some(n) => format!(", {} results", n),
                    None    => String::new(),
                },
            };
            print!("\r\x1b[2K  \x1b[32m✓\x1b[0m {} \x1b[2m({}ms{})\x1b[0m
",
                phase_label(phase), duration_ms, info);
            let _ = stdout.flush();
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// version
// ═══════════════════════════════════════════════════════════════════════════

async fn show_version(server_url: &str) -> Result<()> {
    println!("docbrain CLI  {}", env!("CARGO_PKG_VERSION"));
    println!("Server        {}", server_url);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    match client.get(format!("{}/api/v1/config", server_url)).send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(v) = body["version"].as_str() {
                    println!("Server version  {}", v);
                }
            }
        }
        Ok(resp) => {
            println!("Server version  (unreachable — HTTP {})", resp.status());
        }
        Err(_) => {
            println!("Server version  (unreachable — is the server running?)");
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// main
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();

    let cli = Cli::parse();

    // Resolve API key and server URL once, falling back to config file.
    let api_key = resolve_api_key(cli.api_key.as_deref());
    let server_url = resolve_server_url();

    match cli.command {
        Commands::Login { server, email, github, gitlab, oidc } => {
            if github {
                handle_oauth_login(server.as_deref(), "github").await?;
            } else if gitlab {
                handle_oauth_login(server.as_deref(), "gitlab").await?;
            } else if oidc {
                handle_oauth_login(server.as_deref(), "oidc").await?;
            } else {
                handle_login(server.as_deref(), email.as_deref()).await?;
            }
        }
        Commands::Logout => {
            handle_logout(&server_url).await?;
        }
        Commands::Token { action } => {
            let key = api_key.ok_or_else(|| anyhow::anyhow!(
                "API key required. Run `docbrain login` or set DOCBRAIN_API_KEY."
            ))?;
            handle_token(&server_url, action, &key).await?;
        }
        Commands::Ask { question, session, new, verbose } => {
            ask(&server_url, &question, session.as_deref(), new, verbose, api_key.as_deref()).await?;
        }
        Commands::TraceQuery { question, json } => {
            let key = api_key.ok_or_else(|| anyhow::anyhow!(
                "API key required. Run `docbrain login` or set DOCBRAIN_API_KEY."
            ))?;
            trace_query(&server_url, &question, json, &key).await?;
        }
        Commands::ThumbsUp { episode_id } => {
            submit_quick_feedback(&server_url, episode_id.as_deref(), 1, api_key.as_deref()).await?;
        }
        Commands::ThumbsDown { episode_id, source } => {
            match source {
                Some(n) => submit_source_feedback(&server_url, episode_id.as_deref(), n, api_key.as_deref()).await?,
                None => submit_quick_feedback(&server_url, episode_id.as_deref(), -1, api_key.as_deref()).await?,
            }
        }
        Commands::Feedback { episode_id, rating } => {
            submit_feedback(&server_url, &episode_id, &rating, api_key.as_deref()).await?;
        }
        Commands::Freshness { space } => {
            show_freshness(&server_url, space.as_deref(), api_key.as_deref()).await?;
        }
        Commands::Incident { description } => {
            let desc = match description {
                Some(d) => d,
                None => {
                    eprint!("Describe the incident: ");
                    std::io::stdout().flush().ok();
                    let mut buf = String::new();
                    std::io::stdin().read_line(&mut buf)?;
                    let trimmed = buf.trim().to_string();
                    if trimmed.is_empty() {
                        anyhow::bail!("Incident description cannot be empty");
                    }
                    trimmed
                }
            };
            ask_incident(&server_url, &desc, api_key.as_deref()).await?;
        }
        Commands::Generate {
            ask,
            sources,
            source_urls,
            stdin,
            target,
            template,
            doc_type,
            space,
            out,
            force,
            no_enrich,
            allow_violations,
            max_regen_rounds,
        } => {
            // Editor auth required (the endpoint is editor+; CI keys qualify).
            let key = api_key.ok_or_else(|| anyhow::anyhow!(
                "API key required. Run `docbrain login` or set DOCBRAIN_API_KEY."
            ))?;
            handle_generate(
                &server_url,
                &key,
                &ask,
                &sources,
                &source_urls,
                stdin,
                target.as_deref(),
                template.as_deref(),
                doc_type.as_deref(),
                space.as_deref(),
                out.as_deref(),
                force,
                no_enrich,
                allow_violations,
                max_regen_rounds,
            )
            .await?;
        }
        Commands::Analytics { days } => {
            show_analytics(&server_url, days, api_key.as_deref()).await?;
        }
        Commands::Autopilot { action } => {
            handle_autopilot(&server_url, action, api_key.as_deref()).await?;
        }
        Commands::Ci { action } => {
            let key = api_key.ok_or_else(|| anyhow::anyhow!(
                "API key required. Run `docbrain login` or set DOCBRAIN_API_KEY."
            ))?;
            handle_ci(&server_url, action, &key).await?;
        }
        Commands::Evidence { action } => {
            // Offline subcommands (verify/why/tables) take NO api key and NO
            // server URL — only `export` uses them (resolved inside the
            // handler). Verify/Why/Tables set their own process exit code via
            // `std::process::exit`, so control does not return from them.
            handle_evidence(action, &server_url, api_key.as_deref()).await?;
        }
        Commands::Admin { action } => {
            let key = api_key.ok_or_else(|| anyhow::anyhow!(
                "API key required. Run `docbrain login` or set DOCBRAIN_API_KEY."
            ))?;
            handle_admin(&server_url, action, &key).await?;
        }
        Commands::License { action } => {
            let key = api_key.ok_or_else(|| anyhow::anyhow!(
                "API key required. Run `docbrain login` or set DOCBRAIN_API_KEY."
            ))?;
            handle_license(&server_url, action, &key).await?;
        }
        Commands::Version => {
            show_version(&server_url).await?;
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Auth commands
// ═══════════════════════════════════════════════════════════════════════════

async fn handle_login(server_override: Option<&str>, email_flag: Option<&str>) -> Result<()> {
    // Server URL: --server flag → DOCBRAIN_SERVER_URL env → saved config → default
    let server_url = server_override
        .map(|s| s.to_string())
        .or_else(|| std::env::var("DOCBRAIN_SERVER_URL").ok())
        .or_else(|| read_config().server_url)
        .unwrap_or_else(|| "http://localhost:3000".to_string());

    let email = match email_flag {
        Some(e) => e.to_string(),
        None => {
            eprint!("Email: ");
            std::io::stdout().flush().ok();
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf)?;
            let trimmed = buf.trim().to_string();
            if trimmed.is_empty() {
                anyhow::bail!("Email cannot be empty");
            }
            trimmed
        }
    };

    let password = rpassword::prompt_password("Password: ")?;

    let client = reqwest::Client::new();
    let body = serde_json::json!({ "email": email, "password": password });
    let response = client
        .post(format!("{}/api/v1/auth/login", server_url))
        .json(&body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Login failed ({}): {}", status, body);
    }

    let result: LoginResponse = response.json().await?;

    // Persist server URL + session key to config file
    write_config(&Config {
        server_url: Some(server_url.clone()),
        api_key: Some(result.key),
    })?;

    println!("Logged in to {}.", server_url);
    println!("Session key saved to ~/.docbrain/config.json");
    Ok(())
}

/// OAuth login via browser — same pattern as `gh auth login` and `gcloud auth login`.
///
/// Flow:
///   1. Bind a local TCP listener on a random port.
///   2. Open the browser to the server's OAuth start URL with `?cli_port=<port>`.
///   3. User authorises in browser; server redirects to `http://127.0.0.1:<port>/?code=otp_...`.
///   4. CLI reads the OTP from the local request, POSTs it to `/api/v1/auth/exchange`.
///   5. Server returns the real API key in the response body.
///   6. CLI saves the key and closes the local listener.
async fn handle_oauth_login(server_override: Option<&str>, provider: &str) -> Result<()> {
    use tokio::net::TcpListener;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let server_url = server_override
        .map(|s| s.to_string())
        .or_else(|| std::env::var("DOCBRAIN_SERVER_URL").ok())
        .or_else(|| read_config().server_url)
        .unwrap_or_else(|| "http://localhost:3000".to_string());

    // Bind on a random OS-assigned port
    let listener = TcpListener::bind("127.0.0.1:0").await
        .map_err(|e| anyhow::anyhow!("Failed to bind local callback listener: {}", e))?;
    let port = listener.local_addr()?.port();

    // The CLI callback URL. Defaults to 127.0.0.1 but can be overridden via
    // DOCBRAIN_CLI_CALLBACK_HOST for environments where the server runs in Docker
    // and needs to redirect back to the host machine (e.g. host.docker.internal).
    let callback_host = std::env::var("DOCBRAIN_CLI_CALLBACK_HOST")
        .unwrap_or_else(|_| "127.0.0.1".to_string());
    let cli_callback = format!("http://{}:{}", callback_host, port);

    // Build the start URL for the chosen provider, passing the full callback URL
    // so the server knows exactly where to redirect after authentication.
    let encoded_callback = {
        let mut s = String::new();
        for byte in cli_callback.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~'
                | b':' | b'/' => s.push(byte as char),
                _ => s.push_str(&format!("%{:02X}", byte)),
            }
        }
        s
    };
    let start_url = match provider {
        "github" => format!("{}/api/v1/auth/github/start?cli_callback={}", server_url, encoded_callback),
        "gitlab" => format!("{}/api/v1/auth/gitlab/start?cli_callback={}", server_url, encoded_callback),
        "oidc"   => format!("{}/api/v1/auth/oidc/start?cli_callback={}", server_url, encoded_callback),
        _        => anyhow::bail!("Unknown provider: {}", provider),
    };

    println!("Opening browser to authenticate via {}...", provider);
    println!("If the browser does not open, visit: {}", start_url);
    println!();

    // Open the browser (best-effort — user can also open manually)
    let _ = open::that(&start_url);

    // Wait for the callback request (one-shot local HTTP server)
    let (mut stream, _) = listener.accept().await
        .map_err(|e| anyhow::anyhow!("Local callback listener error: {}", e))?;

    // Read the HTTP request (we only need the first line)
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).await.unwrap_or(0);
    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse the request line: "GET /?code=otp_... HTTP/1.1"
    let otp = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1)) // URL path+query
        .and_then(|path| {
            // Extract ?code=otp_... from the path
            let query = path.split('?').nth(1)?;
            query.split('&')
                .find(|p| p.starts_with("code="))
                .map(|p| p["code=".len()..].to_string())
        });

    // Send a minimal HTTP response so the browser shows a success page
    let html_response = b"HTTP/1.1 200 OK\r
Content-Type: text/html\r
Connection: close\r
\r
\
        <!DOCTYPE html><html><body><h2>DocBrain login successful!</h2>\
        <p>You can close this browser tab and return to your terminal.</p></body></html>";
    let _ = stream.write_all(html_response).await;
    drop(stream);
    drop(listener);

    let otp = otp.ok_or_else(|| anyhow::anyhow!(
        "OAuth callback did not contain a code. Please try logging in again."
    ))?;

    if !otp.starts_with("otp_") {
        anyhow::bail!("Unexpected code format in OAuth callback. Please try again.");
    }

    // Exchange the OTP for the real API key
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/api/v1/auth/exchange", server_url))
        .json(&serde_json::json!({ "code": otp }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to exchange OAuth code: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Token exchange failed ({}): {}", status, body);
    }

    let result: serde_json::Value = response.json().await
        .map_err(|e| anyhow::anyhow!("Failed to parse exchange response: {}", e))?;

    let api_key = result["api_key"].as_str()
        .ok_or_else(|| anyhow::anyhow!("No api_key in exchange response"))?
        .to_string();

    write_config(&Config {
        server_url: Some(server_url.clone()),
        api_key: Some(api_key),
    })?;

    println!("Logged in to {} via {}.", server_url, provider);
    println!("Session key saved to ~/.docbrain/config.json");

    // Inform the user if DOCBRAIN_API_KEY is set — the saved config key takes precedence,
    // but the stale env var may cause confusion in scripts or other tools.
    if std::env::var("DOCBRAIN_API_KEY").is_ok() {
        println!();
        println!("ℹ  DOCBRAIN_API_KEY is set in your environment.");
        println!("   The saved session key will be used (config file takes precedence over env).");
        println!("   To avoid confusion, run:  unset DOCBRAIN_API_KEY");
    }

    Ok(())
}

async fn handle_logout(server_url: &str) -> Result<()> {
    let cfg = read_config();
    let api_key = cfg.api_key.ok_or_else(|| {
        anyhow::anyhow!("Not logged in. Run `docbrain login` first.")
    })?;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/api/v1/auth/logout", server_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Logout failed ({}): {}", status, body);
    }

    // Clear the config (keep server_url, remove api_key)
    let server = cfg.server_url;
    write_config(&Config { server_url: server, api_key: None })?;

    println!("Logged out. Session key revoked and removed from config.");
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Token management commands
// ═══════════════════════════════════════════════════════════════════════════

async fn handle_token(server_url: &str, action: TokenAction, api_key: &str) -> Result<()> {
    match action {
        TokenAction::Create { name, role } => token_create(server_url, &name, &role, api_key).await,
        TokenAction::List => token_list(server_url, api_key).await,
        TokenAction::Revoke { id } => token_revoke(server_url, &id, api_key).await,
    }
}

async fn token_create(server_url: &str, name: &str, role: &str, api_key: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({ "name": name, "role": role });
    // Try self-service endpoint first; fall back to admin endpoint for backward compatibility
    let response = client
        .post(format!("{}/api/v1/me/tokens", server_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Error ({}): {}", status, body);
    }

    let result: serde_json::Value = response.json().await?;
    let key = result["key"].as_str().unwrap_or("");
    let id = result["id"].as_str().unwrap_or("");

    println!();
    println!("  Token created successfully!");
    println!("  Name: {}  Role: {}  ID: {}", name, role, id);
    println!();
    println!("  Key:");
    println!("  {}", key);
    println!();
    println!("  ⚠  Save this key now — it will NOT be shown again.");
    println!("  Add it to MCP config: DOCBRAIN_API_KEY={}", key);
    println!();
    Ok(())
}

async fn token_list(server_url: &str, api_key: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/api/v1/me/tokens", server_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Error ({}): {}", status, body);
    }

    let keys: Vec<serde_json::Value> = response.json().await?;

    println!();
    if keys.is_empty() {
        println!("  No active API keys.");
        println!("  Create one with: docbrain token create --name \"My Key\" --role viewer");
        println!();
        return Ok(());
    }

    println!("  Active API Keys ({} total)", keys.len());
    println!();
    println!("  {:<36} {:<24} {:<8} {:<20} Last Used",
        "ID", "Name", "Role", "Created");
    println!("  {}", "-".repeat(100));

    for key in &keys {
        let id        = key["id"].as_str().unwrap_or("-");
        let name      = key["name"].as_str().unwrap_or("-");
        let role      = key["role"].as_str().unwrap_or("-");
        let created   = key["created_at"].as_str().and_then(|s| s.get(..10)).unwrap_or("-");
        let last_used = key["last_used_at"].as_str().and_then(|s| s.get(..10)).unwrap_or("never");

        let name_display = if name.chars().count() > 22 {
            let t: String = name.chars().take(19).collect();
            format!("{}...", t)
        } else {
            name.to_string()
        };

        println!("  {:<36} {:<24} {:<8} {:<20} {}",
            id, name_display, role, created, last_used);
    }
    println!();
    Ok(())
}

async fn token_revoke(server_url: &str, id: &str, api_key: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let response = client
        .delete(format!("{}/api/v1/me/tokens/{}", server_url, id))
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Error ({}): {}", status, body);
    }

    println!("  Token {} revoked.", id);
    println!();
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Ask / Incident commands
// ═══════════════════════════════════════════════════════════════════════════

/// Admin retrieval pipeline trace command.
///
/// Sends a question to `/api/v1/ask` with `trace: true` and stream off
/// (JSON mode — trace only rides on the JSON response, not the SSE
/// stream). Receives `pipeline_trace` in the AskResponse and renders
/// it as a human-readable table or dumps the raw JSON if `--json`.
///
/// Requires admin auth — non-admin callers get no trace field back
/// and the CLI prints a clear error explaining why.
async fn trace_query(
    server_url: &str,
    question: &str,
    raw_json: bool,
    api_key: &str,
) -> Result<()> {
    let client = reqwest::Client::new();

    // Force JSON mode (stream: false) — the trace is only attached to
    // the non-streaming AskResponse.
    let body = serde_json::json!({
        "question": question,
        "stream": false,
        "trace": true,
    });

    let response = client
        .post(format!("{}/api/v1/ask", server_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Accept", "application/json")
        .header("X-DocBrain-Caller", "cli")
        .json(&body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Server error ({}): {}", status, text);
    }

    let payload: serde_json::Value = response.json().await?;

    let trace = match payload.get("pipeline_trace") {
        Some(serde_json::Value::Null) | None => {
            anyhow::bail!(
                "No pipeline_trace in response. This usually means:
  \
                 - Your API key is not an admin role (trace is admin-only)
  \
                 - The server is running an old build without Phase 3 trace support

\
                 Run `docbrain token list` (admin) or check your role."
            );
        }
        Some(t) => t.clone(),
    };

    if raw_json {
        println!("{}", serde_json::to_string_pretty(&trace)?);
        return Ok(());
    }

    // ── Human-readable table render ──────────────────────────────────
    println!();
    println!("╭─────────────────────────────────────────────────────────────────");
    if let Some(q) = trace.get("question").and_then(|v| v.as_str()) {
        println!("│ Query:            {}", q);
    }
    if let Some(qid) = trace.get("query_id").and_then(|v| v.as_str()) {
        println!("│ Query ID:         {}", qid);
    }
    if let Some(p) = trace.get("rerank_provider").and_then(|v| v.as_str()) {
        println!("│ Rerank provider:  {}", p);
    }
    if let Some(ps) = trace.get("pool_size").and_then(|v| v.as_u64()) {
        println!("│ Pool size:        {}", ps);
    }

    if let Some(subs) = trace.get("sub_queries").and_then(|v| v.as_array()) {
        if subs.len() > 1 {
            println!("│ Sub-queries:      {}", subs.len());
            for (i, s) in subs.iter().enumerate() {
                if let Some(text) = s.as_str() {
                    println!("│   [{}] {}", i, text);
                }
            }
        }
    }

    if let Some(retrievers) = trace.get("retrievers_fired").and_then(|v| v.as_array()) {
        let names: Vec<String> = retrievers
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        if !names.is_empty() {
            println!("│ Retrievers fired: {}", names.join(", "));
        }
    }

    if let Some(durations) = trace.get("stage_durations") {
        println!("├─ Stage timings ────────────────────────────────────────────────");
        for (label, field) in [
            ("query_understanding", "query_understanding"),
            ("kg_doc_retriever", "kg_doc_retriever"),
            ("candidate_generation", "candidate_generation"),
            ("rrf_fusion", "rrf_fusion"),
            ("rerank", "rerank"),
            ("freshness_pre_diversity", "freshness_pre_diversity"),
            ("diversity_select", "diversity_select"),
            ("total", "total"),
        ] {
            if let Some(ms) = durations.get(field).and_then(|v| v.as_u64()) {
                println!("│ {:<25} {} ms", label, ms);
            }
        }
    }

    // Final top-k chunks (survivors). Sort by final_rank ascending.
    let mut survivors: Vec<&serde_json::Value> = trace
        .get("chunks")
        .and_then(|v| v.as_object())
        .map(|m| m.values().collect::<Vec<_>>())
        .unwrap_or_default();
    survivors.retain(|c| c.get("final_rank").is_some() && !c.get("final_rank").unwrap().is_null());
    survivors.sort_by_key(|c| c.get("final_rank").and_then(|v| v.as_u64()).unwrap_or(u64::MAX));

    if !survivors.is_empty() {
        println!("├─ Final top-{} chunks ──────────────────────────────────────────", survivors.len());
        println!("│ rank  score   title (truncated)                                 ");
        for c in survivors {
            let rank = c.get("final_rank").and_then(|v| v.as_u64()).unwrap_or(0);
            let score = c.get("rerank_score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let title = c
                .get("title")
                .and_then(|v| v.as_str())
                .map(|s| {
                    if s.chars().count() > 55 {
                        let truncated: String = s.chars().take(52).collect();
                        format!("{}...", truncated)
                    } else {
                        s.to_string()
                    }
                })
                .unwrap_or_else(|| "<no title>".to_string());
            let doc_id = c
                .get("document_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            println!("│ {:>4}  {:>5.3}  {}", rank, score, title);
            if !doc_id.is_empty() {
                println!("│        └─ doc: {}", doc_id);
            }
        }
    } else {
        println!("├─ Final top-k ──────────────────────────────────────────────────");
        println!("│ (no chunks survived — pipeline returned empty)");
    }

    println!("╰─────────────────────────────────────────────────────────────────");
    println!();
    Ok(())
}

async fn ask(
    server_url: &str,
    question: &str,
    session_id: Option<&str>,
    new_session: bool,
    verbose: bool,
    api_key: Option<&str>,
) -> Result<()> {
    let client = reqwest::Client::new();

    println!();

    let mut body = serde_json::json!({ "question": question, "stream": true });

    if new_session {
        body["session_id"] = serde_json::Value::String("new".to_string());
    } else if let Some(sid) = session_id {
        body["session_id"] = serde_json::Value::String(sid.to_string());
    } else if let Some(local_sid) = read_local_session() {
        body["session_id"] = serde_json::Value::String(local_sid);
    }

    let mut request = client.post(format!("{}/api/v1/ask", server_url))
        .header("Accept", "text/event-stream")
        .header("X-DocBrain-Caller", "cli")
        .json(&body);

    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    let response = request.send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        if status == reqwest::StatusCode::UNAUTHORIZED {
            if std::env::var("DOCBRAIN_API_KEY").is_ok() {
                anyhow::bail!(
                    "Server error ({}): {}

  Hint: DOCBRAIN_API_KEY env var is set and may be stale.
  Run: unset DOCBRAIN_API_KEY",
                    status, body
                );
            }
            anyhow::bail!(
                "Server error ({}): {}

  Hint: Run `docbrain login` to refresh your session.",
                status, body
            );
        }
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let content_type = response.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if content_type.contains("text/event-stream") {
        handle_sse_stream(response, verbose).await?;
    } else {
        let result: AskResponse = response.json().await?;
        print_response(&result, verbose);
    }

    Ok(())
}

// The LLM appends `<!-- confidence: X.X -->` to every response so the server can
// extract a confidence score. The server strips it before storing the answer, but
// in the streaming path the marker arrives as tokens before the server can strip it.
// We hold back the last CONFIDENCE_TAIL_BUF bytes of token output so we can strip
// the marker before it reaches the terminal.
const CONFIDENCE_TAIL_BUF: usize = 40;

fn handle_sse_token(data: &str, phase_count: u32, tokens_streamed: &mut bool, tail_buf: &mut String) {
    if !*tokens_streamed {
        if phase_count > 0 {
            println!();
        }
        *tokens_streamed = true;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) &&
        let Some(text) = value["text"].as_str()
    {
        tail_buf.push_str(text);
        // Flush everything that can't possibly be part of the confidence marker.
        if tail_buf.len() > CONFIDENCE_TAIL_BUF {
            let safe_len = tail_buf.len() - CONFIDENCE_TAIL_BUF;
            // Ensure we split on a char boundary.
            let safe_len = tail_buf.floor_char_boundary(safe_len);
            print!("{}", &tail_buf[..safe_len]);
            let _ = std::io::stdout().flush();
            tail_buf.drain(..safe_len);
        }
    }
}

/// Flush the tail buffer, stripping any trailing `<!-- confidence: X.X -->` marker.
fn flush_tail_buf(tail_buf: &mut String) {
    if tail_buf.is_empty() {
        return;
    }
    // Strip the confidence marker if the LLM included it.
    let s = tail_buf.trim_end();
    let clean = if let Some(start) = s.find("<!-- confidence:") {
        s[..start].trim_end()
    } else {
        s
    };
    if !clean.is_empty() {
        print!("{}", clean);
        let _ = std::io::stdout().flush();
    }
    tail_buf.clear();
}

fn handle_sse_answer(data: &str, phase_count: u32, tokens_streamed: bool, verbose: bool) {
    if tokens_streamed {
        println!();
        if let Ok(result) = serde_json::from_str::<AskResponse>(data) {
            print_response_metadata(&result, verbose);
        }
    } else {
        if phase_count > 0 {
            println!();
        }
        if let Ok(result) = serde_json::from_str::<AskResponse>(data) {
            print_response(&result, verbose);
        }
    }
}

async fn handle_sse_stream(response: reqwest::Response, verbose: bool) -> Result<()> {
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut phase_count = 0u32;
    let mut tokens_streamed = false;
    let mut tail_buf = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find("

") {
            let event_block = buffer[..pos].to_string();
            buffer = buffer[pos + 2..].to_string();

            let (event_type, data) = parse_sse_event(&event_block);

            match event_type.as_str() {
                "phase" => {
                    if let Ok(event) = serde_json::from_str::<PipelineEvent>(&data) {
                        display_phase_event(&event, &mut phase_count);
                    }
                }
                "token" => {
                    handle_sse_token(&data, phase_count, &mut tokens_streamed, &mut tail_buf);
                }
                "answer" => {
                    flush_tail_buf(&mut tail_buf);
                    handle_sse_answer(&data, phase_count, tokens_streamed, verbose);
                }
                "error" => {
                    flush_tail_buf(&mut tail_buf);
                    eprintln!("
  Error: {}", data);
                }
                "done" => {
                    flush_tail_buf(&mut tail_buf);
                    return Ok(());
                }
                _ => {}
            }
        }
    }

    flush_tail_buf(&mut tail_buf);
    Ok(())
}

fn parse_sse_event(block: &str) -> (String, String) {
    let mut event_type = String::from("message");
    let mut data_lines = Vec::new();

    for line in block.lines() {
        if let Some(val) = line.strip_prefix("event:") {
            event_type = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("data:") {
            data_lines.push(val.trim().to_string());
        }
    }

    (event_type, data_lines.join("
"))
}

async fn ask_incident(server_url: &str, description: &str, api_key: Option<&str>) -> Result<()> {
    let client = reqwest::Client::new();

    println!();

    let body = serde_json::json!({ "description": description, "stream": true });

    let mut request = client.post(format!("{}/api/v1/incident", server_url))
        .header("Accept", "text/event-stream")
        .json(&body);

    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    let response = request.send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let content_type = response.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if content_type.contains("text/event-stream") {
        handle_sse_stream(response, false).await?;
    } else {
        let result: AskResponse = response.json().await?;
        println!("\x1b[31;1mINCIDENT RESPONSE\x1b[0m
");
        print_response(&result, false);
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Generate command (AG-T10) — ad-hoc doc generation, RETURN-only
// ═══════════════════════════════════════════════════════════════════════════

/// `docbrain generate` — the primary user/CI surface for ad-hoc doc generation.
///
/// Reads PRIMARY material from `--source` files and/or `--stdin`, optionally a
/// `--template` file, POSTs them inline to `POST /api/v1/generate` (Bearer
/// auth, mirroring `ci.rs`/`ask`), then renders the returned `GeneratedArtifact`.
///
/// # stdout is PIPE-CLEAN
///
/// stdout receives ONLY the generated markdown (or nothing, when `--out` writes
/// it to a file). EVERY diagnostic — doc type, quality score, needs-input,
/// skipped sources, violations, success notices — goes to stderr. This is what
/// makes `docbrain generate ... > doc.md` and `... | pandoc` produce a clean
/// document, and `--out doc.md` print only the summary to the terminal.
///
/// # Exit code
///
/// On error-severity quality violations the function prints the markdown +
/// summary, then `std::process::exit(2)` so CI fails the step while the operator
/// still sees the output. `--allow-violations` suppresses the non-zero exit.
/// (The HTTP request SUCCEEDED, so this is not an `Err`; it is a policy exit.)
///
/// Auto-review: when the Freshness Critic flags contradicted claims and
/// `--max-regen-rounds` > 0, this regenerates (statelessly re-submitting the
/// prior draft + feedback) until the flags clear, a round resolves nothing
/// (plateau), or the round cap is hit. It is the CLI mirror of the autopilot
/// human-in-loop; the hard round cap is the termination guarantee.

/// True when a `--target` URL is a SOURCE-ONLY link that CANNOT be augmented in
/// place — a Slack thread or a GitHub PR/file. `--target` means "this EXISTING doc,
/// augment IT"; a Confluence/Atlassian page IS augmentable, so it is a legitimate
/// target and must NEVER be flagged (augment-from-corpus, with no inline sources,
/// is the primary `--target` flow). Only Slack/GitHub urls in `--target` (no
/// reconcile path) are nudged toward `--source-url`. (The old
/// guard rejected EVERY url-target with no sources, blocking the common
/// augment-a-Confluence-page case.)
fn is_non_augmentable_source_url(s: &str) -> bool {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    let scheme_len = if lower.starts_with("https://") {
        "https://".len()
    } else if lower.starts_with("http://") {
        "http://".len()
    } else {
        return false;
    };
    let host = s[scheme_len..]
        .split(['/', ':', '?'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host == "github.com" || host == "www.github.com" || host.ends_with(".slack.com")
}

/// One `/api/v1/generate` round. POSTs `body`, surfaces a non-2xx as an Err
/// (bubbles to a non-zero exit), and parses the artifact. Extracted so the
/// auto-review loop can call it per round with a swapped body.
async fn generate_once(
    client: &reqwest::Client,
    server_url: &str,
    api_key: &str,
    body: &serde_json::Value,
) -> Result<GeneratedArtifactResponse> {
    use anyhow::Context;
    let response = match client
        .post(format!("{}/api/v1/generate", server_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("X-DocBrain-Caller", "cli")
        .json(body)
        .send()
        .await
    {
        Ok(r) => r,
        // A bounded timeout surfaces as a clear, actionable message rather than the
        // raw reqwest error — generation is long (live fetch + 2 LLM calls + critics),
        // so a timeout usually means a slow server, not a dead one.
        Err(e) if e.is_timeout() => anyhow::bail!(
            "generation timed out — the server may still be working. Retry, narrow the \
             ask, add --no-enrich, or raise DOCBRAIN_GENERATE_TIMEOUT_SECS (default 900)."
        ),
        Err(e) => return Err(anyhow::Error::new(e).context("sending generate request")),
    };
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Server error ({}): {}", status, text);
    }
    response.json().await.context("parsing generate response")
}

/// The CLI's flag-identity set for convergence. A flag's identity is its
/// SORTED `evidence_cited` (the contradicting sources). The server's autopilot
/// key also keys on the section heading, but the CLI freshness DTO drops
/// `source_span`, so the CLI uses the evidence alone — coarser, but the hard
/// round cap is the real backstop, not this comparison. "Progress" = at least one
/// prior key absent from the next round's keys.
fn regen_flag_keys(flags: &[DraftFreshnessFlagResponse]) -> std::collections::HashSet<Vec<String>> {
    flags
        .iter()
        .map(|f| {
            let mut ev = f.evidence_cited.clone();
            ev.sort();
            ev
        })
        .collect()
}

/// Build the regeneration feedback note from the flagged claims. It ASKS
/// the writer to reconcile each contradicted claim against the cited evidence.
/// It is PROMPT-ONLY server-side and can never clear a flag — the critic
/// re-derives flags from evidence. Bounded so it fits the server's feedback cap
/// (2000 bytes): cite at most the first few flags; the rest are summarized.
fn build_regen_feedback(flags: &[DraftFreshnessFlagResponse]) -> String {
    let mut s = String::from(
        "A freshness review found claims your draft makes that the retrieved \
         evidence CONTRADICTS. Revise so the document no longer asserts them \
         (correct or remove each), grounded in the evidence:\n",
    );
    // Cap the enumeration so the note stays under the server's feedback bound.
    const MAX_LISTED: usize = 5;
    for f in flags.iter().take(MAX_LISTED) {
        // Truncate each claim so a single huge claim can't blow the budget.
        let claim: String = f.claim.chars().take(160).collect();
        s.push_str(&format!("- {claim}\n"));
    }
    if flags.len() > MAX_LISTED {
        s.push_str(&format!("- …and {} more flagged claim(s).\n", flags.len() - MAX_LISTED));
    }
    // Hard cap to the server bound (2000 bytes); truncate on a char boundary.
    const MAX_FEEDBACK: usize = 2_000;
    if s.len() > MAX_FEEDBACK {
        let mut end = MAX_FEEDBACK;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

#[allow(clippy::too_many_arguments)]
async fn handle_generate(
    server_url: &str,
    api_key: &str,
    ask: &str,
    sources: &[PathBuf],
    source_urls: &[String],
    stdin: bool,
    target: Option<&str>,
    template: Option<&std::path::Path>,
    doc_type: Option<&str>,
    space: Option<&str>,
    out: Option<&std::path::Path>,
    force: bool,
    no_enrich: bool,
    allow_violations: bool,
    max_regen_rounds: u32,
) -> Result<()> {
    use anyhow::Context;

    // ── 0. --out overwrite guard (CI-safe). Refuse to clobber an existing file
    //    ONLY for an INTERACTIVE human who didn't pass --force — protects against a
    //    fat-fingered filename. A non-interactive run overwrites as before so CI
    //    pipelines that regenerate the same artifact every run keep working with no
    //    flag change. "Interactive" = stderr is a TTY AND the CI env var is unset;
    //    the CI check is belt-and-suspenders for runners that allocate a pseudo-TTY
    //    (most CI systems export CI=true). Checked BEFORE the ~90s generation so the
    //    user fails fast, not after spending the wall-clock.
    if let Some(path) = out {
        let non_interactive = !stderr_is_tty() || std::env::var_os("CI").is_some();
        if path.exists() && !force && !non_interactive {
            anyhow::bail!(
                "{} already exists. Pass --force to overwrite, or choose a different \
                 --out path. (Non-interactive/CI runs overwrite automatically.)",
                path.display()
            );
        }
    }

    // ── 1. Build the seed sources. Reading files/stdin is the CLI's job; the
    //    server takes raw content inline as { kind, label, raw }. ──
    let mut seed_sources: Vec<serde_json::Value> =
        Vec::with_capacity(sources.len() + source_urls.len() + 1);

    // URL sources: the CLI does NOT fetch (it has no MCP/orchestrator access).
    // It sends {kind:url, label:<url>, raw:""} and the SERVER fetches by id via
    // the connected connector, hard-failing if any can't be reached. The URL
    // rides in `label`.
    for url in source_urls {
        seed_sources.push(serde_json::json!({
            "kind": "url",
            "label": url,
            "raw": "",
        }));
    }

    for path in sources {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading source file {}", path.display()))?;
        // Label = the file name (fallback to the full path if there is none).
        let label = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| path.display().to_string());
        seed_sources.push(serde_json::json!({
            "kind": "file",
            "label": label,
            "raw": raw,
        }));
    }

    if stdin {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading PRIMARY material from stdin")?;
        seed_sources.push(serde_json::json!({
            "kind": "stdin",
            "label": "stdin",
            "raw": buf,
        }));
    }

    // ── 1b. Wrong-flag guard. `--target` is the EXISTING DOC to AUGMENT (the page
    //    to update), NOT source material — the server never fetches it as content.
    //    A common mistake is passing a Slack/Jira/GitHub/Confluence thread URL to
    //    --target meaning "write from this". When --target is a fetchable-source
    //    URL AND no real sources were supplied, that almost certainly happened, and
    //    the run would produce a hollow doc from nothing. Reject with a precise fix
    //    instead of generating garbage.
    if seed_sources.is_empty() {
        if let Some(t) = target {
            // Only a NON-augmentable source url (Slack/GitHub) is
            // misuse. A Confluence/Atlassian `--target` is the augment target —
            // `--target <confluence>` with no sources is the intended augment flow.
            if is_non_augmentable_source_url(t) {
                anyhow::bail!(
                    "`--target` is the EXISTING document to AUGMENT, not source \
                     material. The URL you passed ({t}) is a Slack thread or GitHub \
                     PR/file — those can't be augmented in place. Did you mean \
                     `--source-url {t}` (write a NEW doc FROM that link)? Keep \
                     --target for an existing page you want to update (e.g. a \
                     Confluence page)."
                );
            }
        }
    }

    // ── 2. Read the optional template file (raw content; the server parses it). ──
    let template_raw = match template {
        Some(path) => Some(
            std::fs::read_to_string(path)
                .with_context(|| format!("reading template file {}", path.display()))?,
        ),
        None => None,
    };

    // ── 3. Build the BASE request body, mirroring the server `GenerateRequest`
    //    shape. Optional fields are only set when present so the server's
    //    `#[serde(default)]` paths apply for the rest. The auto-review loop
    // clones this per round, swapping the seeds + adding feedback. ──
    let mut base_body = serde_json::json!({
        "ask": ask,
        "sources": seed_sources,
        "no_enrich": no_enrich,
    });
    if let Some(t) = target {
        base_body["target"] = serde_json::Value::String(t.to_string());
        // A LOCAL-FILE `--target` (e.g. ./runbook.md) augments a
        // doc on the CALLER's disk — the server can't read it. If `target` is a path
        // that exists as a file, READ it and send the content inline so the server
        // reconciles against it. A URL/doc-key target has no local file → skip (the
        // server fetches it). The wrong-flag guard above already rejected a
        // non-augmentable source url (Slack/GitHub) in the target slot.
        let tp = std::path::Path::new(t);
        if tp.is_file() {
            let content = std::fs::read_to_string(tp)
                .with_context(|| format!("read --target local file '{t}'"))?;
            base_body["target_content"] = serde_json::Value::String(content);
        }
    }
    if let Some(t) = template_raw {
        base_body["template"] = serde_json::Value::String(t);
    }
    if let Some(d) = doc_type {
        base_body["doc_type"] = serde_json::Value::String(d.to_string());
    }
    if let Some(s) = space {
        base_body["space"] = serde_json::Value::String(s.to_string());
    }

    // Bound the call so a slow/hung server surfaces a clear timeout instead of a
    // frozen terminal (generate had NO client timeout, unlike the health/publish
    // clients). 2026-06-23: default raised 180→900 and the clamp ceiling 600→900 to
    // MATCH the server's real ceiling (ingress proxy-read-timeout + ALB idle_timeout
    // are both 900). A real agentic generate — especially a `--target` augment
    // (gather → live tools → per-section enrich → merge) — legitimately takes several
    // minutes; LIVE-PROVEN. The old 180s default aborted the CLI ~5x too early, so a
    // run that the SERVER would have completed and persisted failed in the user's
    // terminal. Overridable via env for CI; floored so it can't be set uselessly low.
    let timeout_secs = std::env::var("DOCBRAIN_GENERATE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|v| v.clamp(30, 900))
        .unwrap_or(900);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    // ── 4. Round 1: generate from the user's seeds. Progress goes to stderr so
    //    stdout stays pipe-clean. The CLI makes one blocking call per round (no
    //    server-side streaming), so it honestly reports dispatch + completion
    //    rather than faking sub-steps it can't observe. ──
    // Build a breakdown of the primary inputs (files / links / stdin) for the
    // dispatch line, so every input kind is reported symmetrically.
    let mut parts: Vec<String> = Vec::new();
    if !sources.is_empty() {
        parts.push(format!("{} file{}", sources.len(), if sources.len() == 1 { "" } else { "s" }));
    }
    if !source_urls.is_empty() {
        parts.push(format!(
            "{} link{} to fetch server-side",
            source_urls.len(),
            if source_urls.len() == 1 { "" } else { "s" }
        ));
    }
    if stdin {
        parts.push("stdin".to_string());
    }
    let breakdown = if parts.is_empty() {
        "no primary sources — corpus-grounded".to_string()
    } else {
        parts.join(", ")
    };
    let mut dispatch = format!("Generating… ({breakdown})");
    if let Some(t) = target {
        dispatch.push_str(&format!("\n  augmenting: {t}"));
    }
    if !no_enrich {
        dispatch.push_str("\n  live-tool enrichment: on (--no-enrich to disable)");
    }
    eprintln!("{dispatch}");
    eprintln!("  contacting {server_url} …");
    // GEN-UX (2026-06-23): the run is a single blocking call (no server streaming),
    // so we cannot honestly show LIVE sub-steps — but we CAN tell the user what
    // happens server-side and that it takes a while, so a multi-minute wait does not
    // read as a hang. A `--target` augment (gather → live tools → per-section enrich
    // → merge) is the slowest; LIVE-PROVEN to take several minutes. This mirrors the
    // web /generate progress hint. Stays on STDERR so stdout/`--out` stay pipe-clean.
    eprintln!(
        "  server-side: gathering from your sources → searching live tools → grounding & \
         cross-checking → writing/enriching → trust review"
    );
    eprintln!("  this can take a few minutes (longer for a --target augment) — please wait …");

    let mut artifact = generate_once(&client, server_url, api_key, &base_body)
        .await
        .context("initial generation")?;
    eprintln!("  ✓ first draft received.");

    // ── 4b. auto-review loop. If the critic flagged contradicted claims
    //    and rounds remain, RE-SUBMIT the prior draft (as a stdin seed) + a
    //    feedback note that ASKS the writer to address the flags — feedback is
    //    PROMPT-ONLY server-side; it can never clear a flag, the critic
    //    re-derives flags from evidence. Convergence: stop when no PRIOR flag
    //    identity (sorted evidence_cited) cleared this round. The round cap is the
    //    HARD backstop — the loop is finite regardless of convergence.
    //
    //    The loop is STATELESS: each round is a fresh /api/v1/generate with the
    //    prior markdown riding as a seed (bounded by the server's seed budget),
    //    no server-side session. Honest-exit goes to stderr; stdout stays clean.
    let mut round: u32 = 1;
    while round <= max_regen_rounds {
        let flags = artifact
            .freshness
            .as_ref()
            .map(|f| f.flags.as_slice())
            .unwrap_or(&[]);
        if flags.is_empty() {
            break; // converged — no contradicted claims to address.
        }
        let prior_keys = regen_flag_keys(flags);

        eprintln!(
            "\n  auto-review round {}/{}: {} contradicted claim(s) flagged — regenerating…",
            round, max_regen_rounds, flags.len()
        );

        // Build this round's request: the prior draft becomes a stdin seed, plus
        // a feedback note enumerating the flagged claims. The original seeds are
        // dropped — the prior draft already incorporated them, and re-sending both
        // risks exceeding the seed budget.
        let feedback = build_regen_feedback(flags);
        let mut round_body = base_body.clone();
        round_body["sources"] = serde_json::json!([{
            "kind": "stdin",
            "label": format!("prior draft (auto-review round {round})"),
            "raw": artifact.markdown,
        }]);
        round_body["feedback"] = serde_json::Value::String(feedback);

        let next = generate_once(&client, server_url, api_key, &round_body)
            .await
            .with_context(|| format!("auto-review round {round}"))?;

        // Convergence check: did ANY previously-flagged claim clear?
        let next_keys = next
            .freshness
            .as_ref()
            .map(|f| regen_flag_keys(&f.flags))
            .unwrap_or_default();
        let resolved_any = prior_keys.iter().any(|k| !next_keys.contains(k));

        artifact = next;
        round += 1;

        if !resolved_any {
            // Plateau: this round resolved nothing previously flagged. Stop early
            // and report honestly rather than burning the remaining rounds.
            eprintln!(
                "  auto-review stopped: a round resolved no previously-flagged claim \
                 (plateau). Shipping the latest draft with its remaining flags below."
            );
            break;
        }
    }

    // ── 4c. Honest final state on stderr: if flags remain, say so loudly. The
    //    draft still ships (the operator decides) — we NEVER silently drop flags. ──
    if let Some(f) = artifact.freshness.as_ref() {
        if !f.flags.is_empty() {
            eprintln!(
                "\n  ⚠ {} contradicted claim(s) remain UNRESOLVED after auto-review:",
                f.flags.len()
            );
            for flag in &f.flags {
                eprintln!("    - {}", flag.claim);
            }
        }
    }

    // ── 4d. Support critic (B-T6) → stderr. Claims no source supports
    //    (fabrication) + spurious NEEDS-INPUT. Advisory; the doc still ships. ──
    if let Some(g) = artifact.grounding.as_ref() {
        eprint!("{}", format_grounding(g));
    }

    // ── 7. Emit the document. stdout (or --out) gets ONLY the doc, pipe-clean.
    //    On a `--target` MERGE run the MERGED doc is the artifact (unchanged
    //    sections byte-exact + additions + updates) — that is what a human pastes
    //    back / CI redirects into the canonical file, NOT the fresh-from-scratch
    //    `markdown`. The change-map glance goes to STDERR (step 7b) so stdout stays
    //    a clean, valid markdown file with NO diff markers or tags (a tag in stdout
    //    would corrupt the file CI writes — the exact "100% accuracy on the existing
    //    doc" requirement). Falls back to the fresh `markdown` for from-scratch runs. ──
    let emit_doc = artifact.merged_markdown.as_deref().unwrap_or(&artifact.markdown);
    match out {
        Some(path) => {
            std::fs::write(path, emit_doc)
                .with_context(|| format!("writing markdown to {}", path.display()))?;
            eprintln!("Wrote {} bytes of markdown to {}", emit_doc.len(), path.display());
        }
        None => {
            // Pipe-clean: the document is the ONLY thing on stdout.
            println!("{emit_doc}");
        }
    }

    // ── 7a. Merged-doc change-map glance → STDERR. Present ONLY on a
    //    `--target` merge run. stdout already carries the CLEAN merged doc (above);
    //    this is the "what would I lose" answer (count header + per-section labels),
    //    advisory text on stderr so it never pollutes the redirected file. A TTY
    //    gets ANSI colour; a pipe/CI gets a plain, greppable change-map. ──
    if let Some(manifest) = artifact.merge_manifest.as_ref() {
        let target_label = target.unwrap_or("the target doc");
        eprint!("{}", format_merge_glance(target_label, manifest));
    }

    // ── 7b. PRINT-ONLY reconcile proposal → stderr. Present ONLY when a
    //    `--target` was supplied AND the server returned a reconcile patch (target
    //    fetched + ≥1 existing-doc claim flagged). stdout stays the fresh full doc
    //    (pipe-clean, mirroring GG-3's discipline); the proposal is advisory text.
    //    Nothing is applied here — the operator re-fetches + applies (`--apply`
    //    coming soon). `target` is always Some when a patch is present (the server
    //    only reconciles when a target was sent), but we render defensively. ──
    if let Some(patch) = artifact.reconcile_patch.as_ref() {
        let target_label = target.unwrap_or("the target doc");
        eprint!(
            "{}",
            format_reconcile_proposal(target_label, artifact.reconcile_base_version, patch)
        );
    }

    // ── 7c. Shareable web link → stderr. The SERVER composed this from its own
    //    web-base config + the persisted id (the CLI never builds/hardcodes a URL);
    //    present only when the deployment set DOCBRAIN_WEB_BASE_URL and the doc
    //    persisted. Absent → nothing printed (no guessed/broken link). ──
    if let Some(url) = artifact.view_url.as_deref() {
        eprintln!("\n  View in browser: {url}");
    }

    // ── 8. Human/CI summary → stderr (never stdout). ──
    print_generate_summary(&artifact);

    // ── 9. Exit code. The request succeeded, so error-severity is a POLICY
    //    exit, not an Err: print everything first (done above), then exit 2 so
    //    CI fails the step. --allow-violations downgrades to exit 0. ──
    if artifact.quality.has_error_severity && !allow_violations {
        eprintln!(
            "\nError-severity quality violation(s) present — exiting non-zero. \
             Pass --allow-violations to override."
        );
        std::process::exit(2);
    }

    Ok(())
}

/// `1 source` vs `N sources` — picks singular/plural by count. Pure, total.
fn plural(n: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if n == 1 { singular } else { plural }
}

/// Render the honest TRUST SUMMARY lines for the generate
/// stderr block. PURE (no I/O): takes the already-derived scalars and returns the
/// multi-line block the caller writes to STDERR (stdout stays the clean doc). Kept
/// pure so the honesty invariant is exhaustively unit-testable in one place.
///
/// THE honesty invariant, encoded here:
///   - `verified: None` ⇒ "verification unavailable" — NEVER a fabricated number.
///     `None` is produced upstream when the grounding critic was ABSENT or DEGRADED
///     (a digit on a degraded pass would read as a real "0 verified = all failed").
///   - `verified: Some(n)` ⇒ "n of <checked> checkable claims verified" — `n` is the
///     conservative High-tier-supported scalar ONLY (the caller passes
///     `supported_high_count`), never low/partial/unknown.
///   - the coverage line is OMITTED when `searched_tools == 0` (no live fan-out ran):
///     "Searched 0 sources across 0 tools" is noise, not a signal.
///   - the unverified-live banner is appended when `unverified_live_only` is set.
fn render_trust_summary(
    distinct_sources: usize,
    contributed_tools: usize,
    searched_tools: usize,
    verified: Option<usize>,
    checked: usize,
    total_blocks: usize,
    needs_input: usize,
    unverified_live_only: bool,
) -> String {
    use std::fmt::Write as _;
    // `_ = needs_input`: carried in the signature for caller symmetry + future use;
    // the needs-input COUNT is already rendered by the existing "needs input (N):"
    // block, so the headline does not duplicate it.
    let _ = needs_input;
    let mut out = String::new();

    // ── Coverage line. THE moat's #1 signal: it proves fan-out breadth AND
    //    distinguishes an empty corpus from a SILENT tool failure. Three shapes:
    //      • searched == 0          → OMIT entirely (no live fan-out ran: old
    //                                 server / corpus-only generation). "Searched 0
    //                                 across 0" is noise, not a signal (break-lens ii).
    //      • contributed == 0 (but searched > 0) → the HONEST failure shape: every
    //                                 searched tool came back empty/failed. We MUST
    //                                 NOT print "across 0 tools" (reads as "no fan-out
    //                                 ran" — a lie that hides the silent failure the
    //                                 moat exists to surface). Name the search + the
    //                                 zero-contribution explicitly instead.
    //      • contributed > 0        → lead with the REAL grounding breadth (tools that
    //                                 returned evidence); append the wider searched
    //                                 count only when it exceeds the contributing set
    //                                 (a searched-but-empty/failed tool). ──
    if searched_tools == 0 {
        // no live fan-out — omit (handled by falling through to the verification line)
    } else if contributed_tools == 0 {
        // Silent-failure shape: searched, nothing contributed. Honest, not "0 tools".
        let _ = writeln!(
            out,
            "  Searched {} {} across {} {} — none contributed evidence (empty or failed)",
            distinct_sources,
            plural(distinct_sources, "source", "sources"),
            searched_tools,
            plural(searched_tools, "tool", "tools"),
        );
    } else {
        let fanout_note = if searched_tools > contributed_tools {
            format!(" (of {searched_tools} searched)")
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            "  Searched {} {} across {} {}{}",
            distinct_sources,
            plural(distinct_sources, "source", "sources"),
            contributed_tools,
            plural(contributed_tools, "tool", "tools"),
            fanout_note,
        );
    }

    // ── Verification line — THE honest headline scalar. `None` (degraded/absent
    //    critic) renders "verification unavailable", NEVER a digit: a `0` would
    //    read as a real "0 verified = everything failed", the opposite of "we
    //    could not check". `Some(n)` is the conservative High-tier-supported count
    //    ONLY (the caller passes `supported_high_count`). ──
    match verified {
        Some(n) => {
            // Show the full doc surface only when there is an unchecked remainder,
            // so the tail never overstates coverage.
            if total_blocks > checked {
                let _ = writeln!(
                    out,
                    "  Trust: {} of {} checkable {} verified (doc has ~{} assertions)",
                    n,
                    checked,
                    plural(checked, "claim", "claims"),
                    total_blocks,
                );
            } else {
                let _ = writeln!(
                    out,
                    "  Trust: {} of {} checkable {} verified",
                    n,
                    checked,
                    plural(checked, "claim", "claims"),
                );
            }
        }
        None => {
            let _ = writeln!(
                out,
                "  Trust: verification unavailable (grounding critic absent or degraded)",
            );
        }
    }

    // ── Unverified-live banner — appended when the doc is grounded ONLY in
    //    live/low-trust data (useful, NOT authoritative). ──
    if unverified_live_only {
        let _ = writeln!(
            out,
            "  ⚠ UNVERIFIED (live-only): grounded in fresh live data, not durable \
             authoritative sources — useful, verify before relying on it.",
        );
    }

    out
}

/// Render the human/CI-readable generation summary to STDERR.
///
/// Kept separate from [`handle_generate`] so the stdout-cleanliness invariant is
/// auditable in one place: every write here is `eprintln!`. Nothing in this
/// function may touch stdout.
fn print_generate_summary(artifact: &GeneratedArtifactResponse) {
    eprintln!();
    eprintln!("  doc type:       {}", artifact.doc_type);

    // ── The honest TRUST HEADLINE, FIRST so it is the
    //    10-second read. Derived PURELY from the wire fields:
    //      contributed_tools = coverage rows with a proven "hit" + docs;
    //      searched_tools     = every eligible tool the fan-out touched;
    //      verified           = supported_high_count, but ONLY when the critic
    //                           ran AND was not degraded — else None
    //                           ("verification unavailable", never a number).
    //    Goes to STDERR via render_trust_summary's caller (eprint! below); stdout
    //    stays the clean doc. ──
    let contributed_tools = artifact
        .coverage
        .iter()
        .filter(|c| c.status == "hit" && c.doc_count > 0)
        .count();
    let searched_tools = artifact.coverage.len();
    let (verified, checked, total_blocks) = match artifact.grounding.as_ref() {
        // Degraded FIRST: a down dependency means no trustworthy number, even if
        // some claims were judged. Never read the count on this branch.
        Some(g) if !g.degraded.is_empty() => (None, g.checked_count, g.total_blocks),
        // Fully-judged, non-degraded pass: the conservative High-tier scalar ONLY.
        // `supported_high_count` is `Option` — `None` means the field was ABSENT (an
        // old/persisted server predating the tier-split), which routes to the
        // "unavailable" rendering rather than a false "0 of N verified". `Some(0)` is
        // a real fully-judged zero and passes through as a genuine count.
        Some(g) => (g.supported_high_count, g.checked_count, g.total_blocks),
        // Absent critic: nothing ran. Unavailable, never 0.
        None => (None, 0, 0),
    };
    eprint!(
        "{}",
        render_trust_summary(
            artifact.distinct_sources,
            contributed_tools,
            searched_tools,
            verified,
            checked,
            total_blocks,
            artifact.needs_input.len(),
            artifact.unverified_live_only,
        )
    );

    // The style/structure score is LINT (section completeness, formatting), NOT a
    // trust signal — relabeled so it does not read as one next to the headline above.
    match artifact.quality.score {
        Some(s) => eprintln!("  style/structure score:  {:.2} (lint, not trust)", s),
        None => eprintln!("  style/structure score:  unavailable (scorer did not run)"),
    }

    if !artifact.needs_input.is_empty() {
        eprintln!("  needs input ({}):", artifact.needs_input.len());
        for item in &artifact.needs_input {
            eprintln!("    - {}", item);
        }
    }

    if !artifact.skipped_sources.is_empty() {
        eprintln!("  skipped sources ({}):", artifact.skipped_sources.len());
        for s in &artifact.skipped_sources {
            eprintln!("    - {}: {}", s.label, s.reason);
        }
    }

    if artifact.quality.violations.is_empty() {
        eprintln!("  violations:     none");
    } else {
        eprintln!("  violations ({}):", artifact.quality.violations.len());
        for v in &artifact.quality.violations {
            eprintln!("    [{}] {} — {}", v.severity, v.rule_name, v.message);
        }
    }

    // FC-T7 — freshness review: claims the evidence shows are stale. Only present
    // when the critic ran AND flagged something (conservative-KEEP). Diagnostics
    // go to stderr like everything else, so the redirected markdown stays clean.
    if let Some(fr) = &artifact.freshness {
        if !fr.flags.is_empty() {
            eprintln!(
                "  freshness:      {} flagged of {} checked (evidence-contradicted):",
                fr.flags.len(),
                fr.checked_count
            );
            for f in &fr.flags {
                eprintln!("    - {}", f.claim);
                if let Some(note) = &f.note {
                    eprintln!("      why: {note}");
                }
                if !f.evidence_cited.is_empty() {
                    eprintln!("      contradicted by: {}", f.evidence_cited.join(", "));
                }
            }
            if fr.unchecked_count > 0 {
                eprintln!("    ({} claim(s) not checked — over the per-draft limit)", fr.unchecked_count);
            }
        }
    }

    // The LIVE cross-check: a one-line confirmed/stale/not-verified tally.
    // Only when the cross-check ran (a non-empty Vec). The doc body already carries
    // the inline stale / not-verified markers; this is the at-a-glance count. Mirrors
    // the freshness block (stderr only; the redirected markdown stays clean).
    if !artifact.cross_check.is_empty() {
        let confirmed = artifact.cross_check.iter().filter(|c| c.verdict == "confirmed").count();
        let stale = artifact.cross_check.iter().filter(|c| c.verdict == "contradicted").count();
        let not_verified = artifact.cross_check.len() - confirmed - stale;
        eprintln!(
            "  cross-check:    {confirmed} confirmed, {stale} stale, {not_verified} not-verified (of {} claim(s))",
            artifact.cross_check.len()
        );
        // Surface the stale claims explicitly — the load-bearing "what + why".
        for c in artifact.cross_check.iter().filter(|c| c.verdict == "contradicted") {
            eprintln!("    - STALE: {}", c.claim_text);
            if let Some(reason) = &c.reason {
                eprintln!("      live source says: {reason}");
            }
        }
    }

    // T6 (CLI parity) — the live-dispatch coverage SLI: the server's authoritative
    // verdict for HOW MANY verifiable claims were actually dispatched to a live tool
    // (a `rate`), or an honest `cause` when no rate applies. Printed whenever the
    // server sent the field (a current server always does) — it carries its own "n/a —
    // <why>" so it is never noise. The cause labels mirror the web `dispatchRateLabel`
    // verbatim so generate, History, and the web read the SAME phrasing. None = old
    // server (field absent) → print nothing (no fabricated "n/a").
    if let Some(dr) = &artifact.dispatch_rate {
        let label = match dr {
            DispatchRateResponse::Rate { rate } => {
                format!("{}% of verifiable claims dispatched live", (rate * 100.0).round() as i64)
            }
            DispatchRateResponse::Unavailable { cause } => match cause.as_str() {
                "no_routable_claims" => "n/a — no live-verifiable claims in this doc".to_string(),
                "no_changed_claims" => {
                    "n/a — no changed claims to verify (merge made no changes)".to_string()
                }
                "cross_check_disabled" => "n/a — live cross-check was disabled".to_string(),
                "no_orchestrator" => "n/a — live tools unavailable".to_string(),
                // A `cause` from a newer server we do not yet map: honest neutral, never
                // a panic or a misleading mapping to an existing label.
                _ => "n/a".to_string(),
            },
            // An unknown `kind` from a newer server (serde `other`): neutral n/a.
            DispatchRateResponse::Unknown => "n/a".to_string(),
        };
        eprintln!("  live-dispatch:  {label}");
    }

    // The review honesty ledger: how many sections shipped flagged. Only
    // when the review gate ran AND flagged something loud (a clean review adds no
    // noise). The body markers are the per-section detail; this is the doc-level count.
    if let Some(rv) = &artifact.review {
        if rv.loud_stale_count > 0 || rv.unverified_count > 0 || rv.gap_count > 0 {
            eprintln!(
                "  review:         {} loud-stale, {} not-verified, {} needs-input section(s) (markers in the document)",
                rv.loud_stale_count, rv.unverified_count, rv.gap_count
            );
        }
    }

    eprintln!();
}

/// Render the PRINT-ONLY reconcile proposal as a human-readable diff.
///
/// PURE (no I/O): takes the target label, the base page version (at fetch time),
/// and the patch; returns the multi-line block the caller writes to STDERR. Kept
/// pure so it is exhaustively unit-testable and so the stdout-cleanliness
/// invariant (the markdown is the ONLY thing on stdout) is auditable — this fn
/// never touches stdout.
///
/// Shape:
///   - A header naming the target, the base version, and the counts.
///   - One line per OP to replace: the section locator (level + 1-based ordinal)
///     and the contradicted claim that triggered it (`reason`). The op does not
///     carry the heading TEXT (it lives in the storage anchor, an apply-time
///     detail), so we show the structural locator + the claim, which is the
///     load-bearing "what + why".
///   - One line per SKIPPED op: the heading (when known) and the reason, marked
///     "routed to human".
///   - A closing note that this is a PROPOSAL only — re-fetch + apply manually,
///     `--apply` coming soon.
///
/// Never panics on an empty patch (it cannot be reached with one — the server
/// returns `None` for an all-empty patch — but the fn stays total regardless).
/// ANSI colors for the stderr review output — ON only when stderr is a real
/// terminal (a human is watching). When stderr is redirected/piped, `color()` is
/// a no-op so a captured log never gets `\033[..` escape garbage. Zero deps:
/// `std::io::IsTerminal` (Rust 1.70+) + raw SGR codes. This is the "redirect
/// catch" — colors to a TTY, plain to a file.
fn stderr_is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stderr().is_terminal()
}

/// Wrap `s` in an SGR color when stderr is a TTY; otherwise return it plain.
/// `code` is the SGR number (31=red, 32=green, 90=grey).
fn color(s: &str, code: u8, tty: bool) -> String {
    if tty {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

fn format_reconcile_proposal(
    target: &str,
    base_version: Option<i64>,
    patch: &ReconcilePatchResponse,
) -> String {
    use std::fmt::Write as _;
    let tty = stderr_is_tty();
    let mut out = String::new();
    let n = patch.ops.len();
    let m = patch.skipped_ops.len();
    let version = base_version
        .map(|v| v.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let _ = writeln!(
        out,
        "\n  Reconcile proposal for {target} (base version {version}): \
         {n} section(s) to replace, {m} routed to human. \
         PRINT-ONLY — re-fetch + apply manually (--apply coming soon)."
    );

    if !patch.ops.is_empty() {
        let _ = writeln!(out, "  sections to replace ({n}):");
        for op in &patch.ops {
            // 1-based ordinal for human display.
            let _ = writeln!(
                out,
                "    §h{} #{}: {}",
                op.heading_level,
                op.heading_ordinal + 1,
                op.reason
            );
            // Colored before→after line diff (red `-` old, green `+` new) — the
            // "show me what you'd change" view. We render the READABLE MARKDOWN
            // forms (`## FedRAMP`, `1. Shell into…`) so the diff is prose lines,
            // not the Confluence storage XHTML wall (one giant `<ac:…>` line).
            // The SPLICE still uses the storage bytes server-side — this is display
            // only. Fall back to storage if an older server omitted the markdown
            // (the serde-default leaves them empty).
            let before = if op.anchor_markdown.trim().is_empty() {
                op.anchor_storage.as_str()
            } else {
                op.anchor_markdown.as_str()
            };
            let after = if op.new_section_markdown.trim().is_empty() {
                op.new_section_storage.as_str()
            } else {
                op.new_section_markdown.as_str()
            };
            for line in before.lines() {
                let _ = writeln!(out, "      {}", color(&format!("- {line}"), 31, tty));
            }
            for line in after.lines() {
                let _ = writeln!(out, "      {}", color(&format!("+ {line}"), 32, tty));
            }
        }
    }

    if !patch.skipped_ops.is_empty() {
        let _ = writeln!(out, "  routed to human ({m}):");
        for sk in &patch.skipped_ops {
            let heading = sk.heading.as_deref().unwrap_or("(unlocated section)");
            let _ = writeln!(out, "    - {heading}: {}", sk.reason);
        }
    }

    out
}

/// Strip leading markdown hashes + whitespace from a heading line for a clean
/// label; a `None`/empty heading (the preamble) reads as "(preamble)".
fn merge_heading_label(heading: Option<&str>) -> String {
    match heading {
        Some(h) => {
            let stripped = h.trim_start_matches('#').trim();
            if stripped.is_empty() { "(preamble)".to_string() } else { stripped.to_string() }
        }
        None => "(preamble)".to_string(),
    }
}

/// Render the merged-doc change-map to STDERR (stdout already carries the
/// CLEAN merged doc). This is the "what would I lose by copy-pasting" answer in
/// under a second:
///   - a COUNT HEADER (N updated · M new · K unchanged · [S needs-review] · T total),
///     where unchanged + updated + skipped == T accounts for every existing
///     section (nothing hidden); plus
///   - per-section lines in document order, each prefixed with a STABLE
///     `reconcile:<status>` token so a CI step can `grep 'reconcile:'` the stderr
///     and gate on the change-map. On a TTY the status is colored; redirected it is
///     plain (the grep token survives either way). The diff bodies are NOT echoed
///     here — stdout's merged doc already carries the applied text; this is the
///     glance, not the diff (the web shows the word-level red/green).
fn format_merge_glance(target: &str, manifest: &MergeManifestResponse) -> String {
    use std::fmt::Write as _;
    let tty = stderr_is_tty();
    let mut out = String::new();

    // Count header. The pieces are colored on a TTY; the `existing_total`
    // denominator is the exhaustiveness proof.
    let updated = color(&format!("{} updated", manifest.updated_count), 33, tty);
    let new = color(&format!("{} new", manifest.new_count), 32, tty);
    let unchanged = color(&format!("{} unchanged", manifest.unchanged_count), 90, tty);
    let _ = writeln!(
        out,
        "\n  Merged doc for {target} — copy stdout over the existing page; nothing below is lost."
    );
    if manifest.skipped_count > 0 {
        let needs = color(&format!("{} needs review", manifest.skipped_count), 31, tty);
        let _ = writeln!(
            out,
            "  change-map: {updated} · {new} · {unchanged} · {needs} · {} existing section(s).",
            manifest.existing_total
        );
    } else {
        let _ = writeln!(
            out,
            "  change-map: {updated} · {new} · {unchanged} · {} existing section(s).",
            manifest.existing_total
        );
    }

    // Per-section glance, document order. Each line carries a stable
    // `reconcile:<status>` token (greppable by CI) followed by the heading.
    for op in &manifest.ops {
        match op {
            MergeOpResponse::Unchanged { heading, .. } => {
                let label = merge_heading_label(heading.as_deref());
                let _ = writeln!(out, "    {} {label}", color("reconcile:unchanged", 90, tty));
            }
            MergeOpResponse::Updated { heading, .. } => {
                let label = merge_heading_label(heading.as_deref());
                let _ = writeln!(out, "    {} {label}", color("reconcile:updated", 33, tty));
            }
            MergeOpResponse::New { heading, .. } => {
                let label = merge_heading_label(heading.as_deref());
                let _ = writeln!(out, "    {} {label}", color("reconcile:new", 32, tty));
            }
            MergeOpResponse::Skipped { heading, reason } => {
                let label = merge_heading_label(heading.as_deref());
                let _ = writeln!(
                    out,
                    "    {} {label} — left unchanged: {reason}",
                    color("reconcile:needs-review", 31, tty)
                );
            }
        }
    }

    out
}

/// B-T6 — render the support critic's grounding verdict to stderr: the claims no
/// source supports (fabrication) and the spurious NEEDS-INPUT (gaps the sources
/// actually cover). Advisory — the doc still shipped; this just surfaces what to
/// double-check. Colored on a TTY (amber/grey), plain when redirected.
fn format_grounding(g: &GroundingResponse) -> String {
    use std::fmt::Write as _;
    let tty = stderr_is_tty();
    let mut out = String::new();
    if !g.unsupported_claims.is_empty() {
        let header = format!(
            "\n  ⚠ {} claim(s) could not be grounded — marked NEEDS INPUT in the document (no source backs them; not stated as fact):",
            g.unsupported_claims.len()
        );
        let _ = writeln!(out, "{}", color(&header, 33, tty));
        for c in &g.unsupported_claims {
            let loc = c.heading.as_deref().map(|h| format!(" [{h}]")).unwrap_or_default();
            let _ = writeln!(out, "    - {}{loc}", c.claim);
            if let Some(note) = c.note.as_deref() {
                let _ = writeln!(out, "      {}", color(note, 90, tty));
            }
        }
    }
    if !g.preexisting_unverified.is_empty() {
        let header = format!(
            "\n  {} pre-existing claim(s) in your document are unverified — left untouched (your content, not this update's; you may want to review):",
            g.preexisting_unverified.len()
        );
        let _ = writeln!(out, "{}", color(&header, 90, tty));
        for c in &g.preexisting_unverified {
            let loc = c.heading.as_deref().map(|h| format!(" [{h}]")).unwrap_or_default();
            let _ = writeln!(out, "    - {}{loc}", c.claim);
        }
    }
    if !g.spurious_needs_input.is_empty() {
        let _ = writeln!(
            out,
            "  {} NEEDS-INPUT marker(s) flag a gap the sources DO cover (review — may be over-cautious):",
            g.spurious_needs_input.len()
        );
        for topic in &g.spurious_needs_input {
            let _ = writeln!(out, "    - {topic}");
        }
    }
    if !g.unsupported_claims.is_empty()
        || !g.preexisting_unverified.is_empty()
        || !g.spurious_needs_input.is_empty()
    {
        let _ = writeln!(
            out,
            "  ({} of {} claims grounded)",
            g.supported_count, g.checked_count
        );
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Feedback commands
// ═══════════════════════════════════════════════════════════════════════════

async fn submit_quick_feedback(
    server_url: &str,
    episode_id: Option<&str>,
    feedback: i16,
    api_key: Option<&str>,
) -> Result<()> {
    let client = reqwest::Client::new();

    let (url, body) = match episode_id {
        Some(eid) => (
            format!("{}/api/v1/feedback", server_url),
            serde_json::json!({ "episode_id": eid, "feedback": feedback }),
        ),
        None => (
            format!("{}/api/v1/feedback/latest", server_url),
            serde_json::json!({ "feedback": feedback }),
        ),
    };

    let mut request = client.post(&url).json(&body);

    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    let response = request.send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let _result: FeedbackResponse = response.json().await?;

    let emoji = if feedback == 1 { "👍" } else { "👎" };
    println!("{} Feedback recorded.", emoji);

    Ok(())
}

/// Phase A — mark the Nth source (1-based, from the last printed answer) as not
/// relevant. Resolves the source's stable document_id from the local last-answer
/// cache and sends it as `source_ref`.
async fn submit_source_feedback(
    server_url: &str,
    episode_id_override: Option<&str>,
    source_n: usize,
    api_key: Option<&str>,
) -> Result<()> {
    let last = read_last_answer().ok_or_else(|| anyhow::anyhow!(
        "No recent answer cached. Ask a question first, then `thumbsdown --source N`."
    ))?;
    if source_n == 0 || source_n > last.sources.len() {
        anyhow::bail!(
            "Source {} out of range — the last answer had {} source(s).",
            source_n, last.sources.len()
        );
    }
    let (doc_id, title) = &last.sources[source_n - 1];
    if doc_id.is_empty() {
        anyhow::bail!("Source {} ({}) has no stable id and can't be pinned.", source_n, title);
    }
    let episode_id = episode_id_override
        .map(str::to_string)
        .or(last.episode_id)
        .ok_or_else(|| anyhow::anyhow!("No episode id available for the last answer."))?;

    let body = serde_json::json!({
        "episode_id": episode_id,
        "feedback": -1,
        "reason": "not_relevant",
        "source_ref": { "kind": "rag", "key": doc_id, "cached_source_index": source_n - 1 },
    });
    let client = reqwest::Client::new();
    let mut request = client.post(format!("{}/api/v1/feedback", server_url)).json(&body);
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, body);
    }
    let _: FeedbackResponse = response.json().await?;
    println!("👎 Marked source {} (\"{}\") not relevant.", source_n, title);
    Ok(())
}

async fn submit_feedback(server_url: &str, episode_id: &str, rating: &str, api_key: Option<&str>) -> Result<()> {
    let feedback: i16 = match rating {
        "up" | "1" | "thumbsup"     =>  1,
        "down" | "-1" | "thumbsdown" => -1,
        _ => anyhow::bail!("Rating must be 'up' or 'down'"),
    };

    let client = reqwest::Client::new();
    let body = serde_json::json!({ "episode_id": episode_id, "feedback": feedback });

    let mut request = client.post(format!("{}/api/v1/feedback", server_url)).json(&body);

    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    let response = request.send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let result: FeedbackResponse = response.json().await?;
    println!("Feedback submitted (status: {}).", result.status);

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Freshness
// ═══════════════════════════════════════════════════════════════════════════

async fn show_freshness(server_url: &str, space: Option<&str>, api_key: Option<&str>) -> Result<()> {
    let client = reqwest::Client::new();

    let mut url = format!("{}/api/v1/freshness", server_url);
    if let Some(s) = space {
        url = format!("{}?space={}", url, s);
    }

    let mut request = client.get(&url);
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let report: FreshnessReportResponse = response.json().await?;
    let summary = &report.summary;

    let scope = report.space.as_deref().unwrap_or("All spaces");
    println!();
    println!("  Freshness Report: {}", scope);
    println!("  {} docs | avg {:.1} | {} fresh | {} review | {} stale | {} outdated",
        summary.total_docs, summary.avg_score, summary.fresh, summary.review, summary.stale, summary.outdated);
    println!();

    if report.documents.is_empty() {
        println!("  No scored documents yet. Run a consolidation pass first.");
        return Ok(());
    }

    println!("  {:<6} {:<9} {:<50} {:<12} URL",
        "Score", "Status", "Title", "Last Edited");
    println!("  {}", "-".repeat(110));

    for doc in &report.documents {
        let status_colored = match doc.status.as_str() {
            "fresh"        => format!("\x1b[32m{:<12}\x1b[0m", doc.status),
            "needs_review" => format!("\x1b[33m{:<12}\x1b[0m", doc.status),
            "stale"        => format!("\x1b[38;5;208m{:<12}\x1b[0m", doc.status),
            "outdated"     => format!("\x1b[31m{:<12}\x1b[0m", doc.status),
            _              => format!("{:<12}", doc.status),
        };

        let edited = doc.last_edited_at.as_deref()
            .and_then(|s| s.get(..10))
            .unwrap_or("unknown");

        let title = if doc.title.chars().count() > 48 {
            let truncated: String = doc.title.chars().take(45).collect();
            format!("{}...", truncated)
        } else {
            doc.title.clone()
        };

        println!("  {:<6.1} {} {:<50} {:<12} {}",
            doc.total_score, status_colored, title, edited, doc.source_url);
    }

    println!();
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Analytics
// ═══════════════════════════════════════════════════════════════════════════

fn feedback_indicator(avg_feedback: f32) -> &'static str {
    if avg_feedback > 0.0 { "\x1b[32m+\x1b[0m" }
    else if avg_feedback < 0.0 { "\x1b[31m-\x1b[0m" }
    else { " " }
}

fn truncate_display(text: &str, max_chars: usize) -> String {
    if text.chars().count() > max_chars {
        let t: String = text.chars().take(max_chars - 3).collect();
        format!("{}...", t)
    } else {
        text.to_string()
    }
}

fn print_top_queries(queries: &[TopQueryResponse]) {
    if queries.is_empty() {
        return;
    }
    println!("  Top Queries:");
    for (i, q) in queries.iter().take(10).enumerate() {
        let fb = feedback_indicator(q.avg_feedback);
        let display = truncate_display(&q.query_text, 60);
        println!("  {}. {} [{}x] {}", i + 1, fb, q.count, display);
    }
    println!();
}

fn print_doc_gaps(gaps: &[DocGapResponse]) {
    if gaps.is_empty() {
        return;
    }
    println!("  \x1b[33mDoc Gaps (unanswered/negative feedback):\x1b[0m");
    for (i, gap) in gaps.iter().take(10).enumerate() {
        let display = truncate_display(&gap.label, 60);
        println!("  {}. [{}x] {}", i + 1, gap.occurrence_count, display);
    }
    println!();
}

fn print_retrieved_docs(docs: &[RetrievedDocResponse]) {
    if docs.is_empty() {
        return;
    }
    println!("  Most Referenced Docs:");
    for (i, doc) in docs.iter().take(10).enumerate() {
        let fb = feedback_indicator(doc.avg_feedback);
        let display = truncate_display(&doc.title, 45);
        println!("  {}. {} [{}x] {}", i + 1, fb, doc.retrieval_count, display);
    }
    println!();
}

async fn show_analytics(server_url: &str, days: i64, api_key: Option<&str>) -> Result<()> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/v1/analytics?days={}", server_url, days);

    let mut request = client.get(&url);
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let report: AnalyticsResponse = response.json().await?;

    println!();
    println!("  Analytics Report (last {} days)", days);
    println!("  {} queries | {} users | {:.0}% positive feedback",
        report.total_queries, report.unique_users, report.positive_feedback_pct);
    println!();

    print_top_queries(&report.top_queries);

    if !report.top_intents.is_empty() {
        let intent_parts: Vec<String> = report.top_intents.iter()
            .map(|i| format!("{}: {}", i.intent, i.count))
            .collect();
        println!("  Intents: {}", intent_parts.join(" | "));
        println!();
    }

    print_doc_gaps(&report.doc_gaps);
    print_retrieved_docs(&report.most_retrieved_docs);

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Autopilot commands
// ═══════════════════════════════════════════════════════════════════════════

async fn handle_autopilot(server_url: &str, action: AutopilotAction, api_key: Option<&str>) -> Result<()> {
    match action {
        AutopilotAction::Summary => autopilot_summary(server_url, api_key).await,
        AutopilotAction::Gaps { limit } => autopilot_gaps(server_url, limit, api_key).await,
        AutopilotAction::Analyze => autopilot_analyze(server_url, api_key).await,
        AutopilotAction::Drafts { status } => autopilot_drafts(server_url, status.as_deref(), api_key).await,
        AutopilotAction::Generate { cluster_id } => autopilot_generate(server_url, &cluster_id, api_key).await,
        AutopilotAction::Dismiss { cluster_id } => autopilot_dismiss(server_url, &cluster_id, api_key).await,
        AutopilotAction::Publish { draft_id, target } => autopilot_publish(server_url, &draft_id, target.as_deref(), api_key).await,
    }
}

async fn autopilot_summary(server_url: &str, api_key: Option<&str>) -> Result<()> {
    let client = reqwest::Client::new();
    let mut request = client.get(format!("{}/api/v1/autopilot/summary", server_url));
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let summary: AutopilotSummaryResponse = response.json().await?;

    println!();
    println!("  Autopilot Summary");
    println!("  ─────────────────────────────────────");
    println!("  Total gap clusters:  {}", summary.total_gaps);
    println!("  Open gaps:           {}", summary.open_gaps);
    println!("  Critical gaps:       \x1b[31m{}\x1b[0m", summary.critical_gaps);
    println!("  Drafts generated:    {}", summary.drafts_generated);
    println!("  Drafts published:    \x1b[32m{}\x1b[0m", summary.drafts_published);
    if let Some(ref ts) = summary.last_analysis_at {
        println!("  Last analysis:       {}", ts.get(..19).unwrap_or(ts));
    }
    println!();

    Ok(())
}

async fn autopilot_gaps(server_url: &str, limit: i64, api_key: Option<&str>) -> Result<()> {
    let client = reqwest::Client::new();
    let mut request = client.get(format!("{}/api/v1/autopilot/gaps?limit={}", server_url, limit));
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let gaps: Vec<GapClusterResponse> = response.json().await?;

    println!();
    if gaps.is_empty() {
        println!("  No documentation gaps detected. Autopilot needs usage data (ask questions, give feedback).");
        println!();
        return Ok(());
    }

    println!("  Documentation Gaps ({} clusters)", gaps.len());
    println!();

    for (i, gap) in gaps.iter().enumerate() {
        let severity_colored = match gap.severity.as_str() {
            "critical" => format!("\x1b[31m{}\x1b[0m", gap.severity.to_uppercase()),
            "high"     => format!("\x1b[38;5;208m{}\x1b[0m", gap.severity.to_uppercase()),
            "medium"   => format!("\x1b[33m{}\x1b[0m", gap.severity.to_uppercase()),
            _          => gap.severity.to_uppercase(),
        };

        println!("  {}. [{}] {} ({} queries, avg confidence: {:.2})",
            i + 1, severity_colored, gap.label, gap.query_count, gap.avg_confidence);
        println!("     {}", gap.description);

        if !gap.sample_queries.is_empty() {
            let samples: Vec<String> = gap.sample_queries.iter().take(3)
                .map(|q| format!("\"{}\"", q))
                .collect();
            println!("     Samples: {}", samples.join(", "));
        }
        println!("     ID: {}  |  docbrain autopilot generate {}", gap.id, gap.id);
        println!();
    }

    Ok(())
}

async fn autopilot_analyze(server_url: &str, api_key: Option<&str>) -> Result<()> {
    let client = reqwest::Client::new();
    let mut request = client.post(format!("{}/api/v1/autopilot/analyze", server_url));
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    println!("  Running gap analysis...");

    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let result: serde_json::Value = response.json().await?;
    let count = result["new_clusters"].as_i64().unwrap_or(0);

    println!("  Done. {} new gap clusters created.", count);
    if count > 0 {
        println!("  Run `docbrain autopilot gaps` to view them.");
    }
    println!();

    Ok(())
}

async fn autopilot_drafts(server_url: &str, status: Option<&str>, api_key: Option<&str>) -> Result<()> {
    let client = reqwest::Client::new();
    let mut url = format!("{}/api/v1/autopilot/drafts", server_url);
    if let Some(s) = status {
        url = format!("{}?status={}", url, s);
    }

    let mut request = client.get(&url);
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let drafts: Vec<DraftResponse> = response.json().await?;

    println!();
    if drafts.is_empty() {
        println!("  No drafts yet. Generate one with: docbrain autopilot generate <cluster-id>");
        println!();
        return Ok(());
    }

    println!("  Auto-Generated Drafts ({} total)", drafts.len());
    println!();

    for (i, draft) in drafts.iter().enumerate() {
        let status_colored = match draft.status.as_str() {
            "pending_review" => format!("\x1b[33m{}\x1b[0m", draft.status),
            "approved"       => format!("\x1b[36m{}\x1b[0m", draft.status),
            "published"      => format!("\x1b[32m{}\x1b[0m", draft.status),
            "rejected"       => format!("\x1b[31m{}\x1b[0m", draft.status),
            _                => draft.status.clone(),
        };

        println!("  {}. {} [{}] (type: {}, quality: {:.2})",
            i + 1, draft.title, status_colored, draft.content_type, draft.quality_score.unwrap_or(0.0));
        println!("     ID: {}", draft.id);

        let preview: String = draft.content.lines().take(2)
            .map(|l| {
                if l.chars().count() > 80 {
                    let t: String = l.chars().take(77).collect();
                    format!("{}...", t)
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("
     ");
        if !preview.is_empty() {
            println!("     {}", preview);
        }
        println!();
    }

    Ok(())
}

async fn autopilot_generate(server_url: &str, cluster_id: &str, api_key: Option<&str>) -> Result<()> {
    let client = reqwest::Client::new();
    let mut request = client.post(format!("{}/api/v1/autopilot/generate/{}", server_url, cluster_id));
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    println!("  Generating draft for cluster {}...", &cluster_id[..8.min(cluster_id.len())]);

    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let result: serde_json::Value = response.json().await?;
    let title        = result["title"].as_str().unwrap_or("Untitled");
    let content_type = result["content_type"].as_str().unwrap_or("unknown");
    let quality      = result["quality_score"].as_f64().unwrap_or(0.0);

    println!("  Draft generated: \"{}\" (type: {}, quality: {:.2})", title, content_type, quality);
    println!("  View with: docbrain autopilot drafts");
    println!();

    Ok(())
}

async fn autopilot_dismiss(server_url: &str, cluster_id: &str, api_key: Option<&str>) -> Result<()> {
    let client = reqwest::Client::new();
    let mut request = client.post(format!("{}/api/v1/autopilot/gaps/{}/dismiss", server_url, cluster_id));
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    println!("  Gap cluster dismissed.");
    println!();

    Ok(())
}

async fn autopilot_publish(server_url: &str, draft_id: &str, target: Option<&str>, api_key: Option<&str>) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    let mut url = format!("{}/api/v1/autopilot/drafts/{}/publish", server_url, draft_id);
    if let Some(t) = target {
        url.push_str(&format!("?target={}", t));
    }
    let mut request = client.post(&url);
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    println!("  Publishing draft {}...", &draft_id[..draft_id.len().min(8)]);
    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let json: serde_json::Value = response.json().await?;
    let published_url = json["published_url"].as_str().unwrap_or("(unknown)");
    let target_name = target.unwrap_or("auto-routed");

    println!("  \x1b[32m✓\x1b[0m Draft published to {} target", target_name);
    println!("  URL: {}", published_url);
    println!();

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Admin commands
// ═══════════════════════════════════════════════════════════════════════════

async fn handle_admin(server_url: &str, action: AdminAction, api_key: &str) -> Result<()> {
    match action {
        AdminAction::Ingest { action } => match action {
            AdminIngestAction::Trigger { sources } => {
                admin_ingest_trigger(server_url, sources.as_deref(), api_key).await
            }
        },
    }
}

async fn admin_ingest_trigger(
    server_url: &str,
    sources: Option<&str>,
    api_key: &str,
) -> Result<()> {
    #[derive(serde::Serialize)]
    struct Req<'a> {
        #[serde(skip_serializing_if = "Option::is_none")]
        sources: Option<&'a str>,
    }
    #[derive(serde::Deserialize)]
    struct Resp {
        status: String,
        message: String,
    }

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/api/v1/admin/ingest/trigger", server_url))
        .header("X-Api-Key", api_key)
        .json(&Req { sources })
        .send()
        .await?;

    let status = response.status();
    if status == reqwest::StatusCode::CONFLICT {
        println!("  An ingest job is already running. Try again later.");
        return Ok(());
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let resp: Resp = response.json().await?;
    println!("  Status:  {}", resp.status);
    println!("  {}", resp.message);
    println!();
    println!("  Tip: watch progress with `kubectl logs -f deployment/docbrain-server`");

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Subscription license status
//
// A dumb HTTP passthrough: GET the server's answer, print it. No
// verification, counting or signing logic belongs here — the server already
// did that and this just relays what it said (spec §8.4).
// ═══════════════════════════════════════════════════════════════════════════

#[derive(serde::Deserialize)]
struct LicenseSourceResponse {
    kind: String,
    path: Option<String>,
}

#[derive(serde::Deserialize)]
struct LicenseCertificateResponse {
    org_name: String,
    band: String,
    declared_engineers: i64,
    expires_at: i64,
}

#[derive(serde::Deserialize)]
struct RenewalNoticeResponse {
    days_remaining: i64,
    urgency: String,
}

#[derive(serde::Deserialize)]
struct LicenseResponse {
    state: String,
    source: LicenseSourceResponse,
    warning: Option<String>,
    reason: Option<String>,
    certificate: Option<LicenseCertificateResponse>,
    renewal_notice: Option<RenewalNoticeResponse>,
}

async fn handle_license(server_url: &str, action: LicenseAction, api_key: &str) -> Result<()> {
    match action {
        LicenseAction::Show => license_show(server_url, api_key).await,
        LicenseAction::Attest { export, output } => match (export, output) {
            (true, Some(path)) => attest_export(server_url, api_key, &path).await,
            (true, None) => anyhow::bail!("--export requires -o/--output FILE"),
            (false, _) => attest_show(server_url, api_key).await,
        },
    }
}

/// Prints whatever the server reports. No non-`valid` state is treated as an
/// error here — an absent, expired or invalid certificate is a normal,
/// reportable state, not a CLI failure. This command exits non-zero only for
/// actual failures: unreachable server, non-2xx response, unparseable body.
async fn license_show(server_url: &str, api_key: &str) -> Result<()> {
    use anyhow::Context;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/api/v1/admin/license", server_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .context("failed to reach the DocBrain server")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let license: LicenseResponse = response
        .json()
        .await
        .context("failed to parse license response from server")?;

    let source = match license.source.path.as_deref() {
        Some(path) => format!("{} ({})", license.source.kind, path),
        None => license.source.kind,
    };

    println!();
    println!("  Subscription: {}", license.state);
    if let Some(cert) = &license.certificate {
        let expiry = chrono::DateTime::from_timestamp(cert.expires_at, 0)
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| cert.expires_at.to_string());
        println!("  Org:          {}", cert.org_name);
        println!("  Band:         {}", cert.band);
        println!("  Expires:      {}", expiry);
        println!("  Engineers:    {}", cert.declared_engineers);
    }
    if let Some(reason) = &license.reason {
        println!("  Reason:       {}", reason);
    }
    println!("  Source:       {}", source);
    if let Some(warning) = &license.warning {
        println!("  Warning:      {}", warning);
    }
    if let Some(notice) = &license.renewal_notice {
        println!(
            "  Renewal:      {} days remaining ({} urgency)",
            notice.days_remaining, notice.urgency
        );
    }
    println!();

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Identity attestation report (Licensing Phase 2, spec §7)
//
// A dumb HTTP passthrough, same as license_show above: GET the server's
// answer, print it (or write it to a file for --export). No counting,
// classification or signing logic belongs here — the server computed the
// report and signed the export; this only relays what it said.
//
// Exit 0 for every state the report can describe — never_computed, a stale
// snapshot, a count that disagrees with the declared figure — same reasoning
// as license_show: a non-zero exit here would invite wiring attestation into
// a CI gate or deploy check, which is the exact thing this phase is built not
// to do. Non-zero is reserved for genuine failures: unreachable server,
// non-2xx, unparseable body, or a file-write error for --export.
// ═══════════════════════════════════════════════════════════════════════════

#[derive(serde::Deserialize)]
struct AttestationSourceCountResponse {
    source: String,
    raw: i64,
    bots: i64,
    inactive: i64,
    excluded: i64,
    counted: i64,
}

#[derive(serde::Deserialize)]
struct AttestationSourceNoLongerCountedResponse {
    source: String,
    last_counted_at: String,
    last_counted: i64,
}

#[derive(serde::Deserialize)]
struct AttestationStatusResponse {
    state: String,
    computed_at: Option<String>,
    stale: Option<bool>,
    stale_after_days: Option<i64>,
    per_source: Option<Vec<AttestationSourceCountResponse>>,
    floor_count: Option<i64>,
    ceiling_count: Option<i64>,
    sources_no_longer_counted: Option<Vec<AttestationSourceNoLongerCountedResponse>>,
    declared_engineers: Option<i64>,
    consistent_with_declared: Option<bool>,
    instance_public_key: Option<String>,
}

#[derive(serde::Deserialize)]
struct AttestationExportResponse {
    export: String,
    computed_at: String,
    install_id: String,
}

/// Names every field required when `state == "ok"` that came back `None`.
/// Every field on `AttestationStatusResponse` is `Option<T>` (states other
/// than `"ok"` legitimately omit most of them), so a server-side rename of
/// e.g. `floor_count` would otherwise deserialize silently into `None` and
/// the CLI would just print a shorter report instead of failing — exactly
/// the quiet-truncation risk this function exists to catch. An empty
/// result means the response is well-formed for the `"ok"` state.
fn missing_ok_fields(report: &AttestationStatusResponse) -> Vec<&'static str> {
    if report.state != "ok" {
        return Vec::new();
    }
    let checks: [(&'static str, bool); 7] = [
        ("computed_at", report.computed_at.is_some()),
        ("stale", report.stale.is_some()),
        ("stale_after_days", report.stale_after_days.is_some()),
        ("per_source", report.per_source.is_some()),
        ("floor_count", report.floor_count.is_some()),
        ("ceiling_count", report.ceiling_count.is_some()),
        ("sources_no_longer_counted", report.sources_no_longer_counted.is_some()),
    ];
    checks.into_iter().filter(|(_, present)| !present).map(|(name, _)| name).collect()
}

/// Formats an RFC3339 timestamp the way `license_show` formats an epoch —
/// falls back to the raw string if the server ever sends something this
/// can't parse, since a display glitch isn't worth failing the command over.
fn format_attest_timestamp(iso: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|_| iso.to_string())
}

/// Prints whatever the server reports. `never_computed` is a normal,
/// reportable state (a fresh install has nothing to show yet), not a CLI
/// failure — see the module-level exit-code note above.
async fn attest_show(server_url: &str, api_key: &str) -> Result<()> {
    use anyhow::Context;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/api/v1/admin/attestation", server_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .context("failed to reach the DocBrain server")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let report: AttestationStatusResponse = response
        .json()
        .await
        .context("failed to parse attestation response from server")?;

    let missing = missing_ok_fields(&report);
    if !missing.is_empty() {
        anyhow::bail!(
            "Server reported state \"ok\" but is missing expected field(s): {}. This usually \
             means the server renamed a field this CLI does not recognise yet — upgrade the CLI \
             rather than trust a partial report.",
            missing.join(", ")
        );
    }

    println!();

    if let Some(key) = &report.instance_public_key {
        println!("  Instance public key: {}", key);
        println!("    Give this to your vendor so they can verify your exports.");
        println!();
    }

    if report.state == "never_computed" {
        println!("  Attestation: not yet computed.");
        println!("  This is normal on a fresh install — recompute to generate the first snapshot.");
        if let Some(declared) = report.declared_engineers {
            println!("  Declared engineers: {} (nothing counted yet to compare against this)", declared);
        }
        println!();
        return Ok(());
    }

    if let Some(computed_at) = &report.computed_at {
        println!("  Computed at: {}", format_attest_timestamp(computed_at));
    }
    if report.stale == Some(true) {
        println!(
            "  Stale: snapshot is older than {} days — recompute for a current figure.",
            report.stale_after_days.unwrap_or(0)
        );
    }
    println!();

    println!(
        "  {:<20} {:>6} {:>6} {:>10} {:>10} {:>10}",
        "Source", "Raw", "Bots", "Inactive", "Excluded", "Counted"
    );
    println!("  {}", "-".repeat(70));
    for row in report.per_source.unwrap_or_default() {
        println!(
            "  {:<20} {:>6} {:>6} {:>10} {:>10} {:>10}",
            row.source, row.raw, row.bots, row.inactive, row.excluded, row.counted
        );
    }
    println!();

    if let Some(floor) = report.floor_count {
        println!("  Floor (largest single source): {}", floor);
    }
    if let Some(ceiling) = report.ceiling_count {
        println!("  Ceiling (sum across sources):   {}", ceiling);
    }

    if let Some(declared) = report.declared_engineers {
        println!("  Declared engineers:             {}", declared);
        match report.consistent_with_declared {
            Some(true) => println!("  Consistent with the declared figure."),
            Some(false) => println!("  Not consistent with the declared figure."),
            None => {}
        }
    }

    if let Some(sources_no_longer_counted) = &report.sources_no_longer_counted {
        if !sources_no_longer_counted.is_empty() {
            println!();
            println!("  Counted previously, nothing counted now:");
            println!("  (This can mean the source was disconnected, or simply that nothing was seen in the window.)");
            for d in sources_no_longer_counted {
                println!(
                    "    - {} — last counted {}, contributed {}",
                    d.source,
                    format_attest_timestamp(&d.last_counted_at),
                    d.last_counted
                );
            }
        }
    }
    println!();

    Ok(())
}

/// Writes the signed export blob to `path`. The endpoint 409s when no
/// snapshot exists yet (spec §7.1c forbids computing one inline with the
/// request) — that is reported as a plain message, not a crash, matching
/// the module-level exit-code note above.
async fn attest_export(server_url: &str, api_key: &str, path: &std::path::Path) -> Result<()> {
    use anyhow::Context;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/api/v1/admin/attestation/export", server_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .context("failed to reach the DocBrain server")?;

    if response.status() == reqwest::StatusCode::CONFLICT {
        println!("No attestation snapshot exists yet. Run a recompute, then retry the export.");
        return Ok(());
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Server error ({}): {}", status, body);
    }

    let export: AttestationExportResponse = response
        .json()
        .await
        .context("failed to parse attestation export response from server")?;

    std::fs::write(path, &export.export)
        .with_context(|| format!("writing attestation export to {}", path.display()))?;

    println!(
        "Wrote {} bytes of signed attestation export to {} (computed at {}).",
        export.export.len(),
        path.display(),
        format_attest_timestamp(&export.computed_at)
    );
    // §7.3's anti-splitting story rests on this: an operator running more
    // than one instance cannot tell their exports apart without it.
    println!("Instance id: {}", export.install_id);

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// CI/CD Pipeline Capture commands
// ═══════════════════════════════════════════════════════════════════════════

async fn handle_ci(server_url: &str, action: CiAction, api_key: &str) -> Result<()> {
    match action {
        CiAction::Analyze {
            pr_number,
            repo,
            pr_title,
            pr_body,
            diff_stat,
            changed_files,
            labels,
            author,
        } => {
            ci_analyze(
                server_url, api_key, pr_number, &repo, &pr_title, pr_body.as_deref(),
                diff_stat.as_deref(), changed_files.as_deref(), labels.as_deref(),
                author.as_deref(),
            )
            .await
        }
        CiAction::DeployCapture {
            service,
            version,
            environment,
            changelog,
            config_diff,
        } => {
            ci_deploy_capture(
                server_url, api_key, &service, &version, &environment,
                changelog.as_deref(), config_diff.as_deref(),
            )
            .await
        }
    }
}

#[derive(Deserialize)]
struct CiAnalyzeCliResponse {
    fragments_created: usize,
    fragments: Vec<CiFragmentCliSummary>,
    already_analyzed: bool,
}

#[derive(Deserialize)]
struct CiFragmentCliSummary {
    id: String,
    fragment_type: String,
    summary: String,
    confidence: f64,
    routed_action: String,
}

#[allow(clippy::too_many_arguments)]
async fn ci_analyze(
    server_url: &str,
    api_key: &str,
    pr_number: u64,
    repo: &str,
    pr_title: &str,
    pr_body: Option<&str>,
    diff_stat: Option<&str>,
    changed_files: Option<&str>,
    labels: Option<&str>,
    author: Option<&str>,
) -> Result<()> {
    let client = reqwest::Client::new();
    let mut body = serde_json::json!({
        "pr_number": pr_number,
        "repo": repo,
        "pr_title": pr_title,
    });

    if let Some(v) = pr_body {
        body["pr_body"] = serde_json::Value::String(v.to_string());
    }
    if let Some(v) = diff_stat {
        body["diff_stat"] = serde_json::Value::String(v.to_string());
    }
    if let Some(v) = changed_files {
        body["changed_files"] = serde_json::Value::String(v.to_string());
    }
    if let Some(v) = labels {
        body["labels"] = serde_json::Value::String(v.to_string());
    }
    if let Some(v) = author {
        body["author"] = serde_json::Value::String(v.to_string());
    }

    let response = client
        .post(format!("{}/api/v1/ci/analyze", server_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, text);
    }

    let result: CiAnalyzeCliResponse = response.json().await?;

    println!();
    if result.already_analyzed {
        println!("  PR already analyzed — no new fragments created.");
    } else if result.fragments_created == 0 {
        println!("  No knowledge fragments extracted (trivial PR).");
    } else {
        println!("  \x1b[32m{}\x1b[0m fragment(s) extracted from PR", result.fragments_created);
        println!();
        for frag in &result.fragments {
            let action_color = match frag.routed_action.as_str() {
                "auto_index" => "\x1b[32m",    // green
                "queue_for_review" => "\x1b[33m", // yellow
                _ => "\x1b[90m",                // gray
            };
            println!(
                "  [{:<9}] {:.2} {}{}{}",
                frag.fragment_type,
                frag.confidence,
                action_color,
                frag.routed_action,
                "\x1b[0m"
            );
            println!("             {}", frag.summary);
            println!("             id: {}", frag.id);
            println!();
        }
    }

    Ok(())
}

#[derive(Deserialize)]
struct DeployCaptureCliResponse {
    fragment_id: Option<String>,
    summary: String,
}

async fn ci_deploy_capture(
    server_url: &str,
    api_key: &str,
    service: &str,
    version: &str,
    environment: &str,
    changelog: Option<&str>,
    config_diff: Option<&str>,
) -> Result<()> {
    let client = reqwest::Client::new();
    let mut body = serde_json::json!({
        "service": service,
        "version": version,
        "environment": environment,
    });

    if let Some(v) = changelog {
        body["changelog"] = serde_json::Value::String(v.to_string());
    }
    if let Some(v) = config_diff {
        body["config_diff"] = serde_json::Value::String(v.to_string());
    }

    let response = client
        .post(format!("{}/api/v1/ci/deploy-capture", server_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await?;
        anyhow::bail!("Server error ({}): {}", status, text);
    }

    let result: DeployCaptureCliResponse = response.json().await?;

    println!();
    if let Some(ref id) = result.fragment_id {
        println!("  \x1b[32m✓\x1b[0m Deploy captured: {}", result.summary);
        println!("    fragment id: {}", id);
    } else {
        println!("  {}", result.summary);
    }
    println!();

    Ok(())
}



// ═══════════════════════════════════════════════════════════════════════════
// Evidence bundles — verify / why / tables (OFFLINE) + export (network)
// ═══════════════════════════════════════════════════════════════════════════

use docbrain_evidence::{
    chain_heads_for_bundle, read_records, verify_bundle, RecordHeader, Verdict, VerdictReport,
};

/// CLI-level error exit code (NOT a verdict — 0/1/2 are the three verdicts).
/// A missing/unreadable file, a bad argument, or a failed local write.
const EXIT_CLI_ERROR: i32 = 3;

async fn handle_evidence(action: EvidenceAction, server_url: &str, api_key: Option<&str>) -> Result<()> {
    match action {
        EvidenceAction::Verify { bundle, against, json } => {
            // Never returns — sets the process exit code to the verdict.
            evidence_verify(&bundle, against.as_deref(), json);
        }
        EvidenceAction::Why { record, bundle } => {
            evidence_why(&record, &bundle);
        }
        EvidenceAction::Tables { bundle, out } => {
            evidence_tables(&bundle, &out);
        }
        EvidenceAction::Export { range, profile, preset, out } => {
            // The ONLY networked evidence subcommand — returns normally (exit
            // 0 on success, anyhow error → exit 1).
            evidence_export(server_url, api_key, range.as_deref(), profile.as_deref(), preset.as_deref(), &out).await
        }
    }
}

/// Read a `.dbev` file, or print a CLI error to stderr and exit 3. A missing
/// or unreadable file is a CLI-level failure, never a verdict.
fn read_bundle_or_exit(path: &std::path::Path) -> Vec<u8> {
    match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("error: cannot read bundle {}: {e}", path.display());
            std::process::exit(EXIT_CLI_ERROR);
        }
    }
}

/// Flush stdout (block-buffered when piped) before a hard `process::exit`,
/// which does not run destructors or flush on its own.
fn exit_with(code: i32) -> ! {
    std::io::stdout().flush().ok();
    std::process::exit(code);
}

// ---- verify (verdict-bearing: MUST be perfect) ----

/// The outcome of a `--against` cross-bundle comparison over the overlapping
/// journal range. `Consistent`/`Fork` are only ever computed when BOTH
/// bundles independently verify VALID.
enum AgainstOutcome {
    /// Overlapping positions all agree — the chains are prefix-compatible.
    Consistent { shared: usize },
    /// Same journal (shared genesis identity) and a shared position carries
    /// DIFFERENT heads: cryptographic proof of a forked journal.
    Fork { position: u64 },
    /// No overlapping positions — no claim either way.
    NotComparable,
    /// The two bundles are DIFFERENT journals (distinct genesis signing
    /// keys). A head disagreement here is NOT a fork — it is expected between
    /// unrelated journals — so no consistency claim is made and no adverse
    /// verdict is asserted (e.g. the operator grabbed the wrong earlier file).
    DifferentJournals,
    /// One bundle is not VALID, so no consistency claim can be made.
    NotValid { which: &'static str, verdict: Verdict },
    /// Both VALID but the heads could not be re-derived (structurally
    /// unreachable — a VALID verdict means the bootstrap already succeeded —
    /// but handled rather than unwrapped).
    Error { detail: String },
}

/// Verify `bundle` offline, optionally cross-check against an EARLIER bundle,
/// print the report, and exit with the verdict's code (fork forces exit 1).
/// Never returns.
fn evidence_verify(bundle: &std::path::Path, against: Option<&std::path::Path>, json: bool) -> ! {
    let bytes = read_bundle_or_exit(bundle);
    let report = verify_bundle(&bytes);

    let Some(earlier_path) = against else {
        // Plain verify: output shape is IDENTICAL to the standalone
        // `docbrain-verify` binary.
        if json {
            println!("{}", serde_json::to_string_pretty(&report.to_json()).unwrap_or_default());
        } else {
            print!("{}", report.render_human());
        }
        exit_with(report.verdict.exit_code());
    };

    let earlier_bytes = read_bundle_or_exit(earlier_path);
    let earlier_report = verify_bundle(&earlier_bytes);
    let outcome = compare_against(&bytes, &earlier_bytes, &report, &earlier_report);

    if json {
        let out = serde_json::json!({
            "bundle": report.to_json(),
            "against": against_json(&earlier_report, &outcome),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    } else {
        print!("{}", report.render_human());
        print_against_human(earlier_path, &earlier_report, &outcome);
    }

    // A proven fork is cryptographic proof of tampering across the pair — it
    // forces a failing exit even if THIS bundle alone verifies VALID.
    let code = match outcome {
        AgainstOutcome::Fork { .. } => Verdict::Tampered.exit_code(),
        _ => report.verdict.exit_code(),
    };
    exit_with(code);
}

/// Compare two bundles' per-position heads over their overlapping range. The
/// genesis anchor (position 0, all-zero head — a universal constant carrying
/// no journal identity) is excluded, so "consistent" always rests on at least
/// one real shared position. Both bundles must be VALID first; otherwise no
/// claim is made.
fn compare_against(
    bundle_bytes: &[u8],
    earlier_bytes: &[u8],
    report: &VerdictReport,
    earlier_report: &VerdictReport,
) -> AgainstOutcome {
    if report.verdict != Verdict::Valid {
        return AgainstOutcome::NotValid { which: "this bundle", verdict: report.verdict };
    }
    if earlier_report.verdict != Verdict::Valid {
        return AgainstOutcome::NotValid { which: "the --against bundle", verdict: earlier_report.verdict };
    }

    let a = match chain_heads_for_bundle(bundle_bytes) {
        Ok(h) => h,
        Err(e) => return AgainstOutcome::Error { detail: e.to_string() },
    };
    let b = match chain_heads_for_bundle(earlier_bytes) {
        Ok(h) => h,
        Err(e) => return AgainstOutcome::Error { detail: e.to_string() },
    };

    // Identity gate FIRST (the narrowing that keeps FORK an honest accusation):
    // a head disagreement only proves a FORK when both bundles are the SAME
    // journal — i.e. share the genesis signing key (the immutable, self-signed
    // TOFU root). Between DIFFERENT journals a head disagreement is expected,
    // not fraud, so it can never be reported as a fork. A real intra-journal
    // fork still shares the genesis key and is still caught below; a
    // post-compromise successor-genesis lineage is different-genesis → the
    // safe "different journals / not-comparable" answer.
    if a.genesis_identity != b.genesis_identity {
        return AgainstOutcome::DifferentJournals;
    }

    // Map earlier bundle's positions → head, excluding the genesis anchor.
    let earlier_by_pos: std::collections::HashMap<u64, [u8; 32]> =
        b.heads.iter().filter(|(p, _)| *p != 0).copied().collect();

    let mut shared = 0usize;
    let mut fork_at: Option<u64> = None;
    for (pos, head) in a.heads.iter().filter(|(p, _)| *p != 0) {
        if let Some(other) = earlier_by_pos.get(pos) {
            shared += 1;
            if other != head {
                // Report the LOWEST forking position for a stable message.
                fork_at = Some(fork_at.map_or(*pos, |f| f.min(*pos)));
            }
        }
    }

    match (fork_at, shared) {
        (Some(position), _) => AgainstOutcome::Fork { position },
        (None, 0) => AgainstOutcome::NotComparable,
        (None, shared) => AgainstOutcome::Consistent { shared },
    }
}

const AGAINST_CAVEAT: &str =
    "Note: two bundles handed over together can both come from the same clean fork. A \
consistency claim only holds if you (the relying party) retained the earlier bundle \
independently, at the earlier time.";

fn print_against_human(earlier_path: &std::path::Path, earlier_report: &VerdictReport, outcome: &AgainstOutcome) {
    println!("\n--against {} (verdict: {})", earlier_path.display(), earlier_report.verdict.as_str());
    match outcome {
        AgainstOutcome::Consistent { shared } => {
            println!("Cross-check: consistent — {shared} overlapping position(s) agree; the chains are prefix-compatible.");
            println!("{AGAINST_CAVEAT}");
        }
        AgainstOutcome::Fork { position } => {
            println!("Cross-check: FORK DETECTED at position {position} — the two bundles carry different heads for the same position. This is cryptographic proof of a forked journal.");
        }
        AgainstOutcome::NotComparable => {
            println!("Cross-check: not-comparable — the two bundles' exported ranges do not overlap. No consistency claim is made either way.");
        }
        AgainstOutcome::DifferentJournals => {
            println!("Cross-check: different journals — the two bundles have distinct genesis signing keys, so they are not the same journal. not-comparable; no consistency claim is made either way (and this is NOT a fork).");
        }
        AgainstOutcome::NotValid { which, verdict } => {
            println!("Cross-check: skipped — {which} is {} (not VALID). No consistency claim is made.", verdict.as_str());
        }
        AgainstOutcome::Error { detail } => {
            println!("Cross-check: could not compare ({detail}). No consistency claim is made.");
        }
    }
}

fn against_json(earlier_report: &VerdictReport, outcome: &AgainstOutcome) -> serde_json::Value {
    let (result, mut extra) = match outcome {
        AgainstOutcome::Consistent { shared } => ("consistent", serde_json::json!({ "shared_positions": shared, "caveat": AGAINST_CAVEAT })),
        AgainstOutcome::Fork { position } => ("fork_detected", serde_json::json!({ "fork_position": position })),
        AgainstOutcome::NotComparable => ("not_comparable", serde_json::json!({})),
        AgainstOutcome::DifferentJournals => ("different_journals", serde_json::json!({ "note": "distinct genesis signing keys; not the same journal, not a fork" })),
        AgainstOutcome::NotValid { which, verdict } => ("skipped", serde_json::json!({ "reason": format!("{which} is {}", verdict.as_str()) })),
        AgainstOutcome::Error { detail } => ("error", serde_json::json!({ "detail": detail })),
    };
    extra["result"] = serde_json::json!(result);
    extra["earlier_verdict"] = serde_json::json!(earlier_report.verdict.as_str());
    extra
}

// ---- why (offline; refuses non-VALID; lower-criticality rendering) ----

fn evidence_why(record: &str, bundle: &std::path::Path) -> ! {
    let bytes = read_bundle_or_exit(bundle);
    let report = verify_bundle(&bytes);
    if report.verdict != Verdict::Valid {
        eprintln!(
            "refusing to explain a {} bundle — never render record content from an unverified bundle.",
            report.verdict.as_str()
        );
        eprintln!("Reason: {}", report.dominant.detail);
        exit_with(Verdict::CannotVerify.exit_code());
    }

    let records = match read_records(&bytes) {
        Ok(r) => r,
        Err(e) => {
            // Unreachable after a VALID verdict; handled, not unwrapped.
            eprintln!("error: bundle verified VALID but records could not be read: {e}");
            exit_with(EXIT_CLI_ERROR);
        }
    };

    let Some(primary) = find_record(&records, record) else {
        eprintln!("No record in this bundle matches {record:?} (tried journal position and record-body id).");
        exit_with(0);
    };

    render_why(primary, &records);
    exit_with(0);
}

/// Find the record a `why` selector names: a bare integer matches a journal
/// position; anything else matches a record-body id field (`id`, `premise_id`,
/// `fragment_id`).
fn find_record<'a>(records: &'a [RecordHeader], selector: &str) -> Option<&'a RecordHeader> {
    if let Ok(pos) = selector.parse::<u64>()
        && let Some(r) = records.iter().find(|r| r.position == pos)
    {
        return Some(r);
    }
    records.iter().find(|r| {
        ["id", "premise_id", "fragment_id"]
            .iter()
            .any(|k| body_str(&r.body, k).as_deref() == Some(selector))
    })
}

/// A body string field, if present and a string — the graceful accessor `why`
/// uses everywhere so a missing/mistyped field is skipped, never a panic.
fn body_str(body: &serde_json::Value, key: &str) -> Option<String> {
    body.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn render_why(primary: &RecordHeader, all: &[RecordHeader]) {
    println!("Record @position {} — {}/{} at {}", primary.position, primary.class, primary.kind, primary.at);
    println!("  actor: {}", primary.actor);

    // Fields extracted BY NAME from the record body — each printed only if
    // present (decision/fragment-created carry id/fragment_type/summary_hash/
    // routed_status; premise transitions carry the state fields; a discard
    // carries a reason). Unknown shapes simply print nothing extra.
    for (label, key) in [
        ("id", "id"),
        ("type", "fragment_type"),
        ("summary-hash", "summary_hash"),
        ("routed", "routed_status"),
        ("reason", "reason"),
        ("expression", "expression"),
        ("premise-id", "premise_id"),
        ("fragment-id", "fragment_id"),
    ] {
        if let Some(v) = body_str(&primary.body, key) {
            println!("  {label}: {v}");
        }
    }

    // "sources" — extracted by name, rendered if present (a decision may carry
    // a sources array; skipped gracefully when absent).
    if let Some(sources) = primary.body.get("sources") {
        println!("  sources: {sources}");
    }

    // premise state, when this record is itself a transition.
    if let (Some(new_state), old_state) = (body_str(&primary.body, "new_state"), body_str(&primary.body, "old_state")) {
        match old_state {
            Some(old) => println!("  premise state: {old} -> {new_state}"),
            None => println!("  premise state: -> {new_state}"),
        }
    }

    // The record's own id, used to correlate approvals and premise history.
    if let Some(id) = body_str(&primary.body, "id") {
        // approver: an `approved`-kind record whose body id matches — its
        // actor is who approved it.
        if let Some(appr) = all.iter().find(|r| r.kind == "approved" && body_str(&r.body, "id").as_deref() == Some(&id)) {
            println!("  approved by: {}", appr.actor);
        }
        // premise states referencing this record as their fragment.
        let transitions: Vec<&RecordHeader> = all
            .iter()
            .filter(|r| r.kind == "transition" && body_str(&r.body, "fragment_id").as_deref() == Some(&id))
            .collect();
        if !transitions.is_empty() {
            println!("  premise transitions ({}):", transitions.len());
            for t in transitions {
                let expr = body_str(&t.body, "expression").unwrap_or_default();
                let old = body_str(&t.body, "old_state").unwrap_or_else(|| "?".to_string());
                let new = body_str(&t.body, "new_state").unwrap_or_else(|| "?".to_string());
                println!("    - {expr}: {old} -> {new}");
            }
        }
    }
}

// ---- tables (offline; refuses non-VALID; populations CSV) ----

fn evidence_tables(bundle: &std::path::Path, out: &std::path::Path) -> ! {
    let bytes = read_bundle_or_exit(bundle);
    let report = verify_bundle(&bytes);
    if report.verdict != Verdict::Valid {
        eprintln!(
            "refusing to tabulate a {} bundle — never render record content from an unverified bundle.",
            report.verdict.as_str()
        );
        exit_with(Verdict::CannotVerify.exit_code());
    }

    let records = match read_records(&bytes) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: bundle verified VALID but records could not be read: {e}");
            exit_with(EXIT_CLI_ERROR);
        }
    };

    // Populations: record counts per (class, kind), sorted for a stable file.
    let mut populations: std::collections::BTreeMap<(String, String), u64> = std::collections::BTreeMap::new();
    for r in &records {
        *populations.entry((r.class.clone(), r.kind.clone())).or_insert(0) += 1;
    }

    // Bundle-digest header row: a plain SHA-256 of the whole `.dbev`, so the
    // CSV is tied to one specific bundle. Not a trust hash — a display digest.
    let digest = {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(&bytes))
    };

    let mut csv = String::new();
    csv.push_str(&format!(
        "# bundle_sha256={digest},verdict={},range=[{},{}],records={}\n",
        report.verdict.as_str(),
        report.scope.range.0,
        report.scope.range.1,
        report.counts.records,
    ));
    csv.push_str("class,kind,count\n");
    for ((class, kind), count) in &populations {
        csv.push_str(&format!("{},{},{}\n", csv_field(class), csv_field(kind), count));
    }

    if let Err(e) = std::fs::write(out, csv.as_bytes()) {
        eprintln!("error: cannot write CSV {}: {e}", out.display());
        exit_with(EXIT_CLI_ERROR);
    }
    eprintln!("Wrote {} population row(s) to {}", populations.len(), out.display());
    exit_with(0);
}

/// Minimal RFC-4180 CSV escaping: quote a field containing a comma, quote or
/// newline; double any embedded quotes. `class`/`kind` are closed-vocabulary
/// today, but escaping keeps the CSV well-formed if that ever changes.
fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ---- export (the ONLY networked evidence subcommand) ----

async fn evidence_export(
    server_url: &str,
    api_key: Option<&str>,
    range: Option<&str>,
    profile: Option<&str>,
    preset: Option<&str>,
    out: &std::path::Path,
) -> Result<()> {
    use anyhow::Context;
    let key = api_key.ok_or_else(|| anyhow::anyhow!(
        "API key required for export. Run `docbrain login` or set DOCBRAIN_API_KEY. \
         (verify/why/tables need no key — export is the only networked evidence subcommand.)"
    ))?;

    if range.is_some() && preset.is_some() {
        anyhow::bail!("--range and --preset are alternatives; pass at most one.");
    }

    let mut body = serde_json::Map::new();
    if let Some(r) = range {
        let (start, end) = r
            .split_once(',')
            .ok_or_else(|| anyhow::anyhow!("--range must be START,END (e.g. 0,1200)"))?;
        let start: u64 = start.trim().parse().map_err(|_| anyhow::anyhow!("--range START must be a non-negative integer"))?;
        let end: u64 = end.trim().parse().map_err(|_| anyhow::anyhow!("--range END must be a non-negative integer"))?;
        body.insert("range".to_string(), serde_json::json!([start, end]));
    }
    if let Some(p) = profile {
        body.insert("profile".to_string(), serde_json::json!(p));
    }
    if let Some(p) = preset {
        body.insert("preset".to_string(), serde_json::json!(p));
    }

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/api/v1/evidence/export", server_url))
        .header("Authorization", format!("Bearer {}", key))
        .json(&serde_json::Value::Object(body))
        .send()
        .await
        .context("sending evidence export request to the DocBrain server")?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("evidence export failed ({}): {}", status, text);
    }

    let bytes = response.bytes().await.context("reading the exported .dbev bundle body from the server")?;
    std::fs::write(out, &bytes)
        .map_err(|e| anyhow::anyhow!("cannot write bundle {}: {e}", out.display()))?;
    eprintln!("Wrote {} bytes to {}. Verify it offline: docbrain-verify {}", bytes.len(), out.display(), out.display());
    Ok(())
}


// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// The `generate` subcommand parses the full documented flag surface, and
    /// the destructured fields match what `main`'s dispatch arm reads. This is a
    /// pure arg-parse contract test — it does not hit the network.
    #[test]
    fn generate_parses_full_flag_surface() {
        let cli = Cli::try_parse_from([
            "docbrain",
            "generate",
            "runbook for cert rotation",
            "--source",
            "notes.md",
            "--source",
            "incident.log",
            "--stdin",
            "--type",
            "runbook",
            "--space",
            "OPS",
            "--target",
            "https://wiki/existing",
            "--template",
            "team.tmpl",
            "--out",
            "out.md",
            "--force",
            "--no-enrich",
            "--allow-violations",
        ])
        .expect("full generate invocation parses");

        match cli.command {
            Commands::Generate {
                ask,
                sources,
                source_urls,
                stdin,
                target,
                template,
                doc_type,
                space,
                out,
                force,
                no_enrich,
                allow_violations,
                max_regen_rounds,
            } => {
                assert_eq!(ask, "runbook for cert rotation");
                assert_eq!(
                    sources,
                    vec![PathBuf::from("notes.md"), PathBuf::from("incident.log")]
                );
                assert!(source_urls.is_empty());
                assert!(stdin);
                assert_eq!(target.as_deref(), Some("https://wiki/existing"));
                assert_eq!(template, Some(PathBuf::from("team.tmpl")));
                assert_eq!(doc_type.as_deref(), Some("runbook"));
                assert_eq!(space.as_deref(), Some("OPS"));
                assert_eq!(out, Some(PathBuf::from("out.md")));
                assert!(force);
                assert!(no_enrich);
                assert!(allow_violations);
                // Default auto-review rounds when --max-regen-rounds is
                // not passed.
                assert_eq!(max_regen_rounds, 2);
            }
            _ => panic!("expected Commands::Generate"),
        }
    }

    /// The minimal invocation: just the ask. Every optional flag defaults off /
    /// empty, matching the server's `#[serde(default)]` paths.
    #[test]
    fn generate_parses_minimal_invocation() {
        let cli = Cli::try_parse_from(["docbrain", "generate", "write the X guide"])
            .expect("minimal generate invocation parses");

        match cli.command {
            Commands::Generate {
                ask,
                sources,
                source_urls,
                stdin,
                target,
                template,
                doc_type,
                space,
                out,
                force,
                no_enrich,
                allow_violations,
                max_regen_rounds,
            } => {
                assert_eq!(ask, "write the X guide");
                assert!(sources.is_empty());
                assert!(source_urls.is_empty());
                assert!(!stdin);
                assert!(target.is_none());
                assert!(template.is_none());
                assert!(doc_type.is_none());
                assert!(space.is_none());
                assert!(out.is_none());
                assert!(!force, "force defaults off");
                assert!(!no_enrich);
                assert!(!allow_violations);
                assert_eq!(max_regen_rounds, 2, "default auto-review rounds");
            }
            _ => panic!("expected Commands::Generate"),
        }
    }

    // ── — the CLI auto-review loop's pure helpers ──────────────────────
    fn flag(claim: &str, evidence: &[&str]) -> DraftFreshnessFlagResponse {
        DraftFreshnessFlagResponse {
            claim: claim.to_string(),
            evidence_cited: evidence.iter().map(|s| s.to_string()).collect(),
            note: None,
        }
    }

    #[test]
    fn flag_keys_are_evidence_order_independent() {
        // Two flags citing the same evidence in different order have the SAME
        // identity (the key sorts evidence). So a reworded claim over the same
        // evidence does NOT look like a new flag.
        let a = regen_flag_keys(&[flag("X uses LastPass", &["doc-2", "doc-1"])]);
        let b = regen_flag_keys(&[flag("LastPass is used by X", &["doc-1", "doc-2"])]);
        assert_eq!(a, b, "same evidence (any order) ⇒ same flag identity");
    }

    #[test]
    fn convergence_detects_a_resolved_flag() {
        // Round prior had {ev=[d1]} and {ev=[d2]}; next has only {ev=[d2]}. The
        // first flag resolved ⇒ progress (resolved_any = true).
        let prior = regen_flag_keys(&[flag("c1", &["d1"]), flag("c2", &["d2"])]);
        let next = regen_flag_keys(&[flag("c2 reworded", &["d2"])]);
        let resolved_any = prior.iter().any(|k| !next.contains(k));
        assert!(resolved_any, "a previously-flagged claim cleared");
    }

    #[test]
    fn convergence_plateau_when_nothing_resolved() {
        // Next round still flags the SAME evidence (plus a NEW one). No prior key
        // cleared ⇒ plateau (resolved_any = false), even though the count grew.
        let prior = regen_flag_keys(&[flag("c1", &["d1"])]);
        let next = regen_flag_keys(&[flag("c1 reworded", &["d1"]), flag("c3", &["d3"])]);
        let resolved_any = prior.iter().any(|k| !next.contains(k));
        assert!(!resolved_any, "no prior flag cleared ⇒ plateau, despite a new flag appearing");
    }

    #[test]
    fn regen_feedback_is_bounded_and_lists_claims() {
        // Many long flags ⇒ feedback enumerates a capped subset, summarizes the
        // rest, and never exceeds the server's 2000-byte bound.
        let long = "claim ".repeat(100);
        let flags: Vec<_> = (0..20).map(|i| flag(&format!("{long} {i}"), &["d"])).collect();
        let fb = build_regen_feedback(&flags);
        assert!(fb.len() <= 2_000, "feedback stays under the server bound, got {}", fb.len());
        assert!(fb.contains("CONTRADICTS"), "feedback frames the task");
        assert!(fb.contains("more flagged claim"), "summarizes the overflow");
    }

    #[test]
    fn regen_feedback_lists_each_claim_when_few() {
        let flags = vec![flag("GitHub uses LastPass TOTP", &["d1"]), flag("Port is 8080", &["d2"])];
        let fb = build_regen_feedback(&flags);
        assert!(fb.contains("GitHub uses LastPass TOTP"));
        assert!(fb.contains("Port is 8080"));
        assert!(!fb.contains("more flagged claim"), "no overflow summary when few");
    }

    // ── / — the --target wrong-flag guard host check ──────
    #[test]
    fn non_augmentable_source_urls_are_recognized() {
        // Slack threads + GitHub PR/file CANNOT be augmented in place — passing one
        // to --target with no --source is the mistake the guard catches.
        assert!(is_non_augmentable_source_url(
            "https://example.slack.com/archives/C036KEST91A/p1781281610117499"
        ));
        assert!(is_non_augmentable_source_url("https://github.com/acme/repo/pull/42"));
        assert!(is_non_augmentable_source_url("https://www.github.com/acme/repo/blob/main/x.md"));
        // Case + port + query tolerated (host parsed up to / : ?).
        assert!(is_non_augmentable_source_url("HTTPS://Acme.Slack.com:443/archives/C1/p1?x=2"));
    }

    #[test]
    fn augmentable_and_plain_targets_are_left_alone() {
        // A Confluence/Atlassian page IS the augment target —
        // `--target <confluence>` with no sources is the PRIMARY augment-by-corpus
        // flow and must NEVER be flagged. (These previously asserted the OPPOSITE.)
        assert!(!is_non_augmentable_source_url("https://acme.atlassian.net/browse/PROJ-1"));
        assert!(!is_non_augmentable_source_url(
            "https://acme.atlassian.net/wiki/spaces/X/pages/123/Title"
        ));
        // A plain reference, a wiki shortname, or an unknown host → legit augment
        // target, guard must NOT fire.
        assert!(!is_non_augmentable_source_url("runbooks/cert-rotation")); // plain ref
        assert!(!is_non_augmentable_source_url("https://wiki.internal.example/x")); // unknown host
        assert!(!is_non_augmentable_source_url("CERT-ROTATION")); // doc key
        assert!(!is_non_augmentable_source_url("")); // empty
        assert!(!is_non_augmentable_source_url("ftp://github.com/x")); // not http(s)
        // A host that merely CONTAINS a known token but isn't it → no match.
        assert!(!is_non_augmentable_source_url("https://github.com.evil.example/x"));
        assert!(!is_non_augmentable_source_url("https://notslack.com/x"));
    }

    /// The ask is a required positional — omitting it is a parse error, not a
    /// silent empty-ask request.
    #[test]
    fn generate_requires_the_ask() {
        let err = Cli::try_parse_from(["docbrain", "generate"]);
        assert!(err.is_err(), "generate with no ask must fail to parse");
    }

    /// A `GeneratedArtifact` JSON from the server (snake_case, all fields)
    /// round-trips into the CLI's response struct, including the precomputed
    /// `has_error_severity` flag that drives the exit code.
    #[test]
    fn artifact_response_deserializes_server_shape() {
        let json = serde_json::json!({
            "markdown": "# Title body",
            "doc_type": "runbook",
            "provenance": [{"section": "Overview", "source_ids": ["c1", "c2"]}],
            "needs_input": ["what is the rollback step?"],
            "skipped_sources": [{"label": "jira", "reason": "tool not connected"}],
            "quality": {
                "score": 0.42,
                "violations": [
                    {"rule_name": "missing_section", "severity": "error", "message": "no Steps section"}
                ],
                "has_error_severity": true
            }
        });
        let artifact: GeneratedArtifactResponse =
            serde_json::from_value(json).expect("artifact deserializes");
        assert_eq!(artifact.doc_type, "runbook");
        assert!((artifact.quality.score.expect("score present") - 0.42).abs() < f64::EPSILON);
        assert_eq!(artifact.needs_input.len(), 1);
        assert_eq!(artifact.skipped_sources[0].label, "jira");
        assert_eq!(artifact.quality.violations[0].rule_name, "missing_section");
        assert!(artifact.quality.has_error_severity);
    }

    // ── Coverage + honest trust headline ────────────────────────────

    /// The MOAT wire fields (coverage rows, distinct_sources, unverified_live_only,
    /// and the grounding count fields incl. supported_high_count + degraded) all
    /// deserialize off the server's snake_case artifact JSON.
    #[test]
    fn artifact_response_deserializes_moat_fields() {
        let json = serde_json::json!({
            "markdown": "# Title",
            "doc_type": "runbook",
            "quality": {"score": 0.9, "violations": [], "has_error_severity": false},
            "distinct_sources": 11,
            "unverified_live_only": false,
            "coverage": [
                {"tool": "confluence_search", "status": "hit", "doc_count": 7},
                {"tool": "jira_search", "status": "hit", "doc_count": 2},
                {"tool": "github_search", "status": "not_connected", "doc_count": 0}
            ],
            "grounding": {
                "checked_count": 11,
                "supported_count": 9,
                "supported_high_count": 8,
                "total_blocks": 14,
                "unchecked_count": 3,
                "degraded": []
            }
        });
        let a: GeneratedArtifactResponse =
            serde_json::from_value(json).expect("moat artifact deserializes");
        assert_eq!(a.distinct_sources, 11);
        assert!(!a.unverified_live_only);
        assert_eq!(a.coverage.len(), 3);
        assert_eq!(a.coverage[0].status, "hit");
        assert_eq!(a.coverage[0].doc_count, 7);
        let g = a.grounding.expect("grounding present");
        assert_eq!(g.supported_high_count, Some(8));
        assert_eq!(g.total_blocks, 14);
        assert_eq!(g.unchecked_count, 3);
        assert!(g.degraded.is_empty());
    }

    /// BlindSpot HIGH regression: a grounding report that is PRESENT and NON-degraded
    /// but OMITS `supported_high_count` (an old/persisted server predating the
    /// tier-split) must render "verification unavailable", NEVER a false
    /// "0 of N verified". The bug was `supported_high_count: usize` + serde-default,
    /// which collapsed an absent field to a genuine-looking `0`. Now `Option` →
    /// absent deserializes to `None` → the unavailable branch.
    #[test]
    fn pre_tier_split_grounding_renders_unavailable_not_zero_verified() {
        let json = serde_json::json!({
            "markdown": "# Title",
            "doc_type": "runbook",
            "quality": {"score": 0.9, "violations": [], "has_error_severity": false},
            "grounding": {
                "checked_count": 11,
                "supported_count": 9,
                // supported_high_count INTENTIONALLY ABSENT (old server)
                "degraded": []
            }
        });
        let a: GeneratedArtifactResponse =
            serde_json::from_value(json).expect("pre-tier-split artifact deserializes");
        let g = a.grounding.as_ref().expect("grounding present");
        assert_eq!(
            g.supported_high_count, None,
            "absent supported_high_count must deserialize to None, not Some(0)"
        );
        // The non-degraded arm now propagates the Option directly: None → unavailable.
        let verified = match a.grounding.as_ref() {
            Some(g) if !g.degraded.is_empty() => None,
            Some(g) => g.supported_high_count,
            None => None,
        };
        assert_eq!(verified, None, "old-server grounding → verification unavailable");
        let out = render_trust_summary(0, 0, 0, verified, 11, 0, 0, false);
        assert!(
            !out.contains("0 of 11"),
            "must NOT render a false '0 of N verified' for an old-server report: {out}"
        );
        assert!(out.to_lowercase().contains("unavailable"), "expected 'unavailable': {out}");
    }

    /// An OLD server that omits the MOAT fields still deserializes — serde-default
    /// leaves coverage empty, scalars 0, the bool false. (Break-lens case ii.)
    #[test]
    fn artifact_response_old_server_no_moat_fields() {
        let json = serde_json::json!({
            "markdown": "# Title",
            "doc_type": "runbook",
            "quality": {"score": 0.9, "violations": [], "has_error_severity": false}
        });
        let a: GeneratedArtifactResponse =
            serde_json::from_value(json).expect("old-shape artifact deserializes");
        assert!(a.coverage.is_empty());
        assert_eq!(a.distinct_sources, 0);
        assert!(!a.unverified_live_only);
    }

    /// Happy path: a coverage line ("Searched N sources across M tools") AND a real
    /// verified count ("8 of 11 ... verified").
    #[test]
    fn cli_trust_summary_shows_coverage_and_verified() {
        let out = render_trust_summary(
            /* distinct_sources */ 11,
            /* contributed_tools */ 4,
            /* searched_tools */ 6,
            /* verified */ Some(8),
            /* checked */ 11,
            /* total_blocks */ 14,
            /* needs_input */ 0,
            /* unverified_live_only */ false,
        );
        assert!(out.contains("Searched"), "coverage verb: {out}");
        assert!(out.contains("11 sources"), "distinct sources: {out}");
        assert!(out.contains("4 tools"), "contributed tools: {out}");
        assert!(out.contains("8 of 11"), "verified ratio: {out}");
        assert!(out.contains("verified"), "verified label: {out}");
    }

    /// Honesty invariant: verified=None (degraded/absent critic) renders
    /// "verification unavailable" and emits NO fabricated number on that line.
    #[test]
    fn cli_trust_summary_unavailable_when_degraded() {
        let out = render_trust_summary(11, 4, 6, None, 0, 0, 0, false);
        assert!(
            out.contains("verification unavailable"),
            "unavailable wording: {out}"
        );
        // No fabricated "N of M ... verified" number on the verification line.
        assert!(
            !out.contains("of 0 checkable") && !out.contains("0 of"),
            "must not fabricate a verified number: {out}"
        );
    }

    /// The unverified-live-only banner appears when the flag is set.
    #[test]
    fn cli_trust_summary_unverified_live_only_banner() {
        let out = render_trust_summary(3, 1, 2, Some(0), 4, 4, 0, true);
        let lower = out.to_lowercase();
        assert!(lower.contains("unverified"), "unverified word: {out}");
        assert!(lower.contains("live"), "live word: {out}");
    }

    /// Break-lens case ii: searched_tools == 0 ⇒ the coverage line is OMITTED
    /// (no meaningless "Searched 0 sources across 0 tools").
    #[test]
    fn cli_trust_summary_omits_coverage_line_when_no_fanout() {
        let out = render_trust_summary(0, 0, 0, None, 0, 0, 0, false);
        assert!(!out.contains("Searched"), "coverage line omitted: {out}");
    }

    /// ADVERSARIAL A7: the SILENT-FAILURE shape — searched tools, NONE contributed.
    /// The moat's #1 job is to distinguish an empty corpus from a silent tool
    /// failure, so this MUST surface the failure and MUST NOT print "across 0 tools"
    /// (which reads as "no fan-out ran" — the exact lie that hides the failure).
    #[test]
    fn cli_trust_summary_surfaces_silent_tool_failure() {
        // distinct=0, contributed=0, searched=3 → all 3 tools came back empty/failed.
        let out = render_trust_summary(0, 0, 3, None, 0, 0, 0, false);
        assert!(out.contains("Searched"), "must still surface the search: {out}");
        assert!(out.contains("3 tools"), "must name the 3 searched tools: {out}");
        assert!(
            out.contains("none contributed"),
            "must say none contributed: {out}"
        );
        // The lie we are guarding against: "across 0 tools" reads as "no fan-out".
        assert!(!out.contains("across 0 tools"), "must NOT claim 0 tools: {out}");
    }

    /// ADVERSARIAL A4: a genuine non-degraded pass where 0 claims reached High-tier
    /// support is an HONEST "0 of N verified" — NOT "unavailable". Unavailable is
    /// reserved for absent/degraded (verified=None). A real zero is a real number.
    #[test]
    fn cli_trust_summary_real_zero_is_a_number_not_unavailable() {
        let out = render_trust_summary(2, 1, 1, Some(0), 3, 3, 0, false);
        assert!(out.contains("0 of 3"), "honest real-zero ratio: {out}");
        assert!(
            !out.contains("verification unavailable"),
            "a non-degraded Some(0) is NOT unavailable: {out}"
        );
    }

    // ── Reconcile proposal rendering ─────────────────────────────────────────

    /// Helper: a patch with one op + one skipped op.
    fn sample_patch() -> ReconcilePatchResponse {
        ReconcilePatchResponse {
            ops: vec![ReconcileOpResponse {
                heading_level: 2,
                heading_ordinal: 0,
                anchor_storage: "<h2>Auth</h2><p>old</p>".to_string(),
                new_section_storage: "<h2>Auth</h2><p>new</p>".to_string(),
                anchor_markdown: "## Auth\n\nold".to_string(),
                new_section_markdown: "## Auth\n\nnew".to_string(),
                reason: "GitHub uses LastPass for TOTP".to_string(),
                claim_hash: "abc123".to_string(),
            }],
            skipped_ops: vec![ReconcileSkippedOpResponse {
                heading: Some("Limits".to_string()),
                reason: "table edit — routed to human".to_string(),
            }],
        }
    }

    /// The proposal renders the op (locator + claim), the skipped op (heading +
    /// reason), the base version, the counts, and the print-only / --apply note.
    #[test]
    fn reconcile_proposal_renders_ops_and_skipped() {
        let out = format_reconcile_proposal("https://wiki/page/42", Some(7), &sample_patch());
        // Target + base version in the header.
        assert!(out.contains("https://wiki/page/42"), "target shown: {out}");
        assert!(out.contains("base version 7"), "base version shown: {out}");
        // Counts.
        assert!(out.contains("1 section(s) to replace"), "replace count: {out}");
        assert!(out.contains("1 routed to human"), "human count: {out}");
        // The op's claim (reason) and structural locator.
        assert!(out.contains("GitHub uses LastPass for TOTP"), "op reason: {out}");
        assert!(out.contains("§h2 #1"), "1-based ordinal locator: {out}");
        // The diff renders the READABLE MARKDOWN form (`## Auth`), not the storage
        // XHTML wall (`<h2>Auth</h2>`). Red `-` old line, green `+` new line.
        assert!(out.contains("- ## Auth"), "markdown before-line in diff: {out}");
        assert!(out.contains("+ ## Auth"), "markdown after-line in diff: {out}");
        assert!(
            !out.contains("<h2>Auth</h2>"),
            "diff must NOT show storage XHTML when markdown is present: {out}"
        );
        // The skipped op's heading + reason, marked routed-to-human.
        assert!(out.contains("Limits"), "skipped heading: {out}");
        assert!(out.contains("table edit — routed to human"), "skipped reason: {out}");
        // The PRINT-ONLY / --apply note.
        assert!(out.contains("PRINT-ONLY"), "print-only note: {out}");
        assert!(out.contains("--apply coming soon"), "apply-soon note: {out}");
    }

    /// A None base_version renders "unknown" rather than panicking or printing a
    /// confusing empty token.
    #[test]
    fn reconcile_proposal_unknown_base_version() {
        let out = format_reconcile_proposal("the target doc", None, &sample_patch());
        assert!(out.contains("base version unknown"), "unknown version: {out}");
    }

    /// An empty patch (no ops, no skipped) does not panic and still renders the
    /// header with zero counts (defensive — the server returns None for this, so
    /// it is not reached in practice, but the fn stays total).
    #[test]
    fn reconcile_proposal_empty_patch_no_panic() {
        let empty = ReconcilePatchResponse { ops: vec![], skipped_ops: vec![] };
        let out = format_reconcile_proposal("t", Some(1), &empty);
        assert!(out.contains("0 section(s) to replace"), "zero replace: {out}");
        assert!(out.contains("0 routed to human"), "zero human: {out}");
        // No section/human sub-lists when both are empty.
        assert!(!out.contains("sections to replace ("), "no replace list: {out}");
        assert!(!out.contains("routed to human ("), "no human list: {out}");
    }

    /// A skipped op with no heading renders an explicit placeholder, not a blank.
    #[test]
    fn reconcile_proposal_skipped_without_heading() {
        let patch = ReconcilePatchResponse {
            ops: vec![],
            skipped_ops: vec![ReconcileSkippedOpResponse {
                heading: None,
                reason: "could not locate the section in storage".to_string(),
            }],
        };
        let out = format_reconcile_proposal("t", Some(3), &patch);
        assert!(out.contains("(unlocated section)"), "placeholder heading: {out}");
        assert!(out.contains("could not locate"), "reason shown: {out}");
    }

    /// The full server artifact shape — including the reconcile fields —
    /// deserializes into the CLI mirror struct (snake_case, exact match).
    #[test]
    fn artifact_response_deserializes_with_reconcile_fields() {
        let json = serde_json::json!({
            "markdown": "# Title body",
            "doc_type": "guide",
            "quality": { "score": 0.9, "violations": [], "has_error_severity": false },
            "reconcile_base_version": 12,
            "reconcile_patch": {
                "ops": [{
                    "heading_level": 2,
                    "heading_ordinal": 1,
                    "anchor_storage": "<h2>X</h2>",
                    "new_section_storage": "<h2>X</h2><p>n</p>",
                    "reason": "stale claim",
                    "claim_hash": "h"
                }],
                "skipped_ops": [{ "heading": "Y", "reason": "routed to human" }]
            }
        });
        let artifact: GeneratedArtifactResponse =
            serde_json::from_value(json).expect("artifact with reconcile deserializes");
        let patch = artifact.reconcile_patch.expect("patch present");
        assert_eq!(patch.ops.len(), 1);
        assert_eq!(patch.ops[0].reason, "stale claim");
        assert_eq!(patch.skipped_ops.len(), 1);
        assert_eq!(artifact.reconcile_base_version, Some(12));
    }

    /// A from-scratch artifact (no `--target`) omits the reconcile fields entirely
    /// — the CLI mirror must default them to None, not error.
    #[test]
    fn artifact_response_reconcile_fields_default_none() {
        let json = serde_json::json!({
            "markdown": "# Body",
            "doc_type": "guide",
            "quality": { "score": 0.5, "violations": [], "has_error_severity": false }
        });
        let artifact: GeneratedArtifactResponse =
            serde_json::from_value(json).expect("artifact without reconcile deserializes");
        assert!(artifact.reconcile_patch.is_none());
        assert!(artifact.reconcile_base_version.is_none());
    }

    fn ok_attestation_response() -> AttestationStatusResponse {
        AttestationStatusResponse {
            state: "ok".to_string(),
            computed_at: Some("2026-08-01T00:00:00Z".to_string()),
            stale: Some(false),
            stale_after_days: Some(30),
            per_source: Some(vec![]),
            floor_count: Some(1),
            ceiling_count: Some(1),
            sources_no_longer_counted: Some(vec![]),
            declared_engineers: None,
            consistent_with_declared: None,
            instance_public_key: Some("key".to_string()),
        }
    }

    /// A server rename (e.g. `floor_count` -> something else) deserializes
    /// silently into `None` on this all-`Option` struct — this is the guard
    /// that turns that into a named, reported failure instead of a quietly
    /// truncated report.
    #[test]
    fn missing_ok_fields_flags_a_renamed_field() {
        let mut report = ok_attestation_response();
        report.floor_count = None;
        assert_eq!(missing_ok_fields(&report), vec!["floor_count"]);
    }

    #[test]
    fn missing_ok_fields_flags_every_absent_field_at_once() {
        let report = AttestationStatusResponse {
            state: "ok".to_string(),
            computed_at: None,
            stale: None,
            stale_after_days: None,
            per_source: None,
            floor_count: None,
            ceiling_count: None,
            sources_no_longer_counted: None,
            declared_engineers: None,
            consistent_with_declared: None,
            instance_public_key: None,
        };
        assert_eq!(
            missing_ok_fields(&report),
            vec![
                "computed_at",
                "stale",
                "stale_after_days",
                "per_source",
                "floor_count",
                "ceiling_count",
                "sources_no_longer_counted",
            ]
        );
    }

    /// `declared_engineers`/`consistent_with_declared` are legitimately
    /// `None` even in the `"ok"` state (no certificate installed) — they
    /// must never be flagged as missing.
    #[test]
    fn missing_ok_fields_is_empty_for_a_well_formed_ok_response() {
        assert!(missing_ok_fields(&ok_attestation_response()).is_empty());
    }

    /// `never_computed` legitimately omits every one of these fields — the
    /// check must be scoped to `state == "ok"` only.
    #[test]
    fn missing_ok_fields_is_empty_for_never_computed() {
        let report = AttestationStatusResponse {
            state: "never_computed".to_string(),
            computed_at: None,
            stale: None,
            stale_after_days: None,
            per_source: None,
            floor_count: None,
            ceiling_count: None,
            sources_no_longer_counted: None,
            declared_engineers: None,
            consistent_with_declared: None,
            instance_public_key: Some("key".to_string()),
        };
        assert!(missing_ok_fields(&report).is_empty());
    }
}

