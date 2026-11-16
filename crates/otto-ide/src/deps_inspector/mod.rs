//! Multi-ecosystem Dependency & Vulnerability Inspector (`POST /ide/deps/inspect`)
//! and 1-click Manifest Actions (`POST /ide/deps/action`) across Go (`go.mod`),
//! Java (`pom.xml` + `build.gradle*`), JS/TS (`package.json`), Rust (`Cargo.toml`),
//! and Python (`requirements.txt` + `pyproject.toml`).

pub mod actions;
mod enrich;
mod go_mod;
mod parsers;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use crate::java_workspace::{
    parse_pom_properties_scoped, resolve_confined_parent_pom, strip_xml_comments,
};
use crate::types::{IdeDepItem, IdeDepManagedIn, IdeDepsInspectResp};

pub use self::actions::{run_deps_action, run_deps_action_as, run_deps_action_scoped};
use self::enrich::{
    enrich_crates_updates, enrich_maven_updates, enrich_npm_updates, enrich_osv_vulnerabilities,
    enrich_pypi_updates,
};
use self::go_mod::enrich_go_mod_updates;
pub use self::go_mod::parse_go_mod;
pub use self::parsers::{
    apply_cargo_lock_versions, parse_cargo_toml, parse_package_json, parse_python_deps,
};

/// Inspect a manifest file (`go.mod`, `pom.xml`, `build.gradle*`, `package.json`,
/// `Cargo.toml`, `requirements.txt`, `pyproject.toml`) for versions, upgrades,
/// deprecations, parent-managed coords, and OSV CVEs.
pub async fn inspect_dependencies(
    manifest_path: &Path,
    content_override: Option<&str>,
    env_map: &BTreeMap<String, String>,
) -> IdeDepsInspectResp {
    let default_root = manifest_path
        .parent()
        .and_then(|d| {
            #[allow(clippy::disallowed_methods)]
            let c = fs::canonicalize(d).ok();
            c
        })
        .unwrap_or_else(|| {
            manifest_path
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf()
        });
    inspect_dependencies_scoped(
        manifest_path,
        content_override,
        env_map,
        &[default_root],
        false,
    )
    .await
}

/// Scoped variant of [`inspect_dependencies`] that confines `<relativePath>`
/// parent-POM traversal to `allowed_roots` when `!caller_is_root` and rejects
/// `otto_core::secret_paths`.
pub async fn inspect_dependencies_scoped(
    manifest_path: &Path,
    content_override: Option<&str>,
    env_map: &BTreeMap<String, String>,
    allowed_roots: &[PathBuf],
    caller_is_root: bool,
) -> IdeDepsInspectResp {
    let content = match content_override {
        Some(c) => c.to_string(),
        None => {
            let p = manifest_path.to_path_buf();
            tokio::task::spawn_blocking(move || {
                #[allow(clippy::disallowed_methods)]
                let s = fs::read_to_string(&p).unwrap_or_default();
                s
            })
            .await
            .unwrap_or_default()
        }
    };
    let fname = manifest_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    let (ecosystem, osv_eco, mut items) = if fname == "go.mod" {
        let mut list = parse_go_mod(&content);
        enrich_go_mod_updates(manifest_path, &mut list, env_map).await;
        ("go", "Go", list)
    } else if fname == "pom.xml" {
        let mp = manifest_path.to_path_buf();
        let roots = allowed_roots.to_vec();
        let xml = content.clone();
        let mut list = tokio::task::spawn_blocking(move || {
            parse_pom_dependencies_scoped(&mp, &xml, &roots, caller_is_root)
        })
        .await
        .unwrap_or_default();
        enrich_maven_updates(&mut list, env_map).await;
        ("maven", "Maven", list)
    } else if fname == "build.gradle" || fname == "build.gradle.kts" {
        let mut list = parse_gradle_dependencies(&content);
        enrich_maven_updates(&mut list, env_map).await;
        ("gradle", "Maven", list)
    } else if fname == "package.json" {
        let mut list = parse_package_json(&content);
        enrich_npm_updates(&mut list).await;
        ("npm", "npm", list)
    } else if fname == "Cargo.toml" {
        let mut list = parse_cargo_toml(&content);
        let mp = manifest_path.to_path_buf();
        let roots = allowed_roots.to_vec();
        let lock = tokio::task::spawn_blocking(move || {
            // The nearest `Cargo.lock` up the tree (a member's lock lives at the workspace root).
            mp.ancestors()
                .skip(1)
                .take(8)
                .take_while(|d| caller_is_root || roots.iter().any(|r| d.starts_with(r)))
                .find_map(|d| fs::read_to_string(d.join("Cargo.lock")).ok())
        })
        .await
        .ok()
        .flatten();
        if let Some(lock) = lock {
            apply_cargo_lock_versions(&mut list, &lock);
        }
        enrich_crates_updates(&mut list).await;
        ("cargo", "crates.io", list)
    } else if fname == "requirements.txt" || fname == "pyproject.toml" {
        let mut list = parse_python_deps(fname, &content);
        enrich_pypi_updates(&mut list).await;
        ("pypi", "PyPI", list)
    } else {
        return IdeDepsInspectResp {
            ecosystem: "unknown".into(),
            items: Vec::new(),
        };
    };

    if !items.is_empty() && !otto_core::e2e::isolated() {
        enrich_osv_vulnerabilities(osv_eco, &mut items).await;
    }

    IdeDepsInspectResp {
        ecosystem: ecosystem.into(),
        items,
    }
}

// ── 2. Java (`pom.xml` & `build.gradle*`) ───────────────────────────────────

#[allow(clippy::disallowed_methods)]
pub fn parse_pom_dependencies(manifest_path: &Path, content: &str) -> Vec<IdeDepItem> {
    let dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let default_root = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    parse_pom_dependencies_scoped(manifest_path, content, &[default_root], false)
}

#[allow(clippy::disallowed_methods)]
pub fn parse_pom_dependencies_scoped(
    manifest_path: &Path,
    content: &str,
    allowed_roots: &[PathBuf],
    caller_is_root: bool,
) -> Vec<IdeDepItem> {
    let dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let stripped = strip_xml_comments(content);
    let props = parse_pom_properties_scoped(&stripped, dir, allowed_roots, caller_is_root);
    let parent_managed = load_parent_managed_deps(&stripped, dir, allowed_roots, caller_is_root);

    let lines: Vec<&str> = stripped.lines().collect();
    let mut items = Vec::new();
    let mut in_dep_mgmt = false;
    let mut i = 0usize;
    while i < lines.len() {
        let l = lines[i];
        if l.contains("<dependencyManagement>") {
            in_dep_mgmt = true;
        }
        if l.contains("</dependencyManagement>") {
            in_dep_mgmt = false;
        }
        if !in_dep_mgmt && l.contains("<dependency>") {
            let start_idx = i;
            let mut end_idx = i;
            let mut in_excl = false;
            let mut group = String::new();
            let mut artifact = String::new();
            let mut art_line = (i + 1) as u32;
            let mut art_col = 1u32;
            let mut art_end_col = 1u32;
            let mut raw_ver: Option<String> = None;
            let mut ver_line = (i + 1) as u32;
            let mut ver_col = 1u32;
            let mut ver_end_col = 1u32;

            while end_idx < lines.len() {
                let cl = lines[end_idx];
                if cl.contains("<exclusions>") {
                    in_excl = true;
                }
                if !in_excl {
                    if let Some((v, c1, c2)) = extract_line_tag_span(cl, "groupId") {
                        group = v;
                        let _ = (c1, c2);
                    }
                    if let Some((v, c1, c2)) = extract_line_tag_span(cl, "artifactId") {
                        artifact = v;
                        art_line = (end_idx + 1) as u32;
                        art_col = c1;
                        art_end_col = c2;
                    }
                    if let Some((v, c1, c2)) = extract_line_tag_span(cl, "version") {
                        raw_ver = Some(v);
                        ver_line = (end_idx + 1) as u32;
                        ver_col = c1;
                        ver_end_col = c2;
                    }
                }
                if cl.contains("</exclusions>") {
                    in_excl = false;
                }
                if cl.contains("</dependency>") {
                    break;
                }
                end_idx += 1;
            }
            if !artifact.is_empty() {
                let mut prop_name = None;
                let mut managed_in = None;
                let resolved_ver = if let Some(rv) = &raw_ver {
                    if let Some(inner) = rv.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
                        prop_name = Some(inner.to_string());
                        props.get(inner).cloned().unwrap_or_else(|| rv.clone())
                    } else {
                        rv.clone()
                    }
                } else if let Some((mver, mloc)) =
                    parent_managed.get(&(group.clone(), artifact.clone()))
                {
                    managed_in = Some(mloc.clone());
                    ver_line = art_line;
                    ver_col = art_col;
                    ver_end_col = art_end_col;
                    mver.clone()
                } else {
                    String::new()
                };

                let coord_name = if group.is_empty() {
                    artifact.clone()
                } else {
                    format!("{group}:{artifact}")
                };
                items.push(IdeDepItem {
                    name: coord_name,
                    current_version: resolved_ver,
                    latest_version: None,
                    latest_patch: None,
                    indirect: false,
                    deprecated: None,
                    line: art_line,
                    name_col: art_col,
                    name_end_col: art_end_col,
                    version_line: ver_line,
                    version_col: ver_col,
                    version_end_col: ver_end_col,
                    block_start_line: (start_idx + 1) as u32,
                    block_end_line: (end_idx.min(lines.len().saturating_sub(1)) + 1) as u32,
                    property_name: prop_name,
                    managed_in,
                    vulnerabilities: Vec::new(),
                });
            }
            i = end_idx + 1;
            continue;
        }
        i += 1;
    }
    items
}

#[allow(clippy::disallowed_methods)]
fn load_parent_managed_deps(
    stripped_xml: &str,
    dir: &Path,
    allowed_roots: &[PathBuf],
    caller_is_root: bool,
) -> HashMap<(String, String), (String, IdeDepManagedIn)> {
    let mut out = HashMap::new();
    let Some(canon_parent) =
        resolve_confined_parent_pom(stripped_xml, dir, allowed_roots, caller_is_root)
    else {
        return out;
    };
    let Ok(praw) = fs::read_to_string(&canon_parent) else {
        return out;
    };
    let pstrip = strip_xml_comments(&praw);
    let pprops = parse_pom_properties_scoped(
        &pstrip,
        canon_parent.parent().unwrap_or(dir),
        allowed_roots,
        caller_is_root,
    );
    let plines: Vec<&str> = pstrip.lines().collect();
    let mut in_dm = false;
    let mut idx = 0usize;
    while idx < plines.len() {
        if plines[idx].contains("<dependencyManagement>") {
            in_dm = true;
        }
        if plines[idx].contains("</dependencyManagement>") {
            in_dm = false;
        }
        if in_dm && plines[idx].contains("<dependency>") {
            let dep_line = (idx + 1) as u32;
            let mut g = String::new();
            let mut a = String::new();
            let mut v = String::new();
            while idx < plines.len() {
                if let Some((val, _, _)) = extract_line_tag_span(plines[idx], "groupId") {
                    g = crate::java_workspace::resolve_pom_props(&val, &pprops);
                }
                if let Some((val, _, _)) = extract_line_tag_span(plines[idx], "artifactId") {
                    a = crate::java_workspace::resolve_pom_props(&val, &pprops);
                }
                if let Some((val, _, _)) = extract_line_tag_span(plines[idx], "version") {
                    v = crate::java_workspace::resolve_pom_props(&val, &pprops);
                }
                if plines[idx].contains("</dependency>") {
                    break;
                }
                idx += 1;
            }
            if !g.is_empty() && !a.is_empty() {
                out.insert(
                    (g, a),
                    (
                        v,
                        IdeDepManagedIn {
                            path: canon_parent.to_string_lossy().into_owned(),
                            line: dep_line,
                        },
                    ),
                );
            }
        }
        idx += 1;
    }
    out
}

fn extract_line_tag_span(line: &str, tag: &str) -> Option<(String, u32, u32)> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let s = line.find(&open)? + open.len();
    let e = s + line[s..].find(&close)?;
    let val = line[s..e].trim().to_string();
    let col = (line[..s].chars().count() + 1) as u32;
    let end_col = (line[..e].chars().count() + 1) as u32;
    Some((val, col, end_col))
}

pub fn parse_gradle_dependencies(content: &str) -> Vec<IdeDepItem> {
    static RE_GRADLE_DEP: OnceLock<Regex> = OnceLock::new();
    let re = RE_GRADLE_DEP.get_or_init(|| {
        Regex::new(
            r#"(?:implementation|api|compileOnly|runtimeOnly|testImplementation)\s*\(?\s*['"]([^:'"]+):([^:'"]+):([^'"]+)['"]"#,
        )
        .unwrap()
    });
    let mut items = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if let Some(cap) = re.captures(line) {
            let g = cap.get(1).unwrap();
            let a = cap.get(2).unwrap();
            let v = cap.get(3).unwrap();
            let line_no = (idx + 1) as u32;
            let name_col = (line[..g.start()].chars().count() + 1) as u32;
            let name_end_col = (line[..a.end()].chars().count() + 1) as u32;
            let ver_col = (line[..v.start()].chars().count() + 1) as u32;
            let ver_end_col = (line[..v.end()].chars().count() + 1) as u32;
            items.push(IdeDepItem {
                name: format!("{}:{}", g.as_str(), a.as_str()),
                current_version: v.as_str().to_string(),
                latest_version: None,
                latest_patch: None,
                indirect: line.trim_start().starts_with("test"),
                deprecated: None,
                line: line_no,
                name_col,
                name_end_col,
                version_line: line_no,
                version_col: ver_col,
                version_end_col: ver_end_col,
                block_start_line: line_no,
                block_end_line: line_no,
                property_name: None,
                managed_in: None,
                vulnerabilities: Vec::new(),
            });
        }
    }
    items
}
