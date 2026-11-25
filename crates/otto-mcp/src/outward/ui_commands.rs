//! Agent UI control — the command catalog (`docs/contracts/ui-commands.json`).
//!
//! The JSON file is the ONE source of truth for the `otto.ui_*` tools: the
//! daemon embeds it here (`include_str!`), the UI imports the same file
//! (`ui/src/lib/uiCommands/catalog.ts`), and a test on each side pins them
//! together — the Rust tests below validate every entry, the UI's parity test
//! asserts every entry has a registered handler. Each entry becomes one governed
//! tool (`otto.ui_<name>`, stdio `otto_ui_<name>`) via [`specs`], appended to
//! `mcp_outward::otto_tool_specs()` under the "UI control" category, so both
//! MCP surfaces advertise exactly what the catalog declares.
//!
//! Also here: the dependency-free argument validator the bridge runs before a
//! command leaves the daemon ([`validate_args`]), and a port of the UI's
//! `paneKey` (`ui/src/lib/sidePane.ts`) so a catalog entry's `module` is
//! checked against its `route` exactly as the UI will route it.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The embedded catalog text (also served verbatim by `GET /ui/commands/catalog`).
pub const CATALOG_JSON: &str = include_str!("../../../../docs/contracts/ui-commands.json");

/// Prefix every catalog command carries as a governed tool (`ui_<name>`).
pub const TOOL_PREFIX: &str = "ui_";

/// The governed-tool category the control plane groups these under.
pub const CATEGORY: &str = "UI control";

/// Upper bound on any command's deadline (and on the human-confirm extension).
pub const MAX_TIMEOUT_MS: u64 = 120_000;
/// Lower bound on a catalog `timeout_ms`.
pub const MIN_TIMEOUT_MS: u64 = 1_000;

/// How much a command may do. `read` / `navigate` run once the session is
/// granted; `local_write` / `outward` additionally confirm in the UI every
/// time; `execute` runs code on the user's Mac outside the agent's sandbox
/// (a debug launch, an evaluated expression) and confirms every time with no
/// "allow for this session" memory. All three count as mutating for a
/// read-only MCP token scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Read,
    Navigate,
    LocalWrite,
    Outward,
    Execute,
}

impl Risk {
    /// Mutating tiers — denied to an MCP token whose scope is read-only.
    pub fn mutating(self) -> bool {
        matches!(self, Risk::LocalWrite | Risk::Outward | Risk::Execute)
    }
}

/// How the daemon may open a command's module when no document shows it
/// (`open_placement`; default `side`).
pub const OPEN_PLACEMENTS: &[&str] = &["side", "window"];

/// Daemon-side fallbacks a `read` command may run when no Otto window can.
pub const HEADLESS: &[&str] = &["presence", "db_list_connections", "db_mcp_query"];

/// The paneKeys a catalog entry may target: every sidebar module id
/// (`ui/src/lib/sidebar.ts` `SIDEBAR_MODULES`, which `paneKey` mirrors), plus
/// `shell` (any Otto document — navigation / state commands).
pub const KNOWN_MODULES: &[&str] = &[
    "shell",
    "home",
    "agents",
    "threads",
    "rooms",
    "loops",
    "workflows",
    "scheduled-tasks",
    "git",
    "product",
    "vault",
    "ide",
    "workbench",
    "skills-eval",
    "connections",
    "aws",
    "kubernetes",
    "api",
    "browser",
    "mcp",
    "insights",
    "usage",
    "settings",
];

/// One catalog entry (see the file's `$comment` for field semantics).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiCommandSpec {
    pub name: String,
    pub module: String,
    pub route: String,
    pub risk: Risk,
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headless: Option<String>,
    /// Where the bridge's auto-open shows the module (`side` | `window`);
    /// `None` = `side`. A `window` command also skips hidden documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_placement: Option<String>,
    pub description: String,
    pub input_schema: Value,
    /// Block marker / note — ignored.
    #[serde(default, rename = "$comment", skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

impl UiCommandSpec {
    /// The bare governed tool name (`ui_db_run_query`).
    pub fn tool(&self) -> String {
        format!("{TOOL_PREFIX}{}", self.name)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    #[serde(default, rename = "$comment")]
    _comment: Option<String>,
    version: u32,
    commands: Vec<UiCommandSpec>,
}

/// Parse catalog text. Pure — the tests run it on the embedded file.
fn parse(text: &str) -> Result<(u32, Vec<UiCommandSpec>), String> {
    let f: CatalogFile =
        serde_json::from_str(text).map_err(|e| format!("ui-commands.json: {e}"))?;
    Ok((f.version, f.commands))
}

/// The parsed catalog. A malformed embedded file is a build-time bug the
/// tests catch; at runtime it degrades to an EMPTY catalog (no `ui_*` tools)
/// rather than taking the daemon down.
pub fn catalog() -> &'static [UiCommandSpec] {
    static CAT: OnceLock<Vec<UiCommandSpec>> = OnceLock::new();
    CAT.get_or_init(|| match parse(CATALOG_JSON) {
        Ok((_, cmds)) => cmds,
        Err(e) => {
            tracing::error!("agent UI control disabled: {e}");
            Vec::new()
        }
    })
}

/// Look a command up by its catalog name (`db_run_query`).
pub fn get(name: &str) -> Option<&'static UiCommandSpec> {
    catalog().iter().find(|c| c.name == name)
}

/// Look a command up by its bare governed tool name (`ui_db_run_query`).
pub fn by_tool(tool: &str) -> Option<&'static UiCommandSpec> {
    tool.strip_prefix(TOOL_PREFIX).and_then(get)
}

/// True iff `tool` (bare) is a catalog UI-control tool.
pub fn is_ui_tool(tool: &str) -> bool {
    by_tool(tool).is_some()
}

/// Every catalog tool's bare name.
pub fn tool_names() -> Vec<String> {
    catalog().iter().map(UiCommandSpec::tool).collect()
}

/// The governed-tool specs (`otto_tool_specs` shape) for every catalog entry.
pub fn specs() -> Vec<Value> {
    catalog()
        .iter()
        .map(|c| {
            json!({
                "name": format!("otto.{}", c.tool()),
                "mutating": c.risk.mutating(),
                "category": CATEGORY,
                "description": format!(
                    "{} [Agent UI control — runs visibly in the user's Otto window; needs the \
                     user's per-session \"Allow UI control\" grant. Risk: {}.]",
                    c.description,
                    risk_label(c.risk)
                ),
                "inputSchema": c.input_schema,
            })
        })
        .collect()
}

fn risk_label(r: Risk) -> &'static str {
    match r {
        Risk::Read => "read",
        Risk::Navigate => "navigate",
        Risk::LocalWrite => "local write — the user confirms each one",
        Risk::Outward => "outward — the user confirms each one",
        Risk::Execute => "runs code on the user's Mac — the user confirms each one",
    }
}

/// Port of `paneKey` (`ui/src/lib/sidePane.ts`): the sidebar entry a hash
/// route belongs to. `''` → agents, `plugin/<slug>` per plugin, the Database /
/// Message Brokers views → connections, Design / Canvas → product, Personal
/// Agents → scheduled-tasks.
pub fn pane_key(route: &str) -> String {
    let r = route.trim_start_matches('#').trim_start_matches('/');
    let mut parts = r.split(['/', '?']);
    let m = parts.next().unwrap_or("");
    match m {
        "" => "agents".into(),
        "plugin" => format!("plugin/{}", parts.next().unwrap_or("")),
        "database" | "brokers" => "connections".into(),
        "design" | "canvas" => "product".into(),
        "personal-agents" => "scheduled-tasks".into(),
        "workbench" => "ide".into(),
        other => other.into(),
    }
}

// ===========================================================================
// Argument validation — the subset of JSON Schema the catalog uses.
// ===========================================================================

/// Environment variables an agent may not pass to a process it launches (an
/// `env` argument): each one makes a loader, shell, toolchain or runtime run
/// other code (`GOFLAGS=-toolexec=…`, `DYLD_INSERT_LIBRARIES`,
/// `NODE_OPTIONS=--require …`). Names compare case-insensitively.
const DENIED_ENV: &[&str] = &[
    "PATH",
    "ENV",
    "BASH_ENV",
    "ZDOTDIR",
    "SHELL",
    "PS4",
    "PROMPT_COMMAND",
    "IFS",
    "CC",
    "CXX",
    "GOFLAGS",
    "GOROOT",
    "GOPATH",
    "GOTOOLCHAIN",
    "GOENV",
    "GOPROXY",
    "GOWORK",
    "GOBIN",
    "GOEXPERIMENT",
    "GOINSECURE",
    "GONOSUMDB",
    "GONOPROXY",
    "GOPRIVATE",
    "GOSUMDB",
    "GOVCS",
    "RUSTFLAGS",
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTDOCFLAGS",
    "PERL5OPT",
    "PERL5LIB",
    "RUBYOPT",
    "RUBYLIB",
    "MAVEN_OPTS",
    "GRADLE_OPTS",
    "JAVA_HOME",
    "CLASSPATH",
    // C1-019: files / endpoints a tool the program runs reads as code,
    // credentials or a server to trust (credential_process, exec plugins,
    // Testcontainers' Docker, proxies, CA bundles, the user's dotfiles).
    "AWS_CONFIG_FILE",
    "AWS_SHARED_CREDENTIALS_FILE",
    "KUBECONFIG",
    "VIRTUAL_ENV",
    "GOCACHE",
    "GOMODCACHE",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "CURL_CA_BUNDLE",
    "REQUESTS_CA_BUNDLE",
    "HOME",
    "XDG_CONFIG_HOME",
    "SHELLOPTS",
    "BASHOPTS",
    "FPATH",
    "MAKEFLAGS",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "SUDO_ASKPASS",
    "NO_PROXY",
    "TMPDIR",
    "TMP",
    "TEMP",
    // Read by the IDE engine (every_env_name_the_ide_reads_is_decided): Xcode /
    // Flutter toolchain roots, the cache owner and the shell's directory.
    "DEVELOPER_DIR",
    "FLUTTER_ROOT",
    "USER",
    "LOGNAME",
    "PWD",
];

/// Whether an agent may not set the env name `up` (upper-cased).
fn env_denied(up: &str) -> bool {
    DENIED_ENV.contains(&up) || DENIED_ENV_PREFIXES.iter().any(|p| up.starts_with(p))
}
/// Prefixes of the same (`LD_PRELOAD`, `PYTHONSTARTUP`, `JAVA_TOOL_OPTIONS`, …).
const DENIED_ENV_PREFIXES: &[&str] = &[
    "LD_",
    "DYLD_",
    "DLV_",
    "CGO_",
    "PYTHON",
    "NODE_",
    "NPM_CONFIG_",
    "JAVA_",
    "_JAVA_",
    "JDK_",
    "CARGO_",
    "GIT_",
    "BUN_",
    "DENO_",
    // C1-002: the Go adapter splits GO_BUILD_FLAGS / DLV_BUILD_FLAGS into
    // `go build` flags (`-toolexec=…`); OTTO_* switch Otto's own adapters.
    "GO_",
    "OTTO_",
    "DOCKER_",
    "TESTCONTAINERS_",
    "RUSTUP_",
    "BASH_FUNC_",
    "DOTNET_",
    "CORECLR_",
    "COMPLUS_",
    // C2-003: Otto's Java / Maven builders read MAVEN_REPO_LOCAL / M2_REPO as
    // the repository they load jars and plugins from; cargo runs
    // RUSTC_WORKSPACE_WRAPPER; OpenSSL loads engines / modules from its
    // config; AWS_* pick credentials and endpoints; SSH_* askpass / agent.
    "MAVEN_",
    "M2_",
    "GRADLE_",
    "RUSTC_",
    "OPENSSL_",
    "AWS_",
    "SSH_",
];

/// Most `env` entries one launch takes, and the longest value: the confirm
/// shows every entry in full, so it must stay readable.
const MAX_ENV: usize = 50;
const MAX_ENV_VALUE: usize = 4096;

/// A character the confirm would not show for what it is: a control
/// character, a BiDi override / isolate / mark, or a zero-width one.
fn invisible(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{061c}' | '\u{feff}')
}

/// The catalog command's own rules on top of its schema: an `env` argument
/// carries no loader / toolchain variable ([`DENIED_ENV`]), and
/// `ide_debug_request` only reaches this Mac (loopback http/https). Pure.
/// Longest agent-written text a confirm shows in one place (a start's args +
/// env, a request's body): longer is refused, never clipped.
const MAX_SHOWN_TEXT: usize = 8 * 1024;

pub fn validate_command(spec: &UiCommandSpec, args: &Value) -> Result<(), String> {
    validate_args(&spec.input_schema, args)?;
    for tag in args
        .get("build_tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let t = tag.as_str().unwrap_or_default();
        if t.is_empty()
            || !t
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        {
            return Err(format!(
                "build tag {t:?} is not a plain build tag name (letters, digits, _ and . only)"
            ));
        }
    }
    if let Some(env) = args.get("env").and_then(Value::as_object) {
        if env.len() > MAX_ENV {
            return Err(format!("env takes at most {MAX_ENV} entries"));
        }
        for (k, v) in env {
            let plain = (1..=128).contains(&k.len())
                && k.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !plain {
                return Err(format!(
                    "env {k:?} is not a plain name ([A-Za-z_][A-Za-z0-9_]*, ≤ 128)"
                ));
            }
            let up = k.to_ascii_uppercase();
            if env_denied(&up) {
                return Err(format!(
                    "env '{k}' is refused: it changes which code the loader, shell or toolchain \
                     runs. Set it in the project's run configuration if the user wants it"
                ));
            }
            let Some(v) = v.as_str() else {
                return Err(format!("env '{k}' must be a string"));
            };
            if v.chars().count() > MAX_ENV_VALUE {
                return Err(format!(
                    "env '{k}' is longer than {MAX_ENV_VALUE} characters"
                ));
            }
            if v.chars().any(invisible) {
                return Err(format!(
                    "env '{k}' holds invisible or control characters the user could not see in the confirm"
                ));
            }
        }
    }
    // What the person reads in a confirm stays whole AND readable: one budget
    // for a start's arguments + environment, and for a request's body.
    let strs = |k: &str| -> usize {
        let list = args.get(k).and_then(Value::as_array).into_iter().flatten();
        list.filter_map(Value::as_str).map(str::len).sum()
    };
    let env_len: usize = (args
        .get("env")
        .and_then(Value::as_object)
        .into_iter()
        .flatten())
    .map(|(k, v)| k.len() + v.as_str().map_or(0, str::len))
    .sum();
    if strs("args") + env_len > MAX_SHOWN_TEXT {
        return Err(format!(
            "args and env together exceed {MAX_SHOWN_TEXT} characters — too much for the user to read in the confirm"
        ));
    }
    let body = args.get("body").and_then(Value::as_str).unwrap_or_default();
    let headers_len: usize = (args
        .get("headers")
        .and_then(Value::as_object)
        .into_iter()
        .flatten())
    .map(|(k, v)| k.len() + v.as_str().map_or_else(|| v.to_string().len(), str::len))
    .sum();
    if spec.name == "ide_debug_request" && body.len() + headers_len > MAX_SHOWN_TEXT {
        return Err(format!(
            "the headers and body exceed {MAX_SHOWN_TEXT} characters — too much for the user to read in the confirm"
        ));
    }
    if spec.name == "ide_debug_request" {
        let url = args.get("url").and_then(Value::as_str).unwrap_or_default();
        if !is_loopback_url(url) {
            return Err(format!(
                "'{url}' is not on this Mac: ide_debug_request only reaches localhost, \
                 127.0.0.1 or [::1] over http(s)"
            ));
        }
    }
    Ok(())
}

/// Keep an `ide_debug_*` command inside the agent session's folder `cwd`:
/// `ide_debug_start`'s `root` (default `cwd`) is resolved through symlinks
/// and must lie in the resolved `cwd`; it is handed on as the session spells
/// its folder. Every file argument of every command (`target.file`,
/// `breakpoints` / `set` / `clear` `[].file`, `file`) must be a plain
/// relative path — no `:` (so no `file:` URI in any spelling), `\\`, NUL or
/// control character, `~`, absolute path or `..` — and must resolve, through
/// symlinks (its deepest existing ancestor when it does not exist), inside the
/// project: the start's root, else `debug_root` (the root of the session's
/// last start), else `cwd`. Other commands pass through.
pub fn confine_paths(
    spec: &UiCommandSpec,
    mut args: Value,
    cwd: &str,
    debug_root: Option<&str>,
) -> Result<Value, String> {
    use std::path::{Component, Path, PathBuf};
    if spec.module != "ide" {
        return Ok(args);
    }
    let canon = |p: &Path| std::fs::canonicalize(p).map_err(|e| format!("{}: {e}", p.display()));
    let home =
        canon(Path::new(cwd)).map_err(|e| format!("your session's folder is unknown ({e})"))?;
    let base: PathBuf = if spec.name == "ide_debug_start" {
        let asked = args.get("root").and_then(Value::as_str).unwrap_or(cwd);
        let root = canon(Path::new(asked))?;
        if !root.starts_with(&home) {
            return Err(format!(
                "'{asked}' is outside your session's folder {}",
                home.display()
            ));
        }
        // Checked canonically, handed on as the session spells its folder: the
        // IDE showing `/var/…` must not ask to "open" `/private/var/…`.
        let inside = root.strip_prefix(&home).unwrap_or(Path::new(""));
        let shown = Path::new(cwd.trim_end_matches('/')).join(inside);
        args["root"] = json!(shown.to_string_lossy().trim_end_matches('/'));
        root
    } else {
        match debug_root.map(|r| canon(Path::new(r))) {
            Some(Ok(r)) if r.starts_with(&home) => r,
            _ => home.clone(),
        }
    };
    let mut files: Vec<&str> = Vec::new();
    files.extend(args.pointer("/target/file").and_then(Value::as_str));
    files.extend(args.get("file").and_then(Value::as_str));
    for list in ["breakpoints", "set", "clear"] {
        for b in args
            .get(list)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            files.extend(b.get("file").and_then(Value::as_str));
        }
    }
    for f in files {
        let rel = Path::new(f);
        let plain = !f.is_empty()
            && !f.starts_with('~')
            && !f.chars().any(|c| c == ':' || c == '\\' || c.is_control())
            && rel
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
        if !plain {
            return Err(format!(
                "'{f}' must be a plain path inside the project, relative to it"
            ));
        }
        // The deepest part that exists decides where it really points.
        let mut probe = base.join(rel);
        let real = loop {
            if let Ok(real) = std::fs::canonicalize(&probe) {
                break real;
            }
            if !probe.pop() {
                break PathBuf::new();
            }
        };
        if !real.starts_with(&base) {
            return Err(format!("'{f}' resolves outside the project"));
        }
    }
    Ok(args)
}

fn is_loopback_url(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else {
        return false;
    };
    let host = u.host_str().unwrap_or_default();
    let host = host.trim_start_matches('[').trim_end_matches(']');
    matches!(u.scheme(), "http" | "https")
        && (host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback()))
}

/// The deadline the daemon gives the UI for `spec` with these arguments:
/// the catalog default, or for a statement carrying `timeout_ms` that + 15 s;
/// capped at [`MAX_TIMEOUT_MS`].
pub fn deadline_for(spec: &UiCommandSpec, args: &Value) -> std::time::Duration {
    let ms = match args.get("timeout_ms").and_then(Value::as_u64) {
        Some(t) if spec.input_schema["properties"].get("timeout_ms").is_some() => {
            t.saturating_add(15_000).max(spec.timeout_ms)
        }
        _ => spec.timeout_ms,
    };
    std::time::Duration::from_millis(ms.min(MAX_TIMEOUT_MS))
}

/// Validate `args` against a catalog `input_schema`. Supports what the catalog
/// uses: `type` (object/string/integer/number/boolean/array), `required`,
/// `properties`, `additionalProperties: false`, `enum`, `minimum`/`maximum`,
/// `minLength`/`maxLength`, `minItems`/`maxItems`, `maxProperties`, `items`. Returns the first
/// problem as a message the agent can act on. Pure.
pub fn validate_args(schema: &Value, args: &Value) -> Result<(), String> {
    check(schema, args, "arguments")
}

fn type_ok(ty: &str, v: &Value) -> bool {
    match ty {
        "object" => v.is_object(),
        "string" => v.is_string(),
        "integer" => v.as_i64().is_some() || v.as_u64().is_some(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "null" => v.is_null(),
        _ => true,
    }
}

fn check(schema: &Value, v: &Value, at: &str) -> Result<(), String> {
    // `type` is one name or a list of them (`["string", "number"]`).
    let types: Vec<&str> = match schema.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    if !types.is_empty() && !types.iter().any(|t| type_ok(t, v)) {
        return Err(format!("{at} must be of type {}", types.join("|")));
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.iter().any(|a| a == v) {
            let list = allowed
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!("{at} must be one of: {list}"));
        }
    }
    if let Some(n) = v.as_f64() {
        if let Some(min) = schema.get("minimum").and_then(Value::as_f64) {
            if n < min {
                return Err(format!("{at} must be ≥ {min}"));
            }
        }
        if let Some(max) = schema.get("maximum").and_then(Value::as_f64) {
            if n > max {
                return Err(format!("{at} must be ≤ {max}"));
            }
        }
    }
    if let Some(s) = v.as_str() {
        let len = s.chars().count() as u64;
        if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
            if len < min {
                return Err(format!("{at} must be at least {min} characters"));
            }
        }
        if let Some(max) = schema.get("maxLength").and_then(Value::as_u64) {
            if len > max {
                return Err(format!("{at} must be at most {max} characters"));
            }
        }
    }
    if let Some(items) = v.as_array() {
        let n = items.len() as u64;
        if let Some(min) = schema.get("minItems").and_then(Value::as_u64) {
            if n < min {
                return Err(format!("{at} needs at least {min} item(s)"));
            }
        }
        if let Some(max) = schema.get("maxItems").and_then(Value::as_u64) {
            if n > max {
                return Err(format!("{at} takes at most {max} item(s)"));
            }
        }
        if let Some(item_schema) = schema.get("items") {
            for (i, item) in items.iter().enumerate() {
                check(item_schema, item, &format!("{at}[{i}]"))?;
            }
        }
    }
    if let Some(obj) = v.as_object() {
        if let Some(max) = schema.get("maxProperties").and_then(Value::as_u64) {
            if obj.len() as u64 > max {
                return Err(format!("{at} takes at most {max} entries"));
            }
        }
        let props = schema.get("properties").and_then(Value::as_object);
        if let Some(req) = schema.get("required").and_then(Value::as_array) {
            for k in req.iter().filter_map(Value::as_str) {
                if !obj.contains_key(k) {
                    return Err(format!("missing required argument '{k}'"));
                }
            }
        }
        let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
        for (k, val) in obj {
            match props.and_then(|p| p.get(k)) {
                Some(sub) => check(sub, val, k)?,
                None if closed => {
                    let known = props
                        .map(|p| p.keys().cloned().collect::<Vec<_>>().join(", "))
                        .unwrap_or_default();
                    return Err(format!(
                        "unknown argument '{k}' (accepted: {})",
                        if known.is_empty() { "none" } else { &known }
                    ));
                }
                None => {}
            }
        }
    }
    Ok(())
}

/// Every structural rule a catalog entry must satisfy. Returns all problems.
/// Pure; run by the tests on the embedded file (and usable by tooling).
pub fn catalog_problems(version: u32, cmds: &[UiCommandSpec]) -> Vec<String> {
    let mut out = Vec::new();
    if version != 1 {
        out.push(format!("unsupported catalog version {version}"));
    }
    let mut seen = std::collections::HashSet::new();
    for c in cmds {
        let n = &c.name;
        if !seen.insert(n.clone()) {
            out.push(format!("{n}: duplicate name"));
        }
        if n.is_empty()
            || !n
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            || n.starts_with('_')
            || n.ends_with('_')
        {
            out.push(format!("{n}: name must be snake_case [a-z0-9_]"));
        }
        if !KNOWN_MODULES.contains(&c.module.as_str()) {
            out.push(format!("{n}: unknown module '{}'", c.module));
        }
        if c.module == "shell" {
            if !c.route.is_empty() {
                out.push(format!("{n}: a shell command has no route"));
            }
        } else if pane_key(&c.route) != c.module {
            out.push(format!(
                "{n}: route '{}' belongs to '{}', not module '{}'",
                c.route,
                pane_key(&c.route),
                c.module
            ));
        }
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&c.timeout_ms) {
            out.push(format!(
                "{n}: timeout_ms {} outside {MIN_TIMEOUT_MS}..={MAX_TIMEOUT_MS}",
                c.timeout_ms
            ));
        }
        if let Some(h) = &c.headless {
            if c.risk != Risk::Read {
                out.push(format!("{n}: headless is only allowed on a read command"));
            }
            if !HEADLESS.contains(&h.as_str()) {
                out.push(format!("{n}: unknown headless fallback '{h}'"));
            }
        }
        if let Some(p) = &c.open_placement {
            if !OPEN_PLACEMENTS.contains(&p.as_str()) {
                out.push(format!("{n}: unknown open_placement '{p}'"));
            }
            if c.module == "shell" {
                out.push(format!("{n}: a shell command is never auto-opened"));
            }
        }
        if c.description.trim().len() < 20 {
            out.push(format!("{n}: description too short"));
        }
        let s = &c.input_schema;
        if s.get("type") != Some(&json!("object")) {
            out.push(format!("{n}: input_schema.type must be \"object\""));
        }
        if s.get("additionalProperties") != Some(&Value::Bool(false)) {
            out.push(format!(
                "{n}: input_schema.additionalProperties must be false"
            ));
        }
        let props = s.get("properties").and_then(Value::as_object);
        if props.is_none() {
            out.push(format!("{n}: input_schema.properties must be an object"));
        }
        if let Some(req) = s.get("required") {
            match req.as_array() {
                Some(list) => {
                    for r in list {
                        match r.as_str() {
                            Some(k) if props.is_some_and(|p| p.contains_key(k)) => {}
                            _ => out.push(format!(
                                "{n}: required '{}' is not a property",
                                r.as_str().map_or_else(|| r.to_string(), str::to_string)
                            )),
                        }
                    }
                }
                None => out.push(format!("{n}: input_schema.required must be an array")),
            }
        }
        if let Some(p) = props {
            for (k, sub) in p {
                if sub.get("type").and_then(Value::as_str).is_none() {
                    out.push(format!("{n}: property '{k}' has no type"));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_parses_and_is_valid() {
        let (version, cmds) = parse(CATALOG_JSON).expect("ui-commands.json parses");
        assert!(!cmds.is_empty(), "catalog is empty");
        let problems = catalog_problems(version, &cmds);
        assert!(
            problems.is_empty(),
            "catalog problems:\n{}",
            problems.join("\n")
        );
        assert_eq!(catalog().len(), cmds.len());
    }

    #[test]
    fn phase_one_commands_exist() {
        for n in [
            "state",
            "open",
            "focus",
            "db_list_connections",
            "db_open_connection",
            "db_new_tab",
            "db_set_statement",
            "db_run_query",
            "db_get_result",
            "db_page",
            "db_set_view",
            "db_open_object",
            "db_explain",
            "db_stop",
            "db_export",
        ] {
            assert!(get(n).is_some(), "missing catalog command {n}");
        }
        assert_eq!(
            get("db_run_query").unwrap().headless.as_deref(),
            Some("db_mcp_query")
        );
        assert_eq!(
            get("db_list_connections").unwrap().headless.as_deref(),
            Some("db_list_connections")
        );
        assert_eq!(get("state").unwrap().headless.as_deref(), Some("presence"));
        assert_eq!(by_tool("ui_db_run_query").unwrap().name, "db_run_query");
        assert!(by_tool("db_run_query").is_none());
        assert!(!is_ui_tool("ui_nope"));
    }

    #[test]
    fn specs_mirror_the_catalog() {
        let specs = specs();
        assert_eq!(specs.len(), catalog().len());
        for (s, c) in specs.iter().zip(catalog()) {
            assert_eq!(s["name"], json!(format!("otto.ui_{}", c.name)));
            assert_eq!(s["category"], json!(CATEGORY));
            assert_eq!(s["mutating"], json!(c.risk.mutating()));
            assert_eq!(s["inputSchema"], c.input_schema);
        }
    }

    #[test]
    fn pane_key_matches_the_ui() {
        assert_eq!(pane_key(""), "agents");
        assert_eq!(pane_key("#/"), "agents");
        assert_eq!(pane_key("database"), "connections");
        assert_eq!(pane_key("/database/x"), "connections");
        assert_eq!(pane_key("brokers"), "connections");
        assert_eq!(pane_key("canvas/abc"), "product");
        assert_eq!(pane_key("design/abc"), "product");
        assert_eq!(pane_key("personal-agents"), "scheduled-tasks");
        assert_eq!(pane_key("plugin/foo/bar"), "plugin/foo");
        assert_eq!(pane_key("git/repo?x=1"), "git");
        assert_eq!(pane_key("kubernetes"), "kubernetes");
        assert_eq!(pane_key("scheduled-tasks"), "scheduled-tasks");
        assert_eq!(pane_key("workbench"), "ide");
        assert_eq!(pane_key("ide"), "ide");
        assert_eq!(pane_key("threads"), "threads");
    }

    fn spec(
        name: &str,
        module: &str,
        route: &str,
        risk: Risk,
        headless: Option<&str>,
    ) -> UiCommandSpec {
        UiCommandSpec {
            name: name.into(),
            module: module.into(),
            route: route.into(),
            risk,
            timeout_ms: 10_000,
            headless: headless.map(str::to_string),
            open_placement: None,
            description: "A long enough description for the check.".into(),
            input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            comment: None,
        }
    }

    #[test]
    fn catalog_problems_catch_each_rule() {
        let ok = spec(
            "x",
            "connections",
            "database",
            Risk::Read,
            Some("db_mcp_query"),
        );
        assert!(catalog_problems(1, std::slice::from_ref(&ok)).is_empty());
        let has = |cmds: &[UiCommandSpec], needle: &str| {
            let p = catalog_problems(1, cmds);
            assert!(
                p.iter().any(|m| m.contains(needle)),
                "{needle} not in {p:?}"
            );
        };
        has(&[ok.clone(), ok.clone()], "duplicate name");
        has(
            &[spec(
                "Bad-Name",
                "connections",
                "database",
                Risk::Read,
                None,
            )],
            "snake_case",
        );
        has(
            &[spec("x", "nowhere", "nowhere", Risk::Read, None)],
            "unknown module",
        );
        has(
            &[spec("x", "git", "database", Risk::Read, None)],
            "belongs to",
        );
        has(&[spec("x", "shell", "git", Risk::Read, None)], "no route");
        has(
            &[spec(
                "x",
                "connections",
                "database",
                Risk::Navigate,
                Some("db_mcp_query"),
            )],
            "only allowed on a read",
        );
        has(
            &[spec(
                "x",
                "connections",
                "database",
                Risk::Read,
                Some("rm_rf"),
            )],
            "unknown headless",
        );
        let mut t = ok.clone();
        t.timeout_ms = 500;
        has(&[t], "timeout_ms");
        let mut t = ok.clone();
        t.timeout_ms = 999_999;
        has(&[t], "timeout_ms");
        let mut s = ok.clone();
        s.input_schema = json!({"type":"object","properties":{}});
        has(&[s], "additionalProperties");
        let mut s = ok.clone();
        s.input_schema = json!({"type":"array","additionalProperties":false,"properties":{}});
        has(&[s], "type must be");
        let mut s = ok.clone();
        s.input_schema =
            json!({"type":"object","additionalProperties":false,"properties":{},"required":["a"]});
        has(&[s], "required 'a'");
        let mut s = ok.clone();
        s.input_schema =
            json!({"type":"object","additionalProperties":false,"properties":{"a":{}}});
        has(&[s], "no type");
        assert!(!catalog_problems(2, &[]).is_empty());
    }

    #[test]
    fn unknown_fields_in_the_file_are_rejected() {
        let bad = r#"{"version":1,"commands":[{"name":"x","module":"shell","route":"","risk":"read",
            "timeout_ms":5000,"description":"long enough description here","input_schema":{},"surprise":1}]}"#;
        assert!(parse(bad).is_err());
        let bad_risk = r#"{"version":1,"commands":[{"name":"x","module":"shell","route":"","risk":"nuke",
            "timeout_ms":5000,"description":"long enough description here","input_schema":{}}]}"#;
        assert!(parse(bad_risk).is_err());
    }

    #[test]
    fn validate_args_enforces_the_schema() {
        let s = &get("db_run_query").unwrap().input_schema;
        assert!(validate_args(s, &json!({"tab_id":"t","statement":"select 1"})).is_ok());
        assert!(validate_args(s, &json!({})).is_ok());
        let e = validate_args(s, &json!({"tab_id": 5})).unwrap_err();
        assert!(e.contains("tab_id must be of type string"), "{e}");
        let e = validate_args(s, &json!({"sql":"x"})).unwrap_err();
        assert!(e.contains("unknown argument 'sql'"), "{e}");
        let e = validate_args(s, &json!({"row_limit": 0})).unwrap_err();
        assert!(e.contains("≥"), "{e}");
        let e = validate_args(s, &json!({"timeout_ms": 1.5})).unwrap_err();
        assert!(e.contains("integer"), "{e}");
        let e = validate_args(s, &json!([1])).unwrap_err();
        assert!(e.contains("object"), "{e}");

        let s = &get("db_set_statement").unwrap().input_schema;
        let e = validate_args(s, &json!({"tab_id":"t"})).unwrap_err();
        assert!(e.contains("missing required argument 'statement'"), "{e}");

        let s = &get("db_page").unwrap().input_schema;
        assert!(validate_args(s, &json!({"tab_id":"t","delta":-1})).is_ok());
        let e = validate_args(s, &json!({"tab_id":"t","delta":2})).unwrap_err();
        assert!(e.contains("≤"), "{e}");

        let s = &get("db_open_object").unwrap().input_schema;
        let e = validate_args(s, &json!({"connection_id":"c","path":[]})).unwrap_err();
        assert!(e.contains("at least 1"), "{e}");
        let e = validate_args(s, &json!({"connection_id":"c","path":["a", 1]})).unwrap_err();
        assert!(e.contains("path[1]"), "{e}");
    }

    #[test]
    fn mutating_follows_risk() {
        assert!(!Risk::Read.mutating());
        assert!(!Risk::Navigate.mutating());
        assert!(Risk::LocalWrite.mutating());
        assert!(Risk::Outward.mutating());
        assert!(Risk::Execute.mutating());
        assert!(get("db_export").unwrap().risk.mutating());
    }

    #[test]
    fn ide_debug_commands_have_their_tiers() {
        let risk = |n: &str| get(n).unwrap_or_else(|| panic!("missing {n}")).risk;
        // Anything that runs code outside the agent's sandbox is its own tier.
        assert_eq!(risk("ide_debug_start"), Risk::Execute);
        assert_eq!(risk("ide_debug_evaluate"), Risk::Execute);
        assert_eq!(risk("ide_debug_control"), Risk::Navigate);
        assert_eq!(risk("ide_debug_breakpoints"), Risk::LocalWrite);
        assert_eq!(risk("ide_debug_inspect"), Risk::Read);
        assert_eq!(risk("ide_debug_request"), Risk::LocalWrite);
        assert_eq!(risk("ide_debug_db"), Risk::Read);
        // Restart is a new launch: only start can do it, so it always confirms.
        let control = &get("ide_debug_control").unwrap().input_schema;
        let e = validate_args(control, &json!({"action": "restart"})).unwrap_err();
        assert!(e.contains("must be one of"), "{e}");
        // Evaluating is not a read: inspect takes no expression.
        let inspect = &get("ide_debug_inspect").unwrap().input_schema;
        let e = validate_args(inspect, &json!({"expression": "os.Exit(1)"})).unwrap_err();
        assert!(e.contains("unknown argument 'expression'"), "{e}");
        // Every ide_debug_* call fits the bridge's cap, and start opens a window.
        for c in catalog()
            .iter()
            .filter(|c| c.name.starts_with("ide_debug_"))
        {
            assert_eq!(c.module, "ide", "{}", c.name);
            assert!(c.timeout_ms <= MAX_TIMEOUT_MS, "{}", c.name);
            let wait = c.input_schema["properties"]["wait_ms"]["maximum"].as_u64();
            assert!(
                wait.is_none_or(|w| w + 15_000 <= c.timeout_ms),
                "{}",
                c.name
            );
        }
        assert_eq!(
            get("ide_debug_start").unwrap().open_placement.as_deref(),
            Some("window")
        );
        assert!(specs()
            .iter()
            .find(|s| s["name"] == "otto.ui_ide_debug_evaluate")
            .unwrap()["description"]
            .as_str()
            .unwrap()
            .contains("runs code on the user's Mac"));
    }

    #[test]
    fn open_placement_is_checked() {
        let mut s = spec("x", "ide", "ide", Risk::Read, None);
        s.open_placement = Some("window".into());
        assert!(catalog_problems(1, std::slice::from_ref(&s)).is_empty());
        s.open_placement = Some("main".into());
        let p = catalog_problems(1, std::slice::from_ref(&s));
        assert!(
            p.iter().any(|m| m.contains("unknown open_placement")),
            "{p:?}"
        );
        let mut s = spec("x", "shell", "", Risk::Read, None);
        s.open_placement = Some("side".into());
        let p = catalog_problems(1, std::slice::from_ref(&s));
        assert!(p.iter().any(|m| m.contains("never auto-opened")), "{p:?}");
    }

    #[test]
    fn debug_start_refuses_loader_and_toolchain_env() {
        let start = get("ide_debug_start").unwrap();
        let ok = json!({"target": {"kind": "test", "file": "a_test.go", "name": "TestA"},
            "env": {"LOG_LEVEL": "debug", "APP_ENV": "test", "GOOGLE_CLOUD_PROJECT": "p"}});
        assert_eq!(validate_command(start, &ok), Ok(()));
        for bad in [
            "GOFLAGS",
            "goflags",
            "DYLD_INSERT_LIBRARIES",
            "LD_PRELOAD",
            "NODE_OPTIONS",
            "PYTHONPATH",
            "PYTHONSTARTUP",
            "JAVA_TOOL_OPTIONS",
            "_JAVA_OPTIONS",
            "BASH_ENV",
            "PATH",
            "DLV_LISTEN",
            "CGO_LDFLAGS",
            "RUSTC_WRAPPER",
            "GIT_SSH_COMMAND",
        ] {
            let args = json!({"target": {"kind": "main", "file": "main.go"}, "env": {bad: "x"}});
            let e = validate_command(start, &args).unwrap_err();
            assert!(e.contains("is refused"), "{bad}: {e}");
        }
        let e = validate_command(start, &json!({"target": {"kind": "main"}, "env": {"A": 1}}))
            .unwrap_err();
        assert!(e.contains("must be a string"), "{e}");
        // The schema still runs first.
        let e = validate_command(start, &json!({"target": {"kind": "attach"}})).unwrap_err();
        assert!(e.contains("must be one of"), "{e}");
    }

    #[test]
    fn array_types_are_validated() {
        // C1-038: `"type": [..]` was skipped, so any value passed.
        let s = json!({"type": "object", "additionalProperties": false, "properties": {
            "v": {"type": ["string", "number", "boolean"]}}});
        for ok in [json!("a"), json!(1), json!(1.5), json!(true)] {
            assert_eq!(validate_args(&s, &json!({"v": ok})), Ok(()), "{ok}");
        }
        for bad in [json!({}), json!([1]), json!(null)] {
            let e = validate_args(&s, &json!({"v": bad})).unwrap_err();
            assert!(e.contains("v must be of type string|number|boolean"), "{e}");
        }
        // The real catalog entry that uses it.
        let db = &get("ide_debug_db").unwrap().input_schema;
        let args = json!({"action": "watch", "objects": [{"object": "orders",
            "filter": {"column": "id", "value": {"$gt": 1}}}]});
        assert!(validate_args(db, &args).is_err());
    }

    #[test]
    fn debug_start_refuses_build_flag_and_hijack_env() {
        // C1-002 / C1-019: variables the launch code or the tools it runs read
        // as code, credentials or endpoints.
        let start = get("ide_debug_start").unwrap();
        let with_env = |k: &str, v: &str| json!({"target": {"kind": "main", "file": "main.go"}, "env": {k: v}});
        for bad in [
            "GO_BUILD_FLAGS",
            "OTTO_JS_ADAPTER",
            "OTTO_JAVA_DEBUG_DAP_PORT",
            "AWS_CONFIG_FILE",
            "AWS_SHARED_CREDENTIALS_FILE",
            "KUBECONFIG",
            "DOCKER_HOST",
            "DOCKER_CONFIG",
            "TESTCONTAINERS_HOST_OVERRIDE",
            "RUSTUP_TOOLCHAIN",
            "RUSTUP_HOME",
            "VIRTUAL_ENV",
            "GOCACHE",
            "GOMODCACHE",
            "HTTP_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "CURL_CA_BUNDLE",
            "REQUESTS_CA_BUNDLE",
            "HOME",
            "XDG_CONFIG_HOME",
            "SHELLOPTS",
            "BASHOPTS",
            "BASH_FUNC_ls%%",
            "FPATH",
            "DOTNET_STARTUP_HOOKS",
            "CORECLR_PROFILER",
            "COMPLUS_X",
            "MAKEFLAGS",
        ] {
            let e = validate_command(start, &with_env(bad, "x")).unwrap_err();
            assert!(
                e.contains("is refused") || e.contains("not a plain name"),
                "{bad}: {e}"
            );
        }
        // Key syntax, value size and invisible characters.
        for bad_key in ["A B", "1A", "A=B", "", "A-B", &"A".repeat(129)] {
            let e = validate_command(start, &with_env(bad_key, "x")).unwrap_err();
            assert!(e.contains("not a plain name"), "{bad_key:?}: {e}");
        }
        for bad_val in [
            "a\nb",
            "a\u{0}b",
            "x\u{202e}gpj.exe",
            "a\u{2066}b",
            "a\u{200b}b",
            "\u{feff}x",
        ] {
            let e = validate_command(start, &with_env("APP", bad_val)).unwrap_err();
            assert!(e.contains("invisible or control"), "{bad_val:?}: {e}");
        }
        let e = validate_command(start, &with_env("APP", &"x".repeat(4097))).unwrap_err();
        assert!(e.contains("4096"), "{e}");
        assert_eq!(
            validate_command(start, &with_env("APP", &"x".repeat(4096))),
            Ok(())
        );
        let many: serde_json::Map<String, Value> =
            (0..51).map(|i| (format!("V{i}"), json!("x"))).collect();
        let e =
            validate_command(start, &json!({"target": {"kind": "main"}, "env": many})).unwrap_err();
        assert!(e.contains("at most 50"), "{e}");
        // Ordinary app settings still pass (GOOGLE_* is not GO_*).
        let ok = json!({"target": {"kind": "main"}, "env": {"LOG_LEVEL": "debug",
            "GOOGLE_CLOUD_PROJECT": "p", "DB_HOST": "127.0.0.1", "_X": "a b"}});
        assert_eq!(validate_command(start, &ok), Ok(()));
    }

    #[test]
    fn debug_start_refuses_build_tool_env() {
        // C2-003: names Otto's Java / Rust / Maven builders or the tools they
        // run read as a repository, a wrapper, a config or credentials.
        let start = get("ide_debug_start").unwrap();
        for bad in [
            "MAVEN_REPO_LOCAL",
            "M2_REPO",
            "MAVEN_ARGS",
            "GRADLE_USER_HOME",
            "RUSTC_WORKSPACE_WRAPPER",
            "OPENSSL_CONF",
            "OPENSSL_ENGINES",
            "OPENSSL_MODULES",
            "AWS_PROFILE",
            "AWS_ENDPOINT_URL",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "SSH_ASKPASS",
            "SUDO_ASKPASS",
            "NO_PROXY",
            "TMPDIR",
            "SSH_AUTH_SOCK",
            "DEVELOPER_DIR",
            "FLUTTER_ROOT",
            "USER",
            "LOGNAME",
            "PWD",
        ] {
            let args = json!({"target": {"kind": "main", "file": "Main.java"}, "env": {bad: "x"}});
            let e = validate_command(start, &args).unwrap_err();
            assert!(e.contains("is refused"), "{bad}: {e}");
        }
    }

    /// Env names the IDE engine reads that an agent may still set: each one
    /// was looked at and changes no code, repository, endpoint or credential.
    const REVIEWED_ENV_READS: &[&str] = &[
        // Database containers' own variables (DB Watch reads them from a container).
        "CLICKHOUSE_DB",
        "CLICKHOUSE_PASSWORD",
        "CLICKHOUSE_USER",
        "MARIADB_DATABASE",
        "MARIADB_PASSWORD",
        "MARIADB_ROOT_PASSWORD",
        "MARIADB_USER",
        "MONGO_INITDB_DATABASE",
        "MONGO_INITDB_ROOT_PASSWORD",
        "MONGO_INITDB_ROOT_USERNAME",
        "MONGODB_DATABASE",
        "MYSQL_DATABASE",
        "MYSQL_PASSWORD",
        "MYSQL_ROOT_PASSWORD",
        "MYSQL_USER",
        "POSTGRES_DB",
        "POSTGRES_PASSWORD",
        "POSTGRES_USER",
        "REDIS_PASSWORD",
        // Runtime tuning and app settings.
        "GOGC",
        "GOMEMLIMIT",
        "RA_LRU_CAP",
        "SHLVL",
        "SPRING_PROFILES_ACTIVE",
        // Fixture names inside the shell-env probe's tests.
        "FAKE_VAR",
        "GO_DEBUGGING_PRODAMS",
        "MULTILINE_CERT",
        "TRAILING",
    ];

    /// Every env name the IDE engine (launch, builders, adapters, toolchains)
    /// looks up by literal is either refused to an agent or reviewed above —
    /// a new lookup fails here until someone decides (C2-003).
    #[test]
    #[allow(clippy::disallowed_methods)] // a sync test walking source files
    fn every_env_name_the_ide_reads_is_decided() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs")
                    && !p.to_string_lossy().contains("test")
                {
                    out.push(p);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../otto-ide/src");
        let mut files = Vec::new();
        walk(&root, &mut files);
        assert!(files.len() > 50, "otto-ide sources not found at {root:?}");
        let mut undecided = std::collections::BTreeSet::new();
        let mut seen = 0;
        for f in &files {
            let text = std::fs::read_to_string(f).unwrap();
            for call in ["get(", "var(", "var_os(", "contains_key(", "remove("] {
                for (i, _) in text.match_indices(call) {
                    let rest = text[i + call.len()..].trim_start();
                    let Some(rest) = rest.strip_prefix('"') else {
                        continue;
                    };
                    let name: String = rest.chars().take_while(|c| *c != '"').collect();
                    let env_like = name.len() >= 3
                        && name.starts_with(|c: char| c.is_ascii_uppercase())
                        && name
                            .chars()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
                    if !env_like {
                        continue;
                    }
                    seen += 1;
                    if !env_denied(&name) && !REVIEWED_ENV_READS.contains(&name.as_str()) {
                        undecided.insert(format!(
                            "{name} ({})",
                            f.strip_prefix(&root).unwrap().display()
                        ));
                    }
                }
            }
        }
        assert!(
            seen > 30,
            "the scan found only {seen} env reads — is it still looking?"
        );
        assert!(
            undecided.is_empty(),
            "decide (deny, or add to REVIEWED_ENV_READS): {undecided:#?}"
        );
    }

    #[test]
    fn build_tags_are_plain_tag_names() {
        // C1-002: a build tag is the only build input an agent gives; it must
        // not smuggle a flag (`-toolexec=…`) or a list separator.
        let start = get("ide_debug_start").unwrap();
        let tags =
            |t: Value| json!({"target": {"kind": "test", "file": "a_test.go"}, "build_tags": t});
        assert_eq!(
            validate_command(start, &tags(json!(["component", "go1.22", "linux_amd64"]))),
            Ok(())
        );
        for bad in [
            "-toolexec=/bin/sh",
            "a b",
            "a,b",
            "x\u{202e}",
            "a=b",
            "!a",
            "a/b",
        ] {
            let e = validate_command(start, &tags(json!([bad]))).unwrap_err();
            assert!(e.contains("build tag"), "{bad:?}: {e}");
        }
    }

    #[test]
    fn debug_request_only_reaches_this_mac() {
        let req = get("ide_debug_request").unwrap();
        for ok in [
            "http://localhost:8080/orders",
            "https://LOCALHOST/x",
            "http://127.0.0.1:3000",
            "http://127.0.0.2:3000",
            "http://[::1]:8080/a?b=c",
        ] {
            assert_eq!(validate_command(req, &json!({"url": ok})), Ok(()), "{ok}");
        }
        for bad in [
            "http://example.com/",
            "http://localhost.evil.com/",
            "http://10.0.0.5:8080/",
            "http://169.254.169.254/latest/meta-data",
            "file:///etc/passwd",
            "ftp://localhost/",
            "localhost:8080/orders",
            "http://user@0x7f.example/",
        ] {
            let e = validate_command(req, &json!({"url": bad})).unwrap_err();
            assert!(e.contains("not on this Mac"), "{bad}: {e}");
        }
        // Other commands are untouched by the URL rule.
        let send = get("api_send").unwrap();
        assert_eq!(validate_command(send, &json!({})), Ok(()));
    }

    #[test]
    fn ide_paths_stay_inside_the_sessions_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("app");
        let other = tmp.path().join("other");
        std::fs::create_dir_all(home.join("svc")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("evil.go"), "package main").unwrap();
        std::fs::write(home.join("svc/main.go"), "package main").unwrap();
        std::os::unix::fs::symlink(&other, home.join("link")).unwrap();
        std::os::unix::fs::symlink(other.join("evil.go"), home.join("svc/evil.go")).unwrap();
        let cwd = home.to_string_lossy().into_owned();
        let sub = home.join("svc").to_string_lossy().into_owned();
        let start = get("ide_debug_start").unwrap();
        // Default root = the session folder, as the session spells it.
        let ok = confine_paths(
            start,
            json!({"target": {"kind": "main", "file": "main.go"}}),
            &cwd,
            None,
        )
        .unwrap();
        assert_eq!(ok["root"], json!(cwd));
        let file_uri = format!("file:{}", other.join("evil.go").display());
        let file_uri2 = format!("file://{}", other.join("evil.go").display());
        let abs = other.join("evil.go").to_string_lossy().into_owned();
        // (command, args, the start's root for later commands, allowed)
        let start_main = |file: &str| json!({"target": {"kind": "main", "file": file}});
        let bp = |file: &str| json!({"set": [{"file": file, "line": 1}]});
        let clear = |file: &str| json!({"clear": [{"file": file, "line": 1}]});
        let to_line = |file: &str| json!({"action": "run_to_line", "file": file, "line": 1});
        let table: Vec<(&str, Value, Option<&str>, bool)> = vec![
            ("ide_debug_start", start_main("main.go"), None, true),
            (
                "ide_debug_start",
                json!({"root": sub, "target": {"kind": "main", "file": "main.go"}}),
                None,
                true,
            ),
            (
                "ide_debug_start",
                json!({"root": home.join("link"), "target": {"kind": "main", "file": "x.go"}}),
                None,
                false,
            ),
            (
                "ide_debug_start",
                json!({"root": other, "target": {"kind": "main", "file": "x.go"}}),
                None,
                false,
            ),
            ("ide_debug_start", start_main(&abs), None, false),
            ("ide_debug_start", start_main(&file_uri), None, false),
            ("ide_debug_start", start_main(&file_uri2), None, false),
            (
                "ide_debug_start",
                start_main("../other/evil.go"),
                None,
                false,
            ),
            ("ide_debug_start", start_main("svc/evil.go"), None, false),
            (
                "ide_debug_start",
                start_main("link/new_file.go"),
                None,
                false,
            ),
            (
                "ide_debug_start",
                start_main("svc\\..\\..\\other"),
                None,
                false,
            ),
            ("ide_debug_start", start_main("~/x.go"), None, false),
            ("ide_debug_start", start_main("a\u{0}b.go"), None, false),
            ("ide_debug_start", start_main("SVC/main.go"), None, true),
            (
                "ide_debug_start",
                json!({"target": {"kind": "main", "file": "main.go"}, "breakpoints": [{"file": "/etc/hosts", "line": 1}]}),
                None,
                false,
            ),
            ("ide_debug_breakpoints", bp("svc/main.go"), None, true),
            (
                "ide_debug_breakpoints",
                bp("main.go"),
                Some(sub.as_str()),
                true,
            ),
            (
                "ide_debug_breakpoints",
                bp("evil.go"),
                Some(sub.as_str()),
                false,
            ),
            ("ide_debug_breakpoints", bp("link/evil.go"), None, false),
            ("ide_debug_breakpoints", bp(&file_uri), None, false),
            ("ide_debug_breakpoints", bp("/etc/hosts"), None, false),
            ("ide_debug_breakpoints", clear("link/evil.go"), None, false),
            ("ide_debug_control", to_line("svc/evil.go"), None, false),
            ("ide_debug_control", to_line("../x.go"), None, false),
            ("ide_debug_control", to_line("svc/main.go"), None, true),
        ];
        for (name, args, last_root, allowed) in table {
            let got = confine_paths(get(name).unwrap(), args.clone(), &cwd, last_root);
            assert_eq!(
                got.is_ok(),
                allowed,
                "{name} {args} (root {last_root:?}): {got:?}"
            );
        }
        // Other modules pass through untouched.
        let other_cmd = get("db_run_query").unwrap();
        assert!(confine_paths(other_cmd, json!({"x": "/etc"}), "/nonexistent", None).is_ok());
    }

    #[test]
    fn confirm_text_budget_refuses_instead_of_clipping() {
        let start = get("ide_debug_start").unwrap();
        let target = json!({"kind": "main", "file": "main.go"});
        let args: Vec<String> = (0..10).map(|_| "a".repeat(900)).collect();
        assert!(validate_command(start, &json!({"target": target, "args": args})).is_err());
        let env: serde_json::Map<String, Value> = (0..3)
            .map(|i| (format!("K{i}"), json!("v".repeat(3000))))
            .collect();
        assert!(validate_command(start, &json!({"target": target, "env": env})).is_err());
        assert!(validate_command(start, &json!({"target": target, "args": ["-v"]})).is_ok());
        let req = get("ide_debug_request").unwrap();
        let url = "http://localhost:8080/x";
        assert!(validate_command(req, &json!({"url": url, "body": "b".repeat(9000)})).is_err());
        assert!(validate_command(req, &json!({"url": url, "body": "{}"})).is_ok());
        // C3-004: header names + values count with the body (refused, not cut)…
        let big: serde_json::Map<String, Value> = (0..3)
            .map(|i| (format!("X-H{i}"), json!("v".repeat(3000))))
            .collect();
        let r = validate_command(req, &json!({"url": url, "headers": big, "body": "{}"}));
        assert!(r.unwrap_err().contains("headers"));
        let half = json!({"X-A": "v".repeat(4000)});
        assert!(validate_command(
            req,
            &json!({"url": url, "headers": half, "body": "b".repeat(4500)})
        )
        .is_err());
        // …and there are at most 50 of them.
        let many: serde_json::Map<String, Value> =
            (0..51).map(|i| (format!("X-{i}"), json!("1"))).collect();
        assert!(validate_command(req, &json!({"url": url, "headers": many})).is_err());
        let few = json!({"Content-Type": "application/json", "Authorization": "Bearer t"});
        assert!(validate_command(req, &json!({"url": url, "headers": few, "body": "{}"})).is_ok());
    }
}
