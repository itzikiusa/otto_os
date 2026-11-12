//! How a person reads and acts on a finished review — read-time joins only,
//! nothing here changes how a review runs:
//! - [`verdict`]: the verdict, blocker count and one-line summary a reader
//!   sees, computed from the tracked findings (the review row stores none).
//! - [`link_comments`]: which tracked finding each draft comment posts. A
//!   comment (what goes to the PR) and its finding (what is triaged) are two
//!   rows; declining one and dismissing the other must move together
//!   ([`follow_comment`], [`follow_finding`]).
//! - [`posted_copies`]: drafts that repeat a comment another review of the
//!   same PR already posted — "Already on the PR", never posted twice.
//! - [`triage`]: all of it for one review, plus the prior findings a re-run
//!   settled (fixed / still open) so they do not vanish from the board.

use crate::verification::{incomplete_cause, is_unverified, UNVERIFIED_MARK};
use otto_core::domain::{CommentState, Review, ReviewComment, ReviewStatus};
use otto_core::event::Event;
use otto_core::finding::{Finding, FindingSeverity, FindingStatus};
use otto_core::{Error, Id, Result};
use otto_state::review_findings::{normalize_anchor_text, normalize_finding_path};
use otto_state::{FindingEventsRepo, ReviewFindingsRepo, ReviewsRepo};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use tokio::sync::broadcast;

/// What a reader sees at the top of a review. All `None` while a review is
/// running, cancelled, or failed with nothing usable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    /// `incomplete` | `changes_requested` | `approved`.
    pub verdict: Option<String>,
    /// Unresolved findings of blocking severity (critical / high).
    pub blocker_count: Option<u32>,
    /// One line: open findings by severity, plus how many are unverified.
    pub summary: Option<String>,
}

/// A finding still to be dealt with — the same axis merge readiness counts.
fn unresolved(s: FindingStatus) -> bool {
    matches!(
        s,
        FindingStatus::Open | FindingStatus::Accepted | FindingStatus::Fixed
    )
}

/// The verdict of `review` over its tracked `findings` (those whose latest
/// sighting is this review). An incomplete review is never "approved": its
/// claims were not verified, so it says `incomplete` whatever it found.
pub fn verdict(review: &Review, findings: &[Finding]) -> Verdict {
    let incomplete = review.error.as_deref().and_then(incomplete_cause).is_some();
    if !incomplete && review.status != ReviewStatus::Done {
        return Verdict::default();
    }
    let open: Vec<&Finding> = findings.iter().filter(|f| unresolved(f.status)).collect();
    let blockers = open
        .iter()
        .filter(|f| crate::engine::is_blocking_severity(f.severity.as_str()))
        .count() as u32;
    let verdict = match (incomplete, blockers) {
        (true, _) => "incomplete",
        (false, 0) => "approved",
        (false, _) => "changes_requested",
    };
    let by_severity: Vec<String> = [
        FindingSeverity::Critical,
        FindingSeverity::High,
        FindingSeverity::Medium,
        FindingSeverity::Low,
        FindingSeverity::Info,
    ]
    .into_iter()
    .filter_map(|s| match open.iter().filter(|f| f.severity == s).count() {
        0 => None,
        n => Some(format!("{n} {}", s.as_str())),
    })
    .collect();
    let mut summary = match open.len() {
        0 => "No open findings".to_string(),
        1 => format!("1 open finding: {}", by_severity.join(", ")),
        n => format!("{n} open findings: {}", by_severity.join(", ")),
    };
    let unverified = review
        .comments
        .iter()
        .filter(|c| c.state != CommentState::Declined && is_unverified(&c.body))
        .count();
    if unverified > 0 {
        summary.push_str(&format!(" · {unverified} unverified"));
    }
    Verdict {
        verdict: Some(verdict.into()),
        blocker_count: Some(blockers),
        summary: Some(summary),
    }
}

/// [`verdict`] written onto the review for the read path. Best effort: a
/// failed findings read leaves the fields empty rather than failing the read.
pub async fn with_verdict(findings: &ReviewFindingsRepo, mut review: Review) -> Review {
    match findings.list_full_for_review(&review.id).await {
        Ok(f) => {
            let v = verdict(&review, &f);
            review.verdict = v.verdict;
            review.blocker_count = v.blocker_count;
            review.summary_md = v.summary;
        }
        Err(e) => tracing::warn!(review = %review.id, "review verdict: {e}"),
    }
    review
}

/// [`with_verdict`] for a list of reviews (history, newest first).
pub async fn with_verdicts(findings: &ReviewFindingsRepo, reviews: Vec<Review>) -> Vec<Review> {
    let mut out = Vec::with_capacity(reviews.len());
    for r in reviews {
        out.push(with_verdict(findings, r).await);
    }
    out
}

// ---------------------------------------------------------------------------
// Comment ↔ finding link
// ---------------------------------------------------------------------------

/// A comment body as a key: no unverified mark, alphanumerics only.
fn body_key(body: &str) -> String {
    normalize_anchor_text(body.strip_prefix(UNVERIFIED_MARK).unwrap_or(body))
}

fn path_key(path: Option<&str>) -> &str {
    normalize_finding_path(path.unwrap_or(""))
}

/// How well a finding's text matches a comment on the same anchor: the
/// summarizer promotes each comment with its body (and a title taken from
/// it), so the finding created from THIS comment scores highest.
fn affinity(c: &ReviewComment, f: &Finding) -> u8 {
    let body = body_key(&c.body);
    let title = normalize_anchor_text(&f.title);
    if body == body_key(&f.body) {
        3
    } else if !title.is_empty() && body.starts_with(&title) {
        2
    } else {
        u8::from(!title.is_empty() && body.contains(&title))
    }
}

/// Comment id → finding id, one-to-one. `created` is the audit trail's link
/// (finding id → the comment its `created` event names), exact but written
/// only when a finding is first seen. Otherwise a comment links to the
/// finding last seen in ITS review on the same path + line, preferring the
/// closest text when one line carries several findings.
pub fn link_comments(
    comments: &[ReviewComment],
    findings: &[Finding],
    created: &HashMap<String, String>,
) -> HashMap<Id, Id> {
    let mut out: HashMap<Id, Id> = HashMap::new();
    let mut taken: HashSet<&str> = HashSet::new();
    for f in findings {
        if let Some(cid) = created.get(&f.id) {
            if !out.contains_key(cid) && comments.iter().any(|c| &c.id == cid) {
                out.insert(cid.clone(), f.id.clone());
                taken.insert(&f.id);
            }
        }
    }
    for c in comments {
        if out.contains_key(&c.id) {
            continue;
        }
        let best = findings
            .iter()
            .filter(|f| {
                !taken.contains(f.id.as_str())
                    && f.review_id == c.review_id
                    && path_key(f.path.as_deref()) == path_key(c.path.as_deref())
                    && f.line == c.line
            })
            .max_by_key(|f| affinity(c, f));
        if let Some(f) = best {
            taken.insert(&f.id);
            out.insert(c.id.clone(), f.id.clone());
        }
    }
    out
}

/// The `created` audit links of `findings` (finding id → comment id).
async fn created_links(
    events: &FindingEventsRepo,
    findings: &[Finding],
) -> Result<HashMap<String, String>> {
    let ids: Vec<String> = findings.iter().map(|f| f.id.clone()).collect();
    let by = events.list_for_findings(&ids).await?;
    Ok(by
        .into_iter()
        .filter_map(|(fid, evs)| {
            let cid = evs
                .iter()
                .find(|e| e.kind == "created")?
                .detail
                .get("comment_id")?
                .as_str()?
                .to_string();
            Some((fid, cid))
        })
        .collect())
}

/// The findings seen by `review` and its comments' links to them.
pub async fn review_links(
    findings: &ReviewFindingsRepo,
    events: &FindingEventsRepo,
    review: &Review,
) -> Result<(Vec<Finding>, HashMap<Id, Id>)> {
    let seen = findings.list_seen_for_review(&review.id).await?;
    let created = created_links(events, &seen).await?;
    let links = link_comments(&review.comments, &seen, &created);
    Ok((seen, links))
}

/// What declining a comment records on its finding: "Not a problem"
/// (`false_positive`, the default) or "Won't fix" (`waived`).
pub fn dismissal(kind: Option<&str>) -> Result<FindingStatus> {
    match kind.unwrap_or("false_positive") {
        "false_positive" => Ok(FindingStatus::FalsePositive),
        "waived" => Ok(FindingStatus::Waived),
        other => Err(Error::Invalid(format!(
            "unknown dismissal `{other}` — use false_positive or waived"
        ))),
    }
}

/// Whether a comment action moves a finding in `from` to `to`: a dismissal
/// only closes a finding still to be dealt with (a verified one stays
/// verified), a restore (`Open`) only reopens a dismissed one.
fn follows(from: FindingStatus, to: FindingStatus) -> bool {
    let dismissed = matches!(from, FindingStatus::FalsePositive | FindingStatus::Waived);
    from != to
        && from.can_transition(to)
        && if to == FindingStatus::Open {
            dismissed
        } else {
            unresolved(from)
        }
}

/// The stores the comment ↔ finding link reads and writes, the event bus it
/// tells the open board and panel on, and who acted (the audit actor).
pub struct Links<'a> {
    pub reviews: &'a ReviewsRepo,
    pub findings: &'a ReviewFindingsRepo,
    pub events: &'a FindingEventsRepo,
    pub bus: &'a broadcast::Sender<Event>,
    pub actor: String,
}

/// Carry a comment decision onto its finding: declined → `to` (false
/// positive / waived), restored to draft → `Open`, then broadcast
/// `finding_updated`. `false` when the comment has no finding or the finding
/// is already settled. Never fails the comment action: a failed link is
/// logged.
pub async fn follow_comment(
    l: &Links<'_>,
    review: &Review,
    comment_id: &str,
    to: FindingStatus,
    reason: Option<&str>,
) -> bool {
    let res: Result<Option<Finding>> = async {
        let (seen, links) = review_links(l.findings, l.events, review).await?;
        let Some(f) = links
            .get(comment_id)
            .and_then(|id| seen.iter().find(|f| &f.id == id))
        else {
            return Ok(None);
        };
        if !follows(f.status, to) {
            return Ok(None);
        }
        let updated = l.findings.set_status(&f.id, to, &l.actor).await?;
        let kind = if to == FindingStatus::Open {
            "reopened"
        } else {
            to.as_str()
        };
        l.events
            .append(
                &f.id,
                &f.workspace_id,
                kind,
                &l.actor,
                Some(f.status.as_str()),
                Some(to.as_str()),
                serde_json::json!({ "reason": reason, "comment_id": comment_id }),
            )
            .await?;
        Ok(Some(updated))
    }
    .await;
    match res {
        Ok(Some(f)) => {
            let _ = l.bus.send(Event::FindingUpdated {
                workspace_id: f.workspace_id,
                review_id: f.review_id,
                finding_id: f.id,
                status: f.status.as_str().to_string(),
            });
            true
        }
        Ok(None) => false,
        Err(e) => {
            tracing::warn!(comment = %comment_id, "comment → finding link: {e}");
            false
        }
    }
}

/// A finding was dismissed (false positive / waived): decline the
/// still-draft, unposted comments that would post it, so "Post all" cannot
/// send a dismissed issue, then broadcast `review_changed` so an open panel
/// re-reads its comments. `true` when a comment changed. Never fails the
/// finding action.
pub async fn follow_finding(l: &Links<'_>, finding: &Finding) -> bool {
    if !matches!(
        finding.status,
        FindingStatus::FalsePositive | FindingStatus::Waived
    ) {
        return false;
    }
    let res: Result<Option<Review>> = async {
        let review = l.reviews.get_review(&finding.review_id).await?;
        let (_, links) = review_links(l.findings, l.events, &review).await?;
        let mut changed = false;
        for c in &review.comments {
            let linked = links.get(&c.id) == Some(&finding.id);
            if linked && c.state == CommentState::Draft && !c.posted {
                l.reviews
                    .set_comment_state(&c.id, CommentState::Declined, false)
                    .await?;
                changed = true;
            }
        }
        Ok(changed.then_some(review))
    }
    .await;
    match res {
        Ok(Some(review)) => {
            let _ = l.bus.send(Event::ReviewChanged {
                workspace_id: finding.workspace_id.clone(),
                session_id: None,
                review_id: review.id,
                status: review.status.as_str().to_string(),
            });
            true
        }
        Ok(None) => false,
        Err(e) => {
            tracing::warn!(finding = %finding.id, "finding → comment link: {e}");
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Copies already on the PR + the per-review triage view
// ---------------------------------------------------------------------------

/// A comment already posted by another review of the same PR. The forge's
/// own comment id is not stored, so this names the Otto comment that posted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PostedCopy {
    pub review_id: Id,
    pub comment_id: Id,
}

/// Unposted comments of `comments` that repeat a `posted` comment of another
/// review: the same tracked finding when both are linked, else the same
/// path + line + normalised body. `posted` pairs each comment with its
/// finding id.
pub fn posted_copies(
    comments: &[ReviewComment],
    links: &HashMap<Id, Id>,
    posted: &[(ReviewComment, Option<Id>)],
) -> HashMap<Id, PostedCopy> {
    let same_text = |a: &ReviewComment, b: &ReviewComment| {
        let key = body_key(&a.body);
        !key.is_empty()
            && path_key(a.path.as_deref()) == path_key(b.path.as_deref())
            && a.line == b.line
            && key == body_key(&b.body)
    };
    comments
        .iter()
        .filter(|c| !c.posted)
        .filter_map(|c| {
            let fid = links.get(&c.id);
            let (p, _) = posted.iter().find(|(p, pf)| {
                p.posted
                    && p.review_id != c.review_id
                    && ((fid.is_some() && pf.as_ref() == fid) || same_text(c, p))
            })?;
            Some((
                c.id.clone(),
                PostedCopy {
                    review_id: p.review_id.clone(),
                    comment_id: p.id.clone(),
                },
            ))
        })
        .collect()
}

/// One comment of a review, as triage sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommentTriage {
    pub comment_id: Id,
    pub finding_id: Option<Id>,
    pub finding_status: Option<FindingStatus>,
    /// The final verifier did not check it (body starts `[unverified] `).
    pub unverified: bool,
    /// Another review of this PR already posted it.
    pub on_pr: Option<PostedCopy>,
}

/// A finding an earlier run raised that this run re-checked (the scope
/// plan's prior findings) and did not raise again.
#[derive(Debug, Clone, Serialize)]
pub struct PriorFinding {
    pub finding: Finding,
    /// `fixed` | `still_open` | `not_relevant` (the scope assessment's word).
    pub disposition: String,
    pub evidence: String,
}

/// `GET /reviews/{id}/triage`.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewTriage {
    pub review_id: Id,
    pub comments: Vec<CommentTriage>,
    pub prior: Vec<PriorFinding>,
}

/// Build the triage view of one review.
pub async fn triage(l: &Links<'_>, review: &Review) -> Result<ReviewTriage> {
    let (seen, links) = review_links(l.findings, l.events, review).await?;
    // A local review (PR #0) posts nowhere.
    let mut posted: Vec<(ReviewComment, Option<Id>)> = Vec::new();
    if review.pr_number != 0 {
        for other in l
            .reviews
            .list_for_pr(&review.repo_id, review.pr_number as i64)
            .await?
        {
            if other.id == review.id || !other.comments.iter().any(|c| c.posted) {
                continue;
            }
            let (_, ol) = review_links(l.findings, l.events, &other).await?;
            posted.extend(
                other
                    .comments
                    .iter()
                    .filter(|c| c.posted)
                    .map(|c| (c.clone(), ol.get(&c.id).cloned())),
            );
        }
    }
    let copies = posted_copies(&review.comments, &links, &posted);
    let comments = review
        .comments
        .iter()
        .map(|c| {
            let finding = links
                .get(&c.id)
                .and_then(|id| seen.iter().find(|f| &f.id == id));
            CommentTriage {
                comment_id: c.id.clone(),
                finding_id: finding.map(|f| f.id.clone()),
                finding_status: finding.map(|f| f.status),
                unverified: is_unverified(&c.body),
                on_pr: copies.get(&c.id).cloned(),
            }
        })
        .collect();
    let mut prior = Vec::new();
    for p in review.scope_plan.iter().flat_map(|s| &s.prior_findings) {
        // A finding this run raised again is on the board already.
        match l.findings.get_full(&p.finding_id).await {
            Ok(f) if f.review_id != review.id => prior.push(PriorFinding {
                finding: f,
                disposition: p.disposition.clone(),
                evidence: p.evidence.clone(),
            }),
            Ok(_) => {}
            Err(e) => tracing::warn!(finding = %p.finding_id, "prior finding: {e}"),
        }
    }
    Ok(ReviewTriage {
        review_id: review.id.clone(),
        comments,
        prior,
    })
}

#[cfg(test)]
#[path = "triage_tests.rs"]
mod tests;
