//! Shared HTTP/WebSocket bridge helpers for `/lsp/capabilities`,
//! `/workspaces/{id}/lsp/install`, `/ws/lsp`, and workspace-root RBAC.
//!
//! Keeps `otto-server`'s `lsp/mod.rs` a thin Axum router/WebSocket shell while
//! testing and enforcing path canonicalization, secret-path denial, Go build-tag
//! wiring, and attachment framing inside `otto-ide`.

use std::collections::HashSet;
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

/// Project environment key opting a Zig project into automatic ZLS startup
/// (ZLS evaluates `build.zig` during source discovery).
pub const TRUST_ZIG_LSP_ENV: &str = "OTTO_IDE_TRUST_ZIG_BUILD";
/// Project environment key opting a Kotlin project into automatic `kotlin-language-server`
/// startup (KLS invokes Gradle/Maven build scripts during workspace import).
pub const TRUST_KOTLIN_LSP_ENV: &str = "OTTO_IDE_TRUST_KOTLIN_LSP";
/// Shared project environment key opting a project into trusted build-tool execution.
pub const TRUST_PROJECT_EXECUTION_ENV: &str = "OTTO_IDE_TRUST_PROJECT_EXECUTION";

fn lsp_execution_trusted_for_key(
    envs: &crate::environments::EnvironmentManager,
    root: &str,
    boundary_root: Option<&str>,
    ssh_conn_id: Option<&str>,
    lang_key: &str,
) -> bool {
    let trimmed = root.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return false;
    }
    let boundary = boundary_root
        .map(|b| b.trim().trim_end_matches('/'))
        .filter(|b| !b.is_empty())
        .map(Path::new);
    let has_opt_in = |env: &crate::types::IdeProjectEnvironment| {
        env.env_vars.iter().any(|v| {
            !v.secret
                && (v.key == lang_key || v.key == TRUST_PROJECT_EXECUTION_ENV)
                && v.value.as_deref().is_some_and(|val| val.trim() == "1")
        })
    };
    let mut cur = Path::new(trimmed);
    for _ in 0..8 {
        if let Some(b) = boundary {
            if !cur.starts_with(b) {
                break;
            }
        }
        let cur_s = cur.to_string_lossy();
        let lookup_key = match ssh_conn_id {
            Some(cid) => format!("ssh:{cid}:{cur_s}"),
            None => cur_s.into_owned(),
        };
        if envs
            .cached_explicit_project_env(&lookup_key)
            .is_some_and(|env| has_opt_in(&env))
        {
            return true;
        }
        if boundary.is_some_and(|b| cur == b) {
            break;
        }
        let Some(parent) = cur.parent() else {
            break;
        };
        if parent == cur || parent == Path::new("/") {
            break;
        }
        cur = parent;
    }
    false
}

/// Returns `true` only when the opened project root (or a sub-module inside it, NEVER
/// an ancestor directory outside `opened_project_root`) explicitly opted in to Zig build
/// execution via a non-secret `OTTO_IDE_TRUST_ZIG_BUILD=1` or
/// `OTTO_IDE_TRUST_PROJECT_EXECUTION=1` entry in `IdeProjectEnvironment::env_vars`.
pub fn zig_lsp_execution_trusted_for_project(
    envs: &crate::environments::EnvironmentManager,
    resolved_root: &str,
    opened_project_root: &str,
    ssh_conn_id: Option<&str>,
) -> bool {
    lsp_execution_trusted_for_key(
        envs,
        resolved_root,
        Some(opened_project_root),
        ssh_conn_id,
        TRUST_ZIG_LSP_ENV,
    )
}

/// Convenience wrapper checking exact project trust on `root` (does not inherit from
/// filesystem ancestors above `root`).
pub fn zig_lsp_execution_trusted(
    envs: &crate::environments::EnvironmentManager,
    root: &str,
    ssh_conn_id: Option<&str>,
) -> bool {
    zig_lsp_execution_trusted_for_project(envs, root, root, ssh_conn_id)
}

/// Returns `true` only when the opened project root (or a sub-module inside it, NEVER
/// an ancestor directory outside `opened_project_root`) explicitly opted in to Kotlin
/// LSP build execution via a non-secret `OTTO_IDE_TRUST_KOTLIN_LSP=1` or
/// `OTTO_IDE_TRUST_PROJECT_EXECUTION=1` entry in `IdeProjectEnvironment::env_vars`.
pub fn kotlin_lsp_execution_trusted_for_project(
    envs: &crate::environments::EnvironmentManager,
    resolved_root: &str,
    opened_project_root: &str,
    ssh_conn_id: Option<&str>,
) -> bool {
    lsp_execution_trusted_for_key(
        envs,
        resolved_root,
        Some(opened_project_root),
        ssh_conn_id,
        TRUST_KOTLIN_LSP_ENV,
    )
}

/// Convenience wrapper checking exact project trust on `root` (does not inherit from
/// filesystem ancestors above `root`).
pub fn kotlin_lsp_execution_trusted(
    envs: &crate::environments::EnvironmentManager,
    root: &str,
    ssh_conn_id: Option<&str>,
) -> bool {
    kotlin_lsp_execution_trusted_for_project(envs, root, root, ssh_conn_id)
}

/// Validate query parameters, authorize SSH or local workspace roots, sync
/// effective Go build tags for `go`, and prepare the [`PoolKey`] + [`SpawnLaunch`].
pub async fn prepare_lsp_ws_session<C: IdeCtx>(
    ctx: &C,
    user: &User,
    q: LspWsQuery,
) -> std::result::Result<PreparedLspSession, LspWsError> {
    let has_root_auth = otto_core::auth::root_authority(user);
    let ide = ctx.ide();
    ide.lsp_pool.record_owner_privilege(&user.id, has_root_auth);

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
        if remote.lang == "zig"
            && !zig_lsp_execution_trusted_for_project(
                &ide.environments,
                &norm_root,
                &norm_root,
                Some(&conn_id),
            )
        {
            return Err(LspWsError {
                status: StatusCode::FORBIDDEN,
                code: "zig_lsp_requires_trust",
                message: format!(
                    "ZLS evaluates build.zig during discovery; set {TRUST_ZIG_LSP_ENV}=1 in project environment settings to start ZLS"
                ),
            });
        }
        if remote.lang == "kotlin"
            && !kotlin_lsp_execution_trusted(&ide.environments, &norm_root, Some(&conn_id))
        {
            return Err(LspWsError {
                status: StatusCode::FORBIDDEN,
                code: "kotlin_lsp_requires_trust",
                message: format!(
                    "kotlin-language-server runs Gradle/Maven build scripts during workspace import; set {TRUST_KOTLIN_LSP_ENV}=1 (or {TRUST_PROJECT_EXECUTION_ENV}=1) in project environment settings to start it"
                ),
            });
        }
        let remote_zig_home = if remote.lang == "zig" {
            ide.environments
                .cached_project_env(&format!("ssh:{conn_id}:{norm_root}"))
                .and_then(|e| e.zig_home)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        } else {
            None
        };
        return Ok(PreparedLspSession {
            key: PoolKey {
                owner: user.id.to_string(),
                lang: remote.lang,
                root: norm_root.clone(),
                ssh: Some(ssh_spec),
                go_build_tags: Vec::new(),
            },
            launch: SpawnLaunch {
                cmd: remote.primary_cmd,
                args: remote.primary_args,
                fallback_cmd: remote.fallback_cmd,
                fallback_args: remote.fallback_args,
                project_root: Some(norm_root),
                zig_home: remote_zig_home,
            },
            file_hint,
        });
    }

    // A local language server runs unsandboxed as the daemon's user: root
    // only. Remote SSH targets (above) run on the remote host.
    if !has_root_auth {
        return Err(LspWsError {
            status: StatusCode::FORBIDDEN,
            code: "lsp_requires_root",
            message: "only a root account can open a local language server".into(),
        });
    }
    let expanded_root_buf =
        otto_core::paths::expand_home_strict(&root).map_err(LspWsError::bad_request)?;
    let expanded_root = expanded_root_buf.to_string_lossy().into_owned();
    let file_hint = match file_hint {
        Some(fh) => {
            let p = otto_core::paths::expand_home_strict(&fh).map_err(LspWsError::bad_request)?;
            Some(p.to_string_lossy().into_owned())
        }
        None => None,
    };

    let resolved_root = {
        let lang_cl = lang.clone();
        let root_cl = expanded_root.clone();
        let fh_cl = file_hint.clone();
        let plugins_cl = ide.plugins.clone();
        tokio::task::spawn_blocking(move || {
            resolve_local_lsp_module_root(&lang_cl, &root_cl, fh_cl, &plugins_cl)
        })
        .await
        .map_err(|_| LspWsError::internal("local module root resolution failed"))??
    };

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
    #[allow(clippy::disallowed_methods)]
    let opened_root = std::fs::canonicalize(&expanded_root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| expanded_root.clone());
    if resolved.lang == "zig"
        && !zig_lsp_execution_trusted_for_project(
            &ide.environments,
            &resolved_root,
            &opened_root,
            None,
        )
    {
        return Err(LspWsError {
            status: StatusCode::FORBIDDEN,
            code: "zig_lsp_requires_trust",
            message: format!(
                "ZLS evaluates build.zig during discovery; set {TRUST_ZIG_LSP_ENV}=1 in project environment settings to start ZLS"
            ),
        });
    }
    if resolved.lang == "kotlin"
        && !kotlin_lsp_execution_trusted_for_project(
            &ide.environments,
            &resolved_root,
            &opened_root,
            None,
        )
    {
        return Err(LspWsError {
            status: StatusCode::FORBIDDEN,
            code: "kotlin_lsp_requires_trust",
            message: format!(
                "kotlin-language-server runs Gradle/Maven build scripts during workspace import; set {TRUST_KOTLIN_LSP_ENV}=1 (or {TRUST_PROJECT_EXECUTION_ENV}=1) in project environment settings to start it"
            ),
        });
    }

    let mut launch_cmd = resolved.command;
    if resolved.lang == "zig" {
        if let Some(zig_home) = ide
            .environments
            .cached_project_env_for_module(&resolved_root, Some(&opened_root))
            .and_then(|e| e.zig_home)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            let data_dir = ide.data_dir.clone();
            if let Ok(Some(matched_zls)) = tokio::task::spawn_blocking(move || {
                let zig_ver = crate::toolchains::discovery::zig_bin_in_home(Path::new(&zig_home))
                    .and_then(|b| crate::toolchains::discovery::read_zig_bin_version(&b));
                let mut tc_roots = vec![data_dir.join("toolchains")];
                if !otto_core::e2e::isolated() {
                    if let Some(dot_otto) = dirs::home_dir().map(|h| h.join(".otto/toolchains")) {
                        if !tc_roots.contains(&dot_otto) {
                            tc_roots.push(dot_otto);
                        }
                    }
                }
                crate::toolchains::discovery::resolve_managed_zls_for_zig_version(
                    &tc_roots,
                    zig_ver.as_deref(),
                )
            })
            .await
            {
                launch_cmd = matched_zls.to_string_lossy().into_owned();
            }
        }
    }

    let canon_go_tags = if resolved.lang == "go" {
        let eff_tags = ide
            .environments
            .effective_go_build_tags_for_local_root(&resolved_root);
        ide.lsp_pool
            .set_go_build_tags(&resolved_root, eff_tags.clone());
        crate::go_build_tags::canonicalize_go_build_tags(&eff_tags)
    } else {
        Vec::new()
    };

    let mut launch = SpawnLaunch::primary(launch_cmd, resolved.args);
    launch.project_root = Some(opened_root);
    Ok(PreparedLspSession {
        key: PoolKey {
            owner: user.id.to_string(),
            lang: resolved.lang,
            root: resolved_root,
            ssh: None,
            go_build_tags: canon_go_tags,
        },
        launch,
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
    plugins: &crate::plugins::IdePluginRegistry,
) -> std::result::Result<String, LspWsError> {
    let expanded = PathBuf::from(otto_core::paths::expand_tilde(root));
    if !expanded.exists() || !expanded.is_dir() {
        return Err(LspWsError::bad_request(format!(
            "root does not exist or is not a directory: {root}"
        )));
    }
    let norm = servers::normalize_lsp_lang(&lang.trim().to_ascii_lowercase()).to_string();
    let mut extra_markers = Vec::new();
    for ls in plugins.active_languages() {
        if servers::normalize_lsp_lang(&ls.id.trim().to_ascii_lowercase()) == norm {
            extra_markers.extend(ls.root_markers);
        }
    }
    for sp in plugins.active_lsp_servers() {
        if servers::normalize_lsp_lang(&sp.language.trim().to_ascii_lowercase()) == norm {
            extra_markers.extend(sp.root_markers);
        }
    }
    let expanded_str = expanded.to_string_lossy();
    servers::resolve_lsp_module_root_with_markers(
        lang,
        &expanded_str,
        file_hint.as_deref(),
        &[],
        &extra_markers,
    )
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

    #[test]
    fn zig_lsp_requires_explicit_project_execution_trust() {
        let tmp = tempfile::tempdir().unwrap();
        let envs = crate::environments::EnvironmentManager::new(tmp.path().to_path_buf());
        let root = tmp.path().join("zig-repo");
        let nested = root.join("sub-pkg");
        let root_s = root.to_string_lossy().into_owned();
        let nested_s = nested.to_string_lossy().into_owned();

        // Untrusted by default (neither root nor nested sub-package may auto-start ZLS).
        assert!(!zig_lsp_execution_trusted(&envs, &root_s, None));
        assert!(!zig_lsp_execution_trusted(&envs, &nested_s, None));

        // Secret or value != "1" must not enable execution trust.
        envs.seed_snapshot(
            &root_s,
            crate::types::IdeProjectEnvironment {
                env_vars: vec![
                    crate::types::IdeEnvVarEntry {
                        key: TRUST_ZIG_LSP_ENV.into(),
                        value: Some("1".into()),
                        secret: true,
                        has_secret: true,
                    },
                    crate::types::IdeEnvVarEntry {
                        key: TRUST_PROJECT_EXECUTION_ENV.into(),
                        value: Some("0".into()),
                        secret: false,
                        has_secret: false,
                    },
                ],
                ..Default::default()
            },
        );
        assert!(!zig_lsp_execution_trusted(&envs, &root_s, None));

        // Explicit non-secret OTTO_IDE_TRUST_ZIG_BUILD=1 approves the opened project and its
        // own sub-modules, but opening the nested directory as an independent project must NOT
        // inherit trust from an ancestor directory outside the opened project (SEC-2).
        envs.seed_snapshot(
            &root_s,
            crate::types::IdeProjectEnvironment {
                env_vars: vec![crate::types::IdeEnvVarEntry {
                    key: TRUST_ZIG_LSP_ENV.into(),
                    value: Some("1".into()),
                    secret: false,
                    has_secret: false,
                }],
                ..Default::default()
            },
        );
        assert!(zig_lsp_execution_trusted(&envs, &root_s, None));
        assert!(zig_lsp_execution_trusted_for_project(
            &envs, &nested_s, &root_s, None
        ));
        assert!(!zig_lsp_execution_trusted(&envs, &nested_s, None));
        assert!(!zig_lsp_execution_trusted(&envs, &root_s, Some("ssh-1")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn prepare_lsp_ws_session_enforces_kotlin_trust_locally_over_ssh_and_blocks_ancestor_inheritance(
    ) {
        use std::os::unix::fs::PermissionsExt;

        use otto_core::auth::{BoxFuture, RoleChecker};
        use otto_core::domain::{User, WorkspaceRole};
        use otto_core::secrets::SecretStore;
        use otto_core::{Id, Result};

        #[derive(Default)]
        struct NoopSecrets;
        impl SecretStore for NoopSecrets {
            fn put(&self, _k: &str, _v: &str) -> Result<()> {
                Ok(())
            }
            fn get(&self, _k: &str) -> Result<Option<String>> {
                Ok(None)
            }
            fn delete(&self, _k: &str) -> Result<()> {
                Ok(())
            }
        }
        struct AllowRoles;
        impl RoleChecker for AllowRoles {
            fn check<'a>(
                &'a self,
                _user: &'a User,
                _ws: &'a Id,
                _min: WorkspaceRole,
            ) -> BoxFuture<'a, Result<()>> {
                Box::pin(async { Ok(()) })
            }
        }
        #[derive(Clone)]
        struct TestIdeCtx {
            ide: Arc<IdeEngine>,
            pool: otto_state::DbPool,
            secrets: Arc<dyn SecretStore>,
            roles: Arc<dyn RoleChecker>,
            data_dir: PathBuf,
        }
        impl IdeCtx for TestIdeCtx {
            fn ide(&self) -> &Arc<IdeEngine> {
                &self.ide
            }
            fn pool(&self) -> otto_state::DbPool {
                self.pool.clone()
            }
            fn secrets(&self) -> &Arc<dyn SecretStore> {
                &self.secrets
            }
            fn data_dir(&self) -> &Path {
                &self.data_dir
            }
            fn roles(&self) -> &Arc<dyn RoleChecker> {
                &self.roles
            }
            fn resolve_ssh_connection<'a>(
                &'a self,
                _user: &'a User,
                connection_id: &'a Id,
            ) -> BoxFuture<'a, Result<crate::IdeSshSpec>> {
                let cid = connection_id.to_string();
                Box::pin(async move {
                    Ok(crate::IdeSshSpec {
                        connection_id: cid,
                        name: "remote".into(),
                        host: "remote.example".into(),
                        port: Some(22),
                        user: "dev".into(),
                        identity_file: None,
                        proxy_jump: None,
                        environment: None,
                    })
                })
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        // Provision a fake managed kotlin-language-server so `resolve_lang_with_ide` reports available=true.
        let fake_kls = data_dir.join("toolchains/kotlin/bin/kotlin-language-server");
        std::fs::create_dir_all(fake_kls.parent().unwrap()).unwrap();
        std::fs::write(&fake_kls, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&fake_kls, std::fs::Permissions::from_mode(0o755)).unwrap();

        let parent_repo = tmp.path().join("trusted-parent");
        let sub_mod = parent_repo.join("sub-mod");
        let untrusted_child = parent_repo.join("untrusted-checkout");
        std::fs::create_dir_all(sub_mod.join("src/main/kotlin")).unwrap();
        std::fs::create_dir_all(untrusted_child.join("src/main/kotlin")).unwrap();
        std::fs::write(
            parent_repo.join("settings.gradle.kts"),
            "include(\":sub-mod\")\n",
        )
        .unwrap();
        std::fs::write(
            sub_mod.join("build.gradle.kts"),
            "plugins { kotlin(\"jvm\") }\n",
        )
        .unwrap();
        let sub_kt = sub_mod.join("src/main/kotlin/App.kt");
        std::fs::write(&sub_kt, "fun main() {}\n").unwrap();
        std::fs::write(
            untrusted_child.join("build.gradle.kts"),
            "plugins { kotlin(\"jvm\") }\n",
        )
        .unwrap();

        let canon_parent = std::fs::canonicalize(&parent_repo).unwrap();
        let canon_child = std::fs::canonicalize(&untrusted_child).unwrap();
        let canon_sub_kt = std::fs::canonicalize(&sub_kt).unwrap();

        let pool = otto_state::db::open(&tmp.path().join("ide.db"))
            .await
            .unwrap();
        let ide = Arc::new(IdeEngine::new(data_dir.clone()));
        let ctx = TestIdeCtx {
            ide: Arc::clone(&ide),
            pool,
            secrets: Arc::new(NoopSecrets),
            roles: Arc::new(AllowRoles),
            data_dir,
        };
        let root_user = User {
            id: "u-root".into(),
            username: "root".into(),
            display_name: "Root".into(),
            is_root: true,
            disabled: false,
            created_at: chrono::Utc::now(),
        };

        // 1. Untrusted local Kotlin project is denied with 403 kotlin_lsp_requires_trust.
        let err = prepare_lsp_ws_session(
            &ctx,
            &root_user,
            LspWsQuery {
                lang: Some("kotlin".into()),
                root: Some(canon_parent.to_string_lossy().into_owned()),
                file: None,
                connection_id: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::FORBIDDEN);
        assert_eq!(err.code, "kotlin_lsp_requires_trust");

        // 2. Trusting `canon_parent` — EVEN WHEN `canon_parent` configures `module_sdks`
        //    and `module_go_build_tags` targeting `canon_child` (which synthesizes a child
        //    snapshot in `EnvironmentManager`) — allows opening `canon_parent` (including its
        //    own `sub-mod`), but MUST NOT trust an independent checkout opened at `canon_child`.
        ide.environments.seed_snapshot(
            &format!("local:{}", canon_parent.to_string_lossy()),
            crate::types::IdeProjectEnvironment {
                env_vars: vec![crate::types::IdeEnvVarEntry {
                    key: TRUST_KOTLIN_LSP_ENV.into(),
                    value: Some("1".into()),
                    secret: false,
                    has_secret: false,
                }],
                module_sdks: std::collections::BTreeMap::from([(
                    canon_child.to_string_lossy().into_owned(),
                    "/opt/jdk-21".into(),
                )]),
                module_go_build_tags: std::collections::BTreeMap::from([(
                    canon_child.to_string_lossy().into_owned(),
                    vec!["e2e".into()],
                )]),
                ..Default::default()
            },
        );
        let ok_parent = prepare_lsp_ws_session(
            &ctx,
            &root_user,
            LspWsQuery {
                lang: Some("kotlin".into()),
                root: Some(canon_parent.to_string_lossy().into_owned()),
                file: Some(canon_sub_kt.to_string_lossy().into_owned()),
                connection_id: None,
            },
        )
        .await;
        assert!(
            ok_parent.is_ok(),
            "trusted project sub-module should be allowed: {ok_parent:?}"
        );

        let err_nested = prepare_lsp_ws_session(
            &ctx,
            &root_user,
            LspWsQuery {
                lang: Some("kotlin".into()),
                root: Some(canon_child.to_string_lossy().into_owned()),
                file: None,
                connection_id: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err_nested.status, StatusCode::FORBIDDEN);
        assert_eq!(
            err_nested.code, "kotlin_lsp_requires_trust",
            "independent nested checkout must never inherit trust from ancestor directory"
        );

        // 3. Remote SSH Kotlin workspace also requires explicit trust on its ssh key.
        let err_ssh = prepare_lsp_ws_session(
            &ctx,
            &root_user,
            LspWsQuery {
                lang: Some("kotlin".into()),
                root: Some("/srv/kt-remote".into()),
                file: None,
                connection_id: Some("ssh-1".into()),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err_ssh.status, StatusCode::FORBIDDEN);
        assert_eq!(err_ssh.code, "kotlin_lsp_requires_trust");

        ide.environments.seed_snapshot(
            "ssh:ssh-1:/srv/kt-remote",
            crate::types::IdeProjectEnvironment {
                env_vars: vec![crate::types::IdeEnvVarEntry {
                    key: TRUST_PROJECT_EXECUTION_ENV.into(),
                    value: Some("1".into()),
                    secret: false,
                    has_secret: false,
                }],
                ..Default::default()
            },
        );
        let ok_ssh = prepare_lsp_ws_session(
            &ctx,
            &root_user,
            LspWsQuery {
                lang: Some("kotlin".into()),
                root: Some("/srv/kt-remote".into()),
                file: None,
                connection_id: Some("ssh-1".into()),
            },
        )
        .await;
        assert!(
            ok_ssh.is_ok(),
            "trusted SSH Kotlin root should succeed: {ok_ssh:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn prepare_lsp_ws_session_enforces_zig_trust_and_wires_umbrella_and_ssh_zig_slots() {
        use std::os::unix::fs::PermissionsExt;

        use otto_core::auth::{BoxFuture, RoleChecker};
        use otto_core::domain::{User, WorkspaceRole};
        use otto_core::secrets::SecretStore;
        use otto_core::{Id, Result};

        #[derive(Default)]
        struct NoopSecrets;
        impl SecretStore for NoopSecrets {
            fn put(&self, _k: &str, _v: &str) -> Result<()> {
                Ok(())
            }
            fn get(&self, _k: &str) -> Result<Option<String>> {
                Ok(None)
            }
            fn delete(&self, _k: &str) -> Result<()> {
                Ok(())
            }
        }
        struct AllowRoles;
        impl RoleChecker for AllowRoles {
            fn check<'a>(
                &'a self,
                _user: &'a User,
                _ws: &'a Id,
                _min: WorkspaceRole,
            ) -> BoxFuture<'a, Result<()>> {
                Box::pin(async { Ok(()) })
            }
        }
        #[derive(Clone)]
        struct TestIdeCtx {
            ide: Arc<IdeEngine>,
            pool: otto_state::DbPool,
            secrets: Arc<dyn SecretStore>,
            roles: Arc<dyn RoleChecker>,
            data_dir: PathBuf,
        }
        impl IdeCtx for TestIdeCtx {
            fn ide(&self) -> &Arc<IdeEngine> {
                &self.ide
            }
            fn pool(&self) -> otto_state::DbPool {
                self.pool.clone()
            }
            fn secrets(&self) -> &Arc<dyn SecretStore> {
                &self.secrets
            }
            fn data_dir(&self) -> &Path {
                &self.data_dir
            }
            fn roles(&self) -> &Arc<dyn RoleChecker> {
                &self.roles
            }
            fn resolve_ssh_connection<'a>(
                &'a self,
                _user: &'a User,
                connection_id: &'a Id,
            ) -> BoxFuture<'a, Result<crate::IdeSshSpec>> {
                let cid = connection_id.to_string();
                Box::pin(async move {
                    Ok(crate::IdeSshSpec {
                        connection_id: cid,
                        name: "remote".into(),
                        host: "remote.example".into(),
                        port: Some(22),
                        user: "dev".into(),
                        identity_file: None,
                        proxy_jump: None,
                        environment: None,
                    })
                })
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        // Provision fake managed ZLS + versioned 0.13 slot and a pinned Zig 0.13 SDK.
        let fake_zls = data_dir.join("toolchains/zig/bin/zls");
        let zls_13 = data_dir.join("toolchains/zig/zls/0.13/zls");
        let pinned_zig_13 = tmp.path().join("sdks/zig-0.13.0");
        for p in [&fake_zls, &zls_13, &pinned_zig_13.join("zig")] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "#!/bin/sh\necho 0.13.0\n").unwrap();
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let parent_repo = tmp.path().join("trusted-zig-umbrella");
        let sub_mod = parent_repo.join("sub-pkg");
        let independent_nested = parent_repo.join("independent-checkout");
        std::fs::create_dir_all(sub_mod.join("src")).unwrap();
        std::fs::create_dir_all(independent_nested.join("src")).unwrap();
        std::fs::write(parent_repo.join("build.zig"), "").unwrap();
        std::fs::write(sub_mod.join("build.zig"), "").unwrap();
        let sub_main = sub_mod.join("src/main.zig");
        std::fs::write(&sub_main, "pub fn main() void {}\n").unwrap();
        std::fs::write(independent_nested.join("build.zig"), "").unwrap();
        let ind_main = independent_nested.join("src/main.zig");
        std::fs::write(&ind_main, "pub fn main() void {}\n").unwrap();

        let canon_parent = std::fs::canonicalize(&parent_repo).unwrap();
        let canon_ind = std::fs::canonicalize(&independent_nested).unwrap();
        let canon_sub_main = std::fs::canonicalize(&sub_main).unwrap();

        let pool = otto_state::db::open(&tmp.path().join("ide-zig.db"))
            .await
            .unwrap();
        let ide = Arc::new(IdeEngine::new(data_dir.clone()));
        let ctx = TestIdeCtx {
            ide: Arc::clone(&ide),
            pool,
            secrets: Arc::new(NoopSecrets),
            roles: Arc::new(AllowRoles),
            data_dir,
        };
        let root_user = User {
            id: "u-root".into(),
            username: "root".into(),
            display_name: "Root".into(),
            is_root: true,
            disabled: false,
            created_at: chrono::Utc::now(),
        };

        // 1. SEC-2: Untrusted local Zig project is refused BEFORE attach with 403.
        let err_local = prepare_lsp_ws_session(
            &ctx,
            &root_user,
            LspWsQuery {
                lang: Some("zig".into()),
                root: Some(canon_parent.to_string_lossy().into_owned()),
                file: Some(canon_sub_main.to_string_lossy().into_owned()),
                connection_id: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err_local.status, StatusCode::FORBIDDEN);
        assert_eq!(err_local.code, "zig_lsp_requires_trust");

        // 2. SEC-2: Trusting parent + pinning Zig 0.13 + configuring a `module_sdks` entry
        //    for `independent_nested` on parent creates a synthetic child snapshot for
        //    `local:<canon_ind>`. Opening `sub_pkg` AS A MODULE OF `parent` succeeds (and
        //    inherits 0.13 ZLS + project_root), while opening `independent_nested` AS ITS OWN
        //    PROJECT must STILL be denied with 403 (ignoring the synthetic child snapshot)!
        let mut module_sdks = std::collections::BTreeMap::new();
        module_sdks.insert(
            canon_ind.to_string_lossy().into_owned(),
            pinned_zig_13.to_string_lossy().into_owned(),
        );
        ide.environments.seed_snapshot(
            &format!("local:{}", canon_parent.to_string_lossy()),
            crate::types::IdeProjectEnvironment {
                zig_home: Some(pinned_zig_13.to_string_lossy().into_owned()),
                module_sdks,
                env_vars: vec![crate::types::IdeEnvVarEntry {
                    key: TRUST_ZIG_LSP_ENV.into(),
                    value: Some("1".into()),
                    secret: false,
                    has_secret: false,
                }],
                ..Default::default()
            },
        );

        let err_nested = prepare_lsp_ws_session(
            &ctx,
            &root_user,
            LspWsQuery {
                lang: Some("zig".into()),
                root: Some(canon_ind.to_string_lossy().into_owned()),
                file: None,
                connection_id: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err_nested.status, StatusCode::FORBIDDEN);
        assert_eq!(
            err_nested.code, "zig_lsp_requires_trust",
            "independent nested checkout must not inherit trust from parent directory"
        );

        let ok_mod = prepare_lsp_ws_session(
            &ctx,
            &root_user,
            LspWsQuery {
                lang: Some("zig".into()),
                root: Some(canon_parent.to_string_lossy().into_owned()),
                file: Some(canon_sub_main.to_string_lossy().into_owned()),
                connection_id: None,
            },
        )
        .await
        .expect("sub-module inside trusted umbrella must attach");
        assert_eq!(
            ok_mod.launch.project_root.as_deref(),
            Some(canon_parent.to_string_lossy().as_ref())
        );
        assert_eq!(ok_mod.launch.cmd, zls_13.to_string_lossy());

        // 3. SEC-2 + BUG-4: Untrusted SSH Zig workspace is denied with 403; once trusted,
        //    `prepare_lsp_ws_session` forwards the pinned remote `zig_home` to SpawnLaunch.
        let err_ssh = prepare_lsp_ws_session(
            &ctx,
            &root_user,
            LspWsQuery {
                lang: Some("zig".into()),
                root: Some("/srv/zig-remote".into()),
                file: None,
                connection_id: Some("ssh-1".into()),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err_ssh.status, StatusCode::FORBIDDEN);
        assert_eq!(err_ssh.code, "zig_lsp_requires_trust");

        ide.environments.seed_snapshot(
            "ssh:ssh-1:/srv/zig-remote",
            crate::types::IdeProjectEnvironment {
                zig_home: Some("/home/dev/.otto/toolchains/zig/sdks/0.13.0".into()),
                env_vars: vec![crate::types::IdeEnvVarEntry {
                    key: TRUST_ZIG_LSP_ENV.into(),
                    value: Some("1".into()),
                    secret: false,
                    has_secret: false,
                }],
                ..Default::default()
            },
        );
        let ok_ssh = prepare_lsp_ws_session(
            &ctx,
            &root_user,
            LspWsQuery {
                lang: Some("zig".into()),
                root: Some("/srv/zig-remote".into()),
                file: None,
                connection_id: Some("ssh-1".into()),
            },
        )
        .await
        .expect("trusted SSH Zig workspace should succeed");
        assert_eq!(
            ok_ssh.launch.zig_home.as_deref(),
            Some("/home/dev/.otto/toolchains/zig/sdks/0.13.0")
        );
    }
}
