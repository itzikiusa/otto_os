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
    collect_java_files_rec, infer_java_fqcn_from_file, prepare_java_execution,
    prepare_java_execution_cancellable, JavaExecutionPlan,
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

/// Validate and canonicalize a candidate directory or manifest file referenced
/// from Maven (`<module>`, `<relativePath>`) or Gradle (`settings.gradle` `include` / `projectDir`).
///
/// Rejects `otto_core::secret_paths` unconditionally, and confines paths strictly
/// within `allowed_roots` whenever `!caller_is_root` or `strict_roots` is set.
pub fn confine_manifest_path(
    cand: &Path,
    allowed_roots: &[PathBuf],
    caller_is_root: bool,
    must_be_dir: bool,
    strict_roots: bool,
) -> Option<PathBuf> {
    let canon = fs::canonicalize(cand).ok()?;
    if must_be_dir {
        if !canon.is_dir() {
            return None;
        }
    } else if !canon.is_file() {
        return None;
    }
    if otto_core::secret_paths::in_protected_dir(&canon)
        || otto_core::secret_paths::is_denied_file(&canon)
        || otto_core::secret_paths::is_denied_dir(&canon)
    {
        return None;
    }
    if !caller_is_root && crate::target::enforce_non_root_path(&canon).is_err() {
        return None;
    }
    if (!caller_is_root || strict_roots)
        && (allowed_roots.is_empty() || !allowed_roots.iter().any(|r| canon.starts_with(r)))
    {
        return None;
    }
    Some(canon)
}

/// Resolve and canonicalize a Maven `<parent><relativePath>` POM file, enforcing
/// `allowed_roots` confinement when `!caller_is_root` and rejecting `secret_paths`.
pub fn resolve_confined_parent_pom(
    stripped_xml: &str,
    dir: &Path,
    allowed_roots: &[PathBuf],
    caller_is_root: bool,
) -> Option<PathBuf> {
    let parent_block = extract_first_tag_block(stripped_xml, "parent")?;
    let rel = match extract_tag_text(parent_block, "relativePath") {
        Some(s) if s.is_empty() => return None,
        Some(s) => s,
        None => "../pom.xml".to_string(),
    };
    let parent_pom = if rel.ends_with(".xml") || rel.ends_with(".pom") {
        dir.join(rel)
    } else {
        dir.join(rel).join("pom.xml")
    };
    confine_manifest_path(&parent_pom, allowed_roots, caller_is_root, false, false)
}

/// Scan all attached IDE project folders (and Maven `<modules>` / Gradle subprojects)
/// to build a `JavaWorkspaceGraph`. Submodules and Gradle includes are strictly
/// confined within `roots`.
pub fn scan_java_workspace(roots: &[PathBuf]) -> JavaWorkspaceGraph {
    scan_java_workspace_scoped(roots, false)
}

/// Scoped variant of [`scan_java_workspace`] carrying `caller_is_root` for
/// `<relativePath>` parent-POM property resolution while always confining
/// `<module>` and Gradle `include` / `projectDir` subprojects within `roots`.
pub fn scan_java_workspace_scoped(roots: &[PathBuf], caller_is_root: bool) -> JavaWorkspaceGraph {
    let mut graph = JavaWorkspaceGraph::default();
    let mut canon_roots = Vec::new();
    for root in roots {
        if let Ok(canon) = fs::canonicalize(root) {
            if !caller_is_root && crate::target::enforce_non_root_path(&canon).is_err() {
                continue;
            }
            if !canon_roots.contains(&canon) {
                canon_roots.push(canon);
            }
        }
    }
    let mut visited_dirs = HashSet::new();
    for canon in &canon_roots {
        scan_java_dir_recursive(
            canon,
            0,
            &canon_roots,
            caller_is_root,
            &mut visited_dirs,
            &mut graph,
        );
    }
    graph
}

fn scan_java_dir_recursive(
    dir: &Path,
    depth: usize,
    canon_roots: &[PathBuf],
    caller_is_root: bool,
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
            let (module, submodules) =
                parse_pom_module_scoped(dir, &raw, canon_roots, caller_is_root);
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
        if let Some(c) = confine_manifest_path(&sub, canon_roots, caller_is_root, true, true) {
            scan_java_dir_recursive(&c, depth + 1, canon_roots, caller_is_root, visited, graph);
        }
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
    let default_root = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    parse_pom_module_scoped(dir, xml, &[default_root], false)
}

pub fn parse_pom_module_scoped(
    dir: &Path,
    xml: &str,
    allowed_roots: &[PathBuf],
    caller_is_root: bool,
) -> (WorkspaceJavaModule, Vec<String>) {
    let stripped = strip_xml_comments(xml);
    let props = parse_pom_properties_scoped(&stripped, dir, allowed_roots, caller_is_root);
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
    let default_root = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    parse_pom_properties_scoped(stripped_xml, dir, &[default_root], false)
}

pub fn parse_pom_properties_scoped(
    stripped_xml: &str,
    dir: &Path,
    allowed_roots: &[PathBuf],
    caller_is_root: bool,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(canon_parent) =
        resolve_confined_parent_pom(stripped_xml, dir, allowed_roots, caller_is_root)
    {
        if let Ok(praw) = fs::read_to_string(&canon_parent) {
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
    let pdir_re = Regex::new(r#"projectDir\s*=\s*file\s*\(\s*['"]([^'"]+)['"]\s*\)"#).unwrap();
    let mut out = Vec::new();
    for cap in re.captures_iter(raw) {
        for q in q_re.captures_iter(&cap[1]) {
            out.push(q[1].to_string());
        }
    }
    for cap in pdir_re.captures_iter(raw) {
        out.push(cap[1].to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_parent_relative_path_and_modules_and_gradle_includes_stay_confined() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside-secret");
        let ws_maven = tmp.path().join("ws-maven");
        let child_maven = ws_maven.join("sub");
        let ws_gradle = tmp.path().join("ws-gradle");
        let valid_sub = ws_gradle.join("valid-sub");

        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(&child_maven).unwrap();
        fs::create_dir_all(&valid_sub).unwrap();

        fs::write(
            outside.join("pom.xml"),
            "<project><groupId>leak</groupId><artifactId>outside-mod</artifactId><properties><secret.ver>9.9.9-LEAKED</secret.ver></properties></project>",
        )
        .unwrap();
        fs::write(
            outside.join("build.gradle"),
            "group = 'leak'\nversion = '9.9.9'\n",
        )
        .unwrap();

        fs::write(
            ws_maven.join("pom.xml"),
            "<project><groupId>acme</groupId><artifactId>parent</artifactId><version>1.0.0</version><properties><allowed.ver>2.4.0</allowed.ver></properties><modules><module>sub</module><module>../outside-secret</module></modules></project>",
        )
        .unwrap();
        let child_xml = r#"<project>
            <parent>
                <groupId>acme</groupId>
                <artifactId>parent</artifactId>
                <version>1.0.0</version>
                <relativePath>../../outside-secret/pom.xml</relativePath>
            </parent>
            <artifactId>sub</artifactId>
            <version>${secret.ver}</version>
        </project>"#;
        fs::write(child_maven.join("pom.xml"), child_xml).unwrap();

        let canon_ws_maven = fs::canonicalize(&ws_maven).unwrap();
        let maven_graph = scan_java_workspace_scoped(std::slice::from_ref(&canon_ws_maven), false);
        assert!(
            maven_graph
                .modules
                .iter()
                .all(|m| m.artifact_id != "outside-mod"),
            "malicious <module>../outside-secret</module> must not be scanned: {:?}",
            maven_graph.modules
        );
        let sub_mod = maven_graph
            .modules
            .iter()
            .find(|m| m.artifact_id == "sub")
            .expect("valid child module present");
        assert_ne!(
            sub_mod.version, "9.9.9-LEAKED",
            "malicious <relativePath> outside allowed_roots must not leak parent properties"
        );

        // Legitimate parent inside allowed_roots resolves cleanly for non-root callers.
        let valid_child_xml = r#"<project>
            <parent><relativePath>../pom.xml</relativePath></parent>
            <artifactId>sub</artifactId>
        </project>"#;
        let props = parse_pom_properties_scoped(
            valid_child_xml,
            &fs::canonicalize(&child_maven).unwrap(),
            std::slice::from_ref(&canon_ws_maven),
            false,
        );
        assert_eq!(props.get("allowed.ver").map(String::as_str), Some("2.4.0"));

        // Gradle settings.gradle include / projectDir escape must be blocked.
        fs::write(
            ws_gradle.join("build.gradle"),
            "group = 'acme'\nversion = '1.0.0'\n",
        )
        .unwrap();
        fs::write(
            ws_gradle.join("settings.gradle"),
            "include(':valid-sub', ':../outside-secret')\nproject(':evil').projectDir = file('../outside-secret')\n",
        )
        .unwrap();
        fs::write(
            valid_sub.join("build.gradle"),
            "group = 'acme'\nversion = '1.0.0'\n",
        )
        .unwrap();

        let canon_ws_gradle = fs::canonicalize(&ws_gradle).unwrap();
        let gradle_graph =
            scan_java_workspace_scoped(std::slice::from_ref(&canon_ws_gradle), false);
        let gradle_ids: Vec<&str> = gradle_graph
            .modules
            .iter()
            .map(|m| m.artifact_id.as_str())
            .collect();
        assert!(gradle_ids.contains(&"valid-sub"));
        assert!(
            !gradle_ids.contains(&"outside-secret"),
            "Gradle settings include/projectDir outside workspace root must be ignored: {gradle_ids:?}"
        );
    }
}
