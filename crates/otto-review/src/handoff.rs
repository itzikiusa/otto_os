//! Getting a long prompt into an agent's terminal reliably. A 100+ KB paste
//! can outlast the PTY write deadline on a busy daemon, and then the agent
//! never starts: on 2026-11-12 two reviews lost their scope assessment and
//! summarizer that way (reviewer prompts run 130–154 KB). A prompt over
//! [`PASTE_INLINE_MAX`] is written to a file and only a short pointer is
//! typed. Shared by reviewers, the scope assessment
//! ([`crate::session::run_agent_session`]) and the summarizer
//! ([`crate::summarizer::Attempt::prompt`]).
//!
//! The file lives in a private directory under the system temp dir. Every
//! review session can read it whatever its provider (claude, codex, agy): the
//! forced `read_only` Seatbelt profile keeps reads global
//! (`otto_sandbox::SandboxPolicy::for_agent`), and each reviewer already reads
//! the review diff from that same temp dir by the path in its prompt. Never
//! the reviewed checkout: that is the user's PR worktree.

use std::path::{Path, PathBuf};

/// Largest prompt typed into a terminal as is.
pub const PASTE_INLINE_MAX: usize = 16 * 1024;

/// What to type for `full`: `full` itself when it is short, otherwise `full`
/// written to `file` and a pointer to it followed by `extra`. `extra` carries
/// whatever must be in the typed text itself (the result hand-off), so it
/// must not look like a result either: no JSON. If the file cannot be
/// written the prompt is pasted whole, as before.
pub fn typed_prompt(full: &str, file: &Path, extra: &str) -> String {
    if full.len() <= PASTE_INLINE_MAX || std::fs::write(file, full).is_err() {
        return full.to_string();
    }
    format!(
        "Your complete instructions for this task are in the file {} ({} KB). \
Read the WHOLE file first (in parts if it is long) and follow it exactly.{extra}",
        file.display(),
        full.len().div_ceil(1024)
    )
}

/// The instructions file of one agent run. It outlives every retry attempt of
/// the run (each attempt rewrites it, so a temp sweep mid-run cannot strand a
/// retry) and is removed with its directory when this value drops: at the end
/// of the run, and on cancellation, when the run's future is dropped.
pub struct PromptFile {
    dir: tempfile::TempDir,
}

impl PromptFile {
    pub fn new() -> std::io::Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("otto-agent-prompt-")
            .tempdir()?;
        Ok(Self { dir })
    }

    pub fn path(&self) -> PathBuf {
        self.dir.path().join("instructions.md")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_prompt_is_typed_and_no_file_is_written() {
        let file = PromptFile::new().unwrap();
        assert_eq!(
            typed_prompt("Review it.", &file.path(), " tail"),
            "Review it."
        );
        assert!(!file.path().exists());
    }

    #[test]
    fn a_long_prompt_goes_by_file_and_the_file_goes_with_the_run() {
        let file = PromptFile::new().unwrap();
        let path = file.path();
        let full = format!("START {} END", "x".repeat(PASTE_INLINE_MAX));
        let typed = typed_prompt(&full, &path, "\n\nThen write to /tmp/out.json.");
        assert!(
            typed.len() < 1024,
            "typed text stays short: {}",
            typed.len()
        );
        assert!(typed.contains(&path.display().to_string()));
        assert!(typed.contains("Read the WHOLE file first"));
        assert!(typed.ends_with("Then write to /tmp/out.json."));
        // The file holds the full text, byte for byte.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), full);
        // The pointer is not a result: the reviewer and scope watchers accept
        // a turn holding a JSON array as the answer.
        assert!(crate::session::parse_findings_array(&typed).is_none());
        // The file is under the temp dir the confined sessions can read.
        assert!(path.starts_with(std::env::temp_dir()));
        drop(file);
        assert!(!path.exists(), "removed when the run ends");
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn an_unwritable_file_falls_back_to_pasting_the_prompt() {
        let full = "y".repeat(PASTE_INLINE_MAX + 1);
        let missing = Path::new("/nonexistent-otto-dir/instructions.md");
        assert_eq!(typed_prompt(&full, missing, ""), full);
    }
}
