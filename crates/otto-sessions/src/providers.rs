//! Data-driven agent provider registry: claude / codex / shell built-ins,
//! overridable from the `providers` settings JSON.

use std::collections::HashMap;
use std::sync::RwLock;

use otto_core::{Error, Result};
use otto_pty::CommandSpec;
use serde::Deserialize;

/// How to launch (and resume) one agent provider CLI.
///
/// `args` / `resume_args` may contain the template vars `{sid}` (the
/// provider session id) and `{cwd}` (the session working directory).
#[derive(Debug, Clone)]
pub struct ProviderSpec {
    pub cmd: String,
    pub args: Vec<String>,
    pub resume_args: Option<Vec<String>>,
    /// Shell command to run to update this provider's CLI, e.g. `"claude update"`.
    /// `None` means "no update command" (built-in shell provider, or unset custom).
    pub update_command: Option<String>,
}

/// Shape accepted from the settings override JSON
/// (`{"<name>": {"cmd": "...", "args": [...], "resume_args": [...], "update_command": "..."}}`).
#[derive(Debug, Deserialize)]
struct ProviderOverride {
    cmd: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    resume_args: Option<Vec<String>>,
    #[serde(default)]
    update_command: Option<String>,
}

/// Registry of available agent providers. Interior-mutable so the settings
/// route can reload it live (custom providers apply without a daemon
/// restart).
#[derive(Debug)]
pub struct ProviderRegistry {
    map: RwLock<HashMap<String, ProviderSpec>>,
}

fn expand(template: &str, sid: &str, cwd: &str) -> String {
    template.replace("{sid}", sid).replace("{cwd}", cwd)
}

impl ProviderRegistry {
    /// Built-in providers, optionally overridden/extended by the `providers`
    /// settings value.
    pub fn new(overrides: Option<&serde_json::Value>) -> Self {
        Self {
            map: RwLock::new(Self::build_map(overrides)),
        }
    }

    /// Rebuild the registry from builtins + `overrides` (settings `providers`
    /// key). Existing sessions keep running; new spawns use the new map.
    pub fn reload(&self, overrides: Option<&serde_json::Value>) {
        *self.map.write().expect("provider registry lock") = Self::build_map(overrides);
    }

    fn build_map(overrides: Option<&serde_json::Value>) -> HashMap<String, ProviderSpec> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let mut map = HashMap::new();
        // Each agent CLI is launched as-is (no `-p`) with its own
        // skip-permissions flag so unattended sessions never block on a
        // tool-approval prompt.
        map.insert(
            "claude".to_string(),
            ProviderSpec {
                cmd: "claude".into(),
                args: vec![
                    "--session-id".into(),
                    "{sid}".into(),
                    "--dangerously-skip-permissions".into(),
                ],
                resume_args: Some(vec![
                    "--resume".into(),
                    "{sid}".into(),
                    "--dangerously-skip-permissions".into(),
                ]),
                update_command: Some("claude update".into()),
            },
        );
        map.insert(
            "codex".to_string(),
            ProviderSpec {
                cmd: "codex".into(),
                args: vec![
                    "--dangerously-bypass-approvals-and-sandbox".into(),
                    "--search".into(),
                ],
                resume_args: None,
                update_command: Some("codex update".into()),
            },
        );
        map.insert(
            "agy".to_string(),
            ProviderSpec {
                cmd: "agy".into(),
                args: vec![
                    "--dangerously-skip-permissions".into(),
                    "--add-dir={cwd}".into(),
                ],
                resume_args: None,
                update_command: Some("agy update".into()),
            },
        );
        map.insert(
            "shell".to_string(),
            ProviderSpec {
                cmd: shell,
                args: vec!["-l".into()],
                resume_args: None,
                update_command: None,
            },
        );

        if let Some(value) = overrides {
            if let Ok(parsed) =
                serde_json::from_value::<HashMap<String, ProviderOverride>>(value.clone())
            {
                for (name, o) in parsed {
                    // Only keep a non-empty update_command string.
                    let update_command = o.update_command.filter(|s| !s.trim().is_empty());
                    map.insert(
                        name,
                        ProviderSpec {
                            cmd: o.cmd,
                            args: o.args,
                            resume_args: o.resume_args,
                            update_command,
                        },
                    );
                }
            } else {
                tracing::warn!("ignoring malformed provider registry override");
            }
        }

        map
    }

    /// All `(provider_name, update_command)` pairs for providers that have an
    /// update command set (non-empty). Sorted by name for stable ordering.
    pub fn update_commands(&self) -> Vec<(String, String)> {
        let map = self.map.read().expect("provider registry lock");
        let mut pairs: Vec<(String, String)> = map
            .iter()
            .filter_map(|(name, spec)| {
                spec.update_command
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|cmd| (name.clone(), cmd.to_string()))
            })
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs
    }

    /// True when `name` exists and supports resume.
    pub fn supports_resume(&self, name: &str) -> bool {
        self.map
            .read()
            .expect("provider registry lock")
            .get(name)
            .is_some_and(|p| p.resume_args.is_some())
    }

    /// Provider names, sorted (for `/meta.providers`).
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .map
            .read()
            .expect("provider registry lock")
            .keys()
            .cloned()
            .collect();
        names.sort();
        names
    }

    /// Return the resolved program binary name for `provider`, or `None` if
    /// the provider is not registered.
    pub fn program_for(&self, name: &str) -> Option<String> {
        self.map
            .read()
            .expect("provider registry lock")
            .get(name)
            .map(|spec| spec.cmd.clone())
    }

    /// Build the concrete command for `provider`, expanding `{sid}`/`{cwd}`.
    /// `resume = true` uses `resume_args` (error when unsupported).
    pub fn build_spec(
        &self,
        provider: &str,
        sid: &str,
        cwd: &str,
        resume: bool,
    ) -> Result<CommandSpec> {
        let map = self.map.read().expect("provider registry lock");
        let spec = map
            .get(provider)
            .ok_or_else(|| Error::Invalid(format!("unknown provider '{provider}'")))?;
        let args = if resume {
            spec.resume_args
                .as_ref()
                .ok_or_else(|| Error::Invalid(format!("provider '{provider}' has no resume")))?
        } else {
            &spec.args
        };
        Ok(CommandSpec {
            program: expand(&spec.cmd, sid, cwd),
            args: args.iter().map(|a| expand(a, sid, cwd)).collect(),
            cwd: Some(cwd.to_string()),
            env: vec![],
        })
    }
}
