//! Proof-pack engine: evidence assembly, redaction-aware content ingestion,
//! status/risk recompute, and the gate entry points the integration sites call.
//!
//! All persisted artifact content flows through [`upsert_content_artifact`] /
//! [`add_content_artifact`], which redact (via `otto_core::redact`) and cap before
//! storing — so the auto-gate paths (goal loop / review / workflow / session) and
//! the artifact API share one trust boundary. [`recompute_and_emit`] is serialized
//! per pack so concurrent mutations can't lose an update.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use otto_core::proof::{
    compute_badges, compute_done_contract, compute_risk, derive_status_with_policy,
    preview as core_preview, CiSummary, DoneContractPolicy, ProofArtifact, ProofArtifactKind,
    ProofArtifactStatus, ProofBadge, ProofPack, ProofStatus, WorkItemKind, STORE_CAP,
};
use otto_core::event::Event;
use otto_core::{redact, Error, Id, Result};
use otto_git::local::{DiffTarget, LocalGit};
use serde_json::{json, Value};

use crate::state::ServerCtx;

/// Per-pack async locks: the outer `std::Mutex` guards the map; each value is an
/// async mutex held across the recompute read→derive→write.
pub type ProofLocks = Arc<Mutex<HashMap<Id, Arc<tokio::sync::Mutex<()>>>>>;

pub fn new_locks() -> ProofLocks {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Default bound for an assembled command (tests can be slow; keep generous but
/// finite so a hang can't wedge a gate).
const CMD_TIMEOUT_SECS: u64 = 600;

// ---------------------------------------------------------------------------
// Risky-file classification (D13 — path segments / extensions / basenames)
// ---------------------------------------------------------------------------

/// Whether a changed path is "risky" and should raise the risk score / badge.
/// Matches by path SEGMENT, extension, or basename — not naive substring — so
/// `author.rs` / `tokenizer.rs` aren't false-flagged.
pub fn is_risky_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    let segs: Vec<&str> = lower.split('/').collect();
    let base = segs.last().copied().unwrap_or("");

    // Migrations + CI config (path segments).
    if segs.contains(&"migrations") || segs.contains(&".github") {
        return true;
    }
    // SQL / lockfiles (extension / basename).
    if lower.ends_with(".sql")
        || base == "cargo.lock"
        || base == "package-lock.json"
        || base == "yarn.lock"
        || base == "pnpm-lock.yaml"
    {
        return true;
    }
    // Security-sensitive areas: match a whole word in the basename (split on
    // non-alphanumeric), so `auth.rs`/`policy.rs`/`oauth_config.ts` hit but
    // `author.rs`/`tokenizer.rs` do not.
    const RISKY_WORDS: &[&str] = &[
        "auth", "rbac", "keychain", "netguard", "policy", "secret", "secrets", "password",
        "passwords", "crypto", "token", "tokens", "credential", "credentials",
    ];
    let words: Vec<&str> = base
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    words.iter().any(|w| RISKY_WORDS.contains(w))
}

/// Classify a command as `test | build | lint` for artifact metadata.
pub fn classify_test_kind(cmd: &str) -> &'static str {
    let c = cmd.to_lowercase();
    if c.contains("clippy") || c.contains("vet") || c.contains("svelte-check") || c.contains("lint")
    {
        "lint"
    } else if c.contains("build") || c.contains("compile") {
        "build"
    } else {
        "test"
    }
}

// ---------------------------------------------------------------------------
// Command execution (capture stdout+stderr+exit+duration)
// ---------------------------------------------------------------------------

pub struct CmdRun {
    pub success: bool,
    pub exit_code: i32,
    pub output: String,
    pub duration_ms: u64,
}

/// Run `sh -c <cmd>` in `cwd`, capturing combined output, exit code, and wall
/// time. Bounded by `timeout_secs`.
pub async fn run_command(cwd: &str, cmd: &str, timeout_secs: u64) -> CmdRun {
    let start = std::time::Instant::now();
    let fut = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .output();
    match tokio::time::timeout(Duration::from_secs(timeout_secs), fut).await {
        Ok(Ok(out)) => {
            let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
            let err = String::from_utf8_lossy(&out.stderr);
            if !err.is_empty() {
                s.push_str("\n--- stderr ---\n");
                s.push_str(&err);
            }
            CmdRun {
                success: out.status.success(),
                exit_code: out.status.code().unwrap_or(-1),
                output: s,
                duration_ms: start.elapsed().as_millis() as u64,
            }
        }
        Ok(Err(e)) => CmdRun {
            success: false,
            exit_code: -1,
            output: format!("failed to spawn command: {e}"),
            duration_ms: start.elapsed().as_millis() as u64,
        },
        Err(_) => CmdRun {
            success: false,
            exit_code: -1,
            output: format!("command timed out after {timeout_secs}s"),
            duration_ms: start.elapsed().as_millis() as u64,
        },
    }
}

// ---------------------------------------------------------------------------
// Content ingestion (the single redaction + cap boundary)
// ---------------------------------------------------------------------------

/// Redact and cap `content`, returning `(stored, extra_meta_patch)` where the
/// patch carries `ref_kind`, `redactions`, and `truncated`.
fn prepare_content(content: &str) -> (String, Value) {
    let red = redact::redact_text(content);
    let redactions: usize = red.hits.iter().map(|h| h.count).sum();
    let mut value = red.value;
    let mut truncated = false;
    if value.len() > STORE_CAP {
        let cut = value
            .char_indices()
            .take_while(|(i, _)| *i <= STORE_CAP)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(STORE_CAP);
        value.truncate(cut);
        value.push_str("\n…(truncated)");
        truncated = true;
    }
    (
        value,
        json!({ "ref_kind": "inline", "redactions": redactions, "truncated": truncated }),
    )
}

fn merge_meta(base: Value, patch: Value) -> Value {
    let mut m = match base {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    if let Value::Object(p) = patch {
        for (k, v) in p {
            m.insert(k, v);
        }
    }
    Value::Object(m)
}

/// Upsert an inline-content artifact by `(pack, kind, title)` — the canonical
/// path for AUTO evidence (goal loop / review / workflow / session). Redacts +
/// caps; merges `extra_meta` with the ref-kind patch.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_content_artifact(
    ctx: &ServerCtx,
    pack: &ProofPack,
    kind: ProofArtifactKind,
    title: &str,
    content: &str,
    status: ProofArtifactStatus,
    extra_meta: Value,
    created_by: &str,
) -> Result<ProofArtifact> {
    let (stored, patch) = prepare_content(content);
    let meta = merge_meta(extra_meta, patch);
    ctx.proof_repo
        .upsert_artifact_by_title(
            &pack.id,
            &pack.workspace_id,
            kind,
            title,
            Some(&stored),
            status,
            &meta,
            created_by,
        )
        .await
}

/// Add an inline/url artifact (the artifact API path). `url` is stored verbatim
/// with `ref_kind=url`; inline content is redacted + capped. Appends a new row.
#[allow(clippy::too_many_arguments)]
pub async fn add_content_artifact(
    ctx: &ServerCtx,
    pack: &ProofPack,
    kind: ProofArtifactKind,
    title: &str,
    content: Option<&str>,
    url: Option<&str>,
    status: ProofArtifactStatus,
    extra_meta: Value,
    created_by: &str,
) -> Result<ProofArtifact> {
    let (content_ref, meta) = if let Some(u) = url {
        (Some(u.to_string()), merge_meta(extra_meta, json!({"ref_kind": "url"})))
    } else if let Some(c) = content {
        let (stored, patch) = prepare_content(c);
        (Some(stored), merge_meta(extra_meta, patch))
    } else {
        (None, merge_meta(extra_meta, json!({"ref_kind": "none"})))
    };
    ctx.proof_repo
        .add_artifact(
            &pack.id,
            &pack.workspace_id,
            kind,
            title,
            content_ref.as_deref(),
            status,
            &meta,
            created_by,
        )
        .await
}

// ---------------------------------------------------------------------------
// Auto-assembly
// ---------------------------------------------------------------------------

/// Assemble a `diff` artifact from `cwd` (vs `base`, or the working tree vs HEAD
/// when `base` is None). Best-effort: returns Ok(()) without an artifact if the
/// path isn't a git repo or the diff fails. Idempotent (upsert by title).
pub async fn assemble_diff(ctx: &ServerCtx, pack: &ProofPack, cwd: &str, base: Option<&str>) -> Result<()> {
    let git = LocalGit::new(cwd);
    let (text, resp) = match base {
        Some(b) => {
            let t = git.diff_text_against(b).await;
            // Structured metadata must describe the work done since `base`
            // (the `base..HEAD` range), NOT `git show <base>` (the base commit's
            // own patch). The latter made files_changed/additions/risky_files —
            // and therefore the derived risk_score — reflect the wrong commit.
            let r = git
                .diff(DiffTarget::Range(b.to_string(), "HEAD".to_string()), None)
                .await;
            (t, r)
        }
        None => {
            let t = git.working_diff_text().await;
            let r = git.diff(DiffTarget::Working, None).await;
            (t, r)
        }
    };
    let (text, resp) = match (text, resp) {
        (Ok(t), Ok(r)) => (t, r),
        _ => return Ok(()), // not a git repo / diff failed — skip silently
    };
    if resp.files.is_empty() {
        return Ok(());
    }
    let additions: u32 = resp.files.iter().filter_map(|f| f.added).sum();
    let deletions: u32 = resp.files.iter().filter_map(|f| f.deleted).sum();
    let risky_files: Vec<String> = resp
        .files
        .iter()
        .map(|f| f.path.clone())
        .filter(|p| is_risky_file(p))
        .collect();
    let meta = json!({
        "files_changed": resp.files.len(),
        "additions": additions,
        "deletions": deletions,
        "risky_files": risky_files,
    });
    upsert_content_artifact(
        ctx,
        pack,
        ProofArtifactKind::Diff,
        "Working tree diff",
        &text,
        ProofArtifactStatus::Info,
        meta,
        "otto",
    )
    .await?;
    Ok(())
}

/// Run a command, capturing it as a `command` artifact (upsert by command).
/// Returns the artifact status.
pub async fn run_command_artifact(
    ctx: &ServerCtx,
    pack: &ProofPack,
    cwd: &str,
    cmd: &str,
    kind_hint: Option<&str>,
) -> Result<ProofArtifactStatus> {
    let run = run_command(cwd, cmd, CMD_TIMEOUT_SECS).await;
    let test_kind = kind_hint.unwrap_or_else(|| classify_test_kind(cmd));
    let status = if run.success {
        ProofArtifactStatus::Passed
    } else {
        ProofArtifactStatus::Failed
    };
    let meta = json!({
        "test_kind": test_kind,
        "exit_code": run.exit_code,
        "duration_ms": run.duration_ms,
    });
    upsert_content_artifact(ctx, pack, ProofArtifactKind::Command, cmd, &run.output, status, meta, "otto").await?;
    Ok(status)
}

// ---------------------------------------------------------------------------
// Gate + recompute
// ---------------------------------------------------------------------------

/// Ensure-or-create the pack for a work item (the gate entry point).
pub async fn gate(
    ctx: &ServerCtx,
    kind: WorkItemKind,
    work_item_id: &str,
    workspace_id: &str,
    title: &str,
    created_by: &str,
) -> Result<ProofPack> {
    ctx.proof_repo
        .ensure_pack(workspace_id, kind, work_item_id, title, created_by)
        .await
}

fn lock_for(ctx: &ServerCtx, pack_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = ctx.proof_locks.lock().unwrap();
    map.entry(pack_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// Resolve the done-contract policy for a pack: the work-item-kind defaults
/// (which reproduce the legacy status) strengthened by any repo opt-ins. Repo
/// lookup is best-effort — an unknown/unlinked repo yields the default policy, so
/// a pack never gets *weaker* than v1 and a lookup failure can't wedge recompute.
pub async fn policy_for_pack(ctx: &ServerCtx, pack: &ProofPack) -> DoneContractPolicy {
    let base = DoneContractPolicy::for_kind(pack.work_item_kind);
    let Some(repo_id) = pack.repo_id.as_deref() else {
        return base;
    };
    match ctx.git_store.get_proof_config(repo_id).await {
        Ok(cfg) => base.with_repo(&cfg),
        Err(_) => base,
    }
}

/// Reload a pack's artifacts, derive its status (policy-aware) + risk +
/// done-contract score, persist, and broadcast `ProofPackUpdated`. Serialized
/// per pack so concurrent callers can't lose an update. Returns the refreshed
/// pack.
pub async fn recompute_and_emit(ctx: &ServerCtx, pack_id: &str) -> Result<ProofPack> {
    let lock = lock_for(ctx, pack_id);
    let _guard = lock.lock().await;

    let pack = ctx.proof_repo.get_pack(pack_id).await?;
    let arts = ctx.proof_repo.list_artifacts(pack_id).await?;
    let policy = policy_for_pack(ctx, &pack).await;
    let status = derive_status_with_policy(&pack, &arts, &policy);
    let risk = compute_risk(&arts);
    let done = compute_done_contract(&pack, &arts, &policy).score;
    ctx.proof_repo
        .set_status_risk_done(pack_id, status, risk, done)
        .await?;

    let _ = ctx.events.send(Event::ProofPackUpdated {
        workspace_id: pack.workspace_id.clone(),
        proof_pack_id: pack.id.clone(),
        work_item_kind: pack.work_item_kind.as_str().to_string(),
        work_item_id: pack.work_item_id.clone(),
        status: status.as_str().to_string(),
        risk_score: risk,
        done_score: done,
    });

    ctx.proof_repo.get_pack(pack_id).await
}

// ---------------------------------------------------------------------------
// v2 evidence capture — CI, API/DB/Kafka reads, media, PR-check, snapshots
// ---------------------------------------------------------------------------

/// Record a CI aggregate as a `ci` artifact (upsert) and recompute.
pub async fn record_ci_artifact(ctx: &ServerCtx, pack: &ProofPack, ci: &CiSummary) -> Result<()> {
    let status = otto_core::proof::ci_artifact_status(&ci.state);
    let body = format!(
        "CI state: {}\nchecks: {} total · {} passed · {} failed{}",
        ci.state,
        ci.total,
        ci.passed,
        ci.failed,
        ci.url.as_deref().map(|u| format!("\nurl: {u}")).unwrap_or_default()
    );
    let meta = json!({
        "evidence": "ci", "state": ci.state, "total": ci.total,
        "passed": ci.passed, "failed": ci.failed, "url": ci.url
    });
    upsert_content_artifact(
        ctx,
        pack,
        ProofArtifactKind::Ci,
        "CI status",
        &body,
        status,
        meta,
        "otto",
    )
    .await?;
    recompute_and_emit(ctx, &pack.id).await?;
    Ok(())
}

/// Capture an HTTP request/response as an `api` artifact (redacted + capped).
pub async fn attach_api_evidence(
    ctx: &ServerCtx,
    pack: &ProofPack,
    req: &otto_core::api::ApiEvidenceReq,
    by: &str,
) -> Result<ProofArtifact> {
    let status = otto_core::proof::http_evidence_status(req.status);
    let mut body = format!("{} {}\nstatus: {}", req.method.to_uppercase(), req.url, req.status);
    if let Some(d) = req.duration_ms {
        body.push_str(&format!("\nduration_ms: {d}"));
    }
    if let Some(rq) = &req.request {
        body.push_str("\n\n--- request ---\n");
        body.push_str(rq);
    }
    if let Some(rs) = &req.response {
        body.push_str("\n\n--- response ---\n");
        body.push_str(rs);
    }
    let meta = merge_meta(
        json!({"evidence": "api", "method": req.method, "url": req.url,
               "http_status": req.status, "duration_ms": req.duration_ms}),
        req.metadata.clone().unwrap_or(Value::Null),
    );
    let art = add_content_artifact(
        ctx,
        pack,
        ProofArtifactKind::Api,
        &req.title,
        Some(&body),
        None,
        status,
        meta,
        by,
    )
    .await?;
    recompute_and_emit(ctx, &pack.id).await?;
    Ok(art)
}

/// Capture a DB read result as a `db` artifact.
pub async fn attach_db_evidence(
    ctx: &ServerCtx,
    pack: &ProofPack,
    req: &otto_core::api::DbEvidenceReq,
    by: &str,
) -> Result<ProofArtifact> {
    let status = otto_core::proof::read_evidence_status(req.error.is_some());
    let mut body = String::new();
    if let Some(e) = &req.engine {
        body.push_str(&format!("engine: {e}\n"));
    }
    if let Some(q) = &req.query {
        body.push_str(&format!("query: {q}\n"));
    }
    if let Some(c) = &req.columns {
        body.push_str(&format!("columns: {}\n", c.join(", ")));
    }
    if let Some(n) = req.row_count {
        body.push_str(&format!("rows: {n}\n"));
    }
    if let Some(s) = &req.sample {
        body.push_str("\n--- sample ---\n");
        body.push_str(s);
    }
    if let Some(e) = &req.error {
        body.push_str("\n--- error ---\n");
        body.push_str(e);
    }
    let meta = merge_meta(
        json!({"evidence": "db", "engine": req.engine, "row_count": req.row_count,
               "columns": req.columns}),
        req.metadata.clone().unwrap_or(Value::Null),
    );
    let art = add_content_artifact(
        ctx,
        pack,
        ProofArtifactKind::Db,
        &req.title,
        Some(&body),
        None,
        status,
        meta,
        by,
    )
    .await?;
    recompute_and_emit(ctx, &pack.id).await?;
    Ok(art)
}

/// Capture a Kafka read result as a `kafka` artifact.
pub async fn attach_kafka_evidence(
    ctx: &ServerCtx,
    pack: &ProofPack,
    req: &otto_core::api::KafkaEvidenceReq,
    by: &str,
) -> Result<ProofArtifact> {
    let count = req.message_count.unwrap_or(0);
    let status = if req.error.is_some() {
        ProofArtifactStatus::Failed
    } else if count > 0 {
        ProofArtifactStatus::Passed
    } else {
        ProofArtifactStatus::Info
    };
    let mut body = format!("topic: {}\nmessages: {}\n", req.topic, count);
    if req.truncated.unwrap_or(false) {
        body.push_str("(truncated)\n");
    }
    if let Some(s) = &req.sample {
        body.push_str("\n--- sample ---\n");
        body.push_str(s);
    }
    if let Some(e) = &req.error {
        body.push_str("\n--- error ---\n");
        body.push_str(e);
    }
    let meta = merge_meta(
        json!({"evidence": "kafka", "topic": req.topic, "message_count": count,
               "truncated": req.truncated}),
        req.metadata.clone().unwrap_or(Value::Null),
    );
    let art = add_content_artifact(
        ctx,
        pack,
        ProofArtifactKind::Kafka,
        &req.title,
        Some(&body),
        None,
        status,
        meta,
        by,
    )
    .await?;
    recompute_and_emit(ctx, &pack.id).await?;
    Ok(art)
}

/// Allowed media MIME types (R4). Anything else is rejected (415).
pub const ALLOWED_MEDIA_MIMES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/svg+xml",
    "video/mp4",
    "video/webm",
];

/// Store a media blob (screenshot/video) and create the owning artifact.
#[allow(clippy::too_many_arguments)]
pub async fn attach_media(
    ctx: &ServerCtx,
    pack: &ProofPack,
    kind: ProofArtifactKind,
    title: &str,
    mime: &str,
    data: &[u8],
    extra: Value,
    by: &str,
) -> Result<ProofArtifact> {
    let sha = otto_core::proof::bytes_sha256(data);
    let meta = merge_meta(
        extra,
        json!({"ref_kind": "blob", "mime": mime, "size_bytes": data.len(), "sha256": sha}),
    );
    let art = ctx
        .proof_repo
        .add_artifact(
            &pack.id,
            &pack.workspace_id,
            kind,
            title,
            None,
            ProofArtifactStatus::Info,
            &meta,
            by,
        )
        .await?;
    ctx.proof_repo
        .add_blob(&art.id, &pack.workspace_id, &sha, mime, data)
        .await?;
    ctx.proof_repo
        .set_artifact_ref(&art.id, &format!("blob:{}", art.id))
        .await?;
    recompute_and_emit(ctx, &pack.id).await?;
    ctx.proof_repo.get_artifact(&art.id).await
}

/// Run the PR-description consistency check against the pack's actual change and
/// test evidence, storing a `pr_check` artifact. Returns the report.
pub async fn run_pr_check(
    ctx: &ServerCtx,
    pack: &ProofPack,
    title: &str,
    description: &str,
    base: Option<&str>,
    cwd: Option<&str>,
    by: &str,
) -> Result<otto_core::proof::PrConsistencyReport> {
    // Files + LOC from the actual diff (best-effort).
    let mut files_changed: Vec<String> = Vec::new();
    let (mut additions, mut deletions) = (0u32, 0u32);
    if let Some(c) = cwd {
        let git = LocalGit::new(c);
        // The PR's changes are `base..HEAD`, NOT the base commit's own patch.
        // `DiffTarget::Commit(b)` is `git show <b>` (the wrong fileset) — the same
        // trap `assemble_diff` documents. Use the range so files_changed/LOC
        // describe the actual PR.
        let resp = match base {
            Some(b) => git.diff(DiffTarget::Range(b.to_string(), "HEAD".to_string()), None).await,
            None => git.diff(DiffTarget::Working, None).await,
        };
        if let Ok(r) = resp {
            files_changed = r.files.iter().map(|f| f.path.clone()).collect();
            additions = r.files.iter().filter_map(|f| f.added).sum();
            deletions = r.files.iter().filter_map(|f| f.deleted).sum();
        }
    }
    // Test evidence already on the pack.
    let arts = ctx.proof_repo.list_artifacts(&pack.id).await?;
    let has_passing_tests = arts
        .iter()
        .any(|a| otto_core::proof::is_test_artifact(a) && a.status == ProofArtifactStatus::Passed);
    let has_failing_tests = arts
        .iter()
        .any(|a| otto_core::proof::is_test_artifact(a) && a.status == ProofArtifactStatus::Failed);

    // Redact before the check (D6 — one trust boundary; heuristics still work).
    let title_red = redact::redact_text(title).value;
    let desc_red = redact::redact_text(description).value;

    let report = otto_core::proof::check_pr_consistency(&otto_core::proof::PrConsistencyInput {
        title: title_red.clone(),
        description: desc_red.clone(),
        files_changed,
        additions,
        deletions,
        has_passing_tests,
        has_failing_tests,
    });

    // Pure, testable mapping (see `pr_check_artifact_status`): a *dishonest* PR
    // fails the pack; a passing check is positive evidence; an honest-but-thin
    // description is neutral — which keeps auto-running it on every PR non-disruptive.
    let status = otto_core::proof::pr_check_artifact_status(&report);
    let mut body = format!(
        "PR consistency: {}/100 ({})\n",
        report.score,
        if report.hard_fail {
            "FAILED — inconsistent with the change"
        } else if report.passed {
            "passed"
        } else {
            "weak — review the description"
        }
    );
    for c in &report.checks {
        body.push_str(&format!(
            "[{}] {} — {}\n",
            if c.passed { "x" } else { " " },
            c.label,
            c.detail
        ));
    }
    let meta = json!({
        "evidence": "pr_check",
        "score": report.score,
        "hard_fail": report.hard_fail,
        "report": serde_json::to_value(&report).unwrap_or(Value::Null),
    });
    upsert_content_artifact(
        ctx,
        pack,
        ProofArtifactKind::PrCheck,
        "PR consistency",
        &body,
        status,
        meta,
        by,
    )
    .await?;
    recompute_and_emit(ctx, &pack.id).await?;
    Ok(report)
}

/// Build + persist an immutable snapshot of the pack's current evidence with a
/// frozen, tamper-evident bundle and rendered Markdown/HTML reports.
pub async fn make_snapshot(
    ctx: &ServerCtx,
    pack: &ProofPack,
    note: &str,
    by: &str,
) -> Result<otto_state::ProofSnapshotRow> {
    use otto_core::proof::SNAPSHOT_ARTIFACT_CAP;
    let pack = ctx.proof_repo.get_pack(&pack.id).await?;
    let arts = ctx.proof_repo.list_artifacts(&pack.id).await?;
    let policy = policy_for_pack(ctx, &pack).await;
    let contract = compute_done_contract(&pack, &arts, &policy);
    let badges = badge_strings(&pack, &arts);
    let generated_at = chrono::Utc::now().to_rfc3339();

    // Frozen artifact list — content capped to SNAPSHOT_ARTIFACT_CAP; full
    // content_sha256 preserved for tamper-evidence.
    let arts_json: Vec<Value> = arts
        .iter()
        .map(|a| {
            let capped = a.content_ref.as_ref().map(|c| {
                if c.len() > SNAPSHOT_ARTIFACT_CAP {
                    let mut s = c
                        .char_indices()
                        .take_while(|(i, _)| *i <= SNAPSHOT_ARTIFACT_CAP)
                        .last()
                        .map(|(i, _)| c[..i].to_string())
                        .unwrap_or_default();
                    s.push_str("\n…(truncated in snapshot)");
                    s
                } else {
                    c.clone()
                }
            });
            json!({
                "id": a.id, "kind": a.kind.as_str(), "title": a.title,
                "status": a.status.as_str(), "content": capped,
                "content_sha256": a.content_sha256, "metadata": a.metadata,
                "created_at": a.created_at,
            })
        })
        .collect();

    let bundle = json!({
        "pack": &pack,
        "badges": &badges,
        "done_contract": &contract,
        "artifacts": arts_json,
        "generated_at": generated_at,
    });
    let sha256 = otto_core::proof::bundle_sha256(&bundle);

    let view = otto_core::proof::ReportView {
        pack: &pack,
        artifacts: &arts,
        contract: &contract,
        badges: &badges,
        generated_at: &generated_at,
    };
    let report_md = otto_core::proof::render_report_md(&view);
    let report_html = otto_core::proof::render_report_html(&view);
    let bundle_json = serde_json::to_string(&bundle)
        .map_err(|e| Error::Internal(format!("serialize snapshot bundle: {e}")))?;

    ctx.proof_repo
        .create_snapshot(
            &pack.id,
            &pack.workspace_id,
            &sha256,
            pack.status.as_str(),
            contract.score,
            pack.risk_score,
            &bundle_json,
            &report_md,
            &report_html,
            note,
            by,
        )
        .await
}

/// Render a live (non-frozen) Markdown/HTML report for a pack.
pub async fn render_report(ctx: &ServerCtx, pack_id: &str, html: bool) -> Result<String> {
    let pack = ctx.proof_repo.get_pack(pack_id).await?;
    let arts = ctx.proof_repo.list_artifacts(pack_id).await?;
    let policy = policy_for_pack(ctx, &pack).await;
    let contract = compute_done_contract(&pack, &arts, &policy);
    let badges = badge_strings(&pack, &arts);
    let generated_at = chrono::Utc::now().to_rfc3339();
    let view = otto_core::proof::ReportView {
        pack: &pack,
        artifacts: &arts,
        contract: &contract,
        badges: &badges,
        generated_at: &generated_at,
    };
    Ok(if html {
        otto_core::proof::render_report_html(&view)
    } else {
        otto_core::proof::render_report_md(&view)
    })
}

/// Resolve the registered repo whose path contains `cwd` (longest-prefix match).
pub async fn resolve_repo_for_cwd(ctx: &ServerCtx, ws_id: &str, cwd: &str) -> Option<String> {
    let repos = ctx.git_store.list_repos(&ws_id.to_string()).await.ok()?;
    repos
        .into_iter()
        .filter(|r| cwd == r.path || cwd.starts_with(&format!("{}/", r.path)))
        .max_by_key(|r| r.path.len())
        .map(|r| r.id)
}

/// The done-contract for a pack, recomputed live (used by the detail route).
pub async fn live_contract(ctx: &ServerCtx, pack: &ProofPack, arts: &[ProofArtifact]) -> otto_core::proof::DoneContract {
    let policy = policy_for_pack(ctx, pack).await;
    compute_done_contract(pack, arts, &policy)
}

/// Badge strings for a pack (used by the route DTOs).
pub fn badge_strings(pack: &ProofPack, arts: &[ProofArtifact]) -> Vec<String> {
    compute_badges(pack, arts)
        .into_iter()
        .map(|b: ProofBadge| b.as_str().to_string())
        .collect()
}

/// Preview helper re-exported for routes.
pub fn preview(content: &str) -> (String, bool) {
    core_preview(content)
}

/// Resolve the workspace a pack belongs to, erroring if not found.
pub async fn pack_workspace(ctx: &ServerCtx, pack_id: &str) -> Result<String> {
    Ok(ctx.proof_repo.get_pack(pack_id).await?.workspace_id)
}

/// Look up the proof pack for a work item, returning None if absent. Convenience
/// for gates that should not create a pack (e.g. the PR gate read path).
pub async fn pack_for_work_item(
    ctx: &ServerCtx,
    kind: WorkItemKind,
    work_item_id: &str,
) -> Result<Option<ProofPack>> {
    ctx.proof_repo.find_by_work_item(kind, work_item_id).await
}

// ---------------------------------------------------------------------------
// Session gate (the all-done edge)
// ---------------------------------------------------------------------------

/// Detect the repo's test command from `cwd`, if recognizable. Conservative:
/// only returns a command when a test runner is reliably inferable.
fn detect_test_command(cwd: &str) -> Option<String> {
    let p = std::path::Path::new(cwd);
    if p.join("Cargo.toml").is_file() {
        return Some("cargo test".to_string());
    }
    if p.join("go.mod").is_file() {
        return Some("go test ./...".to_string());
    }
    let pkg = p.join("package.json");
    if pkg.is_file() {
        if let Ok(s) = std::fs::read_to_string(&pkg) {
            // Only if a `test` script is declared (avoid npm's "missing script").
            if s.contains("\"test\"") {
                return Some("npm test".to_string());
            }
        }
    }
    None
}

/// Whether session-done auto-test is enabled. Default OFF: running a repo's test
/// suite from the daemon in the user's live cwd can be disruptive/expensive, so
/// it is opt-in via `OTTO_PROOF_AUTO_TEST=1`. The diff is always assembled.
fn auto_test_enabled() -> bool {
    std::env::var("OTTO_PROOF_AUTO_TEST")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Gate a session that just reported all tasks done: ensure a pack, link it to a
/// parent (goal-loop) pack when applicable, assemble the working-tree diff, run
/// the repo's tests when enabled, recompute, and surface a Notice when the
/// evidence is incomplete. Best-effort throughout — never fails the caller.
pub async fn gate_session(ctx: &ServerCtx, session: &otto_core::domain::Session) {
    let pack = match gate(
        ctx,
        WorkItemKind::Session,
        &session.id,
        &session.workspace_id,
        &session.title,
        &session.created_by,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!("proof gate_session ensure failed: {e}");
            return;
        }
    };

    // Rollup: link a goal-loop-spawned session's pack to the loop's pack (D21).
    if session.meta.get("source").and_then(|v| v.as_str()) == Some("goal_loop") {
        if let Some(loop_id) = session.meta.get("loop_id").and_then(|v| v.as_str()) {
            if let Ok(Some(parent)) =
                pack_for_work_item(ctx, WorkItemKind::GoalLoop, loop_id).await
            {
                let _ = ctx.proof_repo.set_parent(&pack.id, &parent.id).await;
            }
        }
    }

    // Link the pack to a registered repo (best-effort) so per-repo proof policy
    // (R3: "test command required") applies on recompute, and learn its
    // configured test command.
    let repo_test_cmd = match resolve_repo_for_cwd(ctx, &session.workspace_id, &session.cwd).await {
        Some(repo_id) => {
            let _ = ctx.proof_repo.set_repo_link(&pack.id, Some(&repo_id), None).await;
            ctx.git_store
                .get_proof_config(&repo_id)
                .await
                .ok()
                .and_then(|c| c.test_cmd)
        }
        None => None,
    };

    // Always assemble the diff (best-effort).
    let _ = assemble_diff(ctx, &pack, &session.cwd, None).await;

    // Optionally auto-run the repo's tests — prefer the repo's declared command.
    if auto_test_enabled() {
        let cmd = repo_test_cmd.or_else(|| detect_test_command(&session.cwd));
        if let Some(cmd) = cmd {
            let _ = run_command_artifact(ctx, &pack, &session.cwd, &cmd, Some("test")).await;
        }
    }

    let updated = match recompute_and_emit(ctx, &pack.id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!("proof gate_session recompute failed: {e}");
            return;
        }
    };

    // Surface a non-blocking nudge when the proof is incomplete (the badge is the
    // durable signal; this is a one-shot toast on the done edge).
    if matches!(updated.status, ProofStatus::Missing | ProofStatus::Partial) {
        let _ = ctx.events.send(Event::Notice {
            level: "warn".into(),
            title: "Tasks done — proof incomplete".into(),
            body: format!(
                "{} reported done but its proof pack is {}. Add tests / a self-review to the proof pack.",
                session.title,
                updated.status.as_str()
            ),
        });
    }
}

// ---------------------------------------------------------------------------
// PR-creation gate (hard teeth)
// ---------------------------------------------------------------------------

/// Whether the PR-creation gate is enforced. Default ON — opening a PR over an
/// unproven proof pack is blocked unless explicitly overridden. Disable with
/// `OTTO_PROOF_REQUIRE_PR=0`.
pub fn pr_gate_enabled() -> bool {
    std::env::var("OTTO_PROOF_REQUIRE_PR")
        .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
        .unwrap_or(true)
}

/// Gate a PR creation on its linked proof pack. Returns `Err(Conflict)` when the
/// linked pack is not `passed`/`waived` and the caller did not pass
/// `allow_unproven`. A PR with no linked pack is not gated (Otto can't enforce
/// evidence it can't locate). The override is recorded as an audit artifact.
pub async fn gate_pr(
    ctx: &ServerCtx,
    _workspace_id: &str,
    req: &otto_core::api::CreatePrReq,
) -> Result<()> {
    let Some(pack_id) = req.proof_pack_id.as_deref() else {
        return Ok(()); // unlinked PR — nothing to gate
    };
    if !pr_gate_enabled() {
        return Ok(());
    }
    let pack = match ctx.proof_repo.get_pack(pack_id).await {
        Ok(p) => p,
        Err(_) => return Ok(()), // unknown pack — don't block the user
    };
    if !pr_should_block(pack.status, req.allow_unproven.unwrap_or(false)) {
        // Allowed: either the pack is proven, or the caller overrode. Record the
        // override (only) as an audit artifact.
        if !matches!(pack.status, ProofStatus::Passed | ProofStatus::Waived) {
            let _ = add_content_artifact(
                ctx,
                &pack,
                ProofArtifactKind::Approval,
                "PR opened over unproven proof",
                Some("A pull request was opened despite an unproven proof pack (explicit override)."),
                None,
                ProofArtifactStatus::Passed,
                serde_json::json!({ "override": true, "kind": "pr_override" }),
                "otto",
            )
            .await;
            let _ = recompute_and_emit(ctx, &pack.id).await;
        }
        return Ok(());
    }
    Err(Error::Conflict(format!(
        "proof pack {} is '{}', not passed — provide evidence or open with allow_unproven to override",
        pack.id,
        pack.status.as_str()
    )))
}

/// Whether a PR over a pack with `status` should be blocked, given the caller's
/// `allow_unproven` flag. Pure — the decision core of [`gate_pr`].
fn pr_should_block(status: ProofStatus, allow_unproven: bool) -> bool {
    !matches!(status, ProofStatus::Passed | ProofStatus::Waived) && !allow_unproven
}

/// Map an error to a not-found-tolerant unit (used by best-effort gates).
pub fn ignore_err<T>(r: Result<T>) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::debug!("proof gate best-effort error: {e}");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (pure helpers; DB-touching gate/recompute tested in tests/proof_engine.rs)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risky_file_segments_not_substrings() {
        assert!(is_risky_file("crates/otto-state/migrations/0077.sql"));
        assert!(is_risky_file("crates/otto-server/src/policy.rs"));
        assert!(is_risky_file("crates/otto-rbac/src/auth.rs"));
        assert!(is_risky_file(".github/workflows/ci.yml"));
        assert!(is_risky_file("ui/package-lock.json"));
        assert!(is_risky_file("Cargo.lock"));
        // false positives the naive substring approach would hit:
        assert!(!is_risky_file("crates/otto-core/src/author.rs"));
        assert!(!is_risky_file("ui/src/lib/tokenizer.ts"));
        assert!(!is_risky_file("crates/otto-server/src/modules.rs"));
    }

    #[test]
    fn classify_kinds() {
        assert_eq!(classify_test_kind("cargo clippy --workspace"), "lint");
        assert_eq!(classify_test_kind("go vet ./..."), "lint");
        assert_eq!(classify_test_kind("npm run build"), "build");
        assert_eq!(classify_test_kind("cargo test --workspace"), "test");
    }

    #[tokio::test]
    async fn run_command_captures_success_and_failure() {
        let here = ".";
        let ok = run_command(here, "true", 30).await;
        assert!(ok.success && ok.exit_code == 0);
        let bad = run_command(here, "exit 3", 30).await;
        assert!(!bad.success && bad.exit_code == 3);
        let echo = run_command(here, "echo hello", 30).await;
        assert!(echo.output.contains("hello"));
    }

    #[test]
    fn prepare_content_caps_and_reports() {
        let (small, meta) = prepare_content("hello");
        assert_eq!(small, "hello");
        assert_eq!(meta["truncated"], json!(false));
        let big = "x".repeat(STORE_CAP + 1000);
        let (capped, meta) = prepare_content(&big);
        assert!(capped.len() <= STORE_CAP + 32);
        assert_eq!(meta["truncated"], json!(true));
    }

    #[test]
    fn pr_gate_decision() {
        // Unproven pack with no override → block.
        assert!(pr_should_block(ProofStatus::Failed, false));
        assert!(pr_should_block(ProofStatus::Missing, false));
        assert!(pr_should_block(ProofStatus::Partial, false));
        // Proven (or waived) → never block.
        assert!(!pr_should_block(ProofStatus::Passed, false));
        assert!(!pr_should_block(ProofStatus::Waived, false));
        // Override lets an unproven pack through.
        assert!(!pr_should_block(ProofStatus::Failed, true));
    }

    #[test]
    fn prepare_content_redacts_secrets() {
        // A trust layer must not itself leak secrets (D6). A Bearer token in
        // captured output must be redacted before storage.
        let raw = "running with Authorization: Bearer abcdef0123456789ABCDEF0123456789 in the log";
        let (stored, meta) = prepare_content(raw);
        assert!(
            !stored.contains("abcdef0123456789ABCDEF0123456789"),
            "secret must be redacted from stored content"
        );
        assert!(
            meta["redactions"].as_u64().unwrap_or(0) >= 1,
            "redaction count should be reported"
        );
    }
}
