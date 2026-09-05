//! Whole-corpus replay (design §4.1). Ignored by default — it walks the real
//! `~/.claude/projects` and `~/.codex/sessions` trees:
//!
//! ```sh
//! OTTO_TRANSCRIPT_CORPUS=$HOME cargo test -p otto-transcript --test corpus -- --ignored --nocapture
//! ```
//!
//! Asserts: no panic on any file, and `stats.unknown_records == 0` everywhere
//! except the synthetic fixtures Otto's own tests leave under
//! `/var/folders/**/T--tmp*` project dirs (skipped). Prints the per-provider
//! census recorded in the design doc §8.

use std::path::{Path, PathBuf};
use std::time::Instant;

use otto_transcript::{fold_file, model::Block, FoldOpts, Provider};

fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out, depth + 1);
        } else if p.extension().is_some_and(|x| x == "jsonl") {
            out.push(p);
        }
    }
}

/// Claude project dirs named after a temp cwd (`/var/folders/**/T/tmp*` →
/// `-var-folders-…-T-tmp…` / `T--tmp…`) hold Otto's synthetic test fixtures.
fn is_synthetic(p: &Path) -> bool {
    p.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s.starts_with("-var-folders-") || s.starts_with("-private-var-folders-")
    }) && p.to_string_lossy().contains("-T--tmp")
        || p.to_string_lossy().contains("/T--tmp")
}

#[derive(Default)]
struct Census {
    files: usize,
    records: usize,
    turns: u64,
    tool_calls: u64,
    unknown: u64,
    bad_files: Vec<(PathBuf, u64)>,
    longest: (PathBuf, usize),
    largest_bytes: (PathBuf, u64),
    reasoning: u64,
    thinking: u64,
    images: u64,
    subagent_blocks: u64,
    artifacts: u64,
}

fn run(provider: Provider, files: &[PathBuf]) -> Census {
    let mut c = Census::default();
    for f in files {
        if is_synthetic(f) {
            continue;
        }
        let size = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
        let started = Instant::now();
        let folded = std::panic::catch_unwind(|| fold_file(provider, f, FoldOpts::default()))
            .unwrap_or_else(|_| panic!("PANIC folding {}", f.display()))
            .unwrap_or_else(|e| panic!("read {}: {e}", f.display()));
        let _ = started;
        c.files += 1;
        c.records += folded.record_count;
        c.turns += folded.stats.turns;
        c.tool_calls += folded.stats.tool_calls;
        c.unknown += folded.stats.unknown_records;
        c.reasoning += folded.stats.reasoning_steps;
        c.thinking += folded.stats.thinking_steps;
        c.artifacts += folded.artifacts.len() as u64;
        for t in &folded.turns {
            for b in &t.turn.blocks {
                match b {
                    Block::Image { .. } => c.images += 1,
                    Block::Subagent { .. } => c.subagent_blocks += 1,
                    _ => {}
                }
            }
        }
        if folded.stats.unknown_records > 0 {
            c.bad_files.push((f.clone(), folded.stats.unknown_records));
        }
        if folded.record_count > c.longest.1 {
            c.longest = (f.clone(), folded.record_count);
        }
        if size > c.largest_bytes.1 {
            c.largest_bytes = (f.clone(), size);
        }
    }
    c
}

fn report(label: &str, c: &Census) {
    println!(
        "{label}: files={} records={} turns={} tool_calls={} unknown_records={} reasoning={} thinking={} images={} subagent_blocks={} artifacts={}",
        c.files, c.records, c.turns, c.tool_calls, c.unknown, c.reasoning, c.thinking, c.images, c.subagent_blocks, c.artifacts
    );
    println!(
        "{label}: longest file = {} ({} records); largest = {} ({} bytes)",
        c.longest.0.display(),
        c.longest.1,
        c.largest_bytes.0.display(),
        c.largest_bytes.1
    );
    for (f, n) in c.bad_files.iter().take(30) {
        println!("{label}: UNKNOWN x{n} in {}", f.display());
    }
}

#[test]
#[ignore = "walks the real local corpus; set OTTO_TRANSCRIPT_CORPUS=$HOME"]
fn whole_local_corpus_folds_without_unknown_records() {
    let root = std::env::var("OTTO_TRANSCRIPT_CORPUS")
        .map(PathBuf::from)
        .expect("OTTO_TRANSCRIPT_CORPUS not set");
    let t0 = Instant::now();

    let mut claude = Vec::new();
    walk(&root.join(".claude/projects"), &mut claude, 0);
    // Only top-level `<sid>.jsonl` + `subagents/*.jsonl`; both are Claude shape.
    claude.sort();
    let cc = run(Provider::Claude, &claude);
    report("claude", &cc);

    let mut codex = Vec::new();
    walk(&root.join(".codex/sessions"), &mut codex, 0);
    codex.sort();
    let xc = run(Provider::Codex, &codex);
    report("codex", &xc);

    println!("total wall time: {:.1}s", t0.elapsed().as_secs_f64());
    assert!(cc.files + xc.files >= 100, "corpus too small to be meaningful");
    assert_eq!(cc.unknown, 0, "claude unknown records: {:?}", cc.bad_files);
    assert_eq!(xc.unknown, 0, "codex unknown records: {:?}", xc.bad_files);
}
