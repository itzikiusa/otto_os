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
    enrich_crates_updates, enrich_maven_updates, enrich_npm_updates, enrich_nuget_updates,
    enrich_osv_vulnerabilities, enrich_pypi_updates,
};
use self::go_mod::enrich_go_mod_updates;
pub use self::go_mod::parse_go_mod;
pub use self::parsers::{
    apply_cargo_lock_versions, parse_build_zig_zon, parse_cargo_toml, parse_cmake_deps,
    parse_conanfile_txt, parse_dotnet_deps, parse_package_json, parse_python_deps,
    parse_unreal_plugins, parse_vcpkg_json,
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

    let fname_low = fname.to_ascii_lowercase();
    let is_dotnet_manifest = fname_low.ends_with(".csproj")
        || fname_low.ends_with(".fsproj")
        || fname_low.ends_with(".vbproj")
        || fname_low == "directory.packages.props"
        || fname_low == "packages.config";
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
    } else if fname == "libs.versions.toml" {
        let mut list = parse_gradle_version_catalog(&content);
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
    } else if fname == "build.zig.zon" {
        let list = parse_build_zig_zon(&content);
        ("zig", "", list)
    } else if fname == "vcpkg.json" {
        let list = parse_vcpkg_json(&content);
        ("vcpkg", "", list)
    } else if fname == "conanfile.txt" {
        let list = parse_conanfile_txt(&content);
        ("conan", "", list)
    } else if fname == "CMakeLists.txt" {
        let list = parse_cmake_deps(&content);
        ("cmake", "", list)
    } else if fname.ends_with(".uproject") || fname.ends_with(".uplugin") {
        let list = parse_unreal_plugins(&content);
        ("unreal", "", list)
    } else if is_dotnet_manifest {
        let mut list = parse_dotnet_deps(&content);
        enrich_nuget_updates(&mut list).await;
        ("nuget", "NuGet", list)
    } else {
        return IdeDepsInspectResp {
            ecosystem: "unknown".into(),
            items: Vec::new(),
        };
    };

    if !items.is_empty() && !osv_eco.is_empty() && !otto_core::e2e::isolated() {
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
                    range: false,
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
    static RE_KOTLIN_DSL: OnceLock<Regex> = OnceLock::new();
    let re = RE_GRADLE_DEP.get_or_init(|| {
        Regex::new(
            r#"(?:implementation|api|compileOnly|runtimeOnly|testImplementation|testRuntimeOnly|kapt|ksp)\s*\(?\s*['"]([^:'"]+):([^:'"]+):([^'"]+)['"]"#,
        )
        .unwrap()
    });
    let re_kt = RE_KOTLIN_DSL.get_or_init(|| {
        Regex::new(
            r#"(?:implementation|api|compileOnly|runtimeOnly|testImplementation|testRuntimeOnly)\s*\(\s*kotlin\s*\(\s*"([^"]+)"(?:\s*,\s*"([^"]+)")?\s*\)\s*\)"#,
        )
        .unwrap()
    });
    let mut items = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        if let Some(cap) = re.captures(line) {
            let g = cap.get(1).unwrap();
            let a = cap.get(2).unwrap();
            let v = cap.get(3).unwrap();
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
                range: false,
            });
        } else if let Some(cap) = re_kt.captures(line) {
            let mod_m = cap.get(1).unwrap();
            let ver_m = cap.get(2);
            let name_col = (line[..mod_m.start()].chars().count() + 1) as u32;
            let name_end_col = (line[..mod_m.end()].chars().count() + 1) as u32;
            let ver = ver_m
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| "plugin".into());
            let (ver_col, ver_end_col) = ver_m
                .map(|m| {
                    (
                        (line[..m.start()].chars().count() + 1) as u32,
                        (line[..m.end()].chars().count() + 1) as u32,
                    )
                })
                .unwrap_or((0, 0));
            items.push(IdeDepItem {
                name: format!("org.jetbrains.kotlin:kotlin-{}", mod_m.as_str()),
                current_version: ver,
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
                range: false,
            });
        }
    }
    items
}

/// Parse Gradle Version Catalog (`gradle/libs.versions.toml`) `[versions]` + `[libraries]`.
pub fn parse_gradle_version_catalog(content: &str) -> Vec<IdeDepItem> {
    static RE_KV: OnceLock<Regex> = OnceLock::new();
    static RE_MOD_VER: OnceLock<Regex> = OnceLock::new();
    static RE_GROUP_NAME: OnceLock<Regex> = OnceLock::new();
    static RE_VER_REF: OnceLock<Regex> = OnceLock::new();
    static RE_VER_INLINE: OnceLock<Regex> = OnceLock::new();

    let re_kv = RE_KV.get_or_init(|| Regex::new(r#"^([A-Za-z0-9_.-]+)\s*=\s*"([^"]+)""#).unwrap());
    let re_mod_ver = RE_MOD_VER
        .get_or_init(|| Regex::new(r#"module\s*=\s*"([^":]+:[^":]+)(?::([^"]+))?""#).unwrap());
    let re_group_name = RE_GROUP_NAME
        .get_or_init(|| Regex::new(r#"group\s*=\s*"([^"]+)".*?name\s*=\s*"([^"]+)""#).unwrap());
    let re_ver_ref =
        RE_VER_REF.get_or_init(|| Regex::new(r#"version\.ref\s*=\s*"([^"]+)""#).unwrap());
    let re_ver_inline =
        RE_VER_INLINE.get_or_init(|| Regex::new(r#"version\s*=\s*"([^"]+)""#).unwrap());

    let mut versions = HashMap::<String, (String, u32, u32, u32)>::new();
    let mut section = String::new();
    for (idx, line) in content.lines().enumerate() {
        let t = line.trim();
        let line_no = (idx + 1) as u32;
        if t.starts_with('[') && t.ends_with(']') {
            section = t[1..t.len() - 1].trim().to_ascii_lowercase();
            continue;
        }
        if section == "versions" {
            if let Some(cap) = re_kv.captures(t) {
                let val = cap[2].to_string();
                let (vc, vec) = line
                    .find(&format!("\"{val}\""))
                    .map(|pos| {
                        let start = (line[..pos + 1].chars().count() + 1) as u32;
                        (start, start + val.chars().count() as u32)
                    })
                    .unwrap_or((1, 1));
                versions.insert(cap[1].to_string(), (val, line_no, vc, vec));
            }
        }
    }

    let mut items = Vec::new();
    section.clear();
    for (idx, line) in content.lines().enumerate() {
        let t = line.trim();
        let line_no = (idx + 1) as u32;
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t.starts_with('[') && t.ends_with(']') {
            section = t[1..t.len() - 1].trim().to_ascii_lowercase();
            continue;
        }
        if section != "libraries" {
            continue;
        }
        let mut coord: Option<String> = None;
        let mut ver: Option<String> = None;
        let mut prop_name: Option<String> = None;
        let mut ver_span: Option<(u32, u32, u32)> = None;
        if let Some(cap) = re_kv.captures(t) {
            let parts: Vec<&str> = cap[2].split(':').collect();
            if parts.len() >= 2 {
                coord = Some(format!("{}:{}", parts[0], parts[1]));
                if let Some(v) = parts.get(2) {
                    ver = Some((*v).to_string());
                    if let Some(pos) = line.rfind(&format!(":{v}\"")) {
                        let start = (line[..pos + 1].chars().count() + 1) as u32;
                        ver_span = Some((line_no, start, start + v.chars().count() as u32));
                    }
                }
            }
        } else if let Some(cap) = re_mod_ver.captures(t) {
            coord = Some(cap[1].to_string());
            if let Some(m) = cap.get(2) {
                let v = m.as_str();
                ver = Some(v.to_string());
                if let Some(pos) = line.find(&format!(":{v}\"")) {
                    let start = (line[..pos + 1].chars().count() + 1) as u32;
                    ver_span = Some((line_no, start, start + v.chars().count() as u32));
                }
            }
        } else if let Some(cap) = re_group_name.captures(t) {
            coord = Some(format!("{}:{}", &cap[1], &cap[2]));
        }
        if let Some(c) = coord {
            if ver.is_none() {
                if let Some(vref) = re_ver_ref.captures(t) {
                    let key = vref[1].to_string();
                    if let Some((v, vl, vc, vec)) = versions.get(&key) {
                        ver = Some(v.clone());
                        ver_span = Some((*vl, *vc, *vec));
                    }
                    prop_name = Some(key);
                } else if let Some(vin) = re_ver_inline.captures(t) {
                    let v = vin[1].to_string();
                    if let Some(pos) = line.find(&format!("\"{v}\"")) {
                        let start = (line[..pos + 1].chars().count() + 1) as u32;
                        ver_span = Some((line_no, start, start + v.chars().count() as u32));
                    }
                    ver = Some(v);
                }
            }
            let current_version = ver.unwrap_or_else(|| "*".into());
            let name_col = line
                .find(&c)
                .map(|i| (line[..i].chars().count() + 1) as u32)
                .unwrap_or(1);
            let name_end_col = name_col + c.chars().count() as u32;
            let (version_line, version_col, version_end_col) = ver_span.unwrap_or((line_no, 0, 0));
            items.push(IdeDepItem {
                name: c,
                current_version,
                latest_version: None,
                latest_patch: None,
                indirect: false,
                deprecated: None,
                line: line_no,
                name_col,
                name_end_col,
                version_line,
                version_col,
                version_end_col,
                block_start_line: line_no,
                block_end_line: line_no,
                property_name: prop_name,
                managed_in: None,
                vulnerabilities: Vec::new(),
                range: false,
            });
        }
    }
    items
}
