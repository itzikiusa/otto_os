//! Fast (`< 20ms`, zero subprocess spawns) local disk SDK scanner populating
//! `IdeToolchainsResp::detected` for Java, Go, Node, Python, and Rust on macOS/Linux.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::types::{IdeDetectedSdk, IdeEcosystemKind};

use super::discovery::{
    bin_prefix, dir_name_version, find_executable_in, is_executable_file, is_goroot,
    node_prefix_version, python_dir_version, read_go_root_version, read_java_release_version,
    read_subdirs, resolve_java_home_dir, rust_toolchain_label, system_search_dirs,
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
        // Otto-managed Android SDK installed by `otto-simulators` at `<data_dir>/android-sdk`.
        if let Some(parent) = root.parent() {
            push_android_sdk_if_valid(&parent.join("android-sdk"), "otto", &mut out, &mut seen);
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
    if let Some(gr) = find_executable_in(&search, &["go"])
        .and_then(|b| bin_prefix(&b))
        .filter(|gr| is_goroot(gr))
    {
        let v = read_go_root_version(&gr).unwrap_or_else(|| "system".into());
        push_det(
            &mut out,
            &mut seen,
            "go",
            format!("Go {v}"),
            &v,
            &gr,
            "path",
        );
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
    if let Some(prefix) = find_executable_in(&search, &["node"]).and_then(|b| bin_prefix(&b)) {
        let ver = node_prefix_version(&prefix);
        push_det(
            &mut out,
            &mut seen,
            "node",
            format!("Node ({ver})"),
            &ver,
            &prefix,
            "system",
        );
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

    // 7. iOS / Xcode, Android SDK, and Flutter (zero-subprocess disk scan)
    scan_mobile_sdks(home, out, seen);
}

fn read_xcode_plist_version(app_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(app_dir.join("Contents/version.plist")).ok()?;
    let key_idx = raw.find("<key>CFBundleShortVersionString</key>")?;
    let after = &raw[key_idx..];
    let s_start = after.find("<string>")? + "<string>".len();
    let rest = &after[s_start..];
    let s_end = rest.find("</string>")?;
    let v = rest[..s_end].trim();
    (!v.is_empty()).then(|| v.to_string())
}

fn read_android_sdk_version(sdk_dir: &Path) -> String {
    let mut apis: Vec<u32> = read_subdirs(&sdk_dir.join("platforms"))
        .into_iter()
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?;
            let rest = name.strip_prefix("android-")?;
            rest.split('-').next()?.parse::<u32>().ok()
        })
        .collect();
    apis.sort_unstable();
    if let Some(max_api) = apis.last() {
        format!("API {max_api}")
    } else if sdk_dir.join("platform-tools/adb").is_file() {
        "platform-tools".into()
    } else {
        "sdk".into()
    }
}

fn push_android_sdk_if_valid(
    cand: &Path,
    src: &str,
    out: &mut Vec<IdeDetectedSdk>,
    seen: &mut BTreeSet<String>,
) {
    if cand.is_dir()
        && (cand.join("platform-tools").is_dir()
            || cand.join("cmdline-tools").is_dir()
            || cand.join("platforms").is_dir()
            || cand.join("emulator").is_dir())
    {
        let ver = read_android_sdk_version(cand);
        push_det(
            out,
            seen,
            "android",
            format!("Android SDK ({ver})"),
            &ver,
            cand,
            src,
        );
    }
}

fn read_flutter_version(flutter_root: &Path) -> String {
    std::fs::read_to_string(flutter_root.join("version"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "installed".into())
}

#[allow(clippy::disallowed_methods)] // synchronous scanner invoked inside spawn_blocking
pub(crate) fn scan_mobile_sdks(
    home: Option<&Path>,
    out: &mut Vec<IdeDetectedSdk>,
    seen: &mut BTreeSet<String>,
) {
    // 7a. Xcode / Apple Developer Tools
    if let Ok(rd) = std::fs::read_dir(Path::new("/Applications")) {
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.starts_with("Xcode") && name.ends_with(".app") && p.is_dir() {
                let dev_dir = p.join("Contents/Developer");
                if dev_dir.is_dir() {
                    let ver = read_xcode_plist_version(&p).unwrap_or_else(|| "installed".into());
                    let label = format!("{} ({ver})", name.trim_end_matches(".app"));
                    push_det(out, seen, "ios", label, &ver, &dev_dir, "xcode");
                }
            } else if name.starts_with("Xcode") && name.ends_with(".appdownload") {
                out.push(IdeDetectedSdk {
                    ecosystem: IdeEcosystemKind::Ios,
                    name: format!("{} (downloading…)", name.trim_end_matches(".appdownload")),
                    subtitle: format!("downloading · {}", p.to_string_lossy()),
                    version: "downloading".into(),
                    arch: std::env::consts::ARCH.to_string(),
                    home_path: p.to_string_lossy().into_owned(),
                    bin_path: p.to_string_lossy().into_owned(),
                    path: p.to_string_lossy().into_owned(),
                    source: "downloading".into(),
                    valid: false,
                });
            }
        }
    }
    if let Ok(dev) = std::env::var("DEVELOPER_DIR") {
        let p = PathBuf::from(dev.trim());
        if p.is_dir()
            && (p.join("usr/bin/xcodebuild").is_file() || p.join("usr/bin/swift").is_file())
        {
            let ver = p
                .parent()
                .and_then(|c| c.parent())
                .and_then(read_xcode_plist_version)
                .unwrap_or_else(|| "env".into());
            push_det(
                out,
                seen,
                "ios",
                format!("DEVELOPER_DIR ({ver})"),
                &ver,
                &p,
                "env",
            );
        }
    }
    let clt = PathBuf::from("/Library/Developer/CommandLineTools");
    if clt.join("usr/bin/swift").is_file() {
        push_det(
            out,
            seen,
            "ios",
            "Apple Command Line Tools".into(),
            "clt",
            &clt,
            "system",
        );
    }

    // 7b. Android SDK
    let mut android_cands: Vec<(PathBuf, &str)> = Vec::new();
    for env_key in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Ok(val) = std::env::var(env_key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                android_cands.push((PathBuf::from(trimmed), "env"));
            }
        }
    }
    if let Some(h) = home {
        android_cands.push((h.join("Library/Android/sdk"), "android-studio"));
        android_cands.push((h.join("Android/Sdk"), "android-studio"));
    }
    android_cands.push((
        PathBuf::from("/opt/homebrew/share/android-commandlinetools"),
        "homebrew",
    ));
    android_cands.push((
        PathBuf::from("/usr/local/share/android-commandlinetools"),
        "homebrew",
    ));
    for (cand, src) in android_cands {
        push_android_sdk_if_valid(&cand, src, out, seen);
    }

    // 7c. Flutter SDK
    let mut flutter_cands: Vec<(PathBuf, &str)> = Vec::new();
    if let Ok(val) = std::env::var("FLUTTER_ROOT") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            flutter_cands.push((PathBuf::from(trimmed), "env"));
        }
    }
    if let Some(h) = home {
        flutter_cands.push((h.join("development/flutter"), "user"));
        flutter_cands.push((h.join("flutter"), "user"));
        for dir in read_subdirs(&h.join(".fvm/versions")) {
            flutter_cands.push((dir, "fvm"));
        }
    }
    for cask_root in [
        "/opt/homebrew/Caskroom/flutter",
        "/usr/local/Caskroom/flutter",
    ] {
        for dir in read_subdirs(Path::new(cask_root)) {
            flutter_cands.push((dir.join("flutter"), "homebrew"));
        }
    }
    if let Some(fb) = find_executable_in(&system_search_dirs(), &["flutter"]) {
        if let Some(root) = bin_prefix(&fb) {
            flutter_cands.push((root, "path"));
        }
    }
    for (cand, src) in flutter_cands {
        if cand.join("bin/flutter").is_file() {
            let ver = read_flutter_version(&cand);
            push_det(
                out,
                seen,
                "flutter",
                format!("Flutter {ver}"),
                &ver,
                &cand,
                src,
            );
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
        "ios" => IdeEcosystemKind::Ios,
        "android" => IdeEcosystemKind::Android,
        "flutter" => IdeEcosystemKind::Flutter,
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
                IdeEcosystemKind::Ios => path.join("usr/bin/xcodebuild"),
                IdeEcosystemKind::Android => path.join("platform-tools/adb"),
                IdeEcosystemKind::Flutter => path.join("bin/flutter"),
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
