//! Narrative builders for skill self-improvement (Task 4.4): what the PO
//! changed in the generated test cases, and the answered clarifications.

/// Build a self-improvement narrative from test-case review outcomes.
///
/// Describes the story, then each case with its title, category, and status.
/// Cases with `status == "changes_requested"` or a non-empty `review_note`
/// are highlighted so the improvement engine can learn what the PO changed or
/// rejected. Framed as a signal for improving the `story-test-cases` skill.
pub fn build_improve_narrative_from_tests(
    story: &otto_state::ProductStory,
    cases: &[otto_state::ProductTestcase],
) -> String {
    let mut s = String::new();
    s.push_str("# Test-Case Skill Improvement Signal\n\n");
    s.push_str(
        "The Product Owner has reviewed the generated test cases for the following story. \
         Reflect on the story-test-cases skill and how it can be improved based on the PO's \
         feedback (what was changed, rejected, or approved).\n\n",
    );
    s.push_str("## Story\n\n");
    s.push_str("**Title:** ");
    s.push_str(&story.title);
    s.push('\n');
    s.push_str("**Source:** ");
    s.push_str(&story.source_kind);
    s.push_str(" / ");
    s.push_str(&story.source_key);
    s.push('\n');
    s.push_str("**Stage:** ");
    s.push_str(&story.stage);
    s.push_str("\n\n");

    if cases.is_empty() {
        s.push_str("## Test Cases\n\n(none)\n");
        return s;
    }

    s.push_str("## Test Cases\n\n");
    for case in cases {
        let needs_emphasis = case.status == "changes_requested"
            || case
                .review_note
                .as_ref()
                .is_some_and(|n| !n.trim().is_empty());

        if needs_emphasis {
            s.push_str("### [FEEDBACK] ");
        } else {
            s.push_str("### ");
        }
        s.push_str(&case.title);
        s.push('\n');
        s.push_str("- **Category:** ");
        s.push_str(&case.category);
        s.push('\n');
        s.push_str("- **Priority:** ");
        s.push_str(&case.priority);
        s.push('\n');
        s.push_str("- **Status:** ");
        s.push_str(&case.status);
        s.push('\n');

        if let Some(ref note) = case.review_note {
            if !note.trim().is_empty() {
                s.push_str("- **PO Review Note:** ");
                s.push_str(note.trim());
                s.push('\n');
            }
        }

        if case.status == "changes_requested" {
            s.push_str(
                "  *(PO requested changes — consider what generated this case and what \
                 should be improved in the skill to avoid this outcome.)*\n",
            );
        }
        s.push('\n');
    }

    let approved = cases.iter().filter(|c| c.status == "approved").count();
    let changed = cases
        .iter()
        .filter(|c| c.status == "changes_requested")
        .count();
    let total = cases.len();
    s.push_str(&format!(
        "## Summary\n\n{total} case(s) total: {approved} approved, {changed} with changes requested.\n"
    ));

    s
}

/// Build a self-improvement narrative from clarifying questions and notes.
///
/// Includes posted/answered questions and internal notes as signals for
/// improving clarifying-questions / po-story-overview skills.
pub fn build_improve_narrative_from_clarifications(
    story: &otto_state::ProductStory,
    questions: &[otto_state::ProductQuestion],
    notes: &[otto_state::ProductNote],
) -> String {
    let mut s = String::new();
    s.push_str("# Clarifying-Questions Skill Improvement Signal\n\n");
    s.push_str(
        "The following answered questions and internal notes were captured for this story. \
         Use them to improve the clarifying-questions and po-story-overview skills — \
         what kinds of questions were useful, what gaps they uncovered, and what notes \
         the team found important enough to record.\n\n",
    );
    s.push_str("## Story\n\n");
    s.push_str("**Title:** ");
    s.push_str(&story.title);
    s.push('\n');
    s.push_str("**Source:** ");
    s.push_str(&story.source_kind);
    s.push_str(" / ");
    s.push_str(&story.source_key);
    s.push_str("\n\n");

    let answered: Vec<&otto_state::ProductQuestion> = questions
        .iter()
        .filter(|q| q.status == "answered")
        .collect();

    if answered.is_empty() {
        s.push_str("## Answered Questions\n\n(none)\n\n");
    } else {
        s.push_str("## Answered Questions\n\n");
        for q in &answered {
            s.push_str("**Q:** ");
            s.push_str(&q.text);
            s.push('\n');
            if let Some(ref ans) = q.answer {
                if !ans.trim().is_empty() {
                    s.push_str("**A:** ");
                    s.push_str(ans.trim());
                    s.push('\n');
                }
            }
            if !q.category.is_empty() && q.category != "general" {
                s.push_str("**Category:** ");
                s.push_str(&q.category);
                s.push('\n');
            }
            s.push('\n');
        }
    }

    if notes.is_empty() {
        s.push_str("## Internal Notes\n\n(none)\n");
    } else {
        s.push_str("## Internal Notes\n\n");
        for note in notes {
            if let Some(ref section) = note.section {
                s.push_str("**[");
                s.push_str(section);
                s.push_str("]** ");
            }
            s.push_str(&note.body);
            s.push_str("\n\n");
        }
    }

    s
}
