//! **Otto Assistant** — the personal agent's backend (plan
//! `personal-agent/plan.md` §2–§3; contract `docs/contracts/api.md`
//! "Otto Assistant", `ws.md` `assistant_*`).
//!
//! One front door that chats in threads, remembers the user, runs tasks and
//! reminders, delegates to Personal Agents and asks before anything outward.
//! Everything is per user (`owner_user_id`); events are owner-scoped.
//!
//! * [`threads`] — a thread is ONE resumable CLI session (`meta.assistant_thread`)
//!   in the user's scratch workspace. Turns are pasted with the shared
//!   `submit_prompt` (bracketed paste + echo check), replies are indexed from
//!   the provider transcript (`otto-transcript`) when the turn goes quiet, and
//!   sessions are resumed on demand — never kept alive (the idle sweep may
//!   suspend them between turns; a turn in flight holds `hold_for_turn`).
//! * [`router`] — local keyword rules → provider/model/account, pin +
//!   `@claude`/`@codex` override, no silent switching.
//! * [`limits`] — usage-limit detection → `assistant_limit` + "continue on X?".
//! * [`memory`] — `profile.md` + the `assistant` otto-memory collection,
//!   memory chips with Undo, optional memory approval, the Hermes seed import.
//! * [`tasks`] — tasks, reminders (`once` cadence), delegation, approvals and
//!   the one needs-you queue; the 30 s tick.
//! * [`tools`] — `POST /assistant/agent/{tool}`, the back-end of the
//!   `assistant_*` MCP tools.

pub mod hermes;
pub mod limits;
pub mod memory;
pub mod router;
pub mod tasks;
pub mod threads;
pub mod tools;
pub mod types;

use std::path::PathBuf;

use otto_core::event::Event;
use otto_core::{Error, Result};
use otto_state::{AssistantRepo, AssistantTask, AssistantThread, AssistantTurn, NewAssistantTurn};
use serde_json::Value;

use crate::state::ServerCtx;

pub use tasks::start;

/// The `meta.source` stamped on every assistant session (a background source:
/// see `otto_core::domain::BACKGROUND_SESSION_SOURCES`).
pub const SESSION_SOURCE: &str = "assistant";

pub fn repo(ctx: &ServerCtx) -> AssistantRepo {
    AssistantRepo::new(ctx.pool.clone())
}

/// `<data_dir>/personal/assistant/<user_id>` — the assistant's cwd for this
/// user (persona files, `profile.md`, `inbox/`). User ids are daemon ULIDs,
/// re-validated before the join so a hostile id fails closed.
pub fn assistant_dir(ctx: &ServerCtx, user_id: &str) -> PathBuf {
    let id = otto_core::paths::safe_component(user_id).unwrap_or("invalid");
    ctx.data_dir.join("personal").join("assistant").join(id)
}

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// Broadcast `assistant_turn` (owner-scoped).
pub fn emit_turn(
    ctx: &ServerCtx,
    owner: &str,
    turn: &AssistantTurn,
    thread: Option<&AssistantThread>,
) {
    let _ = ctx.events.send(Event::AssistantTurn {
        user_id: owner.to_string(),
        thread_id: turn.thread_id.clone(),
        turn: to_value(turn),
        thread: thread.map(to_value),
    });
}

/// Broadcast `assistant_task_update` (owner-scoped).
pub fn emit_task(ctx: &ServerCtx, task: &AssistantTask) {
    let _ = ctx.events.send(Event::AssistantTaskUpdate {
        user_id: task.owner_user_id.clone(),
        task: to_value(task),
    });
}

/// Broadcast `assistant_needs_you` with the queue size after the change.
pub async fn emit_needs_you(ctx: &ServerCtx, task: &AssistantTask) {
    let open = repo(ctx)
        .count_needs_you(&task.owner_user_id)
        .await
        .unwrap_or(0);
    let _ = ctx.events.send(Event::AssistantNeedsYou {
        user_id: task.owner_user_id.clone(),
        task: to_value(task),
        open_count: open,
    });
}

/// Emit the update for a task that changed state; also the needs-you event
/// when it entered or left the queue (`was` = its state before).
pub async fn emit_task_change(ctx: &ServerCtx, task: &AssistantTask, was: &str) {
    emit_task(ctx, task);
    if (was == "needs_you") != (task.state == "needs_you") {
        emit_needs_you(ctx, task).await;
    }
}

/// Append a SYSTEM turn (memory chip, delegation, reminder, route, limit,
/// approval, task) to a thread and broadcast it. Best-effort by design: a
/// thread deleted meanwhile just drops the line.
pub async fn system_turn(
    ctx: &ServerCtx,
    owner: &str,
    thread_id: &str,
    kind: &str,
    text: &str,
    data: Option<Value>,
) -> Option<AssistantTurn> {
    let turn = repo(ctx)
        .add_turn(NewAssistantTurn {
            thread_id: thread_id.to_string(),
            role: "system".into(),
            kind: kind.into(),
            text: text.into(),
            data,
            ..Default::default()
        })
        .await
        .ok()?;
    emit_turn(ctx, owner, &turn, None);
    Some(turn)
}

/// The caller's thread, with its live `status` derived.
pub async fn owned_thread(ctx: &ServerCtx, owner: &str, id: &str) -> Result<AssistantThread> {
    let t = repo(ctx).get_thread(owner, id).await?;
    Ok(threads::with_status(ctx, t).await)
}

/// Reject empty / oversized free text uniformly.
pub fn check_text(field: &str, text: &str, max_bytes: usize) -> Result<()> {
    if text.trim().is_empty() {
        return Err(Error::Invalid(format!("{field} is required")));
    }
    if text.len() > max_bytes {
        return Err(Error::PayloadTooLarge(format!(
            "{field} exceeds {} KiB",
            max_bytes / 1024
        )));
    }
    Ok(())
}

/// The machine's IANA timezone (`/etc/localtime` → `…/zoneinfo/Area/City`),
/// the default for reminders that name none. `UTC` when unknown.
pub fn local_timezone() -> String {
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim().trim_start_matches(':');
        if tz.parse::<chrono_tz::Tz>().is_ok() {
            return tz.to_string();
        }
    }
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| {
            let s = p.to_string_lossy().to_string();
            s.split_once("zoneinfo/").map(|(_, tz)| tz.to_string())
        })
        .filter(|tz| tz.parse::<chrono_tz::Tz>().is_ok())
        .unwrap_or_else(|| "UTC".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_text_rejects_blank_and_oversized() {
        assert!(check_text("text", "hi", 10).is_ok());
        assert!(matches!(check_text("text", "   ", 10), Err(Error::Invalid(_))));
        assert!(matches!(
            check_text("text", &"x".repeat(11), 10),
            Err(Error::PayloadTooLarge(_))
        ));
    }

    #[test]
    fn local_timezone_is_a_valid_iana_name() {
        assert!(local_timezone().parse::<chrono_tz::Tz>().is_ok());
    }
}
