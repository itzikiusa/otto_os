//! Unit tests for the product runners' pure helpers: scratch cwds, JSON
//! extraction, the prompt builders and the self-improvement narratives.

use super::plan::PlanFindings;
use super::prompts::{build_plan_summarizer_prompt, PLAN_AUTONOMY_DIRECTIVE};
use super::*;

// -----------------------------------------------------------------------
// session_cwd — per-session cwd attribution
// -----------------------------------------------------------------------

#[test]
fn output_contract_publishes_complete_json_atomically_last() {
    let prompt = augment_with_out_path("task", "/tmp/result.json");
    assert!(prompt.contains("sibling temporary file"));
    assert!(prompt.contains("atomically rename"));
    assert!(prompt.contains("LAST action"));
}

#[test]
fn session_cwd_passes_real_path_through_unchanged() {
    // A real directory (use the manifest dir of this crate, which surely
    // exists and is NOT the temp dir) must pass through verbatim — the
    // architecture lens needs the actual repo cwd.
    let real = env!("CARGO_MANIFEST_DIR");
    let out = session_cwd(real);
    assert_eq!(out, real, "a real story cwd must pass through unchanged");
}

#[test]
fn session_cwd_rewrites_shared_temp_fallback_to_existing_child() {
    // The fallback string used by the handlers.
    let fallback = std::env::temp_dir().to_string_lossy().to_string();
    let out = session_cwd(&fallback);

    // It must NOT be the shared temp dir itself.
    assert_ne!(out, fallback, "shared temp fallback must be rewritten");

    let out_path = std::path::Path::new(&out);
    // It must be a child of the temp dir...
    assert!(
        out_path.starts_with(std::env::temp_dir()),
        "rewritten cwd must live under the temp dir; got {out}"
    );
    // ...and it must actually exist on disk (created by the helper).
    assert!(
        out_path.exists(),
        "rewritten cwd must exist on disk; got {out}"
    );

    // Cleanup.
    let _ = std::fs::remove_dir_all(out_path);
}

/// S4-14: product scratch dirs share one stable root, and stale ones are swept.
#[test]
fn session_cwd_uses_a_stable_swept_root() {
    let fallback = std::env::temp_dir().to_string_lossy().to_string();
    let out = session_cwd(&fallback);
    let root = std::env::temp_dir().join(SCRATCH_ROOT);
    assert!(std::path::Path::new(&out).starts_with(&root), "{out}");
    let _ = std::fs::remove_dir_all(&out);
    // The sweep removes children older than max-age (private root, so
    // parallel tests' fresh dirs are never touched).
    let mine = tempfile::tempdir().unwrap();
    let child = mine.path().join("old");
    std::fs::create_dir_all(&child).unwrap();
    let live = mine.path().join("live");
    std::fs::create_dir_all(&live).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let keep: std::collections::HashSet<String> =
        [live.to_string_lossy().into_owned()].into_iter().collect();
    sweep_stale_scratch(mine.path(), std::time::Duration::from_secs(3600), &keep);
    assert!(child.exists(), "fresh scratch kept");
    sweep_stale_scratch(mine.path(), std::time::Duration::from_millis(1), &keep);
    assert!(!child.exists(), "stale scratch swept");
    // S4-304: an old dir an unarchived session still uses survives.
    assert!(live.exists(), "a live session's scratch cwd is never swept");
}

#[test]
fn session_cwd_two_calls_yield_distinct_dirs() {
    let fallback = std::env::temp_dir().to_string_lossy().to_string();
    let a = session_cwd(&fallback);
    let b = session_cwd(&fallback);
    assert_ne!(a, b, "two fallback rewrites must yield distinct dirs");

    // Cleanup.
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

// -----------------------------------------------------------------------
// extract_json_block
// -----------------------------------------------------------------------

#[test]
fn extract_json_block_finds_json_in_fence() {
    let input = "Here is the result:\n```json\n{\"a\":1}\n```\n";
    let v = extract_json_block(input).expect("should find json");
    assert_eq!(v["a"], 1);
}

#[test]
fn extract_json_block_finds_bare_object() {
    let input = r#"Some prose. {"key": "value", "n": 42}"#;
    let v = extract_json_block(input).expect("should find bare object");
    assert_eq!(v["key"], "value");
    assert_eq!(v["n"], 42);
}

#[test]
fn extract_json_block_returns_none_for_prose() {
    let input = "This is just some text without any JSON at all.";
    assert!(extract_json_block(input).is_none());
}

#[test]
fn extract_json_block_fence_wins_over_prose_brace() {
    // There is a `{` in prose text BEFORE the fenced block; the fence should win.
    let input = "intro text { not json } then\n```json\n{\"a\":1}\n```";
    let v = extract_json_block(input).expect("should find fenced json");
    // The fenced {"a":1} wins, not the prose brace.
    assert_eq!(v["a"], 1);
    // Make sure the prose brace wasn't interpreted as the result.
    assert!(v.get("not").is_none());
}

#[test]
fn extract_json_as_skips_a_decoy_that_does_not_fit_the_shape() {
    // A quoted `{}`-ish snippet precedes the plan object; taking the first
    // span and then deserializing read the run as "no parseable output".
    let input = r##"Config is `{"name":"svc"}`. {"plan_markdown":"# Plan"}"##;
    let plan = extract_json_as::<PlanFindings>(input).expect("plan found past the decoy");
    assert_eq!(plan.plan_markdown, "# Plan");
    assert!(extract_json_block(input)
        .and_then(|v| serde_json::from_value::<PlanFindings>(v).ok())
        .is_none());
}

#[test]
fn extract_json_block_empty_string_returns_none() {
    assert!(extract_json_block("").is_none());
}

#[test]
fn extract_json_block_nested_json_balanced() {
    let input = r#"{"outer":{"inner":"yes"},"list":[1,2,3]}"#;
    let v = extract_json_block(input).expect("should parse nested json");
    assert_eq!(v["outer"]["inner"], "yes");
}

// -----------------------------------------------------------------------
// build_analysis_prompt
// -----------------------------------------------------------------------

#[test]
fn build_analysis_prompt_includes_skill_body() {
    let skill = "## Skill: Analyse stories\nBe thorough.";
    let prompt = build_analysis_prompt(skill, "/tmp/ctx.md", None);
    assert!(
        prompt.contains(skill),
        "prompt must include the entire skill body; got:\n{prompt}"
    );
}

#[test]
fn build_analysis_prompt_includes_context_path_reference() {
    let ctx_path = "/tmp/otto-product-TEST-context.md";
    let prompt = build_analysis_prompt("skill body", ctx_path, None);
    assert!(
        prompt.contains(ctx_path),
        "prompt must include the context file path; got:\n{prompt}"
    );
    // The prompt should tell the agent to read the file.
    assert!(
        prompt.to_lowercase().contains("read"),
        "prompt should instruct agent to read the context file; got:\n{prompt}"
    );
}

#[test]
fn build_analysis_prompt_includes_output_contract_marker() {
    let prompt = build_analysis_prompt("skill body", "/tmp/ctx.md", None);
    // The key distinguishing text from the OUTPUT CONTRACT
    assert!(
        prompt.contains("EXACTLY ONE"),
        "prompt must include OUTPUT CONTRACT text 'EXACTLY ONE'; got:\n{prompt}"
    );
    assert!(
        prompt.contains("suggested_learnings"),
        "prompt must include OUTPUT CONTRACT field 'suggested_learnings'"
    );
    assert!(
        prompt.contains("open_questions"),
        "prompt must include OUTPUT CONTRACT field 'open_questions'"
    );
}

#[test]
fn build_analysis_prompt_includes_prior_summary_when_given() {
    let prompt =
        build_analysis_prompt("skill", "/tmp/ctx.md", Some("Previously we found X and Y."));
    assert!(
        prompt.contains("Previously we found X and Y."),
        "prompt must include prior summary"
    );
}

#[test]
fn build_analysis_prompt_omits_prior_summary_section_when_none() {
    let prompt = build_analysis_prompt("skill", "/tmp/ctx.md", None);
    assert!(
        !prompt.contains("Prior Analysis Summary"),
        "no prior summary section when None"
    );
}

// -----------------------------------------------------------------------
// augment_with_out_path — the write-to-file instruction
// -----------------------------------------------------------------------

#[test]
fn augment_with_out_path_appends_write_instruction_and_path() {
    let base = build_analysis_prompt("skill body", "/tmp/otto-product-X-context.md", None);
    let out = augment_with_out_path(&base, "/tmp/otto-product-X-0.json");
    // The context path is preserved.
    assert!(out.contains("/tmp/otto-product-X-context.md"));
    // The write-to-file path is present.
    assert!(
        out.contains("/tmp/otto-product-X-0.json"),
        "augmented prompt must include the out_path; got:\n{out}"
    );
    // It instructs writing JSON to the file.
    assert!(out.to_lowercase().contains("write"));
    assert!(out.contains("JSON"));
}

// -----------------------------------------------------------------------
// build_summarizer_prompt — consolidate/dedupe instruction + schema marker
// -----------------------------------------------------------------------

#[test]
fn build_summarizer_prompt_includes_consolidate_dedupe_and_questions_schema() {
    let lenses = vec![
        (
            "PO Overview".to_string(),
            "claude".to_string(),
            r#"{"summary":"a","open_questions":[{"text":"q1"}]}"#.to_string(),
        ),
        (
            "Architecture".to_string(),
            "codex".to_string(),
            r#"{"summary":"b","risks":["r1"]}"#.to_string(),
        ),
    ];
    let prompt = build_summarizer_prompt("synth skill", "Login Story", &lenses);

    // Consolidate / dedupe / resolve-conflict instructions are present.
    assert!(
        prompt.to_lowercase().contains("consolidate"),
        "summarizer prompt must instruct consolidation; got:\n{prompt}"
    );
    assert!(
        prompt.to_uppercase().contains("DEDUPE"),
        "summarizer prompt must instruct dedupe; got:\n{prompt}"
    );
    assert!(
        prompt.to_lowercase().contains("conflict"),
        "summarizer prompt must instruct conflict resolution; got:\n{prompt}"
    );
    // The output schema marker: a `questions` array (the summarizer schema).
    assert!(
        prompt.contains("\"questions\""),
        "summarizer prompt must include the questions schema marker; got:\n{prompt}"
    );
    assert!(
        prompt.contains("conflict_notes"),
        "summarizer prompt must include conflict_notes schema field; got:\n{prompt}"
    );
    // Each lens is labelled with name + provider, and its JSON is fed in.
    assert!(prompt.contains("Lens PO Overview (claude)"));
    assert!(prompt.contains("Lens Architecture (codex)"));
    assert!(prompt.contains(r#"{"summary":"a","open_questions":[{"text":"q1"}]}"#));
    // The story title is included.
    assert!(prompt.contains("Login Story"));
}

#[test]
fn summary_findings_parses_summarizer_schema() {
    let raw = r#"```json
{"summary":"consolidated","questions":[{"text":"q","rationale":"r","category":"scope"}],
 "related_repos":["a"],"functionalities":["f"],"integration_points":["i"],"risks":["x"],
 "suggested_learnings":[{"kind":"pattern","title":"t","body":"b"}],"conflict_notes":"resolved X"}
```"#;
    let v = extract_json_block(raw).expect("should find json");
    let parsed: SummaryFindings = serde_json::from_value(v).expect("should parse SummaryFindings");
    assert_eq!(parsed.summary, "consolidated");
    assert_eq!(parsed.questions.len(), 1);
    assert_eq!(parsed.questions[0].text, "q");
    assert_eq!(parsed.suggested_learnings.len(), 1);
    assert_eq!(parsed.conflict_notes, "resolved X");
}

// -----------------------------------------------------------------------
// build_rewrite_prompt
// -----------------------------------------------------------------------

#[test]
fn build_rewrite_prompt_includes_writer_skill_body() {
    let skill = "## Jira Story Writer\nWrite excellent stories.";
    let prompt = build_rewrite_prompt(skill, "/tmp/ctx-rewrite.md");
    assert!(
        prompt.contains(skill),
        "prompt must include the entire writer skill body; got:\n{prompt}"
    );
}

#[test]
fn build_rewrite_prompt_includes_context_path_reference() {
    let ctx_path = "/tmp/otto-product-rewrite-TEST-context.md";
    let prompt = build_rewrite_prompt("skill", ctx_path);
    assert!(
        prompt.contains(ctx_path),
        "prompt must reference the context file path; got:\n{prompt}"
    );
    assert!(
        prompt.to_lowercase().contains("read"),
        "prompt should instruct agent to read the context file; got:\n{prompt}"
    );
}

#[test]
fn build_rewrite_prompt_includes_body_markdown_contract_marker() {
    let prompt = build_rewrite_prompt("skill", "/tmp/ctx.md");
    assert!(
        prompt.contains("body_markdown"),
        "prompt must include 'body_markdown' from the OUTPUT CONTRACT; got:\n{prompt}"
    );
    assert!(
        prompt.contains("EXACTLY ONE"),
        "prompt must include 'EXACTLY ONE' from the OUTPUT CONTRACT; got:\n{prompt}"
    );
    assert!(
        prompt.contains("change_notes"),
        "prompt must include 'change_notes' from the OUTPUT CONTRACT; got:\n{prompt}"
    );
}

// -----------------------------------------------------------------------
// build_tests_prompt
// -----------------------------------------------------------------------

#[test]
fn build_tests_prompt_includes_skill_body_verbatim() {
    let skill = "## Story Test Cases\nDraft readable test cases. Happy path plus validations and error coverage.";
    let prompt = build_tests_prompt(skill, "/tmp/ctx-tests.md");
    assert!(
        prompt.contains(skill),
        "prompt must include the entire tests skill body verbatim; got:\n{prompt}"
    );
}

#[test]
fn build_tests_prompt_skill_body_mentions_happy_and_coverage_guidance() {
    // The bundled skill body mentions "happy" and "validation" / "error" coverage.
    // We embed it verbatim, so the prompt must contain those words.
    let skill =
        "Cover happy path plus meaningful validations and realistic errors. Not over-defensive.";
    let prompt = build_tests_prompt(skill, "/tmp/ctx.md");
    assert!(
        prompt.contains("happy"),
        "prompt must include 'happy' (from skill body); got:\n{prompt}"
    );
    assert!(
        prompt.contains("validation") || prompt.contains("error"),
        "prompt must reference 'validation' or 'error' coverage (from skill body); got:\n{prompt}"
    );
}

#[test]
fn build_tests_prompt_includes_context_path_reference() {
    let ctx_path = "/tmp/otto-product-tests-TEST-context.md";
    let prompt = build_tests_prompt("skill", ctx_path);
    assert!(
        prompt.contains(ctx_path),
        "prompt must reference the context file path; got:\n{prompt}"
    );
    assert!(
        prompt.to_lowercase().contains("read"),
        "prompt should instruct agent to read the context file; got:\n{prompt}"
    );
}

#[test]
fn build_plan_prompt_includes_context_path_reference() {
    let ctx_path = "/tmp/otto-product-plan-TEST-context.md";
    let prompt = build_plan_prompt("skill", ctx_path);
    assert!(
        prompt.contains(ctx_path),
        "prompt must reference the context file path; got:\n{prompt}"
    );
    assert!(
        prompt.to_lowercase().contains("read"),
        "prompt should instruct agent to read the context file; got:\n{prompt}"
    );
}

#[test]
fn build_plan_prompt_includes_plan_contract_marker() {
    let prompt = build_plan_prompt("skill body", "/tmp/ctx.md");
    assert!(
        prompt.contains("plan_markdown"),
        "prompt must include 'plan_markdown' from the OUTPUT CONTRACT; got:\n{prompt}"
    );
    assert!(
        prompt.contains("EXACTLY ONE"),
        "prompt must include 'EXACTLY ONE' from the OUTPUT CONTRACT; got:\n{prompt}"
    );
    assert!(
        prompt.contains("- [ ]"),
        "prompt must instruct emitting '- [ ]' checkboxes; got:\n{prompt}"
    );
}

#[test]
fn build_plan_summarizer_prompt_includes_candidates_and_consolidate_contract() {
    let candidates = vec![
        (
            "claude".to_string(),
            "### Task 1: A\n- [ ] do a".to_string(),
        ),
        ("codex".to_string(), "### Task 1: B\n- [ ] do b".to_string()),
    ];
    let prompt =
        build_plan_summarizer_prompt("synth skill", "Login Story", "/tmp/ctx.md", &candidates);
    // Consolidation instruction + the plan output contract marker.
    assert!(
        prompt.to_lowercase().contains("consolidate"),
        "must instruct consolidation:\n{prompt}"
    );
    assert!(
        prompt.contains("plan_markdown"),
        "must keep the plan_markdown contract:\n{prompt}"
    );
    assert!(
        prompt.contains("EXACTLY ONE"),
        "must include the EXACTLY ONE contract text:\n{prompt}"
    );
    // Each candidate is labelled by provider and fed in verbatim.
    assert!(
        prompt.contains("Plan (claude)"),
        "must label the claude candidate:\n{prompt}"
    );
    assert!(
        prompt.contains("Plan (codex)"),
        "must label the codex candidate:\n{prompt}"
    );
    assert!(
        prompt.contains("- [ ] do a"),
        "must include the first candidate body:\n{prompt}"
    );
    // Story title + context path are referenced.
    assert!(prompt.contains("Login Story"));
    assert!(prompt.contains("/tmp/ctx.md"));
}

#[test]
fn autonomy_directive_instructs_no_questions_and_assumptions() {
    // The non-interactive directive must tell the agent to work unattended,
    // not ask questions, and state assumptions.
    assert!(PLAN_AUTONOMY_DIRECTIVE
        .to_uppercase()
        .contains("UNATTENDED"));
    assert!(PLAN_AUTONOMY_DIRECTIVE
        .to_lowercase()
        .contains("do not ask"));
    assert!(PLAN_AUTONOMY_DIRECTIVE
        .to_lowercase()
        .contains("assumption"));
}

#[test]
fn build_tests_prompt_includes_testcases_contract_marker() {
    let prompt = build_tests_prompt("skill body", "/tmp/ctx.md");
    assert!(
        prompt.contains("testcases"),
        "prompt must include 'testcases' from the OUTPUT CONTRACT; got:\n{prompt}"
    );
    assert!(
        prompt.contains("EXACTLY ONE"),
        "prompt must include 'EXACTLY ONE' from the OUTPUT CONTRACT; got:\n{prompt}"
    );
    assert!(
        prompt.contains("happy|validation|error|edge"),
        "prompt must include category enumeration from the OUTPUT CONTRACT; got:\n{prompt}"
    );
}

// -----------------------------------------------------------------------
// build_improve_narrative_from_tests
// -----------------------------------------------------------------------

fn make_testcase(
    title: &str,
    category: &str,
    status: &str,
    review_note: Option<&str>,
) -> otto_state::ProductTestcase {
    use chrono::Utc;
    otto_state::ProductTestcase {
        id: otto_core::new_id(),
        run_id: otto_core::new_id(),
        story_id: otto_core::new_id(),
        title: title.to_string(),
        category: category.to_string(),
        priority: "medium".to_string(),
        steps_json: "{}".to_string(),
        status: status.to_string(),
        review_note: review_note.map(|s| s.to_string()),
        order_idx: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn make_story(title: &str) -> otto_state::ProductStory {
    use chrono::Utc;
    let uid = otto_core::new_id();
    otto_state::ProductStory {
        id: otto_core::new_id(),
        workspace_id: otto_core::new_id(),
        source_kind: "jira".to_string(),
        account_id: uid.clone(),
        source_key: "PROJ-42".to_string(),
        title: title.to_string(),
        url: "https://jira.example.com/PROJ-42".to_string(),
        issue_type: None,
        stage: "tests_drafted".to_string(),
        cwd: None,
        watch_enabled: false,
        watch_cadence_min: 60,
        watch_cursor: None,
        confluence_tests_page_id: None,
        confluence_tests_url: None,
        tags: String::new(),
        parent_id: None,
        tree_kind: "story".to_string(),
        folder: String::new(),
        created_by: uid,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn narrative_from_tests_contains_story_title() {
    let story = make_story("Add Payment Gateway");
    let cases = vec![
        make_testcase("Happy path checkout", "happy", "approved", None),
        make_testcase(
            "Invalid card number",
            "validation",
            "changes_requested",
            Some("Too generic, add specific error codes"),
        ),
    ];
    let narrative = super::build_improve_narrative_from_tests(&story, &cases);
    assert!(
        narrative.contains("Add Payment Gateway"),
        "narrative must contain story title; got:\n{narrative}"
    );
}

#[test]
fn narrative_from_tests_highlights_changes_requested_case() {
    let story = make_story("Checkout Feature");
    let cases = vec![make_testcase(
        "Invalid card number",
        "validation",
        "changes_requested",
        Some("Too generic, add specific error codes"),
    )];
    let narrative = super::build_improve_narrative_from_tests(&story, &cases);
    assert!(
        narrative.contains("Invalid card number"),
        "narrative must contain the changed case's title; got:\n{narrative}"
    );
    assert!(
        narrative.contains("changes_requested"),
        "narrative must mention changes_requested status; got:\n{narrative}"
    );
    assert!(
        narrative.contains("Too generic, add specific error codes"),
        "narrative must include the review_note text; got:\n{narrative}"
    );
}

#[test]
fn narrative_from_tests_flags_nonempty_review_note() {
    let story = make_story("My Story");
    let cases = vec![make_testcase(
        "Edge case timeout",
        "edge",
        "approved",
        Some("Consider shorter timeout"),
    )];
    let narrative = super::build_improve_narrative_from_tests(&story, &cases);
    assert!(
        narrative.contains("Consider shorter timeout"),
        "narrative must include non-empty review_note even for approved cases; got:\n{narrative}"
    );
}

// -----------------------------------------------------------------------
// build_improve_narrative_from_clarifications
// -----------------------------------------------------------------------

fn make_question_full(
    text: &str,
    status: &str,
    answer: Option<&str>,
) -> otto_state::ProductQuestion {
    use chrono::Utc;
    let dummy = otto_core::new_id();
    otto_state::ProductQuestion {
        id: otto_core::new_id(),
        story_id: dummy.clone(),
        analysis_id: None,
        text: text.to_string(),
        rationale: String::new(),
        category: "scope".to_string(),
        status: status.to_string(),
        answer: answer.map(|a| a.to_string()),
        posted_ref: None,
        created_by: dummy,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn make_note(body: &str, section: Option<&str>) -> otto_state::ProductNote {
    use chrono::Utc;
    otto_state::ProductNote {
        id: otto_core::new_id(),
        story_id: otto_core::new_id(),
        section: section.map(|s| s.to_string()),
        body: body.to_string(),
        author_id: otto_core::new_id(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn narrative_from_clarifications_contains_answered_question() {
    let story = make_story("User Authentication Story");
    let questions = vec![
        make_question_full(
            "What OAuth providers are supported?",
            "answered",
            Some("Google and GitHub only."),
        ),
        make_question_full("Unanswered question", "open", None),
    ];
    let notes = vec![];
    let narrative = super::build_improve_narrative_from_clarifications(&story, &questions, &notes);
    assert!(
        narrative.contains("What OAuth providers are supported?"),
        "narrative must contain answered question text; got:\n{narrative}"
    );
    assert!(
        narrative.contains("Google and GitHub only."),
        "narrative must contain the answer text; got:\n{narrative}"
    );
    // Unanswered questions should not appear as answered
    assert!(
        !narrative.contains("Unanswered question"),
        "narrative must only show answered questions; got:\n{narrative}"
    );
}

#[test]
fn narrative_from_clarifications_contains_note_body() {
    let story = make_story("Payment Story");
    let questions = vec![];
    let notes = vec![make_note(
        "Remember to handle 3DS authentication flow",
        Some("security"),
    )];
    let narrative = super::build_improve_narrative_from_clarifications(&story, &questions, &notes);
    assert!(
        narrative.contains("Remember to handle 3DS authentication flow"),
        "narrative must contain the note body; got:\n{narrative}"
    );
}
