//! Data-driven agent provider registry: claude / codex / shell built-ins,
//! overridable from the `providers` settings JSON.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// True when the provider MINTS ITS OWN session id (so Otto can't pass it at
    /// launch and must capture it from disk after spawn — `codex`). False when
    /// Otto assigns the id via a launch flag (`claude --session-id {sid}`), or the
    /// provider isn't resumable at all. Drives whether the `provider_session_id`
    /// is recorded at spawn (assigned) or filled in by a post-spawn capture task.
    pub captures_session_id: bool,
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
    /// Whether built-in agent CLIs launch with their "skip permission prompts"
    /// flag (`--dangerously-skip-permissions` / codex
    /// `--dangerously-bypass-approvals-and-sandbox`). Default **on** so sessions
    /// run unattended. When an admin turns it OFF (the `agent_skip_permissions`
    /// setting), the flag is omitted and each CLI falls back to its own default
    /// permission mode (ask / auto), so tool use prompts in the session terminal.
    skip_permissions: AtomicBool,
    /// Provider names the admin has EXCLUDED (settings `disabled_providers`). A
    /// disabled provider is still built into `map` — so existing sessions that
    /// use it keep resuming — but it is hidden from [`Self::names`], which drives
    /// `/meta.providers` and therefore every picker + the planner's available
    /// set. Lets a user hide an installed CLI they don't want to use.
    disabled: RwLock<HashSet<String>>,
}

fn expand(template: &str, sid: &str, cwd: &str) -> String {
    template.replace("{sid}", sid).replace("{cwd}", cwd)
}

impl ProviderRegistry {
    /// Built-in providers, optionally overridden/extended by the `providers`
    /// settings value. Skip-permissions defaults **on** (unattended); the boot
    /// path applies the `agent_skip_permissions` setting via
    /// [`Self::set_skip_permissions`].
    pub fn new(overrides: Option<&serde_json::Value>) -> Self {
        Self {
            map: RwLock::new(Self::build_map(overrides, true)),
            skip_permissions: AtomicBool::new(true),
            disabled: RwLock::new(HashSet::new()),
        }
    }

    /// Rebuild the registry from builtins + `overrides` (settings `providers`
    /// key), preserving the current skip-permissions mode. Existing sessions keep
    /// running; new spawns use the new map.
    pub fn reload(&self, overrides: Option<&serde_json::Value>) {
        let skip = self.skip_permissions.load(Ordering::Relaxed);
        *self.map.write().expect("provider registry lock") = Self::build_map(overrides, skip);
    }

    /// Set whether built-in agent CLIs launch with skip-permissions, and rebuild
    /// the registry against `overrides` (the current `providers` setting). Called
    /// at boot and whenever the `agent_skip_permissions` setting changes. New
    /// spawns pick up the change immediately; running sessions are unaffected.
    pub fn set_skip_permissions(&self, skip: bool, overrides: Option<&serde_json::Value>) {
        self.skip_permissions.store(skip, Ordering::Relaxed);
        *self.map.write().expect("provider registry lock") = Self::build_map(overrides, skip);
    }

    /// The current skip-permissions mode (for `/meta` / the settings UI).
    pub fn skip_permissions(&self) -> bool {
        self.skip_permissions.load(Ordering::Relaxed)
    }

    /// Replace the set of EXCLUDED providers (settings `disabled_providers`).
    /// Independent of the launch map, so a reload / skip-permissions change never
    /// clears it. Applied at boot and whenever the setting changes; new pickers
    /// pick it up immediately (running sessions are untouched — the specs stay in
    /// `map` so a disabled provider still resumes).
    pub fn set_disabled<S: AsRef<str>>(&self, names: &[S]) {
        let set: HashSet<String> = names
            .iter()
            .map(|s| s.as_ref().trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        *self.disabled.write().expect("provider registry lock") = set;
    }

    /// Resolve a NEW-spawn default provider from ordered `candidates`, SKIPPING
    /// disabled providers: the first non-blank candidate that is currently ENABLED
    /// wins; else "claude" if enabled; else the first enabled provider; else
    /// "claude". This mirrors the UI's `defaultAgentProvider()` so the picker label
    /// and what the daemon actually spawns agree — a provider the admin excluded is
    /// never auto-selected as a default. (Existing sessions still resume a disabled
    /// provider via `build_spec`, which is intentionally NOT filtered.)
    pub fn resolve_default(&self, candidates: &[&str]) -> String {
        let enabled = self.names(); // already disabled-filtered + sorted
        let is_enabled = |name: &str| enabled.iter().any(|n| n == name);
        for c in candidates {
            let c = c.trim();
            if !c.is_empty() && is_enabled(c) {
                return c.to_string();
            }
        }
        if is_enabled(otto_core::provider::FALLBACK_PROVIDER) {
            return otto_core::provider::FALLBACK_PROVIDER.to_string();
        }
        enabled
            .into_iter()
            .next()
            .unwrap_or_else(|| otto_core::provider::FALLBACK_PROVIDER.to_string())
    }

    /// The currently-excluded provider names, sorted (for the settings UI).
    pub fn disabled_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .disabled
            .read()
            .expect("provider registry lock")
            .iter()
            .cloned()
            .collect();
        v.sort();
        v
    }

    fn build_map(
        overrides: Option<&serde_json::Value>,
        skip_permissions: bool,
    ) -> HashMap<String, ProviderSpec> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        // Give agent CLIs a BROAD working-dir scope (the user's home) via
        // `--add-dir`. Claude Code only resets the bash cwd when a `cd` leaves
        // the allowed working dirs (verified: `cd` WITHIN an allowed dir
        // persists; outside it is reset to the launch dir). Without this an
        // agent session is pinned to its one launch directory and `cd` to any
        // sibling project gets reset — so add `$HOME` to let the user hop
        // between projects freely. Sessions already run skip-permissions, so
        // this only widens the cwd SCOPE, not tool access.
        let home = std::env::var("HOME").unwrap_or_default();
        let home_add_dir: Vec<String> = if home.is_empty() {
            vec![]
        } else {
            vec![format!("--add-dir={home}")]
        };
        // The "skip permission prompts" flags, injected only when `skip_permissions`
        // is on (the default). When off, each CLI is launched WITHOUT them and uses
        // its own default permission mode (ask / auto). `skip_flag` is the
        // claude/agy form; `codex_bypass` is codex's equivalent.
        let skip_flag: Vec<String> = if skip_permissions {
            vec!["--dangerously-skip-permissions".into()]
        } else {
            vec![]
        };
        let codex_bypass: Vec<String> = if skip_permissions {
            vec!["--dangerously-bypass-approvals-and-sandbox".into()]
        } else {
            vec![]
        };
        let mut map = HashMap::new();
        // Each agent CLI is launched as-is (no `-p`) with its own
        // skip-permissions flag so unattended sessions never block on a
        // tool-approval prompt.
        map.insert(
            "claude".to_string(),
            ProviderSpec {
                cmd: "claude".into(),
                args: {
                    let mut a = vec!["--session-id".into(), "{sid}".into()];
                    a.extend(skip_flag.iter().cloned());
                    a.extend(home_add_dir.iter().cloned());
                    a
                },
                resume_args: Some({
                    let mut a = vec!["--resume".into(), "{sid}".into()];
                    a.extend(skip_flag.iter().cloned());
                    a.extend(home_add_dir.iter().cloned());
                    a
                }),
                update_command: Some("claude update".into()),
                // Otto assigns the id via `--session-id {sid}`.
                captures_session_id: false,
            },
        );
        map.insert(
            "codex".to_string(),
            ProviderSpec {
                cmd: "codex".into(),
                args: {
                    let mut a = codex_bypass.clone();
                    a.push("--search".into());
                    a
                },
                // Codex doesn't accept a settable session id at launch — it mints
                // its own UUID and records a rollout under `$CODEX_HOME/sessions`.
                // Otto captures that UUID after spawn (see `capture_codex_session_id`)
                // and resumes the exact conversation with `codex resume <uuid>`.
                // The bypass/search flags are valid on the `resume` subcommand too
                // (verified against codex-cli 0.142), so resumed sessions stay
                // unattended-safe with live web search, like a fresh launch.
                resume_args: Some({
                    let mut a = vec!["resume".into()];
                    a.extend(codex_bypass.iter().cloned());
                    a.push("--search".into());
                    a.push("{sid}".into());
                    a
                }),
                update_command: Some("codex update".into()),
                captures_session_id: true,
            },
        );
        map.insert(
            "agy".to_string(),
            ProviderSpec {
                cmd: "agy".into(),
                args: {
                    let mut a = skip_flag.clone();
                    a.push("--add-dir={cwd}".into());
                    a.extend(home_add_dir.iter().cloned());
                    a
                },
                // agy (Antigravity Gemini CLI) mints its OWN conversation id and
                // records it under `~/.gemini/antigravity-cli` (the conversation
                // file `conversations/<id>.db|.pb` plus a `cache/last_conversations.json`
                // map of cwd -> most-recent conversation id). Like codex, Otto can't
                // pass the id at launch, so it captures it from disk after spawn (see
                // `capture_agy_session_id`) and resumes the exact conversation with
                // `agy --conversation <id>`. The skip-permissions/add-dir flags stay
                // on resume so resumed sessions remain unattended-safe.
                resume_args: Some({
                    let mut a = vec!["--conversation".into(), "{sid}".into()];
                    a.extend(skip_flag.iter().cloned());
                    a.push("--add-dir={cwd}".into());
                    a.extend(home_add_dir.iter().cloned());
                    a
                }),
                update_command: Some("agy update".into()),
                captures_session_id: true,
            },
        );
        map.insert(
            "shell".to_string(),
            ProviderSpec {
                cmd: shell,
                args: vec!["-l".into()],
                resume_args: None,
                update_command: None,
                captures_session_id: false,
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
                            // Custom providers use Otto-assigned ids (the `{sid}`
                            // template); rollout-style capture is built-in only.
                            captures_session_id: false,
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

    /// True when the provider mints its OWN session id (codex) — so Otto records
    /// no `provider_session_id` at spawn and instead captures it from disk after
    /// launch. False for Otto-assigned providers (claude) and non-resumables.
    pub fn captures_session_id(&self, name: &str) -> bool {
        self.map
            .read()
            .expect("provider registry lock")
            .get(name)
            .is_some_and(|p| p.captures_session_id)
    }

    /// Provider names, sorted, EXCLUDING any the admin disabled (for
    /// `/meta.providers` → every UI picker + the planner). Disabled specs remain
    /// in `map` so `build_spec` still resumes existing sessions.
    pub fn names(&self) -> Vec<String> {
        let disabled = self.disabled.read().expect("provider registry lock");
        let mut names: Vec<String> = self
            .map
            .read()
            .expect("provider registry lock")
            .keys()
            .filter(|k| !disabled.contains(k.as_str()))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args_for(reg: &ProviderRegistry, provider: &str, resume: bool) -> Vec<String> {
        reg.build_spec(provider, "SID", "/tmp/cwd", resume)
            .expect("build_spec")
            .args
    }

    /// Default (skip on): every built-in agent CLI carries its skip-permissions
    /// flag on both launch and resume.
    #[test]
    fn skip_permissions_on_by_default() {
        let reg = ProviderRegistry::new(None);
        assert!(reg.skip_permissions());
        assert!(args_for(&reg, "claude", false).contains(&"--dangerously-skip-permissions".into()));
        assert!(args_for(&reg, "claude", true).contains(&"--dangerously-skip-permissions".into()));
        assert!(args_for(&reg, "codex", false)
            .contains(&"--dangerously-bypass-approvals-and-sandbox".into()));
        assert!(args_for(&reg, "codex", true)
            .contains(&"--dangerously-bypass-approvals-and-sandbox".into()));
        assert!(args_for(&reg, "agy", false).contains(&"--dangerously-skip-permissions".into()));
    }

    /// Opted out: the skip/bypass flag is gone from every provider (launch + resume)
    /// while the rest of the launch line is preserved.
    #[test]
    fn skip_permissions_off_drops_the_flag_only() {
        let reg = ProviderRegistry::new(None);
        reg.set_skip_permissions(false, None);
        assert!(!reg.skip_permissions());

        let claude = args_for(&reg, "claude", false);
        assert!(!claude.iter().any(|a| a.contains("dangerously")));
        assert!(claude.contains(&"--session-id".into())); // rest of the line intact
        assert!(!args_for(&reg, "claude", true)
            .iter()
            .any(|a| a.contains("dangerously")));

        let codex = args_for(&reg, "codex", false);
        assert!(!codex.iter().any(|a| a.contains("dangerously")));
        assert!(codex.contains(&"--search".into())); // search preserved
        assert!(!args_for(&reg, "codex", true)
            .iter()
            .any(|a| a.contains("dangerously")));

        let agy = args_for(&reg, "agy", false);
        assert!(!agy.iter().any(|a| a.contains("dangerously")));
        assert!(agy.iter().any(|a| a.starts_with("--add-dir="))); // add-dir preserved
    }

    /// Disabling a provider hides it from `names()` (the picker/meta list) but
    /// keeps its launch spec, so an existing session on it still resumes. A
    /// reload or skip-permissions change preserves the disabled set.
    #[test]
    fn disabled_providers_hidden_from_names_but_still_spawnable() {
        let reg = ProviderRegistry::new(None);
        assert!(reg.names().contains(&"agy".to_string()));

        reg.set_disabled(&["agy"]);
        assert!(!reg.names().contains(&"agy".to_string()));
        assert!(reg.names().contains(&"claude".to_string()));
        assert_eq!(reg.disabled_names(), vec!["agy".to_string()]);
        // Still launchable (existing sessions keep resuming).
        assert!(reg.build_spec("agy", "SID", "/tmp", false).is_ok());

        // A providers reload / skip-permissions toggle must not clear it.
        reg.reload(None);
        assert!(!reg.names().contains(&"agy".to_string()));
        reg.set_skip_permissions(false, None);
        assert!(!reg.names().contains(&"agy".to_string()));

        // Re-enable clears the exclusion.
        reg.set_disabled::<String>(&[]);
        assert!(reg.names().contains(&"agy".to_string()));
    }

    /// `resolve_default` never returns a DISABLED provider — a default that names
    /// an excluded provider is skipped, matching the UI picker, so the daemon and
    /// the label agree on what a new spawn actually runs.
    #[test]
    fn resolve_default_skips_disabled_providers() {
        let reg = ProviderRegistry::new(None);
        // Nothing disabled: the configured default wins, else claude.
        assert_eq!(reg.resolve_default(&["", "codex"]), "codex");
        assert_eq!(reg.resolve_default(&["", ""]), "claude");

        // Disable the configured default (codex) → it is skipped, falls to claude.
        reg.set_disabled(&["codex"]);
        assert_eq!(reg.resolve_default(&["", "codex"]), "claude");

        // Disable claude (the fallback) too → falls to the first ENABLED provider
        // (sorted: agy), never a disabled name.
        reg.set_disabled(&["codex", "claude"]);
        let d = reg.resolve_default(&["", "codex"]);
        assert!(d != "codex" && d != "claude", "got disabled provider: {d}");
        assert!(reg.names().contains(&d));
    }

    /// A providers reload preserves the current skip-permissions mode.
    #[test]
    fn reload_preserves_skip_mode() {
        let reg = ProviderRegistry::new(None);
        reg.set_skip_permissions(false, None);
        reg.reload(None); // e.g. a `providers` settings change
        assert!(!reg.skip_permissions());
        assert!(!args_for(&reg, "claude", false)
            .iter()
            .any(|a| a.contains("dangerously")));
    }
}
