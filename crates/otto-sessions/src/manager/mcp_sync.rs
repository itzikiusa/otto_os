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
    /// pi's per-session `-e` extension, `.mcp.json` for everyone else — plus
    /// `mcp_servers_error` when the user-server lookup failed. Best-effort
    /// throughout — never blocks the spawn.
    ///
    /// A failed lookup is NOT "nothing enabled": the previously managed user
    /// entries in the shared files are preserved (only Otto's own entries are
    /// reconciled) so one transient fault can't strip every session's servers.
    ///
    /// Returns the per-spawn launch args: codex's `-c` overrides for the
    /// browser and the user's servers (`codex_otto`: whether the caller
    /// already attached `otto` to codex), or pi's `-e <extension>`.
    pub(super) async fn sync_workspace_mcp(
        &self,
        session: &Session,
        otto_tools: Option<crate::mcp::OttoToolsServer>,
        codex_otto: bool,
    ) -> Vec<String> {
        let (user_servers, lookup_error) = self.resolve_user_mcp_servers(session).await;
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
        let cli_family = self
            .providers
            .model_family_for(&session.provider)
            .unwrap_or_else(|| session.provider.clone());
        let has_otto = otto_tools.is_some();
        let pi_otto = otto_tools.clone().filter(|_| cli_family == "pi");
        let cfg = crate::mcp::ManagedMcpConfig {
            browser: session_browser,
            user_servers: user_servers.clone(),
            otto_tools,
        };
        // The launcher reconciles are synchronous file IO under a per-cwd std
        // mutex: run them (and the canonicalize) on ONE blocking-pool thread,
        // so a contended cwd parks that thread, never a tokio worker.
        let (sid, cwd, provider) = (session.id.clone(), session.cwd.clone(), cli_family.clone());
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
            // Terminal Muse reads the workspace `.mcp.json` above (its `otto`
            // entry forwards the bridge env by name, which Muse requires);
            // opencode is unverified. Their Threads sessions get the full set
            // through ACP `mcpServers` (otto-agent-protocol).
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
        let mut launch_args = Vec::new();
        let delivered: Vec<String> = match cli_family.as_str() {
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
                    launch_args.extend(crate::mcp::codex_browser_args());
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
                    launch_args.extend(out.args);
                    names.extend(out.delivered);
                }
                names
            }
            // pi reads no launcher file Otto manages: a per-session `-e`
            // extension registers this spawn's set (the browser per this
            // session's own opt-in, like codex). Unknown user set → otto and
            // the browser only.
            "pi" => {
                let (doc, names) = crate::mcp::pi_mcp_extension(
                    pi_otto.as_ref(),
                    session_browser,
                    user_servers.as_deref(),
                );
                let path = pi_mcp_extension_path(&session.id);
                match doc {
                    Some(doc) => match crate::mcp::write_private_file(&path, doc.as_bytes()) {
                        Ok(()) => {
                            launch_args
                                .extend(["-e".to_string(), path.to_string_lossy().into_owned()]);
                            names
                        }
                        Err(e) => {
                            tracing::warn!(session = %session.id, "write pi MCP extension: {e}");
                            Vec::new()
                        }
                    },
                    None => {
                        let _ = std::fs::remove_file(&path);
                        Vec::new()
                    }
                }
            }
            _ => mcp_json_names,
        };
        // The enabled user servers this provider was NOT given, with why
        // (error-handling-6-06). Unknown user set (a failed lookup) → nothing
        // claimed skipped; `mcp_servers_error` already says so.
        let skipped = user_servers
            .as_deref()
            .map(|users| crate::mcp::skipped_servers(&cli_family, users, &delivered))
            .filter(|s| !s.is_empty());
        // Snapshot-at-spawn: what this provider was actually given, what it
        // was not, plus the lookup fault if any (json_patch: a null clears a
        // stale error / skip list).
        // A structured runtime session has no `sessions` row: its thread
        // shows the same snapshot as a server list (`runtime_mcp_status`).
        let structured = session.meta.get("runtime").and_then(|v| v.as_str()) == Some("structured");
        if structured {
            let otto_on = self.otto_mcp_enabled(&session.workspace_id).await;
            let status = spawn_mcp_status(
                &cli_family,
                otto_on,
                &delivered,
                skipped.as_deref().unwrap_or_default(),
                lookup_error.as_deref(),
            );
            self.runtime_mcp.insert(session.id.clone(), status);
            return launch_args;
        }
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
        launch_args
    }

    /// The workspace's ENABLED user MCP servers for `session` (Keychain
    /// secrets resolved), or `(None, Some(error))` when the lookup failed —
    /// "unknown", never "nothing enabled". Runs the sync resolver on the
    /// blocking pool.
    pub(super) async fn resolve_user_mcp_servers(
        &self,
        session: &Session,
    ) -> (Option<Vec<crate::mcp::UserMcpServer>>, Option<String>) {
        match &self.mcp_servers {
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
        }
    }
}

/// A structured thread's MCP servers at spawn, as its server list shows them:
/// Otto's own (`otto`, `otto-browser`) and the control-plane servers this
/// provider was given (`Added` until the CLI reports a connection), the
/// enabled ones it could not take with why, and a failed lookup. Pure.
pub(crate) fn spawn_mcp_status(
    provider: &str,
    otto_on: bool,
    delivered: &[String],
    skipped: &[serde_json::Value],
    lookup_error: Option<&str>,
) -> Vec<otto_core::agent_runtime::McpServerStatus> {
    use otto_core::agent_runtime::{McpServerSource as Src, McpServerState as St, McpServerStatus};
    let own = |n: &str| n == crate::mcp::OTTO_TOOLS_KEY || n == crate::mcp::SERVER_KEY;
    let mut out = Vec::new();
    if !delivered.iter().any(|n| n == crate::mcp::OTTO_TOOLS_KEY) {
        out.push(if otto_on {
            McpServerStatus::new(crate::mcp::OTTO_TOOLS_KEY, Src::Otto, St::Failed)
                .because("Otto couldn’t issue this thread’s tool credential (see the daemon log)")
        } else {
            McpServerStatus::new(crate::mcp::OTTO_TOOLS_KEY, Src::Otto, St::Disabled)
                .because("Otto’s tools are off for this workspace (MCP → Otto server)")
        });
    }
    for n in delivered {
        let src = if own(n) { Src::Otto } else { Src::Workspace };
        out.push(McpServerStatus::new(n.clone(), src, St::Added));
    }
    for s in skipped {
        let name = s
            .get("server")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let why = s
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        out.push(
            McpServerStatus::new(name, Src::Workspace, St::Unsupported)
                .because(format!("{} can’t take it: {why}", provider_name(provider))),
        );
    }
    if let Some(e) = lookup_error {
        out.push(
            McpServerStatus::new("Workspace servers", Src::Workspace, St::Failed)
                .because(format!("Otto couldn’t read the enabled servers: {e}")),
        );
    }
    out
}

/// "Claude", "Codex" … for a sentence (the slug for an unknown family).
pub(crate) fn provider_name(family: &str) -> String {
    match family {
        "claude" => "Claude".into(),
        "codex" => "Codex".into(),
        "gemini" => "Gemini".into(),
        "agy" => "Antigravity".into(),
        "grok" => "Grok".into(),
        "opencode" => "opencode".into(),
        "muse" => "Muse".into(),
        other => other.into(),
    }
}

#[cfg(test)]
mod spawn_status_tests {
    use super::*;
    use otto_core::agent_runtime::{McpServerSource as Src, McpServerState as St};

    #[test]
    fn a_thread_lists_what_it_was_given_and_what_it_was_not_with_why() {
        let skipped = [
            serde_json::json!({"server": "probe", "reason": "its command “probe” isn’t on the daemon’s PATH"}),
        ];
        let s = spawn_mcp_status(
            "gemini",
            true,
            &["otto".into(), "jira".into()],
            &skipped,
            None,
        );
        let view: Vec<_> = s
            .iter()
            .map(|s| (s.name.as_str(), s.source, s.state))
            .collect();
        assert_eq!(
            view,
            vec![
                ("otto", Src::Otto, St::Added),
                ("jira", Src::Workspace, St::Added),
                ("probe", Src::Workspace, St::Unsupported),
            ]
        );
        assert_eq!(
            s[2].reason.as_deref(),
            Some("Gemini can’t take it: its command “probe” isn’t on the daemon’s PATH")
        );
        // Otto's tools off for the workspace, and a failed lookup, are said too.
        let s = spawn_mcp_status("codex", false, &[], &[], Some("db locked"));
        assert_eq!(s[0].state, St::Disabled);
        assert_eq!(s[1].state, St::Failed);
        assert!(s[1].reason.as_deref().unwrap().contains("db locked"));
    }
}
