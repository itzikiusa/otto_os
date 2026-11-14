//! Multi-repo Java workspace scanner (Maven `pom.xml` + Gradle `build.gradle*`)
//! and build/classpath pipeline:
//! - Dependencies matching an attached IDE project folder (`project_folders` /
//!   `additional_roots`) compile directly from local source with `javac -g` FIRST
//!   and precede `~/.m2` JARs on `-classpath` so uncommitted edits in `toolkit`
//!   always win for Run, JDWP Debug, Go-to-Definition, Hover, and Diagnostics.
//! - Dependencies NOT in the current IDE project resolve from `MAVEN_REPO_LOCAL` /
//!   `~/.m2/repository` or `~/.gradle/caches/modules-2/files-2.1`.

#![allow(clippy::disallowed_methods)] // synchronous workspace/JAR scanner invoked inside spawn_blocking

pub mod builder;
pub mod zip_jdk;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

pub use self::builder::{
    collect_java_files_rec, infer_java_fqcn_from_file, prepare_java_execution, JavaExecutionPlan,
};
pub use self::zip_jdk::{
    global_jdk_index, read_jar_classes_and_packages, read_zip_central_dir_entries, JdkClassIndex,
};

#[derive(Debug, Clone)]
pub struct JavaDepCoord {
    pub group_id: String,
    pub artifact_id: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceJavaModule {
    pub key: String,
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
    pub dir: PathBuf,
    pub main_src_dirs: Vec<PathBuf>,
    pub test_src_dirs: Vec<PathBuf>,
    pub resource_dirs: Vec<PathBuf>,
    pub dependencies: Vec<JavaDepCoord>,
}

#[derive(Debug, Clone, Default)]
pub struct JavaWorkspaceGraph {
    pub modules: Vec<WorkspaceJavaModule>,
    pub by_coord: HashMap<(String, String), usize>,
    pub by_artifact: HashMap<String, usize>,
}

impl JavaWorkspaceGraph {
    pub fn find_dep(&self, group_id: &str, artifact_id: &str) -> Option<&WorkspaceJavaModule> {
        if !group_id.is_empty() {
            if let Some(&idx) = self
                .by_coord
                .get(&(group_id.to_string(), artifact_id.to_string()))
            {
                return self.modules.get(idx);
            }
        }
        self.by_artifact
            .get(artifact_id)
            .and_then(|&idx| self.modules.get(idx))
    }

    pub fn owning_module(&self, file_or_dir: &Path) -> Option<&WorkspaceJavaModule> {
        let canon = fs::canonicalize(file_or_dir).unwrap_or_else(|_| file_or_dir.to_path_buf());
        self.modules
            .iter()
            .filter(|m| canon.starts_with(&m.dir))
            .max_by_key(|m| m.dir.as_os_str().len())
    }

    pub fn all_source_roots(&self, include_tests: bool) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for m in &self.modules {
            for s in &m.main_src_dirs {
                if s.is_dir() && !out.contains(s) {
                    out.push(s.clone());
                }
            }
            if include_tests {
                for s in &m.test_src_dirs {
                    if s.is_dir() && !out.contains(s) {
                        out.push(s.clone());
                    }
                }
            }
        }
        out
    }
}

/// Scan all attached IDE project folders (and Maven `<modules>` / Gradle subprojects)
/// to build a `JavaWorkspaceGraph`.
pub fn scan_java_workspace(roots: &[PathBuf]) -> JavaWorkspaceGraph {
    let mut graph = JavaWorkspaceGraph::default();
    let mut visited_dirs = HashSet::new();
    for root in roots {
        let canon = fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        scan_java_dir_recursive(&canon, 0, &mut visited_dirs, &mut graph);
    }
    graph
}

fn scan_java_dir_recursive(
    dir: &Path,
    depth: usize,
    visited: &mut HashSet<PathBuf>,
    graph: &mut JavaWorkspaceGraph,
) {
    if depth > 2 || !dir.is_dir() || !visited.insert(dir.to_path_buf()) {
        return;
    }
    let pom = dir.join("pom.xml");
    let gradle = dir.join("build.gradle");
    let gradle_kts = dir.join("build.gradle.kts");

    let mut subdirs_to_visit = Vec::new();
    if pom.is_file() {
        if let Ok(raw) = fs::read_to_string(&pom) {
            let (module, submodules) = parse_pom_module(dir, &raw);
            register_module(graph, module);
            for sub in submodules {
                subdirs_to_visit.push(dir.join(sub));
            }
        }
    } else if gradle.is_file() || gradle_kts.is_file() {
        let gfile = if gradle.is_file() { gradle } else { gradle_kts };
        if let Ok(raw) = fs::read_to_string(&gfile) {
            let module = parse_gradle_module(dir, &raw);
            register_module(graph, module);
        }
        for settings_name in ["settings.gradle", "settings.gradle.kts"] {
            let sp = dir.join(settings_name);
            if let Ok(sraw) = fs::read_to_string(&sp) {
                for sub in parse_gradle_includes(&sraw) {
                    subdirs_to_visit.push(dir.join(sub.replace(':', "/").trim_start_matches('/')));
                }
            }
        }
    } else if depth == 0 && dir.join("src/main/java").is_dir() {
        let name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("app")
            .to_string();
        register_module(
            graph,
            WorkspaceJavaModule {
                key: format!("local:{name}"),
                group_id: String::new(),
                artifact_id: name,
                version: "0.0.0".into(),
                dir: dir.to_path_buf(),
                main_src_dirs: vec![dir.join("src/main/java")],
                test_src_dirs: vec![dir.join("src/test/java")],
                resource_dirs: vec![dir.join("src/main/resources")],
                dependencies: Vec::new(),
            },
        );
    }

    for sub in subdirs_to_visit {
        let c = fs::canonicalize(&sub).unwrap_or(sub);
        scan_java_dir_recursive(&c, depth + 1, visited, graph);
    }
}

fn register_module(graph: &mut JavaWorkspaceGraph, module: WorkspaceJavaModule) {
    let idx = graph.modules.len();
    if !module.group_id.is_empty() && !module.artifact_id.is_empty() {
        graph
            .by_coord
            .insert((module.group_id.clone(), module.artifact_id.clone()), idx);
    }
    if !module.artifact_id.is_empty() {
        graph
            .by_artifact
            .entry(module.artifact_id.clone())
            .or_insert(idx);
    }
    graph.modules.push(module);
}

pub fn parse_pom_module(dir: &Path, xml: &str) -> (WorkspaceJavaModule, Vec<String>) {
    let stripped = strip_xml_comments(xml);
    let props = parse_pom_properties(&stripped, dir);
    let parent_block = extract_first_tag_block(&stripped, "parent").unwrap_or("");
    let parent_group = extract_tag_text(parent_block, "groupId").unwrap_or_default();
    let parent_ver = extract_tag_text(parent_block, "version").unwrap_or_default();

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
    let raw_group = extract_tag_text(&header, "groupId").unwrap_or(parent_group);
    let raw_artifact = extract_tag_text(&header, "artifactId").unwrap_or_else(|| {
        dir.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("app")
            .to_string()
    });
    let raw_ver = extract_tag_text(&header, "version").unwrap_or(parent_ver);

    let group_id = resolve_pom_props(&raw_group, &props);
    let artifact_id = resolve_pom_props(&raw_artifact, &props);
    let version = resolve_pom_props(&raw_ver, &props);

    let mut submodules = Vec::new();
    if let Some(mods_block) = extract_first_tag_block(&header, "modules") {
        let mod_re = Regex::new(r"<module>\s*([^<\s]+)\s*</module>").unwrap();
        for cap in mod_re.captures_iter(mods_block) {
            submodules.push(cap[1].trim().to_string());
        }
    }

    let without_dm = remove_all_tag_blocks(&stripped, "dependencyManagement");
    let mut dependencies = Vec::new();
    let dep_re = Regex::new(r"(?s)<dependency>(.*?)</dependency>").unwrap();
    for cap in dep_re.captures_iter(&without_dm) {
        let body = &cap[1];
        let body_no_excl = remove_all_tag_blocks(body, "exclusions");
        let g = resolve_pom_props(
            &extract_tag_text(&body_no_excl, "groupId").unwrap_or_default(),
            &props,
        );
        let a = resolve_pom_props(
            &extract_tag_text(&body_no_excl, "artifactId").unwrap_or_default(),
            &props,
        );
        let v = extract_tag_text(&body_no_excl, "version")
            .map(|s| resolve_pom_props(&s, &props))
            .filter(|s| !s.is_empty() && !s.starts_with("${"));
        if !a.is_empty() {
            dependencies.push(JavaDepCoord {
                group_id: g,
                artifact_id: a,
                version: v,
            });
        }
    }

    let key = format!("{group_id}:{artifact_id}");
    (
        WorkspaceJavaModule {
            key,
            group_id,
            artifact_id,
            version,
            dir: dir.to_path_buf(),
            main_src_dirs: vec![dir.join("src/main/java")],
            test_src_dirs: vec![dir.join("src/test/java")],
            resource_dirs: vec![
                dir.join("src/main/resources"),
                dir.join("src/test/resources"),
            ],
            dependencies,
        },
        submodules,
    )
}

pub fn parse_pom_properties(stripped_xml: &str, dir: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(parent_block) = extract_first_tag_block(stripped_xml, "parent") {
        let rel = extract_tag_text(parent_block, "relativePath")
            .unwrap_or_else(|| "../pom.xml".to_string());
        let parent_pom = if rel.ends_with(".xml") {
            dir.join(rel)
        } else {
            dir.join(rel).join("pom.xml")
        };
        if let Ok(praw) = fs::read_to_string(&parent_pom) {
            let pstrip = strip_xml_comments(&praw);
            if let Some(pblock) = extract_first_tag_block(&pstrip, "properties") {
                extract_kv_tags(pblock, &mut map);
            }
        }
    }
    if let Some(props_block) = extract_first_tag_block(stripped_xml, "properties") {
        extract_kv_tags(props_block, &mut map);
    }
    map
}

fn extract_kv_tags(block: &str, out: &mut HashMap<String, String>) {
    let re = Regex::new(r"<([A-Za-z0-9_.-]+)>\s*([^<]*?)\s*</([A-Za-z0-9_.-]+)>").unwrap();
    for cap in re.captures_iter(block) {
        if cap[1] == cap[3] {
            out.insert(cap[1].to_string(), cap[2].trim().to_string());
        }
    }
}

pub fn resolve_pom_props(val: &str, props: &HashMap<String, String>) -> String {
    let mut cur = val.trim().to_string();
    for _ in 0..3 {
        let Some(start) = cur.find("${") else {
            break;
        };
        let Some(rel_end) = cur[start + 2..].find('}') else {
            break;
        };
        let key = &cur[start + 2..start + 2 + rel_end];
        if let Some(rep) = props.get(key) {
            cur = format!("{}{}{}", &cur[..start], rep, &cur[start + 3 + rel_end..]);
        } else {
            break;
        }
    }
    cur
}

fn parse_gradle_module(dir: &Path, raw: &str) -> WorkspaceJavaModule {
    let group_re = Regex::new(r#"(?m)^\s*group\s*=?\s*['"]([^'"]+)['"]"#).unwrap();
    let ver_re = Regex::new(r#"(?m)^\s*version\s*=?\s*['"]([^'"]+)['"]"#).unwrap();
    let group_id = group_re
        .captures(raw)
        .map(|c| c[1].trim().to_string())
        .unwrap_or_default();
    let version = ver_re
        .captures(raw)
        .map(|c| c[1].trim().to_string())
        .unwrap_or_else(|| "0.0.0".into());
    let artifact_id = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("app")
        .to_string();

    let mut dependencies = Vec::new();
    let coord_re = Regex::new(
        r#"(?:implementation|api|compileOnly|runtimeOnly|testImplementation)\s*\(?\s*['"]([^:'"]+):([^:'"]+)(?::([^'"]+))?['"]"#,
    )
    .unwrap();
    for cap in coord_re.captures_iter(raw) {
        dependencies.push(JavaDepCoord {
            group_id: cap[1].to_string(),
            artifact_id: cap[2].to_string(),
            version: cap.get(3).map(|m| m.as_str().to_string()),
        });
    }
    let proj_re = Regex::new(r#"project\s*\(\s*['"]:([^'"]+)['"]\s*\)"#).unwrap();
    for cap in proj_re.captures_iter(raw) {
        let sub = cap[1].rsplit(':').next().unwrap_or(&cap[1]).to_string();
        dependencies.push(JavaDepCoord {
            group_id: group_id.clone(),
            artifact_id: sub,
            version: None,
        });
    }

    WorkspaceJavaModule {
        key: format!("{group_id}:{artifact_id}"),
        group_id,
        artifact_id,
        version,
        dir: dir.to_path_buf(),
        main_src_dirs: vec![dir.join("src/main/java")],
        test_src_dirs: vec![dir.join("src/test/java")],
        resource_dirs: vec![dir.join("src/main/resources")],
        dependencies,
    }
}

fn parse_gradle_includes(raw: &str) -> Vec<String> {
    let re = Regex::new(r#"include\s*\(?([^)\n]+)\)?"#).unwrap();
    let q_re = Regex::new(r#"['"]([^'"]+)['"]"#).unwrap();
    let mut out = Vec::new();
    for cap in re.captures_iter(raw) {
        for q in q_re.captures_iter(&cap[1]) {
            out.push(q[1].to_string());
        }
    }
    out
}

pub fn strip_xml_comments(xml: &str) -> String {
    let re = Regex::new(r"(?s)<!--.*?-->").unwrap();
    re.replace_all(xml, |caps: &regex::Captures<'_>| {
        caps[0]
            .chars()
            .map(|c| if c == '\n' { '\n' } else { ' ' })
            .collect::<String>()
    })
    .into_owned()
}

pub fn extract_first_tag_block<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find(&close)?;
    Some(&xml[start..end])
}

pub fn extract_tag_text(xml: &str, tag: &str) -> Option<String> {
    extract_first_tag_block(xml, tag).map(|s| s.trim().to_string())
}

pub fn remove_all_tag_blocks(xml: &str, tag: &str) -> String {
    let pat = format!(r"(?s)<{tag}>.*?</{tag}>");
    let Ok(re) = Regex::new(&pat) else {
        return xml.to_string();
    };
    re.replace_all(xml, "").into_owned()
}

/// Resolve the active local Maven repository directory (`MAVEN_REPO_LOCAL` -> `M2_REPO` -> `~/.m2/repository`).
pub fn resolve_m2_repo(env_map: &BTreeMap<String, String>) -> Option<PathBuf> {
    if let Some(custom) = env_map
        .get("MAVEN_REPO_LOCAL")
        .or_else(|| env_map.get("M2_REPO"))
        .filter(|s| !s.trim().is_empty())
    {
        return Some(PathBuf::from(custom.trim()));
    }
    if let Ok(custom) = std::env::var("MAVEN_REPO_LOCAL") {
        if !custom.trim().is_empty() {
            return Some(PathBuf::from(custom.trim()));
        }
    }
    dirs::home_dir().map(|h| h.join(".m2/repository"))
}

/// Locate a dependency `.jar` (or `-sources.jar` when `sources == true`) in `.m2` or `~/.gradle`.
pub fn resolve_external_dep_jar(
    dep: &JavaDepCoord,
    m2_repo: Option<&Path>,
    sources: bool,
) -> Option<PathBuf> {
    let suffix = if sources { "-sources.jar" } else { ".jar" };
    if let Some(m2) = m2_repo {
        let group_rel = dep.group_id.replace('.', "/");
        let art_base = m2.join(&group_rel).join(&dep.artifact_id);
        if let Some(ver) = dep.version.as_deref().filter(|v| !v.is_empty()) {
            let exact = art_base
                .join(ver)
                .join(format!("{}-{ver}{suffix}", dep.artifact_id));
            if exact.is_file() {
                return Some(exact);
            }
        }
        if let Ok(rd) = fs::read_dir(&art_base) {
            let mut versions: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            versions.sort();
            for vdir in versions.into_iter().rev() {
                let Some(ver) = vdir.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                let cand = vdir.join(format!("{}-{ver}{suffix}", dep.artifact_id));
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
    }
    if let Some(gradle_base) = dirs::home_dir().map(|h| {
        h.join(".gradle/caches/modules-2/files-2.1")
            .join(&dep.group_id)
            .join(&dep.artifact_id)
    }) {
        if let Ok(vrd) = fs::read_dir(&gradle_base) {
            let mut vdirs: Vec<PathBuf> = vrd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            vdirs.sort();
            for vdir in vdirs.into_iter().rev() {
                if let Ok(hrd) = fs::read_dir(&vdir) {
                    for hentry in hrd.flatten() {
                        if let Ok(frd) = fs::read_dir(hentry.path()) {
                            for fentry in frd.flatten() {
                                let fname = fentry.file_name().to_string_lossy().into_owned();
                                let matches = if sources {
                                    fname.ends_with("-sources.jar")
                                } else {
                                    fname.ends_with(".jar")
                                        && !fname.ends_with("-sources.jar")
                                        && !fname.ends_with("-javadoc.jar")
                                };
                                if matches {
                                    return Some(fentry.path());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}
