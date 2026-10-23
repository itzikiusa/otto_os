//! `SessionManager::sync_workspace_mcp`: the per-spawn reconcile of the MCP
//! launcher configs Otto manages in a session's cwd (moved out of
//! `manager.rs` unchanged).

use super::*;

impl SessionManager {
    /// Apply everything Otto manages in this session's cwd MCP launcher
    /// configs — the browser opt-in, the workspace's *enabled* user servers,
    /// and the `otto` first-party entry — as ONE reconcile per file (see
    /// `crate::mcp`): stale managed entries are removed, so a disable/delete
    /// finally propagates, and concurrent spawns sharing a cwd can't lose each
    /// other's updates. Also snapshots the server names THIS provider was
    /// actually given into the session meta (`mcp_servers`) — codex's `-c`
    /// overrides, agy's `.agents/mcp_config.json`, grok's `.grok/config.toml`,
    /// `.mcp.json` for everyone else — plus `mcp_servers_error` when the
    /// user-server lookup failed. Best-effort throughout — never blocks the spawn.
    ///
    /// A failed lookup is NOT "nothing enabled": the previously managed user
    /// entries in the shared files are preserved (only Otto's own entries are
    /// reconciled) so one transient fault can't strip every session's servers.
    ///
    /// Returns the codex `-c` overrides for the browser and the user's servers
    /// (`codex_otto`: whether the caller already attached `otto` to codex).
    pub(super) async fn sync_workspace_mcp(
        &self,
        session: &Session,
        otto_tools: Option<crate::mcp::OttoToolsServer>,
        codex_otto: bool,
    ) -> Vec<String> {
        let (user_servers, lookup_error) = match &self.mcp_servers {
            Some(provider) => {
                // The provider is sync (its Db impl blocks on a bridge thread
                // for the query + Keychain reads) — run it on the blocking pool
                // so a spawn/restart never parks a tokio worker.
                let provider = Arc::clone(provider);
                let ws = session.workspace_id.clone();
                let resolved = tokio::task::spawn_blocking(move || provider.enabled_servers(&ws))
                    .await
                    .unwrap_or_else(|e| Err(format!("mcp server lookup task failed: {e}")));
                match resolved {
                    Ok(specs) => (
                        Some(
                            specs
                                .into_iter()
                                .map(|s| crate::mcp::UserMcpServer {
                                    name: s.name,
                                    command: s.command,
                                    args: s.args,
                                    env: s.env,
                                })
                                .collect::<Vec<_>>(),
                        ),
                        None,
                    ),
                    Err(e) => {
                        tracing::warn!(
                            session = %session.id,
                            workspace = %session.workspace_id,
                            "enabled MCP servers unavailable; keeping the previously managed user entries: {e}"
                        );
                        (None, Some(e))
                    }
                }
            }
            None => (Some(Vec::new()), None),
        };
        let session_browser = session
            .meta
            .get("browser")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        // Union semantics for the browser entry: `browser` is per-SESSION meta
        // but the reconcile is per-CWD — without this, a non-browser spawn in a
        // shared cwd would strip `otto-browser` from `.mcp.json` while a
        // concurrent browser session (PR review + a user session, say) is
        // between its own reconcile and its CLI reading the file. Keep the
        // entry as long as ANY live session in this cwd wants it. One query
        // returns the cwds of the live browser sessions (usually none); only
        // those are canonicalized, on the blocking pool below.
        let browser_cwds: Vec<String> = if session_browser {
            Vec::new()
        } else {
            // Snapshot ids first — never hold a DashMap shard ref across await.
            let live_ids: Vec<Id> = self
                .live
                .iter()
                .map(|e| e.key().clone())
                .filter(|id| *id != session.id)
                .collect();
            if live_ids.is_empty() {
                Vec::new()
            } else {
                self.repo
                    .browser_session_cwds(&live_ids)
                    .await
                    .unwrap_or_else(|e| {
                        tracing::warn!(session = %session.id, "browser-session cwd lookup failed: {e}");
                        Vec::new()
                    })
            }
        };
        let has_otto = otto_tools.is_some();
        let cfg = crate::mcp::ManagedMcpConfig {
            browser: session_browser,
            user_servers: user_servers.clone(),
            otto_tools,
        };
        // The launcher reconciles are synchronous file IO under a per-cwd std
        // mutex: run them (and the canonicalize) on ONE blocking-pool thread,
        // so a contended cwd parks that thread, never a tokio worker.
        let (sid, cwd, provider) = (
            session.id.clone(),
            session.cwd.clone(),
            session.provider.clone(),
        );
        let reconciled = tokio::task::spawn_blocking(move || {
            let mut cfg = cfg;
            if !cfg.browser && !browser_cwds.is_empty() {
                let canon = |p: &str| {
                    std::fs::canonicalize(p)
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| p.to_string())
                };
                let this_cwd = canon(&cwd);
                cfg.browser = browser_cwds.iter().any(|c| canon(c) == this_cwd);
            }
            let mcp_json_names = match crate::mcp::reconcile_managed_servers(&cwd, &cfg) {
                Ok(names) => names,
                Err(e) => {
                    tracing::warn!(session = %sid, cwd = %cwd, "reconcile workspace MCP config: {e}");
                    Vec::new()
                }
            };
            let agy_names = (provider == "agy").then(|| {
                // agy never reads `.mcp.json` — same managed set, its own file.
                crate::mcp::reconcile_managed_servers_agy(&cwd, &cfg).unwrap_or_else(|e| {
                    tracing::warn!(session = %sid, cwd = %cwd, "reconcile agy MCP config: {e}");
                    Vec::new()
                })
            });
            let grok = match (&*provider, &cfg.user_servers) {
                ("grok", Some(users)) => Some(crate::mcp::reconcile_user_servers_grok(&cwd, users)),
                _ => None,
            };
            (mcp_json_names, agy_names, grok)
        })
        .await;
        let (mcp_json_names, agy_names, grok_result) = reconciled.unwrap_or_else(|e| {
            tracing::warn!(session = %session.id, "MCP launcher reconcile task failed: {e}");
            (Vec::new(), None, None)
        });
        let mut codex_args = Vec::new();
        let delivered: Vec<String> = match session.provider.as_str() {
            // agy never reads `.mcp.json` — same managed set, its own file.
            "agy" => agy_names.unwrap_or_default(),
            // grok: `otto` via `enable_otto_tools_grok` (at token mint), user
            // tables here; no browser entry. Unknown user set → leave its
            // tables alone (preserved, but not claimed in the snapshot).
            "grok" => {
                let mut names = Vec::new();
                if has_otto {
                    names.push(crate::mcp::OTTO_TOOLS_KEY.to_string());
                }
                if let (Some(users), Some(result)) = (&user_servers, grok_result) {
                    match result {
                        Ok(()) => names.extend(
                            users
                                .iter()
                                .filter(|s| {
                                    s.name != crate::mcp::SERVER_KEY
                                        && s.name != crate::mcp::OTTO_TOOLS_KEY
                                })
                                .map(|s| s.name.clone()),
                        ),
                        Err(e) => {
                            tracing::warn!(session = %session.id, "reconcile grok MCP config: {e}")
                        }
                    }
                }
                names
            }
            // codex doesn't read any launcher file: per-spawn `-c` overrides
            // for the browser (this session's own opt-in, not the cwd union)
            // and the user servers; `otto` was attached by the caller.
            "codex" => {
                let mut names = Vec::new();
                if session_browser {
                    codex_args.extend(crate::mcp::codex_browser_args());
                    names.push(crate::mcp::SERVER_KEY.to_string());
                }
                if codex_otto {
                    names.push(crate::mcp::OTTO_TOOLS_KEY.to_string());
                }
                if let Some(users) = &user_servers {
                    // A server disabled since the last spawn must not leave
                    // its secrets on disk until the session is removed.
                    remove_codex_server_env_files(&session.id).await;
                    let out = crate::mcp::codex_user_server_args(users, |s| {
                        let path = codex_server_env_path(&session.id, &s.name);
                        crate::mcp::write_env_file(&path, &s.env).map(|()| path)
                    });
                    codex_args.extend(out.args);
                    names.extend(out.delivered);
                }
                names
            }
            _ => mcp_json_names,
        };
        // The enabled user servers this provider was NOT given, with why
        // (error-handling-6-06). Unknown user set (a failed lookup) → nothing
        // claimed skipped; `mcp_servers_error` already says so.
        let skipped = user_servers
            .as_deref()
            .map(|users| crate::mcp::skipped_servers(&session.provider, users, &delivered))
            .filter(|s| !s.is_empty());
        // Snapshot-at-spawn: what this provider was actually given, what it
        // was not, plus the lookup fault if any (json_patch: a null clears a
        // stale error / skip list).
        if let Err(e) = self
            .repo
            .merge_meta(
                &session.id,
                &serde_json::json!({
                    "mcp_servers": delivered,
                    "mcp_skipped": skipped,
                    "mcp_servers_error": lookup_error,
                }),
            )
            .await
        {
            tracing::warn!(session = %session.id, "record MCP server snapshot: {e}");
        }
        codex_args
    }
}
