//! Per-turn final reply protocol, independent of a provider's transcript format.
use super::TurnOpts;
use std::path::{Path, PathBuf};

pub(super) struct Reply {
    // Each attempt gets its own directory: a late producer cannot publish into
    // a successor's result, and dropping the turn cleans only its own files.
    dir: tempfile::TempDir,
}
impl Reply {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            dir: tempfile::Builder::new().prefix("otto-turn-").tempdir()?,
        })
    }
    pub fn path(&self) -> PathBuf {
        self.dir.path().join("reply.json")
    }
    pub fn prompt(&self, prompt: &str) -> String {
        let final_path = self.path();
        let temporary_path = self.dir.path().join("reply.json.tmp");
        format!("{prompt}\n\nOTTO FINAL REPLY: After ALL work and child agents are finished, write your complete final answer \
inside a JSON object with a nonempty string field `reply`. Write the COMPLETE JSON to {} first, \
then atomically rename it to {} as your FINAL action. Preserve the answer's formatting inside the JSON string. \
Do not publish the final file while work is pending. A chat reply alone does not complete this turn.",
            temporary_path.display(), final_path.display())
    }
}
pub(super) fn decode(text: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()?
        .get("reply")?
        .as_str()
        .filter(|reply| !reply.trim().is_empty())
        .map(str::to_owned)
}
pub(super) fn options(path: &Path) -> TurnOpts {
    TurnOpts {
        done_file: Some(path.to_owned()),
        done_file_validator: Some(|text| decode(text).is_some()),
        kill_on_stall: true,
        oracle: true,
        ..Default::default()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reply_preserves_formatting_and_ignores_partial_or_empty_results() {
        for invalid in [
            "",
            "{",
            "{}",
            "[]",
            r#"{"reply":null}"#,
            r#"{"reply":"  "}"#,
        ] {
            assert!(decode(invalid).is_none(), "{invalid}");
        }
        assert_eq!(
            decode(r#"{"reply":"**Done**\n\nAll steps complete."}"#).unwrap(),
            "**Done**\n\nAll steps complete."
        );
    }
    #[test]
    fn each_turn_has_a_separate_result_and_cleans_up_only_its_own_directory() {
        let first = Reply::new().unwrap();
        let second = Reply::new().unwrap();
        let first_path = first.path();
        assert_ne!(first_path, second.path());
        std::fs::write(&first_path, r#"{"reply":"first"}"#).unwrap();
        std::fs::write(second.path(), r#"{"reply":"second"}"#).unwrap();
        drop(first);
        assert!(!first_path.exists());
        assert!(second.path().exists());
    }
}
