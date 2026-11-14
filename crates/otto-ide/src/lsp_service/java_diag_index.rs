//! On-demand Java workspace/JAR symbol scanning helpers for `java_diag`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::java_workspace::{
    read_jar_classes_and_packages, resolve_external_dep_jar, resolve_m2_repo, JavaWorkspaceGraph,
};

use super::java_diag::KNOWN_LOMBOK_CLASSES;

#[allow(clippy::disallowed_methods)]
pub(super) fn collect_same_package_types(
    file_path: &str,
    current_pkg: &str,
    ws: &JavaWorkspaceGraph,
) -> HashSet<String> {
    let mut out = HashSet::new();
    if let Some(parent) = Path::new(file_path).parent() {
        if let Ok(rd) = std::fs::read_dir(parent) {
            for e in rd.flatten() {
                if let Some(stem) = e.path().file_stem().and_then(|s| s.to_str()).filter(|_| {
                    matches!(
                        e.path().extension().and_then(|x| x.to_str()),
                        Some("java" | "kt")
                    )
                }) {
                    out.insert(stem.to_string());
                }
            }
        }
    }
    if !current_pkg.is_empty() {
        let rel = current_pkg.replace('.', "/");
        for sr in ws.all_source_roots(true) {
            if let Ok(rd) = std::fs::read_dir(sr.join(&rel)) {
                for e in rd.flatten() {
                    if let Some(stem) = e.path().file_stem().and_then(|s| s.to_str()) {
                        out.insert(stem.to_string());
                    }
                }
            }
        }
    }
    out
}

#[allow(clippy::disallowed_methods)]
pub(super) fn collect_external_jar_symbols(
    ws: &JavaWorkspaceGraph,
    env_map: &std::collections::BTreeMap<String, String>,
) -> (
    HashSet<String>,
    HashSet<String>,
    HashMap<String, Vec<String>>,
) {
    let mut classes = HashSet::new();
    let mut packages = HashSet::new();
    let mut by_simple: HashMap<String, Vec<String>> = HashMap::new();
    let m2_repo = resolve_m2_repo(env_map);
    let mut seen_jars = HashSet::new();

    for module in &ws.modules {
        for rel in [
            "target/classes",
            "target/test-classes",
            "build/classes/java/main",
        ] {
            let cdir = module.dir.join(rel);
            if cdir.is_dir() {
                collect_compiled_dir_classes(&cdir, &cdir, &mut classes, &mut packages);
            }
        }
        for dep in &module.dependencies {
            if ws.find_dep(&dep.group_id, &dep.artifact_id).is_some() {
                continue;
            }
            if let Some(jar) = resolve_external_dep_jar(dep, m2_repo.as_deref(), false) {
                if seen_jars.insert(jar.clone()) {
                    let (jc, jp) = read_jar_classes_and_packages(&jar);
                    for fqn in jc {
                        if let Some(simple) = fqn.rsplit('.').next() {
                            by_simple
                                .entry(simple.to_string())
                                .or_default()
                                .push(fqn.clone());
                        }
                        classes.insert(fqn);
                    }
                    packages.extend(jp);
                }
            }
        }
    }
    (classes, packages, by_simple)
}

#[allow(clippy::disallowed_methods)]
fn collect_compiled_dir_classes(
    base: &Path,
    cur: &Path,
    classes: &mut HashSet<String>,
    packages: &mut HashSet<String>,
) {
    let Ok(rd) = std::fs::read_dir(cur) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_compiled_dir_classes(base, &p, classes, packages);
        } else if p.extension().and_then(|x| x.to_str()) == Some("class") {
            if let Ok(rel) = p.strip_prefix(base) {
                let s = rel.to_string_lossy().replace('\\', "/");
                if let Some(no_ext) = s.strip_suffix(".class") {
                    if let Some((pkg, _)) = no_ext.rsplit_once('/') {
                        packages.insert(pkg.replace('/', "."));
                    }
                    classes.insert(no_ext.replace(['/', '$'], "."));
                }
            }
        }
    }
}

#[allow(clippy::disallowed_methods)]
pub(super) fn index_workspace_simple_classes(
    ws: &JavaWorkspaceGraph,
) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for sr in ws.all_source_roots(true) {
        let walker = ignore::WalkBuilder::new(&sr).max_depth(Some(10)).build();
        for entry in walker.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            if !matches!(p.extension().and_then(|e| e.to_str()), Some("java" | "kt")) {
                continue;
            }
            let Ok(rel) = p.strip_prefix(&sr) else {
                continue;
            };
            let rel_no_ext = rel.with_extension("");
            let fqn = rel_no_ext.to_string_lossy().replace(['/', '\\'], ".");
            if let Some(simple) = p.file_stem().and_then(|s| s.to_str()) {
                let vec = map.entry(simple.to_string()).or_default();
                if !vec.contains(&fqn) {
                    vec.push(fqn);
                }
            }
        }
    }
    map
}

pub(crate) fn find_import_candidates_by_simple(
    simple: &str,
    ws_by_simple: &HashMap<String, Vec<String>>,
    ext_by_simple: &HashMap<String, Vec<String>>,
    jdk: &crate::java_workspace::JdkClassIndex,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(ws_list) = ws_by_simple.get(simple) {
        for fqn in ws_list {
            if !out.contains(fqn) {
                out.push(fqn.clone());
            }
        }
    }
    for &lombok_fqn in KNOWN_LOMBOK_CLASSES {
        if lombok_fqn.rsplit('.').next() == Some(simple) && !out.contains(&lombok_fqn.to_string()) {
            out.push(lombok_fqn.to_string());
        }
    }
    if let Some(jdk_list) = jdk.by_simple.get(simple) {
        for fqn in jdk_list {
            if !fqn.starts_with("java.lang.") && !out.contains(fqn) {
                out.push(fqn.clone());
            }
        }
    }
    if let Some(ext_list) = ext_by_simple.get(simple) {
        for fqn in ext_list {
            if !out.contains(fqn) {
                out.push(fqn.clone());
            }
        }
    }
    out.truncate(5);
    out
}

pub(crate) fn expected_package_from_path(path: &str) -> Option<String> {
    let norm = path.replace('\\', "/");
    for marker in [
        "/src/main/java/",
        "/src/test/java/",
        "/src/main/kotlin/",
        "/src/test/kotlin/",
        "src/main/java/",
        "src/test/java/",
    ] {
        if let Some((_, tail)) = norm.rsplit_once(marker) {
            let (pkg_dir, _) = tail.rsplit_once('/')?;
            if !pkg_dir.is_empty() {
                return Some(pkg_dir.replace('/', "."));
            }
        }
    }
    None
}
