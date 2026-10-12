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
/// The turn's answer: the reply file's `reply`, else (the agent answered only
/// in chat — see [`TurnOpts::transcript_fallback`]) its native final text.
pub(super) fn final_text(text: &str) -> Option<String> {
    decode(text).or_else(|| Some(text.trim().to_owned()).filter(|t| !t.is_empty()))
}
/// Whether a plain chat turn asks for the reply file. Providers without a
/// native end-of-turn signal (agy, custom CLIs) always do — the file is their
/// only completion channel. Claude and codex skip it when permission prompts
/// are on: writing outside the cwd would wait on an approval nobody expects.
pub(super) fn wants_file(provider: &str, skip_permissions: bool) -> bool {
    skip_permissions || !matches!(provider, "claude" | "codex")
}
pub(super) fn options(path: &Path) -> TurnOpts {
    TurnOpts {
        done_file: Some(path.to_owned()),
        done_file_validator: Some(|text| decode(text).is_some()),
        transcript_fallback: true,
        kill_on_stall: true,
        oracle: true,
        ..Default::default()
    }
}
/// A chat turn without the reply file: the native end of the turn (with
/// nothing pending) completes it.
pub(super) fn chat_options() -> TurnOpts {
    TurnOpts {
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
    /// S-2 (sweep 2026-10-11): a chat-only answer (no reply file) is still the
    /// turn's answer; only an empty one fails.
    #[test]
    fn chat_only_answer_is_the_final_text_when_no_reply_file_was_written() {
        assert_eq!(final_text(r#"{"reply":"from file"}"#).unwrap(), "from file");
        assert_eq!(
            final_text("  Here is the answer.\n").unwrap(),
            "Here is the answer."
        );
        assert!(final_text("   ").is_none());
        let opts = options(Path::new("/tmp/reply.json"));
        assert!(
            opts.transcript_fallback,
            "plain chat accepts the native final text"
        );
        assert!(
            opts.done_file_validator.is_some(),
            "a partial file still never ends a turn"
        );
    }
    #[test]
    fn reply_file_is_skipped_only_where_a_native_end_exists_and_prompts_are_on() {
        for p in ["claude", "codex", "agy", "opencode"] {
            assert!(
                wants_file(p, true),
                "{p}: unattended sessions keep the file"
            );
        }
        assert!(!wants_file("claude", false));
        assert!(!wants_file("codex", false));
        assert!(
            wants_file("agy", false),
            "agy has no native end-of-turn signal"
        );
        let chat = chat_options();
        assert!(chat.done_file.is_none() && chat.oracle);
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
