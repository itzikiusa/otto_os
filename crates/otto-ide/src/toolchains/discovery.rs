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
        version: None,
        outdated: None,
    }
}

pub(super) fn collect_managed_bin_dirs(tc_root: &Path, out: &mut Vec<PathBuf>) {
    if !tc_root.is_dir() {
        return;
    }
    for rel in [
        "java/lsp/bin",
        "java/current/bin",
        "kotlin/bin",
        "kotlin/current/bin",
        "kotlin/lsp/server/bin",
        "go/bin",
        "rust/bin",
        "rust/cargo/bin",
        "rust/dap/extension/adapter",
        "rust/codelldb/adapter",
        "zig/bin",
        "zig/current",
        "zig/current/bin",
        "cpp/bin",
        "cpp/current/bin",
        "cpp/dap/extension/adapter",
        "dotnet/current",
        "dotnet/tools",
        "dotnet/dap",
        "dotnet/netcoredbg",
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
    for sub in [
        "java/jdks",
        "kotlin/sdks",
        "go/sdks",
        "zig/sdks",
        "cpp/sdks",
        "dotnet/sdks",
        "node/sdks",
        "python/interpreters",
    ] {
        for p in read_subdirs(&tc_root.join(sub)) {
            if sub == "zig/sdks" && p.join("zig").is_file() && !out.contains(&p) {
                out.push(p.clone());
            }
            if sub == "dotnet/sdks" && p.join("dotnet").is_file() && !out.contains(&p) {
                out.push(p.clone());
            }
            for rel in ["bin", "Contents/Home/bin", "go/bin", "kotlinc/bin"] {
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
    for extra in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/local/share/dotnet",
        "/opt/homebrew/opt/dotnet/libexec",
        "/usr/bin",
        "/bin",
    ] {
        let p = PathBuf::from(extra);
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }
    if !otto_core::e2e::isolated() {
        if let Some(home) = dirs::home_dir() {
            for rel in [
                ".cargo/bin",
                "go/bin",
                ".local/bin",
                ".zvm/bin",
                ".dotnet",
                ".dotnet/tools",
            ] {
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
        EcosystemKind::Zig => {
            for dir in read_subdirs(&root.join("zig/sdks")) {
                if let Some(zh) = resolve_zig_home_dir(&dir) {
                    let v = zig_dir_version(&dir);
                    push_sdk(out, seen, "zig", &v, &zh, "otto");
                }
            }
        }
        EcosystemKind::Cpp => {
            for dir in read_subdirs(&root.join("cpp/sdks")) {
                if dir.join("bin/clang++").is_file() || dir.join("bin/clang").is_file() {
                    let v = dir_name_version(&dir);
                    push_sdk(out, seen, "cpp", &v, &dir, "otto");
                }
            }
        }
        EcosystemKind::Kotlin => {
            for dir in read_subdirs(&root.join("kotlin/sdks")) {
                let kh = if dir.join("kotlinc/bin/kotlinc").is_file() {
                    dir.join("kotlinc")
                } else {
                    dir.clone()
                };
                if kh.join("bin/kotlinc").is_file() {
                    let v = dir_name_version(&dir);
                    push_sdk(out, seen, "kotlin", &v, &kh, "otto");
                }
            }
        }
        EcosystemKind::Dotnet => {
            for dir in read_subdirs(&root.join("dotnet/sdks")) {
                if let Some(dr) = resolve_dotnet_root_dir(&dir) {
                    let v = dotnet_dir_version(&dr);
                    push_sdk(out, seen, "dotnet", &v, &dr, "otto");
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
        EcosystemKind::Ios | EcosystemKind::Android | EcosystemKind::Flutter => {}
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
        EcosystemKind::Zig => {
            for cand in [
                "/opt/homebrew/opt/zig",
                "/usr/local/opt/zig",
                "/usr/local/zig",
            ] {
                let p = PathBuf::from(cand);
                if let Some(zh) = resolve_zig_home_dir(&p) {
                    let v = zig_dir_version(&zh);
                    push_sdk(out, seen, "zig", &v, &zh, "system");
                }
            }
            if let Some(h) = &home {
                for (parent, src) in [
                    (h.join(".zvm"), "zvm"),
                    (h.join("zig"), "zigup"),
                    (h.join(".local/share/zigup"), "zigup"),
                    (h.join(".asdf/installs/zig"), "asdf"),
                    (h.join(".local/share/mise/installs/zig"), "mise"),
                ] {
                    for dir in read_subdirs(&parent) {
                        if let Some(zh) = resolve_zig_home_dir(&dir) {
                            let v = zig_dir_version(&dir);
                            push_sdk(out, seen, "zig", &v, &zh, src);
                        }
                    }
                }
            }
            if let Some(zig_bin) = find_executable_in(&search, &["zig"]) {
                if let Some(zh) = zig_home_from_bin(&zig_bin) {
                    let v = read_zig_bin_version(&zig_bin).unwrap_or_else(|| zig_dir_version(&zh));
                    push_sdk(out, seen, "zig", &v, &zh, "system");
                }
            }
        }
        EcosystemKind::Cpp => {
            for (cand, label_src) in [
                (
                    "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr",
                    "xcode",
                ),
                ("/Library/Developer/CommandLineTools/usr", "xcode-clt"),
                ("/opt/homebrew/opt/llvm", "homebrew"),
                ("/usr/local/opt/llvm", "homebrew"),
            ] {
                let p = PathBuf::from(cand);
                let bin = p.join("bin/clang++");
                if is_executable_file(&bin) {
                    let v = read_cpp_compiler_version(&bin)
                        .unwrap_or_else(|| dir_name_version(&p));
                    push_sdk(out, seen, "cpp", &v, &p, label_src);
                }
            }
            if let Some(cxx_bin) = find_executable_in(&search, &["clang++", "g++", "clang", "gcc"])
            {
                let prefix = bin_prefix(&cxx_bin).unwrap_or_else(|| {
                    cxx_bin
                        .parent()
                        .and_then(|p| p.parent())
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| PathBuf::from("/usr"))
                });
                let v = read_cpp_compiler_version(&cxx_bin).unwrap_or_else(|| "system".into());
                push_sdk(out, seen, "cpp", &v, &prefix, "system");
            }
        }
        EcosystemKind::Kotlin => {
            if let Some(h) = &home {
                for dir in read_subdirs(&h.join(".sdkman/candidates/kotlin")) {
                    if dir.join("bin/kotlinc").is_file() {
                        let v = dir_name_version(&dir);
                        push_sdk(out, seen, "kotlin", &v, &dir, "sdkman");
                    }
                }
            }
            for cand in [
                "/opt/homebrew/opt/kotlin/libexec",
                "/usr/local/opt/kotlin/libexec",
                "/opt/homebrew/opt/kotlin",
                "/usr/local/opt/kotlin",
            ] {
                let p = PathBuf::from(cand);
                if p.join("bin/kotlinc").is_file() {
                    let v = dir_name_version(&p);
                    push_sdk(out, seen, "kotlin", &v, &p, "homebrew");
                }
            }
            if let Some(kc) = find_executable_in(&search, &["kotlinc"]) {
                if let Some(kh) = bin_prefix(&kc) {
                    let v = dir_name_version(&kh);
                    push_sdk(out, seen, "kotlin", &v, &kh, "system");
                }
            }
        }
        EcosystemKind::Dotnet => {
            if let Ok(env_root) = std::env::var("DOTNET_ROOT") {
                let p = PathBuf::from(env_root.trim());
                if let Some(dr) = resolve_dotnet_root_dir(&p) {
                    let v = dotnet_dir_version(&dr);
                    push_sdk(out, seen, "dotnet", &v, &dr, "system");
                }
            }
            for cand in [
                "/usr/local/share/dotnet",
                "/opt/homebrew/opt/dotnet/libexec",
                "/usr/local/opt/dotnet/libexec",
                "/usr/share/dotnet",
            ] {
                let p = PathBuf::from(cand);
                if let Some(dr) = resolve_dotnet_root_dir(&p) {
                    let v = dotnet_dir_version(&dr);
                    push_sdk(out, seen, "dotnet", &v, &dr, "system");
                }
            }
            if let Some(h) = &home {
                let dot_dotnet = h.join(".dotnet");
                if let Some(dr) = resolve_dotnet_root_dir(&dot_dotnet) {
                    let v = dotnet_dir_version(&dr);
                    push_sdk(out, seen, "dotnet", &v, &dr, "system");
                }
                for (parent, src) in [
                    (h.join(".asdf/installs/dotnet-core"), "asdf"),
                    (h.join(".asdf/installs/dotnet"), "asdf"),
                    (h.join(".local/share/mise/installs/dotnet"), "mise"),
                ] {
                    for dir in read_subdirs(&parent) {
                        if let Some(dr) = resolve_dotnet_root_dir(&dir) {
                            let v = dotnet_dir_version(&dr);
                            push_sdk(out, seen, "dotnet", &v, &dr, src);
                        }
                    }
                }
            }
            if let Some(bin) = find_executable_in(&search, &["dotnet"]) {
                if let Some(dr) = dotnet_root_from_bin(&bin) {
                    let v =
                        read_dotnet_bin_version(&bin).unwrap_or_else(|| dotnet_dir_version(&dr));
                    push_sdk(out, seen, "dotnet", &v, &dr, "system");
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
        EcosystemKind::Ios | EcosystemKind::Android | EcosystemKind::Flutter => {
            // System mobile SDKs are populated by `detected.rs::scan_mobile_sdks`
            // and merged into `installed` in `ToolchainManager::list_local`.
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
        EcosystemKind::Kotlin => &[
            "kotlin/bin/kotlin-language-server",
            "kotlin/lsp/server/bin/kotlin-language-server",
        ],
        EcosystemKind::Go => &["go/bin/gopls"],
        EcosystemKind::Rust => &["rust/bin/rust-analyzer", "rust/cargo/bin/rust-analyzer"],
        EcosystemKind::Zig => &["zig/bin/zls"],
        EcosystemKind::Cpp => &["cpp/bin/clangd", "cpp/current/bin/clangd"],
        EcosystemKind::Dotnet => &[
            "dotnet/tools/csharp-ls",
            "dotnet/bin/csharp-ls",
            "dotnet/tools/fsautocomplete",
        ],
        EcosystemKind::Node => &["node/lsp/node_modules/.bin/typescript-language-server"],
        EcosystemKind::Python => &["python/bin/pyright-langserver", "python/bin/pyright"],
        EcosystemKind::Shell => &["shell/lsp/node_modules/.bin/bash-language-server"],
        EcosystemKind::Json => &[
            "json/lsp/node_modules/.bin/vscode-json-language-server",
            "json/lsp/node_modules/.bin/vscode-json-languageserver",
        ],
        EcosystemKind::Ios | EcosystemKind::Android | EcosystemKind::Flutter => &[],
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
                version: None,
                outdated: None,
            };
        }
    }
    let sys_names: &[&str] = match eco {
        EcosystemKind::Java => &["jdtls"],
        EcosystemKind::Kotlin => &["kotlin-language-server"],
        EcosystemKind::Go => &["gopls"],
        EcosystemKind::Rust => &["rust-analyzer"],
        EcosystemKind::Zig => &["zls"],
        EcosystemKind::Cpp => &["clangd"],
        EcosystemKind::Dotnet => &[
            "csharp-ls",
            "OmniSharp",
            "omnisharp",
            "Microsoft.CodeAnalysis.LanguageServer",
            "fsautocomplete",
        ],
        EcosystemKind::Node => &["typescript-language-server"],
        EcosystemKind::Python => &["pyright-langserver", "pyright"],
        EcosystemKind::Shell => &["bash-language-server"],
        EcosystemKind::Json => &["vscode-json-language-server", "vscode-json-languageserver"],
        EcosystemKind::Ios => &["sourcekit-lsp"],
        EcosystemKind::Android => &["kotlin-language-server"],
        EcosystemKind::Flutter => &["dart"],
    };
    let mut search = system_search_dirs_with_shell(shell_path);
    if matches!(
        eco,
        EcosystemKind::Rust | EcosystemKind::Zig | EcosystemKind::Cpp | EcosystemKind::Ios
    ) {
        for extra_dir in [
            "/opt/homebrew/opt/llvm/bin",
            "/usr/local/opt/llvm/bin",
            "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin",
            "/Applications/Xcode.app/Contents/Developer/usr/bin",
        ] {
            let p = PathBuf::from(extra_dir);
            if p.is_dir() && !search.contains(&p) {
                search.push(p);
            }
        }
    }
    if let Some(sys) = find_executable_in(&search, sys_names) {
        return IdeToolBinaryStatus {
            name,
            available: true,
            path: Some(sys.to_string_lossy().into_owned()),
            version: None,
            outdated: None,
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
        EcosystemKind::Java | EcosystemKind::Kotlin => vec![java_rel.as_str()],
        EcosystemKind::Go => vec!["go/bin/dlv", "dlv/dlv"],
        EcosystemKind::Rust | EcosystemKind::Zig | EcosystemKind::Cpp => vec![
            "cpp/dap/extension/adapter/codelldb",
            "rust/dap/extension/adapter/codelldb",
            "rust/codelldb/adapter/codelldb",
            "zig/dap/extension/adapter/codelldb",
        ],
        EcosystemKind::Dotnet => vec![
            "dotnet/dap/netcoredbg/netcoredbg",
            "dotnet/dap/netcoredbg",
            "dotnet/netcoredbg/netcoredbg",
            "dotnet/bin/netcoredbg",
        ],
        EcosystemKind::Node => vec![
            "node/dap/js-debug/src/dapDebugServer.js",
            "node/js-debug/src/dapDebugServer.js",
        ],
        EcosystemKind::Python => vec!["python/dap/bin/python"],
        EcosystemKind::Shell => vec!["shell/bin/bashdb"],
        EcosystemKind::Json => vec!["json/bin/jq"],
        EcosystemKind::Ios | EcosystemKind::Android | EcosystemKind::Flutter => Vec::new(),
    };
    for root in roots {
        for rel in &rels {
            let cand = root.join(rel);
            let ok = match eco {
                EcosystemKind::Java
                | EcosystemKind::Kotlin
                | EcosystemKind::Node
                | EcosystemKind::Python => cand.is_file(),
                EcosystemKind::Go
                | EcosystemKind::Rust
                | EcosystemKind::Zig
                | EcosystemKind::Cpp
                | EcosystemKind::Dotnet
                | EcosystemKind::Shell
                | EcosystemKind::Json
                | EcosystemKind::Ios
                | EcosystemKind::Android
                | EcosystemKind::Flutter => is_executable_file(&cand),
            };
            if ok {
                return IdeToolBinaryStatus {
                    name,
                    available: true,
                    path: Some(cand.to_string_lossy().into_owned()),
                    version: None,
                    outdated: None,
                };
            }
        }
    }
    let sys_names: &[&str] = match eco {
        EcosystemKind::Java | EcosystemKind::Kotlin => &["java-debug", "java"],
        EcosystemKind::Go => &["dlv"],
        EcosystemKind::Rust | EcosystemKind::Zig | EcosystemKind::Cpp => {
            &["codelldb", "lldb-dap", "lldb"]
        }
        EcosystemKind::Dotnet => &["netcoredbg", "vsdbg"],
        EcosystemKind::Node => &["js-debug-dap", "node"],
        EcosystemKind::Python => &["debugpy", "debugpy-adapter", "python3"],
        EcosystemKind::Shell => &["bashdb", "bash", "zsh"],
        EcosystemKind::Json => &["jq"],
        EcosystemKind::Ios => &["lldb-dap", "lldb"],
        EcosystemKind::Android => &["adb"],
        EcosystemKind::Flutter => &["flutter"],
    };
    let mut search = system_search_dirs_with_shell(shell_path);
    if matches!(
        eco,
        EcosystemKind::Rust | EcosystemKind::Zig | EcosystemKind::Cpp | EcosystemKind::Ios
    ) {
        for extra_dir in [
            "/opt/homebrew/opt/llvm/bin",
            "/usr/local/opt/llvm/bin",
            "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin",
            "/Applications/Xcode.app/Contents/Developer/usr/bin",
        ] {
            let p = PathBuf::from(extra_dir);
            if p.is_dir() && !search.contains(&p) {
                search.push(p);
            }
        }
    }
    if let Some(sys) = find_executable_in(&search, sys_names) {
        return IdeToolBinaryStatus {
            name,
            available: true,
            path: Some(sys.to_string_lossy().into_owned()),
            version: None,
            outdated: None,
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

/// `major.minor` of a Go version (`go1.25.3`, `1.25`) for comparison.
fn go_major_minor(v: &str) -> Option<(u32, u32)> {
    let mut parts = v.trim().trim_start_matches("go").split(['.', '-', ' ']);
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// The Go a binary was built with: `go version <bin>` prints `<bin>: go1.22.3`.
#[allow(clippy::disallowed_methods)] // called from the toolchain listing's blocking task
fn go_built_with(goroot: &Path, bin: &str) -> Option<String> {
    let out = std::process::Command::new(goroot.join("bin/go"))
        .args(["version", bin])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let v = text.rsplit(": ").next()?.split_whitespace().next()?;
    v.starts_with("go").then(|| v.to_string())
}

/// gopls / Delve built with an older Go than the project's (`goroot`) can't
/// load or debug its code ("Version of Delve is too old"): record the build
/// version and why it is outdated. One `go version` exec per tool, only when
/// the toolchain status is listed.
pub(super) fn mark_go_tool_outdated(st: &mut IdeToolBinaryStatus, goroot: Option<&Path>) {
    let (Some(goroot), Some(path)) = (goroot, st.path.as_deref()) else {
        return;
    };
    let Some(project) = read_go_root_version(goroot) else {
        return;
    };
    st.version = go_built_with(goroot, path);
    let built = st.version.as_deref().and_then(go_major_minor);
    if let (Some(b), Some(p)) = (built, go_major_minor(&project)) {
        if b < p {
            st.outdated = Some(format!(
                "built with go{}.{}, project uses go{}.{}",
                b.0, b.1, p.0, p.1
            ));
        }
    }
}

/// `major.minor` of a Zig / ZLS version (`0.14.0`, `0.14.0-dev.123`) for comparison.
pub(crate) fn zig_major_minor(v: &str) -> Option<(u32, u32)> {
    let mut parts = v.trim().trim_start_matches('v').split(['.', '-', '+', ' ']);
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

#[allow(clippy::disallowed_methods)]
pub(crate) fn read_zig_bin_version(zig_bin: &Path) -> Option<String> {
    let out = std::process::Command::new(zig_bin)
        .arg("version")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let ver = text.lines().next()?.trim();
    (!ver.is_empty()).then(|| ver.to_string())
}

#[allow(clippy::disallowed_methods)]
pub(crate) fn read_zls_bin_version(zls_bin: &Path) -> Option<String> {
    let out = std::process::Command::new(zls_bin)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let ver = text.lines().next()?.trim();
    (!ver.is_empty()).then(|| ver.to_string())
}

/// Locate the `zig` executable inside a Zig SDK home (`<home>/zig` or `<home>/bin/zig`).
pub(crate) fn zig_bin_in_home(zig_home: &Path) -> Option<PathBuf> {
    for rel in ["bin/zig", "zig"] {
        let cand = zig_home.join(rel);
        if is_executable_file(&cand) {
            return Some(cand);
        }
    }
    None
}

/// Resolve a Zig SDK home directory from either a parent extraction directory or direct SDK dir.
pub(crate) fn resolve_zig_home_dir(dir: &Path) -> Option<PathBuf> {
    if zig_bin_in_home(dir).is_some() {
        return Some(dir.to_path_buf());
    }
    read_subdirs(dir)
        .into_iter()
        .find(|sub| zig_bin_in_home(sub).is_some())
}

/// Derive a Zig SDK home directory from a `zig` executable path (resolving symlinks).
#[allow(clippy::disallowed_methods)]
pub(crate) fn zig_home_from_bin(bin: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(bin).unwrap_or_else(|_| bin.to_path_buf());
    let parent = real.parent()?;
    if parent.file_name().and_then(|n| n.to_str()) == Some("bin") {
        parent.parent().map(Path::to_path_buf)
    } else {
        Some(parent.to_path_buf())
    }
}

/// Extract a Zig version string from a directory name like `zig-macos-aarch64-0.14.0`, `0.14.0_2`, `0.14.0`.
pub(crate) fn zig_dir_version(dir: &Path) -> String {
    let raw = dir.file_name().and_then(|s| s.to_str()).unwrap_or("system");
    let stripped = raw
        .trim_start_matches("zig-macos-aarch64-")
        .trim_start_matches("zig-macos-x86_64-")
        .trim_start_matches("zig-linux-aarch64-")
        .trim_start_matches("zig-linux-x86_64-")
        .trim_start_matches("zig-aarch64-macos-")
        .trim_start_matches("zig-x86_64-macos-")
        .trim_start_matches("zig-aarch64-linux-")
        .trim_start_matches("zig-x86_64-linux-")
        .trim_start_matches("zig-")
        .trim_start_matches('v');
    let clean = stripped.split('_').next().unwrap_or(stripped);
    if clean.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        clean.to_string()
    } else if let Some(bin) = zig_bin_in_home(dir) {
        read_zig_bin_version(&bin).unwrap_or_else(|| "system".into())
    } else {
        "system".into()
    }
}

/// ZLS must match the active Zig compiler's `major.minor` version (e.g. Zig 0.14.x <-> ZLS 0.14.x).
/// Record `zls --version` and flag `outdated` when `major.minor` diverges from the active Zig SDK.
pub(super) fn mark_zig_tool_outdated(
    st: &mut IdeToolBinaryStatus,
    active_zig_home: Option<&Path>,
    active_zig_version: Option<&str>,
) {
    let Some(zls_path) = st.path.as_deref() else {
        return;
    };
    let Some(zls_ver) = read_zls_bin_version(Path::new(zls_path)) else {
        return;
    };
    st.version = Some(zls_ver.clone());
    let zig_ver = active_zig_version
        .map(str::to_string)
        .filter(|v| zig_major_minor(v).is_some())
        .or_else(|| {
            active_zig_home
                .and_then(zig_bin_in_home)
                .and_then(|b| read_zig_bin_version(&b))
        });
    if let (Some(zb), Some(zv)) = (
        zig_major_minor(&zls_ver),
        zig_ver.as_deref().and_then(zig_major_minor),
    ) {
        if zb != zv {
            st.outdated = Some(format!(
                "zls {zls_ver} does not match Zig {}.{} — reinstall ZLS for Zig {}.{}",
                zv.0, zv.1, zv.0, zv.1
            ));
        }
    }
}

/// Locate the `dotnet` executable inside a .NET SDK root (`<root>/dotnet` or `<root>/bin/dotnet`).
pub(crate) fn dotnet_bin_in_root(dotnet_root: &Path) -> Option<PathBuf> {
    for rel in ["dotnet", "bin/dotnet"] {
        let cand = dotnet_root.join(rel);
        if is_executable_file(&cand) {
            return Some(cand);
        }
    }
    None
}

/// Resolve a .NET SDK root directory containing `dotnet` (or `libexec/dotnet`).
pub(crate) fn resolve_dotnet_root_dir(dir: &Path) -> Option<PathBuf> {
    if dotnet_bin_in_root(dir).is_some() {
        return Some(dir.to_path_buf());
    }
    let libexec = dir.join("libexec");
    if dotnet_bin_in_root(&libexec).is_some() {
        return Some(libexec);
    }
    read_subdirs(dir)
        .into_iter()
        .find(|sub| dotnet_bin_in_root(sub).is_some())
}

/// Derive a .NET SDK root from a `dotnet` executable path (resolving symlinks).
#[allow(clippy::disallowed_methods)]
pub(crate) fn dotnet_root_from_bin(bin: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(bin).unwrap_or_else(|_| bin.to_path_buf());
    let parent = real.parent()?;
    if parent.file_name().and_then(|n| n.to_str()) == Some("bin") {
        let grand = parent.parent()?;
        if grand.join("sdk").is_dir() {
            return Some(grand.to_path_buf());
        }
        if grand.join("libexec/sdk").is_dir() {
            return Some(grand.join("libexec"));
        }
        Some(grand.to_path_buf())
    } else {
        Some(parent.to_path_buf())
    }
}

/// Read the latest installed SDK version under `<dotnet_root>/sdk/*` (zero-subprocess).
pub(crate) fn dotnet_dir_version(dotnet_root: &Path) -> String {
    let mut versions: Vec<String> = read_subdirs(&dotnet_root.join("sdk"))
        .into_iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_string))
        .filter(|s| s.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .collect();
    versions.sort();
    if let Some(latest) = versions.pop() {
        return latest;
    }
    let name = dir_name_version(dotnet_root);
    if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        name
    } else {
        "system".into()
    }
}

#[allow(clippy::disallowed_methods)]
pub(crate) fn read_dotnet_bin_version(dotnet_bin: &Path) -> Option<String> {
    let out = std::process::Command::new(dotnet_bin)
        .arg("--version")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let ver = text.lines().next()?.trim();
    (!ver.is_empty()).then(|| ver.to_string())
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

#[allow(clippy::disallowed_methods)]
pub(crate) fn read_cpp_compiler_version(bin: &Path) -> Option<String> {
    let out = std::process::Command::new(bin)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next()?.trim();
    for tok in first.split_whitespace() {
        let clean = tok.trim_matches(|c: char| matches!(c, '(' | ')' | ','));
        if clean.chars().next().is_some_and(|c| c.is_ascii_digit()) && clean.contains('.') {
            return Some(clean.to_string());
        }
    }
    (!first.is_empty()).then(|| first.chars().take(32).collect())
}

pub(super) fn mark_active_sdk(
    installed: &mut [IdeInstalledSdk],
    eco: EcosystemKind,
    active_env: Option<&IdeProjectEnvironment>,
) {
    let chosen = active_env.and_then(|env| match eco {
        EcosystemKind::Java => env.java_home.as_deref(),
        EcosystemKind::Kotlin => env.kotlin_home.as_deref(),
        EcosystemKind::Go => env.go_root.as_deref(),
        EcosystemKind::Rust => env.rust_toolchain.as_deref(),
        EcosystemKind::Zig => env.zig_home.as_deref(),
        EcosystemKind::Cpp => env.cpp_toolchain.as_deref(),
        EcosystemKind::Dotnet => env.dotnet_root.as_deref(),
        EcosystemKind::Node => env.node_home.as_deref(),
        EcosystemKind::Python => env.python_interpreter.as_deref(),
        EcosystemKind::Ios => env.xcode_developer_dir.as_deref(),
        EcosystemKind::Android => env.android_home.as_deref(),
        EcosystemKind::Flutter => env.flutter_root.as_deref(),
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
