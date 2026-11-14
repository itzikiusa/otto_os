//! Multi-module Java source compilation (`javac -g`) and `-classpath` assembly
//! prioritizing in-project workspace modules ahead of `~/.m2` / Gradle JARs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use otto_core::{Error, Result};
use regex::Regex;

use super::{
    parse_pom_module, resolve_external_dep_jar, resolve_m2_repo, scan_java_workspace,
    JavaWorkspaceGraph, WorkspaceJavaModule,
};

#[derive(Debug, Clone)]
pub struct JavaExecutionPlan {
    pub main_class: String,
    pub classpath: String,
    pub source_roots: Vec<PathBuf>,
    pub compiler_output: String,
}

/// Compile in-project workspace modules FIRST (`javac -g`) and assemble `-classpath`
/// with in-project classes BEFORE `.m2` / Gradle JARs.
pub fn prepare_java_execution(
    target_file_or_dir: &Path,
    explicit_main_class: Option<&str>,
    include_tests: bool,
    project_roots: &[PathBuf],
    env_map: &BTreeMap<String, String>,
    javac_bin: &str,
    out_base_dir: &Path,
) -> Result<JavaExecutionPlan> {
    let cp_sep = if cfg!(windows) { ";" } else { ":" };
    let graph = scan_java_workspace(project_roots);
    let m2_repo = resolve_m2_repo(env_map);
    fs::create_dir_all(out_base_dir)
        .map_err(|e| Error::Internal(format!("create java build dir: {e}")))?;

    let mut in_project_cp: Vec<String> = Vec::new();
    let mut external_jars: Vec<String> = Vec::new();
    let mut source_roots: Vec<PathBuf> = Vec::new();
    let mut compiler_logs: Vec<String> = Vec::new();

    if let Some(owner) = graph.owning_module(target_file_or_dir) {
        let mut compiled_mods: HashMap<String, PathBuf> = HashMap::new();
        let mut visiting: HashSet<String> = HashSet::new();
        compile_dep_tree(
            owner,
            &graph,
            m2_repo.as_deref(),
            javac_bin,
            env_map,
            out_base_dir,
            cp_sep,
            &mut visiting,
            &mut compiled_mods,
            &mut in_project_cp,
            &mut external_jars,
            &mut source_roots,
            &mut compiler_logs,
        )?;

        let owner_out = out_base_dir.join(sanitize_slug(&owner.key));
        fs::create_dir_all(&owner_out)
            .map_err(|e| Error::Internal(format!("create target module classes dir: {e}")))?;
        let mut owner_srcs = Vec::new();
        for sdir in &owner.main_src_dirs {
            if sdir.is_dir() {
                source_roots.push(sdir.clone());
                collect_java_files_rec(sdir, &mut owner_srcs);
            }
        }
        let target_is_test = target_file_or_dir
            .to_string_lossy()
            .contains("/src/test/java");
        if include_tests || target_is_test {
            for tdir in &owner.test_src_dirs {
                if tdir.is_dir() {
                    source_roots.push(tdir.clone());
                    collect_java_files_rec(tdir, &mut owner_srcs);
                }
            }
        }
        if target_file_or_dir.is_file()
            && target_file_or_dir.extension().and_then(|s| s.to_str()) == Some("java")
            && !owner_srcs.iter().any(|p| same_file(p, target_file_or_dir))
        {
            owner_srcs.push(target_file_or_dir.to_path_buf());
        }

        let mut build_cp: Vec<String> = Vec::new();
        build_cp.extend(in_project_cp.iter().cloned());
        build_cp.extend(external_jars.iter().cloned());
        for existing_rel in ["target/classes", "build/classes/java/main"] {
            let ex = owner.dir.join(existing_rel);
            if ex.is_dir() {
                build_cp.push(ex.to_string_lossy().into_owned());
            }
        }
        if !owner_srcs.is_empty() {
            run_javac(
                javac_bin,
                &owner_out,
                &build_cp.join(cp_sep),
                &owner_srcs,
                env_map,
                &mut compiler_logs,
            )?;
        }

        let mut final_cp = vec![owner_out.to_string_lossy().into_owned()];
        for rdir in &owner.resource_dirs {
            if rdir.is_dir() {
                final_cp.push(rdir.to_string_lossy().into_owned());
            }
        }
        final_cp.extend(in_project_cp);
        final_cp.extend(external_jars);
        for existing_rel in [
            "target/classes",
            "target/test-classes",
            "build/classes/java/main",
        ] {
            let ex = owner.dir.join(existing_rel);
            if ex.is_dir() {
                let s = ex.to_string_lossy().into_owned();
                if !final_cp.contains(&s) {
                    final_cp.push(s);
                }
            }
        }

        let main_class = resolve_main_class(explicit_main_class, target_file_or_dir, &owner_srcs)?;
        return Ok(JavaExecutionPlan {
            main_class,
            classpath: final_cp.join(cp_sep),
            source_roots,
            compiler_output: compiler_logs.join("\n"),
        });
    }

    // Standalone `.java` file / directory (no `pom.xml` or `build.gradle`)
    let target_file = if target_file_or_dir.is_file() {
        target_file_or_dir.to_path_buf()
    } else {
        let mut found = Vec::new();
        collect_java_files_rec(target_file_or_dir, &mut found);
        found.into_iter().next().ok_or_else(|| {
            Error::Invalid(format!(
                "no .java source files found in '{}'",
                target_file_or_dir.display()
            ))
        })?
    };
    let parent_dir = target_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    source_roots.push(parent_dir.clone());
    let mut peer_srcs = Vec::new();
    collect_java_files_shallow(&parent_dir, &mut peer_srcs);
    if !peer_srcs.iter().any(|p| same_file(p, &target_file)) {
        peer_srcs.push(target_file.clone());
    }
    let out_single = out_base_dir.join("__standalone");
    fs::create_dir_all(&out_single)
        .map_err(|e| Error::Internal(format!("create standalone classes dir: {e}")))?;
    run_javac(
        javac_bin,
        &out_single,
        "",
        &peer_srcs,
        env_map,
        &mut compiler_logs,
    )?;
    let main_class = resolve_main_class(explicit_main_class, &target_file, &peer_srcs)?;
    Ok(JavaExecutionPlan {
        main_class,
        classpath: out_single.to_string_lossy().into_owned(),
        source_roots,
        compiler_output: compiler_logs.join("\n"),
    })
}

#[allow(clippy::too_many_arguments)]
fn compile_dep_tree(
    module: &WorkspaceJavaModule,
    graph: &JavaWorkspaceGraph,
    m2_repo: Option<&Path>,
    javac_bin: &str,
    env_map: &BTreeMap<String, String>,
    out_base_dir: &Path,
    cp_sep: &str,
    visiting: &mut HashSet<String>,
    compiled_mods: &mut HashMap<String, PathBuf>,
    in_project_cp: &mut Vec<String>,
    external_jars: &mut Vec<String>,
    source_roots: &mut Vec<PathBuf>,
    compiler_logs: &mut Vec<String>,
) -> Result<()> {
    if !visiting.insert(module.key.clone()) {
        return Ok(());
    }
    for dep in &module.dependencies {
        if let Some(in_proj) = graph.find_dep(&dep.group_id, &dep.artifact_id) {
            if in_proj.dir != module.dir {
                compile_dep_tree(
                    in_proj,
                    graph,
                    m2_repo,
                    javac_bin,
                    env_map,
                    out_base_dir,
                    cp_sep,
                    visiting,
                    compiled_mods,
                    in_project_cp,
                    external_jars,
                    source_roots,
                    compiler_logs,
                )?;
                let out_dir = if let Some(existing) = compiled_mods.get(&in_proj.key) {
                    existing.clone()
                } else {
                    let mod_out = out_base_dir.join(sanitize_slug(&in_proj.key));
                    fs::create_dir_all(&mod_out).map_err(|e| {
                        Error::Internal(format!("create module out '{}': {e}", in_proj.key))
                    })?;
                    let mut srcs = Vec::new();
                    for sdir in &in_proj.main_src_dirs {
                        if sdir.is_dir() {
                            if !source_roots.contains(sdir) {
                                source_roots.push(sdir.clone());
                            }
                            collect_java_files_rec(sdir, &mut srcs);
                        }
                    }
                    if !srcs.is_empty() {
                        let mut dep_cp = in_project_cp.clone();
                        dep_cp.extend(external_jars.iter().cloned());
                        run_javac(
                            javac_bin,
                            &mod_out,
                            &dep_cp.join(cp_sep),
                            &srcs,
                            env_map,
                            compiler_logs,
                        )?;
                    }
                    compiled_mods.insert(in_proj.key.clone(), mod_out.clone());
                    mod_out
                };
                let out_str = out_dir.to_string_lossy().into_owned();
                if !in_project_cp.contains(&out_str) {
                    in_project_cp.push(out_str);
                }
                for rdir in &in_proj.resource_dirs {
                    if rdir.is_dir() {
                        let rs = rdir.to_string_lossy().into_owned();
                        if !in_project_cp.contains(&rs) {
                            in_project_cp.push(rs);
                        }
                    }
                }
                continue;
            }
        }
        // Not in current IDE project -> resolve from `.m2` / Gradle cache!
        if let Some(jar) = resolve_external_dep_jar(dep, m2_repo, false) {
            let js = jar.to_string_lossy().into_owned();
            if !external_jars.contains(&js) {
                external_jars.push(js);
            }
            let pom_in_m2 = jar.with_extension("pom");
            if let Ok(pom_raw) = fs::read_to_string(&pom_in_m2) {
                let (m2_mod, _) = parse_pom_module(
                    pom_in_m2.parent().unwrap_or_else(|| Path::new(".")),
                    &pom_raw,
                );
                for trans in &m2_mod.dependencies {
                    if graph
                        .find_dep(&trans.group_id, &trans.artifact_id)
                        .is_none()
                    {
                        if let Some(tjar) = resolve_external_dep_jar(trans, m2_repo, false) {
                            let ts = tjar.to_string_lossy().into_owned();
                            if !external_jars.contains(&ts) {
                                external_jars.push(ts);
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn run_javac(
    javac_bin: &str,
    out_dir: &Path,
    cp: &str,
    srcs: &[PathBuf],
    env_map: &BTreeMap<String, String>,
    logs: &mut Vec<String>,
) -> Result<()> {
    let mut cmd = Command::new(javac_bin);
    cmd.arg("-g")
        .arg("-encoding")
        .arg("UTF-8")
        .arg("-d")
        .arg(out_dir);
    if !cp.is_empty() {
        cmd.arg("-cp").arg(cp);
    }
    for s in srcs {
        cmd.arg(s);
    }
    for (k, v) in env_map {
        cmd.env(k, v);
    }
    let output = cmd
        .output()
        .map_err(|e| Error::Internal(format!("failed to execute '{javac_bin}': {e}")))?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !combined.trim().is_empty() {
        logs.push(combined.trim().to_string());
    }
    if !output.status.success() {
        return Err(Error::Invalid(format!(
            "javac compilation failed:\n{}",
            combined.trim()
        )));
    }
    Ok(())
}

pub fn infer_java_fqcn_from_file(java_file: &Path) -> Option<String> {
    let simple = java_file.file_stem()?.to_str()?.trim();
    if simple.is_empty() {
        return None;
    }
    let pkg = fs::read_to_string(java_file)
        .ok()
        .and_then(|src| {
            src.lines().find_map(|line| {
                let t = line.trim();
                t.strip_prefix("package ")
                    .and_then(|rest| rest.split(';').next())
                    .map(|p| p.trim().to_string())
            })
        })
        .filter(|p| !p.is_empty());
    Some(match pkg {
        Some(p) => format!("{p}.{simple}"),
        None => simple.to_string(),
    })
}

fn resolve_main_class(
    explicit: Option<&str>,
    target_file_or_dir: &Path,
    srcs: &[PathBuf],
) -> Result<String> {
    if let Some(exp) = explicit
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.ends_with(".java") && !s.contains('/'))
    {
        return Ok(exp.to_string());
    }
    if target_file_or_dir.is_file()
        && target_file_or_dir.extension().and_then(|s| s.to_str()) == Some("java")
    {
        if let Some(fqcn) = infer_java_fqcn_from_file(target_file_or_dir) {
            return Ok(fqcn);
        }
    }
    let main_re = Regex::new(r"\bpublic\s+static\s+void\s+main\s*\(").unwrap();
    for src in srcs {
        if let Ok(content) = fs::read_to_string(src) {
            if main_re.is_match(&content) {
                if let Some(fqcn) = infer_java_fqcn_from_file(src) {
                    return Ok(fqcn);
                }
            }
        }
    }
    Err(Error::Invalid(
        "could not determine Java main class; pass target .java file or main class name".into(),
    ))
}

pub fn collect_java_files_rec(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_java_files_rec(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("java") {
            out.push(p);
        }
    }
}

fn collect_java_files_shallow(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("java") {
            out.push(p);
        }
    }
}

fn sanitize_slug(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || (fs::canonicalize(a).ok().is_some()
            && fs::canonicalize(a).ok() == fs::canonicalize(b).ok())
}
