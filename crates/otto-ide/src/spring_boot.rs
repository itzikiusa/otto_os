//! Spring Boot IDE intelligence: workspace detection, `application.{properties,yml}`
//! hover/navigation/diagnostics, loopback-only Actuator inspector (Endpoints, Beans,
//! Health, redacted Env), and DevTools live restart trigger.

#[path = "spring_boot/actuator.rs"]
pub mod actuator;
#[path = "spring_boot/props.rs"]
pub mod props;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::types::{IdeSpringBootApp, IdeSpringBootInfo};

/// Scan one or more canonical workspace roots for Spring Boot applications,
/// `application*.{properties,yml,yaml}` configuration files, server/actuator
/// ports, Actuator presence, and DevTools configuration.
#[allow(clippy::disallowed_methods)] // bounded manifest/config scan inside spawn_blocking
pub fn detect_spring_boot_workspace(canon_roots: &[PathBuf]) -> Option<IdeSpringBootInfo> {
    detect_spring_boot_workspace_scoped(canon_roots, canon_roots, true)
}

struct ModuleSpringScan {
    module_root: String,
    has_spring_boot: bool,
    has_app: bool,
    has_actuator: bool,
    has_devtools: bool,
    server_port: u16,
    management_port: Option<u16>,
    actuator_base_path: String,
    trigger_file: Option<String>,
    apps: Vec<IdeSpringBootApp>,
    config_files: Vec<String>,
    profiles: BTreeSet<String>,
}

/// Scoped Spring Boot workspace/module detector that confines every file read
/// strictly inside `allowed_roots` (rejecting symlinks escaping the workspace
/// and protected secret paths).
#[allow(clippy::disallowed_methods)] // bounded manifest/config scan inside spawn_blocking
pub fn detect_spring_boot_workspace_scoped(
    module_roots: &[PathBuf],
    allowed_roots: &[PathBuf],
    is_root_user: bool,
) -> Option<IdeSpringBootInfo> {
    let mut scans: Vec<ModuleSpringScan> = Vec::new();
    for root in module_roots {
        if let Some(scan) = scan_single_spring_module(root, allowed_roots, is_root_user) {
            scans.push(scan);
        }
    }

    // Select one coherent primary Spring Boot module (preferring a module with an
    // `@SpringBootApplication` entry point) so `module_root`, `server_port`,
    // `management_port`, `actuator_base_path`, `has_actuator`, `has_devtools`, and
    // `trigger_file` always come from the SAME application (BUG2-9).
    let primary_idx = scans
        .iter()
        .position(|s| s.has_app)
        .or_else(|| scans.iter().position(|s| s.has_spring_boot))?;

    let primary = &scans[primary_idx];
    let mut apps: Vec<IdeSpringBootApp> = Vec::new();
    let mut config_files: Vec<String> = Vec::new();
    let mut profiles: BTreeSet<String> = BTreeSet::new();
    for s in &scans {
        apps.extend(s.apps.iter().cloned());
        for cf in &s.config_files {
            if !config_files.contains(cf) {
                config_files.push(cf.clone());
            }
        }
        profiles.extend(s.profiles.iter().cloned());
    }

    let effective_port = primary.management_port.unwrap_or(primary.server_port);
    let actuator_url = format!(
        "http://127.0.0.1:{effective_port}{}",
        primary.actuator_base_path
    );

    Some(IdeSpringBootInfo {
        detected: true,
        apps,
        profiles: profiles.into_iter().collect(),
        config_files,
        server_port: primary.server_port,
        management_port: primary.management_port,
        actuator_base_path: primary.actuator_base_path.clone(),
        actuator_url,
        has_actuator: primary.has_actuator,
        has_devtools: primary.has_devtools,
        trigger_file: primary.trigger_file.clone(),
        module_root: Some(primary.module_root.clone()),
    })
}

#[allow(clippy::disallowed_methods)]
fn scan_single_spring_module(
    root: &Path,
    allowed_roots: &[PathBuf],
    is_root_user: bool,
) -> Option<ModuleSpringScan> {
    let mut has_spring_boot = false;
    let mut has_actuator = false;
    let mut has_devtools = false;
    let mut apps: Vec<IdeSpringBootApp> = Vec::new();
    let mut config_files: Vec<String> = Vec::new();
    let mut profiles: BTreeSet<String> = BTreeSet::new();

    // 1. Check build manifests (`pom.xml`, `build.gradle`, `build.gradle.kts`)
    for mf in ["pom.xml", "build.gradle.kts", "build.gradle"] {
        let p = root.join(mf);
        if let Some(head) = read_confined_bounded(&p, 128 * 1024, allowed_roots, is_root_user) {
            if head.contains("org.springframework.boot")
                || head.contains("spring-boot-starter")
                || head.contains("spring-boot-maven-plugin")
            {
                has_spring_boot = true;
            }
            if head.contains("spring-boot-starter-actuator") {
                has_actuator = true;
            }
            if head.contains("spring-boot-devtools") {
                has_devtools = true;
            }
        }
    }

    // 2. Scan `src/main/{java,kotlin}` for `@SpringBootApplication`
    for src_sub in ["src/main/java", "src/main/kotlin"] {
        let src_dir = root.join(src_sub);
        if src_dir.is_dir() {
            let mut scanned = 0usize;
            scan_spring_apps(
                &src_dir,
                root,
                allowed_roots,
                is_root_user,
                0,
                &mut scanned,
                &mut apps,
            );
        }
    }
    let has_app = !apps.is_empty();
    if has_app {
        has_spring_boot = true;
    }

    // 3. Scan `src/main/resources` & `config/` in two passes:
    //    Pass 1: base config files (`application.*`, `bootstrap.*`) set base ports/paths
    //            and collect `spring.profiles.active` + `spring.profiles.default`.
    //    Pass 2: profile-specific files apply ONLY when their profile is active — using
    //            `spring.profiles.active` if set, else `spring.profiles.default`, else
    //            Spring Boot's implicit `"default"` profile (BUG2-8).
    let mut base_configs: Vec<(PathBuf, String)> = Vec::new();
    let mut profile_configs: Vec<(PathBuf, String, String)> = Vec::new();
    for res_rel in ["src/main/resources", "config"] {
        let res_dir = root.join(res_rel);
        let Ok(rd) = fs::read_dir(&res_dir) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !props::is_spring_config_filename(fname) {
                continue;
            }
            let Some(raw) = read_confined_bounded(&path, 64 * 1024, allowed_roots, is_root_user)
            else {
                continue;
            };
            let display = path.to_string_lossy().into_owned();
            if !config_files.contains(&display) {
                config_files.push(display);
            }
            if let Some(prof) = extract_profile_from_filename(fname) {
                let prof_name = prof.to_string();
                profiles.insert(prof_name.clone());
                profile_configs.push((path, prof_name, raw));
            } else {
                base_configs.push((path, raw));
            }
        }
    }

    if !has_spring_boot {
        return None;
    }

    let mut server_port: u16 = 8080;
    let mut management_port: Option<u16> = None;
    let mut actuator_base_path = "/actuator".to_string();
    let mut trigger_file: Option<String> = None;
    let mut active_profiles: BTreeSet<String> = BTreeSet::new();
    let mut default_profiles: BTreeSet<String> = BTreeSet::new();

    let apply_parsed = |parsed: &std::collections::BTreeMap<String, String>,
                        server_port: &mut u16,
                        management_port: &mut Option<u16>,
                        actuator_base_path: &mut String,
                        trigger_file: &mut Option<String>,
                        profiles: &mut BTreeSet<String>,
                        active_profiles: &mut BTreeSet<String>,
                        default_profiles: &mut BTreeSet<String>,
                        collect_profile_selectors: bool| {
        if let Some(p) = parsed
            .get("server.port")
            .and_then(|v| v.trim().parse::<u16>().ok())
        {
            *server_port = p;
        }
        if let Some(p) = parsed
            .get("management.server.port")
            .and_then(|v| v.trim().parse::<u16>().ok())
        {
            *management_port = Some(p);
        }
        if let Some(bp) = parsed
            .get("management.endpoints.web.base-path")
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        {
            *actuator_base_path = if bp.starts_with('/') {
                bp.trim_end_matches('/').to_string()
            } else {
                format!("/{}", bp.trim_end_matches('/'))
            };
        }
        if let Some(tf) = parsed
            .get("spring.devtools.restart.trigger-file")
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        {
            *trigger_file = Some(tf.to_string());
        }
        if collect_profile_selectors {
            if let Some(act_profs) = parsed.get("spring.profiles.active") {
                for p in act_profs
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && !s.contains("${"))
                {
                    profiles.insert(p.to_string());
                    active_profiles.insert(p.to_string());
                }
            }
            if let Some(def_profs) = parsed.get("spring.profiles.default") {
                for p in def_profs
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && !s.contains("${"))
                {
                    profiles.insert(p.to_string());
                    default_profiles.insert(p.to_string());
                }
            }
        }
    };

    for (path, raw) in &base_configs {
        let parsed = props::extract_config_map(&path.to_string_lossy(), raw);
        apply_parsed(
            &parsed,
            &mut server_port,
            &mut management_port,
            &mut actuator_base_path,
            &mut trigger_file,
            &mut profiles,
            &mut active_profiles,
            &mut default_profiles,
            true,
        );
    }

    let effective_profiles: BTreeSet<String> = if !active_profiles.is_empty() {
        active_profiles.clone()
    } else if !default_profiles.is_empty() {
        default_profiles.clone()
    } else {
        BTreeSet::from(["default".to_string()])
    };

    for (path, prof, raw) in &profile_configs {
        if !effective_profiles.contains(prof) {
            continue;
        }
        let parsed = props::extract_config_map(&path.to_string_lossy(), raw);
        apply_parsed(
            &parsed,
            &mut server_port,
            &mut management_port,
            &mut actuator_base_path,
            &mut trigger_file,
            &mut profiles,
            &mut active_profiles,
            &mut default_profiles,
            false,
        );
    }

    Some(ModuleSpringScan {
        module_root: root.to_string_lossy().into_owned(),
        has_spring_boot,
        has_app,
        has_actuator,
        has_devtools,
        server_port,
        management_port,
        actuator_base_path,
        trigger_file,
        apps,
        config_files,
        profiles,
    })
}

fn extract_profile_from_filename(fname: &str) -> Option<&str> {
    let rest = fname
        .strip_prefix("application-")
        .or_else(|| fname.strip_prefix("bootstrap-"))?;
    rest.strip_suffix(".properties")
        .or_else(|| rest.strip_suffix(".yml"))
        .or_else(|| rest.strip_suffix(".yaml"))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

#[allow(clippy::disallowed_methods)]
pub(crate) fn read_confined_bounded(
    path: &Path,
    max_bytes: usize,
    allowed_roots: &[PathBuf],
    is_root_user: bool,
) -> Option<String> {
    use std::io::Read;
    let canon = fs::canonicalize(path).ok()?;
    if !canon.is_file() {
        return None;
    }
    if !props::is_confined_spring_path(&canon, allowed_roots, is_root_user) {
        return None;
    }
    let f = fs::File::open(&canon).ok()?;
    let mut buf = Vec::new();
    f.take(max_bytes as u64).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

#[allow(clippy::disallowed_methods)]
fn scan_spring_apps(
    dir: &Path,
    workspace_root: &Path,
    allowed_roots: &[PathBuf],
    is_root_user: bool,
    depth: usize,
    scanned: &mut usize,
    out: &mut Vec<IdeSpringBootApp>,
) {
    if depth > 7 || *scanned >= 80 || out.len() >= 8 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for entry in entries {
        if *scanned >= 80 || out.len() >= 8 {
            break;
        }
        let Ok(meta) = fs::symlink_metadata(&entry) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            scan_spring_apps(
                &entry,
                workspace_root,
                allowed_roots,
                is_root_user,
                depth + 1,
                scanned,
                out,
            );
            continue;
        }
        let Some(ext) = entry.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        if ext != "java" && ext != "kt" {
            continue;
        }
        *scanned += 1;
        let Some(head) = read_confined_bounded(&entry, 16 * 1024, allowed_roots, is_root_user)
        else {
            continue;
        };
        if !head.contains("@SpringBootApplication")
            && !head.contains("SpringApplication.run(")
            && !head.contains("runApplication<")
        {
            continue;
        }
        let stem = entry
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Application");
        let cls = if ext == "kt" {
            crate::dap::java_bridge::infer_kotlin_class_stem(stem, &head)
        } else {
            stem.to_string()
        };
        let pkg = head.lines().find_map(|l| {
            let t = l.trim();
            t.strip_prefix("package ")
                .map(|r| r.trim_end_matches(';').trim().to_string())
        });
        let main_class = match pkg {
            Some(p) if !p.is_empty() => format!("{p}.{cls}"),
            _ => cls,
        };
        let rel = entry
            .strip_prefix(workspace_root)
            .unwrap_or(&entry)
            .to_string_lossy()
            .into_owned();
        out.push(IdeSpringBootApp {
            name: stem.to_string(),
            main_class,
            file_path: rel,
            language: if ext == "kt" { "kotlin" } else { "java" }.to_string(),
        });
    }
}
