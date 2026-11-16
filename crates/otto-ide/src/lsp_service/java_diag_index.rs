//! Cached Java workspace & external-JAR symbol indexing for `java_diag`.
//!
//! Avoids re-walking Maven/Gradle build manifests, `.m2` JAR central directories,
//! and source trees on every 250ms editor debounce or `code_actions` pass while
//! invalidating immediately when build manifests or source/class directories change.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use crate::java_workspace::{
    read_jar_classes_and_packages, resolve_external_dep_jar, resolve_m2_repo, scan_java_workspace,
    JavaWorkspaceGraph,
};

use super::java_diag::KNOWN_LOMBOK_CLASSES;

const MAX_JAR_CACHE_ENTRIES: usize = 512;
const MAX_WORKSPACE_INDEX_ENTRIES: usize = 16;
const MAX_TRACKED_DIRS: usize = 512;
const LARGE_TREE_DIR_TTL: Duration = Duration::from_secs(3);

const MANIFEST_NAMES: &[&str] = &[
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
];

#[derive(Clone)]
struct CachedJarSymbols {
    fingerprint: (u128, u64),
    classes: Arc<Vec<String>>,
    packages: Arc<Vec<String>>,
}

pub(super) struct CachedJavaProjectIndex {
    pub(super) ws: JavaWorkspaceGraph,
    pub(super) ext_classes: HashSet<String>,
    pub(super) ext_packages: HashSet<String>,
    pub(super) ext_by_simple: HashMap<String, Vec<String>>,
    pub(super) ws_by_simple: HashMap<String, Vec<String>>,
    manifest_fingerprints: Vec<(PathBuf, Option<(u128, u64)>)>,
    dir_fingerprints: Vec<(PathBuf, Option<u128>)>,
    dirs_complete: bool,
    built_at: Instant,
}

type JarCacheKey = PathBuf;
type JarCacheMap = HashMap<JarCacheKey, (u64, CachedJarSymbols)>;
type WorkspaceCacheKey = (Vec<PathBuf>, Option<PathBuf>);
type WorkspaceCacheEntry = (u64, Arc<CachedJavaProjectIndex>);
type WorkspaceCacheMap = HashMap<WorkspaceCacheKey, WorkspaceCacheEntry>;

static JAR_SYMBOL_CACHE: OnceLock<RwLock<JarCacheMap>> = OnceLock::new();
static WORKSPACE_INDEX_CACHE: OnceLock<RwLock<WorkspaceCacheMap>> = OnceLock::new();
static CACHE_TICK: AtomicU64 = AtomicU64::new(1);
#[cfg(test)]
static WORKSPACE_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static WORKSPACE_CACHE_MISSES: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
pub(super) fn workspace_cache_stats() -> (u64, u64) {
    (
        WORKSPACE_CACHE_HITS.load(Ordering::Relaxed),
        WORKSPACE_CACHE_MISSES.load(Ordering::Relaxed),
    )
}

#[allow(clippy::disallowed_methods)]
fn file_fingerprint(path: &Path) -> Option<(u128, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    Some((mtime, meta.len()))
}

#[allow(clippy::disallowed_methods)]
fn dir_mtime_ns(path: &Path) -> Option<u128> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_dir() {
        return None;
    }
    Some(
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos()),
    )
}

impl CachedJavaProjectIndex {
    fn is_fresh(&self) -> bool {
        for (path, expected) in &self.manifest_fingerprints {
            if file_fingerprint(path) != *expected {
                return false;
            }
        }
        if !self.dirs_complete && self.built_at.elapsed() >= LARGE_TREE_DIR_TTL {
            return false;
        }
        for (dir, expected) in &self.dir_fingerprints {
            if dir_mtime_ns(dir) != *expected {
                return false;
            }
        }
        true
    }
}

#[allow(clippy::disallowed_methods)]
pub(super) fn load_or_build_java_project_index(
    project_roots: &[PathBuf],
    env_map: &BTreeMap<String, String>,
) -> Arc<CachedJavaProjectIndex> {
    if project_roots.is_empty() {
        return Arc::new(CachedJavaProjectIndex {
            ws: JavaWorkspaceGraph::default(),
            ext_classes: HashSet::new(),
            ext_packages: HashSet::new(),
            ext_by_simple: HashMap::new(),
            ws_by_simple: HashMap::new(),
            manifest_fingerprints: Vec::new(),
            dir_fingerprints: Vec::new(),
            dirs_complete: true,
            built_at: Instant::now(),
        });
    }

    let norm_roots: Vec<PathBuf> = project_roots
        .iter()
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .collect();
    let m2_repo = resolve_m2_repo(env_map);
    let key = (norm_roots.clone(), m2_repo.clone());
    let cache = WORKSPACE_INDEX_CACHE.get_or_init(|| RwLock::new(HashMap::new()));

    let cached_opt = cache
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .map(|(_, entry)| Arc::clone(entry));
    if let Some(existing) = cached_opt {
        if existing.is_fresh() {
            #[cfg(test)]
            WORKSPACE_CACHE_HITS.fetch_add(1, Ordering::Relaxed);
            return existing;
        }
    }

    #[cfg(test)]
    WORKSPACE_CACHE_MISSES.fetch_add(1, Ordering::Relaxed);

    let ws = scan_java_workspace(&norm_roots);
    let mut tracked_dirs: Vec<PathBuf> = Vec::new();
    let mut dirs_complete = true;
    let mut push_dir = |d: PathBuf| {
        if tracked_dirs.len() < MAX_TRACKED_DIRS {
            if !tracked_dirs.contains(&d) {
                tracked_dirs.push(d);
            }
        } else {
            dirs_complete = false;
        }
    };

    for root in &norm_roots {
        push_dir(root.clone());
    }
    for module in &ws.modules {
        push_dir(module.dir.clone());
        for rel in [
            "src",
            "src/main",
            "src/main/java",
            "src/test",
            "src/test/java",
            "target",
            "target/classes",
            "target/test-classes",
            "build/classes/java/main",
        ] {
            push_dir(module.dir.join(rel));
        }
    }

    let (ext_classes, ext_packages, ext_by_simple) =
        collect_external_jar_symbols(&ws, m2_repo.as_deref(), &mut push_dir);
    let ws_by_simple = index_workspace_simple_classes(&ws, &mut push_dir);

    let mut manifest_paths: Vec<PathBuf> = Vec::new();
    for root in &norm_roots {
        for name in MANIFEST_NAMES {
            let p = root.join(name);
            if !manifest_paths.contains(&p) {
                manifest_paths.push(p);
            }
        }
    }
    for module in &ws.modules {
        for name in MANIFEST_NAMES {
            let p = module.dir.join(name);
            if !manifest_paths.contains(&p) {
                manifest_paths.push(p);
            }
        }
    }
    let manifest_fingerprints = manifest_paths
        .into_iter()
        .map(|p| {
            let fp = file_fingerprint(&p);
            (p, fp)
        })
        .collect();
    let dir_fingerprints = tracked_dirs
        .into_iter()
        .map(|d| {
            let mt = dir_mtime_ns(&d);
            (d, mt)
        })
        .collect();

    let built = Arc::new(CachedJavaProjectIndex {
        ws,
        ext_classes,
        ext_packages,
        ext_by_simple,
        ws_by_simple,
        manifest_fingerprints,
        dir_fingerprints,
        dirs_complete,
        built_at: Instant::now(),
    });

    let tick = CACHE_TICK.fetch_add(1, Ordering::Relaxed);
    let mut map = cache.write().unwrap_or_else(|e| e.into_inner());
    if !map.contains_key(&key) && map.len() >= MAX_WORKSPACE_INDEX_ENTRIES {
        if let Some(oldest) = map
            .iter()
            .min_by_key(|(_, (t, _))| *t)
            .map(|(k, _)| k.clone())
        {
            map.remove(&oldest);
        }
    }
    map.insert(key, (tick, Arc::clone(&built)));
    built
}

fn read_cached_jar_symbols(jar: &Path) -> (Arc<Vec<String>>, Arc<Vec<String>>) {
    let Some(fp) = file_fingerprint(jar) else {
        return (Arc::new(Vec::new()), Arc::new(Vec::new()));
    };
    let cache = JAR_SYMBOL_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    if let Some(hit) = cache
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(jar)
        .filter(|(_, entry)| entry.fingerprint == fp)
        .map(|(_, entry)| entry.clone())
    {
        return (hit.classes, hit.packages);
    }

    let (jc, jp) = read_jar_classes_and_packages(jar);
    let classes = Arc::new(jc.into_iter().collect::<Vec<_>>());
    let packages = Arc::new(jp.into_iter().collect::<Vec<_>>());
    let entry = CachedJarSymbols {
        fingerprint: fp,
        classes: Arc::clone(&classes),
        packages: Arc::clone(&packages),
    };
    let tick = CACHE_TICK.fetch_add(1, Ordering::Relaxed);
    let mut map = cache.write().unwrap_or_else(|e| e.into_inner());
    if !map.contains_key(jar) && map.len() >= MAX_JAR_CACHE_ENTRIES {
        if let Some(oldest) = map
            .iter()
            .min_by_key(|(_, (t, _))| *t)
            .map(|(k, _)| k.clone())
        {
            map.remove(&oldest);
        }
    }
    map.insert(jar.to_path_buf(), (tick, entry));
    (classes, packages)
}

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
fn collect_external_jar_symbols(
    ws: &JavaWorkspaceGraph,
    m2_repo: Option<&Path>,
    mut on_dir: impl FnMut(PathBuf),
) -> (
    HashSet<String>,
    HashSet<String>,
    HashMap<String, Vec<String>>,
) {
    let mut classes = HashSet::new();
    let mut packages = HashSet::new();
    let mut by_simple: HashMap<String, Vec<String>> = HashMap::new();
    let mut seen_jars = HashSet::new();

    for module in &ws.modules {
        for rel in [
            "target/classes",
            "target/test-classes",
            "build/classes/java/main",
        ] {
            let cdir = module.dir.join(rel);
            if cdir.is_dir() {
                collect_compiled_dir_classes(
                    &cdir,
                    &cdir,
                    &mut classes,
                    &mut packages,
                    &mut on_dir,
                );
            }
        }
        for dep in &module.dependencies {
            if ws.find_dep(&dep.group_id, &dep.artifact_id).is_some() {
                continue;
            }
            if let Some(jar) = resolve_external_dep_jar(dep, m2_repo, false) {
                if seen_jars.insert(jar.clone()) {
                    let (jc, jp) = read_cached_jar_symbols(&jar);
                    for fqn in jc.iter() {
                        if let Some(simple) = fqn.rsplit('.').next() {
                            by_simple
                                .entry(simple.to_string())
                                .or_default()
                                .push(fqn.clone());
                        }
                        classes.insert(fqn.clone());
                    }
                    packages.extend(jp.iter().cloned());
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
    on_dir: &mut impl FnMut(PathBuf),
) {
    on_dir(cur.to_path_buf());
    let Ok(rd) = std::fs::read_dir(cur) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_compiled_dir_classes(base, &p, classes, packages, on_dir);
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
fn index_workspace_simple_classes(
    ws: &JavaWorkspaceGraph,
    mut on_dir: impl FnMut(PathBuf),
) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for sr in ws.all_source_roots(true) {
        on_dir(sr.clone());
        let walker = ignore::WalkBuilder::new(&sr).max_depth(Some(10)).build();
        for entry in walker.flatten() {
            let p = entry.path();
            if p.is_dir() {
                on_dir(p.to_path_buf());
                continue;
            }
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
