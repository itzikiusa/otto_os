//! Cold-open cost breakdown of one transcript, phase by phase, exactly as the
//! daemon's `GET …/transcript` pays it (read → parse → fold → cache charge →
//! page → serialize). Usage:
//!
//! ```text
//! cargo run --release -p otto-transcript --example fold_bench -- <file.jsonl> [runs]
//! ```
//!
//! `FOLD_BENCH_ONLY=collect|stream` runs just that fold once (whole-file
//! `Vec<Value>` vs streamed) so `/usr/bin/time -l` shows its peak RSS.
use std::path::PathBuf;
use std::time::Instant;

use otto_transcript::{fold, fold_bytes, parse_records, read_subagents, Block, FoldOpts, Provider};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().expect("usage: fold_bench <file.jsonl> [runs]"));
    let runs: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(3);
    let provider = otto_transcript::provider_for_path(&path).unwrap_or(Provider::Claude);
    if let Ok(only) = std::env::var("FOLD_BENCH_ONLY") {
        let bytes = std::fs::read(&path).unwrap();
        let t = Instant::now();
        let folded = match only.as_str() {
            "collect" => fold(provider, &parse_records(&bytes), FoldOpts::default()),
            _ => fold_bytes(provider, &bytes, FoldOpts::default()),
        };
        println!("{only}: {} turns in {:.1} ms", folded.turns.len(), ms(t));
        return;
    }
    for run in 0..runs {
        let t = Instant::now();
        let bytes = std::fs::read(&path).unwrap();
        let read_ms = ms(t);

        let t = Instant::now();
        let records = parse_records(&bytes);
        let parse_ms = ms(t);

        let t = Instant::now();
        let subagents = read_subagents(&path);
        let sub_ms = ms(t);

        let t = Instant::now();
        let folded = fold(
            provider,
            &records,
            FoldOpts {
                images: None,
                price: None,
                subagents: subagents.clone(),
            },
        );
        let fold_ms = ms(t);

        // `transcript_cache::Snapshot::charge` serializes every turn.
        let t = Instant::now();
        let mut charge = 0usize;
        for ft in &folded.turns {
            charge += serde_json::to_vec(&ft.turn).map(|v| v.len()).unwrap_or(0);
        }
        let charge_ms = ms(t);

        // The live tail's `live_page` snapshot is a full clone.
        let t = Instant::now();
        let cloned = folded.clone();
        let clone_ms = ms(t);
        drop(cloned);

        let t = Instant::now();
        let page = folded.page(None, 60, subagents.clone());
        let page_ms = ms(t);

        let t = Instant::now();
        let body = serde_json::to_vec(&page).unwrap();
        let ser_ms = ms(t);

        let t = Instant::now();
        drop(records);
        let drop_ms = ms(t);

        // What the daemon runs now: the streamed fold (parse + fold + drop).
        let t = Instant::now();
        let streamed = fold_bytes(
            provider,
            &bytes,
            FoldOpts {
                images: None,
                price: None,
                subagents: subagents.clone(),
            },
        );
        let stream_ms = ms(t);
        assert_eq!(streamed.turns.len(), folded.turns.len());

        println!(
            "run {run}: file {:.1} MB, {} records, {} turns, {} subagents | read {read_ms:.1} ms, parse {parse_ms:.1} ms, sidecars {sub_ms:.1} ms, fold {fold_ms:.1} ms, charge {charge_ms:.1} ms ({:.1} MB), clone {clone_ms:.1} ms, page {page_ms:.1} ms, serialize {ser_ms:.1} ms, drop-records {drop_ms:.1} ms | streamed parse+fold {stream_ms:.1} ms | page: {} turns, {:.0} KB",
            bytes.len() as f64 / 1e6,
            folded.record_count,
            folded.turns.len(),
            subagents.len(),
            charge as f64 / 1e6,
            page.turns.len(),
            body.len() as f64 / 1024.0,
        );
        if run == 0 {
            breakdown(&page);
        }
    }
}

/// Where the first page's bytes go, by block kind.
fn breakdown(page: &otto_transcript::Transcript) {
    let mut by: std::collections::BTreeMap<&str, (usize, usize)> = Default::default();
    let mut add = |k: &'static str, n: usize| {
        let e = by.entry(k).or_default();
        e.0 += 1;
        e.1 += n;
    };
    for t in &page.turns {
        add("turn.system", serde_json::to_vec(&t.system).unwrap().len());
        for b in &t.blocks {
            match b {
                Block::ToolCall { input, result, .. } => {
                    add("tool.input", serde_json::to_vec(input).unwrap().len());
                    if let Some(r) = result {
                        add("tool.result.text", r.text.as_ref().map_or(0, String::len));
                        add("tool.result.patch", r.patch.as_ref().map_or(0, String::len));
                    }
                }
                Block::Text { md } => add("text", md.len()),
                other => add("other", serde_json::to_vec(other).unwrap().len()),
            }
        }
    }
    add(
        "subagents",
        serde_json::to_vec(&page.subagents).unwrap().len(),
    );
    for (k, (n, b)) in by {
        println!("  {k:<18} {n:>6} × → {:>8.1} KB", b as f64 / 1024.0);
    }
}
