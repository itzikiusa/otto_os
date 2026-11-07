//! Exact committed PR scope: merge-base to reviewed HEAD, never target-tip or worktree dirt.
use crate::{local::GitCmd, LocalGit};
use otto_core::{Error, Result};

pub struct CausalReviewDiff {
    pub base_sha: String,
    pub head_sha: String,
    pub text: String,
}

impl LocalGit {
    /// Local objects only; missing/shallow ancestry fails instead of silently
    /// comparing against the target tip. The 32 MiB safety limit is a hard error,
    /// never an apparently complete truncated review.
    pub async fn causal_review_diff(&self, target: &str) -> Result<CausalReviewDiff> {
        Self::guard_ref(target)?;
        let head = self.rev_parse("HEAD").await?;
        let target = self.rev_parse(target).await?;
        let base = self
            .run_read(&["merge-base", "--end-of-options", &target, &head])
            .await?;
        let base = base.trim();
        if base.len() != 40 || !base.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid(
                "A complete causal PR merge-base is unavailable".into(),
            ));
        }
        let text = self
            .exec_text(
                &GitCmd::diff("diff")
                    .args(["-M", "--end-of-options", base, &head, "--"])
                    .max_stdout(32 * 1024 * 1024),
            )
            .await?;
        Ok(CausalReviewDiff {
            base_sha: base.into(),
            head_sha: head,
            text,
        })
    }
}
