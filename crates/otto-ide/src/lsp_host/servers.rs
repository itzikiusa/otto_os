//! LSP server registry for code languages (Go, Python, TS, JS, Java, Rust,
//! Shell, CSS, HTML) + runtime IDE plugin servers, with `<data_dir>/toolchains/**/bin`,
//! `~/.otto/toolchains/**/bin`, and standard developer `bin` resolution.
//!
//! Lightweight data and document formats (`markdown`, `csv`, `sql`, `json`,
//! `yaml`, `toml`, `xml`) use Otto's built-in fast static intelligence and
//! never spawn external LSP subprocesses.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

pub use super::servers_root::{
    is_lightweight_non_lsp_lang, is_multi_repo_umbrella_dir, resolve_lsp_module_root,
};
use crate::plugins::IdePluginRegistry;
use crate::types::PluginLspServerSpec;

/// One entry in the built-in registry: a language and how to launch its server.
pub struct ServerDef {
    pub lang: &'static str,
    pub cmd: &'static str,
    pub args: &'static [&'static str],
    pub fallback_cmd: Option<&'static str>,
    pub fallback_args: &'static [&'static str],
    pub extensions: &'static [&'static str],
    pub install_command: Option<&'static str>,
}

/// Normalize language aliases (`sh`/`bash`/`zsh` -> `shell`, `tsx` -> `typescript`, etc.).
pub fn normalize_lsp_lang(lang: &str) -> &str {
    match lang.trim() {
        "sh" | "bash" | "zsh" | "shellscript" => "shell",
        "jsonc" | "jsonl" | "ndjson" => "json",
        "md" | "mdx" => "markdown",
        "yml" => "yaml",
        "tsx" | "typescriptreact" => "typescript",
        "jsx" | "javascriptreact" => "javascript",
        other => other,
    }
}

/// Built-in external LSP registry for full programming languages.
pub static REGISTRY: &[ServerDef] = &[
    ServerDef {
        lang: "go",
        cmd: "gopls",
        args: &[],
        fallback_cmd: None,
        fallback_args: &[],
        extensions: &[".go"],
        install_command: Some("go install golang.org/x/tools/gopls@latest"),
    },
    ServerDef {
        lang: "python",
        cmd: "pyright-langserver",
        args: &["--stdio"],
        fallback_cmd: Some("pylsp"),
        fallback_args: &[],
        extensions: &[".py", ".pyi"],
        install_command: Some("npm install -g pyright"),
    },
    ServerDef {
        lang: "typescript",
        cmd: "typescript-language-server",
        args: &["--stdio"],
        fallback_cmd: Some("vtsls"),
        fallback_args: &["--stdio"],
        extensions: &[".ts", ".tsx", ".mts", ".cts"],
        install_command: Some("npm install -g typescript-language-server typescript"),
    },
    ServerDef {
        lang: "javascript",
        cmd: "typescript-language-server",
        args: &["--stdio"],
        fallback_cmd: Some("vtsls"),
        fallback_args: &["--stdio"],
        extensions: &[".js", ".jsx", ".mjs", ".cjs"],
        install_command: Some("npm install -g typescript-language-server typescript"),
    },
    ServerDef {
        lang: "java",
        cmd: "jdtls",
        args: &[],
        fallback_cmd: None,
        fallback_args: &[],
        extensions: &[".java"],
        install_command: Some("brew install jdtls"),
    },
    ServerDef {
        lang: "rust",
        cmd: "rust-analyzer",
        args: &[],
        fallback_cmd: None,
        fallback_args: &[],
        extensions: &[".rs"],
        install_command: Some("rustup component add rust-analyzer"),
    },
    ServerDef {
        lang: "shell",
        cmd: "bash-language-server",
        args: &["start"],
        fallback_cmd: None,
        fallback_args: &[],
        extensions: &[".sh", ".bash", ".zsh", ".bats"],
        install_command: Some("npm install -g bash-language-server"),
    },
    ServerDef {
        lang: "css",
        cmd: "vscode-css-language-server",
        args: &["--stdio"],
        fallback_cmd: Some("css-languageserver"),
        fallback_args: &["--stdio"],
        extensions: &[".css", ".scss", ".less"],
        install_command: Some("npm install -g vscode-langservers-extracted"),
    },
    ServerDef {
        lang: "html",
        cmd: "vscode-html-language-server",
        args: &["--stdio"],
        fallback_cmd: Some("html-languageserver"),
        fallback_args: &["--stdio"],
        extensions: &[".html", ".htm"],
        install_command: Some("npm install -g vscode-langservers-extracted"),
    },
];

/// Result of resolving one language server against toolchain dirs + PATH.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub lang: String,
    pub available: bool,
    pub command: String,
    pub args: Vec<String>,
    pub install_command: Option<String>,
}

/// Remote SSH launch specification for a language server.
#[derive(Debug, Clone)]
pub struct RemoteLspSpec {
    pub lang: String,
    pub primary_cmd: String,
    pub primary_args: Vec<String>,
    pub fallback_cmd: Option<String>,
    pub fallback_args: Vec<String>,
}

/// Collect `bin` / `.bin` / `shims` directories installed under `<data_dir>/toolchains`
/// and `~/.otto/toolchains` (bounded depth <= 5, skipping bulky SDK source trees).
#[allow(clippy::disallowed_methods)]
pub fn collect_toolchain_bin_dirs(data_dir: &Path) -> Vec<PathBuf> {
    let mut out =
        crate::toolchains::ToolchainManager::new(data_dir.to_path_buf()).extra_local_bin_dirs();
    let root = data_dir.join("toolchains");
    if root.is_dir() {
        walk_toolchain_bins(&root, 0, &mut out);
    }
    if !otto_core::e2e::isolated() {
        if let Some(dot_otto) = dirs::home_dir().map(|h| h.join(".otto/toolchains")) {
            if dot_otto != root && dot_otto.is_dir() {
                walk_toolchain_bins(&dot_otto, 0, &mut out);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn collect_developer_bin_dirs(
    shell_env: Option<&BTreeMap<String, String>>,
    include_user_home: bool,
) -> Vec<PathBuf> {
    let mut dirs = Vec::with_capacity(24);
    if include_user_home {
        if let Some(shell_path) = shell_env.and_then(|m| m.get("PATH")) {
            for p in shell_path.split(':').filter(|s| !s.is_empty()) {
                dirs.push(PathBuf::from(p));
            }
        }
        if !otto_core::e2e::isolated() {
            if let Some(home) = dirs::home_dir() {
                for rel in [
                    "go/bin",
                    ".cargo/bin",
                    ".local/bin",
                    ".npm-global/bin",
                    ".volta/bin",
                ] {
                    dirs.push(home.join(rel));
                }
            }
        }
    }
    for sys in [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/go/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
    ] {
        dirs.push(PathBuf::from(sys));
    }
    dirs
}

#[allow(clippy::disallowed_methods)]
fn walk_toolchain_bins(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 5 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if matches!(name, "bin" | ".bin" | "shims") {
            out.push(path);
            continue;
        }
        if name == "node_modules" {
            let dot_bin = path.join(".bin");
            if dot_bin.is_dir() {
                out.push(dot_bin);
            }
            continue;
        }
        if name == "lib" {
            let nm_bin = path.join("node_modules").join(".bin");
            if nm_bin.is_dir() {
                out.push(nm_bin);
            }
            continue;
        }
        if matches!(
            name,
            "src"
                | "pkg"
                | "share"
                | "include"
                | "man"
                | "doc"
                | "docs"
                | "jmods"
                | "legal"
                | "conf"
                | "demo"
                | "target"
                | ".git"
                | "site-packages"
        ) {
            continue;
        }
        walk_toolchain_bins(&path, depth + 1, out);
    }
}

/// Build an augmented `PATH` string scoped to `caller_is_root` and an optional
/// injected login-shell snapshot.
pub fn augmented_local_path_for_user(
    data_dir: &Path,
    shell_env: Option<&BTreeMap<String, String>>,
    caller_is_root: bool,
) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(32);
    let mut push_unique = |s: String| {
        if !s.is_empty() && !parts.contains(&s) {
            parts.push(s);
        }
    };
    for p in collect_toolchain_bin_dirs(data_dir) {
        push_unique(p.to_string_lossy().into_owned());
    }
    for p in collect_developer_bin_dirs(shell_env, caller_is_root) {
        if p.is_dir() {
            push_unique(p.to_string_lossy().into_owned());
        }
    }
    if caller_is_root {
        let sys_path = std::env::var("PATH").unwrap_or_default();
        for seg in sys_path.split(':') {
            push_unique(seg.to_string());
        }
    }
    parts.join(":")
}

/// Build an augmented `PATH` string prepending `<data_dir>/toolchains` bin dirs
/// and standard developer `bin` directories.
pub fn augmented_local_path(data_dir: &Path) -> String {
    augmented_local_path_for_user(data_dir, None, true)
}

/// Discover a `JAVA_HOME` candidate inside `<data_dir>/toolchains` or system JDKs.
pub fn detect_toolchain_java_home(data_dir: &Path) -> Option<PathBuf> {
    for bin in collect_toolchain_bin_dirs(data_dir) {
        if bin.join("java").is_file() {
            if let Some(home) = bin.parent() {
                return Some(home.to_path_buf());
            }
        }
    }
    for fixed in [
        "/opt/homebrew/opt/openjdk/libexec/openjdk.jdk/Contents/Home",
        "/usr/local/opt/openjdk/libexec/openjdk.jdk/Contents/Home",
    ] {
        let p = PathBuf::from(fixed);
        if p.join("bin/java").is_file() {
            return Some(p);
        }
    }
    None
}

fn which_in(executable: &str, extra_dirs: &[PathBuf]) -> Option<String> {
    let exec_path = Path::new(executable);
    if exec_path.is_absolute() {
        return exec_path.is_file().then(|| executable.to_string());
    }
    for dir in extra_dirs {
        let candidate = dir.join(executable);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in path_var.split(':').filter(|d| !d.is_empty()) {
        let candidate = Path::new(dir).join(executable);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    for dir in collect_developer_bin_dirs(None, true) {
        let candidate = dir.join(executable);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

fn resolve_def(def: &ServerDef, extra_dirs: &[PathBuf]) -> Resolved {
    if let Some(resolved) = which_in(def.cmd, extra_dirs) {
        return Resolved {
            lang: def.lang.to_string(),
            available: true,
            command: resolved,
            args: def.args.iter().map(|s| s.to_string()).collect(),
            install_command: def.install_command.map(str::to_string),
        };
    }
    if def.lang == "python" {
        if let Some(resolved) = which_in("basedpyright-langserver", extra_dirs) {
            return Resolved {
                lang: def.lang.to_string(),
                available: true,
                command: resolved,
                args: vec!["--stdio".into()],
                install_command: def.install_command.map(str::to_string),
            };
        }
    }
    if let Some(fb_cmd) = def.fallback_cmd {
        if let Some(resolved) = which_in(fb_cmd, extra_dirs) {
            return Resolved {
                lang: def.lang.to_string(),
                available: true,
                command: resolved,
                args: def.fallback_args.iter().map(|s| s.to_string()).collect(),
                install_command: def.install_command.map(str::to_string),
            };
        }
    }
    Resolved {
        lang: def.lang.to_string(),
        available: false,
        command: def.cmd.to_string(),
        args: def.args.iter().map(|s| s.to_string()).collect(),
        install_command: def.install_command.map(str::to_string),
    }
}

fn resolve_plugin_spec(spec: &PluginLspServerSpec, extra_dirs: &[PathBuf]) -> Resolved {
    if let Some(resolved) = which_in(&spec.command, extra_dirs) {
        Resolved {
            lang: spec.language.clone(),
            available: true,
            command: resolved,
            args: spec.args.clone(),
            install_command: spec.install_hint.clone(),
        }
    } else {
        Resolved {
            lang: spec.language.clone(),
            available: false,
            command: spec.command.clone(),
            args: spec.args.clone(),
            install_command: spec.install_hint.clone(),
        }
    }
}

const DETECT_CACHE_TTL: Duration = Duration::from_secs(60);
const MAX_IDE_DETECT_CACHE_ENTRIES: usize = 16;

type IdeDetectKey = (PathBuf, Option<SystemTime>, Vec<(String, String)>);

struct DetectCaches {
    plain: Option<(Instant, Vec<Resolved>)>,
    with_ide: HashMap<IdeDetectKey, (Instant, Vec<Resolved>)>,
}

fn detect_caches() -> &'static Mutex<DetectCaches> {
    static CACHES: OnceLock<Mutex<DetectCaches>> = OnceLock::new();
    CACHES.get_or_init(|| {
        Mutex::new(DetectCaches {
            plain: None,
            with_ide: HashMap::new(),
        })
    })
}

/// Clear cached LSP binary detection snapshots (called after toolchain or plugin mutations).
pub fn invalidate_detect_cache() {
    if let Ok(mut guard) = detect_caches().lock() {
        guard.plain = None;
        guard.with_ide.clear();
    }
}

/// Resolve all built-in entries against system + user developer `PATH` (cached for 60 s).
pub fn detect_all() -> Vec<Resolved> {
    let now = Instant::now();
    if let Ok(guard) = detect_caches().lock() {
        if let Some((born, cached)) = &guard.plain {
            if now.duration_since(*born) < DETECT_CACHE_TTL {
                return cached.clone();
            }
        }
    }
    let fresh: Vec<Resolved> = REGISTRY.iter().map(|def| resolve_def(def, &[])).collect();
    if let Ok(mut guard) = detect_caches().lock() {
        guard.plain = Some((now, fresh.clone()));
    }
    fresh
}

/// Resolve all built-in entries + active IDE plugin LSP servers against
/// `<data_dir>/toolchains/**/bin` and system/developer `PATH` (cached for 60 s).
#[allow(clippy::disallowed_methods)]
pub fn detect_all_with_ide(data_dir: &Path, plugins: &IdePluginRegistry) -> Vec<Resolved> {
    let now = Instant::now();
    let tc_mtime = std::fs::metadata(data_dir.join("toolchains"))
        .and_then(|m| m.modified())
        .ok();
    let active_specs = plugins.active_lsp_servers();
    let spec_fp: Vec<(String, String)> = active_specs
        .iter()
        .map(|s| (s.language.clone(), s.command.clone()))
        .collect();
    let cache_key: IdeDetectKey = (data_dir.to_path_buf(), tc_mtime, spec_fp);

    if let Ok(guard) = detect_caches().lock() {
        if let Some((born, cached)) = guard.with_ide.get(&cache_key) {
            if now.duration_since(*born) < DETECT_CACHE_TTL {
                return cached.clone();
            }
        }
    }

    let extra = collect_toolchain_bin_dirs(data_dir);
    let mut out: Vec<Resolved> = REGISTRY
        .iter()
        .map(|def| resolve_def(def, &extra))
        .collect();

    for spec in active_specs {
        if is_lightweight_non_lsp_lang(&spec.language) {
            continue;
        }
        let plugin_resolved = resolve_plugin_spec(&spec, &extra);
        if let Some(existing) = out.iter_mut().find(|r| r.lang == spec.language) {
            if !existing.available || plugin_resolved.available {
                *existing = plugin_resolved;
            }
        } else {
            out.push(plugin_resolved);
        }
    }

    if let Ok(mut guard) = detect_caches().lock() {
        if guard.with_ide.len() >= MAX_IDE_DETECT_CACHE_ENTRIES {
            guard
                .with_ide
                .retain(|_, (born, _)| now.duration_since(*born) < DETECT_CACHE_TTL);
            if guard.with_ide.len() >= MAX_IDE_DETECT_CACHE_ENTRIES {
                guard.with_ide.clear();
            }
        }
        guard.with_ide.insert(cache_key, (now, out.clone()));
    }
    out
}

/// Resolve one language against system/developer `PATH`.
pub fn resolve_lang(lang: &str) -> Option<Resolved> {
    if is_lightweight_non_lsp_lang(lang) {
        return None;
    }
    let norm = normalize_lsp_lang(lang);
    detect_all().into_iter().find(|r| r.lang == norm)
}

/// Resolve one language against `<data_dir>/toolchains/**/bin`, `PATH`, and
/// active runtime IDE plugins.
pub fn resolve_lang_with_ide(
    lang: &str,
    data_dir: &Path,
    plugins: &IdePluginRegistry,
) -> Option<Resolved> {
    if is_lightweight_non_lsp_lang(lang) {
        return None;
    }
    let norm = normalize_lsp_lang(lang);
    detect_all_with_ide(data_dir, plugins)
        .into_iter()
        .find(|r| r.lang == norm)
}

/// Resolve remote command metadata for a language when launching over SSH.
pub fn resolve_remote_lang(lang: &str, plugins: &IdePluginRegistry) -> Option<RemoteLspSpec> {
    if is_lightweight_non_lsp_lang(lang) {
        return None;
    }
    let norm = normalize_lsp_lang(lang);
    for spec in plugins.active_lsp_servers() {
        if spec.language == norm {
            return Some(RemoteLspSpec {
                lang: spec.language,
                primary_cmd: spec.command,
                primary_args: spec.args,
                fallback_cmd: None,
                fallback_args: Vec::new(),
            });
        }
    }
    REGISTRY
        .iter()
        .find(|def| def.lang == norm)
        .map(|def| RemoteLspSpec {
            lang: def.lang.to_string(),
            primary_cmd: def.cmd.to_string(),
            primary_args: def.args.iter().map(|s| s.to_string()).collect(),
            fallback_cmd: def.fallback_cmd.map(str::to_string),
            fallback_args: def.fallback_args.iter().map(|s| s.to_string()).collect(),
        })
}
