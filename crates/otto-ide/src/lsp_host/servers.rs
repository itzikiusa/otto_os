//! LSP server registry: 11 built-in languages (including Go, Python, TS, JS,
//! Java, Rust, Shell, JSON, CSS, HTML, Markdown) + runtime IDE plugin servers,
//! with `<data_dir>/toolchains/**/bin`, `~/.otto/toolchains/**/bin`, cached
//! login-shell `PATH`, and standard developer `bin` resolution.

use std::path::{Path, PathBuf};

use crate::plugins::IdePluginRegistry;
use crate::types::PluginLspServerSpec;

/// One entry in the built-in registry: a language and how to launch its server.
pub struct ServerDef {
    pub lang: &'static str,
    /// Primary executable name.
    pub cmd: &'static str,
    /// Args passed to the primary.
    pub args: &'static [&'static str],
    /// Fallback executable (tried when primary is absent from PATH).
    pub fallback_cmd: Option<&'static str>,
    /// Args for the fallback.
    pub fallback_args: &'static [&'static str],
    /// File extensions that map to this language (dot-prefixed, e.g. ".go").
    pub extensions: &'static [&'static str],
    /// Shell command the UI can run to install this server; None = manual.
    pub install_command: Option<&'static str>,
}

/// Normalize language aliases (`sh`/`bash`/`zsh` -> `shell`, `jsonl`/`jsonc` -> `json`).
pub fn normalize_lsp_lang(lang: &str) -> &str {
    match lang.trim() {
        "sh" | "bash" | "zsh" | "shellscript" => "shell",
        "jsonc" | "jsonl" | "ndjson" => "json",
        "tsx" | "typescriptreact" => "typescript",
        "jsx" | "javascriptreact" => "javascript",
        other => other,
    }
}

/// The full built-in registry covering all core IDE languages.
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
        lang: "json",
        cmd: "vscode-json-language-server",
        args: &["--stdio"],
        fallback_cmd: Some("vscode-json-languageserver"),
        fallback_args: &["--stdio"],
        extensions: &[".json", ".jsonc", ".jsonl"],
        install_command: Some("npm install -g vscode-langservers-extracted"),
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
    ServerDef {
        lang: "markdown",
        cmd: "marksman",
        args: &["server"],
        fallback_cmd: None,
        fallback_args: &[],
        extensions: &[".md", ".markdown"],
        install_command: Some("brew install marksman"),
    },
];

/// Result of resolving one language server against toolchain dirs + PATH.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub lang: String,
    pub available: bool,
    /// The resolved executable path (or the primary name when unavailable).
    pub command: String,
    /// Args to pass to the resolved command.
    pub args: Vec<String>,
    /// Optional install command the UI can present.
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

/// Standard user/system developer `bin` directories missing under macOS `launchd`
/// (`~/go/bin`, `~/.cargo/bin`, `/opt/homebrew/bin`, `/usr/local/go/bin`, + cached shell `PATH`).
fn collect_shell_and_developer_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::with_capacity(24);
    let shell_env = crate::environments::cached_shell_env_snapshot();
    if let Some(shell_path) = shell_env.get("PATH") {
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
    for sys in [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/go/bin",
        "/usr/local/bin",
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

/// Build an augmented `PATH` string prepending `<data_dir>/toolchains` bin dirs,
/// cached login-shell `PATH`, and standard developer `bin` directories.
pub fn augmented_local_path(data_dir: &Path) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(32);
    let mut push_unique = |s: String| {
        if !s.is_empty() && !parts.contains(&s) {
            parts.push(s);
        }
    };
    for p in collect_toolchain_bin_dirs(data_dir) {
        push_unique(p.to_string_lossy().into_owned());
    }
    for p in collect_shell_and_developer_bin_dirs() {
        if p.is_dir() {
            push_unique(p.to_string_lossy().into_owned());
        }
    }
    let sys_path = std::env::var("PATH").unwrap_or_default();
    for seg in sys_path.split(':') {
        push_unique(seg.to_string());
    }
    parts.join(":")
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

/// Locate `executable` across `extra_dirs` first, then `PATH`, then login-shell/dev dirs.
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
    for dir in collect_shell_and_developer_bin_dirs() {
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

/// Resolve all built-in entries against system + user developer `PATH`.
pub fn detect_all() -> Vec<Resolved> {
    REGISTRY.iter().map(|def| resolve_def(def, &[])).collect()
}

/// Resolve all built-in entries + active IDE plugin LSP servers against
/// `<data_dir>/toolchains/**/bin` and system/shell `PATH`.
pub fn detect_all_with_ide(data_dir: &Path, plugins: &IdePluginRegistry) -> Vec<Resolved> {
    let extra = collect_toolchain_bin_dirs(data_dir);
    let mut out: Vec<Resolved> = REGISTRY
        .iter()
        .map(|def| resolve_def(def, &extra))
        .collect();

    for spec in plugins.active_lsp_servers() {
        let plugin_resolved = resolve_plugin_spec(&spec, &extra);
        if let Some(existing) = out.iter_mut().find(|r| r.lang == spec.language) {
            if !existing.available || plugin_resolved.available {
                *existing = plugin_resolved;
            }
        } else {
            out.push(plugin_resolved);
        }
    }
    out
}

/// Resolve one language against system/shell `PATH`.
pub fn resolve_lang(lang: &str) -> Option<Resolved> {
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
    let norm = normalize_lsp_lang(lang);
    detect_all_with_ide(data_dir, plugins)
        .into_iter()
        .find(|r| r.lang == norm)
}

/// Resolve remote command metadata for a language when launching over SSH.
pub fn resolve_remote_lang(lang: &str, plugins: &IdePluginRegistry) -> Option<RemoteLspSpec> {
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

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn finds_binaries_installed_under_data_dir_toolchains_and_normalizes_shell_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let go_bin = tmp
            .path()
            .join("toolchains")
            .join("go")
            .join("1.23.0")
            .join("bin");
        std::fs::create_dir_all(&go_bin).unwrap();
        let fake_gopls = go_bin.join("gopls");
        std::fs::write(&fake_gopls, "#!/bin/sh\nexit 0\n").unwrap();

        let bins = collect_toolchain_bin_dirs(tmp.path());
        assert!(bins.contains(&go_bin));

        let reg = IdePluginRegistry::with_dirs(tmp.path().to_path_buf(), None);
        let resolved = resolve_lang_with_ide("go", tmp.path(), &reg).unwrap();
        assert!(resolved.available);
        assert_eq!(resolved.command, fake_gopls.to_string_lossy());

        // Shell alias resolution (`bash`, `zsh`, `sh` -> `shell`):
        assert_eq!(normalize_lsp_lang("bash"), "shell");
        assert_eq!(normalize_lsp_lang("jsonl"), "json");
        assert!(resolve_remote_lang("bash", &reg).is_some());
        assert!(resolve_remote_lang("jsonc", &reg).is_some());
    }

    #[test]
    fn includes_custom_lsp_from_enabled_ide_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("ide-plugins").join("kotlin-ide");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let fake_bin_dir = tmp.path().join("toolchains").join("kotlin").join("bin");
        std::fs::create_dir_all(&fake_bin_dir).unwrap();
        let fake_kls = fake_bin_dir.join("kotlin-language-server");
        std::fs::write(&fake_kls, "#!/bin/sh\nexit 0\n").unwrap();

        std::fs::write(
            plugin_dir.join("otto-ide-plugin.json"),
            r#"{
                "slug": "kotlin-ide",
                "name": "Kotlin IDE",
                "version": "1.0.0",
                "contributions": {
                    "lsp_servers": [{
                        "language": "kotlin",
                        "command": "kotlin-language-server",
                        "args": [],
                        "extensions": [".kt", ".kts"],
                        "install_hint": "brew install kotlin-language-server"
                    }]
                }
            }"#,
        )
        .unwrap();

        let reg = IdePluginRegistry::with_dirs(tmp.path().to_path_buf(), None);
        let resolved = resolve_lang_with_ide("kotlin", tmp.path(), &reg).unwrap();
        assert!(resolved.available);
        assert_eq!(resolved.command, fake_kls.to_string_lossy());

        let remote = resolve_remote_lang("kotlin", &reg).unwrap();
        assert_eq!(remote.primary_cmd, "kotlin-language-server");
    }
}
