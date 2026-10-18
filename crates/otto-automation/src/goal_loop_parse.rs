//! Tolerant extraction of a JSON OBJECT from arbitrary agent output.
//!
//! `otto_orchestrator::parse::find_json_array` handles arrays; the goal-loop
//! roles (definer / evaluator) emit JSON *objects*, often wrapped in ```json
//! fences or surrounding prose. This scans for the first balanced `{...}` block,
//! respecting string literals and escapes, so a `}` inside a string doesn't end
//! the object early.

use otto_core::domain::{GoalLoopDefinition, GoalLoopEvaluation};

/// The first balanced top-level `{...}` substring, or `None` (shared:
/// [`otto_core::text::find_json_object`]).
pub use otto_core::text::find_json_object;

/// The first JSON object in `text` that deserializes as `T` and passes
/// `accept`. A fenced value that is JSON but not `T` does not hide a later
/// object, and neither does a balanced span that fails `accept`.
fn parse_object<T, F>(text: &str, accept: F) -> Option<T>
where
    T: serde::de::DeserializeOwned,
    F: Fn(&serde_json::Value) -> bool,
{
    for value in json_candidates(text) {
        if !accept(&value) {
            continue;
        }
        if let Ok(parsed) = serde_json::from_value(value) {
            return Some(parsed);
        }
    }
    None
}

fn fenced_bodies(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```json") {
        let body = &rest[start + "```json".len()..];
        let Some(end) = body.find("```") else { break };
        out.push(body[..end].trim());
        rest = &body[end + 3..];
    }
    out
}

fn balanced_objects(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(span) = find_json_object(rest) {
        out.push(span);
        let next = span.as_ptr() as usize - rest.as_ptr() as usize + span.len();
        rest = &rest[next..];
    }
    out
}

fn json_candidates(text: &str) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for body in fenced_bodies(text) {
        if let Ok(value) = serde_json::from_str(body) {
            out.push(value);
        }
    }
    for span in balanced_objects(text) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(span) {
            out.push(value);
        }
    }
    out
}

/// Parse the definer's structured goal from its (possibly fenced/prose-wrapped)
/// reply.
pub fn parse_definition(text: &str) -> Option<GoalLoopDefinition> {
    parse_object(text, |value| {
        value.get("title").is_some() || value.get("acceptance_criteria").is_some()
    })
}

/// Parse the evaluator's structured verdict.
pub fn parse_evaluation(text: &str) -> Option<GoalLoopEvaluation> {
    parse_object(text, |value| {
        value.get("verdict").is_some() && value.get("progress_pct").is_some()
    })
}

/// One executor's self-reported result (written to its out-file).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExecutorResult {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub blockers: Vec<String>,
}

/// A completion review: the first object that actually carries `approved`
/// and `findings`. A fenced note or a prose `{not-the-review}` span does not
/// count, and does not hide a later review.
pub fn parse_review(text: &str) -> Option<serde_json::Value> {
    parse_object(text, |value| {
        value
            .get("approved")
            .and_then(serde_json::Value::as_bool)
            .is_some()
            && value
                .get("findings")
                .and_then(serde_json::Value::as_array)
                .is_some()
    })
}

/// Parse an executor's result JSON (tolerant of fences/prose). `None` when no
/// JSON object is present.
pub fn parse_executor_result(text: &str) -> Option<ExecutorResult> {
    // Every field defaults, so a stray `{"note":"x"}` must not count.
    parse_object(text, |value| {
        ["summary", "changed_files", "notes", "blockers"]
            .iter()
            .any(|key| value.get(key).is_some())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_object_from_fences_and_prose() {
        let raw = "Here you go:\n```json\n{\"a\": 1, \"b\": \"x\"}\n```\nthanks";
        assert_eq!(find_json_object(raw), Some("{\"a\": 1, \"b\": \"x\"}"));
    }

    #[test]
    fn ignores_braces_inside_strings() {
        let raw = "{\"text\": \"a } b { c\", \"n\": 2}";
        assert_eq!(find_json_object(raw), Some(raw));
    }

    #[test]
    fn handles_nested_objects() {
        let raw = "prefix {\"x\": {\"y\": 1}} suffix";
        assert_eq!(find_json_object(raw), Some("{\"x\": {\"y\": 1}}"));
    }

    #[test]
    fn none_when_no_object() {
        assert!(find_json_object("no json here").is_none());
        assert!(find_json_object("[1,2,3]").is_none());
    }

    #[test]
    fn a_prose_placeholder_before_the_object_no_longer_hides_it() {
        // The old first-`{` scan returned `{criterion}` and the parse failed.
        let raw = "Checked each {criterion}. Result:\n{\"summary\":\"ok\",\"changed_files\":[]}";
        let r = parse_executor_result(raw).expect("skips the non-JSON span");
        assert_eq!(r.summary, "ok");
    }

    #[test]
    fn parse_evaluation_roundtrip() {
        let raw = "```json\n{\"progress_pct\": 80, \"verdict\": \"continue\", \
                   \"criteria\": [{\"id\":\"c1\",\"met\":true,\"evidence\":\"tests pass\"}], \
                   \"feedback\": \"do x\", \"rationale\": \"y\"}\n```";
        let e = parse_evaluation(raw).expect("parses");
        assert_eq!(e.progress_pct, 80);
        assert_eq!(e.verdict, "continue");
        assert_eq!(e.criteria.len(), 1);
        assert!(e.criteria[0].met);
    }

    #[test]
    fn a_fenced_note_does_not_hide_the_later_verdict() {
        let raw = "```json\n{\"note\":\"example\"}\n```\n{\"progress_pct\":80,\"verdict\":\"continue\",\"criteria\":[],\"feedback\":\"f\",\"rationale\":\"r\"}";
        let eval = parse_evaluation(raw).expect("later object");
        assert_eq!(eval.verdict, "continue");
        assert_eq!(eval.progress_pct, 80);
        let exec = "```json\n{\"note\":\"x\"}\n```\n{\"summary\":\"did the thing\"}";
        let result = parse_executor_result(exec).expect("summary object");
        assert_eq!(result.summary, "did the thing");
    }

    #[test]
    fn a_fenced_note_does_not_hide_the_later_review() {
        let raw = "```json\n{\"note\":\"example\"}\n```\n{\"approved\":false,\"findings\":[\"the test was not run\"]}";
        let review = parse_review(raw).expect("later review");
        assert_eq!(review["approved"], false);
        assert_eq!(review["findings"][0], "the test was not run");
        assert!(parse_review("```json\n{\"approved\":true}\n```").is_none());
    }

    #[test]
    fn parse_executor_result_roundtrip() {
        let raw = "{\"summary\":\"did the thing\",\"changed_files\":[\"a.rs\"],\"blockers\":[]}";
        let r = parse_executor_result(raw).expect("parses");
        assert_eq!(r.summary, "did the thing");
        assert_eq!(r.changed_files, vec!["a.rs"]);
        assert!(r.blockers.is_empty());
    }
}
