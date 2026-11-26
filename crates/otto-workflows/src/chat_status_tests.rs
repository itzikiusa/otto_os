use super::*;
use chrono::TimeZone;
use otto_channels::test_util::{Recorded, RecordingAdapter};
use otto_core::agent_runtime::WaitReason;
use otto_core::domain::Channel;
use otto_core::workflows::{
    ChildThread, Deliverable, FailedStep, QueueInfo, RunHold, Subject, SubjectPr, WorkflowEdge,
};
use serde_json::json;

fn node(id: &str, kind: &str, name: &str) -> WorkflowNode {
    serde_json::from_value(json!({"id": id, "kind": kind, "name": name})).unwrap()
}

/// The PR Reviewer of engine.md §9.2, as a chain.
fn graph() -> WorkflowGraph {
    let nodes = vec![
        node("n1", "manual_trigger", "Start"),
        node("n2", "prepare_context", "Fetch Jira ticket"),
        node("n3", "git_checkout", "Checkout PR branch"),
        node("n4", "agent_prompt", "Gather context (Vault)"),
        node("n5", "agent_prompt", "Cross-repo contract check"),
        node("n6", "script", "Go PR gates"),
        node("n7", "review_run", "Multi-agent review"),
        node("n8", "agent_prompt", "Draft PR summary"),
        node("n9", "post_review_comments", "Post review comments"),
    ];
    let edges = nodes
        .windows(2)
        .map(|w| WorkflowEdge {
            id: format!("{}-{}", w[0].id, w[1].id),
            source: w[0].id.clone(),
            target: w[1].id.clone(),
            condition: None,
        })
        .collect();
    WorkflowGraph { nodes, edges }
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 26, 8, 50, 0).unwrap()
}

fn view() -> ChatView<'static> {
    ChatView {
        workflow: "PR Reviewer",
        now: now(),
        tz: FixedOffset::east_opt(2 * 3600).unwrap(),
    }
}

fn st(id: &str, status: NodeStatus) -> NodeRunState {
    NodeRunState {
        node_id: id.into(),
        status,
        ..Default::default()
    }
}

fn done(id: &str, ms: u64, brief: Option<&str>) -> NodeRunState {
    NodeRunState {
        duration_ms: Some(ms),
        brief: brief.map(str::to_owned),
        ..st(id, NodeStatus::Success)
    }
}

fn child(status: &str) -> ChildThread {
    serde_json::from_value(json!({"id": "c", "label": "L", "provider": "claude", "status": status}))
        .unwrap()
}

/// The run of engine.md §9.2's example, mid-review.
fn mid_review() -> Vec<NodeRunState> {
    vec![
        done("n1", 0, None),
        done("n2", 0, None),
        done("n3", 14_000, Some("PR #271 feature/ABC-123 → develop")),
        done("n4", 112_000, None),
        NodeRunState {
            error: Some("one repo".into()),
            ..st("n5", NodeStatus::Skipped)
        },
        done("n6", 68_000, Some("4 passed, 1 not run")),
        NodeRunState {
            started_at: Some(now() - chrono::Duration::seconds(260)),
            phase: Some(StepPhase::Working),
            children: ["success", "success", "error", "running", "queued"]
                .map(child)
                .to_vec(),
            ..st("n7", NodeStatus::Running)
        },
    ]
}

fn running_overview() -> RunOverview {
    RunOverview {
        state: RunState::Running,
        steps_done: 5,
        steps_total: 9,
        elapsed_ms: 400_000,
        typical_ms: Some(720_000),
        ..Default::default()
    }
}

#[test]
fn status_message_is_the_designed_card_word_for_word() {
    assert_eq!(
        render_status(&view(), &running_overview(), &mid_review(), &graph()),
        "*PR Reviewer* · reviewing · 5 of 9 steps · 6m 40s (usually 12m)\n\
         ✅ Fetch Jira ticket · 0s\n\
         ✅ Checkout PR branch · 14s — PR #271 feature/ABC-123 → develop\n\
         ✅ Gather context (Vault) · 1m 52s\n\
         ⏭ Cross-repo contract check — skipped (one repo)\n\
         ✅ Go PR gates · 1m 08s — 4 passed, 1 not run\n\
         ▶ Multi-agent review · 4m 20s — 3 of 5 reviewers done\n\
         ◻ Draft PR summary\n\
         ◻ Post review comments"
    );
}

#[test]
fn waiting_needs_you_failed_and_stopped_steps_say_so() {
    let mut nodes = mid_review();
    nodes[5] = NodeRunState {
        phase: Some(StepPhase::Waiting),
        wait: Some(WaitReason {
            code: "slots".into(),
            text: "waiting for a free agent slot (6 of 6 running)".into(),
            since: Utc.with_ymd_and_hms(2026, 11, 26, 8, 42, 0).unwrap(),
            detail: None,
        }),
        ..st("n6", NodeStatus::Running)
    };
    nodes[6] = NodeRunState {
        phase: Some(StepPhase::NeedsYou),
        attention: Some(Attention::new(
            AttentionKind::Approval,
            "Approve posting 6 comments on PR #412",
            now(),
        )),
        ..st("n7", NodeStatus::Running)
    };
    nodes.push(NodeRunState {
        error: Some("go build failed in payments-api\nexit status 1".into()),
        ..st("n8", NodeStatus::Error)
    });
    nodes.push(NodeRunState {
        error_class: Some("canceled".into()),
        ..st("n9", NodeStatus::Skipped)
    });
    let text = render_status(&view(), &running_overview(), &nodes, &graph());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[5],
        "⏳ Go PR gates — waiting for a free agent slot (6 of 6 running) since 10:42"
    );
    assert_eq!(
        lines[6],
        "⚠️ Multi-agent review — Approve posting 6 comments on PR #412"
    );
    assert_eq!(
        lines[7],
        "❌ Draft PR summary failed — go build failed in payments-api"
    );
    assert_eq!(lines[8], "⏹ Post review comments — stopped");
    assert!(
        lines[0].starts_with("*PR Reviewer* · reviewing · "),
        "{}",
        lines[0]
    );
}

#[test]
fn a_queued_run_says_where_it_is_in_line() {
    let overview = RunOverview {
        state: RunState::Queued,
        queue: Some(QueueInfo {
            position: 2,
            ahead: 1,
            cap: 2,
            lane: None,
        }),
        ..Default::default()
    };
    assert_eq!(
        render_status(&view(), &overview, &[], &graph()),
        "*PR Reviewer* · queued\n⏳ Queued — 2nd in line"
    );
}

#[test]
fn without_an_overview_the_counts_and_clock_come_from_the_nodes() {
    let mut nodes = mid_review();
    nodes[1].started_at = Some(now() - chrono::Duration::seconds(605));
    let text = render_status(&view(), &RunOverview::default(), &nodes, &graph());
    assert!(
        text.starts_with("*PR Reviewer* · reviewing · 5 of 8 steps · 10m 05s\n"),
        "{text}"
    );
}

#[test]
fn a_long_card_folds_its_finished_steps() {
    let mut g = WorkflowGraph::default();
    let mut nodes = vec![];
    for i in 0..60 {
        g.nodes
            .push(node(&format!("s{i}"), "agent_prompt", &format!("Step {i}")));
        nodes.push(done(&format!("s{i}"), 1_000, Some(&"brief ".repeat(15))));
    }
    g.nodes.push(node("last", "agent_prompt", "Last step"));
    nodes.push(st("last", NodeStatus::Running));
    let text = render_status(&view(), &RunOverview::default(), &nodes, &g);
    assert!(text.chars().count() < STATUS_FOLD_CHARS, "{}", text.len());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[1], "✅ 60 steps done");
    assert!(lines[2].starts_with("▶ Last step · "));
}

#[test]
fn utility_kinds_stay_off_the_card_and_the_review_is_on_it() {
    for kind in ["manual_trigger", "log", "transform", "delay", "condition"] {
        assert!(!is_status_step(kind), "{kind}");
    }
    for kind in ["review_run", "agent_prompt", "channel_notify", "loop"] {
        assert!(is_status_step(kind), "{kind}");
    }
}

fn finished(state: RunState) -> RunOverview {
    RunOverview {
        state,
        elapsed_ms: 712_000,
        subject: Some(Subject {
            pr: Some(SubjectPr {
                repo: "r".into(),
                number: 271,
                url: "https://example.test/pr/271".into(),
                title: None,
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn deliverable(label: &str, state: DeliverableState, detail: Option<&str>) -> Deliverable {
    Deliverable {
        label: label.into(),
        state,
        detail: detail.map(str::to_owned),
        url: None,
    }
}

#[test]
fn final_post_of_a_passed_review_is_the_designed_text() {
    let mut overview = finished(RunState::Succeeded);
    overview.deliverables = vec![
        deliverable(
            "PR comments",
            DeliverableState::Done,
            Some("5 of 5 posted — PR #271"),
        ),
        deliverable("PR summary comment", DeliverableState::Done, None),
    ];
    let nodes = vec![NodeRunState {
        output: Some(json!({"score": 86, "passed": true})),
        ..done("n7", 1, None)
    }];
    assert_eq!(
        render_final(&view(), &overview, &nodes, &graph(), None),
        "✅ *PR Reviewer* passed — 86/100 · 11m 52s\n\
         • PR comments: 5 of 5 posted — PR #271\n\
         • PR summary comment: posted"
    );
}

#[test]
fn a_review_below_its_pass_mark_is_never_called_passed() {
    let nodes = vec![NodeRunState {
        output: Some(json!({"score": 69, "passed": false, "threshold": 80})),
        ..done("n7", 1, None)
    }];
    assert_eq!(
        render_final(
            &view(),
            &finished(RunState::Succeeded),
            &nodes,
            &graph(),
            None
        ),
        "⚠️ *PR Reviewer* done — review 69/100, not passed · 11m 52s"
    );
}

#[test]
fn final_post_never_says_posted_for_what_was_not() {
    let mut overview = finished(RunState::Succeeded);
    overview.deliverables = vec![
        deliverable(
            "Jira comment",
            DeliverableState::Failed,
            Some("no permission to comment"),
        ),
        deliverable("PR comments", DeliverableState::PendingApproval, None),
        deliverable("Confluence page", DeliverableState::Unknown, None),
        deliverable(
            "Inline comments",
            DeliverableState::Partial,
            Some("3 of 4 posted"),
        ),
    ];
    let text = render_final(
        &view(),
        &overview,
        &[],
        &graph(),
        Some("https://mac.ts.net/#/workflows/w/runs/r"),
    );
    assert_eq!(
        text,
        "⚠️ *PR Reviewer* done · 11m 52s\n\
         • Jira comment: not posted (no permission to comment)\n\
         • PR comments: pending approval\n\
         • Confluence page: not confirmed\n\
         • Inline comments: 3 of 4 posted\n\
         [Open in Otto](https://mac.ts.net/#/workflows/w/runs/r)"
    );
}

#[test]
fn a_failed_run_leads_with_the_failing_steps_own_error() {
    let mut overview = finished(RunState::Failed);
    overview.failed_step = Some(FailedStep {
        node_id: "n6".into(),
        name: "Go PR gates".into(),
        error: "go build failed in payments-api".into(),
        error_class: Some("script".into()),
    });
    overview.deliverables = vec![deliverable(
        "PR comments",
        DeliverableState::PendingApproval,
        None,
    )];
    assert_eq!(
        render_final(&view(), &overview, &[], &graph(), None),
        "❌ *PR Reviewer* failed at *Go PR gates* — go build failed in payments-api\n\
         • PR comments: pending approval\n\
         Nothing was posted on PR #271."
    );
    // No overview failed_step: the first failed node's error, by its name.
    let mut bare = finished(RunState::Failed);
    bare.subject = None;
    let nodes = vec![NodeRunState {
        error: Some("review_run: score 62 is below 80".into()),
        ..st("n7", NodeStatus::Error)
    }];
    assert_eq!(
        render_final(&view(), &bare, &nodes, &graph(), None),
        "❌ *PR Reviewer* failed at *Multi-agent review* — review_run: score 62 is below 80"
    );
}

#[test]
fn canceled_and_held_runs_say_what_happened() {
    let nodes = vec![NodeRunState {
        error_class: Some("canceled".into()),
        ..st("n6", NodeStatus::Skipped)
    }];
    assert_eq!(
        render_final(
            &view(),
            &finished(RunState::Canceled),
            &nodes,
            &graph(),
            None
        ),
        "⏹ *PR Reviewer* canceled at *Go PR gates* · 11m 52s"
    );
    for (kind, want) in [
        (HoldKind::Paused, "⏸ *PR Reviewer* paused — paused by Dana"),
        (
            HoldKind::Blocked,
            "⚠️ *PR Reviewer* is blocked — the build is red",
        ),
        (
            HoldKind::Exhausted,
            "⚠️ *PR Reviewer* is out of budget — 5 of 5 iterations used",
        ),
    ] {
        let reason = want.split(" — ").nth(1).unwrap();
        let mut overview = finished(RunState::Paused);
        overview.hold = Some(RunHold {
            kind,
            reason: reason.into(),
            since: now(),
            by: None,
            node_ids: vec![],
            iteration: None,
        });
        assert_eq!(render_final(&view(), &overview, &[], &graph(), None), want);
    }
}

#[test]
fn needs_you_is_a_new_post_with_the_mention_on_slack_only() {
    let approval = Attention::new(AttentionKind::Approval, "", now());
    assert_eq!(
        render_needs_you(
            &view(),
            "Post review comments",
            &approval,
            Some("U42"),
            None
        ),
        "⚠️ <@U42> *PR Reviewer* needs you — *Post review comments* is waiting for an approval. \
         Approve it in Otto (Workflows → Runs)."
    );
    let question = Attention::new(AttentionKind::Question, "Which repo holds the DTOs?", now());
    assert_eq!(
        render_needs_you(&view(), "Gather context", &question, None, Some("https://m/#/r")),
        "⚠️ *PR Reviewer* needs you — *Gather context* has a question — Which repo holds the DTOs?. \
         Answer it in Otto (Workflows → Runs).\n[Open in Otto](https://m/#/r)"
    );
}

#[test]
fn captions_name_the_step_and_are_redacted() {
    assert_eq!(
        step_caption("Checkout PR branch", &done("n3", 14_000, None), None),
        "✅ Checkout PR branch · 14s"
    );
    let failed = NodeRunState {
        error: Some(format!("push refused for ghp_{}\nmore", "a1B2".repeat(9))),
        ..st("n6", NodeStatus::Error)
    };
    let caption = step_caption("Go PR gates", &failed, None);
    assert!(
        caption.starts_with("❌ Go PR gates failed — push refused for "),
        "{caption}"
    );
    assert!(
        !caption.contains("ghp_a1B2") && !caption.contains("more"),
        "{caption}"
    );
    assert_eq!(
        step_caption("Fix", &done("l", 1, None), Some(2)),
        "› ✅ Fix · iteration 2"
    );
}

#[test]
fn durations_read_as_designed() {
    assert_eq!(
        [0, 14_000, 68_000, 712_000, 3_900_000].map(clock),
        ["0s", "14s", "1m 08s", "11m 52s", "1h 05m"]
    );
    assert_eq!(
        [45_000, 720_000, 3_900_000].map(approx),
        ["45s", "12m", "1h 5m"]
    );
    assert_eq!(
        [1, 2, 3, 4, 11, 12, 13, 21, 22].map(ordinal),
        ["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd"]
    );
}

#[test]
fn the_style_comes_from_the_input_then_the_start_node() {
    let mut g = graph();
    assert_eq!(ChatUpdates::resolve(&g, &json!({})), ChatUpdates::Live);
    g.nodes[0].params = json!({"chat_updates": "per_step"});
    assert_eq!(ChatUpdates::resolve(&g, &json!({})), ChatUpdates::PerStep);
    assert_eq!(
        ChatUpdates::resolve(&g, &json!({"chat_updates": "live"})),
        ChatUpdates::Live
    );
    assert_eq!(
        ChatUpdates::resolve(&graph(), &json!({"chat_updates": "per_step"})),
        ChatUpdates::PerStep
    );
}

fn status_msg() -> StatusMessage {
    StatusMessage {
        channel: "slack".into(),
        chat: "C1".into(),
        thread: Some("t1".into()),
        message_id: None,
    }
}

#[tokio::test]
async fn live_status_posts_once_then_edits_and_skips_unchanged_text() {
    let rec = RecordingAdapter::new(Channel::Slack, "ws-live-1", "C1", Some("t1"));
    let mut live = LiveStatus::new(status_msg());
    assert_eq!(live.wait(), Duration::ZERO);
    assert!(
        live.show(rec.as_ref(), "a").await.unwrap(),
        "first show posts"
    );
    assert!(live.wait() > Duration::from_secs(2));
    assert!(!live.show(rec.as_ref(), "a").await.unwrap());
    assert!(!live.show(rec.as_ref(), "b").await.unwrap());
    let log = rec.recorded();
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(
        matches!(&log[0], Recorded::Send { id, thread: Some(t), formatted: true, .. } if id == "m1" && t == "t1")
    );
    assert!(
        matches!(&log[1], Recorded::Edit { message_id, text, formatted: true, .. } if message_id == "m1" && text == "b")
    );
    assert_eq!(live.msg.message_id.as_deref(), Some("m1"));
}

#[tokio::test]
async fn a_lost_status_message_is_posted_again_and_a_stored_one_is_edited() {
    let rec = RecordingAdapter::new(Channel::Slack, "ws-live-2", "C1", Some("t1"));
    // After a restart: the stored message is edited, not posted again.
    let mut live = LiveStatus::new(StatusMessage {
        message_id: Some("m0".into()),
        ..status_msg()
    });
    assert!(!live.show(rec.as_ref(), "a").await.unwrap());
    rec.fail_next_edit("slack chat.update: message_not_found");
    assert!(live.show(rec.as_ref(), "b").await.unwrap(), "re-posted");
    assert_eq!(live.msg.message_id.as_deref(), Some("m1"));
    live.show(rec.as_ref(), "c").await.unwrap();
    let log = rec.recorded();
    assert!(matches!(&log[0], Recorded::Edit { message_id, .. } if message_id == "m0"));
    assert!(matches!(&log[1], Recorded::Send { id, text, .. } if id == "m1" && text == "b"));
    assert!(
        matches!(&log[2], Recorded::Edit { message_id, text, .. } if message_id == "m1" && text == "c")
    );
}

#[tokio::test(start_paused = true)]
async fn a_rate_limit_is_waited_out_and_other_errors_are_retried_later() {
    let rec = RecordingAdapter::new(Channel::Slack, "ws-live-3", "C1", Some("t1"));
    let mut live = LiveStatus::new(status_msg());
    live.show(rec.as_ref(), "a").await.unwrap();
    rec.fail_next_edit("slack chat.update: ratelimited (retry after 2s)");
    let t0 = tokio::time::Instant::now();
    assert!(!live.show(rec.as_ref(), "b").await.unwrap());
    assert!(
        t0.elapsed() >= Duration::from_secs(2),
        "waited the asked delay"
    );
    rec.fail_next_edit("slack chat.update: internal_error");
    assert!(live.show(rec.as_ref(), "c").await.is_err());
    // The failed text was not taken as shown: the next update sends it.
    live.show(rec.as_ref(), "c").await.unwrap();
    let edits: Vec<String> = rec
        .recorded()
        .into_iter()
        .filter_map(|r| match r {
            Recorded::Edit { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(edits, ["b", "c"]);
}

#[test]
fn the_stored_status_message_round_trips() {
    let msg = StatusMessage {
        message_id: Some("1.2".into()),
        ..status_msg()
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(
        v,
        json!({"channel": "slack", "chat": "C1", "thread": "t1", "message_id": "1.2"})
    );
    assert_eq!(serde_json::from_value::<StatusMessage>(v).unwrap(), msg);
}

/// The thread in each style for the same run (owner's answer 1): live keeps
/// every step file and the final output, and only the text lines go.
async fn drive(style: ChatUpdates, ws: &str) -> Vec<Recorded> {
    let rec = RecordingAdapter::new(Channel::Slack, ws, "C1", Some("t1"));
    let mut thread = ChatThread::new(style, rec.clone(), status_msg());
    thread.status("*PR Reviewer* · working".into()).await;
    thread.line("▶ *Checkout* started").await;
    thread.line("✅ *Checkout* done (14.0s)").await;
    thread
        .status("*PR Reviewer* · working · 1 of 2".into())
        .await;
    thread
        .step_file("step2-checkout.md", b"# Checkout", "✅ Checkout · 14s")
        .await;
    thread.line("❌ *Gates* failed").await;
    thread
        .step_file("step3-gates.md", b"# Gates", "❌ Gates failed — build")
        .await;
    thread
        .post("❌ *PR Reviewer* failed at *Gates* — build")
        .await;
    thread.file("final-output.md", b"# Final").await;
    rec.recorded()
}

fn uploads(log: &[Recorded]) -> Vec<(String, Option<String>)> {
    log.iter()
        .filter_map(|r| match r {
            Recorded::Upload { name, caption, .. } => Some((name.clone(), caption.clone())),
            _ => None,
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn live_style_drops_only_the_text_lines_and_keeps_every_file() {
    let live = drive(ChatUpdates::Live, "ws-thread-1").await;
    assert_eq!(
        uploads(&live),
        [
            (
                "step2-checkout.md".to_string(),
                Some("✅ Checkout · 14s".to_string())
            ),
            (
                "step3-gates.md".to_string(),
                Some("❌ Gates failed — build".to_string())
            ),
            ("final-output.md".to_string(), None),
        ]
    );
    let sends: Vec<&str> = live
        .iter()
        .filter_map(|r| match r {
            Recorded::Send { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        sends,
        [
            "*PR Reviewer* · working",
            "❌ *PR Reviewer* failed at *Gates* — build"
        ],
        "the status message first, then the final post; no step lines"
    );
    assert!(
        matches!(&live[0], Recorded::Send { .. }),
        "the status message is the first post"
    );
    // The second status text came within 3 s: kept, then shown before the final post.
    let final_at = live
        .iter()
        .position(|r| matches!(r, Recorded::Send { text, .. } if text.starts_with("❌")))
        .unwrap();
    assert!(matches!(&live[final_at - 1], Recorded::Edit { text, .. } if text.ends_with("1 of 2")));
}

#[tokio::test]
async fn per_step_style_is_todays_lines_and_uncaptioned_files() {
    let old = drive(ChatUpdates::PerStep, "ws-thread-2").await;
    let shape: Vec<String> = old
        .iter()
        .map(|r| match r {
            Recorded::Send { text, .. } => format!("send {text}"),
            Recorded::Edit { .. } => "edit".into(),
            Recorded::Upload { name, caption, .. } => format!("upload {name} {caption:?}"),
        })
        .collect();
    assert_eq!(
        shape,
        [
            "send ▶ *Checkout* started",
            "send ✅ *Checkout* done (14.0s)",
            "upload step2-checkout.md None",
            "send ❌ *Gates* failed",
            "upload step3-gates.md None",
            "send ❌ *PR Reviewer* failed at *Gates* — build",
            "upload final-output.md None",
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_rate_limited_upload_is_retried_and_a_failed_one_never_stops_the_rest() {
    let rec = RecordingAdapter::new(Channel::Slack, "ws-thread-3", "C1", Some("t1"));
    let mut thread = ChatThread::new(ChatUpdates::Live, rec.clone(), status_msg());
    rec.fail_next_upload("slack upload PUT: ratelimited (retry after 3s)");
    let t0 = tokio::time::Instant::now();
    thread.step_file("a.md", b"a", "✅ A · 1s").await;
    assert!(t0.elapsed() >= Duration::from_secs(3));
    rec.fail_next_upload("slack files.completeUploadExternal: not_in_channel");
    thread.step_file("b.md", b"b", "✅ B · 1s").await;
    thread.step_file("c.md", b"c", "✅ C · 1s").await;
    let names: Vec<String> = uploads(&rec.recorded())
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(
        names,
        ["a.md", "c.md"],
        "b failed and was skipped; c still arrived in order"
    );
}

#[tokio::test(start_paused = true)]
async fn status_edits_collapse_to_the_newest_with_a_trailing_edit() {
    let rec = RecordingAdapter::new(Channel::Slack, "ws-thread-4", "C1", Some("t1"));
    let mut thread = ChatThread::new(ChatUpdates::Live, rec.clone(), status_msg());
    assert!(thread.status("v1".into()).await, "the first status posts");
    for v in ["v2", "v3", "v4"] {
        assert!(!thread.status(v.into()).await);
    }
    let wait = thread.flush_in().expect("a kept text");
    assert!(wait <= STATUS_EDIT_EVERY);
    tokio::time::sleep(wait).await;
    thread.flush().await;
    assert_eq!(thread.flush_in(), None);
    let log = rec.recorded();
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(matches!(&log[1], Recorded::Edit { text, .. } if text == "v4"));
}
