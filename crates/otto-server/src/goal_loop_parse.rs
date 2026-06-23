//! Tolerant extraction of a JSON OBJECT from arbitrary agent output.
//!
//! `otto_orchestrator::parse::find_json_array` handles arrays; the goal-loop
//! roles (definer / evaluator) emit JSON *objects*, often wrapped in ```json
//! fences or surrounding prose. This scans for the first balanced `{...}` block,
//! respecting string literals and escapes, so a `}` inside a string doesn't end
//! the object early.

use otto_core::domain::{GoalLoopDefinition, GoalLoopEvaluation};

/// Return the first balanced top-level `{...}` substring, or `None`.
pub fn find_json_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for i in start..bytes.len() {
        let c = bytes[i] as char;
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse the definer's structured goal from its (possibly fenced/prose-wrapped)
/// reply.
pub fn parse_definition(text: &str) -> Option<GoalLoopDefinition> {
    let json = find_json_object(text)?;
    serde_json::from_str(json).ok()
}

/// Parse the evaluator's structured verdict.
pub fn parse_evaluation(text: &str) -> Option<GoalLoopEvaluation> {
    let json = find_json_object(text)?;
    serde_json::from_str(json).ok()
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

/// Parse an executor's result JSON (tolerant of fences/prose). `None` when no
/// JSON object is present.
pub fn parse_executor_result(text: &str) -> Option<ExecutorResult> {
    let json = find_json_object(text)?;
    serde_json::from_str(json).ok()
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
    fn parse_executor_result_roundtrip() {
        let raw = "{\"summary\":\"did the thing\",\"changed_files\":[\"a.rs\"],\"blockers\":[]}";
        let r = parse_executor_result(raw).expect("parses");
        assert_eq!(r.summary, "did the thing");
        assert_eq!(r.changed_files, vec!["a.rs"]);
        assert!(r.blockers.is_empty());
    }
}
