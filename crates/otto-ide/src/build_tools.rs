//! Stateless `< 15ms` Maven / Gradle / Spring Boot project & profile model
//! builder (`POST /ide/build-tools/model`) powering the right-hand Build Tool Window.

#![allow(clippy::disallowed_methods)] // synchronous scanner invoked inside spawn_blocking

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::java_workspace::{
    extract_first_tag_block, extract_tag_text, parse_pom_module, parse_pom_properties,
    remove_all_tag_blocks, resolve_pom_props, strip_xml_comments,
};
use crate::target::canonicalize_local_root_dir;
use crate::types::{
    IdeBuildToolProfile, IdeBuildToolProject, IdeBuildToolsModelReq, IdeBuildToolsModelResp,
};

const MAVEN_LIFECYCLE: &[&str] = &[
    "clean", "validate", "compile", "test", "package", "verify", "install", "deploy",
];
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

pub fn build_tools_model(
    req: &IdeBuildToolsModelReq,
    is_root_user: bool,
) -> IdeBuildToolsModelResp {
    let mut raw_roots = req.roots.clone();
    if let Some(t) = &req.target {
        if !t.root.trim().is_empty() {
            raw_roots.insert(0, t.root.clone());
        }
        for extra in t.merged_extra_roots() {
            raw_roots.push(extra);
        }
    }

    let home = dirs::home_dir();
    let mut canon_roots = Vec::new();
    for raw in raw_roots {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(c) = canonicalize_local_root_dir(trimmed, is_root_user) {
            if !canon_roots.contains(&c) {
                canon_roots.push(c);
            }
        }
    }

    let mut profiles: Vec<IdeBuildToolProfile> = Vec::new();
    let mut seen_profiles: HashSet<(String, String)> = HashSet::new();
    let mut projects: Vec<IdeBuildToolProject> = Vec::new();
    let mut visited_maven: HashSet<PathBuf> = HashSet::new();
    let mut visited_gradle: HashSet<PathBuf> = HashSet::new();
    let mut has_maven = false;
    let mut has_gradle = false;

    for root in &canon_roots {
        let mut matched_root = false;
        if root.join("pom.xml").is_file() {
            matched_root = true;
            if let Some(proj) = scan_maven_tree(
                root,
                0,
                home.as_deref(),
                &mut visited_maven,
                &mut profiles,
                &mut seen_profiles,
            ) {
                has_maven = true;
                projects.push(proj);
            }
        }
        if root.join("build.gradle").is_file() || root.join("build.gradle.kts").is_file() {
            matched_root = true;
            if let Some(proj) = scan_gradle_tree(
                root,
                0,
                home.as_deref(),
                &mut visited_gradle,
                &mut profiles,
                &mut seen_profiles,
            ) {
                has_gradle = true;
                projects.push(proj);
            }
        }
        if !matched_root {
            if let Ok(rd) = fs::read_dir(root) {
                // 1-level child repo scan if parent folder was passed
                let mut subdirs: Vec<PathBuf> = rd
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect();
                subdirs.sort();
                for sub in subdirs {
                    if sub.join("pom.xml").is_file() {
                        if let Some(proj) = scan_maven_tree(
                            &sub,
                            0,
                            home.as_deref(),
                            &mut visited_maven,
                            &mut profiles,
                            &mut seen_profiles,
                        ) {
                            has_maven = true;
                            projects.push(proj);
                        }
                    }
                    if sub.join("build.gradle").is_file() || sub.join("build.gradle.kts").is_file()
                    {
                        if let Some(proj) = scan_gradle_tree(
                            &sub,
                            0,
                            home.as_deref(),
                            &mut visited_gradle,
                            &mut profiles,
                            &mut seen_profiles,
                        ) {
                            has_gradle = true;
                            projects.push(proj);
                        }
                    }
                }
            }
        }
    }

    IdeBuildToolsModelResp {
        has_maven,
        has_gradle,
        profiles,
        projects,
    }
}

fn scan_maven_tree(
    dir: &Path,
    depth: usize,
    home: Option<&Path>,
    visited: &mut HashSet<PathBuf>,
    profiles: &mut Vec<IdeBuildToolProfile>,
    seen_profiles: &mut HashSet<(String, String)>,
) -> Option<IdeBuildToolProject> {
    if depth > 3 || !visited.insert(dir.to_path_buf()) {
        return None;
    }
    let pom_path = dir.join("pom.xml");
    let raw = fs::read_to_string(&pom_path).ok()?;
    let stripped = strip_xml_comments(&raw);
    let props = parse_pom_properties(&stripped, dir);
    let (ws_mod, submodules) = parse_pom_module(dir, &raw);

    let mut header = stripped.clone();
    for tag in [
        "parent",
        "dependencies",
        "dependencyManagement",
        "build",
        "profiles",
    ] {
        header = remove_all_tag_blocks(&header, tag);
    }
    let display_name = extract_tag_text(&header, "name")
        .map(|n| resolve_pom_props(&n, &props))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| ws_mod.artifact_id.clone());

    // Extract Maven `<profiles>`
    let prof_re = Regex::new(r"(?s)<profile>(.*?)</profile>").unwrap();
    if let Some(profs_block) = extract_first_tag_block(&stripped, "profiles") {
        for cap in prof_re.captures_iter(profs_block) {
            let pbody = &cap[1];
            if let Some(pid) = extract_tag_text(pbody, "id").filter(|s| !s.is_empty()) {
                let active = extract_first_tag_block(pbody, "activation")
                    .and_then(|act| extract_tag_text(act, "activeByDefault"))
                    .is_some_and(|v| v.eq_ignore_ascii_case("true"));
                if seen_profiles.insert(("maven".into(), pid.clone())) {
                    profiles.push(IdeBuildToolProfile {
                        id: pid,
                        kind: "maven".into(),
                        source_project: display_name.clone(),
                        active_by_default: active,
                    });
                }
            }
        }
    }

    // Extract Spring Boot `application-<profile>.yml/.yaml/.properties`
    scan_spring_profiles(dir, &display_name, profiles, seen_profiles);

    // Extract `<plugins>`
    let mut plugins = Vec::new();
    let plug_re = Regex::new(r"(?s)<plugin>(.*?)</plugin>").unwrap();
    for cap in plug_re.captures_iter(&stripped) {
        if let Some(aid) = extract_tag_text(&cap[1], "artifactId").filter(|s| !s.is_empty()) {
            let clean = aid
                .trim_start_matches("maven-")
                .trim_end_matches("-plugin")
                .to_string();
            let label = if clean.is_empty() { aid } else { clean };
            if !plugins.contains(&label) {
                plugins.push(label);
            }
        }
    }

    // Extract `<repositories>`
    let mut repositories = vec!["central (https://repo.maven.apache.org/maven2)".to_string()];
    let repo_re = Regex::new(r"(?s)<repository>(.*?)</repository>").unwrap();
    for cap in repo_re.captures_iter(&stripped) {
        let rid = extract_tag_text(&cap[1], "id").unwrap_or_else(|| "repo".into());
        let rurl = extract_tag_text(&cap[1], "url").unwrap_or_default();
        let label = if rurl.is_empty() {
            rid
        } else {
            format!("{rid} ({rurl})")
        };
        if !repositories.contains(&label) {
            repositories.push(label);
        }
    }

    let dependencies: Vec<String> = ws_mod
        .dependencies
        .iter()
        .map(|d| match &d.version {
            Some(v) if !v.is_empty() => format!("{}:{}:{v}", d.group_id, d.artifact_id),
            _ => format!("{}:{}", d.group_id, d.artifact_id),
        })
        .collect();

    let mut child_modules = Vec::new();
    for sub in submodules {
        let child_dir = fs::canonicalize(dir.join(&sub)).unwrap_or_else(|_| dir.join(&sub));
        if let Some(child_proj) = scan_maven_tree(
            &child_dir,
            depth + 1,
            home,
            visited,
            profiles,
            seen_profiles,
        ) {
            child_modules.push(child_proj);
        }
    }

    let project_coord = if ws_mod.group_id.is_empty() {
        ws_mod.artifact_id.clone()
    } else {
        format!("{}:{}", ws_mod.group_id, ws_mod.artifact_id)
    };

    Some(IdeBuildToolProject {
        name: display_name,
        group_id: ws_mod.group_id,
        artifact_id: ws_mod.artifact_id,
        version: ws_mod.version,
        tool: "maven".into(),
        project_coord,
        root_dir: dir.to_string_lossy().into_owned(),
        location_display: shorten_home_path(dir, home),
        manifest_path: pom_path.to_string_lossy().into_owned(),
        lifecycle: MAVEN_LIFECYCLE.iter().map(|s| (*s).to_string()).collect(),
        plugins,
        dependencies,
        repositories,
        modules: child_modules,
    })
}

fn scan_gradle_tree(
    dir: &Path,
    depth: usize,
    home: Option<&Path>,
    visited: &mut HashSet<PathBuf>,
    profiles: &mut Vec<IdeBuildToolProfile>,
    seen_profiles: &mut HashSet<(String, String)>,
) -> Option<IdeBuildToolProject> {
    if depth > 3 || !visited.insert(dir.to_path_buf()) {
        return None;
    }
    let manifest = if dir.join("build.gradle.kts").is_file() {
        dir.join("build.gradle.kts")
    } else {
        dir.join("build.gradle")
    };
    let raw = fs::read_to_string(&manifest).ok()?;
    let artifact_id = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("gradle-app")
        .to_string();
    let group_re = Regex::new(r#"(?m)^\s*group\s*=?\s*['"]([^'"]+)['"]"#).unwrap();
    let ver_re = Regex::new(r#"(?m)^\s*version\s*=?\s*['"]([^'"]+)['"]"#).unwrap();
    let group_id = group_re
        .captures(&raw)
        .map(|c| c[1].trim().to_string())
        .unwrap_or_default();
    let version = ver_re
        .captures(&raw)
        .map(|c| c[1].trim().to_string())
        .unwrap_or_else(|| "0.0.0".into());

    scan_spring_profiles(dir, &artifact_id, profiles, seen_profiles);

    // Gradle `-Pprofile` checks (`findProperty("profile")` or `profile-*.gradle`)
    let prop_prof_re = Regex::new(r#"profile-([A-Za-z0-9_-]+)\.gradle"#).unwrap();
    for cap in prop_prof_re.captures_iter(&raw) {
        let pid = cap[1].to_string();
        if seen_profiles.insert(("gradle".into(), pid.clone())) {
            profiles.push(IdeBuildToolProfile {
                id: pid,
                kind: "gradle".into(),
                source_project: artifact_id.clone(),
                active_by_default: false,
            });
        }
    }

    let mut plugins = Vec::new();
    let plug_re = Regex::new(r#"id\s*\(?\s*['"]([^'"]+)['"]"#).unwrap();
    for cap in plug_re.captures_iter(&raw) {
        let p = cap[1].to_string();
        if !plugins.contains(&p) {
            plugins.push(p);
        }
    }

    let mut dependencies = Vec::new();
    let dep_re = Regex::new(
        r#"(?:implementation|api|compileOnly|runtimeOnly|testImplementation)\s*\(?\s*['"]([^'"]+)['"]"#,
    )
    .unwrap();
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
    let inc_re = Regex::new(r#"include\s*\(?([^)\n]+)\)?"#).unwrap();
    let q_re = Regex::new(r#"['"]([^'"]+)['"]"#).unwrap();
    for settings_name in ["settings.gradle", "settings.gradle.kts"] {
        if let Ok(sraw) = fs::read_to_string(dir.join(settings_name)) {
            for cap in inc_re.captures_iter(&sraw) {
                for q in q_re.captures_iter(&cap[1]) {
                    let rel = q[1].replace(':', "/").trim_start_matches('/').to_string();
                    let cdir = fs::canonicalize(dir.join(&rel)).unwrap_or_else(|_| dir.join(&rel));
                    if let Some(cproj) =
                        scan_gradle_tree(&cdir, depth + 1, home, visited, profiles, seen_profiles)
                    {
                        child_modules.push(cproj);
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

    Some(IdeBuildToolProject {
        name: artifact_id.clone(),
        group_id,
        artifact_id,
        version,
        tool: "gradle".into(),
        project_coord,
        root_dir: dir.to_string_lossy().into_owned(),
        location_display: shorten_home_path(dir, home),
        manifest_path: manifest.to_string_lossy().into_owned(),
        lifecycle: GRADLE_LIFECYCLE.iter().map(|s| (*s).to_string()).collect(),
        plugins,
        dependencies,
        repositories,
        modules: child_modules,
    })
}

fn scan_spring_profiles(
    dir: &Path,
    project_name: &str,
    profiles: &mut Vec<IdeBuildToolProfile>,
    seen: &mut HashSet<(String, String)>,
) {
    for rel in ["src/main/resources", "src/test/resources", "config"] {
        let res_dir = dir.join(rel);
        let Ok(rd) = fs::read_dir(&res_dir) else {
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

fn shorten_home_path(path: &Path, home: Option<&Path>) -> String {
    if let Some(h) = home {
        if let Ok(rel) = path.strip_prefix(h) {
            return format!("~/{}", rel.to_string_lossy().replace('\\', "/"));
        }
    }
    path.to_string_lossy().into_owned()
}
