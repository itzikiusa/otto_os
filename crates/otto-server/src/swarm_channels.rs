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

use crate::state::ServerCtx;

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
pub async fn launch(ctx: &ServerCtx, swarm: &Swarm, opts: LaunchOpts) -> Result<String> {
    let name = opts.name.clone().unwrap_or_else(|| {
        opts.goal.lines().next().unwrap_or("Channel task").chars().take(80).collect()
    });
    let project = ctx
        .swarm_repo
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
            .swarm_repo
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
            .swarm_repo
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
        let _ = crate::product_swarm::seed_tasks(&ctx2, &project2, &creator, &goal).await;
        if start {
            let _ = ctx2.swarm_repo.set_swarm_status(&swarm_id, "active").await;
            crate::swarm_runtime::set_paused(&ctx2, &swarm_id, false);
            crate::swarm_runtime::start_coordinator(ctx2.clone(), swarm_id.clone());
            crate::swarm_runtime::emit_status(&ctx2, &workspace_id, &swarm_id, "active");
        }
    });

    Ok(project.id)
}

/// Reply/escalate back to the channel that launched `project` (if any). Best-effort.
pub async fn notify_origin(ctx: &ServerCtx, project: &SwarmProject, text: &str) {
    let (Some(channel), Some(chat)) = (project.origin_channel.as_deref(), project.origin_chat.as_deref())
    else {
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
            let ch = if channel == "slack" { Channel::Slack } else { Channel::Telegram };
            if let Ok(Some(integ)) = ctx.integrations_store.get(&project.workspace_id, ch).await {
                let _ = otto_channels::improve_notify::send_to(&ctx.secrets, &integ, chat, thread, text).await;
            }
        }
        _ => {}
    }
}

/// The `SwarmTrigger` implementation injected into the channel `Bridge`: an inbound
/// message on a swarm-bound channel launches that swarm.
pub struct SwarmTriggerImpl {
    pub ctx: ServerCtx,
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
            .swarm_repo
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
                let kw = t.keyword.trim();
                let lower = text.trim_start();
                if !lower.to_lowercase().starts_with(&kw.to_lowercase()) {
                    continue;
                }
                lower[kw.len()..].trim().to_string()
            };
            if body.is_empty() {
                continue;
            }
            let swarm = match self.ctx.swarm_repo.get_swarm(&t.swarm_id).await {
                Ok(s) => s,
                Err(_) => continue,
            };
            let opts = LaunchOpts {
                goal: body.clone(),
                name: None,
                repo_path: t.repo_path.clone(),
                goals: Vec::new(),
                origin: if t.reply {
                    Some(Origin { channel: channel.to_string(), chat: chat.to_string(), thread: thread.map(str::to_string) })
                } else {
                    None
                },
                start: t.auto_start,
                created_by: swarm.created_by.clone(),
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
                    return Some(LaunchAck { reply: format!("⚠️ Could not launch the swarm: {e}") });
                }
            }
        }
        None
    }
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}
