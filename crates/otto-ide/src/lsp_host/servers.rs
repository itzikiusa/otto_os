//! LSP server registry: 10 built-in languages + runtime IDE plugin servers,
//! with `<data_dir>/toolchains/**/bin` and `PATH` executable resolution.

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

/// The full built-in registry, in the order defined in the design spec.
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
        extensions: &[".py"],
        install_command: Some("npm install -g pyright"),
    },
    ServerDef {
        lang: "typescript",
        cmd: "typescript-language-server",
        args: &["--stdio"],
        fallback_cmd: None,
        fallback_args: &[],
        extensions: &[".ts", ".tsx", ".mts", ".cts"],
        install_command: Some("npm install -g typescript-language-server"),
    },
    ServerDef {
        lang: "javascript",
        cmd: "typescript-language-server",
        args: &["--stdio"],
        fallback_cmd: None,
        fallback_args: &[],
        extensions: &[".js", ".jsx", ".mjs", ".cjs"],
        install_command: Some("npm install -g typescript-language-server"),
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
        lang: "java",
        cmd: "jdtls",
        args: &[],
        fallback_cmd: None,
        fallback_args: &[],
        extensions: &[".java"],
        install_command: None,
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
    ServerDef {
        lang: "json",
        cmd: "vscode-json-language-server",
        args: &["--stdio"],
        fallback_cmd: None,
        fallback_args: &[],
        extensions: &[".json", ".jsonc"],
        install_command: Some("npm install -g vscode-langservers-extracted"),
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

/// Remote SSH launch specification for a language server (resolved on the remote
/// host at spawn time so a remote-only toolchain works even when absent locally).
#[derive(Debug, Clone)]
pub struct RemoteLspSpec {
    pub lang: String,
    pub primary_cmd: String,
    pub primary_args: Vec<String>,
    pub fallback_cmd: Option<String>,
    pub fallback_args: Vec<String>,
}

/// Collect `bin` / `.bin` / `shims` directories installed under `<data_dir>/toolchains`
/// (bounded depth <= 5, skipping large non-bin SDK trees).
#[allow(clippy::disallowed_methods)] // bounded depth<=5 scan of local <data_dir>/toolchains dirs
pub fn collect_toolchain_bin_dirs(data_dir: &Path) -> Vec<PathBuf> {
    let mut out =
        crate::toolchains::ToolchainManager::new(data_dir.to_path_buf()).extra_local_bin_dirs();
    let root = data_dir.join("toolchains");
    if root.is_dir() {
        walk_toolchain_bins(&root, 0, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

#[allow(clippy::disallowed_methods)] // bounded depth<=5 scan of local <data_dir>/toolchains dirs
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
        // Skip bulky SDK directories that never hold top-level binaries.
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

/// Build an augmented `PATH` string prepending `<data_dir>/toolchains` bin dirs.
pub fn augmented_local_path(data_dir: &Path) -> String {
    let extra = collect_toolchain_bin_dirs(data_dir);
    let sys_path = std::env::var("PATH").unwrap_or_default();
    if extra.is_empty() {
        return sys_path;
    }
    let mut parts: Vec<String> = extra
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    if !sys_path.is_empty() {
        parts.push(sys_path);
    }
    parts.join(":")
}

/// Discover a `JAVA_HOME` candidate inside `<data_dir>/toolchains` when the
/// daemon environment does not already define `JAVA_HOME`.
pub fn detect_toolchain_java_home(data_dir: &Path) -> Option<PathBuf> {
    for bin in collect_toolchain_bin_dirs(data_dir) {
        if bin.join("java").is_file() {
            if let Some(home) = bin.parent() {
                return Some(home.to_path_buf());
            }
        }
    }
    None
}

/// Locate `executable` across `extra_dirs` first, then `PATH`.
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

/// Resolve all built-in entries against the current system `PATH`.
pub fn detect_all() -> Vec<Resolved> {
    REGISTRY.iter().map(|def| resolve_def(def, &[])).collect()
}

/// Resolve all built-in entries + active IDE plugin LSP servers against
/// `<data_dir>/toolchains/**/bin` and system `PATH`.
pub fn detect_all_with_ide(data_dir: &Path, plugins: &IdePluginRegistry) -> Vec<Resolved> {
    let extra = collect_toolchain_bin_dirs(data_dir);
    let mut out: Vec<Resolved> = REGISTRY
        .iter()
        .map(|def| resolve_def(def, &extra))
        .collect();

    for spec in plugins.active_lsp_servers() {
        let plugin_resolved = resolve_plugin_spec(&spec, &extra);
        if let Some(existing) = out.iter_mut().find(|r| r.lang == spec.language) {
            // Let an enabled plugin fill in an unavailable built-in language or
            // override when the plugin's server binary is available.
            if !existing.available || plugin_resolved.available {
                *existing = plugin_resolved;
            }
        } else {
            out.push(plugin_resolved);
        }
    }
    out
}

/// Resolve one language against system `PATH` only.
pub fn resolve_lang(lang: &str) -> Option<Resolved> {
    detect_all().into_iter().find(|r| r.lang == lang)
}

/// Resolve one language against `<data_dir>/toolchains/**/bin`, `PATH`, and
/// active runtime IDE plugins.
pub fn resolve_lang_with_ide(
    lang: &str,
    data_dir: &Path,
    plugins: &IdePluginRegistry,
) -> Option<Resolved> {
    detect_all_with_ide(data_dir, plugins)
        .into_iter()
        .find(|r| r.lang == lang)
}

/// Resolve remote command metadata for a language when launching over SSH.
pub fn resolve_remote_lang(lang: &str, plugins: &IdePluginRegistry) -> Option<RemoteLspSpec> {
    // Check active plugins first if they contribute a custom language or override.
    for spec in plugins.active_lsp_servers() {
        if spec.language == lang {
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
        .find(|def| def.lang == lang)
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
    fn finds_binaries_installed_under_data_dir_toolchains() {
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
