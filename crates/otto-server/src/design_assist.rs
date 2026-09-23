//! Design Hall — the ONE agent-assist pipeline for every studio and format
//! (proposal §4.1). It supersedes the two per-surface pipelines for graph
//! artifacts: `mockup_assist` (Product arena attachments) and `canvas_assist`
//! (Canvas scenes) keep serving their legacy routes unchanged — they write
//! legacy storage and return legacy shapes, and the startup/admin import
//! mirrors their results into the graph as `sync` versions — while every
//! Design Hall artifact (including the imported ones) is assisted here.
//!
//! One turn:
//!   1. **materialize** the artifact's working copy `<data>/design/<id>/work/`
//!      (the head bytes; a per-format adapter picks the file the agent edits —
//!      an `otto-canvas` whiteboard exposes its inner Mermaid / D2 / Excalidraw
//!      source) plus `CONTEXT.md` (the deterministic, bounded context brief:
//!      story + acceptance criteria, uses / used-in links, the brand kit, the
//!      numbered references `[R1..Rn]`, the approved team rules and the team's
//!      `design` memories), `refs/R<n>.json` excerpts (+ `.png` thumbnails so a
//!      multimodal model can see them) and `render/current.png`;
//!   2. run ONE resumable agent turn (the shared claude-PTY session runner the
//!      legacy assists use) that edits the file IN PLACE — each valid save is
//!      broadcast live (`design_artifact_updated {change:"live"}`), an invalid
//!      one never is;
//!   3. **validate** the result for its format (otto-design's cheap checks +
//!      the server's `scene3d` schema) and **commit a new version** (author
//!      `agent`, the session id, the agent's one-line summary as the message)
//!      whose `provenance` records the references offered, the ones the agent
//!      cited (`[R1]` markers / an optional `provenance.json`, VERIFIED against
//!      the offered set — unverifiable citations are dropped and listed), the
//!      brand-kit version, the team rules in force and a prompt summary;
//!   4. record an `agent_draft` signal and link the cited references
//!      (`references`, pinned); the UI records accept / reject / edit later.
//!
//! Variants (`POST …/variants {n ≤ 4}`) run n parallel fresh turns in scratch
//! dirs and commit each on `variant/<run>/<k>` without touching the head;
//! accepting one fast-forwards main (see `otto_design::variants`). Costs stay
//! bounded: one run (turn or variants set) per artifact at a time (409), a
//! wall-clock cap per turn, n ≤ 4.
//!
//! Learning v1 (`GET /design/learned`, `POST /design/learned/extract`): the
//! deterministic extractor (`otto_design::learn`) turns repeated signals into
//! candidate rules; ready ones go to otto-improve's `design` evidence source
//! as PENDING edits of the `design-team-style` skill — approve / reject /
//! rollback stay human-only through the existing improvement-edit routes.
//!
//! Routes (registered in modules.rs; `/design/*` = Feature::Design, writes Edit):
//!   POST /api/v1/design/artifacts/{id}/assist                     (ws editor) → 202 DesignAssistTurn
//!   GET  /api/v1/design/artifacts/{id}/assist                     (ws viewer) → DesignAssistTurn[]
//!   POST /api/v1/design/artifacts/{id}/variants                   (ws editor) → 202 DesignVariantRun
//!   GET  /api/v1/design/artifacts/{id}/variants                   (ws viewer) → DesignVariantRun[]
//!   POST /api/v1/design/artifacts/{id}/variants/{version}/accept  (ws editor) → DesignVariantAcceptResp
//!   GET  /api/v1/design/learned?workspace_id=                     (ws viewer) → DesignLearnedResp
//!   POST /api/v1/design/learned/extract                           (ws editor) → DesignLearnExtractResp

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use otto_core::domain::{ImprovementEditStatus, User, Workspace, WorkspaceRole};
use otto_core::event::Event;
use otto_core::{new_id, Error, Id};
use otto_design::cite::{self, CitationReport, CitedRef, OfferedRef};
use otto_design::format::{self as dformat, Encoding};
use otto_design::learn::{self, RuleCandidate};
use otto_design::service::{bound_json, Author, SaveOpts};
use otto_design::store::ArtifactFilter;
use otto_design::uri::{DesignUri, VersionSel};
use otto_design::{
    CreateLinkReq, DesignArtifact, DesignService, DesignVersion, SignalReq, UpdateArtifactReq,
};
use otto_improve::design::{added_rules, DesignRuleProposal, DESIGN_SKILL};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, oneshot};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Live-preview file poll cadence while the agent edits.
const POLL: Duration = Duration::from_millis(900);
/// Wall-clock cap on one agent turn (cost bound); the session is killed past it.
const TURN_CAP: Duration = Duration::from_secs(20 * 60);
/// No-output idle trip for one turn.
const STUCK_AFTER: Duration = Duration::from_secs(10 * 60);
/// Non-transcript providers (codex …): this much PTY silence ends the turn.
const QUIET_DONE: Duration = Duration::from_secs(150);
/// How long `POST …/assist` waits for the session to go live before answering
/// (the turn keeps running either way; completion arrives over WS).
const READY_WAIT: Duration = Duration::from_secs(20);
/// References offered to one turn (explicit first, then links, then search).
const MAX_REFS: usize = 8;
const MAX_PROMPT_CHARS: usize = 8_000;
const MAX_SELECTION_BYTES: usize = 4 * 1024;
const MAX_BRIEF_BYTES: usize = 16 * 1024;
const MAX_REF_EXCERPT: usize = 6_000;
const MAX_FINDINGS: usize = 20;
const MAX_RULES_IN_BRIEF: usize = 20;
const MAX_MEMORIES: usize = 6;
const MAX_LINK_LINES: usize = 12;
/// Recent turns kept in memory per artifact (`GET …/assist`).
const TURN_HISTORY: usize = 20;
/// Look-back for the learning extractor (≈ the proposal's 90-day half-life).
const LEARN_LOOKBACK_DAYS: i64 = 90;
const LEARN_SIGNAL_LIMIT: i64 = 1_000;
/// Default variant directions (proposal §6.2 anti-echo-chamber: #1 follows
/// the team rules, #2 is an explicit Explore that ignores soft preferences).
const DIRECTIONS: &[(&str, &str)] = &[
    (
        "defaults",
        "Follow the team rules and the brand kit closely — the safe, on-brand take.",
    ),
    (
        "explore",
        "EXPLORE: deliberately ignore the team's soft preferences and try a bolder, unexpected direction (still valid, on-brand tokens, accessible).",
    ),
    (
        "calm",
        "A calmer, more minimal take: fewer elements, more whitespace, quieter colour.",
    ),
    (
        "story",
        "A story-driven take: stronger narrative hierarchy and a clear flow from problem to action.",
    ),
];

// ---------------------------------------------------------------------------
// Request / response bodies (mirrored in ui/src/lib/api/types.ts)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct DesignAssistReq {
    pub prompt: String,
    /// `generate` | `refine` (default) | `critique` | `a11y`.
    #[serde(default)]
    pub mode: Option<String>,
    /// The focused node/section (`{node_id?, section_id?, …}`, ≤ 4 KB JSON).
    #[serde(default)]
    pub selection: Option<Value>,
    /// Explicit references: `<artifact_id>`, `<artifact_id>@v12` or
    /// `otto://design/<id>[@v12]` (≤ 8).
    #[serde(default)]
    pub references: Vec<String>,
    /// Agent provider for a NEW assist session (a resumed one keeps its own).
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DesignVariantsReq {
    pub prompt: String,
    /// 1..=4 (default 3).
    #[serde(default = "default_variants")]
    pub n: usize,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub selection: Option<Value>,
    #[serde(default)]
    pub provider: Option<String>,
    /// Per-variant providers, cycled (e.g. `["claude","codex"]` — the
    /// Iris/Kenji personas); overrides `provider`.
    #[serde(default)]
    pub providers: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Custom direction per variant (cycled); default the built-in set.
    #[serde(default)]
    pub directions: Vec<String>,
}

fn default_variants() -> usize {
    3
}

#[derive(Debug, Default, Deserialize)]
pub struct AcceptVariantReq {
    /// Accept even though main moved since the variants were drawn.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize)]
pub struct LearnedQuery {
    pub workspace_id: Id,
}

#[derive(Debug, Deserialize)]
pub struct LearnExtractReq {
    pub workspace_id: Id,
}

/// One design-assist agent turn (main or one variant).
#[derive(Debug, Clone, Serialize)]
pub struct DesignAssistTurn {
    pub turn_id: Id,
    pub artifact_id: Id,
    pub workspace_id: Id,
    /// `generate` | `refine` | `critique` | `a11y` | `variant`.
    pub mode: String,
    /// `starting` → `running` → `done` | `unchanged` | `conflict` | `failed`.
    pub status: String,
    /// `main`, or `variant/<run>/<k>`.
    pub branch: String,
    /// Variant direction label (`defaults`, `explore`, …).
    pub direction: Option<String>,
    pub provider: String,
    pub session_id: Option<Id>,
    /// The head the turn started from.
    pub base_version_id: Option<Id>,
    /// The version it committed (`done`, or `conflict`'s side version).
    pub version_id: Option<Id>,
    /// The numbered references offered (`[R1..Rn]`).
    pub references: Vec<OfferedRef>,
    /// Verified citations.
    pub cited: Vec<CitedRef>,
    /// Citations that could not be verified (dropped from provenance).
    pub unverified_citations: Vec<String>,
    /// Keys of the approved team rules the turn was given.
    pub team_rules: Vec<String>,
    /// Critique / a11y findings (`findings.json`, ≤ 20 objects).
    pub findings: Vec<Value>,
    /// The agent's one-line summary.
    pub message: Option<String>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// One variants run: its versions (persisted) + live turn states (in memory).
#[derive(Debug, Clone, Serialize)]
pub struct DesignVariantRun {
    pub run_id: Id,
    pub artifact_id: Id,
    pub base_version_id: Option<Id>,
    /// `running` | `ready` | `accepted`.
    pub status: String,
    pub versions: Vec<DesignVersion>,
    /// The accepted VARIANT version, once one was.
    pub accepted_version_id: Option<Id>,
    pub turns: Vec<DesignAssistTurn>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesignVariantAcceptResp {
    pub artifact: DesignArtifact,
    /// The new main version (head) carrying the variant's bytes.
    pub version: DesignVersion,
    pub run_id: String,
    pub accepted_version_id: Id,
    pub rejected_version_ids: Vec<Id>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesignRuleLine {
    pub key: String,
    pub rule: String,
}

/// An ACTIVE learned rule (a line of the `design-team-style` skill).
#[derive(Debug, Clone, Serialize)]
pub struct DesignLearnedRule {
    pub key: String,
    pub rule: String,
    /// The applied improvement edit that added it (rollback target), if known.
    pub edit_id: Option<Id>,
    /// Design signal ids behind it.
    pub evidence: Vec<String>,
    pub applied_at: Option<DateTime<Utc>>,
}

/// One improvement edit of the design skill (pending or history).
#[derive(Debug, Clone, Serialize)]
pub struct DesignLearnedEdit {
    pub edit_id: Id,
    /// `pending` | `applied` | `rejected` | `rolled_back` | `conflict`.
    pub status: String,
    pub rules: Vec<DesignRuleLine>,
    pub rationale: String,
    pub evidence: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub applied_at: Option<DateTime<Utc>>,
    pub actor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesignLearnedResp {
    pub workspace_id: Id,
    /// `suggest` (default) | `off` (workspace setting `design_learning`).
    pub mode: String,
    /// The overlay skill the rules live in (`design-team-style`).
    pub skill: String,
    pub skill_path: String,
    pub active: Vec<DesignLearnedRule>,
    pub pending: Vec<DesignLearnedEdit>,
    /// Applied / rejected / rolled-back / conflicted edits, newest first (≤ 50).
    pub history: Vec<DesignLearnedEdit>,
    /// What the extractor sees right now (read-only; `ready` ones are proposed
    /// by `POST /design/learned/extract` and after a variant accept).
    pub candidates: Vec<RuleCandidate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesignLearnExtractResp {
    pub mode: String,
    pub run_id: Option<Id>,
    /// Newly proposed (pending) improvement edit ids.
    pub proposed: Vec<Id>,
    pub skipped: usize,
    pub candidates: Vec<RuleCandidate>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// The design-assist routes (paths relative to `/api/v1`). The artifact path
/// segment is `{id}` — the same name `otto_design::router` uses, so the two
/// routers merge without a matchit conflict.
pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/design/artifacts/{id}/assist",
            get(list_turns).post(start_assist),
        )
        .route(
            "/design/artifacts/{id}/variants",
            get(list_variant_runs).post(start_variants),
        )
        .route(
            "/design/artifacts/{id}/variants/{version}/accept",
            post(accept_variant),
        )
        .route("/design/learned", get(get_learned))
        .route("/design/learned/extract", post(extract_learned))
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Generate,
    Refine,
    Critique,
    A11y,
    Variant,
}

impl Mode {
    /// The `/assist` modes (`variant` is only reachable through `/variants`).
    fn parse(s: Option<&str>) -> Result<Mode, Error> {
        match s
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("refine")
        {
            "generate" => Ok(Mode::Generate),
            "refine" => Ok(Mode::Refine),
            "critique" => Ok(Mode::Critique),
            "a11y" => Ok(Mode::A11y),
            other => Err(Error::Invalid(format!(
                "mode must be generate | refine | critique | a11y, not {other:?}"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Mode::Generate => "generate",
            Mode::Refine => "refine",
            Mode::Critique => "critique",
            Mode::A11y => "a11y",
            Mode::Variant => "variant",
        }
    }

    /// Does this mode commit the agent's edit? (A critique never does.)
    fn edits(self) -> bool {
        self != Mode::Critique
    }

    fn instructions(self) -> &'static str {
        match self {
            Mode::Generate => "MODE: generate — create a fresh design for the request. You may replace the current content entirely; keep the document valid.",
            Mode::Refine => "MODE: refine — make the requested change to the current design and keep everything else intact.",
            Mode::Critique => "MODE: critique — do NOT modify the design file. Review it for hierarchy, rhythm, contrast, copy clarity, brand adherence and the team rules. Write your findings to `findings.json` as a JSON array (≤ 20) of {\"severity\":\"high|medium|low\",\"rule\":\"…\",\"message\":\"…\",\"node_id\":\"…\",\"fix\":\"…\"}.",
            Mode::A11y => "MODE: a11y — fix accessibility problems IN PLACE: text contrast ≥ 4.5:1 (3:1 for large text), alt text on images, tap targets ≥ 44 px at mobile width, a logical heading order, visible focus and reduced-motion defaults. Also list what you fixed in `findings.json` as a JSON array of {\"rule\":\"…\",\"message\":\"…\",\"node_id\":\"…\",\"fixed\":true}.",
            Mode::Variant => "MODE: variant — draw ONE distinct direction for the request (see DIRECTION below); other agents draw the other directions in parallel.",
        }
    }
}

/// Where a turn commits.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Branch {
    Main,
    Variant { run_id: Id, k: usize },
}

impl Branch {
    fn name(&self) -> String {
        match self {
            Branch::Main => "main".into(),
            Branch::Variant { run_id, k } => otto_design::variants::branch_name(run_id, *k),
        }
    }
}

/// Per-format adapter: which file the agent edits and how its content maps
/// back to the artifact's document.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Adapter {
    format: String,
    /// The file the agent edits (relative to its working dir).
    agent_file: &'static str,
    /// Fence language of a fallback source block in the reply.
    fence: &'static str,
    /// What the file holds (prompt wording).
    kind: &'static str,
    /// `otto-canvas` only: the inner diagram format the agent edits.
    canvas_inner: Option<&'static str>,
}

fn adapter_for(format: &str, doc: &[u8]) -> Result<Adapter, Error> {
    let spec = dformat::spec(format)
        .ok_or_else(|| Error::Invalid(format!("unknown design format {format:?}")))?;
    if spec.encoding == Encoding::Binary {
        return Err(Error::Invalid(format!(
            "{format} artifacts are binary and not agent-editable — upload a new version instead"
        )));
    }
    let a = |agent_file: &'static str, fence: &'static str, kind: &'static str| Adapter {
        format: format.to_string(),
        agent_file,
        fence,
        kind,
        canvas_inner: None,
    };
    Ok(match format {
        "html" => a(
            spec.file_name,
            "html",
            "ONE complete, self-contained HTML document",
        ),
        "svg" => a(spec.file_name, "svg", "ONE complete SVG document"),
        "mermaid" => a(
            spec.file_name,
            "mermaid",
            "ONE complete, valid Mermaid diagram",
        ),
        "d2" => a(spec.file_name, "d2", "ONE complete, valid D2 diagram"),
        "excalidraw" => a(
            spec.file_name,
            "json",
            "ONE complete, valid Excalidraw JSON document",
        ),
        "scene3d" => a(
            spec.file_name,
            "json",
            "ONE complete, valid `otto-scene3d` JSON document",
        ),
        "otto-canvas" => {
            let inner = serde_json::from_slice::<Value>(doc)
                .ok()
                .and_then(|v| v.get("format").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            let (file, fence, kind, inner): (
                &'static str,
                &'static str,
                &'static str,
                &'static str,
            ) = match inner.as_str() {
                "excalidraw" => (
                    "source.excalidraw.json",
                    "json",
                    "ONE complete, valid Excalidraw JSON document (a whiteboard's source)",
                    "excalidraw",
                ),
                "d2" => (
                    "source.d2",
                    "d2",
                    "ONE complete, valid D2 diagram (a whiteboard's source)",
                    "d2",
                ),
                _ => (
                    "source.mmd",
                    "mermaid",
                    "ONE complete, valid Mermaid diagram (a whiteboard's source)",
                    "mermaid",
                ),
            };
            Adapter {
                format: format.to_string(),
                agent_file: file,
                fence,
                kind,
                canvas_inner: Some(inner),
            }
        }
        _ => a(
            spec.file_name,
            "json",
            "ONE complete, valid JSON document of this format (keep its `type` and `version`)",
        ),
    })
}

impl Adapter {
    /// What the agent's file starts with for document `doc`.
    fn agent_source(&self, doc: &[u8]) -> String {
        match self.canvas_inner {
            None => String::from_utf8_lossy(doc).into_owned(),
            Some(inner) => serde_json::from_slice::<Value>(doc)
                .ok()
                .and_then(|v| v.get("source").and_then(Value::as_str).map(str::to_string))
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| canvas_base(inner).to_string()),
        }
    }

    /// The artifact document for agent source `src` (a canvas keeps every
    /// other field of `base_doc`).
    fn wrap(&self, base_doc: &[u8], src: &str) -> Result<Vec<u8>, Error> {
        match self.canvas_inner {
            None => Ok(src.as_bytes().to_vec()),
            Some(inner) => {
                if inner == "excalidraw"
                    && !serde_json::from_str::<Value>(src).is_ok_and(|v| v.is_object())
                {
                    return Err(Error::Invalid(
                        "the whiteboard's Excalidraw source is not a JSON object".into(),
                    ));
                }
                let mut doc = serde_json::from_slice::<Value>(base_doc)
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({ "type": "otto-canvas", "version": 1 }));
                doc["format"] = json!(inner);
                doc["source"] = json!(src);
                Ok(doc.to_string().into_bytes())
            }
        }
    }
}

/// The starting source of an empty whiteboard (mirrors `canvas_assist`).
fn canvas_base(inner: &str) -> &'static str {
    match inner {
        "excalidraw" => {
            "{\n  \"type\": \"excalidraw\",\n  \"version\": 2,\n  \"source\": \"otto\",\n  \"elements\": []\n}\n"
        }
        "d2" => "direction: right\n",
        _ => "flowchart TD\n",
    }
}

/// Every check a committed document passes: otto-design's cheap per-encoding
/// checks, then the server's deep ones (`scene3d` schema).
fn validate_doc(format: &str, bytes: &[u8]) -> Result<(), Error> {
    let spec = dformat::spec(format)
        .ok_or_else(|| Error::Invalid(format!("unknown design format {format:?}")))?;
    dformat::validate(spec, bytes)?;
    crate::design_hall::validate_content(format, bytes)
}

/// The contents of the first ```<lang> fenced block.
fn extract_fenced(raw: &str, lang: &str) -> Option<String> {
    let open = format!("```{lang}");
    let start = raw.find(&open)?;
    let after = &raw[start + open.len()..];
    let after = after.strip_prefix('\n').unwrap_or(after);
    let end = after.find("```")?;
    let body = after[..end].trim();
    (!body.is_empty()).then(|| body.to_string())
}

/// The agent's one-line summary: the first non-empty, non-fence line (≤ 300).
fn summary_line(reply: &str) -> Option<String> {
    reply
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("```"))
        .map(|l| l.chars().take(300).collect())
}

fn cap_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// Cut `s` to at most `max` BYTES on a char boundary, with a marker.
fn cap_bytes(mut s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let mut end = max.saturating_sub(64);
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push_str("\n\n… (context truncated at the size cap)\n");
    s
}

const STOPWORDS: &[&str] = &[
    "about", "also", "could", "from", "have", "into", "just", "like", "make", "more", "need",
    "please", "should", "some", "than", "that", "their", "them", "then", "they", "this", "those",
    "very", "want", "what", "when", "where", "which", "while", "with", "would", "your",
];

/// Up to `max` salient search terms from a prompt: words ≥ 4 chars, no stop
/// words or bare numbers, longest first (ties alphabetical) — deterministic.
fn prompt_terms(prompt: &str, max: usize) -> Vec<String> {
    let mut words: Vec<String> = prompt
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| {
            w.chars().count() >= 4
                && !STOPWORDS.contains(&w.as_str())
                && !w.chars().all(|c| c.is_ascii_digit())
        })
        .collect();
    words.sort_by(|a, b| {
        b.chars()
            .count()
            .cmp(&a.chars().count())
            .then_with(|| a.cmp(b))
    });
    words.dedup();
    words.truncate(max);
    words
}

/// `<id>`, `<id>@v12`, `otto://design/<id>[@v12]` → `(id, Some(12))`.
fn parse_ref_spec(r: &str) -> Option<(String, Option<i64>)> {
    let r = r.trim();
    let uri = if r.starts_with("otto://") {
        DesignUri::parse(r)?
    } else {
        DesignUri::parse(&format!("otto://design/{r}"))?
    };
    let seq = match uri.version {
        VersionSel::Seq(n) => Some(n),
        _ => None,
    };
    Some((uri.artifact_id, seq))
}

/// A `findings.json` body → ≤ 20 bounded objects (an array, or `{findings:[…]}`).
fn parse_findings(raw: &str) -> Vec<Value> {
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return vec![];
    };
    let arr = match v {
        Value::Array(a) => a,
        Value::Object(o) => o
            .get("findings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => vec![],
    };
    arr.into_iter()
        .filter(|x| x.is_object() && x.to_string().len() <= 2_048)
        .take(MAX_FINDINGS)
        .collect()
}

/// The workspace's design-learning mode: `off` when `settings.design_learning
/// == "off"`, else `suggest` (the default — never auto-apply).
fn learning_mode(settings: &Value) -> &'static str {
    match settings.get("design_learning").and_then(Value::as_str) {
        Some("off") => "off",
        _ => "suggest",
    }
}

/// The resumable assist session stored on the artifact (`meta.assist`), when
/// it ran on `provider` (or recorded none).
fn stored_session(meta: &Value, provider: &str) -> Option<Id> {
    let s = meta.get("assist")?;
    let sid = s
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())?;
    let p = s.get("provider").and_then(Value::as_str).unwrap_or("");
    (p.is_empty() || p == provider).then(|| sid.to_string())
}

/// Everything the context brief (`CONTEXT.md`) is rendered from.
#[derive(Debug, Default, Clone)]
struct BriefInput {
    artifact_id: String,
    title: String,
    studio: String,
    format: String,
    status: String,
    head_seq: Option<i64>,
    mode: String,
    /// `(label, excerpt)` of the story the artifact implements.
    story: Option<(String, String)>,
    uses: Vec<String>,
    used_in: Vec<String>,
    /// Header line + token excerpt.
    brand: Option<String>,
    /// Offered refs + whether a thumbnail file exists.
    refs: Vec<(OfferedRef, bool)>,
    rules: Vec<(String, String)>,
    memories: Vec<String>,
    selection: Option<Value>,
    has_render: bool,
}

fn ref_line(o: &OfferedRef) -> String {
    let v = o.seq.map(|s| format!(", v{s}")).unwrap_or_default();
    let uri = match o.seq {
        Some(s) => format!("otto://design/{}@v{s}", o.artifact_id),
        None => format!("otto://design/{}", o.artifact_id),
    };
    format!(
        "[{}] {} ({}, {}{v}) — {uri}",
        o.label,
        cap_chars(&o.title, 120),
        o.studio,
        o.status
    )
}

/// Render the deterministic, size-bounded context brief.
fn render_brief(b: &BriefInput) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "# Design context — {}\n\n",
        cap_chars(&b.title, 200)
    ));
    s.push_str(&format!(
        "Artifact: `otto://design/{}` · studio {} · format {} · status {}{}\nMode: {}\n\n",
        b.artifact_id,
        b.studio,
        b.format,
        b.status,
        b.head_seq.map(|n| format!(" · v{n}")).unwrap_or_default(),
        b.mode
    ));
    s.push_str("## Story\n");
    match &b.story {
        Some((label, excerpt)) => {
            s.push_str(&format!(
                "{}\n\n{}\n\n",
                cap_chars(label, 200),
                excerpt.trim()
            ));
        }
        None => s.push_str("(no linked story)\n\n"),
    }
    s.push_str("## Links\n");
    if b.uses.is_empty() && b.used_in.is_empty() {
        s.push_str("(none)\n");
    }
    for l in b.uses.iter().take(MAX_LINK_LINES) {
        s.push_str(&format!("- uses: {l}\n"));
    }
    for l in b.used_in.iter().take(MAX_LINK_LINES) {
        s.push_str(&format!("- used in: {l}\n"));
    }
    s.push('\n');
    s.push_str("## Brand kit\n");
    match &b.brand {
        Some(t) => s.push_str(&format!("{}\n\n", t.trim())),
        None => s.push_str("(no brand kit linked — keep to a small, consistent palette)\n\n"),
    }
    s.push_str("## References (cite what you borrow as [R1] …)\n");
    if b.refs.is_empty() {
        s.push_str("(none offered — do not cite any)\n");
    }
    for (o, png) in &b.refs {
        s.push_str(&format!(
            "{} — excerpt refs/{}.json{}\n",
            ref_line(o),
            o.label,
            if *png {
                format!(", thumbnail refs/{}.png", o.label)
            } else {
                String::new()
            }
        ));
    }
    s.push('\n');
    s.push_str("## Team rules (approved by a person)\n");
    if b.rules.is_empty() {
        s.push_str("(none yet)\n");
    }
    for (k, t) in b.rules.iter().take(MAX_RULES_IN_BRIEF) {
        s.push_str(&format!("- [{k}] {}\n", cap_chars(t, 400)));
    }
    s.push('\n');
    s.push_str("## Team memory (design)\n");
    if b.memories.is_empty() {
        s.push_str("(none)\n");
    }
    for m in b.memories.iter().take(MAX_MEMORIES) {
        s.push_str(&format!("- {}\n", cap_chars(m, 300)));
    }
    s.push('\n');
    if let Some(sel) = &b.selection {
        s.push_str("## Selection (focus the change here)\n```json\n");
        s.push_str(&cap_chars(&sel.to_string(), MAX_SELECTION_BYTES));
        s.push_str("\n```\n\n");
    }
    if b.has_render {
        s.push_str("The current render is `render/current.png`.\n");
    }
    cap_bytes(s, MAX_BRIEF_BYTES)
}

/// Everything the turn prompt is built from.
struct PromptInput<'a> {
    user_prompt: &'a str,
    mode: Mode,
    title: &'a str,
    studio: &'a str,
    adapter: &'a Adapter,
    /// `(label, text, k, n)` for a variant turn.
    direction: Option<(&'a str, &'a str, usize, usize)>,
    refs: &'a [OfferedRef],
    rules: &'a [(String, String)],
    selection: Option<&'a Value>,
}

/// Format-specific must-follow rules (the essentials; the bundled skills carry
/// the rest).
fn format_rules(a: &Adapter) -> &'static str {
    match (a.format.as_str(), a.canvas_inner) {
        ("html", _) => "- A full `<!doctype html>` page with `<meta name=viewport>`; ALL CSS inline in one `<style>`; NO external requests (no CDNs, fonts, images or scripts — system-ui fonts, CSS shapes, inline SVG).\n- It renders in a sandboxed iframe with scripts disabled — make it look right with pure HTML + CSS.\n- Realistic content from the story (never lorem ipsum); a clear hierarchy and a small cohesive palette.\n",
        ("svg", _) => "- One `<svg xmlns=…>` root, no scripts, no external references.\n",
        ("mermaid", _) | (_, Some("mermaid")) => "- Pick the best diagram type (flowchart, sequenceDiagram, classDiagram, erDiagram, stateDiagram-v2); short labels; colour via classDef at the end.\n",
        ("d2", _) | (_, Some("d2")) => "- Valid D2 only; short labels; group related nodes in containers.\n",
        ("excalidraw", _) | (_, Some("excalidraw")) => "- `{\"type\":\"excalidraw\",\"version\":2,\"elements\":[…],\"appState\":{…},\"files\":{}}`; `frame` elements as artboards; 8-pt grid; every element a unique `id`, `versionNonce`, `seed`; no images.\n",
        ("scene3d", _) => "- `type: \"otto-scene3d\", version: 1`; metres, y-up, rotation in DEGREES; objects box|sphere|cylinder|cone|torus|plane|text|gltf|group; `gltf` objects reference an existing attachment by `attachment_id` only; unique path-safe ids; ≤ 2000 objects; finite numbers.\n",
        _ => "- Keep the document's `type`/`version`; reference other artifacts only as `otto://design/<id>[@v<n>]`.\n",
    }
}

/// The bundled skill that fits the format (never modified — inlined as-is).
fn skill_for(a: &Adapter) -> Option<&'static str> {
    match a.format.as_str() {
        "scene3d" => Some("otto-design-3d"),
        "html" | "svg" | "excalidraw" | "otto-site" | "otto-layout" | "otto-brand" => {
            Some("otto-design-2d")
        }
        _ => None,
    }
}

fn build_prompt(p: &PromptInput) -> String {
    let file = p.adapter.agent_file;
    let mut s = format!(
        "OTTO_TASK: design_assist\n\
         You are Otto's design agent in Design Hall, working on \"{}\" ({} studio). You are \
         EDITING the file `{file}` in your working directory. Read it, make the change IN PLACE \
         and save it; it must always hold {} (no ``` fences inside the file).\n\n\
         {}\n\n\
         FORMAT RULES:\n{}\n\
         CONTEXT: read `CONTEXT.md` first — the story and its acceptance criteria, the links, the \
         brand kit, the numbered references and the team's approved design rules. Reference \
         excerpts are in `refs/` (R1.json …, with R1.png thumbnails when available); \
         `render/current.png` is the current render when present.\n",
        cap_chars(p.title, 200),
        p.studio,
        p.adapter.kind,
        p.mode.instructions(),
        format_rules(p.adapter),
    );
    if let Some((label, text, k, n)) = p.direction {
        s.push_str(&format!(
            "\nDIRECTION (variant {k} of {n}, \"{label}\"): {text}\n"
        ));
    }
    s.push_str("\nREFERENCES you may build on:\n");
    if p.refs.is_empty() {
        s.push_str("(none offered — do not cite any)\n");
    }
    for o in p.refs {
        s.push_str(&format!("{}\n", ref_line(o)));
    }
    s.push_str(
        "CITATIONS: when you borrow from a reference, name it inline in your summary by its \
         label in brackets (e.g. \"layout rhythm from [R2]\"). Optionally also write `provenance.json` as \
         {\"refs\":[\"R1\"],\"why\":\"<one line>\"}. Never cite a reference that is not listed \
         above — unlisted citations are discarded.\n",
    );
    if !p.rules.is_empty() {
        s.push_str("\nTEAM RULES (approved by a person — apply them unless the request says otherwise; when you do, start your summary with \"Applied your team rules: …\"):\n");
        for (k, t) in p.rules.iter().take(MAX_RULES_IN_BRIEF) {
            s.push_str(&format!("- [{k}] {}\n", cap_chars(t, 400)));
        }
    }
    if let Some(sel) = p.selection {
        s.push_str(&format!(
            "\nFOCUS: the user selected {} — change that part and leave the rest intact.\n",
            cap_chars(&sel.to_string(), 1_000)
        ));
    }
    if let Some(name) = skill_for(p.adapter) {
        if let Some(body) = otto_skills::bundled_body(name).filter(|b| !b.trim().is_empty()) {
            s.push_str(&format!("\nSKILL `{name}` — follow it:\n{}\n", body.trim()));
        }
    }
    s.push_str(&format!(
        "\nReply with ONE short sentence describing what you changed (with its [R…] citations).\n\n\
         Request: {}\n",
        cap_chars(p.user_prompt, MAX_PROMPT_CHARS)
    ));
    s
}

/// The provenance recorded on a committed version (always ≤ the 16 KB cap:
/// falls back to a minimal record rather than failing the commit).
#[allow(clippy::too_many_arguments)]
fn build_provenance(
    turn_id: &str,
    mode: Mode,
    branch: &Branch,
    direction: Option<&str>,
    provider: &str,
    session_id: Option<&str>,
    user_prompt: &str,
    offered: &[OfferedRef],
    cit: &CitationReport,
    brand: Option<&Value>,
    rule_keys: &[String],
    selection: Option<&Value>,
) -> Value {
    let assist = json!({
        "turn_id": turn_id,
        "mode": mode.as_str(),
        "branch": branch.name(),
        "direction": direction,
        "provider": provider,
        "session_id": session_id,
    });
    let full = json!({
        "assist": assist,
        "prompt_summary": cap_chars(user_prompt.trim(), 280),
        "references_offered": offered.iter().map(|o| json!({
            "label": o.label,
            "artifact_id": o.artifact_id,
            "version_id": o.version_id,
            "seq": o.seq,
            "title": cap_chars(&o.title, 120),
            "source": o.source,
        })).collect::<Vec<_>>(),
        "references_cited": cit.cited.iter().take(16).collect::<Vec<_>>(),
        "citations_unverified": cit.unverified.iter().take(16).collect::<Vec<_>>(),
        "why": cit.why,
        "brand_kit": brand,
        "team_rules": rule_keys.iter().take(MAX_RULES_IN_BRIEF).collect::<Vec<_>>(),
        "selection": selection.map(|s| cap_chars(&s.to_string(), 1_000)),
    });
    bound_json(
        full,
        otto_design::service::MAX_PROVENANCE_BYTES,
        "provenance",
    )
    .unwrap_or_else(|_| {
        json!({
            "assist": assist,
            "references_cited": cit.cited.iter().take(8).map(|c| json!({
                "label": c.label, "artifact_id": c.artifact_id, "seq": c.seq,
            })).collect::<Vec<_>>(),
            "truncated": true,
        })
    })
}

// ---------------------------------------------------------------------------
// In-memory turn registry (one run per artifact; recent turns for GET)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ArtifactRuns {
    /// What holds the artifact right now: `turn:<id>` / `variants:<run>`.
    busy: Option<String>,
    /// Recent turns, oldest first (capped).
    turns: VecDeque<DesignAssistTurn>,
}

fn registry() -> &'static Mutex<HashMap<Id, ArtifactRuns>> {
    static R: OnceLock<Mutex<HashMap<Id, ArtifactRuns>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

fn with_registry<T>(f: impl FnOnce(&mut HashMap<Id, ArtifactRuns>) -> T) -> T {
    let mut g = registry().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut *g)
}

/// Holds an artifact's single run slot; released on drop (every path).
struct BusyGuard(Id);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        with_registry(|m| {
            if let Some(r) = m.get_mut(&self.0) {
                r.busy = None;
            }
        });
    }
}

fn try_busy(artifact_id: &str, what: String) -> Result<BusyGuard, Error> {
    with_registry(|m| {
        let r = m.entry(artifact_id.to_string()).or_default();
        if let Some(b) = &r.busy {
            return Err(Error::Conflict(format!(
                "an agent is already working on design artifact {artifact_id} ({b}); wait for it to finish"
            )));
        }
        r.busy = Some(what);
        Ok(BusyGuard(artifact_id.to_string()))
    })
}

fn busy_of(artifact_id: &str) -> Option<String> {
    with_registry(|m| m.get(artifact_id).and_then(|r| r.busy.clone()))
}

fn put_turn(t: DesignAssistTurn) {
    with_registry(|m| {
        let r = m.entry(t.artifact_id.clone()).or_default();
        match r.turns.iter().position(|x| x.turn_id == t.turn_id) {
            Some(i) => r.turns[i] = t,
            None => {
                r.turns.push_back(t);
                while r.turns.len() > TURN_HISTORY {
                    r.turns.pop_front();
                }
            }
        }
    });
}

fn update_turn(
    artifact_id: &str,
    turn_id: &str,
    f: impl FnOnce(&mut DesignAssistTurn),
) -> Option<DesignAssistTurn> {
    with_registry(|m| {
        let t = m
            .get_mut(artifact_id)?
            .turns
            .iter_mut()
            .find(|x| x.turn_id == turn_id)?;
        f(&mut *t);
        Some(t.clone())
    })
}

fn get_turn(artifact_id: &str, turn_id: &str) -> Option<DesignAssistTurn> {
    with_registry(|m| {
        m.get(artifact_id)?
            .turns
            .iter()
            .find(|x| x.turn_id == turn_id)
            .cloned()
    })
}

/// Recent turns, newest first.
fn turns_for(artifact_id: &str) -> Vec<DesignAssistTurn> {
    with_registry(|m| {
        m.get(artifact_id)
            .map(|r| r.turns.iter().rev().cloned().collect())
            .unwrap_or_default()
    })
}

fn emit_turn(events: &broadcast::Sender<Event>, t: &DesignAssistTurn) {
    let _ = events.send(Event::DesignAssistUpdated {
        workspace_id: t.workspace_id.clone(),
        artifact_id: t.artifact_id.clone(),
        turn_id: t.turn_id.clone(),
        status: t.status.clone(),
        mode: t.mode.clone(),
        branch: t.branch.clone(),
        session_id: t.session_id.clone(),
        version_id: t.version_id.clone(),
        error: t.error.clone(),
    });
}

// ---------------------------------------------------------------------------
// Context assembly
// ---------------------------------------------------------------------------

/// The context shared by every turn of one request.
struct Assembled {
    brief: BriefInput,
    offered: Vec<OfferedRef>,
    /// `(label, excerpt json, thumbnail source path)`.
    ref_files: Vec<(String, String, Option<PathBuf>)>,
    rules: Vec<(String, String)>,
    brand: Option<Value>,
    render_src: Option<PathBuf>,
}

/// Caches "may this user view workspace X" for one request.
struct ViewCheck<'a> {
    ctx: &'a ServerCtx,
    user: &'a User,
    cache: HashMap<Id, bool>,
}

impl<'a> ViewCheck<'a> {
    fn new(ctx: &'a ServerCtx, user: &'a User) -> Self {
        Self {
            ctx,
            user,
            cache: HashMap::new(),
        }
    }

    async fn can(&mut self, ws: &str) -> bool {
        if let Some(v) = self.cache.get(ws) {
            return *v;
        }
        let ok = self
            .ctx
            .roles
            .check(self.user, &ws.to_string(), WorkspaceRole::Viewer)
            .await
            .is_ok();
        self.cache.insert(ws.to_string(), ok);
        ok
    }
}

/// The version a reference offers: an explicit `@vN`, else approved, else head.
async fn offered_version(
    svc: &DesignService,
    a: &DesignArtifact,
    seq: Option<i64>,
    pinned: Option<&str>,
) -> Result<Option<DesignVersion>, Error> {
    if let Some(n) = seq {
        return svc.store().get_version_by_seq(&a.id, n).await;
    }
    if let Some(p) = pinned {
        if let Some(v) = svc.store().get_version(p).await? {
            if v.artifact_id == a.id {
                return Ok(Some(v));
            }
        }
    }
    match a
        .approved_version_id
        .as_deref()
        .or(a.head_version_id.as_deref())
    {
        Some(id) => svc.store().get_version(id).await,
        None => Ok(None),
    }
}

/// A bounded excerpt of a version for `refs/R<n>.json`.
async fn ref_excerpt(svc: &DesignService, a: &DesignArtifact, v: &DesignVersion) -> String {
    let Ok(bytes) = svc.version_bytes(v).await else {
        return String::new();
    };
    let binary = dformat::spec(&a.format).is_none_or(|s| s.encoding == Encoding::Binary);
    if binary {
        return String::new();
    }
    if bytes.len() <= MAX_REF_EXCERPT {
        return String::from_utf8_lossy(&bytes).into_owned();
    }
    cap_chars(
        &otto_design::extract::extract(&a.format, &bytes).text,
        MAX_REF_EXCERPT,
    )
}

fn thumb_path(svc: &DesignService, a: &DesignArtifact) -> Option<PathBuf> {
    a.thumb_blob
        .as_deref()
        .filter(|s| otto_design::blobs::is_sha(s))
        .map(|s| svc.blobs().root().join(s))
        .filter(|p| p.is_file())
}

/// Build the whole context for a turn on `a` (deterministic for a given graph
/// state). Explicit references that don't exist / aren't visible are errors;
/// everything else degrades to "none".
#[allow(clippy::too_many_arguments)]
async fn assemble(
    ctx: &ServerCtx,
    svc: &DesignService,
    user: &User,
    a: &DesignArtifact,
    explicit: &[String],
    prompt: &str,
    selection: Option<&Value>,
    mode: Mode,
) -> Result<Assembled, Error> {
    if explicit.len() > MAX_REFS {
        return Err(Error::Invalid(format!("at most {MAX_REFS} references")));
    }
    let mut view = ViewCheck::new(ctx, user);
    let mut picked: Vec<(DesignArtifact, Option<DesignVersion>, &'static str)> = Vec::new();
    let mut seen: HashSet<Id> = HashSet::from([a.id.clone()]);

    // 1. Explicit references (the request's).
    for r in explicit {
        let (id, seq) = parse_ref_spec(r)
            .ok_or_else(|| Error::Invalid(format!("reference {r:?} is not an artifact id")))?;
        let t = svc.store().require_artifact(&id).await?;
        if !view.can(&t.workspace_id).await {
            return Err(Error::Forbidden(format!("no access to reference {id}")));
        }
        let v = offered_version(svc, &t, seq, None).await?;
        if seq.is_some() && v.is_none() {
            return Err(Error::NotFound(format!("version {r}")));
        }
        if seen.insert(t.id.clone()) {
            picked.push((t, v, "explicit"));
        }
    }

    // 2. Explicit `references` / `derived_from` links, and the link panels.
    let links_out = svc.store().links_out(&a.id).await.unwrap_or_default();
    let links_in = svc.store().links_in(&a.id).await.unwrap_or_default();
    let mut ids: Vec<String> = links_out
        .iter()
        .filter(|l| l.dst_kind == "artifact")
        .map(|l| l.dst_id.clone())
        .chain(links_in.iter().map(|l| l.src_artifact_id.clone()))
        .collect();
    ids.sort();
    ids.dedup();
    let others: HashMap<Id, DesignArtifact> = svc
        .store()
        .artifacts_by_ids(&ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|x| (x.id.clone(), x))
        .collect();
    let mut uses = Vec::new();
    let mut used_in = Vec::new();
    for l in &links_out {
        let is_artifact = l.dst_kind == "artifact";
        let target = if is_artifact {
            others.get(&l.dst_id)
        } else {
            None
        };
        let visible = match target {
            Some(t) => view.can(&t.workspace_id).await,
            None => false,
        };
        let line = match target {
            Some(t) if visible => format!(
                "{} → \"{}\" ({}, {}) otto://design/{}{}",
                l.rel,
                cap_chars(&t.title, 120),
                t.studio,
                t.format,
                t.id,
                if l.broken { " [broken]" } else { "" }
            ),
            // An artifact the caller can't see (or a dangling id) stays out.
            _ if is_artifact => continue,
            _ => format!("{} → {} {}", l.rel, l.dst_kind, cap_chars(&l.dst_id, 120)),
        };
        uses.push(line);
        if matches!(l.rel.as_str(), "references" | "derived_from")
            && !l.broken
            && picked.len() < MAX_REFS
        {
            if let Some(t) = target {
                if seen.contains(&t.id) {
                    continue;
                }
                let v = offered_version(svc, t, None, l.pinned_version_id.as_deref()).await?;
                seen.insert(t.id.clone());
                picked.push((t.clone(), v, "link"));
            }
        }
    }
    for l in &links_in {
        if let Some(src) = others.get(&l.src_artifact_id) {
            if view.can(&src.workspace_id).await {
                used_in.push(format!(
                    "\"{}\" ({}) {} this",
                    cap_chars(&src.title, 120),
                    src.studio,
                    l.rel
                ));
            }
        }
    }

    // 3. Library search (same workspace; shipped first): the title, then the
    //    prompt's salient terms one at a time (FTS ANDs terms, so one query
    //    per term finds more).
    if picked.len() < MAX_REFS {
        let filter = ArtifactFilter {
            workspaces: Some(vec![a.workspace_id.clone()]),
            limit: 6,
            ..Default::default()
        };
        let mut queries = vec![a.title.clone()];
        queries.extend(prompt_terms(prompt, 3));
        'search: for q in queries {
            for (t, _snip, _score) in svc.store().search(&q, &filter).await.unwrap_or_default() {
                if picked.len() >= MAX_REFS {
                    break 'search;
                }
                if t.head_version_id.is_none() || seen.contains(&t.id) {
                    continue;
                }
                let v = offered_version(svc, &t, None, None).await?;
                seen.insert(t.id.clone());
                picked.push((t, v, "search"));
            }
        }
    }

    let mut offered = Vec::new();
    let mut ref_files = Vec::new();
    let mut brief_refs = Vec::new();
    for (i, (t, v, source)) in picked.into_iter().enumerate() {
        let label = format!("R{}", i + 1);
        let o = OfferedRef {
            label: label.clone(),
            artifact_id: t.id.clone(),
            version_id: v.as_ref().map(|v| v.id.clone()),
            seq: v.as_ref().map(|v| v.seq),
            title: t.title.clone(),
            studio: t.studio.clone(),
            format: t.format.clone(),
            status: t.status.clone(),
            source: source.to_string(),
        };
        let excerpt = match &v {
            Some(v) => ref_excerpt(svc, &t, v).await,
            None => String::new(),
        };
        let uri = match o.seq {
            Some(s) => format!("otto://design/{}@v{s}", t.id),
            None => format!("otto://design/{}", t.id),
        };
        let file = json!({
            "label": label,
            "uri": uri,
            "title": t.title,
            "studio": t.studio,
            "format": t.format,
            "status": t.status,
            "tags": t.tags,
            "excerpt": excerpt,
        });
        let thumb = thumb_path(svc, &t);
        brief_refs.push((o.clone(), thumb.is_some()));
        ref_files.push((
            label,
            serde_json::to_string_pretty(&file).unwrap_or_default(),
            thumb,
        ));
        offered.push(o);
    }

    // Story (acceptance criteria live in the imported source body).
    let mut story = None;
    for sid in svc.store().story_ids_for(&a.id).await.unwrap_or_default() {
        let Ok(st) = ctx.product_repo.get_story(&sid).await else {
            continue;
        };
        if !view.can(&st.workspace_id).await {
            continue;
        }
        let body = ctx
            .product_repo
            .latest_source_version(&st.id)
            .await
            .ok()
            .flatten()
            .map(|v| cap_chars(v.body_md.trim(), 2_500))
            .unwrap_or_default();
        story = Some((format!("{} {}", st.source_key, st.title), body));
        break;
    }

    // Brand kit: the project's, else a `uses_tokens` link.
    let mut brand_art: Option<DesignArtifact> = None;
    if let Some(p) = &a.project_id {
        if let Ok(Some(proj)) = svc.store().get_project(p).await {
            if let Some(b) = &proj.brand_kit_id {
                brand_art = svc.store().get_artifact(b).await.ok().flatten();
            }
        }
    }
    if brand_art.is_none() {
        if let Some(l) = links_out
            .iter()
            .find(|l| l.rel == "uses_tokens" && l.dst_kind == "artifact" && !l.broken)
        {
            brand_art = others.get(&l.dst_id).cloned();
        }
    }
    let mut brand_line = None;
    let mut brand = None;
    if let Some(b) = brand_art {
        if view.can(&b.workspace_id).await {
            if let Ok(Some(v)) = offered_version(svc, &b, None, None).await {
                let tokens = ref_excerpt(svc, &b, &v).await;
                brand_line = Some(format!(
                    "\"{}\" v{} (otto://design/{}@v{}):\n{}",
                    cap_chars(&b.title, 120),
                    v.seq,
                    b.id,
                    v.seq,
                    cap_chars(&tokens, 3_000)
                ));
                brand = Some(json!({ "artifact_id": b.id, "version_id": v.id, "seq": v.seq }));
            }
        }
    }

    // Team rules (approved) + the `design` memory collection.
    let rules = ctx
        .improve_engine
        .design_rules_active(&a.workspace_id)
        .await
        .unwrap_or_default();
    let mut memories = Vec::new();
    let text = {
        let mut t = prompt_terms(prompt, 3);
        t.extend(prompt_terms(&a.title, 2));
        t.join(" ")
    };
    for q in [Some(text).filter(|t| !t.is_empty()), None] {
        let hits = ctx
            .memory
            .search(
                &a.workspace_id,
                otto_memory::MemoryQuery {
                    text: q,
                    collection: Some("design".into()),
                    k: MAX_MEMORIES,
                    mode: otto_memory::SearchMode::Keyword,
                    viewer: Some(user.id.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_or_default();
        for h in hits {
            let line = if h.memory.title.trim().is_empty() {
                h.memory.body.clone()
            } else {
                format!("{}: {}", h.memory.title.trim(), h.memory.body.trim())
            };
            let line = line.replace('\n', " ");
            if !memories.contains(&line) && memories.len() < MAX_MEMORIES {
                memories.push(line);
            }
        }
        if !memories.is_empty() {
            break;
        }
    }

    let render_src = thumb_path(svc, a);
    let brief = BriefInput {
        artifact_id: a.id.clone(),
        title: a.title.clone(),
        studio: a.studio.clone(),
        format: a.format.clone(),
        status: a.status.clone(),
        head_seq: a.head_seq,
        mode: mode.as_str().to_string(),
        story,
        uses,
        used_in,
        brand: brand_line,
        refs: brief_refs,
        rules: rules.clone(),
        memories,
        selection: selection.cloned(),
        has_render: render_src.is_some(),
    };
    Ok(Assembled {
        brief,
        offered,
        ref_files,
        rules,
        brand,
        render_src,
    })
}

/// Write the context files into a turn directory (CONTEXT.md, refs/,
/// render/current.png) and clear the previous turn's outputs.
async fn write_context(dir: &FsPath, ctxin: &Assembled) -> Result<(), Error> {
    let io = |e: std::io::Error| Error::Internal(format!("design assist dir: {e}"));
    tokio::fs::create_dir_all(dir).await.map_err(io)?;
    for stale in ["provenance.json", "findings.json"] {
        let _ = tokio::fs::remove_file(dir.join(stale)).await;
    }
    for sub in ["refs", "render"] {
        let _ = tokio::fs::remove_dir_all(dir.join(sub)).await;
    }
    tokio::fs::write(dir.join("CONTEXT.md"), render_brief(&ctxin.brief))
        .await
        .map_err(io)?;
    if !ctxin.ref_files.is_empty() {
        let refs = dir.join("refs");
        tokio::fs::create_dir_all(&refs).await.map_err(io)?;
        for (label, body, thumb) in &ctxin.ref_files {
            tokio::fs::write(refs.join(format!("{label}.json")), body)
                .await
                .map_err(io)?;
            if let Some(t) = thumb {
                let _ = tokio::fs::copy(t, refs.join(format!("{label}.png"))).await;
            }
        }
    }
    if let Some(src) = &ctxin.render_src {
        let render = dir.join("render");
        tokio::fs::create_dir_all(&render).await.map_err(io)?;
        let _ = tokio::fs::copy(src, render.join("current.png")).await;
    }
    Ok(())
}

/// Read a file the agent may have written, capped at the content limit.
async fn read_capped(path: &FsPath, cap: usize) -> Option<Vec<u8>> {
    let len = tokio::fs::metadata(path).await.ok()?.len();
    if len > cap as u64 {
        tracing::warn!(
            "design assist: {} exceeds {cap} bytes; ignored",
            path.display()
        );
        return None;
    }
    tokio::fs::read(path).await.ok()
}

/// Put the base document on disk for a turn in `dir`: the working copy (main
/// turns — a diverging, never-committed working copy is backed up first, the
/// blob store being the truth) and the file the agent edits.
async fn materialize(
    dir: &FsPath,
    adapter: &Adapter,
    work_file: Option<&FsPath>,
    base_doc: &[u8],
    turn_id: &str,
) -> Result<PathBuf, Error> {
    let io = |e: std::io::Error| Error::Internal(format!("design assist working copy: {e}"));
    tokio::fs::create_dir_all(dir).await.map_err(io)?;
    if let Some(wf) = work_file {
        if let Ok(existing) = tokio::fs::read(wf).await {
            if existing != base_doc {
                let short: String = turn_id.chars().take(8).collect();
                let name = wf
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "work".into());
                let _ =
                    tokio::fs::rename(wf, dir.join(format!("{name}.pre-assist-{short}.bak"))).await;
            }
        }
        tokio::fs::write(wf, base_doc).await.map_err(io)?;
    }
    let agent_path = dir.join(adapter.agent_file);
    if adapter.canvas_inner.is_some() || work_file.is_none() {
        tokio::fs::write(&agent_path, adapter.agent_source(base_doc))
            .await
            .map_err(io)?;
    }
    Ok(agent_path)
}

// ---------------------------------------------------------------------------
// The turn
// ---------------------------------------------------------------------------

/// Everything one spawned turn owns.
struct TurnJob {
    ctx: ServerCtx,
    ws: Workspace,
    user: User,
    artifact: DesignArtifact,
    turn_id: Id,
    mode: Mode,
    branch: Branch,
    direction: Option<String>,
    provider: String,
    model: Option<String>,
    existing_session: Option<Id>,
    adapter: Adapter,
    dir: PathBuf,
    agent_path: PathBuf,
    /// The artifact's working copy (main turns) — restored on failure.
    work_file: Option<PathBuf>,
    base_doc: Vec<u8>,
    base_source: String,
    base_version_id: Option<Id>,
    prompt: String,
    user_prompt: String,
    offered: Vec<OfferedRef>,
    rule_keys: Vec<String>,
    brand: Option<Value>,
    selection: Option<Value>,
    ready_tx: Option<oneshot::Sender<Id>>,
}

fn done_marker() -> (PathBuf, String) {
    let path = std::env::temp_dir().join(format!("otto-design-done-{}.txt", new_id()));
    let _ = std::fs::write(&path, "");
    let line = format!(
        "\nLAST OF ALL — strictly after every other step above is complete — write your ONE-LINE \
         summary (with its [R…] citations) as plain text to this exact filesystem path: `{}`. \
         Writing this file ends your turn; never write it early.\n",
        path.display()
    );
    (path, line)
}

/// Broadcast each valid change of the agent's file while a MAIN turn runs.
fn spawn_live_poll(job: &TurnJob) -> tokio::task::JoinHandle<()> {
    let events = job.ctx.events.clone();
    let path = job.agent_path.clone();
    let adapter = job.adapter.clone();
    let base_doc = job.base_doc.clone();
    let ws = job.artifact.workspace_id.clone();
    let aid = job.artifact.id.clone();
    let mut last = job.base_source.clone().into_bytes();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(POLL).await;
            let Some(bytes) = read_capped(&path, dformat::MAX_CONTENT_BYTES).await else {
                continue;
            };
            if bytes == last || bytes.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            last = bytes.clone();
            let Ok(src) = String::from_utf8(bytes) else {
                continue;
            };
            let Ok(doc) = adapter.wrap(&base_doc, &src) else {
                continue;
            };
            // A half-written / invalid document is never pushed to viewers.
            if validate_doc(&adapter.format, &doc).is_err() {
                continue;
            }
            let content =
                dformat::spec(&adapter.format).and_then(|s| dformat::event_content(s, &doc));
            let _ = events.send(Event::DesignArtifactUpdated {
                workspace_id: ws.clone(),
                artifact_id: aid.clone(),
                format: adapter.format.clone(),
                change: "live".into(),
                version_id: None,
                content,
            });
        }
    })
}

/// Restore the on-disk copies of a MAIN turn to the base document (after a
/// failed / invalid / critique turn), so the working copy keeps mirroring
/// the head.
async fn restore_base(job: &TurnJob) {
    if job.branch != Branch::Main {
        return;
    }
    if let Some(wf) = &job.work_file {
        let _ = tokio::fs::write(wf, &job.base_doc).await;
    }
    if job.adapter.canvas_inner.is_some() {
        let _ = tokio::fs::write(&job.agent_path, &job.base_source).await;
    }
}

/// Run one turn to completion and return its final state (also stored in the
/// registry and broadcast as `design_assist_updated`).
async fn run_job(mut job: TurnJob) -> DesignAssistTurn {
    let ctx = job.ctx.clone();
    let svc = crate::design_hall::service(&ctx);
    let aid = job.artifact.id.clone();
    let tid = job.turn_id.clone();

    let (done_path, done_line) = done_marker();
    let prompt = format!("{}{done_line}", job.prompt);
    let poll = (job.branch == Branch::Main && job.mode.edits()).then(|| spawn_live_poll(&job));

    let sid_cell: Arc<Mutex<Option<Id>>> = Arc::new(Mutex::new(None));
    let on_ready = {
        let events = ctx.events.clone();
        let cell = Arc::clone(&sid_cell);
        let aid = aid.clone();
        let tid = tid.clone();
        let ready_tx = job.ready_tx.take();
        move |sid: &Id| {
            *cell.lock().unwrap_or_else(|e| e.into_inner()) = Some(sid.clone());
            if let Some(t) = update_turn(&aid, &tid, |t| {
                t.session_id = Some(sid.clone());
                t.status = "running".into();
            }) {
                emit_turn(&events, &t);
            }
            if let Some(tx) = ready_tx {
                let _ = tx.send(sid.clone());
            }
        }
    };
    let mut meta = json!({
        "source": "design_assist",
        "artifact_id": aid,
        "turn_id": tid,
        "mode": job.mode.as_str(),
        "branch": job.branch.name(),
    });
    if let Some(m) = job
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        meta["model"] = json!(m);
    }
    let title = match &job.branch {
        Branch::Main => format!("Design: {}", job.artifact.title),
        Branch::Variant { k, .. } => format!("Design variant {k}: {}", job.artifact.title),
    };
    let dir_str = job.dir.to_string_lossy().to_string();
    let turn = tokio::time::timeout(
        TURN_CAP,
        crate::agent_session::run_session_turn_with(
            &ctx,
            &job.ws,
            &job.user,
            job.existing_session.as_ref(),
            &title,
            &dir_str,
            &job.provider,
            meta,
            &prompt,
            STUCK_AFTER,
            crate::agent_session::TurnOpts {
                done_file: Some(done_path.clone()),
                quiet_done: Some(QUIET_DONE),
                ..Default::default()
            },
            on_ready,
        ),
    )
    .await;
    if let Some(p) = poll {
        p.abort();
    }
    let _ = std::fs::remove_file(done_path);
    let known_sid: Option<Id> = { sid_cell.lock().unwrap_or_else(|e| e.into_inner()).clone() };

    let result: Result<(String, Id), String> = match turn {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e.0.to_string()),
        Err(_) => {
            if let Some(s) = &known_sid {
                let _ = ctx.manager.kill_session(s).await;
            }
            Err(format!(
                "the agent turn exceeded {} minutes and was stopped",
                TURN_CAP.as_secs() / 60
            ))
        }
    };
    let sid = result
        .as_ref()
        .ok()
        .map(|(_, s)| s.clone())
        .or(known_sid.clone());

    // Keep the main assist session resumable (success or not).
    if job.branch == Branch::Main {
        if let Some(s) = &sid {
            let _ = svc
                .update_meta(
                    &aid,
                    UpdateArtifactReq {
                        meta: Some(
                            json!({ "assist": { "session_id": s, "provider": job.provider } }),
                        ),
                        ..Default::default()
                    },
                    &Author::user(&job.user.id),
                )
                .await;
        }
    }

    let outcome = match result {
        Ok((reply, sid)) => finalize(&svc, &job, &reply, &sid).await,
        Err(e) => {
            restore_base(&job).await;
            Outcome::failed(e)
        }
    };
    let final_turn = update_turn(&aid, &tid, |t| {
        t.status = outcome.status.to_string();
        t.session_id = sid.clone().or(t.session_id.clone());
        t.version_id = outcome.version_id.clone();
        t.cited = outcome.cit.cited.clone();
        t.unverified_citations = outcome.cit.unverified.clone();
        t.findings = outcome.findings.clone();
        t.message = outcome.message.clone();
        t.error = outcome.error.clone();
        t.finished_at = Some(Utc::now());
    });
    match final_turn {
        Some(t) => {
            emit_turn(&ctx.events, &t);
            t
        }
        None => {
            // Evicted from the history (very busy artifact) — still report.
            let t = DesignAssistTurn {
                turn_id: tid,
                artifact_id: aid,
                workspace_id: job.artifact.workspace_id.clone(),
                mode: job.mode.as_str().into(),
                status: outcome.status.to_string(),
                branch: job.branch.name(),
                direction: job.direction.clone(),
                provider: job.provider.clone(),
                session_id: sid,
                base_version_id: job.base_version_id.clone(),
                version_id: outcome.version_id,
                references: job.offered.clone(),
                cited: outcome.cit.cited,
                unverified_citations: outcome.cit.unverified,
                team_rules: job.rule_keys.clone(),
                findings: outcome.findings,
                message: outcome.message,
                error: outcome.error,
                started_at: Utc::now(),
                finished_at: Some(Utc::now()),
            };
            emit_turn(&ctx.events, &t);
            t
        }
    }
}

/// How a finished turn ended.
#[derive(Default)]
struct Outcome {
    status: &'static str,
    version_id: Option<Id>,
    cit: CitationReport,
    findings: Vec<Value>,
    message: Option<String>,
    error: Option<String>,
}

impl Outcome {
    fn failed(e: impl Into<String>) -> Self {
        Self {
            status: "failed",
            error: Some(e.into()),
            ..Default::default()
        }
    }
}

/// Read the agent's result, validate it and commit it (main head, a variant
/// branch, or — when a human saved meanwhile — a side version).
async fn finalize(svc: &DesignService, job: &TurnJob, reply: &str, sid: &Id) -> Outcome {
    let findings = match read_capped(&job.dir.join("findings.json"), 64 * 1024).await {
        Some(b) => parse_findings(&String::from_utf8_lossy(&b)),
        None => vec![],
    };
    let prov_file: Option<Value> = read_capped(&job.dir.join("provenance.json"), 16 * 1024)
        .await
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(Value::is_object);
    let cit = cite::verify(&job.offered, reply, prov_file.as_ref());
    let message = summary_line(reply);
    let mut out = Outcome {
        status: "unchanged",
        cit,
        findings,
        message: message.clone(),
        ..Default::default()
    };

    if !job.mode.edits() {
        restore_base(job).await;
        return out;
    }

    // The committed source: the agent's in-place edit, else a fenced block in
    // the reply (an agent that printed instead of editing / the E2E stub).
    let after = read_capped(&job.agent_path, dformat::MAX_CONTENT_BYTES).await;
    let src: Option<String> = match after {
        Some(b) if b != job.base_source.as_bytes() && !b.iter().all(u8::is_ascii_whitespace) => {
            match String::from_utf8(b) {
                Ok(s) => Some(s),
                Err(_) => {
                    restore_base(job).await;
                    return Outcome {
                        status: "failed",
                        error: Some("the agent wrote a non-UTF-8 file".into()),
                        ..out
                    };
                }
            }
        }
        _ => extract_fenced(reply, job.adapter.fence),
    };
    let Some(src) = src else {
        return out; // nothing changed
    };
    let doc = match job
        .adapter
        .wrap(&job.base_doc, &src)
        .and_then(|d| validate_doc(&job.adapter.format, &d).map(|_| d))
    {
        Ok(d) => d,
        Err(e) => {
            restore_base(job).await;
            return Outcome {
                status: "failed",
                error: Some(format!(
                    "the agent produced an invalid {} document — nothing committed: {e}",
                    job.adapter.format
                )),
                ..out
            };
        }
    };

    let author = Author {
        kind: "agent".into(),
        id: job.user.id.clone(),
        session_id: Some(sid.clone()),
    };
    let provenance = build_provenance(
        &job.turn_id,
        job.mode,
        &job.branch,
        job.direction.as_deref(),
        &job.provider,
        Some(sid.as_str()),
        &job.user_prompt,
        &job.offered,
        &out.cit,
        job.brand.as_ref(),
        &job.rule_keys,
        job.selection.as_ref(),
    );
    let commit_msg = message
        .clone()
        .unwrap_or_else(|| format!("Design assist ({})", job.mode.as_str()));

    let committed: Result<(DesignVersion, bool), Error> = match &job.branch {
        Branch::Variant { run_id, k } => svc
            .commit_variant(
                &job.artifact,
                doc,
                run_id,
                *k,
                job.base_version_id.as_deref(),
                author.clone(),
                commit_msg,
                provenance,
            )
            .await
            .map(|v| (v, true)),
        Branch::Main => {
            let fresh = match svc.store().require_artifact(&job.artifact.id).await {
                Ok(a) => a,
                Err(e) => return Outcome::failed(e.to_string()),
            };
            let res = svc
                .commit_bytes(
                    &fresh,
                    doc.clone(),
                    SaveOpts {
                        base: Some(job.base_version_id.clone().unwrap_or_default()),
                        kind: "agent".into(),
                        author: author.clone(),
                        message: commit_msg.clone(),
                        provenance: provenance.clone(),
                        force: false,
                        validate: true,
                        change: "content",
                    },
                )
                .await;
            match res {
                Ok(saved) => Ok((saved.version, saved.created)),
                Err(Error::Conflict(_)) => {
                    // A human (or another writer) saved while the agent worked:
                    // never clobber — keep the draft as a side version they can
                    // compare and accept (`POST …/variants/{v}/accept`).
                    out.status = "conflict";
                    out.error = Some(
                        "the design changed while the agent worked — its draft was kept as a \
                         variant instead of replacing your edits"
                            .into(),
                    );
                    svc.commit_variant(
                        &fresh,
                        doc,
                        &job.turn_id,
                        1,
                        job.base_version_id.as_deref(),
                        author.clone(),
                        commit_msg,
                        provenance,
                    )
                    .await
                    .map(|v| (v, true))
                }
                Err(e) => Err(e),
            }
        }
    };
    let (version, created) = match committed {
        Ok(v) => v,
        Err(e) => {
            restore_base(job).await;
            return Outcome {
                status: "failed",
                error: Some(e.to_string()),
                ..out
            };
        }
    };
    if !created {
        return out; // byte-identical to the head
    }
    if out.status != "conflict" {
        out.status = "done";
    }
    out.version_id = Some(version.id.clone());

    // Learning signal + provenance links (best-effort).
    let payload = json!({
        "turn_id": job.turn_id,
        "mode": job.mode.as_str(),
        "branch": version.branch,
        "direction": job.direction,
        "provider": job.provider,
        "cited": out.cit.cited.iter().map(|c| c.artifact_id.clone()).collect::<Vec<_>>(),
        "offered": job.offered.len(),
        "unverified": out.cit.unverified.len(),
    });
    let _ = svc
        .record_signal(
            &job.artifact,
            SignalReq {
                artifact_id: job.artifact.id.clone(),
                kind: "agent_draft".into(),
                version_id: Some(version.id.clone()),
                actor_kind: Some("agent".into()),
                session_id: Some(sid.clone()),
                payload: Some(payload),
            },
            &author,
        )
        .await;
    if version.branch == "main" {
        for c in &out.cit.cited {
            let _ = svc
                .create_link(
                    &job.artifact,
                    CreateLinkReq {
                        rel: "references".into(),
                        dst_kind: "artifact".into(),
                        dst_id: c.artifact_id.clone(),
                        dst_node: None,
                        src_node: None,
                        policy: Some("pinned".into()),
                        pinned_version_id: c.version_id.clone(),
                        meta: Some(json!({
                            "via": "design_assist",
                            "label": c.label,
                            "cited_in_version": version.id,
                        })),
                    },
                    &author,
                )
                .await;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn clean_prompt(p: &str) -> Result<String, Error> {
    let p = p.trim();
    if p.is_empty() {
        return Err(Error::Invalid("prompt must not be empty".into()));
    }
    Ok(p.chars().take(MAX_PROMPT_CHARS).collect())
}

fn clean_selection(sel: Option<Value>) -> Result<Option<Value>, Error> {
    match sel {
        None | Some(Value::Null) => Ok(None),
        Some(v) => bound_json(v, MAX_SELECTION_BYTES, "selection").map(Some),
    }
}

async fn pick_provider(ctx: &ServerCtx, ws: &Workspace, requested: Option<&str>) -> String {
    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    otto_core::provider::resolve_provider(&[
        requested.unwrap_or(""),
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ])
}

async fn load_editable(
    ctx: &ServerCtx,
    svc: &DesignService,
    user: &User,
    id: &str,
    role: WorkspaceRole,
) -> ApiResult<DesignArtifact> {
    let a = svc.store().require_artifact(id).await?;
    crate::auth::require_ws_role(ctx, user, &a.workspace_id, role).await?;
    Ok(a)
}

#[allow(clippy::too_many_arguments)]
fn new_turn(
    turn_id: &str,
    a: &DesignArtifact,
    mode: Mode,
    branch: &Branch,
    direction: Option<String>,
    provider: &str,
    base_version_id: Option<Id>,
    offered: &[OfferedRef],
    rules: &[(String, String)],
) -> DesignAssistTurn {
    DesignAssistTurn {
        turn_id: turn_id.to_string(),
        artifact_id: a.id.clone(),
        workspace_id: a.workspace_id.clone(),
        mode: mode.as_str().into(),
        status: "starting".into(),
        branch: branch.name(),
        direction,
        provider: provider.to_string(),
        session_id: None,
        base_version_id,
        version_id: None,
        references: offered.to_vec(),
        cited: vec![],
        unverified_citations: vec![],
        team_rules: rules.iter().map(|(k, _)| k.clone()).collect(),
        findings: vec![],
        message: None,
        error: None,
        started_at: Utc::now(),
        finished_at: None,
    }
}

/// `POST /design/artifacts/{id}/assist` — start one agent turn on the
/// artifact's working copy. Answers 202 once the session is live (or after
/// `READY_WAIT`); completion arrives as `design_assist_updated` (+ the usual
/// `design_artifact_updated` for a committed version).
pub async fn start_assist(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<DesignAssistReq>,
) -> ApiResult<(StatusCode, Json<DesignAssistTurn>)> {
    let prompt = clean_prompt(&req.prompt)?;
    let mode = Mode::parse(req.mode.as_deref())?;
    let selection = clean_selection(req.selection)?;
    let svc = crate::design_hall::service(&ctx);
    let a = load_editable(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
    let (head, base_doc) = svc.head_content(&a).await?;
    let adapter = adapter_for(&a.format, &base_doc)?;
    let work_file = svc.work_file(&a).ok_or_else(|| {
        ApiError(Error::Invalid(format!(
            "{} artifacts have no editable working copy",
            a.format
        )))
    })?;
    let dir = work_file.parent().map(FsPath::to_path_buf).ok_or_else(|| {
        ApiError(Error::Internal(
            "design working copy has no directory".into(),
        ))
    })?;
    let turn_id = new_id();
    let guard = try_busy(&a.id, format!("turn:{turn_id}"))?;

    let ws = ctx.workspaces.get(&a.workspace_id).await?;
    let provider = pick_provider(&ctx, &ws, req.provider.as_deref()).await;
    let existing = stored_session(&a.meta, &provider);
    let ctxin = assemble(
        &ctx,
        &svc,
        &user,
        &a,
        &req.references,
        &prompt,
        selection.as_ref(),
        mode,
    )
    .await?;
    let agent_path = materialize(
        &dir,
        &adapter,
        Some(work_file.as_path()),
        &base_doc,
        &turn_id,
    )
    .await?;
    write_context(&dir, &ctxin).await?;
    let dir_str = dir.to_string_lossy().to_string();
    otto_sessions::trust::ensure_trusted(&provider, &dir_str);

    let full_prompt = build_prompt(&PromptInput {
        user_prompt: &prompt,
        mode,
        title: &a.title,
        studio: &a.studio,
        adapter: &adapter,
        direction: None,
        refs: &ctxin.offered,
        rules: &ctxin.rules,
        selection: selection.as_ref(),
    });
    let turn = new_turn(
        &turn_id,
        &a,
        mode,
        &Branch::Main,
        None,
        &provider,
        Some(head.id.clone()),
        &ctxin.offered,
        &ctxin.rules,
    );
    put_turn(turn.clone());
    emit_turn(&ctx.events, &turn);

    let (tx, rx) = oneshot::channel();
    let base_source = adapter.agent_source(&base_doc);
    let job = TurnJob {
        ctx: ctx.clone(),
        ws,
        user,
        artifact: a.clone(),
        turn_id: turn_id.clone(),
        mode,
        branch: Branch::Main,
        direction: None,
        provider,
        model: req.model,
        existing_session: existing,
        adapter,
        dir,
        agent_path,
        work_file: Some(work_file),
        base_doc,
        base_source,
        base_version_id: Some(head.id),
        prompt: full_prompt,
        user_prompt: prompt,
        offered: ctxin.offered,
        rule_keys: ctxin.rules.iter().map(|(k, _)| k.clone()).collect(),
        brand: ctxin.brand,
        selection,
        ready_tx: Some(tx),
    };
    tokio::spawn(async move {
        let _slot = guard;
        run_job(job).await;
    });
    let _ = tokio::time::timeout(READY_WAIT, rx).await;
    let snapshot = get_turn(&a.id, &turn_id).unwrap_or(turn);
    Ok((StatusCode::ACCEPTED, Json(snapshot)))
}

/// `GET /design/artifacts/{id}/assist` — recent turns (in memory; newest first).
pub async fn list_turns(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<DesignAssistTurn>>> {
    let svc = crate::design_hall::service(&ctx);
    let a = load_editable(&ctx, &svc, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(turns_for(&a.id)))
}

/// `POST /design/artifacts/{id}/variants` — n (≤ 4) parallel fresh turns, each
/// committed on `variant/<run>/<k>` (head untouched). `design_variants_ready`
/// fires once every turn finished.
pub async fn start_variants(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<DesignVariantsReq>,
) -> ApiResult<(StatusCode, Json<DesignVariantRun>)> {
    let prompt = clean_prompt(&req.prompt)?;
    let n = req.n;
    if n == 0 || n > otto_design::variants::MAX_VARIANTS {
        return Err(ApiError(Error::Invalid(format!(
            "n must be 1..={}",
            otto_design::variants::MAX_VARIANTS
        ))));
    }
    let selection = clean_selection(req.selection)?;
    let svc = crate::design_hall::service(&ctx);
    let a = load_editable(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
    let (head, base_doc) = svc.head_content(&a).await?;
    let adapter = adapter_for(&a.format, &base_doc)?;
    let run_id = new_id();
    let guard = try_busy(&a.id, format!("variants:{run_id}"))?;
    let ws = ctx.workspaces.get(&a.workspace_id).await?;
    let default_provider = pick_provider(&ctx, &ws, req.provider.as_deref()).await;
    let ctxin = assemble(
        &ctx,
        &svc,
        &user,
        &a,
        &req.references,
        &prompt,
        selection.as_ref(),
        Mode::Variant,
    )
    .await?;
    let root = otto_core::paths::confine_join(&svc.root(), &a.id)
        .ok_or_else(|| ApiError(Error::Invalid(format!("unsafe artifact id {}", a.id))))?
        .join("variants")
        .join(&run_id);

    let mut jobs = Vec::new();
    let mut turns = Vec::new();
    for k in 1..=n {
        let (label, text): (String, String) = if req.directions.is_empty() {
            let (l, t) = DIRECTIONS[(k - 1) % DIRECTIONS.len()];
            (l.to_string(), t.to_string())
        } else {
            let d = cap_chars(req.directions[(k - 1) % req.directions.len()].trim(), 300);
            let l = otto_design::learn::slug(&d);
            (
                if l.is_empty() {
                    format!("direction-{k}")
                } else {
                    l
                },
                d,
            )
        };
        let provider = if req.providers.is_empty() {
            default_provider.clone()
        } else {
            pick_provider(
                &ctx,
                &ws,
                Some(req.providers[(k - 1) % req.providers.len()].as_str()),
            )
            .await
        };
        let dir = root.join(k.to_string());
        let agent_path = materialize(&dir, &adapter, None, &base_doc, &run_id).await?;
        write_context(&dir, &ctxin).await?;
        otto_sessions::trust::ensure_trusted(&provider, &dir.to_string_lossy());
        let branch = Branch::Variant {
            run_id: run_id.clone(),
            k,
        };
        let full_prompt = build_prompt(&PromptInput {
            user_prompt: &prompt,
            mode: Mode::Variant,
            title: &a.title,
            studio: &a.studio,
            adapter: &adapter,
            direction: Some((label.as_str(), text.as_str(), k, n)),
            refs: &ctxin.offered,
            rules: &ctxin.rules,
            selection: selection.as_ref(),
        });
        let turn_id = new_id();
        let turn = new_turn(
            &turn_id,
            &a,
            Mode::Variant,
            &branch,
            Some(label.clone()),
            &provider,
            Some(head.id.clone()),
            &ctxin.offered,
            &ctxin.rules,
        );
        put_turn(turn.clone());
        emit_turn(&ctx.events, &turn);
        turns.push(turn);
        jobs.push(TurnJob {
            ctx: ctx.clone(),
            ws: ws.clone(),
            user: user.clone(),
            artifact: a.clone(),
            turn_id,
            mode: Mode::Variant,
            branch,
            direction: Some(label),
            provider,
            model: req.model.clone(),
            existing_session: None,
            adapter: adapter.clone(),
            dir,
            agent_path,
            work_file: None,
            base_doc: base_doc.clone(),
            base_source: adapter.agent_source(&base_doc),
            base_version_id: Some(head.id.clone()),
            prompt: full_prompt,
            user_prompt: prompt.clone(),
            offered: ctxin.offered.clone(),
            rule_keys: ctxin.rules.iter().map(|(k, _)| k.clone()).collect(),
            brand: ctxin.brand.clone(),
            selection: selection.clone(),
            ready_tx: None,
        });
    }

    let events = ctx.events.clone();
    let (ws_id, aid, rid, base) = (
        a.workspace_id.clone(),
        a.id.clone(),
        run_id.clone(),
        Some(head.id.clone()),
    );
    tokio::spawn(async move {
        let _slot = guard;
        let handles: Vec<_> = jobs.into_iter().map(|j| tokio::spawn(run_job(j))).collect();
        let mut version_ids = Vec::new();
        let mut failed = 0usize;
        for h in handles {
            match h.await {
                Ok(t) if t.status == "done" => version_ids.extend(t.version_id),
                _ => failed += 1,
            }
        }
        let _ = events.send(Event::DesignVariantsReady {
            workspace_id: ws_id,
            artifact_id: aid,
            run_id: rid,
            base_version_id: base,
            version_ids,
            failed,
        });
    });
    Ok((
        StatusCode::ACCEPTED,
        Json(DesignVariantRun {
            run_id,
            artifact_id: a.id.clone(),
            base_version_id: Some(head.id),
            status: "running".into(),
            versions: vec![],
            accepted_version_id: None,
            turns,
        }),
    ))
}

/// `GET /design/artifacts/{id}/variants` — the artifact's variant runs,
/// newest first (≤ 10): persisted versions + live turn states.
pub async fn list_variant_runs(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<DesignVariantRun>>> {
    let svc = crate::design_hall::service(&ctx);
    let a = load_editable(&ctx, &svc, &user, &id, WorkspaceRole::Viewer).await?;
    let versions = svc
        .store()
        .versions_on_branch_prefix(&a.id, otto_design::variants::VARIANT_PREFIX)
        .await?;
    let accepted: HashMap<String, Id> = svc
        .store()
        .list_signals(
            None,
            Some(a.id.as_str()),
            Some("variant_accepted"),
            None,
            1_000,
        )
        .await?
        .into_iter()
        .filter_map(|s| {
            let run = s.payload.get("run_id").and_then(Value::as_str)?.to_string();
            Some((run, s.version_id?))
        })
        .collect();
    let busy = busy_of(&a.id);
    let live = turns_for(&a.id);
    let mut runs: Vec<DesignVariantRun> = Vec::new();
    // Newest run first: iterate versions newest-seq first.
    for v in versions.into_iter().rev() {
        let Some((run, _k)) = otto_design::variants::parse_branch(&v.branch) else {
            continue;
        };
        match runs.iter().position(|r| r.run_id == run) {
            Some(i) => runs[i].versions.insert(0, v),
            None => {
                if runs.len() >= 10 {
                    continue;
                }
                runs.push(DesignVariantRun {
                    run_id: run.clone(),
                    artifact_id: a.id.clone(),
                    base_version_id: v.parent_version_id.clone(),
                    status: String::new(),
                    versions: vec![v],
                    accepted_version_id: accepted.get(&run).cloned(),
                    turns: vec![],
                });
            }
        }
    }
    // A running set may have no committed version yet.
    if let Some(run) = busy.as_deref().and_then(|b| b.strip_prefix("variants:")) {
        if !runs.iter().any(|r| r.run_id == run) {
            runs.insert(
                0,
                DesignVariantRun {
                    run_id: run.to_string(),
                    artifact_id: a.id.clone(),
                    base_version_id: a.head_version_id.clone(),
                    status: String::new(),
                    versions: vec![],
                    accepted_version_id: None,
                    turns: vec![],
                },
            );
        }
    }
    for r in &mut runs {
        let prefix = otto_design::variants::run_prefix(&r.run_id);
        r.turns = live
            .iter()
            .filter(|t| t.branch.starts_with(&prefix))
            .cloned()
            .collect();
        r.status = if r.accepted_version_id.is_some() {
            "accepted"
        } else if busy.as_deref() == Some(format!("variants:{}", r.run_id).as_str()) {
            "running"
        } else {
            "ready"
        }
        .into();
    }
    Ok(Json(runs))
}

/// `POST /design/artifacts/{id}/variants/{version}/accept` — fast-forward main
/// to the variant, record `variant_accepted` / `variant_rejected`, then run a
/// learning pass in the background (suggest-only).
pub async fn accept_variant(
    Path((id, version)): Path<(Id, String)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<AcceptVariantReq>>,
) -> ApiResult<Json<DesignVariantAcceptResp>> {
    let force = body.map(|Json(b)| b.force).unwrap_or(false);
    let svc = crate::design_hall::service(&ctx);
    let a = load_editable(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
    if let Some(b) = busy_of(&a.id) {
        return Err(ApiError(Error::Conflict(format!(
            "an agent is still working on this artifact ({b}); accept once it finished"
        ))));
    }
    let out = svc
        .accept_variant(&a.id, &version, &Author::user(&user.id), force)
        .await?;
    let bg = ctx.clone();
    let ws = a.workspace_id.clone();
    tokio::spawn(async move {
        if let Err(e) = run_learning_pass(&bg, &ws).await {
            tracing::warn!("design learning pass after a variant accept failed: {}", e);
        }
    });
    Ok(Json(DesignVariantAcceptResp {
        artifact: out.saved.artifact,
        version: out.saved.version,
        run_id: out.run_id,
        accepted_version_id: out.accepted.id,
        rejected_version_ids: out.rejected.into_iter().map(|v| v.id).collect(),
    }))
}

// ---------------------------------------------------------------------------
// Learning v1
// ---------------------------------------------------------------------------

async fn workspace_candidates(ctx: &ServerCtx, ws_id: &Id) -> Result<Vec<RuleCandidate>, Error> {
    let svc = crate::design_hall::service(ctx);
    let since = otto_design::store::stamp(Utc::now() - chrono::Duration::days(LEARN_LOOKBACK_DAYS));
    let sigs = svc
        .store()
        .list_signals(
            Some(std::slice::from_ref(ws_id)),
            None,
            None,
            Some(since.as_str()),
            LEARN_SIGNAL_LIMIT,
        )
        .await?;
    Ok(learn::extract(&sigs))
}

/// Extract candidates and propose the ready ones (unless learning is `off`).
async fn run_learning_pass(
    ctx: &ServerCtx,
    ws_id: &Id,
) -> Result<(String, otto_improve::DesignLearnOutcome, Vec<RuleCandidate>), Error> {
    let ws = ctx.workspaces.get(ws_id).await?;
    let mode = learning_mode(&ws.settings).to_string();
    let candidates = workspace_candidates(ctx, ws_id).await?;
    if mode == "off" {
        return Ok((mode, Default::default(), candidates));
    }
    let proposals: Vec<DesignRuleProposal> = candidates
        .iter()
        .filter(|c| c.ready)
        .map(|c| DesignRuleProposal {
            key: c.key.clone(),
            rule: c.rule.clone(),
            rationale: c.rationale.clone(),
            evidence: c.signal_ids.clone(),
        })
        .collect();
    let outcome = ctx
        .improve_engine
        .learn_design_rules(ws_id, &proposals)
        .await?;
    if !outcome.proposed.is_empty() {
        let _ = ctx.events.send(Event::DesignLearningUpdate {
            workspace_id: ws_id.clone(),
            kind: "rule_proposed".into(),
            signal_id: None,
            artifact_id: None,
        });
    }
    Ok((mode, outcome, candidates))
}

fn learned_edit(e: &otto_core::domain::ImprovementEdit) -> DesignLearnedEdit {
    DesignLearnedEdit {
        edit_id: e.id.clone(),
        status: e.status.as_str().to_string(),
        rules: added_rules(e)
            .into_iter()
            .map(|(key, rule)| DesignRuleLine { key, rule })
            .collect(),
        rationale: e.rationale.clone(),
        evidence: e.evidence.clone(),
        created_at: e.created_at,
        applied_at: e.applied_at,
        actor: e.actor.clone(),
    }
}

/// `GET /design/learned?workspace_id=` — active rules (with evidence), the
/// pending proposals, the decision history and the current candidates.
pub async fn get_learned(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<LearnedQuery>,
) -> ApiResult<Json<DesignLearnedResp>> {
    crate::auth::require_ws_role(&ctx, &user, &q.workspace_id, WorkspaceRole::Viewer).await?;
    let ws = ctx.workspaces.get(&q.workspace_id).await?;
    let engine = &ctx.improve_engine;
    let path = engine.design_skill_path(&ws.id).await?;
    let lines = engine.design_rules_active(&ws.id).await?;
    let edits = engine.design_rule_edits(&ws.id).await?;
    let active = lines
        .into_iter()
        .map(|(key, rule)| {
            let edit = edits.iter().find(|e| {
                e.status == ImprovementEditStatus::Applied
                    && added_rules(e).iter().any(|(k, _)| k == &key)
            });
            DesignLearnedRule {
                edit_id: edit.map(|e| e.id.clone()),
                evidence: edit.map(|e| e.evidence.clone()).unwrap_or_default(),
                applied_at: edit.and_then(|e| e.applied_at),
                key,
                rule,
            }
        })
        .collect();
    let pending = edits
        .iter()
        .filter(|e| e.status == ImprovementEditStatus::Pending)
        .map(learned_edit)
        .collect();
    let history = edits
        .iter()
        .filter(|e| e.status != ImprovementEditStatus::Pending)
        .take(50)
        .map(learned_edit)
        .collect();
    let candidates = workspace_candidates(&ctx, &ws.id).await?;
    Ok(Json(DesignLearnedResp {
        workspace_id: ws.id.clone(),
        mode: learning_mode(&ws.settings).to_string(),
        skill: DESIGN_SKILL.to_string(),
        skill_path: path.display().to_string(),
        active,
        pending,
        history,
        candidates,
    }))
}

/// `POST /design/learned/extract {workspace_id}` — run the extractor now and
/// propose the ready candidates as pending edits (suggest-only).
pub async fn extract_learned(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<LearnExtractReq>,
) -> ApiResult<Json<DesignLearnExtractResp>> {
    crate::auth::require_ws_role(&ctx, &user, &req.workspace_id, WorkspaceRole::Editor).await?;
    let (mode, outcome, candidates) = run_learning_pass(&ctx, &req.workspace_id).await?;
    Ok(Json(DesignLearnExtractResp {
        mode,
        run_id: outcome.run_id,
        proposed: outcome.proposed,
        skipped: outcome.skipped,
        candidates,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn offered(n: usize) -> Vec<OfferedRef> {
        (1..=n)
            .map(|i| OfferedRef {
                label: format!("R{i}"),
                artifact_id: format!("A{i}"),
                version_id: Some(format!("V{i}")),
                seq: Some(i as i64 + 2),
                title: format!("Ref {i}"),
                studio: "site".into(),
                format: "html".into(),
                status: "shipped".into(),
                source: "search".into(),
            })
            .collect()
    }

    #[test]
    fn modes_parse_and_critique_never_edits() {
        assert_eq!(Mode::parse(None).unwrap(), Mode::Refine);
        assert_eq!(Mode::parse(Some(" a11y ")).unwrap(), Mode::A11y);
        assert_eq!(Mode::parse(Some("generate")).unwrap(), Mode::Generate);
        assert!(Mode::parse(Some("variant")).is_err());
        assert!(Mode::parse(Some("paint")).is_err());
        assert!(!Mode::Critique.edits());
        assert!(Mode::A11y.edits() && Mode::Variant.edits());
    }

    #[test]
    fn adapters_pick_the_agent_file_per_format() {
        let h = adapter_for("html", b"<p/>").unwrap();
        assert_eq!((h.agent_file, h.fence), ("design.html", "html"));
        assert_eq!(
            adapter_for("scene3d", b"{}").unwrap().agent_file,
            "scene.json"
        );
        assert_eq!(adapter_for("otto-site", b"{}").unwrap().fence, "json");
        assert!(adapter_for("png", b"x").is_err());
        assert!(adapter_for("nope", b"x").is_err());

        // A whiteboard exposes its inner source and wraps it back losslessly.
        let doc =
            br#"{"type":"otto-canvas","version":1,"format":"d2","source":"a -> b","section":"x"}"#;
        let c = adapter_for("otto-canvas", doc).unwrap();
        assert_eq!((c.agent_file, c.canvas_inner), ("source.d2", Some("d2")));
        assert_eq!(c.agent_source(doc), "a -> b");
        let wrapped: Value = serde_json::from_slice(&c.wrap(doc, "a -> c").unwrap()).unwrap();
        assert_eq!(wrapped["source"], "a -> c");
        assert_eq!(wrapped["section"], "x");
        assert_eq!(wrapped["format"], "d2");
        // Empty mermaid whiteboard → the base source; bad excalidraw is refused.
        let m = adapter_for("otto-canvas", br#"{"type":"otto-canvas"}"#).unwrap();
        assert_eq!(m.agent_file, "source.mmd");
        assert_eq!(
            m.agent_source(br#"{"type":"otto-canvas"}"#),
            "flowchart TD\n"
        );
        let ex = adapter_for("otto-canvas", br#"{"format":"excalidraw"}"#).unwrap();
        assert!(ex.wrap(b"{}", "not json").is_err());
        assert!(ex.wrap(b"{}", r#"{"type":"excalidraw"}"#).is_ok());
    }

    #[test]
    fn documents_are_validated_per_format() {
        assert!(validate_doc("html", b"<h1>x</h1>").is_ok());
        assert!(validate_doc("excalidraw", b"[1]").is_err());
        assert!(validate_doc(
            "scene3d",
            br#"{"type":"otto-scene3d","version":1,"objects":[]}"#
        )
        .is_ok());
        assert!(validate_doc("scene3d", br#"{"type":"otto-scene3d","version":9}"#).is_err());
    }

    #[test]
    fn fences_summaries_terms_and_refs() {
        assert_eq!(
            extract_fenced("Done.\n```html\n<p>x</p>\n```", "html").as_deref(),
            Some("<p>x</p>")
        );
        assert!(extract_fenced("no fence", "html").is_none());
        assert_eq!(
            summary_line("\n  Made it bolder [R1].\nmore").as_deref(),
            Some("Made it bolder [R1].")
        );
        assert_eq!(summary_line("```html\n"), None);
        assert_eq!(
            prompt_terms("Make the hero bolder with testimonials, please 2024", 3),
            vec!["testimonials", "bolder", "hero"]
        );
        assert_eq!(parse_ref_spec("A1@v3"), Some(("A1".into(), Some(3))));
        assert_eq!(
            parse_ref_spec("otto://design/A1"),
            Some(("A1".into(), None))
        );
        assert_eq!(parse_ref_spec("bad id!"), None);
    }

    #[test]
    fn findings_are_bounded_objects() {
        let many: Vec<Value> = (0..40).map(|i| json!({"rule": i})).collect();
        assert_eq!(
            parse_findings(&Value::Array(many).to_string()).len(),
            MAX_FINDINGS
        );
        assert_eq!(parse_findings(r#"{"findings":[{"a":1},2]}"#).len(), 1);
        assert!(parse_findings("nope").is_empty());
    }

    #[test]
    fn learning_mode_and_stored_session() {
        assert_eq!(learning_mode(&json!({})), "suggest");
        assert_eq!(learning_mode(&json!({"design_learning": "off"})), "off");
        let meta = json!({"assist": {"session_id": "s1", "provider": "claude"}});
        assert_eq!(stored_session(&meta, "claude").as_deref(), Some("s1"));
        assert_eq!(stored_session(&meta, "codex"), None);
        assert_eq!(stored_session(&json!({}), "claude"), None);
    }

    #[test]
    fn brief_is_deterministic_bounded_and_numbered() {
        let refs = offered(2);
        let b = BriefInput {
            artifact_id: "ART".into(),
            title: "Launch hero".into(),
            studio: "site".into(),
            format: "html".into(),
            status: "draft".into(),
            head_seq: Some(4),
            mode: "refine".into(),
            story: Some(("LOY-142 Rewards+".into(), "AC: join in one tap".into())),
            uses: vec!["embeds → \"Card\" (3d, scene3d) otto://design/C".into()],
            used_in: vec![],
            brand: Some("\"Brand\" v2: color.primary #123".into()),
            refs: refs.iter().cloned().map(|r| (r, true)).collect(),
            rules: vec![("variant_direction:bold".into(), "Lead bold.".into())],
            memories: vec!["Heroes: keep the CTA above the fold".into()],
            selection: Some(json!({"node_id": "hero"})),
            has_render: true,
        };
        let s = render_brief(&b);
        assert_eq!(s, render_brief(&b.clone()));
        assert!(s.contains("[R1] Ref 1 (site, shipped, v3) — otto://design/A1@v3"));
        assert!(s.contains("thumbnail refs/R2.png"));
        assert!(s.contains("LOY-142 Rewards+") && s.contains("AC: join in one tap"));
        assert!(s.contains("- [variant_direction:bold] Lead bold."));
        assert!(s.contains("render/current.png"));
        // Bounded: a huge story is cut at the cap.
        let mut big = b.clone();
        big.story = Some(("S".into(), "x".repeat(40_000)));
        big.uses = (0..500)
            .map(|i| format!("link {i} {}", "y".repeat(100)))
            .collect();
        assert!(render_brief(&big).len() <= MAX_BRIEF_BYTES);
    }

    #[test]
    fn prompt_carries_sentinel_file_refs_rules_and_direction() {
        let a = adapter_for("html", b"").unwrap();
        let refs = offered(1);
        let rules = vec![("a11y:contrast".to_string(), "Check contrast.".to_string())];
        let sel = json!({"section_id": "hero"});
        let p = build_prompt(&PromptInput {
            user_prompt: "bolder hero",
            mode: Mode::Variant,
            title: "Landing",
            studio: "site",
            adapter: &a,
            direction: Some(("explore", "Try something bold.", 2, 3)),
            refs: &refs,
            rules: &rules,
            selection: Some(&sel),
        });
        assert!(p.starts_with("OTTO_TASK: design_assist"));
        assert!(p.contains("`design.html`"));
        assert!(p.contains("[R1] Ref 1"));
        assert!(p.contains("Applied your team rules"));
        assert!(p.contains("variant 2 of 3, \"explore\""));
        assert!(p.contains("\"section_id\":\"hero\""));
        assert!(p.contains("Request: bolder hero"));
        let c = build_prompt(&PromptInput {
            user_prompt: "review",
            mode: Mode::Critique,
            title: "T",
            studio: "frames",
            adapter: &a,
            direction: None,
            refs: &[],
            rules: &[],
            selection: None,
        });
        assert!(c.contains("MODE: critique") && c.contains("do NOT modify"));
        assert!(c.contains("do not cite any"));
        assert!(!c.contains("TEAM RULES"));
    }

    #[test]
    fn provenance_records_offered_cited_and_stays_bounded() {
        let refs = offered(3);
        let cit = cite::verify(&refs, "hero rhythm from [R2] and grid from [R9]", None);
        let p = build_provenance(
            "T1",
            Mode::Refine,
            &Branch::Main,
            None,
            "claude",
            Some("S1"),
            "make it bolder",
            &refs,
            &cit,
            Some(&json!({"artifact_id": "B", "version_id": "BV", "seq": 2})),
            &["variant_direction:bold".to_string()],
            Some(&json!({"node_id": "hero"})),
        );
        assert_eq!(p["assist"]["turn_id"], "T1");
        assert_eq!(p["assist"]["branch"], "main");
        assert_eq!(p["references_offered"].as_array().unwrap().len(), 3);
        assert_eq!(p["references_cited"][0]["label"], "R2");
        assert_eq!(p["citations_unverified"][0], "R9");
        assert_eq!(p["brand_kit"]["seq"], 2);
        assert!(p["selection"].is_string());
        // A variant stamps its direction where `variants::direction_of` reads it.
        let v = build_provenance(
            "T2",
            Mode::Variant,
            &Branch::Variant {
                run_id: "R".into(),
                k: 2,
            },
            Some("explore"),
            "codex",
            None,
            "x",
            &[],
            &CitationReport::default(),
            None,
            &[],
            None,
        );
        assert_eq!(v["assist"]["direction"], "explore");
        assert_eq!(v["assist"]["branch"], "variant/R/2");
        assert!(otto_design::service::bound_json(
            v,
            otto_design::service::MAX_PROVENANCE_BYTES,
            "p"
        )
        .is_ok());
    }

    #[test]
    fn registry_holds_one_run_per_artifact_and_releases_on_drop() {
        let aid = format!("reg-test-{}", new_id());
        let g = try_busy(&aid, "turn:1".into()).unwrap();
        assert!(matches!(
            try_busy(&aid, "turn:2".into()),
            Err(Error::Conflict(_))
        ));
        assert_eq!(busy_of(&aid).as_deref(), Some("turn:1"));
        drop(g);
        assert!(busy_of(&aid).is_none());
        let _g2 = try_busy(&aid, "variants:r".into()).unwrap();

        let a = DesignArtifact {
            id: aid.clone(),
            project_id: None,
            workspace_id: "w".into(),
            studio: "frames".into(),
            format: "html".into(),
            mime: "text/html".into(),
            title: "T".into(),
            status: "draft".into(),
            head_version_id: None,
            head_seq: None,
            approved_version_id: None,
            tags: vec![],
            thumb_blob: None,
            meta: json!({}),
            source_kind: None,
            source_id: None,
            created_by: "u".into(),
            created_by_kind: "user".into(),
            created_session_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by_name: None,
            last_editor_id: None,
            last_editor_kind: None,
            last_editor_name: None,
            story_ids: vec![],
        };
        for i in 0..(TURN_HISTORY + 5) {
            put_turn(new_turn(
                &format!("t{i}"),
                &a,
                Mode::Refine,
                &Branch::Main,
                None,
                "claude",
                None,
                &[],
                &[],
            ));
        }
        let turns = turns_for(&aid);
        assert_eq!(turns.len(), TURN_HISTORY);
        assert_eq!(turns[0].turn_id, format!("t{}", TURN_HISTORY + 4));
        let upd = update_turn(&aid, &turns[0].turn_id, |t| t.status = "done".into()).unwrap();
        assert_eq!(upd.status, "done");
        assert!(
            get_turn(&aid, "t0").is_none(),
            "the oldest turns are evicted"
        );
    }
}
