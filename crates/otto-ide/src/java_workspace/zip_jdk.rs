//! Pure-Rust `< 2ms` ZIP/JAR Central Directory reader and `< 1.5 MB` `OnceLock`
//! JDK Standard Class Index (`ct.sym` / `src.zip` + core fallback).

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

/// Read all entry paths from a ZIP / JAR / `ct.sym` central directory without
/// decompressing any file payloads (`< 2ms` even on 60 MB archives).
pub fn read_zip_central_dir_entries(archive: &Path) -> io::Result<Vec<String>> {
    let mut file = File::open(archive)?;
    let file_len = file.metadata()?.len();
    if file_len < 22 {
        return Ok(Vec::new());
    }
    let tail_len = file_len.min(65_557) as usize;
    file.seek(SeekFrom::End(-(tail_len as i64)))?;
    let mut tail = vec![0u8; tail_len];
    file.read_exact(&mut tail)?;

    // Scan backwards for End of Central Directory signature `0x06054b50`.
    let mut eocd_pos = None;
    for i in (0..=tail.len().saturating_sub(22)).rev() {
        if tail[i..i + 4] == [0x50, 0x4b, 0x05, 0x06] {
            eocd_pos = Some(i);
            break;
        }
    }
    let Some(pos) = eocd_pos else {
        return Ok(Vec::new());
    };
    let cd_size = u32::from_le_bytes([
        tail[pos + 12],
        tail[pos + 13],
        tail[pos + 14],
        tail[pos + 15],
    ]) as usize;
    let cd_offset = u32::from_le_bytes([
        tail[pos + 16],
        tail[pos + 17],
        tail[pos + 18],
        tail[pos + 19],
    ]) as u64;
    if cd_offset == 0xFFFF_FFFF || cd_size == 0 || cd_offset + (cd_size as u64) > file_len {
        return Ok(Vec::new());
    }
    if cd_size > 32 * 1024 * 1024 {
        return Ok(Vec::new());
    }

    file.seek(SeekFrom::Start(cd_offset))?;
    let mut cd = vec![0u8; cd_size];
    file.read_exact(&mut cd)?;

    let mut entries = Vec::new();
    let mut cursor = 0usize;
    while cursor + 46 <= cd.len() {
        if cd[cursor..cursor + 4] != [0x50, 0x4b, 0x01, 0x02] {
            break;
        }
        let name_len = u16::from_le_bytes([cd[cursor + 28], cd[cursor + 29]]) as usize;
        let extra_len = u16::from_le_bytes([cd[cursor + 30], cd[cursor + 31]]) as usize;
        let comment_len = u16::from_le_bytes([cd[cursor + 32], cd[cursor + 33]]) as usize;
        let name_start = cursor + 46;
        let name_end = name_start.saturating_add(name_len);
        if name_end > cd.len() {
            break;
        }
        if let Ok(name) = std::str::from_utf8(&cd[name_start..name_end]) {
            entries.push(name.to_string());
        }
        cursor = name_end
            .saturating_add(extra_len)
            .saturating_add(comment_len);
    }
    Ok(entries)
}

/// Extract the set of public class FQNs (`com.foo.Bar`) and packages (`com.foo`)
/// inside a `.jar` file on demand via its ZIP central directory (`< 1ms`).
pub fn read_jar_classes_and_packages(jar_path: &Path) -> (HashSet<String>, HashSet<String>) {
    let mut classes = HashSet::new();
    let mut packages = HashSet::new();
    let Ok(entries) = read_zip_central_dir_entries(jar_path) else {
        return (classes, packages);
    };
    for entry in entries {
        let Some(no_ext) = entry.strip_suffix(".class") else {
            continue;
        };
        let rel = no_ext
            .strip_prefix("META-INF/versions/")
            .and_then(|r| r.split_once('/').map(|(_, rest)| rest))
            .unwrap_or(no_ext);
        if rel == "module-info" || rel == "package-info" || rel.starts_with("META-INF/") {
            continue;
        }
        let fqn = rel.replace(['/', '$'], ".");
        if let Some((pkg, _)) = rel.rsplit_once('/') {
            packages.insert(pkg.replace('/', "."));
        }
        classes.insert(fqn);
        if !rel.contains('$') {
            classes.insert(rel.replace('/', "."));
        }
    }
    (classes, packages)
}

pub struct JdkClassIndex {
    pub fqns: HashSet<String>,
    pub packages: HashSet<String>,
    pub by_simple: HashMap<String, Vec<String>>,
}

const MAX_JDK_INDEX_CACHE_ENTRIES: usize = 8;

type JdkCacheMap = HashMap<Option<PathBuf>, (u64, Arc<JdkClassIndex>)>;

static GLOBAL_JDK_INDEX_CACHE: OnceLock<RwLock<JdkCacheMap>> = OnceLock::new();
static JDK_CACHE_TICK: AtomicU64 = AtomicU64::new(1);

pub fn global_jdk_index(preferred_java_home: Option<&Path>) -> Arc<JdkClassIndex> {
    let effective_home =
        find_any_java_home(preferred_java_home).map(|p| std::fs::canonicalize(&p).unwrap_or(p));
    let cache = GLOBAL_JDK_INDEX_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    if let Some(found) = cache
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&effective_home)
        .map(|(_, idx)| Arc::clone(idx))
    {
        return found;
    }

    let built = Arc::new(build_jdk_index_for_home(effective_home.as_deref()));
    let mut map = cache.write().unwrap_or_else(|e| e.into_inner());
    let tick = JDK_CACHE_TICK.fetch_add(1, Ordering::Relaxed);
    if let Some((seen_tick, existing)) = map.get_mut(&effective_home) {
        *seen_tick = tick;
        return Arc::clone(existing);
    }
    if map.len() >= MAX_JDK_INDEX_CACHE_ENTRIES {
        if let Some(oldest_key) = map
            .iter()
            .min_by_key(|(_, (t, _))| *t)
            .map(|(k, _)| k.clone())
        {
            map.remove(&oldest_key);
        }
    }
    map.insert(effective_home, (tick, Arc::clone(&built)));
    built
}

fn find_any_java_home(preferred: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = preferred.filter(|p| p.join("lib").is_dir()) {
        return Some(p.to_path_buf());
    }
    if let Ok(jh) = std::env::var("JAVA_HOME") {
        let p = PathBuf::from(jh);
        if p.join("lib").is_dir() {
            return Some(p);
        }
    }
    let mut bases = vec![
        PathBuf::from("/Library/Java/JavaVirtualMachines"),
        PathBuf::from("/opt/homebrew/opt/openjdk/libexec/openjdk.jdk/Contents/Home"),
        PathBuf::from("/usr/lib/jvm"),
    ];
    if let Some(h) = dirs::home_dir() {
        bases.insert(0, h.join("Library/Java/JavaVirtualMachines"));
        bases.push(h.join(".sdkman/candidates/java/current"));
    }
    for base in bases {
        if base.join("lib/ct.sym").is_file() || base.join("lib/src.zip").is_file() {
            return Some(base);
        }
        if let Ok(rd) = std::fs::read_dir(&base) {
            for entry in rd.flatten() {
                for cand in [entry.path().join("Contents/Home"), entry.path()] {
                    if cand.join("lib/ct.sym").is_file() || cand.join("lib/src.zip").is_file() {
                        return Some(cand);
                    }
                }
            }
        }
    }
    None
}

fn build_jdk_index_for_home(resolved_java_home: Option<&Path>) -> JdkClassIndex {
    let mut fqns = HashSet::new();
    let mut packages = HashSet::new();

    if let Some(jh) = resolved_java_home {
        let src_zip = jh.join("lib/src.zip");
        let ct_sym = jh.join("lib/ct.sym");
        if src_zip.is_file() {
            if let Ok(entries) = read_zip_central_dir_entries(&src_zip) {
                for e in entries {
                    if let Some(rest) = e.strip_suffix(".java") {
                        // Format: `java.base/java/util/List` or `java/util/List`
                        let class_slash = match rest.split_once('/') {
                            Some((first, after)) if first.contains('.') => after,
                            _ => rest,
                        };
                        if class_slash.ends_with("module-info")
                            || class_slash.ends_with("package-info")
                            || class_slash.starts_with("sun/")
                            || class_slash.starts_with("jdk/internal/")
                        {
                            continue;
                        }
                        insert_jdk_class(&mut fqns, &mut packages, class_slash);
                    }
                }
            }
        } else if ct_sym.is_file() {
            if let Ok(entries) = read_zip_central_dir_entries(&ct_sym) {
                for e in entries {
                    let Some(rest) = e.strip_suffix(".sig").or_else(|| e.strip_suffix(".class"))
                    else {
                        continue;
                    };
                    // Format: `<release>/<module>/java/util/List`
                    let mut parts = rest.splitn(3, '/');
                    let _rel = parts.next();
                    let _mod = parts.next();
                    let Some(class_slash) = parts.next() else {
                        continue;
                    };
                    if class_slash.contains('$')
                        || class_slash.ends_with("module-info")
                        || class_slash.ends_with("package-info")
                    {
                        continue;
                    }
                    insert_jdk_class(&mut fqns, &mut packages, class_slash);
                }
            }
        }
    }

    // Seed core JDK classes unconditionally so headless/minimal hosts always work.
    for core in CORE_JDK_CLASSES {
        let slash = core.replace('.', "/");
        insert_jdk_class(&mut fqns, &mut packages, &slash);
    }

    let mut by_simple: HashMap<String, Vec<String>> = HashMap::new();
    for fqn in &fqns {
        if let Some((_, simple)) = fqn.rsplit_once('.') {
            let list = by_simple.entry(simple.to_string()).or_default();
            if list.len() < 4 && !list.contains(fqn) {
                list.push(fqn.clone());
            }
        }
    }
    for list in by_simple.values_mut() {
        list.sort_by(|a, b| {
            let a_java = a.starts_with("java.util.") || a.starts_with("java.lang.");
            let b_java = b.starts_with("java.util.") || b.starts_with("java.lang.");
            b_java.cmp(&a_java).then_with(|| a.len().cmp(&b.len()))
        });
    }

    JdkClassIndex {
        fqns,
        packages,
        by_simple,
    }
}

fn insert_jdk_class(fqns: &mut HashSet<String>, packages: &mut HashSet<String>, slash_path: &str) {
    if let Some((pkg_slash, _)) = slash_path.rsplit_once('/') {
        packages.insert(pkg_slash.replace('/', "."));
    }
    fqns.insert(slash_path.replace('/', "."));
}

const CORE_JDK_CLASSES: &[&str] = &[
    "java.lang.Object",
    "java.lang.String",
    "java.lang.Integer",
    "java.lang.Long",
    "java.lang.Double",
    "java.lang.Boolean",
    "java.lang.Exception",
    "java.lang.RuntimeException",
    "java.lang.IllegalArgumentException",
    "java.lang.IllegalStateException",
    "java.lang.Thread",
    "java.lang.Runnable",
    "java.lang.System",
    "java.lang.Math",
    "java.util.List",
    "java.util.ArrayList",
    "java.util.LinkedList",
    "java.util.Map",
    "java.util.HashMap",
    "java.util.LinkedHashMap",
    "java.util.TreeMap",
    "java.util.Set",
    "java.util.HashSet",
    "java.util.Optional",
    "java.util.Collections",
    "java.util.Arrays",
    "java.util.Objects",
    "java.util.UUID",
    "java.util.Properties",
    "java.util.concurrent.CompletableFuture",
    "java.util.concurrent.ConcurrentHashMap",
    "java.util.concurrent.ExecutorService",
    "java.util.concurrent.Executors",
    "java.util.concurrent.TimeUnit",
    "java.util.concurrent.atomic.AtomicInteger",
    "java.util.concurrent.atomic.AtomicLong",
    "java.util.concurrent.atomic.AtomicBoolean",
    "java.util.function.Function",
    "java.util.function.Predicate",
    "java.util.function.Consumer",
    "java.util.function.Supplier",
    "java.util.stream.Collectors",
    "java.util.stream.Stream",
    "java.io.File",
    "java.io.IOException",
    "java.io.InputStream",
    "java.io.OutputStream",
    "java.io.BufferedReader",
    "java.io.InputStreamReader",
    "java.nio.file.Files",
    "java.nio.file.Path",
    "java.nio.file.Paths",
    "java.nio.charset.StandardCharsets",
    "java.math.BigDecimal",
    "java.math.BigInteger",
    "java.math.RoundingMode",
    "java.time.Instant",
    "java.time.Duration",
    "java.time.LocalDate",
    "java.time.LocalDateTime",
    "java.time.ZonedDateTime",
    "java.net.URI",
    "java.net.URL",
    "java.net.http.HttpClient",
    "java.net.http.HttpRequest",
    "java.net.http.HttpResponse",
    "java.sql.Connection",
    "java.sql.PreparedStatement",
    "java.sql.ResultSet",
    "java.sql.SQLException",
];
