//! The `game_engine` workflow node, moved verbatim out of
//! `execute_node`'s match (see `nodes/mod.rs`).

use super::super::*;
use super::NodeCall;

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let NodeCall { node, input, .. } = call;
    let p = &node.params;
    let kind = p.get("game").and_then(Value::as_str).unwrap_or("slots");
    let assets = input
        .get("outputs")
        .cloned()
        .unwrap_or_else(|| input.clone());
    // Game-kind-specific spec (structured scaffold).
    let spec = match kind {
        "crash" => json!({
            "type": "crash",
            "rng": { "scheme": "provably-fair HMAC-SHA256", "house_edge": 0.03 },
            "multiplier_curve": "exponential",
            "auto_cashout": true,
            "max_multiplier": 1000.0,
        }),
        "scratch" => json!({
            "type": "scratch",
            "rng": { "algorithm": "xoshiro256**" },
            "prize_tiers": [
                { "label": "JACKPOT", "p": 0.001 },
                { "label": "BIG", "p": 0.02 },
                { "label": "SMALL", "p": 0.18 },
                { "label": "NONE", "p": 0.799 }
            ],
            "panels": 9,
            "rtp": 0.95,
        }),
        _ => json!({
            "type": "slots",
            "rng": { "algorithm": "xoshiro256**" },
            "reels": 5,
            "rows": 3,
            "paytable": "auto-generated",
            "rtp": 0.96,
        }),
    };
    let build = json!({
        "engine": "otto-games/0.1 (scaffold)",
        "game": kind,
        "spec": spec,
        "assets": assets,
        "note": "Scaffold build: wire a real game engine here.",
    });
    Ok((
        json!({ "build": build }),
        vec![format!("assembled {kind} game (scaffold)")],
    ))
}
