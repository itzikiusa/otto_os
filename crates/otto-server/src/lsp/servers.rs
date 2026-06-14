//! LSP server registry: 10 languages, primary + optional fallback, PATH detection.

/// One entry in the registry: a language and how to launch its server.
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

/// The full registry, in the order defined in the design spec.
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

/// Result of resolving one language server against PATH.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub lang: String,
    pub available: bool,
    /// The resolved executable name (or the primary name when unavailable).
    pub command: String,
    /// Args to pass to the resolved command.
    pub args: Vec<String>,
    /// Optional install command the UI can present.
    pub install_command: Option<String>,
}

/// Locate `executable` on PATH directories by checking each directory for the
/// file. Returns `Some(path_string)` on first match, `None` when absent.
fn which(executable: &str) -> Option<String> {
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in path_var.split(':') {
        let candidate = std::path::Path::new(dir).join(executable);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

/// Resolve all entries in the registry against the current PATH.
pub fn detect_all() -> Vec<Resolved> {
    REGISTRY
        .iter()
        .map(|def| {
            // Try primary first.
            if let Some(resolved) = which(def.cmd) {
                return Resolved {
                    lang: def.lang.to_string(),
                    available: true,
                    command: resolved,
                    args: def.args.iter().map(|s| s.to_string()).collect(),
                    install_command: def.install_command.map(str::to_string),
                };
            }
            // Try fallback.
            if let Some(fb_cmd) = def.fallback_cmd {
                if let Some(resolved) = which(fb_cmd) {
                    return Resolved {
                        lang: def.lang.to_string(),
                        available: true,
                        command: resolved,
                        args: def.fallback_args.iter().map(|s| s.to_string()).collect(),
                        install_command: def.install_command.map(str::to_string),
                    };
                }
            }
            // Neither found.
            Resolved {
                lang: def.lang.to_string(),
                available: false,
                command: def.cmd.to_string(),
                args: def.args.iter().map(|s| s.to_string()).collect(),
                install_command: def.install_command.map(str::to_string),
            }
        })
        .collect()
}

/// Given a language name, return the `Resolved` entry (if the lang is in the
/// registry) and whether the server is available.
pub fn resolve_lang(lang: &str) -> Option<Resolved> {
    detect_all().into_iter().find(|r| r.lang == lang)
}
