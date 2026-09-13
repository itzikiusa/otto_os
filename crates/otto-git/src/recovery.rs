//! Recovery history, reviewable interactive rebases and resumable bisects.

use crate::{
    http::{repo_ctx, repo_lock, ApiResult, GitCtx},
    local::LocalGit,
};
use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Extension, Json, Router,
};
use otto_core::{api::RepoStatusResp, auth::AuthUser, domain::WorkspaceRole, Error, Id, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Write;

#[derive(Debug, Serialize)]
pub struct RecoveryEntry {
    pub sha: String,
    pub selector: String,
    pub subject: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RebaseAction {
    Pick,
    Squash,
    Edit,
}
impl RebaseAction {
    fn word(self) -> &'static str {
        match self {
            Self::Pick => "pick",
            Self::Squash => "squash",
            Self::Edit => "edit",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanCommit {
    pub sha: String,
    pub subject: String,
    pub action: RebaseAction,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractivePlan {
    pub head_sha: String,
    pub onto_sha: String,
    pub base_sha: String,
    pub commits: Vec<PlanCommit>,
}
#[derive(Debug, Serialize)]
pub struct BisectState {
    pub active: bool,
    pub current_sha: String,
    pub current_subject: String,
    pub finished: bool,
    pub first_bad: Option<String>,
    pub remaining: Option<u64>,
    pub log: String,
    pub output: String,
}
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BisectAction {
    Start,
    Good,
    Bad,
    Skip,
    Reset,
}
#[derive(Debug, Deserialize)]
pub struct BisectRequest {
    pub op: BisectAction,
    pub good: Option<String>,
    pub bad: Option<String>,
    pub expected_head: Option<String>,
}

impl LocalGit {
    pub async fn recovery_log(&self, limit: usize, skip: usize) -> Result<Vec<RecoveryEntry>> {
        let text = self
            .run_read(&[
                "reflog",
                "show",
                "--format=%H%x09%gD%x09%gs",
                &format!("--max-count={}", limit.clamp(1, 200)),
                &format!("--skip={skip}"),
                "HEAD",
            ])
            .await?;
        Ok(text
            .lines()
            .filter_map(|line| {
                let mut cols = line.splitn(3, '\t');
                Some(RecoveryEntry {
                    sha: cols.next()?.into(),
                    selector: cols.next()?.into(),
                    subject: cols.next()?.into(),
                })
            })
            .collect())
    }
    async fn commit_sha(&self, revision: &str) -> Result<String> {
        Self::guard_ref(revision)?;
        Ok(self
            .run_read(&[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{revision}^{{commit}}"),
            ])
            .await?
            .trim()
            .into())
    }
    async fn require_idle_clean(&self) -> Result<()> {
        if self.op_in_progress().await.is_some() || self.bisect_state().await?.active {
            return Err(Error::Conflict(
                "finish or abort the current Git operation first".into(),
            ));
        }
        if self.working_dirty().await? {
            return Err(Error::Conflict("commit or stash your changes first".into()));
        }
        Ok(())
    }
    pub async fn interactive_plan(&self, onto: &str) -> Result<InteractivePlan> {
        let head_sha = self.commit_sha("HEAD").await?;
        let onto_sha = self.commit_sha(onto).await?;
        let base_sha = self
            .run_read(&["merge-base", &head_sha, &onto_sha])
            .await?
            .trim()
            .to_string();
        let range = format!("{base_sha}..{head_sha}");
        if !self
            .run_read(&["rev-list", "--merges", &range])
            .await?
            .trim()
            .is_empty()
        {
            return Err(Error::Invalid("interactive planning currently requires linear history; this range contains merge commits".into()));
        }
        let text = self
            .run_read(&["log", "--reverse", "--format=%H%x09%s", &range, "--"])
            .await?;
        let commits = text
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(sha, subject)| PlanCommit {
                sha: sha.into(),
                subject: subject.into(),
                action: RebaseAction::Pick,
            })
            .collect();
        Ok(InteractivePlan {
            head_sha,
            onto_sha,
            base_sha,
            commits,
        })
    }
    pub async fn start_interactive_rebase(&self, plan: InteractivePlan) -> Result<RepoStatusResp> {
        self.require_idle_clean().await?;
        let fresh = self.interactive_plan(&plan.onto_sha).await?;
        if plan.head_sha != fresh.head_sha || plan.base_sha != fresh.base_sha {
            return Err(Error::Conflict(
                "the branch changed since preview — preview again".into(),
            ));
        }
        let expected: HashSet<&str> = fresh.commits.iter().map(|c| c.sha.as_str()).collect();
        let submitted: HashSet<&str> = plan.commits.iter().map(|c| c.sha.as_str()).collect();
        if plan.commits.is_empty() || submitted != expected || submitted.len() != plan.commits.len()
        {
            return Err(Error::Invalid(
                "the plan must contain every previewed commit exactly once".into(),
            ));
        }
        if plan.commits[0].action == RebaseAction::Squash {
            return Err(Error::Invalid(
                "the first commit cannot squash into a previous commit".into(),
            ));
        }
        // Only validated SHA/action pairs enter the todo, never editable subjects
        // or shell commands. Keep the helper alive until Git has consumed it.
        let mut todo =
            tempfile::NamedTempFile::new().map_err(|e| Error::Internal(e.to_string()))?;
        for commit in plan.commits {
            writeln!(todo, "{} {}", commit.action.word(), commit.sha)
                .map_err(|e| Error::Internal(e.to_string()))?;
        }
        let env = [
            (
                "GIT_SEQUENCE_EDITOR".into(),
                "cp \"$OTTO_REBASE_TODO\"".into(),
            ),
            (
                "OTTO_REBASE_TODO".into(),
                todo.path().to_string_lossy().into_owned(),
            ),
            ("GIT_EDITOR".into(), "true".into()),
        ];
        let result = self
            .run_env(
                &[
                    "rebase",
                    "--interactive",
                    "--onto",
                    &fresh.onto_sha,
                    &fresh.base_sha,
                ],
                &env,
            )
            .await;
        if let Err(e) = result {
            if self.conflicted_paths().await?.is_empty() {
                return Err(e);
            }
        }
        self.status().await
    }
    pub async fn skip_rebase(&self) -> Result<RepoStatusResp> {
        if self.op_in_progress().await != Some("rebase") {
            return Err(Error::Conflict("no rebase in progress".into()));
        }
        if let Err(e) = self
            .run_env(
                &["rebase", "--skip"],
                &[("GIT_EDITOR".into(), "true".into())],
            )
            .await
        {
            if self.conflicted_paths().await?.is_empty() {
                return Err(e);
            }
        }
        self.status().await
    }
    pub async fn bisect_state(&self) -> Result<BisectState> {
        let gd = self
            .git_dir()
            .await
            .ok_or_else(|| Error::NotFound("Git directory unavailable".into()))?;
        let active = gd.join("BISECT_START").exists();
        let current_sha = self.commit_sha("HEAD").await?;
        let current_subject = self
            .run_read(&["show", "-s", "--format=%s", "HEAD"])
            .await?
            .trim()
            .into();
        let log = if active {
            self.run_read(&["bisect", "log"]).await?
        } else {
            String::new()
        };
        let first_bad = log.lines().find_map(|line| {
            line.strip_prefix("# first bad commit: [")
                .and_then(|rest| rest.split_once(']').map(|(sha, _)| sha.to_string()))
        });
        let mut remaining = None;
        if active {
            let goods = self
                .run_read(&[
                    "for-each-ref",
                    "--format=%(objectname)",
                    "refs/bisect/good-*",
                ])
                .await?;
            let mut args = vec!["rev-list", "--count", "refs/bisect/bad", "--not"];
            args.extend(goods.lines());
            remaining = self
                .run_read(&args)
                .await
                .ok()
                .and_then(|s| s.trim().parse().ok());
        }
        let output = if active {
            tokio::fs::read_to_string(gd.join("otto-bisect-output"))
                .await
                .unwrap_or_default()
        } else {
            String::new()
        };
        Ok(BisectState {
            active,
            current_sha,
            current_subject,
            finished: first_bad.is_some(),
            first_bad,
            remaining,
            log,
            output,
        })
    }
    pub async fn bisect_action(&self, req: BisectRequest) -> Result<BisectState> {
        let state = self.bisect_state().await?;
        let output =
            match req.op {
                BisectAction::Start => {
                    self.require_idle_clean().await?;
                    let good =
                        self.commit_sha(req.good.as_deref().ok_or_else(|| {
                            Error::Invalid("choose a known good revision".into())
                        })?)
                        .await?;
                    let bad = self
                        .commit_sha(
                            req.bad.as_deref().ok_or_else(|| {
                                Error::Invalid("choose a known bad revision".into())
                            })?,
                        )
                        .await?;
                    if good == bad
                        || !self
                            .run_raw(&["merge-base", "--is-ancestor", &good, &bad], &[])
                            .await?
                            .0
                    {
                        return Err(Error::Invalid(
                            "the good revision must be an ancestor of the bad revision".into(),
                        ));
                    }
                    self.run(&["bisect", "start", &bad, &good]).await?
                }
                BisectAction::Reset => {
                    if !state.active {
                        return Err(Error::Conflict("no bisect in progress".into()));
                    }
                    if self.working_dirty().await? {
                        return Err(Error::Conflict(
                            "commit or stash your test changes before ending bisect".into(),
                        ));
                    }
                    self.run(&["bisect", "reset"]).await?
                }
                mark => {
                    if !state.active || state.finished {
                        return Err(Error::Conflict("no unfinished bisect in progress".into()));
                    }
                    if req.expected_head.as_deref() != Some(state.current_sha.as_str()) {
                        return Err(Error::Conflict(
                            "the bisect candidate changed — refresh before marking".into(),
                        ));
                    }
                    if self.working_dirty().await? {
                        return Err(Error::Conflict(
                            "commit or stash your test changes before marking a candidate".into(),
                        ));
                    }
                    let verb = match mark {
                        BisectAction::Good => "good",
                        BisectAction::Bad => "bad",
                        _ => "skip",
                    };
                    let (ok, out, err, code) = self.run_raw(&["bisect", verb], &[]).await?;
                    // All remaining revisions skipped is an ordinary inconclusive
                    // result; the Git log persists the candidates for inspection.
                    if !ok && code != Some(2) {
                        return Err(crate::local::upstream_err(&err, &out, code));
                    }
                    format!("{out}\n{err}")
                }
            };
        if let Some(gd) = self.git_dir().await {
            tokio::fs::write(gd.join("otto-bisect-output"), &output)
                .await
                .map_err(|e| Error::Internal(e.to_string()))?;
        }
        self.bisect_state().await
    }
}

#[derive(Deserialize)]
struct ReflogQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    skip: usize,
}
fn default_limit() -> usize {
    50
}
#[derive(Deserialize)]
struct PlanQuery {
    onto: String,
}
pub fn router<S: GitCtx>() -> Router<S> {
    Router::new()
        .route("/repos/{id}/reflog", get(reflog::<S>))
        .route(
            "/repos/{id}/rebase/plan",
            get(plan::<S>).post(start_plan::<S>),
        )
        .route("/repos/{id}/rebase/skip", post(skip::<S>))
        .route(
            "/repos/{id}/bisect",
            get(bisect::<S>).post(mark_bisect::<S>),
        )
}
async fn reflog<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<ReflogQuery>,
) -> ApiResult<Json<Vec<RecoveryEntry>>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.recovery_log(q.limit, q.skip).await?))
}
async fn plan<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<PlanQuery>,
) -> ApiResult<Json<InteractivePlan>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.interactive_plan(&q.onto).await?))
}
async fn start_plan<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(plan): Json<InteractivePlan>,
) -> ApiResult<Json<RepoStatusResp>> {
    let lock = repo_lock(&id);
    let guard = lock.lock_owned().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    // Request cancellation must not delete the sequence-editor input or release
    // serialization while Git is still rewriting history.
    let result = tokio::spawn(async move {
        let _guard = guard;
        git.start_interactive_rebase(plan).await
    })
    .await
    .map_err(|e| Error::Internal(e.to_string()))??;
    Ok(Json(result))
}
async fn skip<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<RepoStatusResp>> {
    let lock = repo_lock(&id);
    let guard = lock.lock_owned().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let result = tokio::spawn(async move {
        let _guard = guard;
        git.skip_rebase().await
    })
    .await
    .map_err(|e| Error::Internal(e.to_string()))??;
    Ok(Json(result))
}
async fn bisect<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<BisectState>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.bisect_state().await?))
}
async fn mark_bisect<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<BisectRequest>,
) -> ApiResult<Json<BisectState>> {
    let lock = repo_lock(&id);
    let guard = lock.lock_owned().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let result = tokio::spawn(async move {
        let _guard = guard;
        git.bisect_action(req).await
    })
    .await
    .map_err(|e| Error::Internal(e.to_string()))??;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::LocalGit;
    use std::path::Path;
    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }
    fn fixture() -> (tempfile::TempDir, LocalGit) {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-b", "main"]);
        git(dir.path(), &["config", "user.name", "Test"]);
        git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        for n in 0..5 {
            std::fs::write(dir.path().join(format!("file{n}")), format!("{n}\n")).unwrap();
            git(dir.path(), &["add", "."]);
            git(dir.path(), &["commit", "-m", &format!("test: commit {n}")]);
        }
        let local = LocalGit::new(dir.path());
        (dir, local)
    }
    #[tokio::test]
    async fn reflog_recovery_branch_keeps_current_head_and_worktree() {
        let (dir, local) = fixture();
        let head = git(dir.path(), &["rev-parse", "HEAD"]);
        let entries = local.recovery_log(2, 0).await.unwrap();
        assert_eq!(entries.len(), 2);
        local
            .create_branch("recovered", Some(&entries[1].sha), false)
            .await
            .unwrap();
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head);
        assert_eq!(git(dir.path(), &["rev-parse", "recovered"]), entries[1].sha);
        let page2 = local.recovery_log(2, 2).await.unwrap();
        assert_ne!(entries[1].selector, page2[0].selector);
    }
    #[tokio::test]
    async fn rebase_plan_reorders_squashes_and_rejects_stale_head() {
        let (dir, local) = fixture();
        let mut plan = local.interactive_plan("HEAD~3").await.unwrap();
        plan.commits.swap(0, 1);
        plan.commits[2].action = RebaseAction::Squash;
        local.start_interactive_rebase(plan.clone()).await.unwrap();
        assert_eq!(git(dir.path(), &["rev-list", "--count", "HEAD"]), "4");
        assert!(local.start_interactive_rebase(plan).await.is_err());
        assert!(local.op_in_progress().await.is_none());
    }
    #[tokio::test]
    async fn interactive_edit_can_continue_and_abort() {
        let (dir, local) = fixture();
        let old = git(dir.path(), &["rev-parse", "HEAD"]);
        let mut plan = local.interactive_plan("HEAD~2").await.unwrap();
        plan.commits[0].action = RebaseAction::Edit;
        local.start_interactive_rebase(plan).await.unwrap();
        assert_eq!(local.op_in_progress().await, Some("rebase"));
        local.merge_abort().await.unwrap();
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), old);
        let mut plan = local.interactive_plan("HEAD~2").await.unwrap();
        plan.commits[0].action = RebaseAction::Edit;
        local.start_interactive_rebase(plan).await.unwrap();
        local.merge_commit(None).await.unwrap();
        assert!(local.op_in_progress().await.is_none());
    }
    #[tokio::test]
    async fn bisect_resumes_finds_bad_and_restores_branch() {
        let (dir, local) = fixture();
        let original = git(dir.path(), &["rev-parse", "HEAD"]);
        let bad = git(dir.path(), &["rev-parse", "HEAD~1"]);
        local
            .bisect_action(BisectRequest {
                op: BisectAction::Start,
                good: Some("HEAD~4".into()),
                bad: Some(bad.clone()),
                expected_head: None,
            })
            .await
            .unwrap();
        let state = LocalGit::new(dir.path()).bisect_state().await.unwrap();
        assert!(state.active);
        for _ in 0..5 {
            let state = local.bisect_state().await.unwrap();
            if state.finished {
                break;
            }
            let mark = if state.current_sha == bad {
                BisectAction::Bad
            } else {
                BisectAction::Good
            };
            local
                .bisect_action(BisectRequest {
                    op: mark,
                    good: None,
                    bad: None,
                    expected_head: Some(state.current_sha),
                })
                .await
                .unwrap();
        }
        let state = local.bisect_state().await.unwrap();
        assert!(state.finished);
        assert_eq!(state.first_bad, Some(bad));
        local
            .bisect_action(BisectRequest {
                op: BisectAction::Reset,
                good: None,
                bad: None,
                expected_head: None,
            })
            .await
            .unwrap();
        assert!(!local.bisect_state().await.unwrap().active);
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), original);
        assert_eq!(git(dir.path(), &["branch", "--show-current"]), "main");
    }
    #[tokio::test]
    async fn plan_rejects_duplicates_and_skip_advances_edit_stop() {
        let (_dir, local) = fixture();
        let mut plan = local.interactive_plan("HEAD~2").await.unwrap();
        plan.commits[1] = plan.commits[0].clone();
        assert!(local.start_interactive_rebase(plan).await.is_err());
        assert!(local.op_in_progress().await.is_none());
        let mut plan = local.interactive_plan("HEAD~2").await.unwrap();
        plan.commits[0].action = RebaseAction::Edit;
        local.start_interactive_rebase(plan).await.unwrap();
        local.skip_rebase().await.unwrap();
        assert!(local.op_in_progress().await.is_none());
    }
    #[tokio::test]
    async fn bisect_counts_candidates_and_rejects_stale_marks() {
        let (_dir, local) = fixture();
        let state = local
            .bisect_action(BisectRequest {
                op: BisectAction::Start,
                good: Some("HEAD~4".into()),
                bad: Some("HEAD~1".into()),
                expected_head: None,
            })
            .await
            .unwrap();
        assert_eq!(state.remaining, Some(3));
        let before = state.current_sha;
        assert!(local
            .bisect_action(BisectRequest {
                op: BisectAction::Good,
                good: None,
                bad: None,
                expected_head: Some("stale".into())
            })
            .await
            .is_err());
        assert_eq!(local.bisect_state().await.unwrap().current_sha, before);
    }

    #[tokio::test]
    async fn recovery_operations_reject_dirty_tree_and_stale_marks() {
        let (dir, local) = fixture();
        let plan = local.interactive_plan("HEAD~2").await.unwrap();
        std::fs::write(dir.path().join("file0"), "uncommitted").unwrap();
        assert!(local.start_interactive_rebase(plan).await.is_err());
        assert!(local
            .bisect_action(BisectRequest {
                op: BisectAction::Start,
                good: Some("HEAD~4".into()),
                bad: Some("HEAD".into()),
                expected_head: None
            })
            .await
            .is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file0")).unwrap(),
            "uncommitted"
        );
    }
}
