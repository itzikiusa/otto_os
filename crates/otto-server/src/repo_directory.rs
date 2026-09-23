//! Cross-workspace repo discovery + reference resolution for agent tools.
//!
//! WHY: a repo row belongs to exactly ONE workspace (`repos.path` is globally
//! unique), but agents run in whatever workspace their session was opened in.
//! The git tools used to list only the session's own workspace and take an
//! opaque `repo_id`, so a session in workspace A asked to open a PR on a repo
//! registered in workspace B listed A's repos, guessed the repo NAME as the id,
//! got a 404 and gave up — until a human told it to "search again". This module
//! is the one place that answers "which repo does this caller mean?":
//!
//! - [`visible_repos`] — every repo in every workspace the caller may READ
//!   (Git:View feature grant + workspace Viewer, the exact gates
//!   `GET /workspaces/{id}/repos` applies), narrowed to an MCP token's
//!   workspace pin when one is set. Discovery only — every per-repo operation
//!   still authorizes against the repo's own workspace downstream.
//! - [`match_repo_ref`] — pure matching of a friendly reference (id, local
//!   path, remote `owner/repo` or URL, or name) against that set.
//! - [`resolve_repo`] — the async wrapper: adds the calling session's cwd as the
//!   fallback when no reference is given, and turns "nothing / several matched"
//!   into errors that list the candidates (or what IS available), so an agent
//!   never needs to be told to search again.
//!
//! Served over HTTP as `GET /git/repos/directory` + `GET /git/repos/resolve`
//! (the in-session `ottod mcp-tools` calls these) and used in-process by the
//! governed `otto.*` choke point (`mcp_outward::governed_invoke`).

use std::collections::{HashMap, HashSet};

use axum::extract::{Query, State};
use axum::Json;
use otto_core::auth::{AuthContext, RoleChecker};
use otto_core::domain::{Capability, Feature, Repo, User, WorkspaceRole};
use otto_core::{Error, Id};
use otto_state::{GitStore, GrantsRepo, WorkspacesRepo};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::CurrentAuthContext;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Cap on how many candidates an error message lists. The in-session bridge
/// shows the daemon's message to the agent, so it must stay readable; the
/// message points at `otto_list_repos` for the full set.
const MAX_LISTED: usize = 25;

/// One repo the caller may see, with the name of the workspace it lives in.
#[derive(Debug, Clone)]
pub(crate) struct RepoEntry {
    pub repo: Repo,
    pub workspace_name: String,
}

impl RepoEntry {
    /// The compact agent-facing row. `current` marks the caller's own
    /// workspace (the session's, or the explicit hint).
    pub(crate) fn to_json(&self, current: Option<&str>) -> Value {
        json!({
            "id": self.repo.id,
            "name": self.repo.name,
            "path": self.repo.path,
            "remote_url": self.repo.remote_url,
            "provider": self.repo.provider,
            "workspace_id": self.repo.workspace_id,
            "workspace_name": self.workspace_name,
            "current": current == Some(self.repo.workspace_id.as_str()),
        })
    }

    /// One line for an error listing: `id  name  (workspace: X)  remote-slug`.
    fn describe(&self) -> String {
        let remote = self
            .repo
            .remote_url
            .as_deref()
            .map(|r| format!("  {}", normalize_remote(r)))
            .unwrap_or_default();
        format!(
            "{}  {}  (workspace: {}){remote}",
            self.repo.id, self.repo.name, self.workspace_name
        )
    }
}

/// How a reference was matched — echoed to the caller so an inferred repo is
/// never a silent guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatchedBy {
    Id,
    Path,
    Remote,
    Name,
    SessionCwd,
}

impl MatchedBy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Path => "path",
            Self::Remote => "remote",
            Self::Name => "name",
            Self::SessionCwd => "session_cwd",
        }
    }
}

/// Outcome of [`match_repo_ref`].
#[derive(Debug)]
pub(crate) enum RefMatch<'a> {
    One(&'a RepoEntry, MatchedBy),
    Ambiguous(Vec<&'a RepoEntry>),
    None,
}

/// What repo discovery reads — the few stores behind [`ServerCtx`], split out
/// so the RBAC / pin behaviour is testable against a bare SQLite pool.
pub(crate) struct RepoSources<'a> {
    pub grants: GrantsRepo,
    pub git: &'a GitStore,
    pub roles: &'a dyn RoleChecker,
    pub workspaces: &'a WorkspacesRepo,
}

impl<'a> RepoSources<'a> {
    pub(crate) fn of(ctx: &'a ServerCtx) -> Self {
        Self {
            grants: GrantsRepo::new(ctx.pool.clone()),
            git: &ctx.git_store,
            roles: ctx.roles.as_ref(),
            workspaces: &ctx.workspaces,
        }
    }
}

/// Every repo the caller may READ, across all workspaces. Mirrors the gates of
/// `GET /workspaces/{id}/repos` exactly — the Git:View feature grant (the
/// central guard's axis, re-checked here because the governed choke point
/// calls this in-process, not through a routed request) and the workspace
/// Viewer role (`RoleChecker::check`, root passes). `pin` narrows to one
/// workspace: an MCP token's `McpScope.workspace_id`, so a pinned token can
/// never even SEE a repo outside its pin (resolution then cannot land there).
pub(crate) async fn visible_repos(
    src: &RepoSources<'_>,
    user: &User,
    pin: Option<&str>,
) -> Result<Vec<RepoEntry>, Error> {
    let cap = src.grants.capability_of(user, Feature::Git).await?;
    if cap < Capability::View {
        return Err(Error::Forbidden("requires git:view".into()));
    }
    let all = src.git.list_all_repos().await?;
    let ws_ids: HashSet<Id> = all
        .iter()
        .map(|r| r.workspace_id.clone())
        .filter(|ws| pin.is_none_or(|p| p == ws.as_str()))
        .collect();
    let mut names: HashMap<Id, String> = HashMap::new();
    for ws in ws_ids {
        if src
            .roles
            .check(user, &ws, WorkspaceRole::Viewer)
            .await
            .is_err()
        {
            continue;
        }
        let name = src
            .workspaces
            .get(&ws)
            .await
            .map(|w| w.name)
            .unwrap_or_else(|_| ws.clone());
        names.insert(ws, name);
    }
    Ok(all
        .into_iter()
        .filter_map(|repo| {
            let workspace_name = names.get(&repo.workspace_id)?.clone();
            Some(RepoEntry {
                repo,
                workspace_name,
            })
        })
        .collect())
}

/// Order entries for listing: the caller's workspace first, then by workspace
/// name, then repo name (case-insensitive).
pub(crate) fn sort_for_listing(entries: &mut [RepoEntry], current: Option<&str>) {
    entries.sort_by(|a, b| {
        let a_cur = current == Some(a.repo.workspace_id.as_str());
        let b_cur = current == Some(b.repo.workspace_id.as_str());
        b_cur
            .cmp(&a_cur)
            .then_with(|| {
                a.workspace_name
                    .to_lowercase()
                    .cmp(&b.workspace_name.to_lowercase())
            })
            .then_with(|| a.repo.name.to_lowercase().cmp(&b.repo.name.to_lowercase()))
    });
}

/// Canonical `host/owner/repo` form of a git remote so the https, ssh and
/// scp-like spellings of one remote compare equal: lowercased, scheme,
/// userinfo, port, trailing `/` and `.git` stripped.
/// `https://u@bitbucket.org/team/app.git` and `git@bitbucket.org:team/app`
/// both become `bitbucket.org/team/app`.
pub(crate) fn normalize_remote(raw: &str) -> String {
    let s = raw.trim().to_ascii_lowercase();
    let (has_scheme, rest) = match s.find("://") {
        Some(i) => (true, &s[i + 3..]),
        None => (false, s.as_str()),
    };
    // Userinfo is everything before an `@` that precedes the first `/`.
    let rest = match rest.find('@') {
        Some(at) if !rest[..at].contains('/') => &rest[at + 1..],
        _ => rest,
    };
    let (host, path) = if !has_scheme && rest.contains(':') && !rest.starts_with('/') {
        // scp-like `host:owner/repo`
        rest.split_once(':').unwrap_or((rest, ""))
    } else {
        rest.split_once('/').unwrap_or((rest, ""))
    };
    let host = host.split(':').next().unwrap_or(host);
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path).trim_end_matches('/');
    if path.is_empty() {
        host.to_string()
    } else {
        format!("{host}/{path}")
    }
}

/// True when `reference` should be tried as a local path.
fn is_path_ref(reference: &str) -> bool {
    reference.starts_with('/') || reference.starts_with('~')
}

/// True when `reference` should be tried as a remote: a URL, an scp-like
/// `git@host:owner/repo`, or a slash-separated `owner/repo` / `host/owner/repo`.
fn is_remote_ref(reference: &str) -> bool {
    !is_path_ref(reference)
        && (reference.contains("://") || reference.contains('/') || reference.contains('@'))
}

/// Expand a leading `~` and drop trailing separators.
fn normalize_path(p: &str) -> String {
    let expanded = match p.strip_prefix('~') {
        Some(rest) => match std::env::var("HOME") {
            Ok(home) => format!("{}{}", home.trim_end_matches('/'), rest),
            Err(_) => p.to_string(),
        },
        None => p.to_string(),
    };
    let trimmed = expanded.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".into()
    } else {
        trimmed.to_string()
    }
}

/// Entries whose registered path IS `path` or an ancestor of it — keeping only
/// the deepest (a nested repo beats its parent). Paths are globally unique, so
/// this is at most one entry in practice.
fn match_path<'a>(entries: &'a [RepoEntry], path: &str) -> Vec<&'a RepoEntry> {
    let path = normalize_path(path);
    let hits: Vec<&RepoEntry> = entries
        .iter()
        .filter(|e| {
            let root = normalize_path(&e.repo.path);
            path == root || path.starts_with(&format!("{root}/"))
        })
        .collect();
    let deepest = hits.iter().map(|e| e.repo.path.len()).max().unwrap_or(0);
    hits.into_iter()
        .filter(|e| e.repo.path.len() == deepest)
        .collect()
}

/// Entries whose remote equals `reference` (any URL spelling) or ends with it
/// at a `/` boundary (`owner/repo`, `host/owner/repo`).
fn match_remote<'a>(entries: &'a [RepoEntry], reference: &str) -> Vec<&'a RepoEntry> {
    let want = normalize_remote(reference);
    if want.is_empty() {
        return vec![];
    }
    // A bare `owner/repo` normalizes to itself with `owner` read as the host —
    // harmless: equality then fails and the suffix test below does the work.
    let suffix = format!("/{want}");
    entries
        .iter()
        .filter(|e| {
            e.repo.remote_url.as_deref().is_some_and(|r| {
                let have = normalize_remote(r);
                have == want || have.ends_with(&suffix)
            })
        })
        .collect()
}

/// Settle a tier's hits: one → that entry; several → prefer the one in
/// `prefer_ws` if exactly one lives there, else ambiguous.
fn settle<'a>(hits: Vec<&'a RepoEntry>, by: MatchedBy, prefer_ws: Option<&str>) -> RefMatch<'a> {
    match hits.len() {
        0 => RefMatch::None,
        1 => RefMatch::One(hits[0], by),
        _ => {
            let preferred: Vec<&RepoEntry> = hits
                .iter()
                .copied()
                .filter(|e| prefer_ws == Some(e.repo.workspace_id.as_str()))
                .collect();
            if preferred.len() == 1 {
                RefMatch::One(preferred[0], by)
            } else {
                RefMatch::Ambiguous(hits)
            }
        }
    }
}

/// Match a friendly repo reference against `entries`. Tiers, first hit wins:
/// exact id → local path (the repo containing it) → remote (`owner/repo`, any
/// URL spelling) → name (case-insensitive). Several hits in one tier are broken
/// by `prefer_ws` (the caller's workspace) when exactly one lives there, else
/// reported as ambiguous. Pure — no I/O.
pub(crate) fn match_repo_ref<'a>(
    entries: &'a [RepoEntry],
    reference: &str,
    prefer_ws: Option<&str>,
) -> RefMatch<'a> {
    let reference = reference.trim();
    if reference.is_empty() {
        return RefMatch::None;
    }
    if let Some(e) = entries.iter().find(|e| e.repo.id == reference) {
        return RefMatch::One(e, MatchedBy::Id);
    }
    if is_path_ref(reference) {
        return settle(match_path(entries, reference), MatchedBy::Path, prefer_ws);
    }
    if is_remote_ref(reference) {
        let hits = match_remote(entries, reference);
        if !hits.is_empty() {
            return settle(hits, MatchedBy::Remote, prefer_ws);
        }
    }
    // A name — or, for an `owner/repo` / URL that matched no remote, the last
    // segment naming a checkout whose remote Otto never recorded. A repo WITH
    // a recorded (different) remote is never picked that way: `acme/app` must
    // not land on `github.com/other/app`.
    let remote_like = is_remote_ref(reference);
    let name = reference
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(reference);
    let name = name.strip_suffix(".git").unwrap_or(name);
    let hits = entries
        .iter()
        .filter(|e| !remote_like || e.repo.remote_url.is_none())
        .filter(|e| e.repo.name.eq_ignore_ascii_case(name))
        .collect();
    settle(hits, MatchedBy::Name, prefer_ws)
}

/// A resolved reference.
#[derive(Debug, Clone)]
pub(crate) struct Resolved {
    pub entry: RepoEntry,
    pub matched_by: MatchedBy,
}

impl Resolved {
    /// Human label for approval prompts: `name (workspace: X)`.
    pub(crate) fn label(&self) -> String {
        format!(
            "{} (workspace: {})",
            self.entry.repo.name, self.entry.workspace_name
        )
    }
}

/// The calling session (Otto-minted per-session credentials only), as
/// `(cwd, workspace_id)`. `None` for a human/PAT/external-MCP caller.
async fn caller_session(ctx: &ServerCtx, auth: &AuthContext) -> Option<(String, Id)> {
    let sid = auth
        .managed_session_id
        .as_ref()
        .or(auth.mcp_session_id.as_ref())?;
    let s = ctx.manager.get(sid).await.ok()?;
    Some((s.cwd, s.workspace_id))
}

/// Format the "several matched" error.
fn ambiguous_error(reference: &str, hits: &[&RepoEntry]) -> Error {
    let list = hits
        .iter()
        .take(MAX_LISTED)
        .map(|e| format!("- {}", e.describe()))
        .collect::<Vec<_>>()
        .join("\n");
    Error::Conflict(format!(
        "repo reference '{reference}' matches {} repos — pass one of these ids as repo_id:\n{list}",
        hits.len()
    ))
}

/// Format the "nothing matched" error: near misses first (name contains the
/// reference or vice versa), else a sample of what IS available.
fn not_found_error(what: &str, reference: &str, entries: &[RepoEntry]) -> Error {
    if entries.is_empty() {
        return Error::NotFound(format!(
            "{what}: you have no registered git repositories in any workspace you can read. \
             Register it in Otto (Git → Add repository) first."
        ));
    }
    let needle = reference
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(reference)
        .trim_end_matches(".git")
        .to_lowercase();
    let near: Vec<&RepoEntry> = if needle.len() >= 3 {
        entries
            .iter()
            .filter(|e| {
                let n = e.repo.name.to_lowercase();
                n.contains(&needle) || needle.contains(&n)
            })
            .collect()
    } else {
        vec![]
    };
    let (heading, pool): (&str, Vec<&RepoEntry>) = if near.is_empty() {
        ("Repositories you can use", entries.iter().collect())
    } else {
        ("Closest matches", near)
    };
    let more = pool.len().saturating_sub(MAX_LISTED);
    let mut list = pool
        .iter()
        .take(MAX_LISTED)
        .map(|e| format!("- {}", e.describe()))
        .collect::<Vec<_>>()
        .join("\n");
    if more > 0 {
        list.push_str(&format!(
            "\n… and {more} more — call otto_list_repos for the full list"
        ));
    }
    Error::NotFound(format!(
        "{what} (searched {} repos across every workspace you can read). {heading}:\n{list}\n\
         Pass an id above as repo_id, or a repo name, local path, or remote (owner/repo). \
         If the repo is not registered in Otto, add it under Git → Add repository.",
        entries.len()
    ))
}

/// Resolve a repo reference for `auth`'s effective user.
///
/// - `reference` — an id, local path, remote (`owner/repo` / URL) or name.
///   Empty/`None` → the calling session's cwd (walks up to the enclosing repo,
///   then falls back to matching the checkout's `origin` remote — a workflow
///   clone under `workflow-runs/` is not a registered path, but its remote is).
/// - `ws_filter` — optional: only consider this workspace (the old narrow
///   behaviour). A pinned MCP token may only filter to its own pin.
///
/// Visibility is [`visible_repos`] (Git:View + workspace Viewer + the token's
/// pin), so a reference can never resolve to a repo the caller could not
/// already list. Errors: `Forbidden` (no Git access / pin mismatch),
/// `Conflict` (ambiguous, listing candidates), `NotFound` (nothing matched,
/// or nothing to go on — listing what IS available either way).
pub(crate) async fn resolve_repo(
    ctx: &ServerCtx,
    auth: &AuthContext,
    reference: Option<&str>,
    ws_filter: Option<&str>,
) -> Result<Resolved, Error> {
    let session = caller_session(ctx, auth).await;
    resolve_repo_in(&RepoSources::of(ctx), auth, session, reference, ws_filter).await
}

/// [`resolve_repo`] over explicit sources + an already-looked-up calling
/// session `(cwd, workspace_id)` — the testable core.
pub(crate) async fn resolve_repo_in(
    src: &RepoSources<'_>,
    auth: &AuthContext,
    session: Option<(String, Id)>,
    reference: Option<&str>,
    ws_filter: Option<&str>,
) -> Result<Resolved, Error> {
    let pin = auth
        .mcp_scope
        .as_ref()
        .and_then(|s| s.workspace_id.as_deref())
        .filter(|s| !s.is_empty());
    let ws_filter = ws_filter.filter(|s| !s.is_empty());
    if let (Some(p), Some(f)) = (pin, ws_filter) {
        if p != f {
            return Err(Error::Forbidden(format!(
                "this token is scoped to workspace '{p}'"
            )));
        }
    }
    let mut entries = visible_repos(src, &auth.effective_user, pin).await?;
    if let Some(f) = ws_filter {
        entries.retain(|e| e.repo.workspace_id == f);
    }
    let prefer = ws_filter.or(session.as_ref().map(|(_, ws)| ws.as_str()));
    let reference = reference.map(str::trim).filter(|s| !s.is_empty());

    if let Some(r) = reference {
        if let Some(hit) = found(match_repo_ref(&entries, r, prefer), r)? {
            return Ok(hit);
        }
        // An absolute path outside every registered root (a clone, a worktree):
        // match its checkout's `origin` remote instead. Only ever lands on a
        // repo already in `entries`, so this widens nothing.
        if is_path_ref(r) {
            if let Some(hit) = resolve_checkout_remote(&entries, &normalize_path(r), prefer)
                .await
                .map(|m| found(m, r))
                .transpose()?
                .flatten()
            {
                return Ok(hit);
            }
        }
        return Err(not_found_error(
            &format!("no git repository matches '{r}'"),
            r,
            &entries,
        ));
    }

    if let Some((cwd, _)) = &session {
        let direct = settle(match_path(&entries, cwd), MatchedBy::SessionCwd, prefer);
        let hit = match found(direct, cwd)? {
            Some(h) => Some(h),
            None => resolve_checkout_remote(&entries, cwd, prefer)
                .await
                .map(|m| found(m, cwd))
                .transpose()?
                .flatten(),
        };
        if let Some(hit) = hit {
            return Ok(Resolved {
                matched_by: MatchedBy::SessionCwd,
                ..hit
            });
        }
        return Err(not_found_error(
            &format!(
                "no repo_id given and this session's working directory ({cwd}) is not inside \
                 a registered repo whose remote Otto knows"
            ),
            cwd,
            &entries,
        ));
    }
    Err(not_found_error(
        "repo_id is required (no calling session to infer it from)",
        "",
        &entries,
    ))
}

/// Lift a [`RefMatch`] into the resolver's result: one → resolved, several →
/// the ambiguity error (labelled with what the caller asked for), none → `None`.
fn found(m: RefMatch<'_>, label: &str) -> Result<Option<Resolved>, Error> {
    match m {
        RefMatch::One(e, by) => Ok(Some(Resolved {
            entry: e.clone(),
            matched_by: by,
        })),
        RefMatch::Ambiguous(hits) => Err(ambiguous_error(label, &hits)),
        RefMatch::None => Ok(None),
    }
}

/// Match the `origin` remote of the checkout containing `path` (read-only
/// `git rev-parse` / `git remote get-url`) against `entries`. `None` when the
/// path is not a checkout or has no remote.
async fn resolve_checkout_remote<'a>(
    entries: &'a [RepoEntry],
    path: &str,
    prefer_ws: Option<&str>,
) -> Option<RefMatch<'a>> {
    if !std::path::Path::new(path).is_absolute() || tokio::fs::metadata(path).await.is_err() {
        return None;
    }
    let git = otto_git::LocalGit::new(path);
    let top = git.toplevel().await.ok()?;
    // The checkout's root may itself be registered (the path pointed below it
    // through a symlinked parent, say).
    if let m @ RefMatch::One(..) = settle(match_path(entries, &top), MatchedBy::Path, prefer_ws) {
        return Some(m);
    }
    let remote = otto_git::LocalGit::new(&top).remote_url().await?;
    Some(settle(
        match_remote(entries, &remote),
        MatchedBy::Remote,
        prefer_ws,
    ))
}

// ===========================================================================
// HTTP: GET /git/repos/directory, GET /git/repos/resolve
// ===========================================================================

#[derive(Deserialize)]
pub struct RepoDirectoryQuery {
    /// Optional: only this workspace (the narrow, pre-directory behaviour).
    pub workspace_id: Option<String>,
    /// Optional: which workspace counts as "current" for ordering. Defaults to
    /// the calling session's workspace when the token is a session credential.
    pub prefer_workspace_id: Option<String>,
}

/// `GET /git/repos/directory` — every repo across the workspaces the caller may
/// read, each row carrying its workspace id + name, the caller's current
/// workspace first. Backs `otto_list_repos` / `otto.list_repos`.
pub async fn repo_directory(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Query(q): Query<RepoDirectoryQuery>,
) -> ApiResult<Json<Value>> {
    let filter = q.workspace_id.as_deref().filter(|s| !s.is_empty());
    let mut entries = visible_repos(&RepoSources::of(&ctx), &auth.effective_user, None)
        .await
        .map_err(ApiError)?;
    if let Some(f) = filter {
        entries.retain(|e| e.repo.workspace_id == f);
    }
    let current = match q.prefer_workspace_id.filter(|s| !s.is_empty()) {
        Some(ws) => Some(ws),
        None => caller_session(&ctx, &auth).await.map(|(_, ws)| ws),
    };
    sort_for_listing(&mut entries, current.as_deref());
    let workspaces: HashSet<&str> = entries
        .iter()
        .map(|e| e.repo.workspace_id.as_str())
        .collect();
    Ok(Json(json!({
        "repos": entries
            .iter()
            .map(|e| e.to_json(current.as_deref()))
            .collect::<Vec<_>>(),
        "current_workspace_id": current,
        "workspace_count": workspaces.len(),
    })))
}

#[derive(Deserialize)]
pub struct RepoResolveQuery {
    /// Repo reference: id, name, local path, or remote (`owner/repo` / URL).
    /// Omit to use the calling session's cwd.
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    /// Optional: only look in this workspace.
    pub workspace_id: Option<String>,
}

/// `GET /git/repos/resolve?ref=&workspace_id=` — resolve a friendly repo
/// reference (see [`resolve_repo`]). 404 lists what IS available, 409 lists the
/// ambiguous candidates.
pub async fn repo_resolve(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Query(q): Query<RepoResolveQuery>,
) -> ApiResult<Json<Value>> {
    let current = caller_session(&ctx, &auth).await.map(|(_, ws)| ws);
    let r = resolve_repo(
        &ctx,
        &auth,
        q.reference.as_deref(),
        q.workspace_id.as_deref(),
    )
    .await
    .map_err(ApiError)?;
    Ok(Json(json!({
        "repo": r.entry.to_json(current.as_deref()),
        "matched_by": r.matched_by.as_str(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, ws: &str, name: &str, path: &str, remote: Option<&str>) -> RepoEntry {
        RepoEntry {
            repo: Repo {
                id: id.into(),
                workspace_id: ws.into(),
                name: name.into(),
                path: path.into(),
                remote_url: remote.map(str::to_string),
                provider: None,
                git_account_id: None,
                created_at: chrono::Utc::now(),
                forge: None,
            },
            workspace_name: format!("{ws}-name"),
        }
    }

    fn fixture() -> Vec<RepoEntry> {
        vec![
            entry(
                "r-promo",
                "ws-b",
                "promotions",
                "/Users/me/promotions",
                Some("https://me@bitbucket.org/team/promotions.git"),
            ),
            entry(
                "r-games",
                "ws-b",
                "games_management",
                "/Users/me/games_management",
                Some("git@bitbucket.org:team/games_management.git"),
            ),
            entry(
                "r-otto",
                "ws-a",
                "otto_os",
                "/Users/me/otto_os",
                Some("https://github.com/acme/otto_os.git"),
            ),
            entry(
                "r-nested",
                "ws-a",
                "mcp",
                "/Users/me/otto_os/mcp",
                Some("https://github.com/acme/mcp.git"),
            ),
            // Same remote registered twice (two checkouts in two workspaces).
            entry(
                "r-dup-a",
                "ws-a",
                "shared",
                "/Users/me/a/shared",
                Some("https://github.com/acme/shared.git"),
            ),
            entry(
                "r-dup-b",
                "ws-b",
                "shared",
                "/Users/me/b/shared",
                Some("git@github.com:acme/shared.git"),
            ),
        ]
    }

    fn one(m: RefMatch<'_>) -> (String, MatchedBy) {
        match m {
            RefMatch::One(e, by) => (e.repo.id.clone(), by),
            other => panic!("expected one match, got {other:?}"),
        }
    }

    #[test]
    fn normalize_remote_unifies_https_ssh_and_scp_spellings() {
        let want = "bitbucket.org/team/app";
        for raw in [
            "https://u@bitbucket.org/team/app.git",
            "https://bitbucket.org/team/app",
            "git@bitbucket.org:team/app.git",
            "ssh://git@bitbucket.org:22/team/app.git",
            "HTTPS://BITBUCKET.ORG/team/app/",
        ] {
            assert_eq!(normalize_remote(raw), want, "{raw}");
        }
    }

    #[test]
    fn resolves_by_id_path_remote_and_name() {
        let es = fixture();
        assert_eq!(one(match_repo_ref(&es, "r-otto", None)), ("r-otto".into(), MatchedBy::Id));
        // A path inside a repo resolves to it; the nested repo beats its parent.
        assert_eq!(
            one(match_repo_ref(&es, "/Users/me/otto_os/crates/x", None)),
            ("r-otto".into(), MatchedBy::Path)
        );
        assert_eq!(
            one(match_repo_ref(&es, "/Users/me/otto_os/mcp/src/", None)),
            ("r-nested".into(), MatchedBy::Path)
        );
        // `owner/repo`, full URLs in any spelling, and host/owner/repo.
        assert_eq!(
            one(match_repo_ref(&es, "team/games_management", None)),
            ("r-games".into(), MatchedBy::Remote)
        );
        assert_eq!(
            one(match_repo_ref(
                &es,
                "https://bitbucket.org/team/games_management",
                None
            )),
            ("r-games".into(), MatchedBy::Remote)
        );
        assert_eq!(
            one(match_repo_ref(&es, "bitbucket.org/team/promotions", None)),
            ("r-promo".into(), MatchedBy::Remote)
        );
        // A name, case-insensitively — the exact failure seen in the call log
        // (`repo_id: "games_management"` from a session in another workspace).
        assert_eq!(
            one(match_repo_ref(&es, "Games_Management", None)),
            ("r-games".into(), MatchedBy::Name)
        );
    }

    #[test]
    fn remote_like_refs_never_fall_back_to_a_repo_with_a_different_remote() {
        let mut es = fixture();
        // `other/otto_os` names a different fork than the recorded remote.
        assert!(matches!(
            match_repo_ref(&es, "other/otto_os", None),
            RefMatch::None
        ));
        // …but a checkout whose remote was never recorded still matches by name.
        es.push(entry("r-local", "ws-a", "scratchpad", "/Users/me/scratchpad", None));
        assert_eq!(
            one(match_repo_ref(&es, "acme/scratchpad", None)),
            ("r-local".into(), MatchedBy::Name)
        );
    }

    #[test]
    fn ambiguity_is_reported_unless_the_callers_workspace_settles_it() {
        let es = fixture();
        match match_repo_ref(&es, "shared", None) {
            RefMatch::Ambiguous(hits) => {
                let mut ids: Vec<_> = hits.iter().map(|e| e.repo.id.as_str()).collect();
                ids.sort();
                assert_eq!(ids, ["r-dup-a", "r-dup-b"]);
                let msg = ambiguous_error("shared", &hits).to_string();
                assert!(msg.contains("r-dup-a") && msg.contains("ws-b-name"), "{msg}");
            }
            other => panic!("expected ambiguity, got {other:?}"),
        }
        // Remote ambiguity too (two spellings of one remote).
        assert!(matches!(
            match_repo_ref(&es, "acme/shared", None),
            RefMatch::Ambiguous(_)
        ));
        // The caller's own workspace breaks the tie.
        assert_eq!(
            one(match_repo_ref(&es, "shared", Some("ws-b"))),
            ("r-dup-b".into(), MatchedBy::Name)
        );
        assert_eq!(
            one(match_repo_ref(&es, "acme/shared", Some("ws-a"))),
            ("r-dup-a".into(), MatchedBy::Remote)
        );
    }

    #[test]
    fn not_found_lists_near_misses_or_what_is_available() {
        let es = fixture();
        assert!(matches!(match_repo_ref(&es, "nope", None), RefMatch::None));
        assert!(matches!(
            match_repo_ref(&es, "/elsewhere/checkout", None),
            RefMatch::None
        ));
        let near = not_found_error("no git repository matches 'games'", "games", &es).to_string();
        assert!(near.contains("Closest matches") && near.contains("r-games"), "{near}");
        assert!(!near.contains("r-otto"), "near-miss list stays focused: {near}");
        let all = not_found_error("x", "zz", &es).to_string();
        assert!(all.contains("Repositories you can use") && all.contains("r-otto"), "{all}");
        let none = not_found_error("x", "zz", &[]).to_string();
        assert!(none.contains("Add repository"), "{none}");
    }

    // ---- DB-backed: RBAC + token pin + session-cwd fallback ---------------

    use otto_core::auth::McpScope;
    use otto_state::NewRepo;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use sqlx::SqlitePool;

    struct World {
        pool: SqlitePool,
        git: GitStore,
        roles: otto_rbac::RbacRoleChecker,
        workspaces: WorkspacesRepo,
        ws_a: Id,
        ws_b: Id,
        repo_a: Id,
        repo_b: Id,
    }

    impl World {
        fn src(&self) -> RepoSources<'_> {
            RepoSources {
                grants: GrantsRepo::new(self.pool.clone()),
                git: &self.git,
                roles: &self.roles,
                workspaces: &self.workspaces,
            }
        }
    }

    /// Whether an error's candidate listing (its `- ` rows) names `needle`.
    fn listed(msg: &str, needle: &str) -> bool {
        msg.lines()
            .any(|l| l.starts_with("- ") && l.contains(needle))
    }

    fn user(id: &str, root: bool) -> User {
        User {
            id: id.into(),
            username: id.into(),
            display_name: id.into(),
            is_root: root,
            disabled: false,
            created_at: chrono::Utc::now(),
        }
    }

    fn auth_for(u: User, scope: Option<McpScope>) -> AuthContext {
        AuthContext {
            real_user: u.clone(),
            effective_user: u,
            scope: None,
            mcp_only: scope.is_some(),
            mcp_scope: scope,
            mcp_internal: false,
            mcp_session_id: None,
            managed_session_id: None,
        }
    }

    /// `owner` admins workspaces A and B. `alice` (Git:View) is a Viewer of A
    /// only; `bob` is a member of A but holds no Git grant. `games` lives in B,
    /// `tools` in A.
    async fn world() -> World {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("in-memory sqlite");
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        for id in ["owner", "alice", "bob"] {
            sqlx::query(
                "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
                 VALUES (?, ?, 'x', ?, 0, ?)",
            )
            .bind(id)
            .bind(id)
            .bind(id)
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .expect("seed user");
        }
        let workspaces = WorkspacesRepo::new(pool.clone());
        let owner: Id = "owner".into();
        let ws_a = workspaces.create("alpha", "/nonexistent/a", &owner).await.unwrap().id;
        let ws_b = workspaces.create("beta", "/nonexistent/b", &owner).await.unwrap().id;
        workspaces
            .set_member(&ws_a, &"alice".into(), WorkspaceRole::Viewer)
            .await
            .unwrap();
        workspaces
            .set_member(&ws_a, &"bob".into(), WorkspaceRole::Editor)
            .await
            .unwrap();
        let grants = GrantsRepo::new(pool.clone());
        for u in ["owner", "alice"] {
            grants
                .set_grants(u, &[(Feature::Git, Capability::View)])
                .await
                .unwrap();
        }
        let git = GitStore::new(pool.clone());
        let mk = |ws: &Id, name: &str, remote: &str| NewRepo {
            workspace_id: ws.clone(),
            name: name.into(),
            path: format!("/nonexistent/otto-repo-dir-test/{name}"),
            remote_url: Some(remote.into()),
            provider: None,
            git_account_id: None,
        };
        let repo_a = git
            .create_repo(mk(&ws_a, "tools", "https://bitbucket.org/team/tools.git"))
            .await
            .unwrap()
            .id;
        let repo_b = git
            .create_repo(mk(&ws_b, "games", "git@bitbucket.org:team/games.git"))
            .await
            .unwrap()
            .id;
        World {
            roles: otto_rbac::RbacRoleChecker::new(pool.clone()),
            git,
            workspaces,
            pool,
            ws_a,
            ws_b,
            repo_a,
            repo_b,
        }
    }

    /// The reported bug: a session in workspace A names a repo registered in
    /// workspace B — by name, by remote, from inside its checkout — and it
    /// resolves, carrying B's id + name.
    #[tokio::test]
    async fn resolves_a_repo_registered_in_another_readable_workspace() {
        let w = world().await;
        let owner = auth_for(user("owner", false), None);
        let session_in_a = Some(("/elsewhere".to_string(), w.ws_a.clone()));
        for reference in ["games", "team/games", "https://bitbucket.org/team/games.git"] {
            let r = resolve_repo_in(&w.src(), &owner, session_in_a.clone(), Some(reference), None)
                .await
                .unwrap_or_else(|e| panic!("{reference}: {e}"));
            assert_eq!(r.entry.repo.id, w.repo_b, "{reference}");
            assert_eq!(r.entry.workspace_name, "beta");
        }
        // The directory spans both workspaces.
        let all = visible_repos(&w.src(), &owner.effective_user, None).await.unwrap();
        assert_eq!(all.len(), 2);
        // A `workspace_id` filter keeps the old narrow behaviour.
        let err = resolve_repo_in(&w.src(), &owner, None, Some("games"), Some(&w.ws_a))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "{err}");
        // No reference and the session cwd inside a registered checkout → it.
        let cwd = Some((
            "/nonexistent/otto-repo-dir-test/tools/src/deep".to_string(),
            w.ws_b.clone(),
        ));
        let r = resolve_repo_in(&w.src(), &owner, cwd, None, None).await.unwrap();
        assert_eq!((r.entry.repo.id.as_str(), r.matched_by), (w.repo_a.as_str(), MatchedBy::SessionCwd));
        // No reference and no session → an error that lists what IS available.
        let err = resolve_repo_in(&w.src(), &owner, None, None, None).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(w.repo_a.as_str()) && msg.contains(w.repo_b.as_str()), "{msg}");
    }

    /// Resolution only ever sees workspaces the caller can READ: alice
    /// (Viewer of A only) cannot reach B's repo by any reference, and the
    /// not-found listing never leaks it.
    #[tokio::test]
    async fn resolution_never_crosses_into_an_unreadable_workspace() {
        let w = world().await;
        let alice = auth_for(user("alice", false), None);
        for reference in ["games", "team/games", w.repo_b.as_str()] {
            let err = resolve_repo_in(&w.src(), &alice, None, Some(reference), None)
                .await
                .unwrap_err();
            let msg = err.to_string();
            assert!(matches!(err, Error::NotFound(_)), "{reference}: {msg}");
            // The candidate listing (the `- ` rows) never names B's repo — only
            // the caller's own reference is echoed back.
            assert!(!listed(&msg, w.repo_b.as_str()) && !msg.contains("beta"), "{msg}");
        }
        let visible = visible_repos(&w.src(), &alice.effective_user, None).await.unwrap();
        let ids: Vec<&str> = visible.iter().map(|e| e.repo.id.as_str()).collect();
        assert_eq!(ids, [w.repo_a.as_str()]);
        // No Git feature grant → no directory at all.
        let bob = auth_for(user("bob", false), None);
        let err = resolve_repo_in(&w.src(), &bob, None, Some("tools"), None)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)), "{err}");
    }

    /// A token pinned to workspace A cannot reach B's repo through resolution —
    /// not by name, not by raw id, not by filtering to B.
    #[tokio::test]
    async fn a_workspace_pinned_token_cannot_resolve_outside_its_pin() {
        let w = world().await;
        let pinned = auth_for(
            user("owner", false),
            Some(McpScope {
                tools: None,
                allow_writes: true,
                workspace_id: Some(w.ws_a.clone()),
            }),
        );
        for reference in ["games", w.repo_b.as_str()] {
            let err = resolve_repo_in(&w.src(), &pinned, None, Some(reference), None)
                .await
                .unwrap_err();
            assert!(matches!(err, Error::NotFound(_)), "{reference}: {err}");
            assert!(!listed(&err.to_string(), w.repo_b.as_str()), "{err}");
        }
        let err = resolve_repo_in(&w.src(), &pinned, None, Some("games"), Some(&w.ws_b))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)), "{err}");
        // Inside the pin it still resolves.
        let r = resolve_repo_in(&w.src(), &pinned, None, Some("tools"), None)
            .await
            .unwrap();
        assert_eq!(r.entry.repo.id, w.repo_a);
    }

    #[test]
    fn listing_puts_the_current_workspace_first() {
        let mut es = fixture();
        sort_for_listing(&mut es, Some("ws-b"));
        let first_a = es.iter().position(|e| e.repo.workspace_id == "ws-a").unwrap();
        assert!(es[..first_a].iter().all(|e| e.repo.workspace_id == "ws-b"));
        assert_eq!(es[0].repo.name, "games_management");
        let row = es[0].to_json(Some("ws-b"));
        assert_eq!(row["workspace_name"], "ws-b-name");
        assert_eq!(row["current"], true);
    }
}
