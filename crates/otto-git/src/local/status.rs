//! Working-tree status (`git status --porcelain=v2`) with in-flight coalescing
//! and opt-in large-index acceleration. Split out of `local.rs` (LOC ratchet).

use std::sync::{Arc, OnceLock};
use tokio::process::Command;

use otto_core::api::RepoStatusResp;
use otto_core::Result;

use super::{off_runtime, GitCmd, LocalGit, STATUS_IGNORE_SUBMODULES};
use crate::diff_cache::SingleFlight;

fn status_flights() -> &'static SingleFlight<Arc<RepoStatusResp>> {
    static F: OnceLock<SingleFlight<Arc<RepoStatusResp>>> = OnceLock::new();
    F.get_or_init(SingleFlight::new)
}

impl LocalGit {
    /// Status for a RESPONSE: untracked rows capped at
    /// [`crate::parse::UNTRACKED_ROW_CAP`] (with `untracked_total` /
    /// `untracked_truncated` set past it). Internal callers that act on
    /// untracked paths use [`Self::status_full`].
    pub async fn status(&self) -> Result<RepoStatusResp> {
        self.status_with_cap(crate::parse::UNTRACKED_ROW_CAP).await
    }

    /// Every untracked row, uncapped — for callers that classify specific
    /// paths (discard must still find the 6,000th untracked file).
    pub async fn status_full(&self) -> Result<RepoStatusResp> {
        self.status_with_cap(usize::MAX).await
    }

    async fn status_with_cap(&self, cap: usize) -> Result<RepoStatusResp> {
        let key = format!(
            "{}\0{}\0{cap}",
            self.git_bin.display(),
            self.path().display()
        );
        let git = self.clone();
        let shared = status_flights()
            .run(&key, move || async move {
                git.status_with_cap_inner(cap).await.map(Arc::new)
            })
            .await?;
        Ok(Arc::unwrap_or_clone(shared))
    }

    async fn status_with_cap_inner(&self, cap: usize) -> Result<RepoStatusResp> {
        // `--untracked-files=all` lists every untracked FILE individually instead
        // of collapsing an entirely-new directory (e.g. `.claude/skills/` with
        // 80+ files) into a single entry — so the Changes view can show/stage
        // them per-file. Gitignored paths are still excluded.
        //
        // `-z`: without it git C-quotes any name with a non-ASCII byte, `"` or
        // `\` (`"caf\303\251.txt"`), and that quoted form is what came back
        // to stage/discard — `add` then failed "did not match", and `clean`
        // exited 0 having deleted NOTHING while the UI said "Discarded".
        let mut args: Vec<&str> = Vec::new();
        if self.large_index_accel().await {
            args.extend([
                "-c",
                "core.untrackedCache=true",
                "-c",
                "core.fsmonitor=true",
            ]);
        }
        args.extend([
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
            STATUS_IGNORE_SUBMODULES,
        ]);
        let out = self.exec_text(&GitCmd::read(&args)).await?;
        // 100k rows is tens of ms of parsing — off the async workers.
        let mut st = off_runtime(out.len(), move || {
            crate::parse::parse_status_capped(&out, cap)
        })
        .await?;
        st.op_in_progress = self.op_in_progress().await.map(str::to_string);
        Ok(st)
    }

    /// Opt-in (`OTTO_GIT_FSMONITOR=1`) status acceleration for big repos:
    /// `-c core.untrackedCache=true -c core.fsmonitor=true` on the status
    /// argv when the index is over ~6 MB (≈50k entries). Never written to the
    /// repo config (see the `core.fsmonitor` guard on `.git/config` writes).
    ///
    /// Not on by default: both caches live in the INDEX, and Otto's status
    /// runs with `GIT_OPTIONAL_LOCKS=0` (it must never take `index.lock` from
    /// under an agent's commit), so it can't persist the fsmonitor token or
    /// the untracked cache — the gain only appears when the user's own git
    /// writes the index with them. Skipped when the repo config already
    /// names an fsmonitor (a watchman hook must not be overridden) and on a
    /// git without the built-in daemon (< 2.37 would run `true` as a hook).
    async fn large_index_accel(&self) -> bool {
        if std::env::var("OTTO_GIT_FSMONITOR").ok().as_deref() != Some("1") {
            return false;
        }
        let Some(gd) = self.git_dir().await else {
            return false;
        };
        let big = tokio::fs::metadata(gd.join("index"))
            .await
            .is_ok_and(|m| m.len() > 6 << 20);
        if !big {
            return false;
        }
        let cfg = tokio::fs::read_to_string(gd.join("config"))
            .await
            .unwrap_or_default();
        if cfg.to_ascii_lowercase().contains("fsmonitor") {
            return false;
        }
        static BUILTIN: tokio::sync::OnceCell<bool> = tokio::sync::OnceCell::const_new();
        let bin = self.git_bin.clone();
        *BUILTIN
            .get_or_init(|| async move {
                let out = Command::new(bin).arg("--version").output().await;
                out.ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                    .and_then(|v| {
                        let ver = v.split_whitespace().nth(2)?.to_string();
                        let mut it = ver.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
                        Some((it.next()?, it.next()?))
                    })
                    .is_some_and(|(maj, min)| (maj, min) >= (2, 37))
            })
            .await
    }
}
