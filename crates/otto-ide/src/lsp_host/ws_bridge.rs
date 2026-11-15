//! Shared HTTP/WebSocket bridge helpers for `/lsp/capabilities`,
//! `/workspaces/{id}/lsp/install`, `/ws/lsp`, and workspace-root RBAC.
//!
//! Keeps `otto-server`'s `lsp/mod.rs` a thin Axum router/WebSocket shell while
//! testing and enforcing path canonicalization, secret-path denial, Go build-tag
//! wiring, and attachment framing inside `otto-ide`.

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::http::StatusCode;
use otto_core::domain::User;
use otto_core::Error;
use serde::{Deserialize, Serialize};

use super::pool::{Attached, LspPool, PoolKey, SpawnLaunch};
use super::servers;
use crate::{IdeCtx, IdeEngine};

/// One entry in the capabilities response (`GET /api/v1/lsp/capabilities`).
#[derive(Debug, Clone, Serialize)]
pub struct LspServerStatus {
    pub lang: String,
    pub available: bool,
    /// Resolved executable path, or the primary name when unavailable.
    pub command: String,
    /// Shell command the UI can run to install this server; null when unknown.
    pub install_command: Option<String>,
}

/// `GET /api/v1/lsp/capabilities` response body.
#[derive(Debug, Clone, Serialize)]
pub struct LspCapabilities {
    pub servers: Vec<LspServerStatus>,
}

/// Request body for `POST /api/v1/workspaces/{id}/lsp/install`.
#[derive(Debug, Deserialize)]
pub struct InstallLspReq {
    /// Which languages to install. When absent → install all unavailable ones
    /// that have an `install_command`.
    #[serde(default)]
    pub langs: Option<Vec<String>>,
}

/// Query parameters for `GET /ws/lsp`.
#[derive(Debug, Default, Deserialize)]
pub struct LspWsQuery {
    pub lang: Option<String>,
    pub root: Option<String>,
    /// Optional open file path so multi-repo workspaces attach to the file's own module root.
    #[serde(default)]
    pub file: Option<String>,
    /// Optional SSH connection ID when editing a Remote SSH workspace.
    #[serde(default)]
    pub connection_id: Option<String>,
}

/// Validated, authorized `/ws/lsp` target ready for WebSocket upgrade.
#[derive(Debug, Clone)]
pub struct PreparedLspSession {
    pub key: PoolKey,
    pub launch: SpawnLaunch,
    pub file_hint: Option<String>,
}

/// Pre-upgrade HTTP problem returned by [`prepare_lsp_ws_session`].
#[derive(Debug, Clone)]
pub struct LspWsError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl LspWsError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "server_unavailable",
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal",
            message: message.into(),
        }
    }
}

/// Detect installed LSP servers off the async runtime (backed by a 60 s TTL cache).
pub async fn detect_lsp_capabilities(ide: &Arc<IdeEngine>) -> otto_core::Result<LspCapabilities> {
    let data_dir = ide.data_dir.clone();
    let plugins = ide.plugins.clone();
    let resolved =
        tokio::task::spawn_blocking(move || servers::detect_all_with_ide(&data_dir, &plugins))
            .await
            .map_err(|e| Error::Internal(format!("lsp capabilities probe failed: {e}")))?;
    let servers = resolved
        .into_iter()
        .map(|r| LspServerStatus {
            lang: r.lang,
            available: r.available,
            command: r.command,
            install_command: r.install_command,
        })
        .collect();
    Ok(LspCapabilities { servers })
}

/// Validate requested languages and build the deduplicated shell install command.
pub async fn plan_lsp_install_compound(
    ide: &Arc<IdeEngine>,
    req_langs: Option<Vec<String>>,
) -> otto_core::Result<String> {
    let data_dir = ide.data_dir.clone();
    let plugins = ide.plugins.clone();
    let resolved =
        tokio::task::spawn_blocking(move || servers::detect_all_with_ide(&data_dir, &plugins))
            .await
            .map_err(|e| Error::Internal(format!("lsp install probe failed: {e}")))?;

    let pairs: Vec<String> = match req_langs {
        Some(ref langs) => {
            let mut cmds = Vec::new();
            for lang in langs {
                let norm = servers::normalize_lsp_lang(lang);
                let entry = resolved.iter().find(|r| r.lang == norm);
                match entry {
                    None => {
                        return Err(Error::Invalid(format!("unknown language: '{lang}'")));
                    }
                    Some(r) => match &r.install_command {
                        None => {
                            return Err(Error::Invalid(format!(
                                "no install command for '{lang}' (manual install required)"
                            )));
                        }
                        Some(cmd) => cmds.push(cmd.clone()),
                    },
                }
            }
            cmds
        }
        None => resolved
            .iter()
            .filter(|r| !r.available)
            .filter_map(|r| r.install_command.clone())
            .collect(),
    };

    if pairs.is_empty() {
        return Err(Error::Invalid(
            "no languages need installation (all available, or no install commands)".into(),
        ));
    }

    let mut seen = HashSet::new();
    let unique: Vec<String> = pairs
        .into_iter()
        .filter(|cmd| seen.insert(cmd.clone()))
        .collect();
    Ok(unique.join("; echo; "))
}

/// Validate query parameters, authorize SSH or local workspace roots, sync
/// effective Go build tags for `go`, and prepare the [`PoolKey`] + [`SpawnLaunch`].
pub async fn prepare_lsp_ws_session<C, F, Fut>(
    ctx: &C,
    user: &User,
    q: LspWsQuery,
    local_root_ok: F,
) -> std::result::Result<PreparedLspSession, LspWsError>
where
    C: IdeCtx,
    F: FnOnce(PathBuf) -> Fut,
    Fut: Future<Output = otto_core::Result<bool>>,
{
    let ide = ctx.ide();
    ide.lsp_pool.record_owner_privilege(&user.id, user.is_root);

    let Some(lang) = q.lang.filter(|l| !l.is_empty()) else {
        return Err(LspWsError::bad_request("missing ?lang= parameter"));
    };
    let Some(root) = q.root.filter(|r| !r.is_empty()) else {
        return Err(LspWsError::bad_request("missing ?root= parameter"));
    };
    if root.chars().any(char::is_control) {
        return Err(LspWsError::bad_request(
            "root contains invalid control characters",
        ));
    }
    let file_hint = q
        .file
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(str::to_owned);
    if file_hint
        .as_deref()
        .is_some_and(|f| f.chars().any(char::is_control))
    {
        return Err(LspWsError::bad_request(
            "file contains invalid control characters",
        ));
    }

    let ssh_conn_id = q
        .connection_id
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());

    let db_pool = ctx.pool();
    ide.environments.ensure_hydrated(&db_pool).await;
    let _ = ide.projects.ensure_hydrated(&db_pool).await;

    if let Some(conn_id) = ssh_conn_id {
        if !root.starts_with('/') {
            return Err(LspWsError::bad_request(
                "remote SSH root must be an absolute path",
            ));
        }
        let ssh_spec = match ctx.resolve_ssh_connection(user, &conn_id).await {
            Ok(spec) => spec,
            Err(Error::NotFound(m)) => return Err(LspWsError::not_found(m)),
            Err(Error::Forbidden(m)) => return Err(LspWsError::forbidden(m)),
            Err(Error::Invalid(m)) => return Err(LspWsError::bad_request(m)),
            Err(e) => {
                tracing::warn!(connection_id = %conn_id, "lsp ssh lookup failed: {e}");
                return Err(LspWsError::internal("SSH connection lookup failed"));
            }
        };
        let plugins = ide.plugins.clone();
        let lang_cl = lang.clone();
        let remote =
            tokio::task::spawn_blocking(move || servers::resolve_remote_lang(&lang_cl, &plugins))
                .await
                .map_err(|_| LspWsError::internal("remote language lookup failed"))?;
        let Some(remote) = remote else {
            return Err(LspWsError::bad_request(format!(
                "unknown language: '{lang}'"
            )));
        };
        let trimmed = root.trim_end_matches('/');
        let norm_root = if trimmed.is_empty() { "/" } else { trimmed }.to_string();
        return Ok(PreparedLspSession {
            key: PoolKey {
                owner: user.id.to_string(),
                lang: remote.lang,
                root: norm_root,
                ssh: Some(ssh_spec),
            },
            launch: SpawnLaunch {
                cmd: remote.primary_cmd,
                args: remote.primary_args,
                fallback_cmd: remote.fallback_cmd,
                fallback_args: remote.fallback_args,
            },
            file_hint,
        });
    }

    let resolved_root = {
        let lang_cl = lang.clone();
        let root_cl = root.clone();
        let fh_cl = file_hint.clone();
        tokio::task::spawn_blocking(move || {
            resolve_local_lsp_module_root(&lang_cl, &root_cl, fh_cl)
        })
        .await
        .map_err(|_| LspWsError::internal("local module root resolution failed"))??
    };

    if !user.is_root {
        if let Some(ref fh) = file_hint {
            let fh_cl = PathBuf::from(fh);
            let secret_why = tokio::task::spawn_blocking(move || {
                secret_root_denial(&canonical_or_ancestor(&fh_cl))
            })
            .await
            .map_err(|_| LspWsError::internal("file secret check failed"))?;
            if let Some(why) = secret_why {
                return Err(LspWsError::forbidden(why));
            }
        }
        match local_root_ok(PathBuf::from(&resolved_root)).await {
            Ok(true) => {}
            Ok(false) => {
                let root_cl = PathBuf::from(&resolved_root);
                let secret_why = tokio::task::spawn_blocking(move || {
                    secret_root_denial(&canonical_or_ancestor(&root_cl))
                })
                .await
                .ok()
                .flatten();
                let msg = secret_why
                    .unwrap_or_else(|| "root is not inside a workspace you can access".to_string());
                return Err(LspWsError::forbidden(msg));
            }
            Err(e) => {
                tracing::warn!("lsp: workspace lookup failed: {e}");
                return Err(LspWsError::internal("workspace lookup failed"));
            }
        }
    }

    let resolved = {
        let lang_cl = lang.clone();
        let data_dir = ide.data_dir.clone();
        let plugins = ide.plugins.clone();
        tokio::task::spawn_blocking(move || {
            servers::resolve_lang_with_ide(&lang_cl, &data_dir, &plugins)
        })
        .await
        .map_err(|_| LspWsError::internal("language server lookup failed"))?
    };
    let Some(resolved) = resolved else {
        return Err(LspWsError::bad_request(format!(
            "unknown language: '{lang}'"
        )));
    };
    if !resolved.available {
        return Err(LspWsError::unavailable(format!(
            "no language server found for '{lang}' ('{}')",
            resolved.command
        )));
    }

    if resolved.lang == "go" {
        let eff_tags = ide
            .environments
            .effective_go_build_tags_for_local_root(&resolved_root);
        ide.lsp_pool.set_go_build_tags(&resolved_root, eff_tags);
    }

    Ok(PreparedLspSession {
        key: PoolKey {
            owner: user.id.to_string(),
            lang: resolved.lang,
            root: resolved_root,
            ssh: None,
        },
        launch: SpawnLaunch::primary(resolved.command, resolved.args),
        file_hint,
    })
}

/// Attach a prepared `/ws/lsp` session on `spawn_blocking` and build the initial
/// `otto/lspAttached` JSON-RPC notification payload (`{ root, lang }`).
pub async fn attach_lsp_ws_session(
    pool: Arc<LspPool>,
    session: PreparedLspSession,
) -> std::result::Result<(Attached, String), Option<String>> {
    let PreparedLspSession {
        key,
        launch,
        file_hint,
    } = session;
    let key_lang = key.lang.clone();
    let launch_cmd = launch.cmd.clone();
    let attach_res = tokio::task::spawn_blocking(move || {
        pool.attach_launch_with_file(key, &launch, file_hint.as_deref())
    })
    .await;

    match attach_res {
        Ok(Ok(attached)) => {
            let notice = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "otto/lspAttached",
                "params": {
                    "root": attached.root,
                    "lang": attached.lang,
                }
            })
            .to_string();
            Ok((attached, notice))
        }
        Ok(Err(e)) => {
            tracing::error!(
                lang = key_lang.as_str(),
                cmd = launch_cmd.as_str(),
                "failed to spawn language server: {e}"
            );
            let err = serde_json::json!({
                "type": "error",
                "code": "spawn_failed",
                "message": e.to_string(),
            })
            .to_string();
            Err(Some(err))
        }
        Err(join_err) => {
            tracing::error!(
                lang = key_lang.as_str(),
                "lsp attach join error: {join_err}"
            );
            Err(None)
        }
    }
}

#[allow(clippy::disallowed_methods)]
fn resolve_local_lsp_module_root(
    lang: &str,
    root: &str,
    file_hint: Option<String>,
) -> std::result::Result<String, LspWsError> {
    let root_path = Path::new(root);
    if !root_path.exists() || !root_path.is_dir() {
        return Err(LspWsError::bad_request(format!(
            "root does not exist or is not a directory: {root}"
        )));
    }
    servers::resolve_lsp_module_root(lang, root, file_hint.as_deref(), &[])
        .map_err(|e| LspWsError::bad_request(e.to_string()))
}

#[allow(clippy::disallowed_methods)]
pub fn canonical_or_ancestor(p: &Path) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(p) {
        return c;
    }
    let mut cur = p;
    let mut tail = Vec::new();
    while let Some(parent) = cur.parent() {
        if let Some(name) = cur.file_name() {
            tail.push(name.to_os_string());
        }
        if let Ok(mut base) = std::fs::canonicalize(parent) {
            for comp in tail.into_iter().rev() {
                base.push(comp);
            }
            return base;
        }
        cur = parent;
    }
    p.to_path_buf()
}

pub fn secret_root_denial(canonical: &Path) -> Option<String> {
    use otto_core::secret_paths as sp;
    (sp::in_protected_dir(canonical) || sp::is_denied_file(canonical)).then(|| {
        format!(
            "{} holds credentials or Otto's own state; only the root user can open it",
            canonical.display()
        )
    })
}

#[allow(clippy::disallowed_methods)]
pub fn canonical_of(p: &str) -> Option<PathBuf> {
    if p.trim().is_empty() {
        return None;
    }
    let p = Path::new(p);
    Some(std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
}

pub fn canonical_eq(root: &Path, other: &str) -> bool {
    canonical_of(other).is_some_and(|c| root == c)
}

pub fn root_under_any(root: &Path, workspace_roots: &[String]) -> bool {
    workspace_roots
        .iter()
        .filter_map(|w| canonical_of(w))
        .any(|c| root.starts_with(&c))
}

#[cfg(test)]
mod root_tests {
    use super::*;

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn lsp_root_must_sit_inside_a_member_workspace() {
        let d = tempfile::tempdir().unwrap();
        let ws = d.path().join("repo");
        let other = d.path().join("elsewhere");
        std::fs::create_dir_all(ws.join("sub")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap();
        let roots = vec![ws.to_string_lossy().into_owned(), String::new()];
        assert!(root_under_any(&canon(&ws), &roots));
        assert!(root_under_any(&canon(&ws.join("sub")), &roots));
        assert!(!root_under_any(&canon(&other), &roots));
        assert!(
            !root_under_any(&canon(d.path()), &roots),
            "a parent is not inside"
        );
        let evil = d.path().join("repo-evil");
        std::fs::create_dir_all(&evil).unwrap();
        assert!(!root_under_any(&canon(&evil), &roots));
        let repo = d.path().join("outside-repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let with_repo = vec![
            ws.to_string_lossy().into_owned(),
            repo.to_string_lossy().into_owned(),
        ];
        assert!(!root_under_any(&canon(&repo), &roots));
        assert!(root_under_any(&canon(&repo), &with_repo));
        assert!(root_under_any(&canon(&repo.join("src")), &with_repo));
        assert!(!root_under_any(&canon(&other), &with_repo));
        let home = d.path().to_string_lossy().into_owned();
        assert!(canonical_eq(&canon(d.path()), &home));
        assert!(!canonical_eq(&canon(&other), &home));
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn lsp_root_keeps_the_fs_secret_deny() {
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        let home = PathBuf::from(home);
        assert!(secret_root_denial(&home.join(".ssh")).is_some());
        assert!(secret_root_denial(&home.join(".ssh/keys")).is_some());
        let d = tempfile::tempdir().unwrap();
        let plain = std::fs::canonicalize(d.path()).unwrap();
        assert!(secret_root_denial(&plain).is_none());
    }
}
