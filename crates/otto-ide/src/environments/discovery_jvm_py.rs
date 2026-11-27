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
            vec!["runserver".into()],
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
                Vec::new(),
                cwd.clone(),
                Some(entry.into()),
            ));
        }
    }

    // ML training & evaluation scripts (stream unbuffered live logs via PYTHONUNBUFFERED).
    for train_rel in [
        "train.py",
        "finetune.py",
        "evaluate.py",
        "eval.py",
        "scripts/train.py",
        "src/train.py",
    ] {
        if root.join(train_rel).is_file() {
            let mut cfg = make_auto_config(
                root_badge,
                IdeRunnerKind::Python,
                &format!("ml-{}", train_rel.replace('/', "-")),
                format!("ML Train ({train_rel})"),
                Some("python3"),
                Vec::new(),
                cwd.clone(),
                Some(train_rel.into()),
            );
            cfg.env_overrides
                .insert("PYTHONUNBUFFERED".into(), "1".into());
            out.push(cfg);
        }
    }

    // TensorBoard run config when ML logging dirs or ML dependencies are present.
    let deps_text = read_head(&root.join("requirements.txt")).unwrap_or_default()
        + "\n"
        + &read_head(&root.join("pyproject.toml")).unwrap_or_default();
    let deps_low = deps_text.to_ascii_lowercase();
    let tb_logdir = ["runs", "tb_logs", "lightning_logs", "logs"]
        .into_iter()
        .find(|d| root.join(d).is_dir());
    let has_ml_deps = deps_low.contains("tensorboard")
        || deps_low.contains("torch")
        || deps_low.contains("tensorflow");
    if tb_logdir.is_some() || has_ml_deps {
        let logdir = tb_logdir.unwrap_or("runs");
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Python,
            "tensorboard",
            format!("TensorBoard (--logdir {logdir})"),
            Some("python3"),
            vec![
                "-m".into(),
                "tensorboard.main".into(),
                "--logdir".into(),
                logdir.into(),
            ],
            cwd.clone(),
            None,
        ));
    }

    // Jupyter Lab server config when notebooks exist in root or `notebooks/`.
    let has_notebooks = !list_files_with_suffix(root, ".ipynb").is_empty()
        || !list_files_with_suffix(&root.join("notebooks"), ".ipynb").is_empty();
    if has_notebooks {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Python,
            "jupyter-lab",
            "Jupyter Lab".into(),
            Some("python3"),
            vec!["-m".into(), "jupyterlab".into()],
            cwd.clone(),
            None,
        ));
    }

    // A `tests/` folder alone is no Python sign (Go and JS repos have one too).
    let has_py_tests_dir = ["tests", "test"]
        .iter()
        .any(|d| !list_files_with_suffix(&root.join(d), ".py").is_empty());
    let has_root_py_tests = list_files_with_suffix(root, ".py").iter().any(|p| {
        let n = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        n.starts_with("test_") || n.ends_with("_test.py")
    });

    if has_pytest_ini || has_py_tests_dir || has_root_py_tests || has_pyproject || has_reqs {
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

/// A pom is a runnable Spring Boot app only when it builds a jar/war (not a
/// `pom` aggregator) and applies `spring-boot-maven-plugin` itself; the boot
/// parent or a `<pluginManagement>` entry alone is just a library or a BOM.
fn pom_is_spring_boot_app(pom: &str) -> bool {
    use crate::java_workspace::{extract_tag_text, remove_all_tag_blocks, strip_xml_comments};
    let xml = remove_all_tag_blocks(&strip_xml_comments(pom), "pluginManagement");
    extract_tag_text(&xml, "packaging").as_deref() != Some("pom")
        && xml.contains("spring-boot-maven-plugin")
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
        if pom_is_spring_boot_app(&pom_head) {
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

    for src_sub in ["src/main/java", "src/main/kotlin"] {
        let src_dir = root.join(src_sub);
        if src_dir.is_dir() {
            let mut scanned = 0usize;
            let mut mains = Vec::new();
            scan_jvm_mains(&src_dir, root, 0, &mut scanned, &mut mains);
            for (fqcn, rel_path, is_spring_boot) in mains.into_iter().take(4) {
                let short = fqcn.rsplit('.').next().unwrap_or(&fqcn);
                let label = if is_spring_boot {
                    format!("Spring Boot: {short}")
                } else {
                    format!("Run {short}.main()")
                };
                let mut cfg = make_auto_config(
                    root_badge,
                    IdeRunnerKind::JavaMain,
                    &format!("main-{fqcn}"),
                    label,
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
}

fn scan_jvm_mains(
    dir: &Path,
    workspace_root: &Path,
    depth: usize,
    scanned: &mut usize,
    out: &mut Vec<(String, String, bool)>,
) {
    if depth > 6 || *scanned >= 48 || out.len() >= 4 {
        return;
    }
    for entry_path in list_dir_entries_bounded(dir) {
        if *scanned >= 48 || out.len() >= 4 {
            break;
        }
        if entry_path.is_dir() {
            scan_jvm_mains(&entry_path, workspace_root, depth + 1, scanned, out);
        } else if let Some(ext) = entry_path.extension().and_then(|s| s.to_str()) {
            if ext != "java" && ext != "kt" {
                continue;
            }
            *scanned += 1;
            if let Some(head) = read_head(&entry_path) {
                let is_java_main = ext == "java" && head.contains("static void main(");
                let is_kt_main =
                    ext == "kt" && (head.contains("fun main(") || head.contains("fun main ()"));
                if is_java_main || is_kt_main {
                    let is_spring_boot = head.contains("@SpringBootApplication")
                        || head.contains("SpringApplication.run(")
                        || head.contains("runApplication<");
                    let stem = entry_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Main");
                    let cls = if ext == "kt" {
                        crate::dap::java_bridge::infer_kotlin_class_stem(stem, &head)
                    } else {
                        stem.to_string()
                    };
                    let pkg = head.lines().find_map(|l| {
                        let t = l.trim();
                        t.strip_prefix("package ")
                            .map(|r| r.trim_end_matches(';').trim().to_string())
                    });
                    let fqcn = match pkg {
                        Some(p) if !p.is_empty() => format!("{p}.{cls}"),
                        _ => cls,
                    };
                    let rel = entry_path
                        .strip_prefix(workspace_root)
                        .unwrap_or(&entry_path)
                        .to_string_lossy()
                        .into_owned();
                    out.push((fqcn, rel, is_spring_boot));
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
        let args = Vec::new();
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
        assert!(detected.iter().any(|c| c.runner == IdeRunnerKind::GoRun
            && c.name.contains("[go_admission]")
            && c.name.contains("admission")
            && c.cwd.as_deref() == Some(go_root.to_string_lossy().as_ref())));
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

    #[test]
    fn discovers_per_repo_configs_for_umbrella_folder_with_child_git_repos() {
        let tmp = tempfile::tempdir().unwrap();
        let umbrella = tmp.path().join("workspace_umbrella");
        let svc_go = umbrella.join("svc_go");
        std::fs::create_dir_all(svc_go.join(".git")).unwrap();
        std::fs::write(
            svc_go.join("go.mod"),
            "module example.com/svc_go\n\ngo 1.24\n",
        )
        .unwrap();
        std::fs::write(svc_go.join("svc_test.go"), "package svc\n").unwrap();

        let svc_py = umbrella.join("svc_py");
        std::fs::create_dir_all(svc_py.join(".git")).unwrap();
        std::fs::create_dir_all(svc_py.join("tests")).unwrap();
        std::fs::write(
            svc_py.join("pyproject.toml"),
            "[project]\nname = \"svc_py\"\n",
        )
        .unwrap();
        std::fs::write(svc_py.join("tests/test_api.py"), "def test_ok(): pass\n").unwrap();

        let target = IdeTargetSpec::local(umbrella.to_string_lossy().into_owned());
        let detected = discover_run_configs(&target, &[]);
        let go_test = detected
            .iter()
            .find(|c| c.runner == IdeRunnerKind::GoTest && c.name.contains("[svc_go]"))
            .expect("expected per-repo [svc_go] Go Test config");
        assert_eq!(
            go_test.cwd.as_deref(),
            Some(svc_go.to_string_lossy().as_ref())
        );

        let py_test = detected
            .iter()
            .find(|c| c.runner == IdeRunnerKind::Pytest && c.name.contains("[svc_py]"))
            .expect("expected per-repo [svc_py] pytest config");
        assert_eq!(
            py_test.cwd.as_deref(),
            Some(svc_py.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn spring_boot_config_only_for_application_poms() {
        let parent =
            "<project><parent><artifactId>spring-boot-starter-parent</artifactId></parent>";
        let lib = format!("{parent}<dependencies><dependency><artifactId>spring-boot-starter</artifactId></dependency></dependencies></project>");
        let aggregator = format!("{parent}<packaging>pom</packaging><build><plugins><plugin><artifactId>spring-boot-maven-plugin</artifactId></plugin></plugins></build></project>");
        let managed = format!("{parent}<build><pluginManagement><plugins><plugin><artifactId>spring-boot-maven-plugin</artifactId></plugin></plugins></pluginManagement></build></project>");
        let commented = format!("{parent}<!-- <plugin><artifactId>spring-boot-maven-plugin</artifactId></plugin> --></project>");
        let app = format!("{parent}<build><plugins><plugin><groupId>org.springframework.boot</groupId><artifactId>spring-boot-maven-plugin</artifactId></plugin></plugins></build></project>");
        let tmp = tempfile::tempdir().unwrap();
        let boot_configs = |pom: &str| {
            std::fs::write(tmp.path().join("pom.xml"), pom).unwrap();
            let target = IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned());
            discover_run_configs(&target, &[])
                .into_iter()
                .filter(|c| c.name.starts_with("Spring Boot"))
                .count()
        };
        for none in [&lib, &aggregator, &managed, &commented] {
            assert_eq!(boot_configs(none), 0, "{none}");
        }
        assert_eq!(boot_configs(&app), 1);
    }
}
