//! Multi-ecosystem Dependency & Vulnerability Inspector (`POST /ide/deps/inspect`)
//! and 1-click Manifest Actions (`POST /ide/deps/action`) across Go (`go.mod`),
//! Java (`pom.xml` + `build.gradle*`), JS/TS (`package.json`), Rust (`Cargo.toml`),
//! and Python (`requirements.txt` + `pyproject.toml`).

pub mod actions;
mod enrich;
mod parsers;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use serde_json::Value;

use crate::java_workspace::{parse_pom_properties, strip_xml_comments};
use crate::types::{IdeDepItem, IdeDepManagedIn, IdeDepsInspectResp};

pub use self::actions::run_deps_action;
use self::enrich::{
    enrich_crates_updates, enrich_maven_updates, enrich_npm_updates, enrich_osv_vulnerabilities,
    enrich_pypi_updates, same_major_minor,
};
pub use self::parsers::{parse_cargo_toml, parse_package_json, parse_python_deps};

/// Inspect a manifest file (`go.mod`, `pom.xml`, `build.gradle*`, `package.json`,
/// `Cargo.toml`, `requirements.txt`, `pyproject.toml`) for versions, upgrades,
/// deprecations, parent-managed coords, and OSV CVEs.
pub async fn inspect_dependencies(
    manifest_path: &Path,
    content_override: Option<&str>,
    env_map: &BTreeMap<String, String>,
) -> IdeDepsInspectResp {
    let content = match content_override {
        Some(c) => c.to_string(),
        None => fs::read_to_string(manifest_path).unwrap_or_default(),
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
        let mut list = parse_pom_dependencies(manifest_path, &content);
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

// ── 1. Go (`go.mod`) ────────────────────────────────────────────────────────

pub fn parse_go_mod(content: &str) -> Vec<IdeDepItem> {
    let mut items = Vec::new();
    let mut in_require = false;
    for (idx, raw_line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let code_trimmed = raw_line
            .split_once("//")
            .map_or(raw_line, |(c, _)| c)
            .trim();
        if code_trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = code_trimmed.strip_prefix("require") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with('(') {
                let after = rest.trim();
                if after.starts_with('(') {
                    if after.contains(')') {
                        if let Some(item) = parse_go_require_spec(raw_line, line_no, true) {
                            items.push(item);
                        }
                    } else {
                        in_require = true;
                    }
                    continue;
                }
                if let Some(item) = parse_go_require_spec(raw_line, line_no, true) {
                    items.push(item);
                }
                continue;
            }
        }
        if in_require {
            if code_trimmed.starts_with(')') {
                in_require = false;
                continue;
            }
            if let Some(item) = parse_go_require_spec(raw_line, line_no, false) {
                items.push(item);
            }
            if code_trimmed.ends_with(')') {
                in_require = false;
            }
        }
    }
    items
}

fn parse_go_require_spec(raw_line: &str, line_no: u32, strip_kw: bool) -> Option<IdeDepItem> {
    let (code_part, comment_part) = match raw_line.split_once("//") {
        Some((c, cm)) => (c, Some(cm.trim())),
        None => (raw_line, None),
    };
    let indirect = comment_part.is_some_and(|c| c.contains("indirect"));
    let inline_dep = comment_part.and_then(|c| {
        c.strip_prefix("Deprecated:")
            .or_else(|| c.strip_prefix("deprecated:"))
            .map(|s| s.trim().to_string())
    });

    let work = if strip_kw {
        let pos = code_part.find("require")? + "require".len();
        &code_part[pos..]
    } else {
        code_part
    };
    let trimmed_work = work.trim();
    let inner_work = trimmed_work
        .strip_prefix('(')
        .map(|s| s.strip_suffix(')').unwrap_or(s))
        .unwrap_or_else(|| trimmed_work.strip_suffix(')').unwrap_or(trimmed_work));
    let mut parts = inner_work.split_whitespace();
    let mod_name = parts.next()?;
    let ver = parts.next()?;
    if mod_name.starts_with('(') || mod_name.starts_with(')') {
        return None;
    }
    let name_byte = raw_line.find(mod_name)?;
    let ver_search_start = name_byte + mod_name.len();
    let ver_byte = ver_search_start + raw_line[ver_search_start..].find(ver)?;
    let name_col = (raw_line[..name_byte].chars().count() + 1) as u32;
    let name_end_col = name_col + mod_name.chars().count() as u32;
    let version_col = (raw_line[..ver_byte].chars().count() + 1) as u32;
    let version_end_col = version_col + ver.chars().count() as u32;

    Some(IdeDepItem {
        name: mod_name.to_string(),
        current_version: ver.to_string(),
        latest_version: None,
        latest_patch: None,
        indirect,
        deprecated: inline_dep,
        line: line_no,
        name_col,
        name_end_col,
        version_line: line_no,
        version_col,
        version_end_col,
        block_start_line: line_no,
        block_end_line: line_no,
        property_name: None,
        managed_in: None,
        vulnerabilities: Vec::new(),
    })
}

async fn enrich_go_mod_updates(
    manifest_path: &Path,
    items: &mut [IdeDepItem],
    env_map: &BTreeMap<String, String>,
) {
    if items.is_empty() || otto_core::e2e::isolated() {
        return;
    }
    let Some(dir) = manifest_path.parent().filter(|d| d.is_dir()) else {
        return;
    };
    let mut cmd = tokio::process::Command::new("go");
    cmd.args(["list", "-m", "-u", "-json", "all"])
        .current_dir(dir)
        .env_clear()
        .envs(crate::environments::shell_env::collect_safe_non_root_base_env())
        .envs(env_map)
        .kill_on_drop(true);
    let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(6), cmd.output()).await else {
        return;
    };
    if out.stdout.is_empty() {
        return;
    }
    let stream = serde_json::Deserializer::from_slice(&out.stdout).into_iter::<Value>();
    let mut updates: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    for val in stream.flatten() {
        let Some(path) = val.get("Path").and_then(Value::as_str) else {
            continue;
        };
        let latest = val
            .get("Update")
            .and_then(|u| u.get("Version"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let dep = val
            .get("Deprecated")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        updates.insert(path.to_string(), (latest, dep));
    }
    for item in items.iter_mut() {
        if let Some((latest, dep)) = updates.get(&item.name) {
            if let Some(lv) = latest {
                if lv != &item.current_version {
                    item.latest_version = Some(lv.clone());
                    if same_major_minor(&item.current_version, lv) {
                        item.latest_patch = Some(lv.clone());
                    }
                }
            }
            if dep.is_some() {
                item.deprecated = dep.clone();
            }
        }
    }
}

// ── 2. Java (`pom.xml` & `build.gradle*`) ───────────────────────────────────

pub fn parse_pom_dependencies(manifest_path: &Path, content: &str) -> Vec<IdeDepItem> {
    let dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let stripped = strip_xml_comments(content);
    let props = parse_pom_properties(&stripped, dir);
    let parent_managed = load_parent_managed_deps(&stripped, dir);

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

fn load_parent_managed_deps(
    stripped_xml: &str,
    dir: &Path,
) -> HashMap<(String, String), (String, IdeDepManagedIn)> {
    let mut out = HashMap::new();
    let rel = crate::java_workspace::extract_first_tag_block(stripped_xml, "parent")
        .and_then(|b| crate::java_workspace::extract_tag_text(b, "relativePath"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "../pom.xml".into());
    let parent_pom = if rel.ends_with(".xml") {
        dir.join(rel)
    } else {
        dir.join(rel).join("pom.xml")
    };
    let Ok(canon_parent) = fs::canonicalize(&parent_pom) else {
        return out;
    };
    let Ok(praw) = fs::read_to_string(&canon_parent) else {
        return out;
    };
    let pstrip = strip_xml_comments(&praw);
    let pprops = parse_pom_properties(&pstrip, canon_parent.parent().unwrap_or(dir));
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
