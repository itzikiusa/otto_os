//! Tasks, reminders, delegation, approvals and the ONE needs-you queue.
//!
//! A task moves `queued → running → needs_you → done | failed | cancelled`.
//! Every needs-you item is a task in state `needs_you` with a `needs_you`
//! payload (`approval | question | takeover | limit | memory`), so the Mac
//! popover, the phone and Slack all read one list. Outward actions open an
//! `approval` item in the guideline shape AND a row in the existing MCP
//! approvals queue (`kind: "assistant_outward"`), and either surface can
//! decide it. "Always allow" is recorded per tool + destination only, never
//! for purchases or prod.
//!
//! The 30 s tick ([`start`]) fires due reminders (`once` cadence, delivered to
//! the thread + a user notification), reports finished delegations back into
//! their thread, syncs approvals decided in the MCP queue, and deletes
//! incognito threads 24 h after their last turn.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use otto_core::domain::{Capability, Feature, NoticeKind, NoticeSeverity, User, WorkspaceRole};
use otto_core::{Error, Result};
use otto_state::{
    AssistantTask, NewApproval, NewAssistantTask, NewNotice, PersonalAgent, PersonalAgentsRepo,
};
use serde_json::{json, Value};
use tracing::warn;

use super::limits::{self, LimitHit};
use super::router::{self, RouteTarget};
use super::threads::{self, display_provider, load_settings};
use super::types::{
    always_allow_resource, approval_card, ApprovalCard, CreateTaskReq, DecisionReq, SendReq,
};
use super::{emit_needs_you, emit_task, emit_task_change, repo, system_turn};
use crate::cadence;
use crate::state::ServerCtx;

const TICK: Duration = Duration::from_secs(30);
const SLICE: Duration = Duration::from_millis(500);
/// Incognito threads are deleted this long after their last turn.
const INCOGNITO_TTL_HOURS: i64 = 24;
/// Max wait an approval tool call may block for a decision.
pub const MAX_APPROVAL_WAIT_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// The needs-you state machine (pure)
// ---------------------------------------------------------------------------

/// The actions `POST /assistant/tasks/{id}/{action}` accepts.
pub const ACTIONS: [&str; 5] = ["approve", "deny", "takeover", "handback", "cancel"];

/// Where `action` takes a task in `state` whose needs-you payload is of kind
/// `needs` (if any). `Err` = the action does not fit the task (409).
pub fn next_state(
    task_kind: &str,
    state: &str,
    needs: Option<&str>,
    action: &str,
) -> std::result::Result<&'static str, String> {
    let open = state == "needs_you";
    let live = matches!(state, "queued" | "running");
    match action {
        "approve" | "deny" if open && needs != Some("takeover") => Ok(match (needs, action) {
            // A plain task that asked a question resumes on an answer and is
            // cancelled on a "no"; every other item is settled either way.
            (Some("question"), "approve") if task_kind == "task" || task_kind == "delegation" => {
                "running"
            }
            (Some("question"), "deny") if task_kind == "task" || task_kind == "delegation" => {
                "cancelled"
            }
            _ => "done",
        }),
        "takeover" if live => Ok("needs_you"),
        "handback" if open && needs == Some("takeover") => Ok("running"),
        "cancel" if live || open => Ok("cancelled"),
        _ if !ACTIONS.contains(&action) => Err(format!("unknown action '{action}'")),
        _ => Err(format!("cannot {action} a task that is {state}")),
    }
}

fn needs_kind(t: &AssistantTask) -> Option<String> {
    t.needs_you
        .as_ref()
        .and_then(|n| n.get("kind"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// Resolve a reminder's `run_at` (RFC3339, or local `YYYY-MM-DDTHH:MM` in
/// `timezone`) to UTC via the shared `once` cadence. Must not be in the past
/// by more than a minute (a typo, not a reminder).
pub fn resolve_run_at(
    raw: &str,
    timezone: &str,
    now: DateTime<Utc>,
) -> std::result::Result<(Value, DateTime<Utc>), String> {
    let schedule = json!({"cadence": "once", "run_at": raw.trim()});
    cadence::validate(&schedule).map_err(|e| e.to_string())?;
    let at = cadence::once_at(&schedule, cadence::task_tz(timezone))
        .ok_or_else(|| "run_at is not a valid time".to_string())?;
    if at < now - chrono::Duration::minutes(1) {
        return Err("run_at is in the past".into());
    }
    Ok((schedule, at))
}

fn check_tz(tz: &str) -> Result<String> {
    let t = tz.trim();
    if t.is_empty() {
        return Ok(super::local_timezone());
    }
    t.parse::<chrono_tz::Tz>()
        .map(|_| t.to_string())
        .map_err(|_| Error::Invalid(format!("unknown timezone '{tz}'")))
}

/// Create a reminder or a plain task (user from the Tasks tab, or the agent).
pub async fn create(
    ctx: &ServerCtx,
    owner: &str,
    req: CreateTaskReq,
    from_agent: bool,
) -> Result<AssistantTask> {
    super::check_text("title", &req.title, 2000)?;
    if let Some(tid) = req.thread_id.as_deref() {
        repo(ctx).get_thread(owner, tid).await?;
    }
    let origin = req.origin.clone().unwrap_or_else(|| "thread".into());
    let (task, turn_text) = match req.kind.as_str() {
        "reminder" => {
            let raw = req
                .run_at
                .as_deref()
                .ok_or_else(|| Error::Invalid("a reminder needs run_at".into()))?;
            let tz = check_tz(req.timezone.as_deref().unwrap_or(""))?;
            let (schedule, at) =
                resolve_run_at(raw, &tz, Utc::now()).map_err(Error::Invalid)?;
            let t = repo(ctx)
                .create_task(NewAssistantTask {
                    owner_user_id: owner.to_string(),
                    thread_id: req.thread_id.clone(),
                    kind: "reminder".into(),
                    state: "queued".into(),
                    title: req.title.trim().to_string(),
                    detail: req.detail.clone().unwrap_or_default(),
                    origin,
                    run_at: Some(at.to_rfc3339()),
                    timezone: tz,
                    schedule: Some(schedule),
                    ..Default::default()
                })
                .await?;
            let text = format!("Reminder set: {} ({})", t.title, raw.trim());
            (t, text)
        }
        "task" => {
            let t = repo(ctx)
                .create_task(NewAssistantTask {
                    owner_user_id: owner.to_string(),
                    thread_id: req.thread_id.clone(),
                    kind: "task".into(),
                    state: if from_agent { "running" } else { "queued" }.into(),
                    title: req.title.trim().to_string(),
                    detail: req.detail.clone().unwrap_or_default(),
                    origin,
                    ..Default::default()
                })
                .await?;
            let text = format!("Task: {}", t.title);
            (t, text)
        }
        other => {
            return Err(Error::Invalid(format!(
                "kind must be task|reminder (got '{other}')"
            )))
        }
    };
    emit_task(ctx, &task);
    if let Some(tid) = task.thread_id.as_deref() {
        system_turn(
            ctx,
            owner,
            tid,
            if task.kind == "reminder" { "reminder" } else { "task" },
            &turn_text,
            Some(json!({"task_id": task.id})),
        )
        .await;
    }
    Ok(task)
}

/// The agent moves its own task along (`assistant_update_task`).
pub async fn agent_update(
    ctx: &ServerCtx,
    owner: &str,
    task_id: &str,
    state: &str,
    result: Option<Value>,
    question: Option<&str>,
    options: Option<Value>,
) -> Result<AssistantTask> {
    if !matches!(state, "running" | "needs_you" | "done" | "failed") {
        return Err(Error::Invalid(
            "state must be running|needs_you|done|failed".into(),
        ));
    }
    let task = repo(ctx).get_task(owner, task_id).await?;
    if otto_state::assistant::TERMINAL_STATES.contains(&task.state.as_str()) {
        return Err(Error::Conflict(format!("task is already {}", task.state)));
    }
    let needs = if state == "needs_you" {
        let q = question
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .ok_or_else(|| Error::Invalid("needs_you requires a question".into()))?;
        let opts: Vec<String> = options
            .and_then(|o| serde_json::from_value::<Vec<String>>(o).ok())
            .unwrap_or_default()
            .into_iter()
            .take(8)
            .collect();
        Some(json!({"kind": "question", "prompt": q, "options": opts}))
    } else {
        Some(Value::Null)
    };
    let updated = repo(ctx)
        .set_task_state(&task.id, state, needs, result)
        .await?;
    emit_task_change(ctx, &updated, &task.state).await;
    if let (Some(tid), true) = (updated.thread_id.as_deref(), state != "running") {
        let text = match state {
            "needs_you" => format!("Needs you: {}", question.unwrap_or_default().trim()),
            "done" => format!("Done: {}", updated.title),
            _ => format!("Failed: {}", updated.title),
        };
        system_turn(ctx, owner, tid, "task", &text, Some(json!({"task_id": updated.id}))).await;
    }
    Ok(updated)
}

// ---------------------------------------------------------------------------
// Delegation
// ---------------------------------------------------------------------------

/// Resolve `agent` (a Personal Agent id, or its exact name) to one the user
/// may run: Editor on its workspace + `scheduled_tasks:Edit` (the feature the
/// Personal Agents routes ride).
pub async fn resolve_agent(ctx: &ServerCtx, user: &User, agent: &str) -> Result<PersonalAgent> {
    let pa = PersonalAgentsRepo::new(ctx.pool.clone());
    let candidates: Vec<PersonalAgent> = match pa.get(agent).await {
        Ok(a) => vec![a],
        Err(_) => {
            let mut out = Vec::new();
            for (id, _) in repo(ctx).personal_agents_named(agent).await? {
                if let Ok(a) = pa.get(&id).await {
                    out.push(a);
                }
            }
            out
        }
    };
    let cap = otto_state::GrantsRepo::new(ctx.pool.clone())
        .capability_of(user, Feature::ScheduledTasks)
        .await?;
    if cap < Capability::Edit {
        return Err(Error::Forbidden(
            "delegating needs scheduled_tasks:edit".into(),
        ));
    }
    let mut allowed = Vec::new();
    for a in candidates {
        if ctx
            .roles
            .check(user, &a.workspace_id, WorkspaceRole::Editor)
            .await
            .is_ok()
        {
            allowed.push(a);
        }
    }
    match allowed.len() {
        0 => Err(Error::NotFound(format!("personal agent '{agent}'"))),
        1 => Ok(allowed.remove(0)),
        _ => Err(Error::Conflict(format!(
            "more than one personal agent is named '{agent}' — pass its id"
        ))),
    }
}

/// `assistant_delegate`: start a Personal Agent run with `directive` and
/// report it into the thread ("Asked *Daily Recap*…"). The tick posts the
/// run's summary back when it settles.
pub async fn delegate(
    ctx: &ServerCtx,
    user: &User,
    thread_id: Option<&str>,
    agent: &str,
    directive: &str,
) -> Result<AssistantTask> {
    super::check_text("directive", directive, 16 * 1024)?;
    let owner = user.id.as_str();
    if let Some(tid) = thread_id {
        repo(ctx).get_thread(owner, tid).await?;
    }
    let agent = resolve_agent(ctx, user, agent).await?;
    let run = crate::personal_agents_engine::spawn_directive_run(ctx, &agent, directive).await?;
    let task = repo(ctx)
        .create_task(NewAssistantTask {
            owner_user_id: owner.to_string(),
            thread_id: thread_id.map(str::to_string),
            kind: "delegation".into(),
            state: "running".into(),
            title: format!("Asked {}", agent.name),
            detail: directive.chars().take(4000).collect(),
            origin: "thread".into(),
            agent_id: Some(agent.id.clone()),
            ..Default::default()
        })
        .await?;
    repo(ctx)
        .set_task_agent_run(&task.id, &agent.id, &run.id)
        .await?;
    let task = repo(ctx).get_task_any(&task.id).await?;
    emit_task(ctx, &task);
    if let Some(tid) = thread_id {
        system_turn(
            ctx,
            owner,
            tid,
            "delegation",
            &format!("Asked *{}*…", agent.name),
            Some(json!({"task_id": task.id, "agent_id": agent.id, "run_id": run.id})),
        )
        .await;
    }
    Ok(task)
}

// ---------------------------------------------------------------------------
// Approvals
// ---------------------------------------------------------------------------

/// `assistant_request_approval`. An existing always-allow grant for the
/// card's tool + destination answers at once (never for purchase / prod);
/// otherwise a needs-you item + an MCP approvals row are opened and the call
/// waits up to `wait_seconds` (≤ 30) for a decision.
pub async fn request_approval(
    ctx: &ServerCtx,
    owner: &str,
    thread_id: Option<&str>,
    args: &Value,
) -> Result<Value> {
    let card = approval_card(args).map_err(Error::Invalid)?;
    let wait = args
        .get("wait_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(MAX_APPROVAL_WAIT_SECS);

    if card.always_allow_allowed {
        if let Some(resource) = always_allow_resource(&card) {
            let principal = repo(ctx).assistant_principal(owner).await?;
            if repo(ctx)
                .grant_mode(&principal, "tool_destination", &resource)
                .await?
                .as_deref()
                == Some("allow")
            {
                let task = repo(ctx)
                    .create_task(NewAssistantTask {
                        owner_user_id: owner.to_string(),
                        thread_id: thread_id.map(str::to_string),
                        kind: "approval".into(),
                        state: "done".into(),
                        title: approval_title(&card),
                        detail: card.what.clone(),
                        origin: "thread".into(),
                        ..Default::default()
                    })
                    .await?;
                let task = repo(ctx)
                    .set_task_state(
                        &task.id,
                        "done",
                        None,
                        Some(json!({"decision": "approved", "always_allow": true, "card": card})),
                    )
                    .await?;
                emit_task(ctx, &task);
                if let Some(tid) = thread_id {
                    system_turn(
                        ctx,
                        owner,
                        tid,
                        "approval",
                        &format!("Allowed (always allow): {}", approval_title(&card)),
                        Some(json!({"task_id": task.id})),
                    )
                    .await;
                }
                return Ok(json!({"task": task, "decision": "approved", "reason": "always allow"}));
            }
        }
    }

    let appr = ctx
        .mcp
        .approvals()
        .create(NewApproval {
            workspace_id: None,
            kind: "assistant_outward".into(),
            server_id: None,
            server_name: Some("otto".into()),
            tool: Some(format!(
                "assistant:{}",
                card.tool.clone().unwrap_or_else(|| card.category.clone())
            )),
            title: approval_title(&card),
            detail: Some(serde_json::to_string(&card).unwrap_or_default()),
            args_redacted_json: otto_core::redact::redact_json(args).value.to_string(),
            args_hash: None,
            risk_label: Some(card.category.clone()),
            requested_by: Some(owner.to_string()),
            requested_by_kind: Some("agent".into()),
            expires_at: Some((Utc::now() + chrono::Duration::hours(24)).to_rfc3339()),
        })
        .await?;
    let task = repo(ctx)
        .create_task(NewAssistantTask {
            owner_user_id: owner.to_string(),
            thread_id: thread_id.map(str::to_string),
            kind: "approval".into(),
            state: "needs_you".into(),
            title: approval_title(&card),
            detail: card.what.clone(),
            origin: "thread".into(),
            needs_you: Some(json!({
                "kind": "approval",
                "prompt": format!("Otto wants to {} — {}", card.category, approval_title(&card)),
                "approval": card,
                "mcp_approval_id": appr.id,
            })),
            ..Default::default()
        })
        .await?;
    emit_task(ctx, &task);
    emit_needs_you(ctx, &task).await;
    if let Some(tid) = thread_id {
        system_turn(
            ctx,
            owner,
            tid,
            "approval",
            &format!("Waiting for your approval: {}", approval_title(&card)),
            Some(json!({"task_id": task.id})),
        )
        .await;
    }

    let mut waited = 0u64;
    let mut current = task;
    while waited < wait && current.state == "needs_you" {
        tokio::time::sleep(Duration::from_secs(1)).await;
        waited += 1;
        current = repo(ctx).get_task_any(&current.id).await?;
    }
    let decision = match (current.state.as_str(), &current.result) {
        ("needs_you", _) => "pending",
        (_, Some(r)) if r.get("decision").and_then(Value::as_str) == Some("approved") => "approved",
        _ => "denied",
    };
    let reason = current
        .result
        .as_ref()
        .and_then(|r| r.get("reason"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(json!({"task": current, "decision": decision, "reason": reason}))
}

fn approval_title(card: &ApprovalCard) -> String {
    let t = format!("{} → {}", card.what.trim(), card.where_.trim());
    t.chars().take(200).collect()
}

// ---------------------------------------------------------------------------
// Decisions: POST /assistant/tasks/{id}/{action}
// ---------------------------------------------------------------------------

pub async fn act(
    ctx: &ServerCtx,
    user: &User,
    task_id: &str,
    action: &str,
    req: DecisionReq,
) -> Result<AssistantTask> {
    let owner = user.id.as_str();
    let task = repo(ctx).get_task(owner, task_id).await?;
    let needs = needs_kind(&task);
    let to = next_state(&task.kind, &task.state, needs.as_deref(), action)
        .map_err(Error::Conflict)?;
    let reason = req.reason.clone().map(|r| r.chars().take(2000).collect::<String>());
    let decision = match action {
        "approve" => "approved",
        "deny" => "denied",
        other => other,
    };
    let mut result = json!({
        "decision": decision,
        "reason": reason,
        "decided_at": Utc::now().to_rfc3339(),
    });
    let mut followup: Option<String> = None;
    match (action, needs.as_deref()) {
        ("approve" | "deny", Some("approval")) => {
            let approved = action == "approve";
            let card: Option<ApprovalCard> = task
                .needs_you
                .as_ref()
                .and_then(|n| n.get("approval"))
                .and_then(|c| serde_json::from_value(c.clone()).ok());
            if approved && req.always_allow {
                let card = card
                    .as_ref()
                    .ok_or_else(|| Error::Invalid("approval card missing".into()))?;
                if !card.always_allow_allowed {
                    return Err(Error::Invalid(format!(
                        "\"always allow\" is never offered for {}",
                        card.category
                    )));
                }
                let resource = always_allow_resource(card).ok_or_else(|| {
                    Error::Invalid("always allow needs a tool and a destination".into())
                })?;
                let principal = repo(ctx).assistant_principal(owner).await?;
                repo(ctx)
                    .set_grant(&principal, "tool_destination", &resource, "allow")
                    .await?;
                result["always_allow"] = json!(resource);
            }
            if let Some(aid) = task
                .needs_you
                .as_ref()
                .and_then(|n| n.get("mcp_approval_id"))
                .and_then(Value::as_str)
            {
                // Already decided in the MCP queue is fine (Conflict ignored).
                let _ = ctx
                    .mcp
                    .approvals()
                    .decide(&aid.to_string(), approved, owner, reason.as_deref())
                    .await;
            }
        }
        ("approve", Some("question")) => {
            let answer = req
                .answer
                .as_deref()
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .ok_or_else(|| Error::Invalid("an answer is required".into()))?;
            result["answer"] = json!(answer);
            let prompt = task
                .needs_you
                .as_ref()
                .and_then(|n| n.get("prompt"))
                .and_then(Value::as_str)
                .unwrap_or("");
            followup = Some(format!("Answer to your question \u{201c}{prompt}\u{201d}: {answer}"));
        }
        ("approve", Some("memory")) | ("deny", Some("memory")) => {
            if let Some(mid) = task
                .needs_you
                .as_ref()
                .and_then(|n| n.get("memory_id"))
                .and_then(Value::as_str)
            {
                if action == "approve" {
                    let _ = super::memory::accept(ctx, owner, mid).await;
                } else {
                    let _ = super::memory::forget_one(ctx, owner, mid).await;
                }
            }
        }
        ("approve", Some("limit")) => {
            let provider = req
                .provider
                .clone()
                .or_else(|| {
                    task.needs_you
                        .as_ref()
                        .and_then(|n| n.get("suggestion"))
                        .and_then(|s| s.get("provider"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .ok_or_else(|| Error::Invalid("which provider should it continue on?".into()))?;
            if !router::valid_provider(&provider) {
                return Err(Error::Invalid(format!("provider '{provider}'")));
            }
            result["provider"] = json!(provider);
            if let Some(tid) = task.thread_id.as_deref() {
                repo(ctx).set_failover_choice(tid, "switch").await?;
                resend_last_on(ctx, owner, tid, &provider).await;
            }
        }
        ("deny", Some("limit")) => {
            if let Some(tid) = task.thread_id.as_deref() {
                repo(ctx).set_failover_choice(tid, "stay").await?;
            }
        }
        ("takeover", _) => {
            // Pause the agent: interrupt its current turn (Esc), keep the
            // task open as a takeover item until the user hands back.
            if let Some(sid) = thread_session(ctx, owner, task.thread_id.as_deref()).await {
                let _ = ctx.manager.input(&sid, b"\x1b").await;
            }
        }
        ("handback", _) => {
            let note = req.reason.as_deref().unwrap_or("").trim();
            followup = Some(if note.is_empty() {
                format!("I've handed control back — please continue: {}", task.title)
            } else {
                format!("I've handed control back ({note}) — please continue: {}", task.title)
            });
        }
        _ => {}
    }

    let needs_payload = match to {
        "needs_you" => Some(json!({
            "kind": "takeover",
            "prompt": format!("You have control of: {}. Hand back when you're done.", task.title),
        })),
        _ => Some(Value::Null),
    };
    let updated = repo(ctx)
        .set_task_state(&task.id, to, needs_payload, Some(result))
        .await?;
    emit_task_change(ctx, &updated, &task.state).await;

    if let (Some(text), Some(tid)) = (followup, updated.thread_id.clone()) {
        let _ = threads::send(
            ctx,
            owner,
            &tid,
            SendReq {
                text,
                origin: Some("app".into()),
                ..Default::default()
            },
            None,
        )
        .await;
    }
    Ok(updated)
}

async fn thread_session(ctx: &ServerCtx, owner: &str, thread_id: Option<&str>) -> Option<String> {
    let t = repo(ctx).get_thread(owner, thread_id?).await.ok()?;
    t.session_id
}

/// Re-send the thread's last user message on `provider` (the "continue on
/// Codex" answer to a limit item, or an auto-failover).
async fn resend_last_on(ctx: &ServerCtx, owner: &str, thread_id: &str, provider: &str) {
    let Ok(turns) = repo(ctx).list_turns(thread_id, None, 50).await else {
        return;
    };
    let Some(last) = turns.iter().rev().find(|t| t.role == "user") else {
        return;
    };
    let (settings, _) = load_settings(ctx, owner).await;
    let target = [
        &settings.targets.chat,
        &settings.targets.code,
        &settings.targets.hard,
        &settings.targets.voice,
    ]
    .into_iter()
    .find(|t| t.provider == provider)
    .cloned()
    .unwrap_or_else(|| RouteTarget::new(provider, None));
    let _ = threads::send_boxed(
        ctx.clone(),
        owner.to_string(),
        thread_id.to_string(),
        SendReq {
            text: last.text.clone(),
            origin: Some("app".into()),
            ..Default::default()
        },
        Some(target),
    )
    .await;
}

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

/// A usage limit was detected on `thread`'s route: record it, tell the user,
/// and either switch (auto-failover / "switch" answered before), stay quietly
/// ("stay" answered before), or open ONE "continue on X?" item.
pub async fn on_limit(
    ctx: &ServerCtx,
    owner: &str,
    thread_id: &str,
    route: &RouteTarget,
    hit: &LimitHit,
    source: &str,
) {
    let now = Utc::now();
    let state = limits::state_for(&route.provider, route.account_id.as_deref(), hit, source, now);
    let (settings, snapshot) = load_settings(ctx, owner).await;
    let snapshot = limits::upsert_snapshot(snapshot, state.clone(), now);
    let _ = repo(ctx)
        .set_limits(owner, &serde_json::to_value(&snapshot).unwrap_or(Value::Null))
        .await;
    let Ok(thread) = repo(ctx).get_thread(owner, thread_id).await else {
        return;
    };
    let suggestion = router::alternative(&settings, &route.provider);
    let label = format!(
        "{} limit reached{}",
        display_provider(&route.provider),
        state
            .until
            .as_deref()
            .map(|u| format!(" until {u}"))
            .unwrap_or_default()
    );
    let auto = (settings.auto_failover || thread.failover_choice == "switch")
        && suggestion.is_some()
        && !thread.route_pinned;

    let mut task_id = None;
    if auto || thread.failover_choice == "stay" || suggestion.is_none() {
        system_turn(ctx, owner, thread_id, "limit", &label, Some(json!({"limit": state}))).await;
        if let (true, Some(alt)) = (auto, suggestion.as_ref()) {
            // The turn that hit the limit is still being driven (its claim is
            // held): re-send it on the alternative once the driver lets go.
            let (ctx2, owner2, tid, provider) = (
                ctx.clone(),
                owner.to_string(),
                thread_id.to_string(),
                alt.provider.clone(),
            );
            tokio::spawn(async move {
                for _ in 0..40 {
                    if !threads::is_in_flight(&tid) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                resend_last_on(&ctx2, &owner2, &tid, &provider).await;
            });
        }
    } else {
        let alt = suggestion.clone().unwrap_or_else(|| RouteTarget::new("codex", None));
        let already_open = repo(ctx)
            .needs_you(owner)
            .await
            .map(|v| {
                v.iter()
                    .any(|t| t.kind == "limit" && t.thread_id.as_deref() == Some(thread_id))
            })
            .unwrap_or(false);
        if !already_open {
            let prompt = format!("{label} — continue on {}?", display_provider(&alt.provider));
            if let Ok(t) = repo(ctx)
                .create_task(NewAssistantTask {
                    owner_user_id: owner.to_string(),
                    thread_id: Some(thread_id.to_string()),
                    kind: "limit".into(),
                    state: "needs_you".into(),
                    title: prompt.clone(),
                    origin: "thread".into(),
                    needs_you: Some(json!({
                        "kind": "limit", "prompt": prompt, "limit": state, "suggestion": alt,
                    })),
                    ..Default::default()
                })
                .await
            {
                emit_task(ctx, &t);
                emit_needs_you(ctx, &t).await;
                task_id = Some(t.id.clone());
                system_turn(
                    ctx,
                    owner,
                    thread_id,
                    "limit",
                    &prompt,
                    Some(json!({"limit": state, "task_id": t.id})),
                )
                .await;
            }
        }
    }
    let _ = ctx.events.send(otto_core::event::Event::AssistantLimit {
        user_id: owner.to_string(),
        thread_id: Some(thread_id.to_string()),
        limit: serde_json::to_value(&state).unwrap_or(Value::Null),
        suggestion: suggestion.as_ref().and_then(|s| serde_json::to_value(s).ok()),
        task_id,
        auto_switched: auto,
    });
}

// ---------------------------------------------------------------------------
// The tick
// ---------------------------------------------------------------------------

/// Start the assistant supervisor (30 s tick). Returns a cancel flag, like the
/// other schedulers. Never touches anything outside the assistant's own rows
/// and the sessions it owns.
pub fn start(ctx: ServerCtx) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    tokio::spawn(async move {
        loop {
            if flag.load(Ordering::Relaxed) {
                return;
            }
            tick(&ctx).await;
            let mut waited = Duration::ZERO;
            while waited < TICK {
                if flag.load(Ordering::Relaxed) {
                    return;
                }
                tokio::time::sleep(SLICE).await;
                waited += SLICE;
            }
        }
    });
    cancel
}

async fn tick(ctx: &ServerCtx) {
    fire_due_reminders(ctx).await;
    settle_delegations(ctx).await;
    sync_mcp_approvals(ctx).await;
    expire_incognito(ctx).await;
}

async fn fire_due_reminders(ctx: &ServerCtx) {
    let now = Utc::now();
    let due = match repo(ctx).due_reminders(&now.to_rfc3339()).await {
        Ok(d) => d,
        Err(e) => {
            warn!("assistant: due reminders: {e}");
            return;
        }
    };
    for t in due {
        // The cadence engine has the final say (DST / parse); claim once.
        let tz = cadence::task_tz(&t.timezone);
        let spec = t.schedule.clone().unwrap_or(Value::Null);
        if !cadence::is_due(&spec, None, now, tz) && t.schedule.is_some() {
            continue;
        }
        if !repo(ctx).claim_task(&t.id, "queued", "running").await.unwrap_or(false) {
            continue;
        }
        if let Some(tid) = t.thread_id.as_deref() {
            system_turn(
                ctx,
                &t.owner_user_id,
                tid,
                "reminder",
                &format!("⏰ Reminder: {}", t.title),
                Some(json!({"task_id": t.id})),
            )
            .await;
        }
        let _ = ctx
            .notifications()
            .create(NewNotice {
                kind: NoticeKind::System,
                severity: NoticeSeverity::Info,
                title: format!("Reminder: {}", t.title),
                body: t.detail.clone(),
                source_key: Some(format!("assistant:reminder:{}", t.id)),
                // The notification opens the Assistant module client-side.
                action: None,
                user_id: Some(t.owner_user_id.clone()),
            })
            .await;
        if let Ok(done) = repo(ctx)
            .set_task_state(&t.id, "done", None, Some(json!({"delivered_at": now.to_rfc3339(), "origin": t.origin})))
            .await
        {
            emit_task(ctx, &done);
        }
    }
}

async fn settle_delegations(ctx: &ServerCtx) {
    let Ok(running) = repo(ctx).running_delegations().await else {
        return;
    };
    let pa = PersonalAgentsRepo::new(ctx.pool.clone());
    for t in running {
        let Some(run_id) = t.agent_run_id.as_deref() else {
            continue;
        };
        let run = match pa.get_run(run_id).await {
            Ok(r) => r,
            Err(Error::NotFound(_)) => {
                if let Ok(done) = repo(ctx)
                    .set_task_state(&t.id, "failed", None, Some(json!({"error": "run was pruned"})))
                    .await
                {
                    emit_task(ctx, &done);
                }
                continue;
            }
            Err(_) => continue,
        };
        if run.status == "running" {
            continue;
        }
        let ok = run.status == "ok";
        let agent_name = t.title.trim_start_matches("Asked ").to_string();
        let text = if ok {
            format!("*{agent_name}* finished: {}", run.summary.trim())
        } else {
            format!(
                "*{agent_name}* failed: {}",
                run.error.as_deref().unwrap_or("unknown error")
            )
        };
        if let Some(tid) = t.thread_id.as_deref() {
            system_turn(
                ctx,
                &t.owner_user_id,
                tid,
                "delegation",
                &text,
                Some(json!({"task_id": t.id, "run_id": run.id, "status": run.status})),
            )
            .await;
        }
        if let Ok(done) = repo(ctx)
            .set_task_state(
                &t.id,
                if ok { "done" } else { "failed" },
                None,
                Some(json!({
                    "run_id": run.id, "status": run.status, "summary": run.summary,
                    "error": run.error, "session_id": run.session_id,
                })),
            )
            .await
        {
            emit_task(ctx, &done);
        }
    }
}

/// An approval decided in the MCP approvals queue settles its needs-you item.
async fn sync_mcp_approvals(ctx: &ServerCtx) {
    let Ok(open) = repo(ctx).open_approvals().await else {
        return;
    };
    for t in open {
        let Some(aid) = t
            .needs_you
            .as_ref()
            .and_then(|n| n.get("mcp_approval_id"))
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let Ok(appr) = ctx.mcp.approvals().get(&aid).await else {
            continue;
        };
        let decision = match appr.status.as_str() {
            "approved" => "approved",
            "denied" => "denied",
            "expired" => "expired",
            _ => continue,
        };
        if let Ok(done) = repo(ctx)
            .set_task_state(
                &t.id,
                "done",
                Some(Value::Null),
                Some(json!({
                    "decision": decision, "reason": appr.decision_note,
                    "decided_at": appr.decided_at, "via": "mcp_approvals",
                })),
            )
            .await
        {
            emit_task(ctx, &done);
            emit_needs_you(ctx, &done).await;
        }
    }
}

async fn expire_incognito(ctx: &ServerCtx) {
    let cutoff = (Utc::now() - chrono::Duration::hours(INCOGNITO_TTL_HOURS)).to_rfc3339();
    let Ok(expired) = repo(ctx).expired_incognito(&cutoff).await else {
        return;
    };
    for t in expired {
        if let Some(sid) = t.session_id.as_ref() {
            let _ = ctx.manager.remove(sid).await;
        }
        let _ = repo(ctx).delete_thread(&t.owner_user_id, &t.id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approvals_settle_either_way() {
        assert_eq!(next_state("approval", "needs_you", Some("approval"), "approve"), Ok("done"));
        assert_eq!(next_state("approval", "needs_you", Some("approval"), "deny"), Ok("done"));
        // Not open: nothing to approve.
        assert!(next_state("approval", "done", Some("approval"), "approve").is_err());
    }

    #[test]
    fn a_blocked_task_resumes_on_an_answer_and_is_cancelled_on_no() {
        assert_eq!(next_state("task", "needs_you", Some("question"), "approve"), Ok("running"));
        assert_eq!(next_state("task", "needs_you", Some("question"), "deny"), Ok("cancelled"));
        // A standalone question item is simply settled.
        assert_eq!(next_state("question", "needs_you", Some("question"), "approve"), Ok("done"));
    }

    #[test]
    fn takeover_pauses_and_handback_resumes() {
        assert_eq!(next_state("task", "running", None, "takeover"), Ok("needs_you"));
        assert_eq!(next_state("task", "needs_you", Some("takeover"), "handback"), Ok("running"));
        // A takeover is not approved/denied — it is handed back.
        assert!(next_state("task", "needs_you", Some("takeover"), "approve").is_err());
        // Nothing to hand back when nobody took over.
        assert!(next_state("task", "needs_you", Some("question"), "handback").is_err());
        assert!(next_state("task", "done", None, "takeover").is_err());
    }

    #[test]
    fn cancel_ends_live_or_open_tasks_only() {
        for s in ["queued", "running", "needs_you"] {
            assert_eq!(next_state("task", s, None, "cancel"), Ok("cancelled"), "{s}");
        }
        for s in ["done", "failed", "cancelled"] {
            assert!(next_state("task", s, None, "cancel").is_err(), "{s}");
        }
        assert!(next_state("task", "running", None, "explode")
            .unwrap_err()
            .contains("unknown action"));
    }

    #[test]
    fn run_at_resolves_through_the_once_cadence() {
        let now = Utc::now();
        let future = (now + chrono::Duration::hours(2)).to_rfc3339();
        let (spec, at) = resolve_run_at(&future, "UTC", now).unwrap();
        assert_eq!(spec["cadence"], "once");
        assert!((at - (now + chrono::Duration::hours(2))).num_seconds().abs() <= 1);
        // Local wall-clock in a named zone.
        let (_, at) = resolve_run_at("2099-01-01T09:00", "Asia/Jerusalem", now).unwrap();
        assert_eq!(at.to_rfc3339(), "2099-01-01T07:00:00+00:00");
        // Past and garbage are refused.
        assert!(resolve_run_at("2001-01-01T09:00", "UTC", now).is_err());
        assert!(resolve_run_at("at five", "UTC", now).is_err());
    }
}
