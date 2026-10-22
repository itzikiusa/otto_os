//! Channel ↔ swarm glue (design §4): launch a swarm from a Slack/Telegram message
//! or a webhook, and reply/escalate back to the originating chat.
//!
//! `launch` is shared by the webhook route and the `SwarmTrigger` (Bridge) path so
//! the "message → project → plan → run" flow lives in one place. `notify_origin`
//! posts progress/escalation back to the chat that launched the swarm.

use async_trait::async_trait;
use otto_channels::{Adapter, LaunchAck, SwarmTrigger, WebhookAdapter};
use otto_core::domain::Channel;
use otto_core::Result;
use otto_state::{NewProject, ProjectPatch, Swarm, SwarmProject};

use crate::runtime::host::SwarmRt;

/// Where a channel-launched swarm replies back to.
#[derive(Debug, Clone)]
pub struct Origin {
    pub channel: String, // slack | telegram | webhook
    pub chat: String,    // chat/channel id, or (webhook) the callback URL
    pub thread: Option<String>,
}

/// An explicit goal carried by a launch trigger.
#[derive(Debug, Clone, Default)]
pub struct GoalSpec {
    pub title: String,
    pub description: String,
    pub metric: Option<String>,
    pub comparator: Option<String>,
    pub target_value: Option<f64>,
    pub block_value: Option<f64>,
    pub verify_cmd: Option<String>,
    pub max_retries: Option<i64>,
    pub blocking: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct LaunchOpts {
    pub goal: String,
    pub name: Option<String>,
    pub repo_path: Option<String>,
    pub goals: Vec<GoalSpec>,
    pub origin: Option<Origin>,
    pub start: bool,
    pub created_by: String,
}

/// Create a project from a goal, attach any explicit goals, record the channel
/// origin, then (in the background) seed tasks via the planner and start the
/// coordinator. Returns the new project id immediately. Agents run in worktrees
/// by default (the cwd-mode default), so several can share the repo safely.
pub async fn launch(ctx: &SwarmRt, swarm: &Swarm, opts: LaunchOpts) -> Result<String> {
    let name = opts.name.clone().unwrap_or_else(|| {
        opts.goal
            .lines()
            .next()
            .unwrap_or("Channel task")
            .chars()
            .take(80)
            .collect()
    });
    let project = ctx
        .swarm_repo()
        .create_project(NewProject {
            swarm_id: swarm.id.clone(),
            workspace_id: swarm.workspace_id.clone(),
            name,
            description: "Launched via channel/webhook".into(),
            repo_path: opts.repo_path.clone(),
            goal_md: Some(opts.goal.clone()),
            story_id: None,
            order_idx: 0,
            created_by: opts.created_by.clone(),
        })
        .await?;

    // Record the channel origin so the swarm can reply/escalate back.
    if let Some(origin) = &opts.origin {
        let _ = ctx
            .swarm_repo()
            .update_project(
                &project.id,
                ProjectPatch {
                    origin_channel: Some(Some(origin.channel.clone())),
                    origin_chat: Some(Some(origin.chat.clone())),
                    origin_thread: Some(origin.thread.clone().map(Some).unwrap_or(None)),
                    ..Default::default()
                },
            )
            .await;
    }

    // Attach explicit goals (project-scoped) carried by the trigger.
    for (i, g) in opts.goals.iter().enumerate() {
        if g.title.trim().is_empty() {
            continue;
        }
        let _ = ctx
            .swarm_repo()
            .create_goal(otto_state::NewGoal {
                swarm_id: swarm.id.clone(),
                workspace_id: swarm.workspace_id.clone(),
                project_id: Some(project.id.clone()),
                task_id: None,
                kind: "explicit".into(),
                title: g.title.clone(),
                description: g.description.clone(),
                metric: g.metric.clone(),
                comparator: g.comparator.clone(),
                target_value: g.target_value,
                block_value: g.block_value,
                verify_cmd: g.verify_cmd.clone(),
                max_retries: g.max_retries.unwrap_or(3),
                blocking: g.blocking.unwrap_or(true),
                order_idx: i as i64,
                created_by: opts.created_by.clone(),
            })
            .await;
    }

    // Plan + start in the background; the caller (webhook/Bridge) returns at once.
    let ctx2 = ctx.clone();
    let swarm_id = swarm.id.clone();
    let workspace_id = swarm.workspace_id.clone();
    let creator = opts.created_by.clone();
    let goal = opts.goal.clone();
    let start = opts.start;
    let project2 = project.clone();
    tokio::spawn(async move {
        ctx2.seed_tasks(&project2, &creator, &goal).await;
        if start {
            // The channel gate (`ActivatePolicy::Channel`): a channel message
            // used to flip an ABORTED or budget-paused swarm back to active
            // (clearing its pause reason) and spend past the cap. `activate`
            // releases the swarm's lock before returning, so the refusal post
            // (a slow Slack) never runs under it.
            let refused = crate::runtime::lifecycle::activate(
                &ctx2,
                &workspace_id,
                &swarm_id,
                crate::runtime::lifecycle::ActivatePolicy::Channel,
            )
            .await;
            let why = match refused {
                Ok(_) => return,
                Err(otto_core::Error::Conflict(why)) => why,
                Err(e) => format!("it could not be started ({e})"),
            };
            notify_origin(
                &ctx2,
                &project2,
                &format!("Tasks were added, but the swarm was not started: {why}."),
            )
            .await;
        }
    });

    Ok(project.id)
}

/// Why a channel/webhook launch must not (re)start the swarm, if it must not:
/// it was aborted, it is auto-paused for a budget/limit reason, or the
/// workspace's spend cap blocks new work.
pub(crate) async fn channel_start_refused(
    ctx: &SwarmRt,
    swarm_id: &str,
    workspace_id: &str,
) -> Option<String> {
    // A failed or missing status read used to return None, and the caller
    // treats None as "start allowed", then sets the swarm active. Refuse.
    let swarm = match ctx.swarm_repo().get_swarm(&swarm_id.to_string()).await {
        Ok(swarm) => swarm,
        Err(e) => return status_load_refusal(Err(e.to_string())),
    };
    if let Some(why) =
        status_load_refusal(Ok((swarm.status.as_str(), swarm.pause_reason.as_deref())))
    {
        return Some(why);
    }
    ctx.budget_blocked(workspace_id).await.map(|reason| {
        format!(
            "the workspace budget is exceeded ({})",
            reason.unwrap_or_else(|| "cap reached".into())
        )
    })
}

/// Status refusal after a load. `Err` is a failed read: the launch must not
/// start the swarm. `Ok` is the stored status and pause reason.
fn status_load_refusal(
    loaded: std::result::Result<(&str, Option<&str>), String>,
) -> Option<String> {
    match loaded {
        Err(err) => Some(format!(
            "its status could not be read ({err}) — not started"
        )),
        Ok((status, pause)) => start_refusal(status, pause),
    }
}

/// [`channel_start_refused`]'s pure status part.
fn start_refusal(status: &str, pause_reason: Option<&str>) -> Option<String> {
    if status == "aborted" {
        return Some("it was aborted — start it from Otto".into());
    }
    pause_reason
        .filter(|r| !r.trim().is_empty())
        .map(|r| format!("it is paused ({r}) — resume it from Otto"))
}

/// Reply/escalate back to the channel that launched `project` (if any). Best-effort.
pub async fn notify_origin(ctx: &SwarmRt, project: &SwarmProject, text: &str) {
    let (Some(channel), Some(chat)) = (
        project.origin_channel.as_deref(),
        project.origin_chat.as_deref(),
    ) else {
        return;
    };
    let thread = project.origin_thread.as_deref();
    match channel {
        "webhook" => {
            // `chat` is the callback URL; conversation key = the project id.
            let adapter = WebhookAdapter::new(Some(chat.to_string()));
            let _ = adapter.send_formatted(&project.id, thread, text).await;
        }
        "slack" | "telegram" => {
            let ch = if channel == "slack" {
                Channel::Slack
            } else {
                Channel::Telegram
            };
            if let Ok(Some(integ)) = ctx
                .integrations_store()
                .get(&project.workspace_id, ch)
                .await
            {
                let _ = otto_channels::improve_notify::send_to(
                    ctx.secrets(),
                    &integ,
                    chat,
                    thread,
                    text,
                )
                .await;
            }
        }
        _ => {}
    }
}

/// The `SwarmTrigger` implementation injected into the channel `Bridge`: an inbound
/// message on a swarm-bound channel launches that swarm.
pub struct SwarmTriggerImpl {
    pub ctx: SwarmRt,
}

#[async_trait]
impl SwarmTrigger for SwarmTriggerImpl {
    async fn try_launch(
        &self,
        workspace_id: &str,
        channel: &str,
        chat: &str,
        thread: Option<&str>,
        _user: &str,
        text: &str,
    ) -> Option<LaunchAck> {
        let triggers = self
            .ctx
            .swarm_repo()
            .find_triggers(&workspace_id.to_string(), channel)
            .await
            .unwrap_or_default();
        for t in triggers {
            // Chat filter: empty = any chat in this channel.
            if !t.match_chat.trim().is_empty() && t.match_chat.trim() != chat {
                continue;
            }
            // Keyword filter: empty = any message; else must start with the keyword.
            let body = if t.keyword.trim().is_empty() {
                text.trim().to_string()
            } else {
                let Some(rest) = strip_keyword(text, t.keyword.trim()) else {
                    continue;
                };
                rest.trim().to_string()
            };
            if body.is_empty() {
                continue;
            }
            let swarm = match self.ctx.swarm_repo().get_swarm(&t.swarm_id).await {
                Ok(s) => s,
                Err(_) => continue,
            };
            let opts = LaunchOpts {
                goal: body.clone(),
                name: None,
                repo_path: t.repo_path.clone(),
                goals: Vec::new(),
                origin: if t.reply {
                    Some(Origin {
                        channel: channel.to_string(),
                        chat: chat.to_string(),
                        thread: thread.map(str::to_string),
                    })
                } else {
                    None
                },
                start: t.auto_start,
                created_by: trigger_actor(self.ctx.swarm_repo(), &t.created_by, &swarm).await,
            };
            match launch(&self.ctx, &swarm, opts).await {
                Ok(_pid) => {
                    return Some(LaunchAck {
                        reply: format!(
                            "🐝 Launched **{}** on: _{}_ — I'll report back here.",
                            swarm.name,
                            clip(&body, 160)
                        ),
                    });
                }
                Err(e) => {
                    tracing::warn!("swarm trigger launch failed: {e}");
                    return Some(LaunchAck {
                        reply: format!("⚠️ Could not launch the swarm: {e}"),
                    });
                }
            }
        }
        None
    }
}

use otto_core::text::clip_chars as clip;

/// The message after a case-insensitive leading `kw`, or `None` when it does
/// not start with it. Matches lowercase forms CHARACTER by character and cuts
/// at a char boundary of the message: slicing the original at `kw.len()`
/// after a lowercase match panicked (and dropped the message) whenever
/// lowercasing changed a byte length — a Kelvin sign `K` matching `k`.
/// Who a channel-trigger launch acts as (access-control-5-02): the person who
/// set the trigger up, while that user still exists — the message text is the
/// goal THEY wired to run — else the swarm's creator. A trigger the creator
/// made keeps acting as the creator.
pub async fn trigger_actor(
    repo: &otto_state::SwarmRepo,
    trigger_creator: &str,
    swarm: &Swarm,
) -> otto_core::Id {
    let exists =
        !trigger_creator.is_empty() && repo.user_exists(trigger_creator).await.unwrap_or(false);
    if exists {
        trigger_creator.to_string()
    } else {
        swarm.created_by.clone()
    }
}

fn strip_keyword<'a>(text: &'a str, kw: &str) -> Option<&'a str> {
    let text = text.trim_start();
    let mut want = kw.chars().flat_map(char::to_lowercase).peekable();
    for (i, c) in text.char_indices() {
        if want.peek().is_none() {
            return Some(&text[i..]);
        }
        for lc in c.to_lowercase() {
            if want.next() != Some(lc) {
                return None;
            }
        }
    }
    want.peek().is_none().then_some("")
}

#[cfg(test)]
mod tests {
    use super::{start_refusal, status_load_refusal, strip_keyword};

    #[test]
    fn a_channel_never_restarts_an_aborted_or_auto_paused_swarm() {
        assert!(start_refusal("aborted", None).is_some());
        assert!(start_refusal("paused", Some("max_total_runs reached")).is_some());
        assert!(start_refusal("active", None).is_none());
        assert!(status_load_refusal(Err("db down".into()))
            .is_some_and(|w| w.contains("could not be read")));
        assert!(status_load_refusal(Ok(("aborted", None))).is_some());
        assert!(status_load_refusal(Ok(("active", None))).is_none());
        // An ordinary (manual) pause carries no reason: a launch may start it.
        assert!(start_refusal("paused", None).is_none());
        assert!(start_refusal("idle", Some("  ")).is_none());
    }

    #[test]
    fn keyword_strip_is_case_insensitive_and_char_safe() {
        assert_eq!(
            strip_keyword("  Deploy the app", "deploy"),
            Some(" the app")
        );
        assert_eq!(strip_keyword("deploy", "DEPLOY"), Some(""));
        assert_eq!(strip_keyword("dep", "deploy"), None);
        assert_eq!(strip_keyword("ship it", "deploy"), None);
        // Kelvin sign lowercases to `k` (3 bytes vs 1): no panic, right cut.
        assert_eq!(strip_keyword("\u{212A}8s rollout", "k8s"), Some(" rollout"));
        assert_eq!(strip_keyword("Ünïcode go", "ünïcode"), Some(" go"));
    }
}
