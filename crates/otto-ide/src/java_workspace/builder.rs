//! Multi-module Java source compilation (`javac -g`) and `-classpath` assembly
//! prioritizing in-project workspace modules ahead of `~/.m2` / Gradle JARs.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use otto_core::{Error, Result};
use regex::Regex;
use sha2::{Digest, Sha256};

use super::{
    parse_pom_module, resolve_external_dep_jar, resolve_m2_repo, scan_java_workspace,
    JavaWorkspaceGraph, WorkspaceJavaModule,
};
use crate::environments::shell_env::collect_safe_non_root_base_env;
use crate::proc_group::{HelperGroup, BUILD_STEP_TIMEOUT, HELPER_OUTPUT_CAP};

thread_local! {
    /// Cancel flag of the [`prepare_java_execution_cancellable`] call running on
    /// this (blocking-pool) thread; every `javac` it starts polls it.
    static JAVAC_CANCEL: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// How often a running `javac` checks for cancellation.
const JAVAC_POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub struct JavaExecutionPlan {
    pub main_class: String,
    pub classpath: String,
    pub source_roots: Vec<PathBuf>,
    pub compiler_output: String,
}

/// [`prepare_java_execution`] whose `javac` runs are killed (whole process
/// group) once `cancel` is set — the async caller sets it when Stop drops it.
#[allow(clippy::too_many_arguments)]
pub fn prepare_java_execution_cancellable(
    target_file_or_dir: &Path,
    explicit_main_class: Option<&str>,
    include_tests: bool,
    project_roots: &[PathBuf],
    env_map: &BTreeMap<String, String>,
    javac_bin: &str,
    out_base_dir: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<JavaExecutionPlan> {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            JAVAC_CANCEL.with(|c| *c.borrow_mut() = None);
        }
    }
    JAVAC_CANCEL.with(|c| *c.borrow_mut() = Some(cancel));
    let _reset = Reset;
    prepare_java_execution(
        target_file_or_dir,
        explicit_main_class,
        include_tests,
        project_roots,
        env_map,
        javac_bin,
        out_base_dir,
    )
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
        let maven = maven_module_classpath(&owner.dir, env_map, out_base_dir);
        // A module whose deps Maven can't resolve (an attached repo that was never
        // installed) still gets the javac build below; if that fails too, Maven's
        // error is the one to show.
        let maven_err = match maven {
            Some(Err(e)) => Some(e),
            Some(Ok(cp)) => {
                let mut srcs = Vec::new();
                for dir in owner.main_src_dirs.iter().chain(&owner.test_src_dirs) {
                    if dir.is_dir() {
                        collect_java_files_rec(dir, &mut srcs);
                    }
                }
                // Every workspace module's sources, so stepping into a sibling module maps.
                for m in &graph.modules {
                    source_roots.extend(m.main_src_dirs.iter().chain(&m.test_src_dirs).cloned());
                }
                let main_class =
                    resolve_main_class(explicit_main_class, target_file_or_dir, &srcs)?;
                return Ok(JavaExecutionPlan {
                    main_class,
                    classpath: cp.join(cp_sep),
                    source_roots,
                    compiler_output: String::new(),
                });
            }
            None => None,
        };
        let mut compiled_mods: HashMap<String, PathBuf> = HashMap::new();
        let mut visiting: HashSet<String> = HashSet::new();
        let legacy = (|| -> Result<JavaExecutionPlan> {
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

            let main_class =
                resolve_main_class(explicit_main_class, target_file_or_dir, &owner_srcs)?;
            Ok(JavaExecutionPlan {
                main_class,
                classpath: final_cp.join(cp_sep),
                source_roots,
                compiler_output: compiler_logs.join("\n"),
            })
        })();
        return match (legacy, maven_err) {
            (Err(_), Some(maven_err)) => Err(maven_err),
            (legacy, _) => legacy,
        };
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

/// How many lines of build-tool output an error carries (the rest is noise).
const ERROR_LINES: usize = 20;

fn first_lines(text: &str) -> String {
    let lines: Vec<&str> = text.trim().lines().collect();
    let mut out = lines[..lines.len().min(ERROR_LINES)].join("\n");
    if lines.len() > ERROR_LINES {
        out.push_str(&format!("\n… ({} more lines)", lines.len() - ERROR_LINES));
    }
    out
}

/// Maven's own classpath for a module with a `pom.xml`:
/// `target/test-classes:target/classes:<dependency:build-classpath>`, after
/// `test-compile`. Maven resolves parents, BOMs and transitive deps, which a
/// hand-built javac classpath gets wrong. `None` when the module has no pom or
/// no `mvnw` / `mvn` is available (the caller falls back to javac).
fn maven_module_classpath(
    module_dir: &Path,
    env_map: &BTreeMap<String, String>,
    out_base_dir: &Path,
) -> Option<Result<Vec<String>>> {
    if !module_dir.join("pom.xml").is_file() {
        return None;
    }
    let mvn = crate::environments::find_build_wrapper(
        &module_dir.to_string_lossy(),
        "mvnw",
        &["pom.xml"],
    )
    .map(PathBuf::from)
    .or_else(|| {
        let path = env_map.get("PATH")?;
        crate::toolchains::find_executable_in(
            &std::env::split_paths(path).collect::<Vec<_>>(),
            &["mvn"],
        )
    })?;
    let cp_file = out_base_dir.join(format!(
        "{}.classpath",
        sanitize_slug(&module_dir.to_string_lossy())
    ));
    let _ = fs::remove_file(&cp_file);
    let mut cmd = Command::new(&mvn);
    // Build from the reactor root (`-pl <module> -am`) so sibling modules come from
    // their sources, not from a stale or missing `~/.m2` copy. With `-am` every
    // upstream module writes the output file first; the target module writes it last.
    let reactor = crate::lsp_host::servers::climb_to_build_root("java", module_dir, &|_| true);
    cmd.current_dir(&reactor);
    if let Ok(rel) = module_dir.strip_prefix(&reactor) {
        if !rel.as_os_str().is_empty() {
            cmd.arg("-pl").arg(rel).arg("-am");
        }
    }
    cmd.args([
        "-q",
        "-B",
        "test-compile",
        "dependency:build-classpath",
        "-Dmdep.includeScope=test",
    ]);
    cmd.arg(format!("-Dmdep.outputFile={}", cp_file.display()));
    Some((|| {
        let (status, output) = run_build_tool(cmd, env_map, "mvn")?;
        if !status.success() {
            return Err(Error::Invalid(format!(
                "Maven could not build the module (mvn test-compile dependency:build-classpath):\n{}",
                first_lines(&output)
            )));
        }
        let deps = fs::read_to_string(&cp_file).unwrap_or_default();
        let mut cp: Vec<String> = ["target/test-classes", "target/classes"]
            .iter()
            .map(|d| module_dir.join(d).to_string_lossy().into_owned())
            .collect();
        cp.extend(
            deps.trim()
                .split(if cfg!(windows) { ';' } else { ':' })
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        );
        Ok(cp)
    })())
}

/// Run a build tool in its own process group with the run env, cancellable
/// like `javac`; returns its exit status and combined (tail-capped) output.
fn run_build_tool(
    mut cmd: Command,
    env_map: &BTreeMap<String, String>,
    label: &str,
) -> Result<(std::process::ExitStatus, String)> {
    cmd.env_clear();
    if !env_map.contains_key("PATH") {
        cmd.envs(collect_safe_non_root_base_env());
    }
    cmd.envs(env_map);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let child = cmd
        .spawn()
        .map_err(|e| Error::Internal(format!("failed to execute '{label}': {e}")))?;
    let cancel = JAVAC_CANCEL.with(|c| c.borrow().clone());
    let output = wait_javac(child, BUILD_STEP_TIMEOUT, cancel.as_deref())?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok((output.status, combined))
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

fn compute_javac_stamp(javac_bin: &str, cp: &str, srcs: &[PathBuf]) -> String {
    let mut sorted: Vec<String> = srcs
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    sorted.sort();
    let mut hasher = Sha256::new();
    hasher.update(javac_bin.as_bytes());
    hasher.update(b"\0");
    hasher.update(cp.as_bytes());
    hasher.update(b"\0");
    for sp in &sorted {
        hasher.update(sp.as_bytes());
        if let Ok(meta) = fs::metadata(sp) {
            hasher.update(meta.len().to_le_bytes());
            let nanos = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            hasher.update(nanos.to_le_bytes());
        }
        hasher.update(b"\0");
    }
    hex::encode(hasher.finalize())
}

fn has_any_class_file(dir: &Path) -> bool {
    let Ok(rd) = fs::read_dir(dir) else {
        return false;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() && has_any_class_file(&p) {
            return true;
        }
        if p.extension().and_then(|s| s.to_str()) == Some("class") {
            return true;
        }
    }
    false
}

fn run_javac(
    javac_bin: &str,
    out_dir: &Path,
    cp: &str,
    srcs: &[PathBuf],
    env_map: &BTreeMap<String, String>,
    logs: &mut Vec<String>,
) -> Result<()> {
    let stamp = compute_javac_stamp(javac_bin, cp, srcs);
    let stamp_file = out_dir.join(".otto-javac.stamp");
    if fs::read_to_string(&stamp_file)
        .ok()
        .is_some_and(|prev| prev.trim() == stamp)
        && has_any_class_file(out_dir)
    {
        return Ok(());
    }

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
    let (status, combined) = run_build_tool(cmd, env_map, javac_bin)?;
    if !combined.trim().is_empty() {
        logs.push(combined.trim().to_string());
    }
    if !status.success() {
        return Err(Error::Invalid(format!(
            "javac compilation failed:\n{}",
            first_lines(&combined)
        )));
    }
    let _ = fs::write(&stamp_file, &stamp);
    Ok(())
}

/// Wait for `javac` (spawned in its own process group) on this blocking thread:
/// stdout/stderr are drained by reader threads keeping their last
/// [`HELPER_OUTPUT_CAP`] bytes, and the whole group is killed on `cancel`, on
/// `limit`, and after exit (stragglers).
fn wait_javac(
    mut child: std::process::Child,
    limit: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<std::process::Output> {
    fn drain<R: Read + Send + 'static>(r: Option<R>) -> Option<std::thread::JoinHandle<Vec<u8>>> {
        r.map(|mut r| {
            std::thread::spawn(move || {
                let mut out = Vec::new();
                let mut chunk = [0u8; 16 * 1024];
                loop {
                    match r.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            out.extend_from_slice(&chunk[..n]);
                            if out.len() > HELPER_OUTPUT_CAP * 2 {
                                out.drain(..out.len() - HELPER_OUTPUT_CAP);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
                if out.len() > HELPER_OUTPUT_CAP {
                    out.drain(..out.len() - HELPER_OUTPUT_CAP);
                }
                out
            })
        })
    }
    let mut group = HelperGroup::track(Some(child.id()));
    let out_t = drain(child.stdout.take());
    let err_t = drain(child.stderr.take());
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                group.kill();
                return Err(Error::Internal(format!("waiting for javac: {e}")));
            }
        }
        let stop = if cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
            Some("javac cancelled".to_string())
        } else if started.elapsed() >= limit {
            Some(format!("javac timed out after {}s", limit.as_secs()))
        } else {
            None
        };
        if let Some(msg) = stop {
            group.kill();
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Invalid(msg));
        }
        std::thread::sleep(JAVAC_POLL);
    };
    group.kill();
    let join = |t: Option<std::thread::JoinHandle<Vec<u8>>>| {
        t.and_then(|t| t.join().ok()).unwrap_or_default()
    };
    Ok(std::process::Output {
        status,
        stdout: join(out_t),
        stderr: join(err_t),
    })
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

fn main_method_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\bpublic\s+static\s+void\s+main\s*\(").unwrap())
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
    let main_re = main_method_regex();
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

#[cfg(all(test, unix))]
mod javac_process_tests {
    #![allow(clippy::disallowed_methods)] // test fixtures in a temp dir
    use super::*;
    use std::os::unix::process::CommandExt;

    fn fake_javac(dir: &Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("fake-javac");
        fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn wait_pid_gone(pid: u32) -> bool {
        for _ in 0..50 {
            let alive = std::process::Command::new("/bin/kill")
                .args(["-0", &pid.to_string()])
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if !alive {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Stop sets the cancel flag: javac's whole process group dies at once
    /// instead of compiling to the end on a blocking thread.
    #[test]
    fn cancelled_javac_is_killed_with_its_process_group() {
        let tmp = tempfile::tempdir().unwrap();
        let pid_file = tmp.path().join("child.pid");
        let javac = fake_javac(
            tmp.path(),
            &format!("sleep 60 & echo $! > '{}'; wait", pid_file.display()),
        );
        let out_dir = tmp.path().join("out");
        fs::create_dir_all(&out_dir).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let setter = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            setter.store(true, Ordering::Release);
        });
        JAVAC_CANCEL.with(|c| *c.borrow_mut() = Some(cancel));
        let started = Instant::now();
        let mut logs = Vec::new();
        let src = tmp.path().join("Main.java");
        fs::write(&src, "class Main {}").unwrap();
        let err = run_javac(&javac, &out_dir, "", &[src], &BTreeMap::new(), &mut logs)
            .expect_err("cancelled javac must fail the step");
        JAVAC_CANCEL.with(|c| *c.borrow_mut() = None);
        assert!(err.to_string().contains("cancelled"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
        let pid: u32 = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(wait_pid_gone(pid), "javac child {pid} survived the cancel");
    }

    #[test]
    fn javac_wait_is_bounded_by_its_timeout_and_keeps_output_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let javac = fake_javac(tmp.path(), "sleep 60");
        let child = Command::new(&javac)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let err = wait_javac(child, Duration::from_millis(300), None).unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");

        let loud = fake_javac(
            tmp.path(),
            "i=0; while [ $i -lt 3000 ]; do echo err-$i >&2; i=$((i+1)); done; exit 1",
        );
        let child = Command::new(&loud)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let out = wait_javac(child, Duration::from_secs(30), None).unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).ends_with("err-2999\n"));
    }

    fn maven_module(tmp: &Path) -> (PathBuf, PathBuf) {
        // A reactor (`tmp/pom.xml`) with one module, `svc`.
        fs::write(
            tmp.join("pom.xml"),
            "<project><modules><module>svc</module></modules></project>",
        )
        .unwrap();
        let module = tmp.join("svc");
        let src = module.join("src/main/java/com/acme");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            module.join("pom.xml"),
            "<project><groupId>com.acme</groupId><artifactId>svc</artifactId><version>1</version></project>",
        )
        .unwrap();
        let app = src.join("App.java");
        fs::write(
            &app,
            "package com.acme;\npublic class App { public static void main(String[] a) {} }\n",
        )
        .unwrap();
        (module, app)
    }

    fn fake_bin(dir: &Path, name: &str, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn path_env(bin: &Path) -> BTreeMap<String, String> {
        BTreeMap::from([(
            "PATH".to_string(),
            format!("{}:/usr/bin:/bin", bin.display()),
        )])
    }

    #[test]
    fn a_maven_module_runs_on_mavens_classpath_and_never_on_otto_javac() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let (module, app) = maven_module(&root);
        let bin = root.join("bin");
        let seen = root.join("mvn-call");
        // The fake mvn writes the classpath to -Dmdep.outputFile=… like the real plugin.
        fake_bin(
            &bin,
            "mvn",
            &format!(
                r#"echo "$PWD $*" > {}; for a in "$@"; do case "$a" in -Dmdep.outputFile=*) printf '/m2/a.jar:/m2/b.jar' > "${{a#-Dmdep.outputFile=}}";; esac; done"#,
                seen.display()
            ),
        );
        let javac_hit = root.join("javac-was-called");
        let javac = fake_bin(
            &bin,
            "javac",
            &format!("touch {}; exit 1", javac_hit.display()),
        );
        let plan = prepare_java_execution(
            &app,
            None,
            false,
            std::slice::from_ref(&module),
            &path_env(&bin),
            &javac,
            &root.join("out"),
        )
        .unwrap();
        assert_eq!(plan.main_class, "com.acme.App");
        let t = |d: &str| module.join(d).to_string_lossy().into_owned();
        assert_eq!(
            plan.classpath,
            format!(
                "{}:{}:/m2/a.jar:/m2/b.jar",
                t("target/test-classes"),
                t("target/classes")
            )
        );
        assert!(
            !javac_hit.exists(),
            "Otto's own javac must not run for a Maven module"
        );
        let call = fs::read_to_string(&seen).unwrap();
        assert!(
            call.starts_with(&format!(
                "{} -pl svc -am -q -B test-compile dependency:build-classpath",
                root.display()
            )),
            "built from the reactor root: {call}"
        );
    }

    #[test]
    fn a_failing_maven_build_falls_back_to_javac_and_reports_a_capped_error() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let (module, app) = maven_module(&root);
        let bin = root.join("bin");
        fake_bin(
            &bin,
            "mvn",
            "i=0; while [ $i -lt 500 ]; do echo \"[ERROR] line-$i\"; i=$((i+1)); done; exit 1",
        );
        let prepare = |javac: &str| {
            prepare_java_execution(
                &app,
                None,
                false,
                std::slice::from_ref(&module),
                &path_env(&bin),
                javac,
                &root.join("out"),
            )
        };
        // An attached repo Maven can't resolve still builds with javac.
        let ok_javac = fake_bin(&bin, "javac-ok", "exit 0");
        assert_eq!(prepare(&ok_javac).unwrap().main_class, "com.acme.App");
        // When javac fails too, Maven's error is shown, capped.
        let bad_javac = fake_bin(&bin, "javac-bad", "echo boom >&2; exit 1");
        let err = prepare(&bad_javac).unwrap_err().to_string();
        assert!(err.contains("line-0") && err.contains("line-19"), "{err}");
        assert!(
            !err.contains("line-20") && err.contains("480 more lines"),
            "{err}"
        );
    }
}
