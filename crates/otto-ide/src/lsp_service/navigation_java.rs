//! External Java Maven/Gradle JAR & JDK `src.zip` definition lookup helpers
//! for [`super::navigation_ext`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::java_workspace::{
    read_jar_classes_and_packages, resolve_external_dep_jar, resolve_m2_repo, scan_java_workspace,
};
use crate::types::IdeReferenceLocation;

use super::navigation::{find_definitions_in_content, is_confined_nav_path};

/// Extract candidate Java FQNs (`com.foo.Bar`) for `target_class` from `package` + `import` lines.
pub(super) fn candidate_java_fqns_in_file(content: &str, target_class: &str) -> Vec<String> {
    let mut fqns = Vec::new();
    let mut pkg_name = None;
    for raw in content.lines() {
        let t = raw.split("//").next().unwrap_or("").trim();
        if let Some(p) = t.strip_prefix("package ").and_then(|s| s.strip_suffix(';')) {
            pkg_name = Some(p.trim().to_string());
        } else if let Some(imp) = t.strip_prefix("import ").and_then(|s| s.strip_suffix(';')) {
            let imp = imp.trim().trim_start_matches("static ").trim();
            if imp.ends_with(&format!(".{target_class}")) {
                fqns.push(imp.to_string());
            } else if let Some(pkg) = imp.strip_suffix(".*") {
                fqns.push(format!("{pkg}.{target_class}"));
            }
        }
    }
    if let Some(p) = pkg_name.filter(|p| !p.is_empty()) {
        let same_pkg = format!("{p}.{target_class}");
        if !fqns.contains(&same_pkg) {
            fqns.push(same_pkg);
        }
    }
    if target_class
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
    {
        let jl = format!("java.lang.{target_class}");
        if !fqns.contains(&jl) {
            fqns.push(jl);
        }
    }
    fqns
}

/// Resolve external Java Maven/Gradle `*-sources.jar`/`.jar` or JDK `src.zip` class/method
/// definition, returning a virtual `jar:file:///...!/...` path readable via `TargetManager::read_file`.
pub(super) fn resolve_external_java_definition(
    content: &str,
    symbol: &str,
    qualifier: Option<&str>,
    project_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    env_map: &BTreeMap<String, String>,
    is_root_user: bool,
) -> Vec<IdeReferenceLocation> {
    let target_class = qualifier.unwrap_or(symbol);
    let candidate_fqns = candidate_java_fqns_in_file(content, target_class);
    if candidate_fqns.is_empty() {
        return Vec::new();
    }

    let ws = scan_java_workspace(project_roots);
    let m2_repo = resolve_m2_repo(env_map);
    let mut allowed_ro = readonly_roots.to_vec();
    if let Some(ref m2) = m2_repo {
        if let Ok(canon_m2) = m2.canonicalize() {
            if is_root_user || crate::target::enforce_non_root_path(&canon_m2).is_ok() {
                allowed_ro.push(canon_m2);
            }
        }
    }

    for module in &ws.modules {
        for dep in &module.dependencies {
            if ws.find_dep(&dep.group_id, &dep.artifact_id).is_some() {
                continue;
            }
            if let Some(src_jar) = resolve_external_dep_jar(dep, m2_repo.as_deref(), true) {
                if let Ok(canon_jar) = src_jar.canonicalize() {
                    if is_confined_nav_path(&canon_jar, project_roots, &allowed_ro, is_root_user) {
                        for fqn in &candidate_fqns {
                            let entry = format!("{}.java", fqn.replace('.', "/"));
                            if let Some(src_text) = unzip_single_entry(&canon_jar, &entry) {
                                let uri =
                                    format!("jar:file://{}!/{entry}", canon_jar.to_string_lossy());
                                let defs =
                                    find_definitions_in_content(&uri, "java", &src_text, symbol);
                                if !defs.is_empty() {
                                    return defs;
                                }
                                return vec![IdeReferenceLocation {
                                    path: uri,
                                    line: 1,
                                    column: 1,
                                    preview: format!("class {target_class}"),
                                }];
                            }
                        }
                    }
                }
            }
            if let Some(bin_jar) = resolve_external_dep_jar(dep, m2_repo.as_deref(), false) {
                if let Ok(canon_jar) = bin_jar.canonicalize() {
                    if is_confined_nav_path(&canon_jar, project_roots, &allowed_ro, is_root_user) {
                        let (classes, _) = read_jar_classes_and_packages(&canon_jar);
                        for fqn in &candidate_fqns {
                            if classes.contains(fqn) {
                                let entry = format!("{}.class", fqn.replace('.', "/"));
                                return vec![IdeReferenceLocation {
                                    path: format!(
                                        "jar:file://{}!/{entry}",
                                        canon_jar.to_string_lossy()
                                    ),
                                    line: 1,
                                    column: 1,
                                    preview: format!("class {fqn}"),
                                }];
                            }
                        }
                    }
                }
            }
        }
    }

    let mut src_zips = Vec::new();
    if let Some(jh) = env_map.get("JAVA_HOME").filter(|s| !s.is_empty()) {
        let p = PathBuf::from(jh);
        for rel in ["lib/src.zip", "src.zip", "Contents/Home/lib/src.zip"] {
            if p.join(rel).is_file() {
                src_zips.push(p.join(rel));
            }
        }
    }
    for z in crate::target::sdk_paths::discover_jdk_src_zips(readonly_roots) {
        if !src_zips.contains(&z) {
            src_zips.push(z);
        }
    }

    let mut out = Vec::new();
    for fqn in &candidate_fqns {
        let rel_java = format!("{}.java", fqn.replace('.', "/"));
        for zip in &src_zips {
            let Ok(canon_zip) = zip.canonicalize() else {
                continue;
            };
            if !is_confined_nav_path(&canon_zip, project_roots, &allowed_ro, is_root_user) {
                continue;
            }
            for entry in [
                format!("java.base/{rel_java}"),
                format!("java.logging/{rel_java}"),
                format!("java.sql/{rel_java}"),
                format!("java.net.http/{rel_java}"),
                rel_java.clone(),
            ] {
                if let Some(src_text) = unzip_single_entry(&canon_zip, &entry) {
                    let uri = format!("jar:file://{}!/{entry}", canon_zip.to_string_lossy());
                    let defs = find_definitions_in_content(&uri, "java", &src_text, symbol);
                    if !defs.is_empty() {
                        return defs;
                    }
                    out.push(IdeReferenceLocation {
                        path: uri,
                        line: 1,
                        column: 1,
                        preview: format!("class {target_class}"),
                    });
                    return out;
                }
            }
        }
    }
    out
}

#[allow(clippy::disallowed_methods)]
fn unzip_single_entry(archive: &Path, entry: &str) -> Option<String> {
    let mut cmd = Command::new("/usr/bin/unzip");
    cmd.env_clear()
        .envs(crate::environments::shell_env::collect_safe_non_root_base_env())
        .arg("-p")
        .arg(archive)
        .arg(entry);
    let out = cmd.output().ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}
