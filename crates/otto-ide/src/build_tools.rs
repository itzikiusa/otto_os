//! Stateless `< 15ms` Maven / Gradle / Spring Boot project & profile model
//! builder (`POST /ide/build-tools/model`) powering the right-hand Build Tool Window.

#![allow(clippy::disallowed_methods)] // synchronous scanner invoked inside spawn_blocking

#[path = "build_tools/gradle.rs"]
mod gradle;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use self::gradle::{scan_gradle_tree, scan_spring_profiles, shorten_home_path};
use crate::java_workspace::{
    extract_first_tag_block, extract_tag_text, parse_pom_module_scoped,
    parse_pom_properties_scoped, remove_all_tag_blocks, resolve_pom_props, strip_xml_comments,
};
use crate::target::{canonicalize_local_root_dir, enforce_non_root_path};
use crate::types::{
    IdeBuildToolProfile, IdeBuildToolProject, IdeBuildToolsModelReq, IdeBuildToolsModelResp,
};

const MAVEN_LIFECYCLE: &[&str] = &[
    "clean", "validate", "compile", "test", "package", "verify", "install", "deploy",
];

struct BuildScanState<'a> {
    canon_roots: &'a [PathBuf],
    is_root_user: bool,
    home: Option<&'a Path>,
    visited: HashSet<PathBuf>,
    profiles: &'a mut Vec<IdeBuildToolProfile>,
    seen_profiles: &'a mut HashSet<(String, String)>,
}

/// Resolve `cand` to a canonical directory strictly inside `canon_roots` (so symlinks,
/// `../` `<module>` declarations, and Gradle `include(...)` paths cannot escape workspace roots).
fn confinable_project_dir(
    cand: &Path,
    canon_roots: &[PathBuf],
    is_root_user: bool,
) -> Option<PathBuf> {
    let canon = fs::canonicalize(cand).ok()?;
    if !canon.is_dir() || !canon_roots.iter().any(|r| canon.starts_with(r)) {
        return None;
    }
    if !is_root_user && enforce_non_root_path(&canon).is_err() {
        return None;
    }
    Some(canon)
}

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
            let mut st = BuildScanState {
                canon_roots: &canon_roots,
                is_root_user,
                home: home.as_deref(),
                visited: std::mem::take(&mut visited_maven),
                profiles: &mut profiles,
                seen_profiles: &mut seen_profiles,
            };
            if let Some(proj) = scan_maven_tree(root, 0, &mut st) {
                has_maven = true;
                projects.push(proj);
            }
            visited_maven = st.visited;
        }
        if root.join("build.gradle").is_file() || root.join("build.gradle.kts").is_file() {
            matched_root = true;
            let mut st = BuildScanState {
                canon_roots: &canon_roots,
                is_root_user,
                home: home.as_deref(),
                visited: std::mem::take(&mut visited_gradle),
                profiles: &mut profiles,
                seen_profiles: &mut seen_profiles,
            };
            if let Some(proj) = scan_gradle_tree(root, 0, &mut st) {
                has_gradle = true;
                projects.push(proj);
            }
            visited_gradle = st.visited;
        }
        if !matched_root {
            if let Ok(rd) = fs::read_dir(root) {
                let mut subdirs: Vec<PathBuf> = rd
                    .flatten()
                    .filter(|e| e.file_type().map(|ft| !ft.is_symlink()).unwrap_or(false))
                    .filter_map(|e| confinable_project_dir(&e.path(), &canon_roots, is_root_user))
                    .collect();
                subdirs.sort();
                for sub in subdirs {
                    if sub.join("pom.xml").is_file() {
                        let mut st = BuildScanState {
                            canon_roots: &canon_roots,
                            is_root_user,
                            home: home.as_deref(),
                            visited: std::mem::take(&mut visited_maven),
                            profiles: &mut profiles,
                            seen_profiles: &mut seen_profiles,
                        };
                        if let Some(proj) = scan_maven_tree(&sub, 0, &mut st) {
                            has_maven = true;
                            projects.push(proj);
                        }
                        visited_maven = st.visited;
                    }
                    if sub.join("build.gradle").is_file() || sub.join("build.gradle.kts").is_file()
                    {
                        let mut st = BuildScanState {
                            canon_roots: &canon_roots,
                            is_root_user,
                            home: home.as_deref(),
                            visited: std::mem::take(&mut visited_gradle),
                            profiles: &mut profiles,
                            seen_profiles: &mut seen_profiles,
                        };
                        if let Some(proj) = scan_gradle_tree(&sub, 0, &mut st) {
                            has_gradle = true;
                            projects.push(proj);
                        }
                        visited_gradle = st.visited;
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
    st: &mut BuildScanState<'_>,
) -> Option<IdeBuildToolProject> {
    if depth > 3 || !st.visited.insert(dir.to_path_buf()) {
        return None;
    }
    let pom_path = dir.join("pom.xml");
    if fs::symlink_metadata(&pom_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(true)
    {
        return None;
    }
    let raw = fs::read_to_string(&pom_path).ok()?;
    let stripped = strip_xml_comments(&raw);
    let props = parse_pom_properties_scoped(&stripped, dir, st.canon_roots, st.is_root_user);
    let (ws_mod, submodules) = parse_pom_module_scoped(dir, &raw, st.canon_roots, st.is_root_user);

    // Every <project> child that nests its own <name>, so only the project's <name> is left.
    let mut header = stripped.clone();
    for tag in [
        "parent",
        "dependencies",
        "dependencyManagement",
        "build",
        "profiles",
        "distributionManagement",
        "repositories",
        "pluginRepositories",
        "organization",
        "developers",
        "contributors",
        "licenses",
        "mailingLists",
    ] {
        header = remove_all_tag_blocks(&header, tag);
    }
    let display_name = extract_tag_text(&header, "name")
        .map(|n| resolve_pom_props(&n, &props))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| ws_mod.artifact_id.clone());

    static RE_PROF: OnceLock<Regex> = OnceLock::new();
    let prof_re = RE_PROF.get_or_init(|| Regex::new(r"(?s)<profile>(.*?)</profile>").unwrap());
    if let Some(profs_block) = extract_first_tag_block(&stripped, "profiles") {
        for cap in prof_re.captures_iter(profs_block) {
            let pbody = &cap[1];
            if let Some(pid) = extract_tag_text(pbody, "id").filter(|s| !s.is_empty()) {
                let active = extract_first_tag_block(pbody, "activation")
                    .and_then(|act| extract_tag_text(act, "activeByDefault"))
                    .is_some_and(|v| v.eq_ignore_ascii_case("true"));
                if st.seen_profiles.insert(("maven".into(), pid.clone())) {
                    st.profiles.push(IdeBuildToolProfile {
                        id: pid,
                        kind: "maven".into(),
                        source_project: display_name.clone(),
                        active_by_default: active,
                    });
                }
            }
        }
    }

    scan_spring_profiles(
        dir,
        &display_name,
        st.canon_roots,
        st.profiles,
        st.seen_profiles,
    );

    let mut plugins = Vec::new();
    static RE_PLUG: OnceLock<Regex> = OnceLock::new();
    let plug_re = RE_PLUG.get_or_init(|| Regex::new(r"(?s)<plugin>(.*?)</plugin>").unwrap());
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

    let mut repositories = vec!["central (https://repo.maven.apache.org/maven2)".to_string()];
    static RE_REPO: OnceLock<Regex> = OnceLock::new();
    let repo_re =
        RE_REPO.get_or_init(|| Regex::new(r"(?s)<repository>(.*?)</repository>").unwrap());
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
        let Some(child_dir) =
            confinable_project_dir(&dir.join(&sub), st.canon_roots, st.is_root_user)
        else {
            continue;
        };
        if let Some(child_proj) = scan_maven_tree(&child_dir, depth + 1, st) {
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
        location_display: shorten_home_path(dir, st.home),
        manifest_path: pom_path.to_string_lossy().into_owned(),
        lifecycle: MAVEN_LIFECYCLE.iter().map(|s| (*s).to_string()).collect(),
        plugins,
        dependencies,
        repositories,
        modules: child_modules,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn build_tools_model_refuses_module_escape_outside_workspace_roots() {
        let outer = tempfile::tempdir().unwrap();
        let ws = outer.path().join("workspace");
        let outside = outer.path().join("outside-secret-proj");
        fs::create_dir_all(&ws).unwrap();
        fs::create_dir_all(&outside).unwrap();

        fs::write(
            outside.join("pom.xml"),
            "<project><groupId>sec</groupId><artifactId>escaped-mod</artifactId></project>",
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside, ws.join("symlink-mod")).unwrap();
        fs::write(
            ws.join("pom.xml"),
            "<project><groupId>app</groupId><artifactId>root-app</artifactId><modules><module>../outside-secret-proj</module><module>symlink-mod</module></modules></project>",
        )
        .unwrap();

        let resp = build_tools_model(
            &IdeBuildToolsModelReq {
                target: None,
                roots: vec![ws.to_string_lossy().into_owned()],
            },
            false,
        );
        assert_eq!(resp.projects.len(), 1);
        assert_eq!(resp.projects[0].artifact_id, "root-app");
        assert!(
            resp.projects[0].modules.is_empty(),
            "parent-traversal and symlinked modules outside authorized roots must be skipped"
        );
    }

    #[test]
    fn project_name_ignores_names_nested_in_deploy_repositories() {
        let ws = tempfile::tempdir().unwrap();
        let repos = "<distributionManagement><repository><id>releases</id><name>Corp Releases</name></repository></distributionManagement>\
             <repositories><repository><id>releases</id><name>Corp Releases</name></repository></repositories>";
        fs::write(
            ws.path().join("pom.xml"),
            format!("<project><groupId>com.corp</groupId><artifactId>config-client</artifactId>{repos}</project>"),
        )
        .unwrap();

        let resp = build_tools_model(
            &IdeBuildToolsModelReq {
                target: None,
                roots: vec![ws.path().to_string_lossy().into_owned()],
            },
            false,
        );
        assert_eq!(resp.projects[0].name, "config-client");
    }
}
