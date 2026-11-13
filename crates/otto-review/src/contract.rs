//! The contract between the review agents' PROMPTS and the Rust that reads
//! their answers — one place for every answer format.
//!
//! Each format has ONE canonical example of a valid answer below. The prompt
//! embeds that constant (never a hand-typed copy) and `contract_tests` feeds
//! the same constant through the production reader, so a prompt and its
//! parser cannot drift apart silently. Every list of allowed values a prompt
//! offers (severities, dispositions) is rendered from the list the checker
//! uses. Three reviews were lost to exactly that drift: the summarizer's
//! `lens_score` element (#149), the verifier's `unverified` disposition
//! (#153) and an object-shaped scope plan (#150, #154).
//!
//! | Format | Prompt | Reader | Example |
//! |---|---|---|---|
//! | reviewer findings + lens score | `prompts::reviewer_prompt` | `session::parse_findings_array`, `session::parse_lens_score` | [`REVIEWER_OUTPUT_EXAMPLE`] |
//! | orchestrator per-lens file | `engine::compose_orchestrator_prompt` | `session::merge_lens_files` | [`LENS_FILE_EXAMPLE`] |
//! | scope plan (in its transport) | `scope::scope_prompt` | `scope::plan_from_run` → `scope::parse_plan` | [`SCOPE_PLAN_EXAMPLE`], [`scope_transport_example`] |
//! | verifier comments + notes + score | `prompts::summarizer_prompt` + `prompts::lens_scores_block` | `engine::parse_draft_comments`, `summarizer::valid_result`, `verification::settle` | [`VERIFIER_OUTPUT_EXAMPLE`] |
//! | prior-finding verdict | `prompts::prior_finding_rules` | `resolution::confirmed_fixed` | built from `resolution::FIXED_PRIOR_DISPOSITION` |

/// Severities a reviewer may give a finding, with what each means. Rendered
/// into every reviewer prompt by [`severity_choices`] / [`severity_legend`];
/// `session::normalize_severity` maps each onto itself.
pub const FINDING_SEVERITIES: [(&str, &str); 3] = [
    ("info", "optional"),
    ("warn", "should fix"),
    ("bug", "must fix"),
];

/// `"info"|"warn"|"bug"` — the severity choices as the JSON schemas spell them.
pub fn severity_choices() -> String {
    FINDING_SEVERITIES
        .iter()
        .map(|(s, _)| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join("|")
}

/// `bug=must fix, warn=should fix, info=optional` — most severe first.
pub fn severity_legend() -> String {
    FINDING_SEVERITIES
        .iter()
        .rev()
        .map(|(s, meaning)| format!("{s}={meaning}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The one-finding schema the lens prompts name.
pub fn finding_schema() -> String {
    format!(
        "{{\"path\":string,\"line\":number,\"severity\":{},\"body\":string}}",
        severity_choices()
    )
}

/// The verifier's disposition for a claim it confirmed and publishes. Every
/// other accepted word is one of [`crate::engine::NOTE_DISPOSITIONS`].
pub const VERIFIED: &str = "verified";

/// Every disposition the verifier prompt offers and the check accepts.
pub fn verifier_dispositions() -> Vec<&'static str> {
    std::iter::once(VERIFIED)
        .chain(crate::engine::NOTE_DISPOSITIONS)
        .collect()
}

/// Dispositions the scope assessment may give a previous finding.
pub const PRIOR_SCOPE_DISPOSITIONS: [&str; 3] = ["fixed", "still_open", "not_relevant"];

/// Most lenses one scope plan may select.
pub const MAX_SCOPE_LENSES: usize = 10;

/// Disposition of the stand-in for a verifier item that could not be read
/// at all. Never offered to the agent; `verification::settle` turns these
/// into one "could not be read" note.
pub const UNREADABLE: &str = "unreadable";

/// A reviewer's complete answer: its findings, then its lens score.
pub const REVIEWER_OUTPUT_EXAMPLE: &str = r#"[{"path":"src/auth.rs","line":42,"severity":"bug","body":"allow() now returns true before the session check, and api.rs:8 calls it for anonymous requests, so an unauthenticated caller reads the document. Fix: check the session first."},{"lens_score":4,"score_reason":"one must-fix access-control defect"}]"#;

/// One orchestrator sub-agent's per-lens file: findings tagged with the lens.
pub const LENS_FILE_EXAMPLE: &str = r#"[{"path":"src/auth.rs","line":42,"severity":"bug","body":"allow() skips the session check for anonymous callers (api.rs:8).","lens":"security-review"}]"#;

/// The finding id [`SCOPE_PLAN_EXAMPLE`] accounts for — a placeholder, so the
/// agent cannot mistake it for a real previous finding.
pub const SCOPE_EXAMPLE_PRIOR_ID: &str = "<id of a previous finding listed above>";

/// The scope assessor's plan object (it travels as the body string of
/// [`scope_transport_example`]).
pub const SCOPE_PLAN_EXAMPLE: &str = r#"{"summary":"Changes the permission predicate in src/auth.rs:42, which the document API calls at src/api.rs:8.","related_modules":["src/api.rs:8 — calls allow() before reading a document"],"risks":["anonymous callers could read documents"],"lenses":[{"name":"Security","reason":"src/auth.rs:42 changes who may read a document","instructions":"Trace every caller of allow() and check the anonymous path."}],"prior_findings":[{"finding_id":"<id of a previous finding listed above>","disposition":"still_open","evidence":"src/auth.rs:42 still skips the session check"}]}"#;

/// The scope assessor's complete answer: a one-element array whose `body` is
/// [`SCOPE_PLAN_EXAMPLE`] encoded as a string.
pub fn scope_transport_example() -> String {
    serde_json::json!([{ "severity": "info", "body": SCOPE_PLAN_EXAMPLE }]).to_string()
}

/// The final verifier's complete answer for a 2-claim batch and a 1-claim
/// batch: one verified comment covering two claims, one rejected claim kept as
/// a note, and the overall score.
pub const VERIFIER_OUTPUT_EXAMPLE: &str = r#"[{"path":"src/auth.rs","line":42,"severity":"bug","category":"security","title":"Anonymous callers pass the permission check","body":"allow() returns true before the session check, so an unauthenticated request reads the document.","evidence":"src/auth.rs:42 returns before the session check; src/api.rs:8 calls allow() for anonymous requests","reasoning":"Access control is bypassed on a public route.","suggested_fix":"Check the session before the role in allow().","disposition":"verified","source_ids":["batch:1:claim:1","batch:2:claim:1"]},{"body":"Claim batch:1:claim:2 (missing null check in parse())","disposition":"not_relevant","evidence":"src/parse.rs:10 rejects null before the call","source_ids":["batch:1:claim:2"]},{"lens_score":5,"score_reason":"one verified must-fix defect"}]"#;

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
