//! Built-in workflow templates with no server dependency (moved out of
//! otto-server's routes): pure graph builders the template routes list.

use otto_core::workflows::{WorkflowEdge, WorkflowGraph, WorkflowNode, WorkflowTemplate};

/// Build the three game-pipeline templates. Each chains:
/// trigger → agent (design rules) → game_engine (kind) → verifier.
pub fn game_templates() -> Vec<WorkflowTemplate> {
    fn pipeline(game: &str, design: &str) -> WorkflowGraph {
        let node =
            |id: &str, kind: &str, name: &str, x: f64, params: serde_json::Value| WorkflowNode {
                id: id.into(),
                kind: kind.into(),
                name: name.into(),
                x,
                y: 70.0,
                params,
                retry: None,
            };
        let edge = |s: &str, t: &str| WorkflowEdge {
            id: format!("{s}-{t}"),
            source: s.into(),
            target: t.into(),
            condition: None,
        };
        WorkflowGraph {
            nodes: vec![
                node(
                    "trigger",
                    "manual_trigger",
                    "Start",
                    40.0,
                    serde_json::Value::Null,
                ),
                node(
                    "design",
                    "agent_prompt",
                    "Design game",
                    320.0,
                    serde_json::json!({ "prompt": design }),
                ),
                node(
                    "game",
                    "game_engine",
                    "Build game",
                    600.0,
                    serde_json::json!({ "game": game }),
                ),
                node(
                    "verify",
                    "verifier",
                    "Verify",
                    880.0,
                    serde_json::Value::Null,
                ),
            ],
            edges: vec![
                edge("trigger", "design"),
                edge("design", "game"),
                edge("game", "verify"),
            ],
        }
    }

    vec![
        WorkflowTemplate {
            id: "game-slots".into(),
            name: "Slots game".into(),
            description: "5×3 slot machine: agent designs the paytable & RTP, then the engine assembles and verifies it.".into(),
            instructions: String::new(),
            icon: "grid".into(),
            graph: pipeline(
                "slots",
                "Design a 5x3 slot game for the given theme: reel symbols, paytable, win lines, RTP ~96%, volatility, and a bonus feature. Return a structured spec.",
            ),
        },
        WorkflowTemplate {
            id: "game-crash".into(),
            name: "Crash game (Aviator style)".into(),
            description: "Aviator-style crash game: agent designs the multiplier curve & provably-fair RNG, then build & verify.".into(),
            instructions: String::new(),
            icon: "zap".into(),
            graph: pipeline(
                "crash",
                "Design an aviator-style crash game: the multiplier growth curve, a provably-fair RNG seed/commit scheme, auto-cashout rules, house edge ~3%, and max multiplier. Return a structured spec.",
            ),
        },
        WorkflowTemplate {
            id: "game-scratch".into(),
            name: "Scratch card".into(),
            description: "Scratch-card game: agent designs prize tiers & win probabilities, then build & verify.".into(),
            instructions: String::new(),
            icon: "ticket".into(),
            graph: pipeline(
                "scratch",
                "Design a scratch-card game: prize tiers and their win probabilities, panel layout, reveal mechanic, and RTP ~95%. Return a structured spec.",
            ),
        },
    ]
}
