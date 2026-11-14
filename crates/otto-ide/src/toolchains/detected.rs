//! Fast (`< 20ms`, zero subprocess spawns) local disk SDK scanner populating
//! `IdeToolchainsResp::detected` for Java, Go, Node, Python, and Rust on macOS/Linux.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::types::{IdeDetectedSdk, IdeEcosystemKind};

use super::discovery::{
    dir_name_version, find_executable_in, is_executable_file, python_dir_version,
    read_go_root_version, read_java_release_version, read_subdirs, resolve_java_home_dir,
    rust_toolchain_label, system_search_dirs,
};

pub(super) fn scan_detected_sdks(
    otto_roots: &[PathBuf],
    project_roots: &[PathBuf],
) -> Vec<IdeDetectedSdk> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let home = dirs::home_dir();
    let search = system_search_dirs();

    // 1. Otto-managed SDKs
    for root in otto_roots {
        for dir in read_subdirs(&root.join("java/jdks")) {
            if let Some(jh) = resolve_java_home_dir(&dir) {
                let v = read_java_release_version(&jh).unwrap_or_else(|| dir_name_version(&dir));
                push_det(
                    &mut out,
                    &mut seen,
                    "java",
                    format!("JDK {v}"),
                    &v,
                    &jh,
                    "otto",
                );
            }
        }
        for dir in read_subdirs(&root.join("go/sdks")) {
            let gr = if dir.join("go/bin/go").is_file() {
                dir.join("go")
            } else {
                dir.clone()
            };
            if gr.join("bin/go").is_file() {
                let v = read_go_root_version(&gr).unwrap_or_else(|| dir_name_version(&dir));
                push_det(
                    &mut out,
                    &mut seen,
                    "go",
                    format!("Go {v}"),
                    &v,
                    &gr,
                    "otto",
                );
            }
        }
        for dir in read_subdirs(&root.join("node/sdks")) {
            if dir.join("bin/node").is_file() {
                let v = dir_name_version(&dir);
                push_det(
                    &mut out,
                    &mut seen,
                    "node",
                    format!("Node {v}"),
                    &v,
                    &dir,
                    "otto",
                );
            }
        }
        for dir in read_subdirs(&root.join("python/interpreters")) {
            let py = dir.join("bin/python3");
            if py.is_file() {
                let v = python_dir_version(&dir);
                push_det(
                    &mut out,
                    &mut seen,
                    "python",
                    format!("Python {v}"),
                    &v,
                    &py,
                    "otto",
                );
            }
        }
        for dir in read_subdirs(&root.join("rust/rustup/toolchains")) {
            if dir.join("bin/rustc").is_file() {
                let v = rust_toolchain_label(&dir);
                push_det(
                    &mut out,
                    &mut seen,
                    "rust",
                    format!("Rust {v}"),
                    &v,
                    &dir,
                    "otto",
                );
            }
        }
    }

    // 2. Java: macOS JVMs, Homebrew OpenJDK, SDKMAN, /usr/lib/jvm
    let mut jvm_parents = vec![
        ("/Library/Java/JavaVirtualMachines".into(), "system"),
        ("/usr/lib/jvm".into(), "system"),
    ];
    if let Some(h) = &home {
        jvm_parents.push((h.join("Library/Java/JavaVirtualMachines"), "user-jvm"));
        jvm_parents.push((h.join(".sdkman/candidates/java"), "sdkman"));
    }
    for (parent, src) in jvm_parents {
        for dir in read_subdirs(&parent) {
            let folder_label = dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("JDK")
                .trim_end_matches(".jdk")
                .to_string();
            if let Some(jh) = resolve_java_home_dir(&dir) {
                let v = read_java_release_version(&jh).unwrap_or_else(|| dir_name_version(&dir));
                push_det(
                    &mut out,
                    &mut seen,
                    "java",
                    format!("{folder_label} ({v})"),
                    &v,
                    &jh,
                    src,
                );
            }
        }
    }
    for brew_opt in ["/opt/homebrew/opt", "/usr/local/opt"] {
        for dir in read_subdirs(Path::new(brew_opt)) {
            let nm = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if nm.starts_with("openjdk") || nm.starts_with("java") {
                for cand in [dir.join("libexec/openjdk.jdk/Contents/Home"), dir.clone()] {
                    if cand.join("bin/java").is_file() {
                        let v = read_java_release_version(&cand)
                            .unwrap_or_else(|| dir_name_version(&dir));
                        push_det(
                            &mut out,
                            &mut seen,
                            "java",
                            format!("Homebrew {nm} ({v})"),
                            &v,
                            &cand,
                            "homebrew",
                        );
                        break;
                    }
                }
            }
        }
    }

    // 3. Go: Homebrew, /usr/local/go, ~/sdk/go*, ~/go/pkg/mod/golang.org/toolchain@*
    for brew_opt in ["/opt/homebrew/opt", "/usr/local/opt"] {
        for dir in read_subdirs(Path::new(brew_opt)) {
            let nm = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if nm == "go" || nm.starts_with("go@") {
                let gr = dir.join("libexec");
                if gr.join("bin/go").is_file() {
                    let v = read_go_root_version(&gr).unwrap_or_else(|| dir_name_version(&dir));
                    push_det(
                        &mut out,
                        &mut seen,
                        "go",
                        format!("Homebrew Go {v}"),
                        &v,
                        &gr,
                        "homebrew",
                    );
                }
            }
        }
    }
    let usr_go = PathBuf::from("/usr/local/go");
    if usr_go.join("bin/go").is_file() {
        let v = read_go_root_version(&usr_go).unwrap_or_else(|| "system".into());
        push_det(
            &mut out,
            &mut seen,
            "go",
            format!("Go {v} (/usr/local/go)"),
            &v,
            &usr_go,
            "system",
        );
    }
    if let Some(h) = &home {
        for dir in read_subdirs(&h.join("sdk")) {
            let nm = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if nm.starts_with("go") && dir.join("bin/go").is_file() {
                let v = read_go_root_version(&dir).unwrap_or_else(|| dir_name_version(&dir));
                push_det(
                    &mut out,
                    &mut seen,
                    "go",
                    format!("Go {v} (~/sdk)"),
                    &v,
                    &dir,
                    "sdk",
                );
            }
        }
        for dir in read_subdirs(&h.join("go/pkg/mod/golang.org")) {
            let nm = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if nm.starts_with("toolchain@") && dir.join("bin/go").is_file() {
                let v = read_go_root_version(&dir).unwrap_or_else(|| dir_name_version(&dir));
                push_det(
                    &mut out,
                    &mut seen,
                    "go",
                    format!("Go Toolchain {v}"),
                    &v,
                    &dir,
                    "gotoolchain",
                );
            }
        }
    }
    if let Some(go_bin) = find_executable_in(&search, &["go"]) {
        if let Some(gr) = go_bin.parent().and_then(|p| p.parent()) {
            if gr.join("bin/go").is_file() {
                let v = read_go_root_version(gr).unwrap_or_else(|| "system".into());
                push_det(&mut out, &mut seen, "go", format!("Go {v}"), &v, gr, "path");
            }
        }
    }

    // 4. Node: nvm, fnm, volta, Homebrew, PATH
    if let Some(h) = &home {
        for dir in read_subdirs(&h.join(".nvm/versions/node")) {
            if dir.join("bin/node").is_file() {
                let v = dir_name_version(&dir);
                push_det(
                    &mut out,
                    &mut seen,
                    "node",
                    format!("Node {v} (nvm)"),
                    &v,
                    &dir,
                    "nvm",
                );
            }
        }
        for fnm_base in [
            h.join(".fnm/node-versions"),
            h.join("Library/Application Support/fnm/node-versions"),
        ] {
            for dir in read_subdirs(&fnm_base) {
                let inst = dir.join("installation");
                if inst.join("bin/node").is_file() {
                    let v = dir_name_version(&dir);
                    push_det(
                        &mut out,
                        &mut seen,
                        "node",
                        format!("Node {v} (fnm)"),
                        &v,
                        &inst,
                        "fnm",
                    );
                }
            }
        }
        for dir in read_subdirs(&h.join(".volta/tools/image/node")) {
            if dir.join("bin/node").is_file() {
                let v = dir_name_version(&dir);
                push_det(
                    &mut out,
                    &mut seen,
                    "node",
                    format!("Node {v} (volta)"),
                    &v,
                    &dir,
                    "volta",
                );
            }
        }
    }
    if let Some(node_bin) = find_executable_in(&search, &["node"]) {
        if let Some(prefix) = node_bin.parent().and_then(|p| p.parent()) {
            let v = dir_name_version(prefix);
            let ver = if v == "homebrew" || v == "local" || v == "usr" {
                "system".to_string()
            } else {
                v
            };
            push_det(
                &mut out,
                &mut seen,
                "node",
                format!("Node ({ver})"),
                &ver,
                prefix,
                "system",
            );
        }
    }

    scan_python_and_rust_sdks(project_roots, home.as_deref(), &mut out, &mut seen);
    out
}

fn scan_python_and_rust_sdks(
    project_roots: &[PathBuf],
    home: Option<&Path>,
    out: &mut Vec<IdeDetectedSdk>,
    seen: &mut BTreeSet<String>,
) {
    // 5. Python: per-project/module virtualenvs (.venv, venv, .env), Poetry, Conda, pyenv, uv, Homebrew, /usr/bin/python3
    for proot in project_roots {
        let proj_name = proot
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("project");
        for venv_name in [".venv", "venv", ".env"] {
            let vdir = proot.join(venv_name);
            let py = vdir.join("bin/python3");
            let py_alt = vdir.join("bin/python");
            let bin = if py.is_file() {
                Some(py)
            } else if py_alt.is_file() {
                Some(py_alt)
            } else {
                None
            };
            if let Some(pbin) = bin {
                let ver = read_pyvenv_cfg_version(&vdir).unwrap_or_else(|| "3".into());
                push_det(
                    out,
                    seen,
                    "python",
                    format!("{proj_name}/{venv_name} (Python {ver})"),
                    &ver,
                    &pbin,
                    "venv",
                );
            }
        }
    }
    if let Some(h) = home {
        for poetry_base in [
            h.join("Library/Caches/pypoetry/virtualenvs"),
            h.join(".cache/pypoetry/virtualenvs"),
        ] {
            for dir in read_subdirs(&poetry_base) {
                let py = dir.join("bin/python");
                if py.is_file() {
                    let ver = read_pyvenv_cfg_version(&dir).unwrap_or_else(|| "3".into());
                    let nm = dir.file_name().and_then(|s| s.to_str()).unwrap_or("poetry");
                    push_det(
                        out,
                        seen,
                        "python",
                        format!("{nm} (Poetry {ver})"),
                        &ver,
                        &py,
                        "poetry",
                    );
                }
            }
        }
        for conda_base in [
            h.join("miniconda3/envs"),
            h.join("anaconda3/envs"),
            h.join("miniforge3/envs"),
            h.join(".conda/envs"),
        ] {
            for dir in read_subdirs(&conda_base) {
                let py = dir.join("bin/python3");
                if py.is_file() {
                    let nm = dir.file_name().and_then(|s| s.to_str()).unwrap_or("conda");
                    push_det(
                        out,
                        seen,
                        "python",
                        format!("{nm} (Conda)"),
                        "3",
                        &py,
                        "conda",
                    );
                }
            }
        }
        for dir in read_subdirs(&h.join(".pyenv/versions")) {
            let py = dir.join("bin/python3");
            if py.is_file() {
                let v = python_dir_version(&dir);
                push_det(
                    out,
                    seen,
                    "python",
                    format!("Python {v} (pyenv)"),
                    &v,
                    &py,
                    "pyenv",
                );
            }
        }
        for dir in read_subdirs(&h.join(".local/share/uv/python")) {
            let py = dir.join("bin/python3");
            if py.is_file() {
                let v = python_dir_version(&dir);
                push_det(
                    out,
                    seen,
                    "python",
                    format!("Python {v} (uv)"),
                    &v,
                    &py,
                    "uv",
                );
            }
        }
    }
    for brew_opt in ["/opt/homebrew/opt", "/usr/local/opt"] {
        for dir in read_subdirs(Path::new(brew_opt)) {
            let nm = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if nm.starts_with("python@") {
                for rel in ["bin/python3", "libexec/bin/python3"] {
                    let py = dir.join(rel);
                    if py.is_file() {
                        let v = nm.trim_start_matches("python@").to_string();
                        push_det(
                            out,
                            seen,
                            "python",
                            format!("Homebrew Python {v}"),
                            &v,
                            &py,
                            "homebrew",
                        );
                        break;
                    }
                }
            }
        }
    }
    let sys_py = PathBuf::from("/usr/bin/python3");
    if is_executable_file(&sys_py) {
        push_det(
            out,
            seen,
            "python",
            "System Python 3 (/usr/bin/python3)".into(),
            "3",
            &sys_py,
            "system",
        );
    }

    // 6. Rust: ~/.rustup/toolchains/*
    if let Some(h) = home {
        for dir in read_subdirs(&h.join(".rustup/toolchains")) {
            if dir.join("bin/rustc").is_file() {
                let raw = dir.file_name().and_then(|s| s.to_str()).unwrap_or("stable");
                let v = rust_toolchain_label(&dir);
                push_det(out, seen, "rust", format!("Rust {raw}"), &v, &dir, "rustup");
            }
        }
    }
}

pub(super) fn read_pyvenv_cfg_version(venv_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(venv_dir.join("pyvenv.cfg")).ok()?;
    for line in raw.lines() {
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            if key == "version" || key == "version_info" {
                let ver = v.trim();
                if !ver.is_empty() {
                    return Some(ver.to_string());
                }
            }
        }
    }
    None
}

fn push_det(
    out: &mut Vec<IdeDetectedSdk>,
    seen: &mut BTreeSet<String>,
    ecosystem: &str,
    label: String,
    version: &str,
    path: &Path,
    source: &str,
) {
    let eco = match ecosystem {
        "java" => IdeEcosystemKind::Java,
        "go" => IdeEcosystemKind::Go,
        "rust" => IdeEcosystemKind::Rust,
        "node" => IdeEcosystemKind::Node,
        "python" => IdeEcosystemKind::Python,
        "shell" => IdeEcosystemKind::Shell,
        _ => IdeEcosystemKind::Json,
    };
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let key = format!("{ecosystem}:{}", canon.to_string_lossy());
    if seen.insert(key) {
        let (home_path, bin_path) = if path.is_file() {
            let home = path
                .parent()
                .and_then(|b| b.parent())
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned();
            (home, path.to_string_lossy().into_owned())
        } else {
            let bin = match eco {
                IdeEcosystemKind::Java => path.join("bin/java"),
                IdeEcosystemKind::Go => path.join("bin/go"),
                IdeEcosystemKind::Rust => path.join("bin/rustc"),
                IdeEcosystemKind::Node => path.join("bin/node"),
                IdeEcosystemKind::Python => path.join("bin/python3"),
                _ => path.to_path_buf(),
            };
            (
                path.to_string_lossy().into_owned(),
                bin.to_string_lossy().into_owned(),
            )
        };
        let valid = Path::new(&bin_path).exists() || path.exists();
        out.push(IdeDetectedSdk {
            ecosystem: eco,
            name: label,
            subtitle: format!("{source} · {}", path.to_string_lossy()),
            version: version.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            home_path,
            bin_path,
            path: path.to_string_lossy().into_owned(),
            source: source.to_string(),
            valid,
        });
    }
}
