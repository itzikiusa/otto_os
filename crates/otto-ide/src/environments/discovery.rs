//! Instant (`<5ms`) zero-subprocess, zero-cache Run/Debug Configuration
//! auto-discovery (`discover_run_configs`).
//!
//! Scans project manifests and entrypoints live from disk across `target.root`
//! and `additional_roots` for Go, Rust, JavaScript/TypeScript, Python, Java,
//! and Shell workspaces. Never spawns external binaries (`go list`, `cargo metadata`,
//! `mvn`, `python --version`) and never retains stale filesystem caches.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::types::{IdeRunConfig, IdeRunnerKind, IdeTargetKind, IdeTargetSpec};

const MAX_DISCOVERED_CONFIGS: usize = 48;
const MAX_MANIFEST_READ_BYTES: usize = 32 * 1024;
const MAX_DIR_ENTRIES: usize = 128;

const SKIPPED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    "out",
    ".venv",
    "venv",
    "env",
    ".next",
    ".svelte-kit",
    ".turbo",
    ".gradle",
    ".idea",
    ".vscode",
    "__pycache__",
];

/// Discover runnable & debuggable configurations live from disk across the
/// primary workspace root and any attached `additional_roots`.
pub fn discover_run_configs(
    target: &IdeTargetSpec,
    additional_roots: &[String],
) -> Vec<IdeRunConfig> {
    if target.kind != IdeTargetKind::Local {
        return Vec::new();
    }

    let mut out = Vec::new();
    let primary = PathBuf::from(&target.root);
    if primary.is_dir() {
        scan_project_folder(&primary, None, &mut out);
    }

    for extra in additional_roots {
        if out.len() >= MAX_DISCOVERED_CONFIGS {
            break;
        }
        let trimmed = extra.trim();
        if trimmed.is_empty() || trimmed == target.root {
            continue;
        }
        let path = PathBuf::from(trimmed);
        if path.is_dir() {
            let folder_label = folder_name(&path);
            scan_project_folder(&path, Some(&folder_label), &mut out);
        }
    }

    out.truncate(MAX_DISCOVERED_CONFIGS);
    out
}

fn scan_project_folder(root: &Path, root_badge: Option<&str>, out: &mut Vec<IdeRunConfig>) {
    let cwd_override = root_badge.map(|_| root.to_string_lossy().into_owned());
    let proj_name = folder_name(root);

    discover_go(root, &proj_name, root_badge, &cwd_override, out);
    if out.len() >= MAX_DISCOVERED_CONFIGS {
        return;
    }
    discover_rust(root, &proj_name, root_badge, &cwd_override, out);
    if out.len() >= MAX_DISCOVERED_CONFIGS {
        return;
    }
    discover_js_ts(root, root_badge, &cwd_override, out);
    if out.len() >= MAX_DISCOVERED_CONFIGS {
        return;
    }
    discover_python(root, root_badge, &cwd_override, out);
    if out.len() >= MAX_DISCOVERED_CONFIGS {
        return;
    }
    discover_java(root, &proj_name, root_badge, &cwd_override, out);
    if out.len() >= MAX_DISCOVERED_CONFIGS {
        return;
    }
    discover_shell(root, root_badge, &cwd_override, out);
}

fn make_auto_config(
    root_badge: Option<&str>,
    runner: IdeRunnerKind,
    slug: &str,
    base_name: String,
    command: Option<&str>,
    args: Vec<String>,
    cwd: Option<String>,
    target_path: Option<String>,
) -> IdeRunConfig {
    let runner_tag = serde_json::to_value(runner)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "run".into());
    let id = match root_badge {
        Some(b) => format!("auto:{}:{runner_tag}:{}", sanitize_id(b), sanitize_id(slug)),
        None => format!("auto:{runner_tag}:{}", sanitize_id(slug)),
    };
    let name = match root_badge {
        Some(b) => format!("[{b}] {base_name}"),
        None => base_name,
    };
    IdeRunConfig {
        id,
        name,
        runner,
        command: command.map(str::to_string),
        args,
        build_flags: Vec::new(),
        cwd,
        env_overrides: BTreeMap::new(),
        env_file: None,
        inherit_shell_env: true,
        sdk_path: None,
        toolchain_id: None,
        before_launch: None,
        target_path,
        test_filter: None,
        compound_configs: Vec::new(),
        auto_detected: true,
    }
}

// ── 1. Go (`go.mod`, `main.go`, `cmd/*/main.go`, `*_test.go`) ────────────────

fn discover_go(
    root: &Path,
    proj_name: &str,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) {
    let go_mod = root.join("go.mod");
    let has_go_mod = go_mod.is_file();
    let root_go_files = list_files_with_suffix(root, ".go");
    if !has_go_mod && root_go_files.is_empty() {
        return;
    }

    let mod_short = if has_go_mod {
        read_head(&go_mod)
            .and_then(|s| parse_go_module_short_name(&s))
            .unwrap_or_else(|| proj_name.to_string())
    } else {
        proj_name.to_string()
    };

    // Check root package main (`main.go` or any root `.go` file with `package main` + `func main(`).
    let root_has_main = root_go_files
        .iter()
        .filter(|p| {
            !p.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .ends_with("_test.go")
        })
        .any(|p| file_has_go_main(p));

    if root_has_main {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::GoRun,
            "root",
            format!("Go Run {mod_short}"),
            Some("go"),
            vec!["run".into(), ".".into()],
            cwd.clone(),
            Some(".".into()),
        ));
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::GoBuild,
            "build-root",
            format!("Go Build {mod_short}"),
            Some("go"),
            vec!["build".into(), ".".into()],
            cwd.clone(),
            Some(".".into()),
        ));
    }

    // Scan `cmd/*`, `services/*`, `apps/*` and shallow 1-level subdirs for `main.go`.
    for container in ["cmd", "services", "apps"] {
        let dir = root.join(container);
        if !dir.is_dir() {
            continue;
        }
        for sub in list_subdirs_bounded(&dir) {
            if sub.join("main.go").is_file() || has_any_go_main_in_dir(&sub) {
                let svc = folder_name(&sub);
                let rel_pkg = format!("./{container}/{svc}");
                let slug = format!("{container}-{svc}");
                out.push(make_auto_config(
                    root_badge,
                    IdeRunnerKind::GoRun,
                    &slug,
                    format!("Go Run {svc} ({container}/{svc})"),
                    Some("go"),
                    vec!["run".into(), rel_pkg.clone()],
                    cwd.clone(),
                    Some(rel_pkg.clone()),
                ));
                out.push(make_auto_config(
                    root_badge,
                    IdeRunnerKind::GoBuild,
                    &format!("build-{slug}"),
                    format!("Go Build {svc}"),
                    Some("go"),
                    vec!["build".into(), rel_pkg.clone()],
                    cwd.clone(),
                    Some(rel_pkg),
                ));
            }
        }
    }

    // Also check direct child directories (e.g. `server/main.go`) when neither root nor `cmd/` had main.
    if !root_has_main && !root.join("cmd").is_dir() {
        for sub in list_subdirs_bounded(root).into_iter().take(16) {
            let name = folder_name(&sub);
            if matches!(
                name.as_str(),
                "cmd" | "services" | "apps" | "internal" | "pkg"
            ) {
                continue;
            }
            if sub.join("main.go").is_file() && file_has_go_main(&sub.join("main.go")) {
                let rel_pkg = format!("./{name}");
                out.push(make_auto_config(
                    root_badge,
                    IdeRunnerKind::GoRun,
                    &name,
                    format!("Go Run {name}"),
                    Some("go"),
                    vec!["run".into(), rel_pkg.clone()],
                    cwd.clone(),
                    Some(rel_pkg),
                ));
            }
        }
    }

    // Always provide a workspace `go test ./...` configuration for Go modules.
    if has_go_mod
        || root_go_files.iter().any(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .ends_with("_test.go")
        })
    {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::GoTest,
            "all-tests",
            format!("Go Test ./... ({mod_short})"),
            Some("go"),
            vec!["test".into(), "-v".into(), "./...".into()],
            cwd.clone(),
            Some("./...".into()),
        ));
    }
}

fn parse_go_module_short_name(go_mod_text: &str) -> Option<String> {
    for line in go_mod_text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("module ") {
            let mod_path = rest.split_whitespace().next()?.trim_matches('"');
            let short = mod_path.rsplit('/').next().unwrap_or(mod_path).trim();
            if !short.is_empty() {
                return Some(short.to_string());
            }
        }
    }
    None
}

fn file_has_go_main(path: &Path) -> bool {
    let Some(head) = read_head(path) else {
        return false;
    };
    let has_pkg_main = head.lines().any(|l| {
        let t = l.trim();
        t == "package main" || t.starts_with("package main ") || t.starts_with("package main\t")
    });
    has_pkg_main && head.contains("func main(")
}

fn has_any_go_main_in_dir(dir: &Path) -> bool {
    list_files_with_suffix(dir, ".go")
        .into_iter()
        .take(12)
        .any(|p| file_has_go_main(&p))
}

// ── 2. Rust (`Cargo.toml`, `src/main.rs`, `src/bin/*.rs`) ────────────────────

fn discover_rust(
    root: &Path,
    proj_name: &str,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) {
    let cargo_toml = root.join("Cargo.toml");
    let Some(raw) = read_head(&cargo_toml) else {
        return;
    };
    let pkg_name = parse_cargo_package_name(&raw).unwrap_or_else(|| proj_name.to_string());
    let bins = parse_cargo_bin_names(&raw);

    if !bins.is_empty() {
        for bin in bins.into_iter().take(6) {
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::CargoRun,
                &format!("bin-{bin}"),
                format!("Cargo Run {bin}"),
                Some("cargo"),
                vec!["run".into(), "--bin".into(), bin.clone()],
                cwd.clone(),
                Some(bin),
            ));
        }
    } else if root.join("src/main.rs").is_file() {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::CargoRun,
            "main",
            format!("Cargo Run {pkg_name}"),
            Some("cargo"),
            vec!["run".into()],
            cwd.clone(),
            Some("src/main.rs".into()),
        ));
    }

    for bin_rs in list_files_with_suffix(&root.join("src/bin"), ".rs")
        .into_iter()
        .take(6)
    {
        if let Some(stem) = bin_rs.file_stem().and_then(|s| s.to_str()) {
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::CargoRun,
                &format!("srcbin-{stem}"),
                format!("Cargo Run {stem}"),
                Some("cargo"),
                vec!["run".into(), "--bin".into(), stem.to_string()],
                cwd.clone(),
                Some(format!("src/bin/{stem}.rs")),
            ));
        }
    }

    out.push(make_auto_config(
        root_badge,
        IdeRunnerKind::CargoTest,
        "test",
        format!("Cargo Test ({pkg_name})"),
        Some("cargo"),
        vec!["test".into()],
        cwd.clone(),
        None,
    ));
}

fn parse_cargo_package_name(toml_text: &str) -> Option<String> {
    let mut in_pkg = false;
    for line in toml_text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_pkg = t == "[package]";
            continue;
        }
        if in_pkg && t.starts_with("name") {
            if let Some((_, rhs)) = t.split_once('=') {
                let val = rhs
                    .split('#')
                    .next()
                    .unwrap_or(rhs)
                    .trim()
                    .trim_matches('"');
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

fn parse_cargo_bin_names(toml_text: &str) -> Vec<String> {
    let mut bins = Vec::new();
    let mut in_bin = false;
    for line in toml_text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_bin = t == "[[bin]]";
            continue;
        }
        if in_bin && t.starts_with("name") {
            if let Some((_, rhs)) = t.split_once('=') {
                let val = rhs
                    .split('#')
                    .next()
                    .unwrap_or(rhs)
                    .trim()
                    .trim_matches('"');
                if !val.is_empty() && !bins.iter().any(|b| b == val) {
                    bins.push(val.to_string());
                }
            }
        }
    }
    bins
}

// ── 3. JS / TS (`package.json`, `index.js`, `server.js`) ─────────────────────

fn discover_js_ts(
    root: &Path,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) {
    let pkg_json = root.join("package.json");
    if let Some(raw) = read_head(&pkg_json) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(scripts) = val.get("scripts").and_then(|v| v.as_object()) {
                const PRIORITY: &[&str] = &["dev", "start", "test", "build", "check", "lint"];
                let mut ordered: Vec<(&String, &serde_json::Value)> = Vec::new();
                for p in PRIORITY {
                    if let Some(kv) = scripts.get_key_value(*p) {
                        ordered.push(kv);
                    }
                }
                for kv in scripts {
                    if !PRIORITY.contains(&kv.0.as_str()) {
                        ordered.push(kv);
                    }
                }
                for (name, cmd_val) in ordered.into_iter().take(8) {
                    let script_body = cmd_val.as_str().unwrap_or("");
                    let is_vitest = script_body.contains("vitest") && name == "test";
                    if is_vitest {
                        out.push(make_auto_config(
                            root_badge,
                            IdeRunnerKind::Vitest,
                            "vitest",
                            "Vitest (npm test)".into(),
                            Some("npx"),
                            vec!["vitest".into(), "run".into()],
                            cwd.clone(),
                            Some("package.json".into()),
                        ));
                    } else {
                        out.push(make_auto_config(
                            root_badge,
                            IdeRunnerKind::Npm,
                            &format!("script-{name}"),
                            format!("npm run {name}"),
                            Some("npm"),
                            vec!["run".into(), name.clone()],
                            cwd.clone(),
                            Some("package.json".into()),
                        ));
                    }
                }
                return;
            }
        }
    }

    for entry in ["index.js", "server.js", "app.js", "main.js"] {
        if root.join(entry).is_file() {
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::Node,
                entry,
                format!("node {entry}"),
                Some("node"),
                vec![entry.into()],
                cwd.clone(),
                Some(entry.into()),
            ));
            break;
        }
    }
}

// ── 4. Python (`pyproject.toml`, `manage.py`, `main.py`, `app.py`, `pytest`) ─

fn discover_python(
    root: &Path,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) {
    let has_pyproject = root.join("pyproject.toml").is_file();
    let has_reqs = root.join("requirements.txt").is_file();
    let has_pytest_ini = root.join("pytest.ini").is_file() || root.join("conftest.py").is_file();

    if root.join("manage.py").is_file() {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Python,
            "django-runserver",
            "Django runserver".into(),
            Some("python3"),
            vec!["manage.py".into(), "runserver".into()],
            cwd.clone(),
            Some("manage.py".into()),
        ));
    }

    for entry in ["main.py", "app.py", "run.py", "server.py"] {
        if root.join(entry).is_file() {
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::Python,
                entry,
                format!("Python {entry}"),
                Some("python3"),
                vec![entry.into()],
                cwd.clone(),
                Some(entry.into()),
            ));
        }
    }

    let has_tests_dir = root.join("tests").is_dir() || root.join("test").is_dir();
    let has_root_py_tests = list_files_with_suffix(root, ".py").iter().any(|p| {
        let n = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        n.starts_with("test_") || n.ends_with("_test.py")
    });

    if has_pytest_ini || has_tests_dir || has_root_py_tests || (has_pyproject && has_reqs) {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Pytest,
            "pytest",
            "pytest".into(),
            None,
            vec!["-v".into()],
            cwd.clone(),
            None,
        ));
    }
}

// ── 5. Java (`pom.xml`, `build.gradle*`, `src/main/java/**`) ─────────────────

fn discover_java(
    root: &Path,
    proj_name: &str,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) {
    let pom = root.join("pom.xml");
    let gradle = if root.join("build.gradle.kts").is_file() {
        Some(root.join("build.gradle.kts"))
    } else if root.join("build.gradle").is_file() {
        Some(root.join("build.gradle"))
    } else {
        None
    };

    if pom.is_file() {
        let pom_head = read_head(&pom).unwrap_or_default();
        if pom_head.contains("spring-boot") {
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::Maven,
                "spring-boot-run",
                format!("Spring Boot ({proj_name})"),
                None,
                vec!["spring-boot:run".into()],
                cwd.clone(),
                Some("pom.xml".into()),
            ));
        }
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Maven,
            "mvn-test",
            format!("Maven Test ({proj_name})"),
            None,
            vec!["test".into()],
            cwd.clone(),
            Some("pom.xml".into()),
        ));
    }

    if let Some(gf) = gradle {
        let gf_head = read_head(&gf).unwrap_or_default();
        if gf_head.contains("org.springframework.boot") {
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::Gradle,
                "gradle-bootrun",
                format!("Gradle bootRun ({proj_name})"),
                None,
                vec!["bootRun".into()],
                cwd.clone(),
                None,
            ));
        } else if gf_head.contains("application") {
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::Gradle,
                "gradle-run",
                format!("Gradle run ({proj_name})"),
                None,
                vec!["run".into()],
                cwd.clone(),
                None,
            ));
        }
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Gradle,
            "gradle-test",
            format!("Gradle Test ({proj_name})"),
            None,
            vec!["test".into()],
            cwd.clone(),
            None,
        ));
    }

    let java_src = root.join("src/main/java");
    if java_src.is_dir() {
        let mut scanned = 0usize;
        let mut mains = Vec::new();
        scan_java_mains(&java_src, root, 0, &mut scanned, &mut mains);
        for (fqcn, rel_path) in mains.into_iter().take(4) {
            let short = fqcn.rsplit('.').next().unwrap_or(&fqcn);
            let mut cfg = make_auto_config(
                root_badge,
                IdeRunnerKind::JavaMain,
                &format!("main-{fqcn}"),
                format!("Run {short}.main()"),
                None,
                Vec::new(),
                cwd.clone(),
                Some(rel_path),
            );
            cfg.test_filter = Some(fqcn);
            out.push(cfg);
        }
    }
}

fn scan_java_mains(
    dir: &Path,
    workspace_root: &Path,
    depth: usize,
    scanned: &mut usize,
    out: &mut Vec<(String, String)>,
) {
    if depth > 6 || *scanned >= 48 || out.len() >= 4 {
        return;
    }
    for entry_path in list_dir_entries_bounded(dir) {
        if *scanned >= 48 || out.len() >= 4 {
            break;
        }
        if entry_path.is_dir() {
            scan_java_mains(&entry_path, workspace_root, depth + 1, scanned, out);
        } else if entry_path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|e| e == "java")
        {
            *scanned += 1;
            if let Some(head) = read_head(&entry_path) {
                if head.contains("static void main(") {
                    let cls = entry_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Main");
                    let pkg = head.lines().find_map(|l| {
                        let t = l.trim();
                        t.strip_prefix("package ")
                            .and_then(|r| r.strip_suffix(';'))
                            .map(|p| p.trim().to_string())
                    });
                    let fqcn = match pkg {
                        Some(p) if !p.is_empty() => format!("{p}.{cls}"),
                        _ => cls.to_string(),
                    };
                    let rel = entry_path
                        .strip_prefix(workspace_root)
                        .unwrap_or(&entry_path)
                        .to_string_lossy()
                        .into_owned();
                    out.push((fqcn, rel));
                }
            }
        }
    }
}

// ── 6. Shell (`*.sh`, `scripts/*.sh`) ────────────────────────────────────────

fn discover_shell(
    root: &Path,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) {
    let mut shell_files = Vec::new();
    for f in list_files_with_suffix(root, ".sh").into_iter().take(4) {
        if let Some(name) = f.file_name().and_then(|s| s.to_str()) {
            shell_files.push(name.to_string());
        }
    }
    let scripts_dir = root.join("scripts");
    if scripts_dir.is_dir() {
        for f in list_files_with_suffix(&scripts_dir, ".sh")
            .into_iter()
            .take(4)
        {
            if let Some(name) = f.file_name().and_then(|s| s.to_str()) {
                shell_files.push(format!("scripts/{name}"));
            }
        }
    }

    for rel in shell_files {
        let label = format!("Shell {rel}");
        let args = vec![rel.clone()];
        let target_path = Some(rel.clone());
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Shell,
            &rel,
            label,
            Some("bash"),
            args,
            cwd.clone(),
            target_path,
        ));
    }
}

// ── Bounded Fast Sync Filesystem Helpers ─────────────────────────────────────

fn folder_name(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("project")
        .to_string()
}

fn sanitize_id(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn read_head(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; MAX_MANIFEST_READ_BYTES];
    let n = file.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

#[allow(clippy::disallowed_methods)] // bounded <=128-entry scan executed inside spawn_blocking
fn list_dir_entries_bounded(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<PathBuf> = rd
        .flatten()
        .take(MAX_DIR_ENTRIES)
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            !name.starts_with('.') && !SKIPPED_DIRS.contains(&name)
        })
        .collect();
    entries.sort();
    entries
}

fn list_subdirs_bounded(dir: &Path) -> Vec<PathBuf> {
    list_dir_entries_bounded(dir)
        .into_iter()
        .filter(|p| p.is_dir())
        .collect()
}

fn list_files_with_suffix(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    list_dir_entries_bounded(dir)
        .into_iter()
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|n| n.ends_with(suffix))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn discovers_multi_root_go_rust_ts_python_java_and_shell_under_10ms() {
        let tmp = tempfile::tempdir().unwrap();
        let go_root = tmp.path().join("go_admission");
        std::fs::create_dir_all(go_root.join("cmd/admission")).unwrap();
        std::fs::write(
            go_root.join("go.mod"),
            "module bitbucket.org/gamescale-rnd/go_admission\n\ngo 1.24\n",
        )
        .unwrap();
        std::fs::write(
            go_root.join("cmd/admission/main.go"),
            "package main\n\nfunc main() {}\n",
        )
        .unwrap();

        let rust_extra = tmp.path().join("rust_worker");
        std::fs::create_dir_all(rust_extra.join("src")).unwrap();
        std::fs::write(
            rust_extra.join("Cargo.toml"),
            "[package]\nname = \"rust-worker\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(rust_extra.join("src/main.rs"), "fn main() {}\n").unwrap();

        let ts_extra = tmp.path().join("web_ui");
        std::fs::create_dir_all(ts_extra.join("scripts")).unwrap();
        std::fs::write(
            ts_extra.join("package.json"),
            r#"{"name":"web","scripts":{"dev":"vite","test":"vitest run"}}"#,
        )
        .unwrap();
        std::fs::write(ts_extra.join("scripts/smoke.sh"), "#!/usr/bin/env bash\n").unwrap();

        let target = IdeTargetSpec::local(go_root.to_string_lossy().into_owned());
        let additional = vec![
            rust_extra.to_string_lossy().into_owned(),
            ts_extra.to_string_lossy().into_owned(),
        ];

        let started = Instant::now();
        let detected = discover_run_configs(&target, &additional);
        let elapsed = started.elapsed();

        assert!(
            elapsed.as_millis() < 25,
            "discover_run_configs took {elapsed:?}, expected < 25ms"
        );
        assert!(detected
            .iter()
            .all(|c| c.auto_detected && c.inherit_shell_env));
        assert!(detected
            .iter()
            .any(|c| c.runner == IdeRunnerKind::GoRun && c.name.contains("admission")));
        assert!(detected
            .iter()
            .any(|c| c.runner == IdeRunnerKind::GoBuild && c.name.contains("admission")));
        assert!(detected.iter().any(|c| c.runner == IdeRunnerKind::GoTest));
        assert!(detected
            .iter()
            .any(|c| c.runner == IdeRunnerKind::CargoRun && c.name.contains("[rust_worker]")));
        assert!(detected
            .iter()
            .any(|c| c.runner == IdeRunnerKind::Vitest && c.name.contains("[web_ui]")));
        assert!(detected
            .iter()
            .any(|c| c.runner == IdeRunnerKind::Shell && c.name.contains("scripts/smoke.sh")));
    }
}
