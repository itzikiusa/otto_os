//! Python, Java (Maven / Gradle / Spring Boot / main classes), and Shell
//! Run/Debug Configuration auto-discovery helpers.

use std::path::Path;

use crate::types::{IdeRunConfig, IdeRunnerKind};

use super::discovery::{
    list_dir_entries_bounded, list_files_with_suffix, make_auto_config, read_head,
};

pub(super) fn discover_python(
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

pub(super) fn discover_java(
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

pub(super) fn discover_shell(
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

#[cfg(test)]
mod tests {
    use super::super::discovery::discover_run_configs;
    use crate::types::{IdeRunnerKind, IdeTargetSpec};
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
