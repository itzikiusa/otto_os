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
    let mut apps: Vec<IdeSpringBootApp> = Vec::new();
    let mut config_files: Vec<String> = Vec::new();
    let mut profiles: BTreeSet<String> = BTreeSet::new();
    let mut has_spring_boot = false;
    let mut has_actuator = false;
    let mut has_devtools = false;
    let mut server_port: u16 = 8080;
    let mut management_port: Option<u16> = None;
    let mut actuator_base_path = "/actuator".to_string();
    let mut trigger_file: Option<String> = None;

    for root in canon_roots {
        // 1. Check build manifests (`pom.xml`, `build.gradle`, `build.gradle.kts`)
        for mf in ["pom.xml", "build.gradle.kts", "build.gradle"] {
            let p = root.join(mf);
            if let Ok(head) = read_bounded(&p, 128 * 1024) {
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
                scan_spring_apps(&src_dir, root, 0, &mut scanned, &mut apps);
            }
        }
        if !apps.is_empty() {
            has_spring_boot = true;
        }

        // 3. Scan `src/main/resources` & `config/` for `application*.{properties,yml,yaml}`
        for res_rel in ["src/main/resources", "config"] {
            let res_dir = root.join(res_rel);
            let Ok(rd) = fs::read_dir(&res_dir) else {
                continue;
            };
            let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            entries.sort();
            for path in entries {
                if !path.is_file() {
                    continue;
                }
                let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if !props::is_spring_config_filename(fname) {
                    continue;
                }
                let display = path.to_string_lossy().into_owned();
                if !config_files.contains(&display) {
                    config_files.push(display);
                }
                if let Some(prof) = extract_profile_from_filename(fname) {
                    profiles.insert(prof.to_string());
                }
                if let Ok(raw) = read_bounded(&path, 64 * 1024) {
                    let parsed = props::extract_config_map(&path.to_string_lossy(), &raw);
                    // Only let the default `application.{properties,yml}` set default ports/paths,
                    // or fall back if not already set.
                    let is_base = fname.starts_with("application.");
                    if is_base || server_port == 8080 {
                        if let Some(p) = parsed
                            .get("server.port")
                            .and_then(|v| v.trim().parse::<u16>().ok())
                        {
                            server_port = p;
                        }
                    }
                    if is_base || management_port.is_none() {
                        if let Some(p) = parsed
                            .get("management.server.port")
                            .and_then(|v| v.trim().parse::<u16>().ok())
                        {
                            management_port = Some(p);
                        }
                    }
                    if is_base || actuator_base_path == "/actuator" {
                        if let Some(bp) = parsed
                            .get("management.endpoints.web.base-path")
                            .map(|v| v.trim())
                            .filter(|v| !v.is_empty())
                        {
                            actuator_base_path = if bp.starts_with('/') {
                                bp.trim_end_matches('/').to_string()
                            } else {
                                format!("/{}", bp.trim_end_matches('/'))
                            };
                        }
                    }
                    if let Some(tf) = parsed
                        .get("spring.devtools.restart.trigger-file")
                        .map(|v| v.trim())
                        .filter(|v| !v.is_empty())
                    {
                        trigger_file = Some(tf.to_string());
                    }
                    if let Some(act_profs) = parsed.get("spring.profiles.active") {
                        for p in act_profs
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            profiles.insert(p.to_string());
                        }
                    }
                }
            }
        }
    }

    if !has_spring_boot {
        return None;
    }

    let effective_port = management_port.unwrap_or(server_port);
    let actuator_url = format!("http://127.0.0.1:{effective_port}{actuator_base_path}");

    Some(IdeSpringBootInfo {
        detected: true,
        apps,
        profiles: profiles.into_iter().collect(),
        config_files,
        server_port,
        management_port,
        actuator_base_path,
        actuator_url,
        has_actuator,
        has_devtools,
        trigger_file,
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
fn read_bounded(path: &Path, max_bytes: usize) -> std::io::Result<String> {
    use std::io::Read;
    let f = fs::File::open(path)?;
    let mut buf = Vec::new();
    f.take(max_bytes as u64).read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

#[allow(clippy::disallowed_methods)]
fn scan_spring_apps(
    dir: &Path,
    workspace_root: &Path,
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
        if entry.is_dir() {
            scan_spring_apps(&entry, workspace_root, depth + 1, scanned, out);
            continue;
        }
        let Some(ext) = entry.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        if ext != "java" && ext != "kt" {
            continue;
        }
        *scanned += 1;
        let Ok(head) = read_bounded(&entry, 16 * 1024) else {
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
