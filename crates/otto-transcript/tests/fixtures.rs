//! Redacted real-world fixtures (`fixtures/{claude,codex-new,codex-old}`),
//! produced by `scripts/redact-transcript.py` from the local corpus. Every file
//! must fold with zero unknown records; a handful of shape-specific checks pin
//! the features each fixture was picked for.

use std::path::{Path, PathBuf};

use otto_transcript::model::*;
use otto_transcript::{fold_file, read_subagents, FoldOpts, Folded, Provider};

fn fixtures(sub: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(sub);
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    out.sort();
    out
}

fn fold(provider: Provider, sub: &str, name: &str) -> Folded {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(sub)
        .join(format!("{name}.jsonl"));
    fold_file(provider, &p, FoldOpts::default()).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn blocks(f: &Folded) -> Vec<&Block> {
    f.turns.iter().flat_map(|t| t.turn.blocks.iter()).collect()
}

fn tool_calls(f: &Folded) -> Vec<(&str, ToolKind, Option<&ToolResult>)> {
    blocks(f)
        .into_iter()
        .filter_map(|b| match b {
            Block::ToolCall { name, tool, result, .. } => Some((name.as_str(), *tool, result.as_ref())),
            _ => None,
        })
        .collect()
}

#[test]
fn every_fixture_folds_cleanly() {
    let mut n = 0;
    for (sub, provider) in [
        ("claude", Provider::Claude),
        ("codex-new", Provider::Codex),
        ("codex-old", Provider::Codex),
    ] {
        for p in fixtures(sub) {
            let f = fold_file(provider, &p, FoldOpts::default()).unwrap();
            assert_eq!(f.stats.unknown_records, 0, "{}", p.display());
            assert!(!f.turns.is_empty(), "{} produced no turns", p.display());
            assert!(f.record_count > 0);
            // Paging round-trips: the last page's cursor pages back consistently.
            let page = f.page(None, 2, vec![]);
            let cursor: usize = page.cursor.parse().unwrap();
            let earlier = f.page(Some(cursor), 100, vec![]);
            assert_eq!(earlier.turns.len() + page.turns.len(), f.turns.len().min(earlier.turns.len() + page.turns.len()));
            n += 1;
        }
    }
    // The subagent files are Claude-shaped too.
    let sub_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude/03-agent-subagents/subagents");
    for e in std::fs::read_dir(&sub_dir).unwrap().flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "jsonl") {
            let f = fold_file(Provider::Claude, &p, FoldOpts::default()).unwrap();
            assert_eq!(f.stats.unknown_records, 0, "{}", p.display());
            n += 1;
        }
    }
    assert!(n >= 20, "expected ≥20 fixture files, found {n}");
}

#[test]
fn claude_fixtures_pin_their_features() {
    let f = fold(Provider::Claude, "claude", "01-basic-tools");
    let calls = tool_calls(&f);
    assert!(calls.iter().any(|(n, k, r)| *n == "Edit" && *k == ToolKind::Edit && r.is_some()));
    assert!(calls.iter().any(|(_, k, _)| *k == ToolKind::Shell));
    assert!(f.stats.input_tokens.is_some());
    assert!(f.turns.iter().any(|t| t.turn.role == Role::User));

    let f = fold(Provider::Claude, "claude", "02-image-paste");
    assert!(blocks(&f).iter().any(|b| matches!(b, Block::Image { .. })));
    assert!(f.artifacts.iter().any(|a| a.kind == ArtifactKind::Image));

    let f = fold(Provider::Claude, "claude", "04-task-create-update");
    assert!(blocks(&f).iter().any(|b| matches!(b, Block::Tasks { tasks } if !tasks.is_empty())));

    let f = fold(Provider::Claude, "claude", "06-queue-operation");
    assert!(blocks(&f).iter().any(|b| matches!(b, Block::Queued { .. })));

    let f = fold(Provider::Claude, "claude", "07-pr-link");
    assert!(f.artifacts.iter().any(|a| a.kind == ArtifactKind::Pr && a.url.is_some()));
    assert!(blocks(&f).iter().any(|b| matches!(b, Block::Artifact { .. })));

    let f = fold(Provider::Claude, "claude", "08-mcp-toolsearch");
    let calls = tool_calls(&f);
    assert!(calls.iter().any(|(_, k, _)| *k == ToolKind::Mcp));
    assert!(calls.iter().any(|(n, k, r)| *n == "ToolSearch" && *k == ToolKind::Search && r.is_some()));

    let f = fold(Provider::Claude, "claude", "09-bare-string-result");
    assert!(tool_calls(&f).iter().any(|(_, _, r)| r.is_some_and(|r| !r.ok)));

    let f = fold(Provider::Claude, "claude", "10-write-structured-patch");
    assert!(tool_calls(&f).iter().any(|(_, _, r)| r.is_some_and(|r| r.patch.is_some() && r.file_path.is_some())));
    assert!(f.artifacts.iter().any(|a| a.kind == ArtifactKind::File));

    let f = fold(Provider::Claude, "claude", "11-cost-state");
    assert!(f.stats.cost_usd.is_some() || f.stats.input_tokens.is_some());

    // A `<system-reminder>` INSIDE a tool result is tool output, not a note.
    let f = fold(Provider::Claude, "claude", "13-system-reminder-in-tool-result");
    assert!(tool_calls(&f).iter().any(|(_, _, r)| r.is_some_and(|r| r.text.as_deref().is_some_and(|t| t.contains("<system-reminder>")))));

    let f = fold(Provider::Claude, "claude", "14-stop-hook");
    assert!(f.turns.iter().any(|t| t.turn.system.iter().any(|n| n.kind == SystemNoteKind::Hook)));

    // Hand-written: 1e300 tokens, u64::MAX output tokens twice, two u64::MAX
    // durations — must fold (with overflow checks on) without a panic.
    let f = fold(Provider::Claude, "claude", "15-overflow");
    assert_eq!(f.stats.unknown_records, 0);
    assert_eq!(f.stats.duration_ms, Some(u64::MAX), "saturates, never wraps");
    assert_eq!(f.stats.output_tokens, Some(u64::MAX));
    assert_eq!(f.stats.input_tokens, Some(0), "1e300 is rejected, not saturated to 2^64");
}

#[test]
fn claude_subagent_tree_attaches_to_agent_calls() {
    let main = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude/03-agent-subagents.jsonl");
    let tree = read_subagents(&main);
    assert_eq!(tree.len(), 4);
    assert!(tree.iter().all(|m| m.tool_use_id.is_some()));
    let f = fold_file(Provider::Claude, &main, FoldOpts { subagents: tree.clone(), ..Default::default() }).unwrap();
    let subs: Vec<&Block> = blocks(&f).into_iter().filter(|b| matches!(b, Block::Subagent { .. })).collect();
    assert_eq!(subs.len(), 4, "one subagent block per sidecar");
    // Each sits right after its Agent call.
    for t in &f.turns {
        let bl = &t.turn.blocks;
        for (i, b) in bl.iter().enumerate() {
            if matches!(b, Block::Subagent { .. }) {
                assert!(matches!(&bl[i - 1], Block::ToolCall { name, .. } if name == "Agent"));
            }
        }
    }
}

#[test]
fn codex_new_era_fixtures_pin_their_features() {
    let f = fold(Provider::Codex, "codex-new", "01-command-agent-message");
    assert!(tool_calls(&f).iter().any(|(_, k, r)| *k == ToolKind::Shell && r.is_some()));
    assert!(blocks(&f).iter().any(|b| matches!(b, Block::Text { .. })));
    assert!(f.turns.iter().all(|t| t.turn.id.ends_with(":u") || t.turn.id.ends_with(":a")), "new-era ids");
    assert!(f.stats.reasoning_steps > 0);

    let f = fold(Provider::Codex, "codex-new", "02-file-change-token-usage");
    assert!(tool_calls(&f).iter().any(|(_, k, r)| matches!(k, ToolKind::Edit | ToolKind::Write) && r.is_some_and(|r| r.patch.is_some())));
    assert!(f.stats.input_tokens.is_some());
    assert!(!f.artifacts.is_empty());

    let f = fold(Provider::Codex, "codex-new", "03-mcp-tool-call");
    assert!(tool_calls(&f).iter().any(|(_, k, _)| *k == ToolKind::Mcp));

    let f = fold(Provider::Codex, "codex-new", "04-context-compaction");
    assert!(f.turns.iter().any(|t| t.turn.system.iter().any(|n| n.kind == SystemNoteKind::Compaction)));

    let f = fold(Provider::Codex, "codex-new", "05-extension-imageview");
    assert!(tool_calls(&f).iter().any(|(_, k, _)| matches!(k, ToolKind::Web | ToolKind::Read | ToolKind::Other)));
}

#[test]
fn codex_old_era_fixtures_pin_their_features() {
    let f = fold(Provider::Codex, "codex-old", "07-exec-command");
    assert!(f.turns.iter().all(|t| t.turn.id.starts_with('r')), "old-era ids are r<index>");
    assert!(tool_calls(&f).iter().any(|(n, k, r)| *n == "exec_command" && *k == ToolKind::Shell && r.is_some()));
    assert!(f.turns.iter().any(|t| t.turn.role == Role::User));

    let f = fold(Provider::Codex, "codex-old", "08-custom-exec-patch");
    assert!(tool_calls(&f).iter().any(|(_, k, r)| matches!(k, ToolKind::Edit | ToolKind::Write) && r.is_some_and(|r| r.patch.is_some())));
    assert!(tool_calls(&f).iter().any(|(n, _, _)| *n == "exec_command" || *n == "exec"));

    let f = fold(Provider::Codex, "codex-old", "09-mcp-end");
    assert!(tool_calls(&f).iter().any(|(_, k, _)| *k == ToolKind::Mcp));

    let f = fold(Provider::Codex, "codex-old", "10-web-search");
    assert!(tool_calls(&f).iter().any(|(_, k, _)| *k == ToolKind::Web));

    let f = fold(Provider::Codex, "codex-old", "11-compacted");
    assert!(f.turns.iter().any(|t| t.turn.system.iter().any(|n| n.kind == SystemNoteKind::Compaction)));
}
