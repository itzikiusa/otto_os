//! The `post_review_comments` step (engine.md §7.3, DR-004): the posting
//! part of "Post Inline PR Comments" in code. A workflow review's drafts are
//! stored with PR 0 (a local review), so the PR number comes from the run's
//! `prs.json` (admission and `git_checkout` write it), never from the review
//! row. Drafts of the chosen severities go inline (at most `max_inline`, the
//! rest in one "More findings" comment), the others in one combined comment,
//! then one summary; a draft already on the PR (same path, line and first
//! line) is skipped, and every listed draft is claimed so a resumed step
//! never posts it twice. The daemon hands in the forge and the one-draft post
//! ([`PostHost`]); automated posts record no learning.

use crate::context::RunContextFiles;
use crate::run_refs::{RunPr, PRS_FILE};
use otto_core::domain::{CommentSeverity, CommentState, Review, ReviewComment};
use otto_core::workflows::{Deliverable, NodeStatus, WorkflowCheckpoint, WorkflowNode};
use otto_core::{Id, Result};
use otto_state::{DbPool, ReviewsRepo, WorkflowsRepo};
use serde_json::{json, Value};
use std::sync::Arc;

/// What the step needs from the daemon.
#[async_trait::async_trait]
pub trait PostHost: Send + Sync {
    /// The forge of a registered repo, acting as the run's user.
    async fn forge(
        &self,
        repo_id: &str,
    ) -> Result<(Arc<dyn otto_git::GitProvider>, otto_git::RemoteRef)>;
    /// Post one claimed draft to PR `number` (anchor fallback included); the
    /// forge's error text on failure.
    async fn post(
        &self,
        repo_id: &str,
        number: u64,
        c: &ReviewComment,
    ) -> std::result::Result<(), String>;
}

/// Above this many inline posts the forge may refuse (Bitbucket's 200 per
/// PR; one export run lost 54), so the rest go into one comment.
const DEFAULT_MAX_INLINE: usize = 150;

/// Run the step on the run's state as stored: `(output, logs)`.
pub async fn run(
    host: &dyn PostHost,
    pool: &DbPool,
    files: &RunContextFiles,
    run_id: &Id,
    node: &WorkflowNode,
) -> Result<(Value, Vec<String>)> {
    let reviews = ReviewsRepo::new(pool.clone());
    let p = &node.params;
    let repo = WorkflowsRepo::new(pool.clone());
    let run = repo.get_run(run_id).await?;
    let from = p.get("review_from").and_then(Value::as_str);
    // The named review step, else the latest review that ran before this one.
    let review = run
        .nodes
        .iter()
        .filter(|n| from.is_none_or(|f| n.node_id == f))
        .filter(|n| {
            n.output
                .as_ref()
                .is_some_and(|o| o.get("review_id").is_some())
        })
        .max_by_key(|n| n.step_no)
        .and_then(|n| n.output.clone())
        .ok_or_else(|| {
            otto_core::Error::Invalid(
                "nothing to post: no completed review step before this one".into(),
            )
        })?;
    let rid = review
        .get("review_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if review.get("outcome").and_then(Value::as_str) == Some("incomplete") {
        return Err(otto_core::Error::Upstream(format!(
            "nothing to post: review {rid} is incomplete — no PR comment was posted"
        )));
    }
    let inline: Vec<CommentSeverity> = match p.get("inline").and_then(Value::as_array) {
        Some(a) => a
            .iter()
            .filter_map(Value::as_str)
            .filter_map(CommentSeverity::parse)
            .collect(),
        None => vec![CommentSeverity::Bug, CommentSeverity::Warn],
    };
    let opts = Opts {
        inline,
        info: p.get("info").and_then(Value::as_str) != Some("skip"),
        summary: p.get("summary").and_then(Value::as_bool).unwrap_or(true),
        max_inline: p
            .get("max_inline")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_MAX_INLINE, |n| n as usize),
        intro: p
            .get("summary_intro_from")
            .and_then(Value::as_str)
            .and_then(|id| run.nodes.iter().find(|n| n.node_id == id))
            .and_then(|n| n.step_file.as_deref())
            .and_then(|base| files.read_confined(&format!("{base}.md"))),
        files: p
            .get("summary_files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|f| crate::context::is_bare_name(f))
            .filter_map(|f| files.read_confined(f).map(|t| (f.to_string(), t)))
            .collect(),
    };
    let prs: Vec<RunPr> = files
        .read_named(PRS_FILE)
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let targets: Vec<Value> = match review.get("reviews").and_then(Value::as_array) {
        Some(per_repo) => per_repo.clone(),
        None => vec![review.clone()],
    };
    let mut tally = Tally::default();
    let mut logs = Vec::new();
    let mut pr_link = Value::Null;
    for (i, t) in targets.iter().enumerate() {
        let Some(review_id) = t.get("review_id").and_then(Value::as_str) else {
            continue;
        };
        let stored = reviews.get_review(&review_id.into()).await?;
        let pr = prs
            .iter()
            .find(|pr| pr.repo_id == stored.repo_id)
            .map(|pr| (pr.number, pr.url.clone()))
            .or_else(|| (stored.pr_number != 0).then_some((stored.pr_number, None)));
        let Some((number, url)) = pr else {
            tally.local += 1;
            logs.push(format!(
                "review {review_id}: no PR (local review) — nothing posted"
            ));
            continue;
        };
        pr_link = json!({"number": number, "url": url});
        let checkpoint = format!("{}#summary.{i}", node.id);
        let summary_done = repo.checkpoint(run_id, &checkpoint).await?.is_some();
        let posted = post_target(
            host,
            &reviews,
            &stored,
            number,
            url.as_deref(),
            t,
            &opts,
            summary_done,
            &mut tally,
            &mut logs,
        )
        .await?;
        if posted {
            let cp = WorkflowCheckpoint {
                node_id: checkpoint.clone(),
                loop_id: node.id.clone(),
                iteration: 0,
                step_index: i,
                kind: node.kind.clone(),
                name: format!(
                    "{} — summary",
                    if node.name.trim().is_empty() {
                        &node.kind
                    } else {
                        &node.name
                    }
                ),
                status: NodeStatus::Success,
                attempts: 1,
                input: Value::Null,
                output: None,
                error: None,
                logs: vec![],
                updated_at: chrono::Utc::now(),
                active_ms: 0,
                thread_id: None,
                start_ordinal: None,
            };
            repo.save_checkpoint(run_id, &cp).await?;
        }
    }
    let total = tally.posted + tally.failed.len();
    if total > 0 && tally.posted == 0 && tally.summaries == 0 {
        return Err(otto_core::Error::Upstream(format!(
            "every post failed: {}",
            tally.failed.join("; ")
        )));
    }
    let mut deliverables: Vec<Deliverable> = Vec::new();
    if tally.local > 0 && total == 0 {
        deliverables.push(Deliverable {
            label: "PR comments".into(),
            state: otto_core::workflows::DeliverableState::Done,
            detail: Some("no PR (local review)".into()),
            url: None,
        });
    } else {
        // A clean PR (no findings) has only its summary to show.
        if total > 0 || tally.summaries == 0 {
            let mut d =
                crate::deliverables::tally("PR comments", "posted", tally.posted, &tally.failed, 0);
            d.url = pr_link
                .get("url")
                .and_then(Value::as_str)
                .map(str::to_string);
            deliverables.push(d);
        }
        if tally.summaries > 0 {
            deliverables.push(crate::deliverables::tally(
                "PR summary comment",
                "posted",
                tally.summaries,
                &[],
                0,
            ));
        }
    }
    for f in &tally.failed {
        logs.push(format!("⚠ {f}"));
    }
    let mut out = review.as_object().cloned().unwrap_or_default();
    out.insert("posted".into(), json!(tally.posted));
    out.insert("failed".into(), json!(tally.failed.len()));
    out.insert("duplicates".into(), json!(tally.duplicates));
    out.insert("overflow".into(), json!(tally.overflow));
    out.insert("pr".into(), pr_link);
    out.insert("deliverables".into(), json!(deliverables));
    Ok((Value::Object(out), logs))
}

struct Opts {
    inline: Vec<CommentSeverity>,
    info: bool,
    summary: bool,
    max_inline: usize,
    intro: Option<String>,
    files: Vec<(String, String)>,
}

#[derive(Default)]
struct Tally {
    posted: usize,
    failed: Vec<String>,
    duplicates: usize,
    overflow: usize,
    summaries: usize,
    local: usize,
}

fn first_line(s: &str) -> &str {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

fn sev(s: CommentSeverity) -> &'static str {
    match s {
        CommentSeverity::Bug => "bug",
        CommentSeverity::Warn => "warn",
        CommentSeverity::Info => "info",
    }
}

fn cite(c: &ReviewComment) -> String {
    match (&c.path, c.line) {
        (Some(p), Some(l)) => format!("`{p}:{l}`"),
        (Some(p), None) => format!("`{p}`"),
        _ => String::new(),
    }
}

/// Post one review target's drafts to PR `number`. True when the summary
/// was posted now (the caller checkpoints it).
#[allow(clippy::too_many_arguments)]
async fn post_target(
    host: &dyn PostHost,
    reviews: &ReviewsRepo,
    review: &Review,
    number: u64,
    url: Option<&str>,
    out: &Value,
    opts: &Opts,
    summary_done: bool,
    tally: &mut Tally,
    logs: &mut Vec<String>,
) -> Result<bool> {
    let (provider, remote) = host.forge(&review.repo_id).await?;
    let existing: Vec<(Option<String>, Option<u32>, String)> = provider
        .get_pr(&remote, number)
        .await?
        .comments
        .iter()
        .map(|c| (c.path.clone(), c.line, first_line(&c.body).to_string()))
        .collect();
    let on_pr = |c: &ReviewComment| {
        existing
            .iter()
            .any(|(p, l, f)| *p == c.path && *l == c.line && f == first_line(&c.body))
    };
    let mut drafts: Vec<&ReviewComment> = review
        .comments
        .iter()
        .filter(|c| c.state == CommentState::Draft && !c.posted)
        .collect();
    drafts.sort_by_key(|c| (std::cmp::Reverse(c.severity as u8), c.path.clone(), c.line));
    let (inline, info): (Vec<&ReviewComment>, Vec<&ReviewComment>) = drafts
        .into_iter()
        .partition(|c| opts.inline.contains(&c.severity) && c.path.is_some() && c.line.is_some());
    // `max_inline` and the log line are per PR; the tally is the run's.
    let before = (tally.posted, tally.duplicates, tally.overflow);
    let mut more = Vec::new();
    for c in inline {
        if on_pr(c) {
            tally.duplicates += 1;
            continue;
        }
        if tally.posted - before.0 >= opts.max_inline {
            more.push(c);
            continue;
        }
        match post_draft(host, reviews, &review.repo_id, number, c).await {
            Ok(()) => tally.posted += 1,
            Err(e) => tally.failed.push(format!("{}: {e}", cite(c))),
        }
    }
    tally.overflow += more.len();
    let general = |title: &str, items: &[&ReviewComment]| {
        let mut body = format!("**{title}**\n\n");
        for c in items {
            body.push_str(&format!(
                "- [{}] {} {}\n",
                sev(c.severity),
                cite(c),
                c.body.trim()
            ));
        }
        body
    };
    // The drafts a general comment lists are claimed first, so a resumed
    // step never lists one twice.
    let mut bodies = Vec::new();
    let more = claim(reviews, more).await;
    if !more.is_empty() {
        bodies.push((
            "More findings",
            general(&format!("More findings ({})", more.len()), &more),
            more,
        ));
    }
    let info = match opts.info {
        true => claim(reviews, info.into_iter().filter(|c| !on_pr(c)).collect()).await,
        false => vec![],
    };
    if !info.is_empty() {
        bodies.push((
            "Other findings",
            general(&format!("Other findings ({})", info.len()), &info),
            info,
        ));
    }
    if opts.summary && !summary_done {
        let body = summary_body(out, review, number, url, opts, worktree_head(out).await);
        bodies.push(("Summary", body, vec![]));
    }
    let mut summary_posted = false;
    for (what, body, listed) in bodies {
        let req = otto_core::api::NewPrCommentReq {
            body,
            path: None,
            line: None,
            in_reply_to: None,
            side: None,
            old_line: None,
            commit_id: None,
        };
        let ok = provider.comment(&remote, number, &req).await;
        for c in &listed {
            let _ = match ok {
                Ok(_) => reviews
                    .set_comment_state(&c.id, CommentState::Approved, true)
                    .await
                    .map(|_| ()),
                Err(_) => reviews.release_comment_post(&c.id).await,
            };
        }
        match ok {
            Ok(_) if what == "Summary" => {
                tally.summaries += 1;
                summary_posted = true;
            }
            Ok(_) => tally.posted += 1,
            Err(e) => tally.failed.push(format!("{what} comment: {e}")),
        }
    }
    logs.push(format!(
        "PR #{number}: {} posted, {} duplicates skipped, {} in \"More findings\"",
        tally.posted - before.0,
        tally.duplicates - before.1,
        tally.overflow - before.2
    ));
    Ok(summary_posted)
}

/// The drafts whose post this call claimed (posted 0→1).
async fn claim<'a>(reviews: &ReviewsRepo, items: Vec<&'a ReviewComment>) -> Vec<&'a ReviewComment> {
    let mut out = Vec::new();
    for c in items {
        if matches!(reviews.claim_comment_post(&c.id).await, Ok(true)) {
            out.push(c);
        }
    }
    out
}

/// The reviewed HEAD: the target's worktree as the review left it.
async fn worktree_head(out: &Value) -> Option<String> {
    let wt = out.get("worktree").and_then(Value::as_str)?;
    let o = otto_git::hardened_command()
        .current_dir(wt)
        .args(["rev-parse", "HEAD"])
        .output()
        .await
        .ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn summary_body(
    out: &Value,
    review: &Review,
    number: u64,
    url: Option<&str>,
    opts: &Opts,
    head: Option<String>,
) -> String {
    let mut md = String::new();
    if let Some(intro) = opts
        .intro
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        md.push_str(intro);
        md.push_str("\n\n");
    }
    let verdict = if out.get("passed").and_then(Value::as_bool) == Some(true) {
        "PASSED"
    } else {
        "NOT PASSED"
    };
    let score = out
        .get("score_text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| out.get("score").map_or("—".into(), |s| s.to_string()));
    let threshold = out.get("threshold").and_then(Value::as_u64).unwrap_or(80);
    md.push_str(&format!(
        "**Review {verdict}** — score {score}/100 (pass ≥ {threshold})\n\n"
    ));
    let count = |s: CommentSeverity| review.comments.iter().filter(|c| c.severity == s).count();
    md.push_str(&format!(
        "Findings: {} bug · {} warn · {} info\n",
        count(CommentSeverity::Bug),
        count(CommentSeverity::Warn),
        count(CommentSeverity::Info)
    ));
    if let Some(h) = head {
        md.push_str(&format!("Reviewed HEAD: `{h}`\n"));
    }
    match url {
        Some(u) => md.push_str(&format!("PR: [#{number}]({u})\n")),
        None => md.push_str(&format!("PR: #{number}\n")),
    }
    for (name, text) in &opts.files {
        md.push_str(&format!("\n---\n\n**{name}**\n\n{}\n", text.trim()));
    }
    md
}

/// One stored draft onto the PR, at most once: claimed (posted 0→1) before
/// the forge call, the claim released on failure, then marked approved and
/// posted. A draft declined while claimed is not posted.
async fn post_draft(
    host: &dyn PostHost,
    store: &ReviewsRepo,
    repo_id: &str,
    number: u64,
    c: &ReviewComment,
) -> std::result::Result<(), String> {
    if !store
        .claim_comment_post(&c.id)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(());
    }
    if matches!(store.get_comment(&c.id).await, Ok(f) if f.state == CommentState::Declined) {
        let _ = store.release_comment_post(&c.id).await;
        return Ok(());
    }
    if let Err(e) = host.post(repo_id, number, c).await {
        let _ = store.release_comment_post(&c.id).await;
        return Err(e);
    }
    store
        .set_comment_state(&c.id, CommentState::Approved, true)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}
