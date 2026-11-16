//! Resolver side labels for an in-progress merge or rebase (split out of
//! `local.rs`): the merged tip's branch name, and a rebase's replayed branch
//! and its onto.

use super::LocalGit;

impl LocalGit {
    /// Best-effort source label for an in-progress merge: the branch/tag
    /// MERGE_HEAD is the tip of (`origin/develop` for a pull), else its sha.
    /// MERGE_MSG's first line ("Merge branch 'x' of <url> into y") overflowed
    /// the resolver's side buttons.
    pub(super) async fn merge_source(&self) -> Option<String> {
        self.ref_label("MERGE_HEAD").await
    }

    /// A rebase's (theirs, ours) labels: the branch being replayed
    /// (`head-name`) and what it is replayed onto. During a rebase HEAD is
    /// detached, so the checked-out branch can't name the ours side.
    pub(super) async fn rebase_labels(&self) -> (Option<String>, Option<String>) {
        let Some(git_dir) = self.git_dir().await else {
            return (None, None);
        };
        for d in ["rebase-merge", "rebase-apply"] {
            let read = |f: &str| tokio::fs::read_to_string(git_dir.join(d).join(f));
            let Ok(onto) = read("onto").await else {
                continue;
            };
            let head = read("head-name").await.ok().map(|h| {
                let h = h.trim();
                h.strip_prefix("refs/heads/").unwrap_or(h).to_string()
            });
            return (head, self.ref_label(onto.trim()).await);
        }
        (None, None)
    }

    /// A short branch/tag name whose tip is `rev` (local branches first, no
    /// symrefs), else the short sha.
    async fn ref_label(&self, rev: &str) -> Option<String> {
        let names = self
            .run(&[
                "for-each-ref",
                "--points-at",
                rev,
                "--format=%(symref)\t%(refname:short)",
                "refs/heads",
                "refs/remotes",
                "refs/tags",
            ])
            .await
            .unwrap_or_default();
        if let Some(name) = names.lines().find_map(|l| l.strip_prefix('\t')) {
            return Some(name.to_string());
        }
        self.run(&["rev-parse", "--short", rev])
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}
