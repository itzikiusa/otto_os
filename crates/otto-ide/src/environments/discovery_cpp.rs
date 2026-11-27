//! Instant (`<5ms`) zero-subprocess Run/Debug Configuration auto-discovery
//! for C / C++ (CMake + Presets, Meson, Make, standalone C/C++) and Unreal Engine
//! (`.uproject` / `.uplugin`).

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::types::{IdeRunConfig, IdeRunnerKind};

use super::discovery::{make_auto_config, read_head};

pub(super) fn discover_cpp_and_unreal(
    root: &Path,
    proj_name: &str,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) {
    let found_unreal = discover_unreal(root, proj_name, root_badge, cwd, out);
    let found_cmake = discover_cmake(root, proj_name, root_badge, cwd, out);
    let found_meson = discover_meson(root, proj_name, root_badge, cwd, out);
    let found_make = if !found_cmake && !found_meson && !found_unreal {
        discover_make(root, root_badge, cwd, out)
    } else {
        false
    };
    if !found_cmake && !found_meson && !found_make && !found_unreal {
        discover_standalone_cpp(root, root_badge, cwd, out);
    }
}

// ── 1. Unreal Engine (`.uproject` / `.uplugin`) ──────────────────────────────

#[allow(clippy::disallowed_methods)]
pub(crate) fn find_unreal_descriptor(root: &Path) -> Option<(std::path::PathBuf, bool)> {
    let rd = std::fs::read_dir(root).ok()?;
    let mut plugin_fallback = None;
    for entry in rd.flatten().take(64) {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.ends_with(".uproject") {
            return Some((p, true));
        }
        if name.ends_with(".uplugin") && plugin_fallback.is_none() {
            plugin_fallback = Some((p, false));
        }
    }
    plugin_fallback
}

#[allow(clippy::disallowed_methods)]
fn discover_unreal_editor_target(root: &Path, stem: &str) -> String {
    let source_dir = root.join("Source");
    if let Ok(rd) = std::fs::read_dir(&source_dir) {
        for entry in rd.flatten().take(64) {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(target) = name.strip_suffix(".Target.cs") {
                    if target.ends_with("Editor") {
                        return target.to_string();
                    }
                }
            }
        }
    }
    format!("{stem}Editor")
}

fn unreal_host_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "Mac"
    } else {
        "Linux"
    }
}

fn discover_unreal(
    root: &Path,
    proj_name: &str,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) -> bool {
    let Some((desc_path, is_uproject)) = find_unreal_descriptor(root) else {
        return false;
    };
    let stem = desc_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(proj_name)
        .to_string();
    let desc_name = desc_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let desc_abs = desc_path.to_string_lossy().into_owned();
    let platform = unreal_host_platform();

    if is_uproject {
        let editor_target = discover_unreal_editor_target(root, &stem);
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Unreal,
            "build-editor",
            format!("Unreal Build {editor_target} (Development)"),
            None,
            vec![
                "build".into(),
                editor_target.clone(),
                platform.into(),
                "Development".into(),
                desc_abs.clone(),
            ],
            cwd.clone(),
            Some(desc_name.clone()),
        ));
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Unreal,
            "gen-cdb",
            format!("Unreal Generate compile_commands.json ({stem})"),
            None,
            vec![
                "gen-cdb".into(),
                editor_target,
                platform.into(),
                "Development".into(),
                desc_abs.clone(),
            ],
            cwd.clone(),
            Some(desc_name.clone()),
        ));
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Unreal,
            "launch-editor",
            format!("Unreal Launch Editor ({stem})"),
            None,
            vec!["editor".into(), desc_abs],
            cwd.clone(),
            Some(desc_name),
        ));
    } else {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Unreal,
            "build-plugin",
            format!("Unreal Build Plugin ({stem})"),
            None,
            vec!["build-plugin".into(), desc_abs],
            cwd.clone(),
            Some(desc_name),
        ));
    }
    true
}

// ── 2. CMake (`CMakePresets.json`, `CMakeUserPresets.json`, `CMakeLists.txt`) ─

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CmakePresetsSummary {
    pub configure: Vec<String>,
    pub build: Vec<String>,
    pub test: Vec<String>,
}

pub(crate) fn parse_cmake_presets(json_text: &str) -> CmakePresetsSummary {
    let mut summary = CmakePresetsSummary::default();
    let Ok(val) = serde_json::from_str::<Value>(json_text) else {
        return summary;
    };
    let collect = |key: &str, out: &mut Vec<String>| {
        let Some(arr) = val.get(key).and_then(Value::as_array) else {
            return;
        };
        for item in arr {
            if item.get("hidden").and_then(Value::as_bool).unwrap_or(false) {
                continue;
            }
            if let Some(name) = item
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if !out.iter().any(|existing| existing == name) {
                    out.push(name.to_string());
                }
            }
        }
    };
    collect("configurePresets", &mut summary.configure);
    collect("buildPresets", &mut summary.build);
    collect("testPresets", &mut summary.test);
    summary
}

pub(crate) fn parse_cmake_project_name(cmake_text: &str) -> Option<String> {
    static RE_PROJ: OnceLock<Regex> = OnceLock::new();
    let re = RE_PROJ
        .get_or_init(|| Regex::new(r"(?i)\bproject\s*\(\s*([A-Za-z_][A-Za-z0-9_.-]*)").unwrap());
    for line in cmake_text.lines() {
        let code = line.split('#').next().unwrap_or("");
        if let Some(cap) = re.captures(code) {
            return Some(cap[1].to_string());
        }
    }
    None
}

pub(crate) fn parse_cmake_executables(cmake_text: &str) -> Vec<String> {
    static RE_EXE: OnceLock<Regex> = OnceLock::new();
    let re = RE_EXE.get_or_init(|| {
        Regex::new(r"(?i)\badd_executable\s*\(\s*([A-Za-z_][A-Za-z0-9_.-]*)").unwrap()
    });
    let mut targets = Vec::new();
    for line in cmake_text.lines() {
        let code = line.split('#').next().unwrap_or("");
        if let Some(cap) = re.captures(code) {
            let name = cap[1].to_string();
            if !targets.contains(&name) {
                targets.push(name);
            }
        }
    }
    targets
}

pub(crate) fn cmake_has_tests(cmake_text: &str) -> bool {
    let lower = cmake_text.to_ascii_lowercase();
    lower.contains("enable_testing(")
        || lower.contains("add_test(")
        || lower.contains("gtest_discover_tests(")
        || lower.contains("catch_discover_tests(")
        || lower.contains("include(ctest)")
}

fn discover_cmake(
    root: &Path,
    proj_name: &str,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) -> bool {
    let cmake_lists = root.join("CMakeLists.txt");
    let presets_path = root.join("CMakePresets.json");
    let user_presets_path = root.join("CMakeUserPresets.json");
    let has_cmake = cmake_lists.is_file() || presets_path.is_file() || user_presets_path.is_file();
    if !has_cmake {
        return false;
    }

    let raw_cmake = read_head(&cmake_lists).unwrap_or_default();
    let name = parse_cmake_project_name(&raw_cmake).unwrap_or_else(|| proj_name.to_string());

    let mut presets = CmakePresetsSummary::default();
    for p in [&presets_path, &user_presets_path] {
        if let Some(txt) = read_head(p) {
            let parsed = parse_cmake_presets(&txt);
            for c in parsed.configure {
                if !presets.configure.contains(&c) {
                    presets.configure.push(c);
                }
            }
            for b in parsed.build {
                if !presets.build.contains(&b) {
                    presets.build.push(b);
                }
            }
            for t in parsed.test {
                if !presets.test.contains(&t) {
                    presets.test.push(t);
                }
            }
        }
    }

    for preset in presets.configure.iter().take(4) {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Cmake,
            &format!("cfg-preset-{preset}"),
            format!("CMake Configure ({preset})"),
            Some("cmake"),
            vec!["--preset".into(), preset.clone()],
            cwd.clone(),
            None,
        ));
    }
    for preset in presets.build.iter().take(4) {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Cmake,
            &format!("build-preset-{preset}"),
            format!("CMake Build ({preset})"),
            Some("cmake"),
            vec!["--build".into(), "--preset".into(), preset.clone()],
            cwd.clone(),
            None,
        ));
    }
    for preset in presets.test.iter().take(3) {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Ctest,
            &format!("test-preset-{preset}"),
            format!("CTest ({preset})"),
            Some("ctest"),
            vec![
                "--preset".into(),
                preset.clone(),
                "--output-on-failure".into(),
            ],
            cwd.clone(),
            None,
        ));
    }

    if presets.configure.is_empty() {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Cmake,
            "configure-debug",
            format!("CMake Configure Debug ({name})"),
            Some("cmake"),
            vec![
                "-S".into(),
                ".".into(),
                "-B".into(),
                "build".into(),
                "-DCMAKE_BUILD_TYPE=Debug".into(),
                "-DCMAKE_EXPORT_COMPILE_COMMANDS=ON".into(),
            ],
            cwd.clone(),
            None,
        ));
    }
    if presets.build.is_empty() {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Cmake,
            "build-default",
            format!("CMake Build ({name})"),
            Some("cmake"),
            vec!["--build".into(), "build".into()],
            cwd.clone(),
            None,
        ));
    }

    for exe in parse_cmake_executables(&raw_cmake).into_iter().take(4) {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::CppRun,
            &format!("run-{exe}"),
            format!("Run {exe}"),
            None,
            Vec::new(),
            cwd.clone(),
            Some(exe),
        ));
    }

    if presets.test.is_empty() && cmake_has_tests(&raw_cmake) {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Ctest,
            "ctest",
            format!("CTest ({name})"),
            Some("ctest"),
            vec![
                "--test-dir".into(),
                "build".into(),
                "--output-on-failure".into(),
            ],
            cwd.clone(),
            None,
        ));
    }

    true
}

// ── 3. Meson (`meson.build`) ─────────────────────────────────────────────────

fn discover_meson(
    root: &Path,
    proj_name: &str,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) -> bool {
    let meson_build = root.join("meson.build");
    let Some(raw) = read_head(&meson_build) else {
        return false;
    };
    static RE_PROJ: OnceLock<Regex> = OnceLock::new();
    let name = RE_PROJ
        .get_or_init(|| Regex::new(r"project\s*\(\s*'([^']+)'").unwrap())
        .captures(&raw)
        .map(|c| c[1].to_string())
        .unwrap_or_else(|| proj_name.to_string());

    out.push(make_auto_config(
        root_badge,
        IdeRunnerKind::Meson,
        "setup",
        format!("Meson Setup ({name})"),
        Some("meson"),
        vec![
            "setup".into(),
            "builddir".into(),
            "--buildtype=debug".into(),
        ],
        cwd.clone(),
        None,
    ));
    out.push(make_auto_config(
        root_badge,
        IdeRunnerKind::Meson,
        "compile",
        format!("Meson Compile ({name})"),
        Some("meson"),
        vec!["compile".into(), "-C".into(), "builddir".into()],
        cwd.clone(),
        None,
    ));
    if raw.contains("test(") {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Meson,
            "test",
            format!("Meson Test ({name})"),
            Some("meson"),
            vec![
                "test".into(),
                "-C".into(),
                "builddir".into(),
                "--print-errorlogs".into(),
            ],
            cwd.clone(),
            None,
        ));
    }
    true
}

// ── 4. Make (`Makefile` / `GNUmakefile`) ────────────────────────────────────

pub(crate) fn parse_makefile_targets(text: &str) -> Vec<String> {
    static RE_TGT: OnceLock<Regex> = OnceLock::new();
    let re = RE_TGT.get_or_init(|| Regex::new(r"^([A-Za-z_][A-Za-z0-9_.-]*)\s*:[^=]").unwrap());
    let mut targets = Vec::new();
    for line in text.lines() {
        if line.starts_with('\t') || line.starts_with(' ') || line.starts_with('#') {
            continue;
        }
        if let Some(cap) = re.captures(line) {
            let name = cap[1].to_string();
            if !name.starts_with('.') && !targets.contains(&name) {
                targets.push(name);
            }
        }
    }
    targets
}

fn discover_make(
    root: &Path,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) -> bool {
    let makefile = ["Makefile", "GNUmakefile", "makefile"]
        .into_iter()
        .map(|n| root.join(n))
        .find(|p| p.is_file());
    let Some(mf) = makefile else {
        return false;
    };
    // Only auto-emit Make configs when C/C++ sources exist at root or src/ so Go/Node
    // repos with convenience Makefiles don't get cluttered with duplicate Make entries.
    let has_c_sources = has_c_cpp_files(root) || has_c_cpp_files(&root.join("src"));
    if !has_c_sources {
        return false;
    }
    let raw = read_head(&mf).unwrap_or_default();
    let targets = parse_makefile_targets(&raw);
    let preferred = ["all", "build", "run", "test", "clean"];
    let mut emitted = 0usize;
    for pref in preferred {
        if targets.iter().any(|t| t == pref) {
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::Make,
                &format!("make-{pref}"),
                format!("Make {pref}"),
                Some("make"),
                vec![pref.into()],
                cwd.clone(),
                Some(pref.into()),
            ));
            emitted += 1;
        }
    }
    if emitted == 0 {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Make,
            "make-default",
            "Make".into(),
            Some("make"),
            Vec::new(),
            cwd.clone(),
            None,
        ));
    }
    true
}

#[allow(clippy::disallowed_methods)]
fn has_c_cpp_files(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    rd.flatten().take(64).any(|e| {
        e.path().is_file()
            && e.file_name().to_str().is_some_and(|n| {
                n.ends_with(".c")
                    || n.ends_with(".cpp")
                    || n.ends_with(".cc")
                    || n.ends_with(".cxx")
            })
    })
}

// ── 5. Standalone C / C++ entrypoints ────────────────────────────────────────

fn discover_standalone_cpp(
    root: &Path,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) {
    for rel in [
        "main.cpp",
        "src/main.cpp",
        "main.cc",
        "main.c",
        "src/main.c",
    ] {
        let p = root.join(rel);
        if p.is_file() {
            let label = rel.rsplit('/').next().unwrap_or(rel);
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::CppRun,
                &format!("run-{label}"),
                format!("Run {label}"),
                None,
                Vec::new(),
                cwd.clone(),
                Some(rel.into()),
            ));
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cmake_presets_targets_and_tests() {
        let presets = r#"{
            "version": 6,
            "configurePresets": [
                { "name": "base", "hidden": true },
                { "name": "debug", "inherits": "base" },
                { "name": "release", "inherits": "base" }
            ],
            "buildPresets": [
                { "name": "debug-build", "configurePreset": "debug" }
            ],
            "testPresets": [
                { "name": "debug-test", "configurePreset": "debug" }
            ]
        }"#;
        let p = parse_cmake_presets(presets);
        assert_eq!(p.configure, vec!["debug", "release"]);
        assert_eq!(p.build, vec!["debug-build"]);
        assert_eq!(p.test, vec!["debug-test"]);

        let cm = r#"
cmake_minimum_required(VERSION 3.20)
project(DemoEngine LANGUAGES CXX)
add_executable(demo_app src/main.cpp)
add_executable(demo_tests tests/math_test.cpp)
enable_testing()
add_test(NAME math COMMAND demo_tests)
"#;
        assert_eq!(parse_cmake_project_name(cm).as_deref(), Some("DemoEngine"));
        assert_eq!(parse_cmake_executables(cm), vec!["demo_app", "demo_tests"]);
        assert!(cmake_has_tests(cm));
    }

    #[test]
    fn discovers_unreal_uproject_meson_and_makefile_configs() {
        let tmp = tempfile::tempdir().unwrap();
        let ue = tmp.path().join("MyGame");
        std::fs::create_dir_all(ue.join("Source")).unwrap();
        std::fs::write(
            ue.join("MyGame.uproject"),
            r#"{"FileVersion": 3, "EngineAssociation": "5.4"}"#,
        )
        .unwrap();
        std::fs::write(ue.join("Source/MyGameEditor.Target.cs"), "").unwrap();

        let mut out = Vec::new();
        discover_cpp_and_unreal(
            &ue,
            "MyGame",
            None,
            &Some(ue.to_string_lossy().into()),
            &mut out,
        );
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|c| c.runner == IdeRunnerKind::Unreal));
        assert!(out.iter().any(|c| c.name.contains("MyGameEditor")));
        assert!(out.iter().any(|c| c.name.contains("compile_commands.json")));
        assert!(out.iter().any(|c| c.name.contains("Launch Editor")));

        let meson_dir = tmp.path().join("MesonDemo");
        std::fs::create_dir_all(&meson_dir).unwrap();
        std::fs::write(
            meson_dir.join("meson.build"),
            "project('demo', 'cpp')\nexe = executable('demo', 'main.cpp')\ntest('basic', exe)\n",
        )
        .unwrap();
        let mut mout = Vec::new();
        discover_cpp_and_unreal(
            &meson_dir,
            "MesonDemo",
            None,
            &Some(meson_dir.to_string_lossy().into()),
            &mut mout,
        );
        assert_eq!(mout.len(), 3);
        assert!(mout.iter().all(|c| c.runner == IdeRunnerKind::Meson));
    }
}
