//! Agent turns a runtime plugin asks for (`POST /api/v1/plugin-host/agents/run`).
//!
//! A plugin feeds the agent text it collected elsewhere (Jira issues, git
//! history, …), so the prompt is third-party text. By default the turn runs
//! on the untrusted path: no tools, Seatbelt-confined, in a throwaway scratch
//! dir ([`Orchestrator::run_agent_untrusted`]). Full tools need the plugin's
//! manifest to say `"agent_tools": true`, and even then the turn runs only in
//! the plugin's own data dir or a repo registered in Otto — never the
//! daemon's working directory.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{Orchestrator, UntrustedScratch};

/// Where a plugin's agent turn runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginAgentScope {
    /// Text in, text out: no tools, no MCP, a fresh scratch cwd.
    Untrusted,
    /// Full tools in this (validated, canonical) directory.
    Tools(PathBuf),
}

impl PluginAgentScope {
    /// The directory a tools turn runs in, for the audit row.
    pub fn cwd(&self) -> Option<&Path> {
        match self {
            Self::Untrusted => None,
            Self::Tools(dir) => Some(dir.as_path()),
        }
    }
}

/// Decide where a plugin's turn runs. Without `agent_tools` any requested
/// `cwd` is ignored (the turn needs none). With it, `cwd` defaults to the
/// plugin's data dir and must resolve inside that dir or a registered repo,
/// and is refused outright when it is the daemon's own working directory.
pub fn plugin_agent_scope(
    agent_tools: bool,
    requested_cwd: Option<&str>,
    plugin_data_dir: &Path,
    repo_paths: &[String],
    daemon_cwd: Option<&Path>,
) -> Result<PluginAgentScope, String> {
    if !agent_tools {
        return Ok(PluginAgentScope::Untrusted);
    }
    let want = match requested_cwd.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => PathBuf::from(c),
        None => {
            std::fs::create_dir_all(plugin_data_dir)
                .map_err(|e| format!("plugin data dir: {e}"))?;
            plugin_data_dir.to_path_buf()
        }
    };
    let dir = want
        .canonicalize()
        .map_err(|e| format!("cwd {}: {e}", want.display()))?;
    if daemon_cwd
        .and_then(|d| d.canonicalize().ok())
        .is_some_and(|d| d == dir)
    {
        return Err("cwd must not be the Otto daemon's own directory".into());
    }
    let inside = |root: &Path| root.canonicalize().is_ok_and(|r| dir.starts_with(r));
    if inside(plugin_data_dir) || repo_paths.iter().any(|r| inside(Path::new(r))) {
        Ok(PluginAgentScope::Tools(dir))
    } else {
        Err(format!(
            "cwd {} is neither the plugin's data dir nor a repo registered in Otto",
            dir.display()
        ))
    }
}

impl Orchestrator {
    /// Run one plugin-requested turn on `provider` ("claude" or "codex").
    /// Unattended, so it waits for the machine-load gate (source `plugin`)
    /// and holds its place until the turn ends, whichever runner it uses.
    pub async fn run_plugin_agent(
        &self,
        provider: &str,
        prompt: &str,
        model: Option<&str>,
        scope: &PluginAgentScope,
    ) -> otto_core::Result<String> {
        const NO_PROGRESS: Duration = Duration::from_secs(180);
        if !matches!(provider, "claude" | "codex") {
            return Err(otto_core::Error::Invalid(format!(
                "provider {provider:?} has no headless runner (supported: claude, codex)"
            )));
        }
        let _place = self.admit_background("plugin").await;
        match (provider, scope) {
            ("claude", PluginAgentScope::Untrusted) => {
                self.run_agent_untrusted(prompt, model, NO_PROGRESS).await
            }
            ("claude", PluginAgentScope::Tools(dir)) => {
                self.run_agent(prompt, &dir.to_string_lossy(), model, NO_PROGRESS)
                    .await
            }
            ("codex", PluginAgentScope::Untrusted) => {
                // codex has no tool-less mode: its read-only sandbox (no
                // writes, no network) in a throwaway scratch dir is the floor.
                let scratch = UntrustedScratch::create()?;
                run_codex_exec(prompt, &scratch.path().to_string_lossy(), model).await
            }
            ("codex", PluginAgentScope::Tools(dir)) => {
                run_codex_exec(prompt, &dir.to_string_lossy(), model).await
            }
            (other, _) => Err(otto_core::Error::Invalid(format!("provider {other:?}"))),
        }
    }
}

/// One-shot headless codex run: `codex exec` with a read-only sandbox and
/// `--output-last-message` so the reply is read from a file instead of parsing
/// the streamed TUI output. 10-minute hard cap.
async fn run_codex_exec(prompt: &str, cwd: &str, model: Option<&str>) -> otto_core::Result<String> {
    if otto_core::e2e::isolated() {
        return Ok(crate::e2e_stub::canned_reply(prompt));
    }
    let bin = std::env::var("CODEX_BIN")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "codex".to_string());
    let out_file =
        std::env::temp_dir().join(format!("otto-plugin-codex-{}.txt", uuid::Uuid::new_v4()));
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.arg("exec")
        .arg("--skip-git-repo-check")
        .arg("--sandbox")
        .arg("read-only")
        .arg("--output-last-message")
        .arg(&out_file)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        // The timeout below drops the wait future: without this the timed-out
        // `codex exec` ran on unsupervised (and kept burning tokens).
        .kill_on_drop(true);
    if let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) {
        cmd.arg("--model").arg(m);
    }
    // `--` closes the option list: the prompt is free text and may start
    // with a dash.
    cmd.arg("--").arg(prompt);
    let child = cmd
        .spawn()
        .map_err(|e| otto_core::Error::Upstream(format!("spawn {bin}: {e}")))?;
    let out = match tokio::time::timeout(Duration::from_secs(600), child.wait_with_output()).await {
        Ok(r) => r.map_err(|e| otto_core::Error::Upstream(format!("codex exec: {e}")))?,
        Err(_) => {
            let _ = tokio::fs::remove_file(&out_file).await;
            return Err(otto_core::Error::Upstream("codex exec timed out".into()));
        }
    };
    let text = tokio::fs::read_to_string(&out_file)
        .await
        .unwrap_or_default();
    let _ = tokio::fs::remove_file(&out_file).await;
    if !out.status.success() && text.trim().is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(otto_core::Error::Upstream(format!(
            "codex exec failed ({}): {}",
            out.status,
            err.chars().take(300).collect::<String>()
        )));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_the_manifest_opt_in_the_turn_is_untrusted_whatever_cwd_says() {
        let tmp = tempfile_dir();
        let data = tmp.join("data");
        for cwd in [None, Some("."), Some("/")] {
            assert_eq!(
                plugin_agent_scope(false, cwd, &data, &[], None),
                Ok(PluginAgentScope::Untrusted)
            );
        }
        assert!(!data.exists(), "an untrusted turn needs no data dir");
    }

    #[test]
    fn tools_default_to_the_plugin_data_dir_and_stay_inside_allowed_roots() {
        let tmp = tempfile_dir();
        let data = tmp.join("data");
        let repo = tmp.join("repo");
        let elsewhere = tmp.join("elsewhere");
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let repos = vec![repo.to_string_lossy().to_string()];

        let scope = plugin_agent_scope(true, None, &data, &repos, None).unwrap();
        assert_eq!(scope.cwd(), Some(data.canonicalize().unwrap().as_path()));

        let sub = repo.join("sub").to_string_lossy().to_string();
        let scope = plugin_agent_scope(true, Some(&sub), &data, &repos, None).unwrap();
        assert_eq!(
            scope,
            PluginAgentScope::Tools(repo.join("sub").canonicalize().unwrap())
        );

        let other = elsewhere.to_string_lossy().to_string();
        assert!(plugin_agent_scope(true, Some(&other), &data, &repos, None).is_err());
        // `..` cannot climb out of an allowed root.
        let climb = repo
            .join("..")
            .join("elsewhere")
            .to_string_lossy()
            .to_string();
        assert!(plugin_agent_scope(true, Some(&climb), &data, &repos, None).is_err());
        assert!(
            plugin_agent_scope(true, Some("/definitely/missing"), &data, &repos, None).is_err()
        );
    }

    #[test]
    fn tools_never_run_in_the_daemons_own_directory() {
        let tmp = tempfile_dir();
        let repo = tmp.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let repos = vec![repo.to_string_lossy().to_string()];
        let cwd = repo.to_string_lossy().to_string();
        // Even when the daemon happens to run from a registered repo.
        let err = plugin_agent_scope(true, Some(&cwd), &tmp.join("d"), &repos, Some(&repo))
            .expect_err("daemon dir refused");
        assert!(err.contains("daemon"), "{err}");
        // "." resolves to the test process's cwd, which is no allowed root.
        assert!(plugin_agent_scope(true, Some("."), &tmp.join("d"), &repos, None).is_err());
    }

    #[tokio::test]
    async fn an_unknown_provider_is_refused_before_anything_runs() {
        let orch = Orchestrator::new("/nonexistent/claude");
        let err = orch
            .run_plugin_agent("agy", "hi", None, &PluginAgentScope::Untrusted)
            .await
            .expect_err("agy has no headless runner");
        assert!(matches!(err, otto_core::Error::Invalid(_)), "{err}");
    }

    /// A plugin turn is unattended: it takes a place under the machine-load
    /// gate as `plugin` before any runner starts; a refused provider does not.
    #[tokio::test]
    async fn a_plugin_turn_waits_for_the_load_gate_as_plugin() {
        use crate::admission::AdmissionGuard;
        use std::sync::{Arc, Mutex};
        let orch = Orchestrator::new("/nonexistent/claude");
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let s = Arc::clone(&seen);
        orch.set_admission_hook(Arc::new(
            move |source: &str| -> otto_core::auth::BoxFuture<'static, AdmissionGuard> {
                s.lock().unwrap().push(source.to_string());
                Box::pin(async { Box::new(()) as AdmissionGuard })
            },
        ));
        let _ = orch
            .run_plugin_agent("agy", "hi", None, &PluginAgentScope::Untrusted)
            .await;
        assert!(seen.lock().unwrap().is_empty(), "refused before admission");
        // The claude binary does not exist: the turn fails (or is cut off
        // here), but only after it was admitted.
        let run = orch.run_plugin_agent("claude", "hi", None, &PluginAgentScope::Untrusted);
        let _ = tokio::time::timeout(Duration::from_secs(20), run).await;
        assert_eq!(*seen.lock().unwrap(), ["plugin".to_string()]);
    }

    /// A fresh dir under the system temp dir (this crate has no `tempfile`).
    fn tempfile_dir() -> PathBuf {
        let p = std::env::temp_dir().join(format!("otto-plugin-agent-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
