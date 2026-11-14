//! Local filesystem & remote SSH batched SDK/LSP/DAP discovery helpers.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::toolchains_catalog::{EcosystemKind, TargetPlatform, JAVA_DEBUG_JAR_NAME};
use crate::types::{IdeInstalledSdk, IdeProjectEnvironment, IdeToolBinaryStatus};

pub(super) fn empty_tool(name: &str) -> IdeToolBinaryStatus {
    IdeToolBinaryStatus {
        name: name.to_string(),
        available: false,
        path: None,
    }
}

pub(super) fn collect_managed_bin_dirs(tc_root: &Path, out: &mut Vec<PathBuf>) {
    if !tc_root.is_dir() {
        return;
    }
    for rel in [
        "java/lsp/bin",
        "java/current/bin",
        "go/bin",
        "rust/bin",
        "rust/cargo/bin",
        "rust/dap/extension/adapter",
        "rust/codelldb/adapter",
        "node/current/bin",
        "node/lsp/node_modules/.bin",
        "python/bin",
        "python/current/bin",
        "python/dap/bin",
        "shell/bin",
        "shell/lsp/node_modules/.bin",
        "json/bin",
        "json/lsp/node_modules/.bin",
    ] {
        let c = tc_root.join(rel);
        if c.is_dir() && !out.contains(&c) {
            out.push(c);
        }
    }
    for sub in ["java/jdks", "go/sdks", "node/sdks", "python/interpreters"] {
        for p in read_subdirs(&tc_root.join(sub)) {
            for rel in ["bin", "Contents/Home/bin", "go/bin"] {
                let b = p.join(rel);
                if b.is_dir() && !out.contains(&b) {
                    out.push(b);
                }
            }
        }
    }
}

pub(crate) fn system_search_dirs() -> Vec<PathBuf> {
    let mut dirs = std::env::var_os("PATH")
        .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .unwrap_or_default();
    if let Some(shell_path) = crate::environments::cached_shell_env_snapshot().get("PATH") {
        for p in std::env::split_paths(shell_path) {
            if !dirs.contains(&p) {
                dirs.push(p);
            }
        }
    }
    for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
        let p = PathBuf::from(extra);
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }
    if !otto_core::e2e::isolated() {
        if let Some(home) = dirs::home_dir() {
            for rel in [".cargo/bin", "go/bin", ".local/bin"] {
                let p = home.join(rel);
                if !dirs.contains(&p) {
                    dirs.push(p);
                }
            }
        }
    }
    dirs
}

pub(crate) fn find_executable_in(dirs: &[PathBuf], names: &[&str]) -> Option<PathBuf> {
    dirs.iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|cand| is_executable_file(cand))
}

pub(crate) fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(super) fn discover_local_sdks(roots: &[PathBuf], eco: EcosystemKind) -> Vec<IdeInstalledSdk> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for root in roots {
        scan_otto_sdks(root, eco, &mut seen, &mut out);
    }
    scan_system_sdks(eco, &mut seen, &mut out);
    out
}

fn scan_otto_sdks(
    root: &Path,
    eco: EcosystemKind,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<IdeInstalledSdk>,
) {
    match eco {
        EcosystemKind::Java => {
            for dir in read_subdirs(&root.join("java/jdks")) {
                if let Some(jh) = resolve_java_home_dir(&dir) {
                    let v =
                        read_java_release_version(&jh).unwrap_or_else(|| dir_name_version(&dir));
                    push_sdk(out, seen, "java", &v, &jh, "otto");
                }
            }
        }
        EcosystemKind::Go => {
            for dir in read_subdirs(&root.join("go/sdks")) {
                let gr = if dir.join("go/bin/go").is_file() {
                    dir.join("go")
                } else {
                    dir.clone()
                };
                if gr.join("bin/go").is_file() {
                    let v = read_go_root_version(&gr).unwrap_or_else(|| dir_name_version(&dir));
                    push_sdk(out, seen, "go", &v, &gr, "otto");
                }
            }
        }
        EcosystemKind::Rust => {
            for dir in read_subdirs(&root.join("rust/rustup/toolchains")) {
                if dir.join("bin/rustc").is_file() {
                    push_sdk(out, seen, "rust", &rust_toolchain_label(&dir), &dir, "otto");
                }
            }
        }
        EcosystemKind::Node => {
            for dir in read_subdirs(&root.join("node/sdks")) {
                if dir.join("bin/node").is_file() {
                    push_sdk(out, seen, "node", &dir_name_version(&dir), &dir, "otto");
                }
            }
        }
        EcosystemKind::Python => {
            for dir in read_subdirs(&root.join("python/interpreters")) {
                let py = dir.join("bin/python3");
                if py.is_file() {
                    push_sdk(out, seen, "python", &python_dir_version(&dir), &py, "otto");
                }
            }
        }
        EcosystemKind::Shell => {
            let shfmt = root.join("shell/bin/shfmt");
            if is_executable_file(&shfmt) {
                push_sdk(out, seen, "shell", "3.10.0", &shfmt, "otto");
            }
        }
        EcosystemKind::Json => {
            let jq = root.join("json/bin/jq");
            if is_executable_file(&jq) {
                push_sdk(out, seen, "json", "1.7.1", &jq, "otto");
            }
        }
    }
}

fn scan_system_sdks(
    eco: EcosystemKind,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<IdeInstalledSdk>,
) {
    let search = system_search_dirs();
    let home = dirs::home_dir();
    match eco {
        EcosystemKind::Java => {
            let mut jvm_dirs = vec![
                PathBuf::from("/Library/Java/JavaVirtualMachines"),
                PathBuf::from("/usr/lib/jvm"),
            ];
            if let Some(h) = &home {
                jvm_dirs.push(h.join("Library/Java/JavaVirtualMachines"));
                jvm_dirs.push(h.join(".sdkman/candidates/java"));
            }
            for dir in jvm_dirs.iter().flat_map(|p| read_subdirs(p)) {
                if let Some(jh) = resolve_java_home_dir(&dir) {
                    let v =
                        read_java_release_version(&jh).unwrap_or_else(|| dir_name_version(&dir));
                    push_sdk(out, seen, "java", &v, &jh, "system");
                }
            }
            if let Some(jh) = find_executable_in(&search, &["java"])
                .and_then(|b| b.parent().and_then(|p| p.parent()).map(Path::to_path_buf))
                .filter(|jh| jh != Path::new("/usr"))
            {
                let v = read_java_release_version(&jh).unwrap_or_else(|| "system".into());
                push_sdk(out, seen, "java", &v, &jh, "system");
            }
        }
        EcosystemKind::Go => {
            for cand in ["/opt/homebrew/opt/go/libexec", "/usr/local/go"] {
                let p = PathBuf::from(cand);
                if p.join("bin/go").is_file() {
                    let v = read_go_root_version(&p).unwrap_or_else(|| "system".into());
                    push_sdk(out, seen, "go", &v, &p, "system");
                }
            }
            if let Some(gr) = find_executable_in(&search, &["go"])
                .and_then(|b| b.parent().and_then(|p| p.parent()).map(Path::to_path_buf))
            {
                let v = read_go_root_version(&gr).unwrap_or_else(|| "system".into());
                push_sdk(out, seen, "go", &v, &gr, "system");
            }
        }
        EcosystemKind::Rust => {
            if let Some(h) = &home {
                for dir in read_subdirs(&h.join(".rustup/toolchains")) {
                    if dir.join("bin/rustc").is_file() {
                        push_sdk(
                            out,
                            seen,
                            "rust",
                            &rust_toolchain_label(&dir),
                            &dir,
                            "system",
                        );
                    }
                }
            }
            if out.is_empty() {
                if let Some(root) = find_executable_in(&search, &["rustc"])
                    .and_then(|b| b.parent().and_then(|p| p.parent()).map(Path::to_path_buf))
                {
                    push_sdk(out, seen, "rust", "system", &root, "system");
                }
            }
        }
        EcosystemKind::Node => {
            if let Some(h) = &home {
                for dir in read_subdirs(&h.join(".nvm/versions/node")) {
                    if dir.join("bin/node").is_file() {
                        push_sdk(out, seen, "node", &dir_name_version(&dir), &dir, "system");
                    }
                }
            }
            if let Some(prefix) = find_executable_in(&search, &["node"])
                .and_then(|b| b.parent().and_then(|p| p.parent()).map(Path::to_path_buf))
            {
                push_sdk(out, seen, "node", "system", &prefix, "system");
            }
        }
        EcosystemKind::Python => {
            if let Some(h) = &home {
                for dir in read_subdirs(&h.join(".local/share/uv/python")) {
                    let py = dir.join("bin/python3");
                    if py.is_file() {
                        push_sdk(
                            out,
                            seen,
                            "python",
                            &python_dir_version(&dir),
                            &py,
                            "system",
                        );
                    }
                }
            }
            if let Some(py_bin) = find_executable_in(&search, &["python3"]) {
                push_sdk(out, seen, "python", "3", &py_bin, "system");
            }
        }
        EcosystemKind::Shell => {
            if let Some(sh_bin) = find_executable_in(&search, &["bash", "zsh"]) {
                push_sdk(out, seen, "shell", "system", &sh_bin, "system");
            }
        }
        EcosystemKind::Json => {
            if let Some(jq_bin) = find_executable_in(&search, &["jq"]) {
                push_sdk(out, seen, "json", "system", &jq_bin, "system");
            }
        }
    }
}

pub(super) fn discover_local_lsp(roots: &[PathBuf], eco: EcosystemKind) -> IdeToolBinaryStatus {
    let name = eco.lsp_name().to_string();
    let rels: &[&str] = match eco {
        EcosystemKind::Java => &["java/lsp/bin/jdtls"],
        EcosystemKind::Go => &["go/bin/gopls"],
        EcosystemKind::Rust => &["rust/bin/rust-analyzer", "rust/cargo/bin/rust-analyzer"],
        EcosystemKind::Node => &["node/lsp/node_modules/.bin/typescript-language-server"],
        EcosystemKind::Python => &["python/bin/pyright-langserver", "python/bin/pyright"],
        EcosystemKind::Shell => &["shell/lsp/node_modules/.bin/bash-language-server"],
        EcosystemKind::Json => &[
            "json/lsp/node_modules/.bin/vscode-json-language-server",
            "json/lsp/node_modules/.bin/vscode-json-languageserver",
        ],
    };
    for cand in roots
        .iter()
        .flat_map(|r| rels.iter().map(move |rel| r.join(rel)))
    {
        if cand.is_file() {
            return IdeToolBinaryStatus {
                name,
                available: true,
                path: Some(cand.to_string_lossy().into_owned()),
            };
        }
    }
    let sys_names: &[&str] = match eco {
        EcosystemKind::Java => &["jdtls"],
        EcosystemKind::Go => &["gopls"],
        EcosystemKind::Rust => &["rust-analyzer"],
        EcosystemKind::Node => &["typescript-language-server"],
        EcosystemKind::Python => &["pyright-langserver", "pyright"],
        EcosystemKind::Shell => &["bash-language-server"],
        EcosystemKind::Json => &["vscode-json-language-server", "vscode-json-languageserver"],
    };
    if let Some(sys) = find_executable_in(&system_search_dirs(), sys_names) {
        return IdeToolBinaryStatus {
            name,
            available: true,
            path: Some(sys.to_string_lossy().into_owned()),
        };
    }
    empty_tool(&name)
}

pub(super) fn discover_local_dap(roots: &[PathBuf], eco: EcosystemKind) -> IdeToolBinaryStatus {
    let name = eco.dap_name().to_string();
    let java_rel = format!("java/dap/{JAVA_DEBUG_JAR_NAME}");
    let rels: Vec<&str> = match eco {
        EcosystemKind::Java => vec![java_rel.as_str()],
        EcosystemKind::Go => vec!["go/bin/dlv", "dlv/dlv"],
        EcosystemKind::Rust => vec![
            "rust/dap/extension/adapter/codelldb",
            "rust/codelldb/adapter/codelldb",
        ],
        EcosystemKind::Node => vec![
            "node/dap/js-debug/src/dapDebugServer.js",
            "node/js-debug/src/dapDebugServer.js",
        ],
        EcosystemKind::Python => vec!["python/dap/bin/python"],
        EcosystemKind::Shell => vec!["shell/bin/bashdb"],
        EcosystemKind::Json => vec!["json/bin/jq"],
    };
    for root in roots {
        for rel in &rels {
            let cand = root.join(rel);
            let ok = match eco {
                EcosystemKind::Java | EcosystemKind::Node | EcosystemKind::Python => cand.is_file(),
                EcosystemKind::Go
                | EcosystemKind::Rust
                | EcosystemKind::Shell
                | EcosystemKind::Json => is_executable_file(&cand),
            };
            if ok {
                return IdeToolBinaryStatus {
                    name,
                    available: true,
                    path: Some(cand.to_string_lossy().into_owned()),
                };
            }
        }
    }
    let sys_names: &[&str] = match eco {
        EcosystemKind::Java => &["java-debug", "java"],
        EcosystemKind::Go => &["dlv"],
        EcosystemKind::Rust => &["codelldb", "lldb-dap", "lldb"],
        EcosystemKind::Node => &["js-debug-dap", "node"],
        EcosystemKind::Python => &["debugpy", "debugpy-adapter", "python3"],
        EcosystemKind::Shell => &["bashdb", "bash", "zsh"],
        EcosystemKind::Json => &["jq"],
    };
    if let Some(sys) = find_executable_in(&system_search_dirs(), sys_names) {
        return IdeToolBinaryStatus {
            name,
            available: true,
            path: Some(sys.to_string_lossy().into_owned()),
        };
    }
    empty_tool(&name)
}

pub(super) fn mark_active_sdk(
    installed: &mut [IdeInstalledSdk],
    eco: EcosystemKind,
    active_env: Option<&IdeProjectEnvironment>,
) {
    let chosen = active_env.and_then(|env| match eco {
        EcosystemKind::Java => env.java_home.as_deref(),
        EcosystemKind::Go => env.go_root.as_deref(),
        EcosystemKind::Rust => env.rust_toolchain.as_deref(),
        EcosystemKind::Node => env.node_home.as_deref(),
        EcosystemKind::Python => env.python_interpreter.as_deref(),
        EcosystemKind::Shell | EcosystemKind::Json => None,
    });
    if let Some(sel) = chosen.filter(|s| !s.trim().is_empty()) {
        if let Some(item) = installed
            .iter_mut()
            .find(|i| i.path == sel || i.version == sel)
        {
            item.active = true;
            return;
        }
    }
    if let Some(first) = installed.first_mut() {
        first.active = true;
    }
}

#[allow(clippy::disallowed_methods)]
pub(crate) fn read_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    v.sort();
    v
}

pub(crate) fn resolve_java_home_dir(dir: &Path) -> Option<PathBuf> {
    for base in std::iter::once(dir.to_path_buf()).chain(read_subdirs(dir)) {
        for cand in [base.join("Contents/Home"), base] {
            if cand.join("bin/java").is_file() {
                return Some(cand);
            }
        }
    }
    None
}

pub(super) fn read_java_release_version(java_home: &Path) -> Option<String> {
    std::fs::read_to_string(java_home.join("release"))
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("JAVA_VERSION="))
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
}

pub(super) fn read_go_root_version(goroot: &Path) -> Option<String> {
    let text = std::fs::read_to_string(goroot.join("VERSION")).ok()?;
    let first = text.lines().next()?.trim().trim_start_matches("go");
    (!first.is_empty()).then(|| first.to_string())
}

pub(super) fn dir_name_version(dir: &Path) -> String {
    dir.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .trim_start_matches("jdk-")
        .trim_start_matches("go")
        .trim_start_matches('v')
        .to_string()
}

pub(super) fn rust_toolchain_label(dir: &Path) -> String {
    let raw = dir.file_name().and_then(|s| s.to_str()).unwrap_or("stable");
    ["stable", "beta", "nightly"]
        .into_iter()
        .find(|ch| raw.starts_with(ch))
        .unwrap_or_else(|| raw.split('-').next().unwrap_or(raw))
        .to_string()
}

pub(super) fn python_dir_version(dir: &Path) -> String {
    let raw = dir.file_name().and_then(|s| s.to_str()).unwrap_or("3");
    let stripped = raw.trim_start_matches("cpython-");
    stripped.split('-').next().unwrap_or(stripped).to_string()
}

fn push_sdk(
    out: &mut Vec<IdeInstalledSdk>,
    seen: &mut BTreeSet<String>,
    prefix: &str,
    version: &str,
    path: &Path,
    source: &str,
) {
    let path_str = path.to_string_lossy().into_owned();
    if seen.insert(path_str.clone()) {
        out.push(IdeInstalledSdk {
            id: format!("{prefix}-{source}-{}", out.len() + 1),
            version: version.to_string(),
            path: path_str,
            source: source.to_string(),
            active: false,
        });
    }
}

pub(super) const REMOTE_PROBE_SCRIPT: &str = r#"
echo "UNAME:$(uname -s) $(uname -m)"
export PATH="$HOME/.otto/toolchains/java/current/bin:$HOME/.otto/toolchains/go/current/bin:$HOME/.otto/toolchains/node/current/bin:$HOME/.otto/toolchains/go/bin:$HOME/.otto/toolchains/rust/bin:$HOME/.otto/toolchains/rust/cargo/bin:$HOME/.otto/toolchains/python/bin:$HOME/.otto/toolchains/shell/bin:$HOME/.otto/toolchains/json/bin:$HOME/.cargo/bin:$HOME/go/bin:/opt/homebrew/bin:/usr/local/go/bin:/usr/local/bin:$PATH"
for d in "$HOME"/.otto/toolchains/java/jdks/*; do
  [ -d "$d" ] || continue
  jh=""
  for cand in "$d/Contents/Home" "$d" "$d"/*/Contents/Home "$d"/*; do
    if [ -x "$cand/bin/java" ]; then jh="$cand"; break; fi
  done
  [ -n "$jh" ] && echo "SDK:java:otto:$jh"
done
for d in "$HOME"/.otto/toolchains/go/sdks/*; do
  if [ -x "$d/go/bin/go" ]; then echo "SDK:go:otto:$d/go"; elif [ -x "$d/bin/go" ]; then echo "SDK:go:otto:$d"; fi
done
for d in "$HOME"/.otto/toolchains/rust/rustup/toolchains/*; do [ -d "$d" ] && echo "SDK:rust:otto:$d"; done
for d in "$HOME"/.otto/toolchains/node/sdks/*; do [ -d "$d" ] && echo "SDK:node:otto:$d"; done
for d in "$HOME"/.otto/toolchains/python/interpreters/*; do [ -x "$d/bin/python3" ] && echo "SDK:python:otto:$d/bin/python3"; done
[ -x "$HOME/.otto/toolchains/shell/bin/shfmt" ] && echo "SDK:shell:otto:$HOME/.otto/toolchains/shell/bin/shfmt" || true
[ -x "$HOME/.otto/toolchains/json/bin/jq" ] && echo "SDK:json:otto:$HOME/.otto/toolchains/json/bin/jq" || true
for pair in "java:java" "go:go" "rust:rustc" "node:node" "python:python3" "shell:bash" "json:jq"; do
  e="${pair%%:*}"; b="${pair#*:}"
  command -v "$b" >/dev/null 2>&1 && echo "SDK:$e:system:$(command -v "$b")" || true
done
for pair in "java:jdtls:$HOME/.otto/toolchains/java/lsp/bin/jdtls" "go:gopls:$HOME/.otto/toolchains/go/bin/gopls" "rust:rust-analyzer:$HOME/.otto/toolchains/rust/bin/rust-analyzer" "node:typescript-language-server:$HOME/.otto/toolchains/node/lsp/node_modules/.bin/typescript-language-server" "python:pyright-langserver:$HOME/.otto/toolchains/python/bin/pyright-langserver" "shell:bash-language-server:$HOME/.otto/toolchains/shell/lsp/node_modules/.bin/bash-language-server" "json:vscode-json-language-server:$HOME/.otto/toolchains/json/lsp/node_modules/.bin/vscode-json-language-server"; do
  e="${pair%%:*}"; r="${pair#*:}"; b="${r%%:*}"; m="${r#*:}"
  if [ -x "$m" ]; then echo "LSP:$e:$m"; elif command -v "$b" >/dev/null 2>&1; then echo "LSP:$e:$(command -v "$b")"; fi
done
[ -f "$HOME/.otto/toolchains/java/dap/com.microsoft.java.debug.plugin-0.53.1.jar" ] && echo "DAP:java:$HOME/.otto/toolchains/java/dap/com.microsoft.java.debug.plugin-0.53.1.jar" || true
command -v dlv >/dev/null 2>&1 && echo "DAP:go:$(command -v dlv)" || true
if [ -x "$HOME/.otto/toolchains/rust/dap/extension/adapter/codelldb" ]; then echo "DAP:rust:$HOME/.otto/toolchains/rust/dap/extension/adapter/codelldb"; elif command -v codelldb >/dev/null 2>&1; then echo "DAP:rust:$(command -v codelldb)"; fi
[ -f "$HOME/.otto/toolchains/node/dap/js-debug/src/dapDebugServer.js" ] && echo "DAP:node:$HOME/.otto/toolchains/node/dap/js-debug/src/dapDebugServer.js" || true
[ -x "$HOME/.otto/toolchains/python/dap/bin/python" ] && echo "DAP:python:$HOME/.otto/toolchains/python/dap/bin/python" || true
if [ -x "$HOME/.otto/toolchains/shell/bin/bashdb" ]; then echo "DAP:shell:$HOME/.otto/toolchains/shell/bin/bashdb"; elif command -v bashdb >/dev/null 2>&1; then echo "DAP:shell:$(command -v bashdb)"; fi
if [ -x "$HOME/.otto/toolchains/json/bin/jq" ]; then echo "DAP:json:$HOME/.otto/toolchains/json/bin/jq"; elif command -v jq >/dev/null 2>&1; then echo "DAP:json:$(command -v jq)"; fi
"#;

#[derive(Default)]
pub(crate) struct ProbedRemoteEcosystem {
    pub installed: Vec<IdeInstalledSdk>,
    pub lsp: Option<IdeToolBinaryStatus>,
    pub dap: Option<IdeToolBinaryStatus>,
}

pub(crate) fn parse_ssh_probe_output(
    raw: &str,
) -> (
    TargetPlatform,
    HashMap<EcosystemKind, ProbedRemoteEcosystem>,
) {
    let mut platform = TargetPlatform::host();
    let mut map: HashMap<EcosystemKind, ProbedRemoteEcosystem> = HashMap::new();
    for line in raw.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("UNAME:") {
            if let Some(p) = TargetPlatform::from_combined_uname(rest) {
                platform = p;
            }
        } else if let Some(rest) = line.strip_prefix("SDK:") {
            let parts: Vec<&str> = rest.splitn(3, ':').collect();
            if parts.len() == 3 {
                if let Some(eco) = EcosystemKind::parse(parts[0]) {
                    let entry = map.entry(eco).or_default();
                    entry.installed.push(IdeInstalledSdk {
                        id: format!(
                            "{}-{}-{}",
                            eco.as_str(),
                            parts[1],
                            entry.installed.len() + 1
                        ),
                        version: dir_name_version(Path::new(parts[2])),
                        path: parts[2].to_string(),
                        source: parts[1].to_string(),
                        active: false,
                    });
                }
            }
        } else if let Some((eco_s, path)) =
            line.strip_prefix("LSP:").and_then(|r| r.split_once(':'))
        {
            if let Some(eco) = EcosystemKind::parse(eco_s) {
                map.entry(eco).or_default().lsp = Some(IdeToolBinaryStatus {
                    name: eco.lsp_name().to_string(),
                    available: true,
                    path: Some(path.to_string()),
                });
            }
        } else if let Some((eco_s, path)) =
            line.strip_prefix("DAP:").and_then(|r| r.split_once(':'))
        {
            if let Some(eco) = EcosystemKind::parse(eco_s) {
                map.entry(eco).or_default().dap = Some(IdeToolBinaryStatus {
                    name: eco.dap_name().to_string(),
                    available: true,
                    path: Some(path.to_string()),
                });
            }
        }
    }
    (platform, map)
}
