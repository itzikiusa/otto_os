//! Data-driven agent provider registry: claude / codex / shell built-ins,
//! overridable from the `providers` settings JSON.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
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
    /// Extra environment variables configured on a custom provider override.
    pub env: BTreeMap<String, String>,
    /// Shell command to run to update this provider's CLI, e.g. `"claude update"`.
    /// `None` means "no update command" (built-in shell provider, or unset custom).
    pub update_command: Option<String>,
    /// Model-flag TEMPLATE appended at spawn when the session pins a model
    /// (`meta.model`): each element has `{model}` substituted — e.g.
    /// `["--model","{model}"]` (claude/codex/agy) or `["-m","{model}"]`.
    /// `None` ⇒ the CLI takes no model flag; pickers hide the model control
    /// and a pinned model is dropped (`model_args` in manager.rs).
    pub model_args: Option<Vec<String>>,
    /// The built-in CLI family whose model catalog this provider lists
    /// (`claude` / `codex` / `agy` / `grok`): a built-in's own name, else the
    /// override's explicit `model_family`, else inferred from the command /
    /// name (`claude-work`, `claude --settings …` → `claude`). `None` ⇒ no
    /// catalog — the picker offers no models.
    pub model_family: Option<String>,
    /// True when the provider MINTS ITS OWN session id (so Otto can't pass it at
    /// launch and must capture it from disk after spawn — `codex`). False when
    /// Otto assigns the id via a launch flag (`claude --session-id {sid}`), or the
    /// provider isn't resumable at all. Drives whether the `provider_session_id`
    /// is recorded at spawn (assigned) or filled in by a post-spawn capture task.
    pub captures_session_id: bool,
    /// True for a settings-configured command (custom provider or a built-in
    /// override): when it isn't an executable on `$PATH` — a shell alias or
    /// function from the user's rc files — [`ProviderRegistry::build_spec`]
    /// launches it through the interactive login shell ([`shell_wrapped`]).
    /// Built-ins keep exec'ing their binary directly.
    pub shell_fallback: bool,
}

impl ProviderSpec {
    /// Resolve a Claude-family provider's effective `CLAUDE_CONFIG_DIR`
    /// (explicit `env` / inline `cmd`, wrapper script / shell rc alias, or
    /// conventional `~/.claude-<suffix>`). Returns `None` for non-Claude
    /// providers or built-in `claude` without a custom config dir.
    pub fn effective_claude_config_dir(&self, slug: &str, home: &Path) -> Option<PathBuf> {
        if self.model_family.as_deref() != Some("claude") {
            return None;
        }
        otto_transcript::claude_config::resolve_claude_config_dir(
            slug,
            &self.cmd,
            &self.args,
            self.env.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            home,
        )
    }
}

/// Shape accepted from the settings override JSON
/// (`{"<name>": {"cmd": "...", "args": [...], "resume_args": [...], "env": {...},
/// "update_command": "...", "model_args": ["-m", "{model}"]}}`).
#[derive(Debug, Deserialize)]
struct ProviderOverride {
    cmd: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    resume_args: Option<Vec<String>>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    update_command: Option<String>,
    #[serde(default)]
    model_args: Option<Vec<String>>,
    #[serde(default)]
    model_family: Option<String>,
}

/// Families with a model catalog (`provider_models`) and a `--model` flag.
pub const MODEL_FAMILIES: [&str; 7] =
    ["claude", "codex", "agy", "gemini", "grok", "opencode", "pi"];

fn matches_family_token(candidate: &str, fam: &str) -> bool {
    let lower = candidate.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    if lower == fam
        || lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|tok| tok == fam)
    {
        return true;
    }
    // Short 2-char names like `"pi"` must match on token/word boundaries only
    // (`pi`, `pi-fast`, `my-pi`, `/usr/bin/pi`), never substrings/prefixes like
    // `"api-agent"`, `"pipeline"`, or `"pilot"`.
    if fam.len() <= 2 {
        return false;
    }
    if fam.len() == 3 {
        return lower.starts_with(fam)
            && (lower.ends_with("work")
                || lower
                    .as_bytes()
                    .get(fam.len())
                    .is_some_and(|b| !b.is_ascii_alphabetic()));
    }
    lower.starts_with(fam)
}

/// The catalog family a custom provider belongs to: its explicit
/// `model_family` when that names a known family, else the first known family
/// that its command's basename or its own name matches (`claude-work`,
/// `claudework`, `/opt/bin/claude`, `claude-advisor` → `claude`).
pub fn infer_model_family(explicit: Option<&str>, cmd: &str, name: &str) -> Option<String> {
    let known = |s: &str| {
        MODEL_FAMILIES
            .iter()
            .find(|f| s.eq_ignore_ascii_case(f))
            .map(|f| (*f).to_string())
    };
    if let Some(f) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return known(f);
    }
    let base = cmd
        .split_whitespace()
        .next()
        .unwrap_or("")
        .rsplit('/')
        .next()
        .unwrap_or("");
    [base, name].into_iter().find_map(|s| {
        MODEL_FAMILIES
            .iter()
            .find(|f| matches_family_token(s, f))
            .map(|f| (*f).to_string())
    })
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

pub fn expand_template(template: &str, sid: &str, cwd: &str) -> String {
    template.replace("{sid}", sid).replace("{cwd}", cwd)
}

fn expand(template: &str, sid: &str, cwd: &str) -> String {
    expand_template(template, sid, cwd)
}

/// Whether `program` names an executable Otto can `exec` directly: a path
/// (contains `/`) that exists, or a bare name found as an executable file on
/// `$PATH`. False for shell aliases / functions defined in the user's rc files
/// (`alias claude-work='CLAUDE_CONFIG_DIR=… claude'`) — those exist only inside
/// an interactive shell.
pub fn resolves_on_path(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let is_exec = |p: &Path| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    let expanded = crate::tool_probe::expand_tilde(program);
    if expanded.contains('/') {
        return is_exec(Path::new(&expanded));
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| is_exec(&dir.join(&expanded))))
}

fn normalize_override_command(
    raw_cmd: &str,
    raw_args: Vec<String>,
    mut env: BTreeMap<String, String>,
) -> (String, Vec<String>, Vec<String>, BTreeMap<String, String>) {
    let trimmed = raw_cmd.trim();
    if trimmed.is_empty() {
        return (String::new(), Vec::new(), raw_args, env);
    }
    if !trimmed.chars().any(char::is_whitespace) {
        let expanded = if trimmed.starts_with("~/") || trimmed == "~" {
            crate::tool_probe::expand_tilde(trimmed)
        } else {
            trimmed.to_string()
        };
        return (expanded, Vec::new(), raw_args, env);
    }
    let whole = crate::tool_probe::expand_tilde(trimmed);
    if !whole.contains("{cwd}") && !whole.contains("{sid}") && Path::new(&whole).is_file() {
        return (whole, Vec::new(), raw_args, env);
    }
    let tokens = otto_transcript::claude_config::split_command_tokens(trimmed);
    let mut cmd_bin: Option<String> = None;
    let mut inline_args: Vec<String> = Vec::new();
    for tok in tokens {
        if cmd_bin.is_none() {
            if tok == "env" {
                continue;
            }
            if let Some((k, v)) = otto_transcript::claude_config::parse_env_assignment(&tok) {
                env.entry(k.to_string()).or_insert_with(|| v.to_string());
                continue;
            }
            let expanded_tok = if tok.starts_with("~/") || tok == "~" {
                crate::tool_probe::expand_tilde(&tok)
            } else {
                tok
            };
            cmd_bin = Some(expanded_tok);
        } else {
            inline_args.push(tok);
        }
    }
    let Some(bin) = cmd_bin else {
        return (trimmed.to_string(), Vec::new(), raw_args, env);
    };
    let mut merged = inline_args.clone();
    for a in raw_args {
        let is_dup_skip = matches!(
            a.as_str(),
            "--dangerously-skip-permissions"
                | "--dangerously-bypass-approvals-and-sandbox"
                | "--yolo"
                | "--always-approve"
                | "--auto"
        ) && merged.contains(&a);
        if !is_dup_skip {
            merged.push(a);
        }
    }
    (bin, inline_args, merged, env)
}

fn prepend_missing_inline(inline: &[String], explicit: Vec<String>) -> Vec<String> {
    if inline.is_empty() {
        return explicit;
    }
    let mut out: Vec<String> = inline
        .iter()
        .filter(|a| !explicit.contains(a))
        .cloned()
        .collect();
    out.extend(explicit);
    out
}

/// Launch `program args…` through the user's interactive login shell
/// (`$SHELL -ilc '<program> "$@"' <program> args…`) so a command that only
/// exists as an alias or function in `~/.zshrc` / `~/.bashrc` resolves the way
/// it does in the user's terminal. Args stay out of the script (passed as
/// positional `$@`), so later appends to the returned args — `--add-dir`,
/// `--model`, read-only flags — still reach the command unquoted-safe.
pub fn shell_wrapped(program: &str, args: Vec<String>) -> (String, Vec<String>) {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_string());
    // Keep bare command names unquoted so shell aliases expand, while quoting
    // filesystem paths or names with whitespace/quotes safely.
    let invoked = if program.contains('/')
        || program
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '\'' | '"'))
    {
        format!("'{}'", program.replace('\'', "'\\''"))
    } else {
        program.to_string()
    };
    let mut wrapped = vec![
        "-ilc".to_string(),
        format!("{invoked} \"$@\""),
        program.to_string(),
    ];
    wrapped.extend(args);
    (shell, wrapped)
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
                env: BTreeMap::new(),
                update_command: Some("claude update".into()),
                model_args: Some(vec!["--model".into(), "{model}".into()]),
                model_family: Some("claude".into()),
                // Otto assigns the id via `--session-id {sid}`.
                captures_session_id: false,
                shell_fallback: false,
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
                env: BTreeMap::new(),
                update_command: Some("codex update".into()),
                model_args: Some(vec!["--model".into(), "{model}".into()]),
                model_family: Some("codex".into()),
                captures_session_id: true,
                shell_fallback: false,
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
                env: BTreeMap::new(),
                update_command: Some("agy update".into()),
                // Verified against the installed agy CLI: `agy --help` lists
                // `--model  Model for the current CLI session`.
                model_args: Some(vec!["--model".into(), "{model}".into()]),
                model_family: Some("agy".into()),
                captures_session_id: true,
                shell_fallback: false,
            },
        );
        // gemini (Gemini CLI; verified against 0.43): `--session-id <uuid>`
        // starts a session with Otto's id and `--resume <uuid>` reopens it;
        // `--yolo` approves every tool.
        let yolo: Vec<String> = if skip_permissions {
            vec!["--yolo".into()]
        } else {
            Vec::new()
        };
        map.insert(
            "gemini".to_string(),
            ProviderSpec {
                cmd: "gemini".into(),
                args: [yolo.clone(), vec!["--session-id".into(), "{sid}".into()]].concat(),
                resume_args: Some([yolo, vec!["--resume".into(), "{sid}".into()]].concat()),
                env: BTreeMap::new(),
                update_command: Some("npm install -g @google/gemini-cli@latest".into()),
                model_args: Some(vec!["--model".into(), "{model}".into()]),
                model_family: Some("gemini".into()),
                captures_session_id: false,
                shell_fallback: false,
            },
        );
        // grok (Grok Build; verified against 1.0.77): `--session-id <uuid>`
        // for a new conversation, `--resume <id>` to reopen it;
        // `--always-approve` approves every tool.
        let always: Vec<String> = if skip_permissions {
            vec!["--always-approve".into()]
        } else {
            Vec::new()
        };
        map.insert(
            "grok".to_string(),
            ProviderSpec {
                cmd: "grok".into(),
                args: [always.clone(), vec!["--session-id".into(), "{sid}".into()]].concat(),
                resume_args: Some([always, vec!["--resume".into(), "{sid}".into()]].concat()),
                env: BTreeMap::new(),
                update_command: Some("grok update".into()),
                model_args: Some(vec!["--model".into(), "{model}".into()]),
                model_family: Some("grok".into()),
                captures_session_id: false,
                shell_fallback: false,
            },
        );
        // opencode (opencode.ai; verified against 1.19.5): the TUI is
        // `opencode [project] [--model provider/model] [-s <id> | -c]
        // [--auto]`. `-s` only OPENS an existing session ("Session not
        // found" for any other id) and opencode mints its own ids, so Otto
        // cannot pre-assign one; `-c` resumes the cwd's MOST RECENT session,
        // which would cross-wire two opencode sessions sharing a folder.
        // Terminal opencode sessions therefore start fresh on every
        // (re)spawn (no `resume_args`); Threads resumes opencode exactly via
        // ACP `session/load`. `--auto` approves every permission that is not
        // explicitly denied.
        map.insert(
            "opencode".to_string(),
            ProviderSpec {
                cmd: "opencode".into(),
                args: if skip_permissions {
                    vec!["--auto".into()]
                } else {
                    Vec::new()
                },
                resume_args: None,
                env: BTreeMap::new(),
                update_command: Some("opencode upgrade".into()),
                model_args: Some(vec!["--model".into(), "{model}".into()]),
                model_family: Some("opencode".into()),
                captures_session_id: false,
                shell_fallback: false,
            },
        );
        // pi (pi.dev; verified against 1.5.0): no permission system (tools
        // run unprompted). `--session-id <id>` opens the cwd's session with
        // that id and creates it when missing, so Otto assigns the id like
        // claude's and one argv both starts and resumes. (pi 0.x has no
        // `--session-id`.)
        let pi_args: Vec<String> = vec!["--session-id".into(), "{sid}".into()];
        map.insert(
            "pi".to_string(),
            ProviderSpec {
                cmd: "pi".into(),
                args: pi_args.clone(),
                resume_args: Some(pi_args),
                env: BTreeMap::new(),
                update_command: Some("pi update".into()),
                model_args: Some(vec!["--model".into(), "{model}".into()]),
                model_family: Some("pi".into()),
                captures_session_id: false,
                shell_fallback: false,
            },
        );
        map.insert(
            "shell".to_string(),
            ProviderSpec {
                cmd: shell,
                args: vec!["-l".into()],
                resume_args: None,
                env: BTreeMap::new(),
                update_command: None,
                model_args: None,
                model_family: None,
                captures_session_id: false,
                shell_fallback: false,
            },
        );

        if let Some(value) = overrides {
            if let Ok(parsed) =
                serde_json::from_value::<HashMap<String, ProviderOverride>>(value.clone())
            {
                for (name, o) in parsed {
                    let (cmd, inline_args, base_args, env) =
                        normalize_override_command(&o.cmd, o.args, o.env);
                    // Only keep a non-empty update_command string.
                    let update_command = o.update_command.filter(|s| !s.trim().is_empty());
                    let model_family = infer_model_family(o.model_family.as_deref(), &cmd, &name);
                    // An explicit EMPTY template means "no model flag"; an
                    // unset one defaults to `--model {model}` for a provider of
                    // a known family (every one of them takes it — grok too).
                    let model_args = match o.model_args {
                        Some(v) if v.is_empty() => None,
                        Some(v) => Some(v),
                        None => model_family
                            .as_ref()
                            .map(|_| vec!["--model".into(), "{model}".into()]),
                    };
                    let has_session_flag = base_args.iter().any(|a| {
                        a == "--session-id"
                            || a == "--resume"
                            || a == "--conversation"
                            || a.starts_with("--session-id=")
                            || a.starts_with("--resume=")
                            || a.starts_with("--conversation=")
                    });
                    let mut args = base_args.clone();
                    if matches!(model_family.as_deref(), Some("claude" | "pi")) && !has_session_flag
                    {
                        args.push("--session-id".into());
                        args.push("{sid}".into());
                    }
                    let explicit_resume = o
                        .resume_args
                        .map(|r| prepend_missing_inline(&inline_args, r));
                    let resume_args = match (explicit_resume, model_family.as_deref()) {
                        (Some(r), _) => Some(r),
                        (None, Some("claude")) => {
                            if !has_session_flag {
                                let mut r = base_args;
                                r.push("--resume".into());
                                r.push("{sid}".into());
                                Some(r)
                            } else if base_args.iter().any(|a| a.contains("{sid}")) {
                                Some(
                                    base_args
                                        .into_iter()
                                        .map(|a| {
                                            if a == "--session-id" {
                                                "--resume".to_string()
                                            } else if let Some(rest) =
                                                a.strip_prefix("--session-id=")
                                            {
                                                format!("--resume={rest}")
                                            } else {
                                                a
                                            }
                                        })
                                        .collect(),
                                )
                            } else {
                                None
                            }
                        }
                        (None, Some("pi")) => {
                            if !has_session_flag || args.iter().any(|a| a.contains("{sid}")) {
                                Some(args.clone())
                            } else {
                                None
                            }
                        }
                        (None, Some("codex")) => {
                            if !base_args.iter().any(|a| a.contains("{sid}")) {
                                let mut r = vec!["resume".into()];
                                r.extend(base_args);
                                r.push("{sid}".into());
                                Some(r)
                            } else {
                                None
                            }
                        }
                        (None, Some("agy")) => {
                            if !has_session_flag && !base_args.iter().any(|a| a.contains("{sid}")) {
                                let mut r = vec!["--conversation".into(), "{sid}".into()];
                                r.extend(base_args);
                                Some(r)
                            } else {
                                None
                            }
                        }
                        (None, _) => None,
                    };
                    let captures_session_id =
                        matches!(model_family.as_deref(), Some("codex" | "agy"))
                            && !args.iter().any(|a| a.contains("{sid}"))
                            && resume_args
                                .as_ref()
                                .is_some_and(|r| r.iter().any(|a| a.contains("{sid}")));
                    map.insert(
                        name,
                        ProviderSpec {
                            cmd,
                            args,
                            resume_args,
                            env,
                            update_command,
                            model_args,
                            model_family,
                            captures_session_id,
                            shell_fallback: true,
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

    /// The provider's model-flag TEMPLATE (each element gets `{model}`
    /// substituted at spawn — see `model_args` in manager.rs), or `None` when
    /// the CLI takes no model flag (shell, template-less custom providers).
    pub fn model_args_template(&self, name: &str) -> Option<Vec<String>> {
        let map = self.map.read().expect("provider registry lock");
        match map.get(name) {
            Some(p) => p.model_args.clone(),
            None if MODEL_FAMILIES.contains(&name) => {
                Some(vec!["--model".into(), "{model}".into()])
            }
            None => None,
        }
    }

    /// The catalog family of every ENABLED provider that has one, as
    /// `(provider, family)` pairs sorted by provider (drives
    /// `/meta.provider_models`).
    pub fn model_families(&self) -> Vec<(String, String)> {
        // `names()` takes its own read lock: call it first, never re-entrantly.
        let names = self.names();
        let map = self.map.read().expect("provider registry lock");
        names
            .into_iter()
            .filter_map(|n| {
                let fam = map.get(&n)?.model_family.clone()?;
                Some((n, fam))
            })
            .collect()
    }

    /// True when `name` exists and carries a model-flag template — drives the
    /// `/meta.model_flags` payload, so pickers hide the model control for
    /// providers that would silently drop it.
    pub fn supports_model(&self, name: &str) -> bool {
        let map = self.map.read().expect("provider registry lock");
        match map.get(name) {
            Some(p) => p.model_args.is_some(),
            None => MODEL_FAMILIES.contains(&name),
        }
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

    /// Return the full [`ProviderSpec`] for `name`, if registered.
    pub fn get(&self, name: &str) -> Option<ProviderSpec> {
        self.map
            .read()
            .expect("provider registry lock")
            .get(name)
            .cloned()
    }

    /// Return the model/CLI family for `name` (e.g. `"claude"` for `"claudework"`).
    pub fn model_family_for(&self, name: &str) -> Option<String> {
        self.map
            .read()
            .expect("provider registry lock")
            .get(name)
            .and_then(|p| p.model_family.clone())
    }

    /// Whether `slug` is an enabled built-in or registered custom provider.
    pub fn is_enabled(&self, slug: &str) -> bool {
        let slug = slug.trim();
        if slug.is_empty() {
            return false;
        }
        if self
            .disabled
            .read()
            .expect("provider registry lock")
            .contains(slug)
        {
            return false;
        }
        self.map
            .read()
            .expect("provider registry lock")
            .contains_key(slug)
            || otto_core::agent_runtime::RuntimeProvider::parse(slug).is_some()
    }

    /// Resolve a built-in or custom provider `slug` (e.g. `"claudework"`) to its
    /// Structured Runtime wire protocol ([`otto_core::agent_runtime::RuntimeProvider`]).
    /// Returns `None` when `slug` is disabled, unknown, or has no structured family.
    pub fn runtime_provider_for(
        &self,
        slug: &str,
    ) -> Option<otto_core::agent_runtime::RuntimeProvider> {
        let slug = slug.trim();
        if !self.is_enabled(slug) {
            return None;
        }
        if let Some(builtin) = otto_core::agent_runtime::RuntimeProvider::parse(slug) {
            return Some(builtin);
        }
        let family = self.model_family_for(slug)?;
        otto_core::agent_runtime::RuntimeProvider::parse(&family)
    }

    /// Return `(slug, ProviderSpec)` for every ENABLED provider, sorted by slug.
    pub fn enabled_specs(&self) -> Vec<(String, ProviderSpec)> {
        let names = self.names();
        let map = self.map.read().expect("provider registry lock");
        names
            .into_iter()
            .filter_map(|n| Some((n.clone(), map.get(&n)?.clone())))
            .collect()
    }

    /// Resolve a custom `CLAUDE_CONFIG_DIR` for `provider` (when it belongs to
    /// the `"claude"` family), or `None` when it uses the default `~/.claude`.
    pub fn claude_config_dir_for(&self, provider: &str, home: &Path) -> Option<PathBuf> {
        let spec = self.get(provider)?;
        spec.effective_claude_config_dir(provider, home)
    }

    /// All distinct custom `CLAUDE_CONFIG_DIR` roots across ENABLED providers.
    pub fn active_claude_config_dirs(&self, home: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for (slug, spec) in self.enabled_specs() {
            if let Some(dir) = spec.effective_claude_config_dir(&slug, home) {
                if !out.contains(&dir) {
                    out.push(dir);
                }
            }
        }
        out
    }

    /// Resolve an ENABLED and installed/aliased program for CLI `family`
    /// (`"claude"`, `"codex"`, `"agy"`, `"grok"`, `"opencode"`, `"pi"`).
    /// Prefers the exact slug `family` when enabled and runnable on `$PATH` /
    /// filesystem / rc alias; falls back to any enabled custom provider of that
    /// family whose command resolves; returns `None` when disabled or missing.
    pub fn resolved_family_program(&self, family: &str) -> Option<String> {
        let enabled = self.enabled_specs();
        if let Some((_, spec)) = enabled.iter().find(|(slug, _)| slug == family) {
            if crate::tool_probe::command_resolves_fast(&spec.cmd) {
                return Some(spec.cmd.clone());
            }
        } else if self.is_enabled(family) && crate::tool_probe::command_resolves_fast(family) {
            return Some(family.to_string());
        }
        enabled
            .into_iter()
            .filter(|(slug, spec)| slug != family && spec.model_family.as_deref() == Some(family))
            .map(|(_, spec)| spec.cmd)
            .find(|cmd| crate::tool_probe::command_resolves_fast(cmd))
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
        let raw_program = expand(&spec.cmd, sid, cwd);
        let program =
            if spec.shell_fallback && (raw_program == "~" || raw_program.starts_with("~/")) {
                crate::tool_probe::expand_tilde(&raw_program)
            } else {
                raw_program
            };
        let args: Vec<String> = args.iter().map(|a| expand(a, sid, cwd)).collect();
        let env: Vec<(String, String)> = spec
            .env
            .iter()
            .map(|(k, v)| {
                let expanded = expand(v, sid, cwd);
                let val = if expanded == "~" || expanded.starts_with("~/") {
                    crate::tool_probe::expand_tilde(&expanded)
                } else {
                    expanded
                };
                (k.clone(), val)
            })
            .collect();
        // A command that isn't an executable on PATH (a shell alias/function
        // from the user's rc files) can't be exec'd directly — run it through
        // the login shell instead of failing with "No such file or directory".
        let (program, args) = if !spec.shell_fallback || resolves_on_path(&program) {
            (program, args)
        } else {
            shell_wrapped(&program, args)
        };
        Ok(CommandSpec {
            program,
            args,
            cwd: Some(cwd.to_string()),
            env,
        })
    }
}

#[cfg(test)]
#[path = "providers_tests.rs"]
mod tests;
