//! Local filesystem SDK/LSP/DAP discovery helpers.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::toolchains_catalog::{EcosystemKind, JAVA_DEBUG_JAR_NAME};
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
    system_search_dirs_with_shell(None)
}

pub(crate) fn system_search_dirs_with_shell(shell_path: Option<&str>) -> Vec<PathBuf> {
    let mut dirs = std::env::var_os("PATH")
        .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .unwrap_or_default();
    if let Some(sp) = shell_path {
        for p in std::env::split_paths(sp) {
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

#[cfg(test)]
pub(super) fn discover_local_sdks(roots: &[PathBuf], eco: EcosystemKind) -> Vec<IdeInstalledSdk> {
    discover_local_sdks_with_shell(roots, eco, None)
}

pub(super) fn discover_local_sdks_with_shell(
    roots: &[PathBuf],
    eco: EcosystemKind,
    shell_path: Option<&str>,
) -> Vec<IdeInstalledSdk> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for root in roots {
        scan_otto_sdks(root, eco, &mut seen, &mut out);
    }
    scan_system_sdks(eco, shell_path, &mut seen, &mut out);
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
    shell_path: Option<&str>,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<IdeInstalledSdk>,
) {
    let search = system_search_dirs_with_shell(shell_path);
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
                .and_then(|b| bin_prefix(&b))
                .filter(|jh| jh != Path::new("/usr"))
            {
                let v = read_java_release_version(&jh).unwrap_or_else(|| "system".into());
                push_sdk(out, seen, "java", &v, &jh, "system");
            }
        }
        EcosystemKind::Go => {
            for cand in ["/opt/homebrew/opt/go/libexec", "/usr/local/go"] {
                let p = PathBuf::from(cand);
                if is_goroot(&p) {
                    let v = read_go_root_version(&p).unwrap_or_else(|| "system".into());
                    push_sdk(out, seen, "go", &v, &p, "system");
                }
            }
            if let Some(gr) = find_executable_in(&search, &["go"])
                .and_then(|b| bin_prefix(&b))
                .filter(|gr| is_goroot(gr))
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
            if let Some(prefix) =
                find_executable_in(&search, &["node"]).and_then(|b| bin_prefix(&b))
            {
                push_sdk(
                    out,
                    seen,
                    "node",
                    &node_prefix_version(&prefix),
                    &prefix,
                    "system",
                );
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
                push_sdk(
                    out,
                    seen,
                    "python",
                    &python_bin_version(&py_bin),
                    &py_bin,
                    "system",
                );
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

#[cfg(test)]
pub(super) fn discover_local_lsp(roots: &[PathBuf], eco: EcosystemKind) -> IdeToolBinaryStatus {
    discover_local_lsp_with_shell(roots, eco, None)
}

pub(super) fn discover_local_lsp_with_shell(
    roots: &[PathBuf],
    eco: EcosystemKind,
    shell_path: Option<&str>,
) -> IdeToolBinaryStatus {
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
    if let Some(sys) = find_executable_in(&system_search_dirs_with_shell(shell_path), sys_names) {
        return IdeToolBinaryStatus {
            name,
            available: true,
            path: Some(sys.to_string_lossy().into_owned()),
        };
    }
    empty_tool(&name)
}

#[cfg(test)]
pub(super) fn discover_local_dap(roots: &[PathBuf], eco: EcosystemKind) -> IdeToolBinaryStatus {
    discover_local_dap_with_shell(roots, eco, None)
}

pub(super) fn discover_local_dap_with_shell(
    roots: &[PathBuf],
    eco: EcosystemKind,
    shell_path: Option<&str>,
) -> IdeToolBinaryStatus {
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
    if let Some(sys) = find_executable_in(&system_search_dirs_with_shell(shell_path), sys_names) {
        return IdeToolBinaryStatus {
            name,
            available: true,
            path: Some(sys.to_string_lossy().into_owned()),
        };
    }
    empty_tool(&name)
}

/// Name what will really debug when no dedicated adapter is installed: a bare
/// `python3` without the debugpy module runs Otto's bdb bridge, and a bare
/// shell runs Otto's DEBUG-trap adapter. Runs `python3 -c "import debugpy"`
/// once, so it is called only when the toolchain status is listed.
pub(super) fn label_builtin_dap(
    eco: EcosystemKind,
    mut st: IdeToolBinaryStatus,
) -> IdeToolBinaryStatus {
    let file = st
        .path
        .as_deref()
        .and_then(|p| Path::new(p).file_name()?.to_str().map(str::to_string))
        .unwrap_or_default();
    match eco {
        EcosystemKind::Python
            if file.starts_with("python")
                && !python_has_debugpy(st.path.as_deref().unwrap_or_default()) =>
        {
            st.name = "built-in bdb bridge".into();
        }
        EcosystemKind::Shell if matches!(file.as_str(), "bash" | "zsh") => {
            st.name = "built-in otto-sh-dap".into();
        }
        _ => {}
    }
    st
}

#[allow(clippy::disallowed_methods)] // called from the toolchain listing's blocking task
fn python_has_debugpy(python: &str) -> bool {
    std::process::Command::new(python)
        .args(["-c", "import debugpy"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
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

/// The install prefix of a PATH binary (`<prefix>/bin/<name>`), with symlinks
/// resolved first: Homebrew's `/opt/homebrew/bin/go` links into
/// `…/libexec/bin/go`, and its unresolved grandparent is `/opt/homebrew`.
pub(super) fn bin_prefix(bin: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(bin).ok()?;
    real.parent()?.parent().map(Path::to_path_buf)
}

/// A real GOROOT: a `go` binary next to the standard library.
pub(super) fn is_goroot(dir: &Path) -> bool {
    dir.join("bin/go").is_file()
        && (dir.join("VERSION").is_file() || dir.join("src/runtime").is_dir())
}

/// Two spellings of one SDK (`/opt/homebrew/opt/go/libexec` and its Cellar
/// target) are one row.
pub(super) fn canonical_key(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

/// `python3` on PATH resolves to `python3.13` (Homebrew, python.org): that's its version.
pub(super) fn python_bin_version(bin: &Path) -> String {
    std::fs::canonicalize(bin)
        .ok()
        .and_then(|real| {
            let name = real
                .file_name()?
                .to_str()?
                .strip_prefix("python")?
                .to_string();
            name.contains('.').then_some(name)
        })
        .unwrap_or_else(|| "3".into())
}

/// A Node prefix's version from its folder (`…/Cellar/node/26.1.0`), else "system".
pub(super) fn node_prefix_version(prefix: &Path) -> String {
    let v = dir_name_version(prefix);
    if v.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        v
    } else {
        "system".into()
    }
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
    if seen.insert(canonical_key(&path_str)) {
        out.push(IdeInstalledSdk {
            id: format!("{prefix}-{source}-{}", out.len() + 1),
            version: version.to_string(),
            path: path_str,
            source: source.to_string(),
            active: false,
        });
    }
}
