//! Rebase, pull mode, remotes and commit signing (git batch WP5) — routes are merged into `crate::http::router`.
//!
//! Everything here is a repo-level *policy* the user was previously stuck with:
//! how a pull reconciles (merge / rebase / ff-only, defaulting to the repo's own
//! git config instead of a silent `--no-rebase`), which remotes exist, and
//! whether a commit is signed.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_core::api::RepoStatusResp;
use otto_core::auth::AuthUser;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};

use crate::http::{repo_ctx, repo_lock, ApiResult, GitCtx};
use crate::local::{validate_remote_url, LocalGit, PullOutcome, SpawnClass};

/// How `git pull` reconciles diverged history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullMode {
    Merge,
    Rebase,
    FfOnly,
}

impl PullMode {
    /// The one flag this mode adds to `git pull` — an enum, so no caller string
    /// ever reaches the argv.
    fn flag(self) -> &'static str {
        match self {
            Self::Merge => "--no-rebase",
            Self::Rebase => "--rebase",
            Self::FfOnly => "--ff-only",
        }
    }
}

/// What a rebase onto `onto` would replay, for the confirm dialog.
#[derive(Debug, Clone, Serialize)]
pub struct RebasePreview {
    pub commits: u64,
    pub onto_sha: String,
}

/// One `git remote`, with any `user:password@` userinfo stripped from the URLs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteInfo {
    pub name: String,
    pub fetch_url: String,
    pub push_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteOp {
    Add,
    SetUrl,
    Remove,
}

/// The repo's commit-signing configuration, so the WIP composer can pre-set its
/// toggle instead of guessing.
#[derive(Debug, Clone, Serialize)]
pub struct CommitConfig {
    pub gpgsign: bool,
    /// `openpgp` | `ssh` | `x509`, or None when unset/unrecognised.
    pub format: Option<String>,
    pub signing_key: Option<String>,
}

impl LocalGit {
    /// Rebase the current branch onto `onto`. A CONFLICTING rebase is a normal
    /// outcome (like [`LocalGit::cherry_pick`]): git leaves `rebase-merge` in
    /// place, `status()` reports `op_in_progress:"rebase"` with the conflicted
    /// paths, and the UI routes into the resolver. Starting one ON TOP of an
    /// unfinished operation is refused — git would too, with a worse message.
    pub async fn rebase(&self, onto: &str, auto_stash: bool) -> Result<()> {
        if let Some(op) = self.op_in_progress().await {
            return Err(Error::Conflict(format!(
                "a {} is already in progress — resolve its conflicts or abort it first",
                op.replace('_', "-")
            )));
        }
        Self::guard_ref(onto)?;
        let mut args = vec!["rebase"];
        if auto_stash {
            args.push("--autostash");
        }
        args.push("--end-of-options");
        args.push(onto);
        self.op_conflict_as_result(&args).await
    }

    /// How many commits a rebase onto `onto` would replay, and what it would
    /// land on. Read-only (`rev-list --count`), so the confirm can be honest
    /// before anything is moved.
    pub async fn rebase_preview(&self, onto: &str) -> Result<RebasePreview> {
        Self::guard_ref(onto)?;
        let range = format!("{onto}..HEAD");
        let count = self
            .run_read(&["rev-list", "--count", "--end-of-options", &range, "--"])
            .await?;
        let onto_sha = self
            .run_read(&["rev-parse", "--verify", "--end-of-options", onto])
            .await?
            .trim()
            .to_string();
        Ok(RebasePreview {
            commits: count.trim().parse().unwrap_or(0),
            onto_sha,
        })
    }

    /// Read a single `git config --get <key>`; None when unset (git exits 1) or
    /// empty.
    async fn config_get(&self, key: &str) -> Option<String> {
        match self
            .run_raw_class(&["config", "--get", key], &[], SpawnClass::LocalRead)
            .await
        {
            Ok((true, out, _, _)) => {
                let v = out.trim().to_string();
                (!v.is_empty()).then_some(v)
            }
            _ => None,
        }
    }

    /// The pull mode the repo's OWN config asks for (`pull.rebase`, then
    /// `pull.ff`), so Otto's pull button stops silently overriding it. Read on
    /// every call — the config can change under us and two local `git config`
    /// spawns are cheaper than a stale policy.
    pub async fn pull_mode_default(&self) -> PullMode {
        if let Some(v) = self.config_get("pull.rebase").await {
            if matches!(v.as_str(), "true" | "merges" | "interactive") {
                return PullMode::Rebase;
            }
        }
        if self.config_get("pull.ff").await.as_deref() == Some("only") {
            return PullMode::FfOnly;
        }
        PullMode::Merge
    }

    /// [`LocalGit::pull_outcome`] with an explicit mode. A conflicting
    /// reconcile is an OUTCOME, not a failure — for `--rebase` too, which is why
    /// the conflict test asks `op_in_progress()` (a stopped rebase leaves no
    /// `MERGE_HEAD`) instead of `is_merging()`.
    pub async fn pull_outcome_mode(
        &self,
        token: Option<String>,
        mode: PullMode,
    ) -> Result<PullOutcome> {
        let (ok, stdout, stderr, code) = self.run_remote_raw(&["pull", mode.flag()], token).await?;
        let mut output = crate::local::strip_noise(&stdout);
        let err = crate::local::strip_noise(&stderr);
        if !err.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&err);
        }
        if ok {
            return Ok(PullOutcome {
                output,
                conflicted_files: Vec::new(),
            });
        }
        let conflicted = self.conflicted_paths().await.unwrap_or_default();
        let combined = format!("{stdout}\n{stderr}");
        let is_conflict = !conflicted.is_empty()
            && (combined.contains("CONFLICT")
                || combined.contains("Automatic merge failed")
                || self.op_in_progress().await.is_some());
        if is_conflict {
            return Ok(PullOutcome {
                output,
                conflicted_files: conflicted,
            });
        }
        // `--ff-only` on a diverged branch ("Not possible to fast-forward") is a
        // 409 via `local_refusal`, not a 502: nothing is wrong with the remote.
        Err(crate::local::upstream_err(&stderr, &stdout, code))
    }

    /// [`LocalGit::pull_autostash`] with an explicit mode: stash → pull → pop
    /// around a dirty tree. A pull that stops with CONFLICTS leaves the changes
    /// stashed (popping onto a half-merged tree would bury them); a refused pull
    /// pops them straight back so the tree is exactly as it was.
    pub async fn pull_autostash_mode(
        &self,
        token: Option<String>,
        mode: PullMode,
    ) -> Result<(PullOutcome, Option<String>)> {
        let dirty = !self
            .run(&["status", "--porcelain"])
            .await?
            .trim()
            .is_empty();
        if !dirty {
            return Ok((self.pull_outcome_mode(token, mode).await?, None));
        }
        self.stash_save().await?;
        match self.pull_outcome_mode(token, mode).await {
            Ok(out) if out.conflicted_files.is_empty() => {
                let note = self.pop_after_merge().await;
                Ok((out, note))
            }
            Ok(out) => Ok((
                out,
                Some(
                    "The pull stopped with conflicts. Your uncommitted changes stay stashed — \
                     resolve the conflicts and commit, then run `git stash pop` to restore them."
                        .into(),
                ),
            )),
            Err(e) => {
                let _ = self.stash_pop().await;
                Err(e)
            }
        }
    }

    /// `git remote -v` → one entry per remote, userinfo stripped.
    pub async fn remotes(&self) -> Result<Vec<RemoteInfo>> {
        Ok(parse_remotes(&self.run_read(&["remote", "-v"]).await?))
    }

    /// Add / re-point / remove a remote. Both positional values are validated
    /// before the spawn (neither can begin with `-`), so `git remote` can never
    /// read one as an option.
    pub async fn remote_op(
        &self,
        op: RemoteOp,
        name: &str,
        url: Option<&str>,
    ) -> Result<Vec<RemoteInfo>> {
        validate_remote_name(name)?;
        Self::guard_ref(name)?;
        match op {
            RemoteOp::Add | RemoteOp::SetUrl => {
                let url = url
                    .map(str::trim)
                    .filter(|u| !u.is_empty())
                    .ok_or_else(|| {
                        Error::Invalid("a url is required to add or re-point a remote".into())
                    })?;
                validate_remote_url(url)?;
                let sub = if op == RemoteOp::Add {
                    "add"
                } else {
                    "set-url"
                };
                self.run(&["remote", sub, "--end-of-options", name, url])
                    .await?;
            }
            RemoteOp::Remove => {
                self.run(&["remote", "remove", "--end-of-options", name])
                    .await?;
            }
        }
        self.remotes().await
    }

    /// [`LocalGit::commit`] plus an explicit signing choice: `Some(true)` → `-S`,
    /// `Some(false)` → `--no-gpg-sign`, `None` → whatever `commit.gpgsign` says.
    /// Amending with an EMPTY message keeps the previous message.
    pub async fn commit_signed(
        &self,
        message: &str,
        amend: bool,
        sign: Option<bool>,
    ) -> Result<String> {
        let mut args: Vec<&str> = Vec::new();
        if message.trim().is_empty() {
            if !amend {
                return Err(Error::Invalid("empty commit message".into()));
            }
            args.extend(["commit", "--amend", "--no-edit"]);
        } else {
            args.extend(["commit", "-m", message]);
            if amend {
                args.push("--amend");
            }
        }
        match sign {
            Some(true) => args.push("-S"),
            Some(false) => args.push("--no-gpg-sign"),
            None => {}
        }
        let (ok, out, err, code) = self.run_raw_retry_lock(&args).await?;
        if !ok {
            return Err(crate::local::upstream_err(&err, &out, code));
        }
        let sha = self.run(&["rev-parse", "HEAD"]).await?;
        Ok(sha.trim().to_string())
    }

    /// The repo's signing config (`commit.gpgsign`, `gpg.format`,
    /// `user.signingkey`).
    pub async fn commit_config(&self) -> CommitConfig {
        let gpgsign = self
            .config_get("commit.gpgsign")
            .await
            .is_some_and(|v| v.eq_ignore_ascii_case("true"));
        let format = self
            .config_get("gpg.format")
            .await
            .filter(|f| matches!(f.as_str(), "openpgp" | "ssh" | "x509"));
        let signing_key = self.config_get("user.signingkey").await;
        CommitConfig {
            gpgsign,
            format,
            signing_key,
        }
    }
}

/// `git remote -v` lines: `<name>\t<url> (fetch|push)`, two per remote.
fn parse_remotes(out: &str) -> Vec<RemoteInfo> {
    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, RemoteInfo> = HashMap::new();
    for line in out.lines() {
        let Some((name, rest)) = line.trim_end().split_once('\t') else {
            continue;
        };
        let (url, kind) = match rest.rsplit_once(' ') {
            Some((u, k)) => (u, k.trim_matches(['(', ')'])),
            None => (rest, "fetch"),
        };
        let url = crate::local::strip_url_userinfo(url.trim());
        let entry = map.entry(name.to_string()).or_insert_with(|| {
            order.push(name.to_string());
            RemoteInfo {
                name: name.to_string(),
                fetch_url: String::new(),
                push_url: String::new(),
            }
        });
        if kind == "push" {
            entry.push_url = url;
        } else {
            entry.fetch_url = url;
        }
    }
    order.into_iter().filter_map(|n| map.remove(&n)).collect()
}

/// A remote name is a ref component (`refs/remotes/<name>/…`): it must not begin
/// with `-` (argv smuggling) and must stay inside git's own legal set.
fn validate_remote_name(name: &str) -> Result<()> {
    let ok = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'));
    if !ok {
        return Err(Error::Invalid(format!(
            "invalid remote name '{name}' — use letters, digits, then any of . _ / -"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RebaseReq {
    onto: String,
    #[serde(default)]
    auto_stash: bool,
}

async fn repo_rebase<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<RebaseReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.rebase(&req.onto, req.auto_stash).await?;
    // A conflicting rebase is a normal 200 — the status carries
    // `op_in_progress:"rebase"` + the unmerged paths for the resolver.
    Ok(Json(git.status().await?))
}

#[derive(Deserialize)]
struct RebasePreviewQuery {
    onto: String,
}

async fn repo_rebase_preview<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<RebasePreviewQuery>,
) -> ApiResult<Json<RebasePreview>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.rebase_preview(&q.onto).await?))
}

async fn repo_pull_mode<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(
        serde_json::json!({ "mode": git.pull_mode_default().await }),
    ))
}

async fn repo_remotes<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Vec<RemoteInfo>>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.remotes().await?))
}

#[derive(Deserialize)]
struct RemoteOpReq {
    op: RemoteOp,
    name: String,
    #[serde(default)]
    url: Option<String>,
}

async fn repo_remote_op<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<RemoteOpReq>,
) -> ApiResult<Json<Vec<RemoteInfo>>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(
        git.remote_op(req.op, &req.name, req.url.as_deref()).await?,
    ))
}

async fn repo_commit_config<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<CommitConfig>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.commit_config().await))
}

/// Routes owned by this module (merged into `crate::http::router`).
pub fn router<S: GitCtx>() -> Router<S> {
    Router::new()
        .route("/repos/{id}/rebase", post(repo_rebase::<S>))
        .route("/repos/{id}/rebase-preview", get(repo_rebase_preview::<S>))
        .route("/repos/{id}/pull-mode", get(repo_pull_mode::<S>))
        .route(
            "/repos/{id}/remotes",
            get(repo_remotes::<S>).post(repo_remote_op::<S>),
        )
        .route("/repos/{id}/commit-config", get(repo_commit_config::<S>))
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::api::LocalMergeStrategy;
    use std::path::{Path as FsPath, PathBuf};

    /// Run `git` synchronously for fixture setup.
    fn sh_git(dir: &FsPath, args: &[&str]) {
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn write(dir: &FsPath, rel: &str, content: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    fn init_repo(dir: &FsPath) {
        std::fs::create_dir_all(dir).unwrap();
        sh_git(dir, &["init", "-b", "main"]);
        sh_git(dir, &["config", "user.email", "otto@test.local"]);
        sh_git(dir, &["config", "user.name", "Otto Test"]);
        sh_git(dir, &["config", "commit.gpgsign", "false"]);
    }

    /// One repo with a single commit.
    fn repo() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("repo");
        init_repo(&dir);
        write(&dir, "a.txt", "one\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "init"]);
        (tmp, dir)
    }

    /// A bare "origin" + a clone of it, both with the test identity.
    fn clone_with_origin() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin.git");
        std::fs::create_dir(&origin).unwrap();
        sh_git(&origin, &["init", "--bare", "-b", "main"]);

        let seed = tmp.path().join("seed");
        init_repo(&seed);
        write(&seed, "shared.txt", "line1\nline2\nline3\n");
        sh_git(&seed, &["add", "."]);
        sh_git(&seed, &["commit", "-m", "init"]);
        sh_git(
            &seed,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        sh_git(&seed, &["push", "-u", "origin", "main"]);

        let dir = tmp.path().join("work");
        sh_git(
            tmp.path(),
            &["clone", origin.to_str().unwrap(), dir.to_str().unwrap()],
        );
        sh_git(&dir, &["config", "user.email", "otto@test.local"]);
        sh_git(&dir, &["config", "user.name", "Otto Test"]);
        sh_git(&dir, &["config", "commit.gpgsign", "false"]);
        (tmp, dir, seed)
    }

    /// A rebase that conflicts is an OUTCOME: the status carries
    /// `op_in_progress:"rebase"` with the unmerged paths, and the existing
    /// merge-abort lifecycle clears it.
    #[tokio::test]
    async fn rebase_conflict_lands_as_op_in_progress() {
        let (_tmp, dir) = repo();
        write(&dir, "conf.txt", "base\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "base"]);
        sh_git(&dir, &["checkout", "-b", "topic"]);
        write(&dir, "conf.txt", "topic\n");
        sh_git(&dir, &["commit", "-am", "topic edit"]);
        sh_git(&dir, &["checkout", "main"]);
        write(&dir, "conf.txt", "main\n");
        sh_git(&dir, &["commit", "-am", "main edit"]);
        sh_git(&dir, &["checkout", "topic"]);

        let git = LocalGit::new(&dir);
        git.rebase("main", false)
            .await
            .expect("a conflicting rebase is an outcome, not an error");
        let st = git.status().await.unwrap();
        assert_eq!(st.op_in_progress.as_deref(), Some("rebase"));
        assert!(st
            .changes
            .iter()
            .any(|c| c.path == "conf.txt" && c.kind == "conflicted"));

        git.merge_abort().await.unwrap();
        assert!(git.op_in_progress().await.is_none(), "abort clears it");
    }

    /// Starting a rebase on top of an unfinished merge is a 409, not a worse
    /// git message halfway through.
    #[tokio::test]
    async fn rebase_refused_during_merge() {
        let (_tmp, dir) = repo();
        sh_git(&dir, &["checkout", "-b", "topic"]);
        write(&dir, "a.txt", "topic\n");
        sh_git(&dir, &["commit", "-am", "topic edit"]);
        sh_git(&dir, &["checkout", "main"]);
        write(&dir, "a.txt", "main\n");
        sh_git(&dir, &["commit", "-am", "main edit"]);
        // Leave a conflicted merge in progress.
        let git = LocalGit::new(&dir);
        let _ = git
            .merge_branch("topic", "main", LocalMergeStrategy::MergeCommit, false)
            .await;
        assert_eq!(git.op_in_progress().await, Some("merge"));

        let err = git.rebase("topic", false).await.expect_err("refused");
        assert!(matches!(err, Error::Conflict(_)), "got {err:?}");
        assert!(
            matches!(&err, Error::Conflict(m) if m.contains("merge is already in progress")),
            "message names the op: {err:?}"
        );
    }

    /// The pull mode comes from the repo's own config, not from Otto.
    #[tokio::test]
    async fn pull_mode_default_reads_config() {
        let (_tmp, dir) = repo();
        let git = LocalGit::new(&dir);
        assert_eq!(git.pull_mode_default().await, PullMode::Merge);

        sh_git(&dir, &["config", "pull.ff", "only"]);
        assert_eq!(git.pull_mode_default().await, PullMode::FfOnly);

        // pull.rebase wins over pull.ff (git applies it first too).
        sh_git(&dir, &["config", "pull.rebase", "true"]);
        assert_eq!(git.pull_mode_default().await, PullMode::Rebase);
        sh_git(&dir, &["config", "pull.rebase", "false"]);
        assert_eq!(git.pull_mode_default().await, PullMode::FfOnly);
    }

    /// `--ff-only` on a DIVERGED branch is the caller's choice of mode failing,
    /// not a provider outage: 409 with git's own line, and nothing started.
    #[tokio::test]
    async fn pull_ff_only_diverged_is_409() {
        let (_tmp, dir, seed) = clone_with_origin();
        write(&seed, "shared.txt", "line1\nUPSTREAM\nline3\n");
        sh_git(&seed, &["commit", "-am", "upstream work"]);
        sh_git(&seed, &["push", "origin", "main"]);
        write(&dir, "shared.txt", "line1\nLOCAL\nline3\n");
        sh_git(&dir, &["commit", "-am", "local work"]);

        let git = LocalGit::new(&dir);
        let err = git
            .pull_outcome_mode(None, PullMode::FfOnly)
            .await
            .expect_err("ff-only cannot reconcile a diverged branch");
        assert!(matches!(err, Error::Conflict(_)), "got {err:?}");
        assert!(git.op_in_progress().await.is_none(), "nothing was started");
    }

    /// `--rebase` replays the local commit on top of the fetched tip: no merge
    /// commit, and the local work is last.
    #[tokio::test]
    async fn pull_rebase_mode_replays_local_commit() {
        let (_tmp, dir, seed) = clone_with_origin();
        write(&seed, "upstream.txt", "from upstream\n");
        sh_git(&seed, &["add", "."]);
        sh_git(&seed, &["commit", "-m", "upstream work"]);
        sh_git(&seed, &["push", "origin", "main"]);
        write(&dir, "local.txt", "from local\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "local work"]);

        let git = LocalGit::new(&dir);
        let out = git
            .pull_outcome_mode(None, PullMode::Rebase)
            .await
            .expect("rebase pull");
        assert!(out.conflicted_files.is_empty());

        let log = git
            .run(&["log", "--pretty=%s%x1f%P", "-n", "3"])
            .await
            .unwrap();
        let rows: Vec<&str> = log.lines().collect();
        assert!(
            rows[0].starts_with("local work"),
            "local work ends on top: {log}"
        );
        assert!(
            rows.iter().all(|r| r
                .split('\u{1f}')
                .nth(1)
                .unwrap_or("")
                .split_whitespace()
                .count()
                <= 1),
            "no merge commit was created: {log}"
        );
    }

    /// Neither an option-like URL, an unsupported scheme, nor a `-`-leading name
    /// reaches `git remote`.
    #[tokio::test]
    async fn remote_add_rejects_bad_url_and_name() {
        let (_tmp, dir) = repo();
        let git = LocalGit::new(&dir);

        for bad in [
            "--upload-pack=touch /tmp/pwn",
            "file:///etc/passwd",
            "/etc/passwd",
            "https://exa mple.com/x.git",
            "",
        ] {
            let err = git
                .remote_op(RemoteOp::Add, "up", Some(bad))
                .await
                .expect_err("an unusable url must never reach git remote");
            assert!(matches!(err, Error::Invalid(_)), "{bad}: got {err:?}");
        }
        for bad in ["--config", "-up", "up stream", "up;rm"] {
            let err = git
                .remote_op(RemoteOp::Add, bad, Some("https://example.com/x.git"))
                .await
                .expect_err("bad name");
            assert!(matches!(err, Error::Invalid(_)), "{bad}: got {err:?}");
        }
        // Add without a url is a 400, not a half-run `git remote add`.
        assert!(matches!(
            git.remote_op(RemoteOp::Add, "up", None).await,
            Err(Error::Invalid(_))
        ));
        assert!(git.remotes().await.unwrap().is_empty(), "nothing was added");

        // The happy path still works, and scp-like URLs are accepted.
        let remotes = git
            .remote_op(RemoteOp::Add, "up", Some("git@example.com:otto/x.git"))
            .await
            .unwrap();
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].fetch_url, "git@example.com:otto/x.git");
    }

    /// `remote -v` is parsed into one row per remote, and a credentialed URL is
    /// never echoed back to the client.
    #[tokio::test]
    async fn remotes_lists_and_strips_userinfo() {
        let (_tmp, dir) = repo();
        sh_git(
            &dir,
            &[
                "remote",
                "add",
                "origin",
                "https://bob:s3cr3t@example.com/otto/x.git",
            ],
        );
        let git = LocalGit::new(&dir);
        let remotes = git.remotes().await.unwrap();
        assert_eq!(
            remotes.len(),
            1,
            "one row per remote, not one per direction"
        );
        assert_eq!(remotes[0].name, "origin");
        assert_eq!(remotes[0].fetch_url, "https://example.com/otto/x.git");
        assert_eq!(remotes[0].push_url, "https://example.com/otto/x.git");

        // set-url re-points it; remove drops it.
        let remotes = git
            .remote_op(
                RemoteOp::SetUrl,
                "origin",
                Some("https://example.com/y.git"),
            )
            .await
            .unwrap();
        assert_eq!(remotes[0].fetch_url, "https://example.com/y.git");
        assert!(git
            .remote_op(RemoteOp::Remove, "origin", None)
            .await
            .unwrap()
            .is_empty());
    }

    /// `sign:false` must pass `--no-gpg-sign` even when the repo config says to
    /// sign — otherwise the toggle is decorative.
    #[tokio::test]
    async fn commit_sign_false_passes_no_gpg_sign() {
        let (_tmp, dir) = repo();
        // A repo that WANTS to sign, with a key that cannot work here: the
        // commit must still land because the flag turned signing off.
        sh_git(&dir, &["config", "commit.gpgsign", "true"]);
        sh_git(&dir, &["config", "gpg.format", "ssh"]);
        sh_git(&dir, &["config", "user.signingkey", "/nonexistent/key.pub"]);

        let git = LocalGit::new(&dir);
        write(&dir, "b.txt", "two\n");
        sh_git(&dir, &["add", "."]);
        let sha = git
            .commit_signed("feat: unsigned on purpose", false, Some(false))
            .await
            .expect("--no-gpg-sign overrides commit.gpgsign");
        assert_eq!(sha.len(), 40);
        let verify = git.run(&["log", "-1", "--format=%G?"]).await.unwrap();
        assert_eq!(verify.trim(), "N", "the commit carries no signature");

        // Leaving it to the config, with that same broken key, fails loudly.
        write(&dir, "c.txt", "three\n");
        sh_git(&dir, &["add", "."]);
        assert!(
            git.commit_signed("feat: config decides", false, None)
                .await
                .is_err(),
            "sign:None must not silently disable signing"
        );
    }

    /// `commit-config` reports what the repo asks for, and unset keys are None
    /// (not empty strings the UI would render as a key).
    #[tokio::test]
    async fn commit_config_reads_keys() {
        let (_tmp, dir) = repo();
        let git = LocalGit::new(&dir);
        let cfg = git.commit_config().await;
        assert!(!cfg.gpgsign);
        assert_eq!(cfg.format, None);
        assert_eq!(cfg.signing_key, None);

        sh_git(&dir, &["config", "commit.gpgsign", "true"]);
        sh_git(&dir, &["config", "gpg.format", "ssh"]);
        sh_git(
            &dir,
            &["config", "user.signingkey", "~/.ssh/id_ed25519.pub"],
        );
        let cfg = git.commit_config().await;
        assert!(cfg.gpgsign);
        assert_eq!(cfg.format.as_deref(), Some("ssh"));
        assert_eq!(cfg.signing_key.as_deref(), Some("~/.ssh/id_ed25519.pub"));

        // An unrecognised format is dropped rather than passed through.
        sh_git(&dir, &["config", "gpg.format", "smime"]);
        assert_eq!(git.commit_config().await.format, None);
    }

    /// `rebase-preview` counts what WOULD be replayed without touching the tree.
    #[tokio::test]
    async fn rebase_preview_counts_without_mutating() {
        let (_tmp, dir) = repo();
        sh_git(&dir, &["checkout", "-b", "topic"]);
        for i in 0..3 {
            write(&dir, &format!("t{i}.txt"), "x\n");
            sh_git(&dir, &["add", "."]);
            sh_git(&dir, &["commit", "-m", &format!("topic {i}")]);
        }
        let git = LocalGit::new(&dir);
        let preview = git.rebase_preview("main").await.unwrap();
        assert_eq!(preview.commits, 3);
        assert_eq!(preview.onto_sha.len(), 40);
        assert_eq!(
            git.current_branch().await.unwrap(),
            "topic",
            "a preview moves nothing"
        );
        assert!(matches!(
            git.rebase_preview("--output=/tmp/pwn").await,
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn parse_remotes_pairs_fetch_and_push() {
        let out = "origin\thttps://example.com/a.git (fetch)\n\
                   origin\thttps://example.com/a-push.git (push)\n\
                   up\tgit@example.com:o/b.git (fetch)\n\
                   up\tgit@example.com:o/b.git (push)\n";
        let remotes = parse_remotes(out);
        assert_eq!(remotes.len(), 2);
        assert_eq!(remotes[0].name, "origin");
        assert_eq!(remotes[0].fetch_url, "https://example.com/a.git");
        assert_eq!(remotes[0].push_url, "https://example.com/a-push.git");
        assert_eq!(remotes[1].fetch_url, "git@example.com:o/b.git");
    }
}
