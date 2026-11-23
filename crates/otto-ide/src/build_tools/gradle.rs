//! Gradle build-script, Spring Boot profile, and display-path scanners for
//! [`super::build_tools_model`].

#![allow(clippy::disallowed_methods)] // synchronous scanner invoked inside spawn_blocking

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use crate::types::{IdeBuildToolProfile, IdeBuildToolProject};

use super::{confinable_project_dir, BuildScanState};

const GRADLE_LIFECYCLE: &[&str] = &[
    "clean",
    "assemble",
    "compileJava",
    "test",
    "check",
    "build",
    "jar",
    "bootRun",
];

pub(super) fn scan_gradle_tree(
    dir: &Path,
    depth: usize,
    st: &mut BuildScanState<'_>,
) -> Option<IdeBuildToolProject> {
    if depth > 3 || !st.visited.insert(dir.to_path_buf()) {
        return None;
    }
    let manifest = if dir.join("build.gradle.kts").is_file() {
        dir.join("build.gradle.kts")
    } else {
        dir.join("build.gradle")
    };
    if fs::symlink_metadata(&manifest)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(true)
    {
        return None;
    }
    let raw = fs::read_to_string(&manifest).ok()?;
    let artifact_id = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("gradle-app")
        .to_string();

    static RE_GROUP: OnceLock<Regex> = OnceLock::new();
    static RE_VER: OnceLock<Regex> = OnceLock::new();
    let group_re =
        RE_GROUP.get_or_init(|| Regex::new(r#"(?m)^\s*group\s*=?\s*['"]([^'"]+)['"]"#).unwrap());
    let ver_re =
        RE_VER.get_or_init(|| Regex::new(r#"(?m)^\s*version\s*=?\s*['"]([^'"]+)['"]"#).unwrap());
    let group_id = group_re
        .captures(&raw)
        .map(|c| c[1].trim().to_string())
        .unwrap_or_default();
    let version = ver_re
        .captures(&raw)
        .map(|c| c[1].trim().to_string())
        .unwrap_or_else(|| "0.0.0".into());

    scan_spring_profiles(
        dir,
        &artifact_id,
        st.canon_roots,
        st.profiles,
        st.seen_profiles,
    );

    static RE_PROP_PROF: OnceLock<Regex> = OnceLock::new();
    let prop_prof_re =
        RE_PROP_PROF.get_or_init(|| Regex::new(r#"profile-([A-Za-z0-9_-]+)\.gradle"#).unwrap());
    for cap in prop_prof_re.captures_iter(&raw) {
        let pid = cap[1].to_string();
        if st.seen_profiles.insert(("gradle".into(), pid.clone())) {
            st.profiles.push(IdeBuildToolProfile {
                id: pid,
                kind: "gradle".into(),
                source_project: artifact_id.clone(),
                active_by_default: false,
            });
        }
    }

    let mut plugins = Vec::new();
    static RE_G_PLUG: OnceLock<Regex> = OnceLock::new();
    let plug_re = RE_G_PLUG.get_or_init(|| Regex::new(r#"id\s*\(?\s*['"]([^'"]+)['"]"#).unwrap());
    for cap in plug_re.captures_iter(&raw) {
        let p = cap[1].to_string();
        if !plugins.contains(&p) {
            plugins.push(p);
        }
    }

    let mut dependencies = Vec::new();
    static RE_DEP: OnceLock<Regex> = OnceLock::new();
    let dep_re = RE_DEP.get_or_init(|| {
        Regex::new(
            r#"(?:implementation|api|compileOnly|runtimeOnly|testImplementation)\s*\(?\s*['"]([^'"]+)['"]"#,
        )
        .unwrap()
    });
    for cap in dep_re.captures_iter(&raw) {
        let d = cap[1].to_string();
        if !dependencies.contains(&d) {
            dependencies.push(d);
        }
    }

    let mut repositories = Vec::new();
    for known in [
        "mavenCentral()",
        "google()",
        "gradlePluginPortal()",
        "mavenLocal()",
    ] {
        if raw.contains(known) {
            repositories.push(known.trim_end_matches("()").to_string());
        }
    }

    let mut child_modules = Vec::new();
    static RE_INC: OnceLock<Regex> = OnceLock::new();
    static RE_Q: OnceLock<Regex> = OnceLock::new();
    let inc_re = RE_INC.get_or_init(|| Regex::new(r#"include\s*\(?([^)\n]+)\)?"#).unwrap());
    let q_re = RE_Q.get_or_init(|| Regex::new(r#"['"]([^'"]+)['"]"#).unwrap());
    for settings_name in ["settings.gradle", "settings.gradle.kts"] {
        let settings_path = dir.join(settings_name);
        if fs::symlink_metadata(&settings_path)
            .map(|m| !m.file_type().is_symlink())
            .unwrap_or(false)
        {
            if let Ok(sraw) = fs::read_to_string(&settings_path) {
                for cap in inc_re.captures_iter(&sraw) {
                    for q in q_re.captures_iter(&cap[1]) {
                        let rel = q[1].replace(':', "/").trim_start_matches('/').to_string();
                        let Some(cdir) = confinable_project_dir(
                            &dir.join(&rel),
                            st.canon_roots,
                            st.is_root_user,
                        ) else {
                            continue;
                        };
                        if let Some(cproj) = scan_gradle_tree(&cdir, depth + 1, st) {
                            child_modules.push(cproj);
                        }
                    }
                }
            }
        }
    }

    let project_coord = if group_id.is_empty() {
        artifact_id.clone()
    } else {
        format!("{group_id}:{artifact_id}")
    };

    let lifecycle = if super::mobile::is_android_gradle_content(&raw)
        || dir.join("src/main/AndroidManifest.xml").is_file()
    {
        vec![
            "clean".into(),
            "assembleDebug".into(),
            "installDebug".into(),
            "testDebugUnitTest".into(),
            "connectedDebugAndroidTest".into(),
            "lintDebug".into(),
            "assembleRelease".into(),
        ]
    } else {
        GRADLE_LIFECYCLE.iter().map(|s| (*s).to_string()).collect()
    };

    Some(IdeBuildToolProject {
        name: artifact_id.clone(),
        group_id,
        artifact_id,
        version,
        tool: "gradle".into(),
        project_coord,
        root_dir: dir.to_string_lossy().into_owned(),
        location_display: shorten_home_path(dir, st.home),
        manifest_path: manifest.to_string_lossy().into_owned(),
        lifecycle,
        plugins,
        dependencies,
        repositories,
        modules: child_modules,
    })
}

pub(super) fn scan_spring_profiles(
    dir: &Path,
    project_name: &str,
    canon_roots: &[PathBuf],
    profiles: &mut Vec<IdeBuildToolProfile>,
    seen: &mut HashSet<(String, String)>,
) {
    for rel in ["src/main/resources", "src/test/resources", "config"] {
        let res_dir = dir.join(rel);
        let Ok(canon_res) = fs::canonicalize(&res_dir) else {
            continue;
        };
        if !canon_roots.iter().any(|r| canon_res.starts_with(r)) {
            continue;
        }
        let Ok(rd) = fs::read_dir(&canon_res) else {
            continue;
        };
        for entry in rd.flatten() {
            let fname = entry.file_name().to_string_lossy().into_owned();
            let Some(rest) = fname
                .strip_prefix("application-")
                .or_else(|| fname.strip_prefix("bootstrap-"))
            else {
                continue;
            };
            let prof_opt = rest
                .strip_suffix(".yml")
                .or_else(|| rest.strip_suffix(".yaml"))
                .or_else(|| rest.strip_suffix(".properties"));
            if let Some(prof) = prof_opt.map(str::trim).filter(|s| !s.is_empty()) {
                if seen.insert(("spring".into(), prof.to_string())) {
                    profiles.push(IdeBuildToolProfile {
                        id: prof.to_string(),
                        kind: "spring".into(),
                        source_project: project_name.to_string(),
                        active_by_default: false,
                    });
                }
            }
        }
    }
}

pub(super) fn shorten_home_path(path: &Path, home: Option<&Path>) -> String {
    if let Some(h) = home {
        if let Ok(rel) = path.strip_prefix(h) {
            return format!("~/{}", rel.to_string_lossy().replace('\\', "/"));
        }
    }
    path.to_string_lossy().into_owned()
}
