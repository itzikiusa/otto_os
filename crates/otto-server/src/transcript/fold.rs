//! The server's transcript fold knobs (architecture-3-03r): where a
//! transcript's extracted images live and the price function, shared by the
//! conversation routes (`routes/transcript.rs`) and the live tailer
//! (`transcript_tail.rs`).

use std::path::Path;

use otto_transcript::{FoldOpts, ImageStore, Provider};

use crate::state::ServerCtx;

/// Where a transcript's extracted images live: `<data>/transcripts/<key>/img`.
/// `key` is the provider session id (so a session and its History entry share
/// one store) — validated to a plain token so it can never leave `transcripts/`.
pub(crate) fn image_store(ctx: &ServerCtx, key: &str) -> ImageStore {
    let safe: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let safe = if safe.is_empty() {
        "unknown".to_string()
    } else {
        safe
    };
    ImageStore::new(ctx.data_dir.join("transcripts").join(safe).join("img"))
}

/// Image-store key for a transcript path: the provider session id derived from
/// the filename (Claude `<sid>.jsonl`, Codex `rollout-…-<uuid>.jsonl`).
pub(crate) fn store_key(provider: Provider, path: &Path) -> String {
    otto_transcript::peek::session_id_from_name(provider, path).unwrap_or_else(|| "unknown".into())
}

/// The server's fold knobs for a transcript at `path`: images written to the
/// per-transcript store, cost priced exactly like the Usage module. Owned
/// (`'static`) so a live tail can keep it across polls.
pub(crate) fn fold_opts(ctx: &ServerCtx, provider: Provider, path: &Path) -> FoldOpts<'static> {
    let price: otto_transcript::PriceFn<'static> = &otto_usage::estimate_cost;
    FoldOpts {
        images: Some(image_store(ctx, &store_key(provider, path))),
        price: Some(price),
        subagents: Vec::new(),
    }
}
