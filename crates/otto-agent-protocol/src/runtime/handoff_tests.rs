use super::*;
use crate::driver::item;
use otto_core::agent_runtime::{Attachment, PlanStep, ProviderFailure, TurnItemStatus};

struct T {
    items: Vec<TurnItem>,
}

impl T {
    fn new() -> Self {
        Self { items: Vec::new() }
    }
    fn push(&mut self, turn: Option<&str>, kind: TurnItemKind) -> &mut TurnItem {
        let mut i = item(
            format!("i{}", self.items.len()),
            TurnItemStatus::Completed,
            None,
            kind,
        );
        i.ordinal = self.items.len() as u32 + 1;
        i.turn_id = turn.map(Into::into);
        self.items.push(i);
        self.items.last_mut().unwrap()
    }
    fn user(&mut self, turn: &str, text: &str) {
        self.push(
            Some(turn),
            TurnItemKind::UserMessage {
                text: text.into(),
                attachments: Vec::new(),
            },
        );
    }
    fn answer(&mut self, turn: &str, text: &str) {
        self.push(
            Some(turn),
            TurnItemKind::AssistantMessage {
                message_id: "m".into(),
                text: text.into(),
                streaming: false,
            },
        );
    }
    fn build(&self, budget: usize) -> ContextHandoff {
        build(
            self.items.iter(),
            std::iter::empty(),
            "claude",
            "Claude Code",
            "codex",
            budget,
        )
    }
}

#[test]
fn an_empty_thread_hands_over_nothing() {
    let h = T::new().build(BUDGET_TOKENS);
    assert_eq!(h.text, "");
    assert_eq!((h.turns, h.omitted_turns), (0, 0));
    assert_eq!(prompt_text(&h, "hi"), "hi");
}

#[test]
fn turns_keep_message_answer_and_files_but_not_tool_output() {
    let mut t = T::new();
    t.user("a", "add a README");
    t.push(
        Some("a"),
        TurnItemKind::CommandExecution {
            input: "ls".into(),
            output: Some("SECRET-TOOL-OUTPUT".into()),
            exit_code: Some(0),
        },
    );
    t.push(
        Some("a"),
        TurnItemKind::FileChange {
            file_name: "/w/README.md".into(),
            diff_str: Some("+DIFF-BODY".into()),
            old_str: None,
            new_str: None,
        },
    );
    // A subagent's own tool call is the subagent's (counted once).
    t.push(
        Some("a"),
        TurnItemKind::Subagent {
            tool_name: "Task".into(),
            description: Some("look around".into()),
            subagent_type: None,
            prompt: None,
            output: None,
            background: false,
            last_tool: None,
            tool_uses: 3,
        },
    );
    t.push(
        Some("a"),
        TurnItemKind::CommandExecution {
            input: "find".into(),
            output: None,
            exit_code: None,
        },
    )
    .parent_item_id = Some("i3".into());
    t.answer("a", "Added README.md.");
    t.user("b", "now a LICENSE");
    t.answer("b", "Added LICENSE.");
    let h = t.build(BUDGET_TOKENS);
    assert_eq!((h.turns, h.omitted_turns), (2, 0));
    assert!(h.text.starts_with("[Context handoff from Claude Code]"));
    assert!(h.text.ends_with("[End of handoff]"));
    for want in [
        "Person: add a README",
        "Agent: Added README.md.",
        "Files changed: /w/README.md",
        "Agent work: 1 command, 1 subagent",
        "Person: now a LICENSE",
    ] {
        assert!(h.text.contains(want), "missing {want:?} in {}", h.text);
    }
    assert!(!h.text.contains("SECRET-TOOL-OUTPUT") && !h.text.contains("DIFF-BODY"));
    // Oldest first in the block.
    assert!(
        h.text.find("Turn 1 (Claude Code):").unwrap()
            < h.text.find("Turn 2 (Claude Code):").unwrap()
    );
    // The person's new message follows the block whole.
    let long = "x".repeat(200_000);
    assert!(prompt_text(&h, &long).ends_with(&format!("[End of handoff]\n\n{long}")));
}

#[test]
fn the_budget_keeps_the_newest_whole_turns_and_counts_the_rest() {
    let mut t = T::new();
    for n in 0..10 {
        let id = format!("t{n}");
        t.user(&id, &format!("question {n}"));
        t.answer(&id, &format!("answer {n} {}", "y".repeat(400)));
    }
    // About 110 tokens per turn: a 350-token budget takes three.
    let h = t.build(350);
    assert_eq!((h.turns, h.omitted_turns), (3, 7));
    assert!(h.text.contains("7 earlier turn(s) are not included"));
    for n in 0..10 {
        let whole = format!("answer {n} {}", "y".repeat(400));
        let has_q = h.text.contains(&format!("Person: question {n}\n"));
        // Whole or absent, never cut.
        assert_eq!(has_q, h.text.contains(&whole), "turn {n}");
        assert_eq!(has_q, n >= 7, "turn {n}");
    }
    assert!(h.text.contains("Turn 8 (Claude Code):") && h.text.contains("Turn 10 (Claude Code):"));
}

#[test]
fn a_newest_turn_over_the_budget_is_left_out_not_cut() {
    let mut t = T::new();
    t.user("a", "small");
    t.answer("a", "ok");
    t.user("b", "big");
    t.answer("b", &"z".repeat(10_000));
    let h = t.build(500);
    assert_eq!((h.turns, h.omitted_turns), (0, 2));
    assert!(!h.text.contains(&"z".repeat(100)));
    assert!(h.text.contains("2 earlier turn(s) are not included"));
}

#[test]
fn a_plain_fresh_start_cuts_history_but_an_earlier_handoff_does_not() {
    let mut t = T::new();
    t.user("a", "forgotten");
    t.push(Some("b"), TurnItemKind::SessionBoundary { handoff: None });
    t.user("b", "kept one");
    t.push(
        Some("c"),
        TurnItemKind::SessionBoundary {
            handoff: Some(ContextHandoff {
                from_provider: "codex".into(),
                to_provider: "agy".into(),
                ..Default::default()
            }),
        },
    );
    t.user("c", "kept two");
    let h = t.build(BUDGET_TOKENS);
    assert!(!h.text.contains("forgotten"));
    assert!(h.text.contains("kept one") && h.text.contains("kept two"));
    assert_eq!(h.turns, 2);
    // Each turn names the agent that ran it, across earlier switches.
    assert!(
        h.text.contains("Turn 1 (Codex):\nPerson: kept one"),
        "{}",
        h.text
    );
    assert!(
        h.text.contains("Turn 2 (Antigravity):\nPerson: kept two"),
        "{}",
        h.text
    );
}

#[test]
fn an_open_plan_leads_images_and_errors_are_noted() {
    let mut t = T::new();
    t.push(
        Some("a"),
        TurnItemKind::UserMessage {
            text: "see".into(),
            attachments: vec![Attachment {
                id: "img".into(),
                kind: AttachmentKind::Image,
                mime: "image/png".into(),
                name: None,
            }],
        },
    );
    t.push(
        Some("a"),
        TurnItemKind::Error {
            failure: ProviderFailure {
                class: "provider_error".into(),
                message: "rate limited".into(),
                code: None,
                retryable: None,
            },
        },
    );
    let step = |text: &str, status| PlanStep {
        id: text.into(),
        text: text.into(),
        status,
    };
    let open = PlanArtifact {
        id: "p1".into(),
        turn_id: Some("a".into()),
        steps: vec![
            step("write it", PlanStepStatus::Completed),
            step("test it", PlanStepStatus::Pending),
        ],
        explanation: None,
    };
    let done = PlanArtifact {
        id: "p0".into(),
        turn_id: Some("a".into()),
        steps: vec![step("old step", PlanStepStatus::Completed)],
        explanation: None,
    };
    let plans = [done, open];
    let h = build(
        t.items.iter(),
        plans.iter(),
        "claude",
        "Claude Code",
        "gemini",
        BUDGET_TOKENS,
    );
    assert_eq!(h.images_omitted, 1);
    assert!(h
        .text
        .contains("1 image(s) were attached here; they are not handed over."));
    assert!(h.text.contains("Error: rate limited"));
    assert!(h.text.contains("Open plan:\n- [x] write it\n- [ ] test it"));
    assert!(!h.text.contains("old step"));
    assert!(h.text.find("Open plan").unwrap() < h.text.find("Turn 1 (Claude Code):").unwrap());
}
