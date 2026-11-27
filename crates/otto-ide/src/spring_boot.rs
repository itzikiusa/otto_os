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
            let deps_surface = if mf == "pom.xml" {
                let stripped = crate::java_workspace::strip_xml_comments(&head);
                crate::java_workspace::remove_all_tag_blocks(&stripped, "dependencyManagement")
            } else {
                head
            };
            if deps_surface.contains("spring-boot-starter-actuator") {
                has_actuator = true;
            }
            if deps_surface.contains("spring-boot-devtools") {
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

    // BUG3-6: If this module is a Maven Spring Boot module, also inherit direct
    // `<dependencies>` (excluding `<dependencyManagement>` BOM entries) from its
    // actual `<parent>` POM chain confined strictly within `allowed_roots`.
    if has_spring_boot && (!has_devtools || !has_actuator) && root.join("pom.xml").is_file() {
        inherit_maven_parent_spring_capabilities(
            root,
            allowed_roots,
            is_root_user,
            &mut has_actuator,
            &mut has_devtools,
        );
    }

    // 3. Config data in Spring Boot's precedence order (lowest -> highest): the packaged
    //    group (`classpath:/`, `classpath:/config/`), then the external group (`file:./`,
    //    `file:./config/`). Within a group, the active profiles' files override the plain
    //    ones; within a directory `.properties` overrides YAML; within a file a later
    //    document overrides an earlier one, and a `spring.config.activate.on-profile`
    //    document applies only to an active profile (BUG2-8, BUG3-5).
    let mut config_docs: Vec<ConfigFileDocs> = Vec::new();
    let mut locations: Vec<(u8, PathBuf)> = [
        (0, "src/main/resources"),
        (0, "src/main/resources/config"),
        (1, ""),
        (1, "config"),
    ]
    .map(|(group, rel)| (group, root.join(rel)))
    .into();
    // `file:./config/*/`: each visible subdirectory in path order, so the last one wins.
    if let Ok(rd) = fs::read_dir(root.join("config")) {
        let mut subs: Vec<PathBuf> = rd
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with(".."))
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        subs.sort();
        locations.extend(subs.into_iter().map(|p| (1, p)));
    }
    for (group, res_dir) in locations {
        let Ok(rd) = fs::read_dir(&res_dir) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort_by(|a, b| {
            let rank = |p: &Path| {
                let name = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let is_props = name.ends_with(".properties");
                (is_props, name)
            };
            rank(a).cmp(&rank(b))
        });
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
            let file_profile = extract_profile_from_filename(fname).map(str::to_string);
            profiles.extend(file_profile.iter().cloned());
            let docs = split_config_documents(&raw)
                .iter()
                .map(|doc| {
                    let parsed = props::extract_config_map(&display, doc);
                    let on_profile = parsed
                        .get("spring.config.activate.on-profile")
                        .or_else(|| parsed.get("spring.profiles"))
                        .map(|v| v.trim().to_string());
                    if let Some(expr) = on_profile.as_deref() {
                        profiles.extend(
                            expr.split(',')
                                .flat_map(profile_expr_tokens)
                                .filter(|t| !"()&|!".contains(*t) && !t.contains("${"))
                                .map(String::from),
                        );
                    }
                    (on_profile, parsed)
                })
                .collect();
            if !config_files.contains(&display) {
                config_files.push(display);
            }
            config_docs.push((group, file_profile, docs));
        }
    }

    if !has_spring_boot {
        return None;
    }

    // Profile selectors come from plain documents only; the highest-precedence one wins.
    let mut active_profiles: Vec<String> = Vec::new();
    let mut default_profiles: Vec<String> = Vec::new();
    for (_, file_profile, docs) in &config_docs {
        for (on_profile, parsed) in docs {
            if file_profile.is_some() || on_profile.is_some() {
                continue;
            }
            for (key, selected) in [
                ("spring.profiles.active", &mut active_profiles),
                ("spring.profiles.default", &mut default_profiles),
            ] {
                let list = parsed.get(key).map(|v| profile_list(v)).unwrap_or_default();
                if !list.is_empty() {
                    profiles.extend(list.iter().cloned());
                    *selected = list;
                }
            }
        }
    }
    let effective_profiles = if !active_profiles.is_empty() {
        active_profiles
    } else if !default_profiles.is_empty() {
        default_profiles
    } else {
        vec!["default".to_string()]
    };
    // Each comma-separated profile expression of a document; any match activates it.
    let doc_applies = |on_profile: &Option<String>| match on_profile.as_deref() {
        None => true,
        Some(exprs) => exprs
            .split(',')
            .any(|e| profile_expr_matches(e, &effective_profiles) == Some(true)),
    };

    let mut server_port: u16 = 8080;
    let mut management_port: Option<u16> = None;
    let mut actuator_base_path = "/actuator".to_string();
    let mut trigger_file: Option<String> = None;
    for group in [0, 1] {
        // The group's plain files, then each active profile's files (the last profile wins).
        let plain = config_docs
            .iter()
            .filter(|(g, fp, _)| *g == group && fp.is_none());
        let by_profile = effective_profiles.iter().flat_map(|prof| {
            config_docs
                .iter()
                .filter(move |(g, fp, _)| *g == group && fp.as_ref() == Some(prof))
        });
        for (_, _, docs) in plain.chain(by_profile) {
            for (on_profile, parsed) in docs {
                if !doc_applies(on_profile) {
                    continue;
                }
                if let Some(p) = parsed
                    .get("server.port")
                    .and_then(|v| v.trim().parse::<u16>().ok())
                {
                    server_port = p;
                }
                if let Some(p) = parsed
                    .get("management.server.port")
                    .and_then(|v| v.trim().parse::<u16>().ok())
                {
                    management_port = Some(p);
                }
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
                if let Some(tf) = parsed
                    .get("spring.devtools.restart.trigger-file")
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty())
                {
                    trigger_file = Some(tf.to_string());
                }
            }
        }
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

/// One config file: its location group (0 packaged, 1 external), the profile its name
/// carries, and its documents as `(on-profile, key -> value)`.
type ConfigFileDocs = (
    u8,
    Option<String>,
    Vec<(Option<String>, std::collections::BTreeMap<String, String>)>,
);

/// Split a multi-document config file on YAML `---` and properties `#---` / `!---` lines.
fn split_config_documents(raw: &str) -> Vec<String> {
    let mut docs = vec![String::new()];
    for line in raw.lines() {
        if matches!(line.trim_end(), "---" | "#---" | "!---") {
            docs.push(String::new());
        } else if let Some(doc) = docs.last_mut() {
            doc.push_str(line);
            doc.push('\n');
        }
    }
    docs
}

/// Spring's profile expression tokens: names and the operators `(`, `)`, `&`, `|`, `!`.
fn profile_expr_tokens(expr: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in expr.char_indices() {
        if "()&|!".contains(c) {
            out.extend([&expr[start..i], &expr[i..i + 1]]);
            start = i + 1;
        }
    }
    out.push(&expr[start..]);
    out.into_iter()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect()
}

/// Whether the profile expression `expr` (`!a`, `a & b`, `a | b`, parentheses) matches
/// the active profiles; `None` when malformed, e.g. `a & b | c` without parentheses.
fn profile_expr_matches(expr: &str, active: &[String]) -> Option<bool> {
    let tokens = profile_expr_tokens(expr);
    let mut pos = 0;
    let matched = profile_expr_seq(&tokens, &mut pos, active)?;
    (pos == tokens.len()).then_some(matched)
}

/// Operands joined by one operator, up to a `)` or the end.
fn profile_expr_seq(t: &[&str], pos: &mut usize, active: &[String]) -> Option<bool> {
    let mut vals = vec![profile_expr_operand(t, pos, active)?];
    let mut op = None;
    while let Some(&tok) = t.get(*pos).filter(|tok| **tok != ")") {
        if !matches!(tok, "&" | "|") || op.is_some_and(|o| o != tok) {
            return None;
        }
        op = Some(tok);
        *pos += 1;
        vals.push(profile_expr_operand(t, pos, active)?);
    }
    Some(if op == Some("&") {
        vals.iter().all(|v| *v)
    } else {
        vals.iter().any(|v| *v)
    })
}

fn profile_expr_operand(t: &[&str], pos: &mut usize, active: &[String]) -> Option<bool> {
    let tok = *t.get(*pos)?;
    *pos += 1;
    match tok {
        "!" => profile_expr_operand(t, pos, active).map(|v| !v),
        "(" => {
            let v = profile_expr_seq(t, pos, active)?;
            if t.get(*pos) != Some(&")") {
                return None;
            }
            *pos += 1;
            Some(v)
        }
        "&" | "|" | ")" => None,
        name => Some(active.iter().any(|a| a == name)),
    }
}

/// `a, b` -> `["a", "b"]` in declared order, skipping placeholders.
fn profile_list(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in raw.split(',').map(str::trim) {
        if !p.is_empty() && !p.contains("${") && !out.iter().any(|o| o == p) {
            out.push(p.to_string());
        }
    }
    out
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

/// Walk up to 6 confined `<parent>` POM hops starting from `module_root/pom.xml`
/// and inherit direct `<dependencies>` signals (`spring-boot-devtools` and
/// `spring-boot-starter-actuator`), excluding XML comments and BOM-only
/// `<dependencyManagement>` blocks (BUG3-6).
#[allow(clippy::disallowed_methods)]
fn inherit_maven_parent_spring_capabilities(
    module_root: &Path,
    allowed_roots: &[PathBuf],
    is_root_user: bool,
    has_actuator: &mut bool,
    has_devtools: &mut bool,
) {
    let mut visited = std::collections::HashSet::new();
    let mut cur_pom = module_root.join("pom.xml");
    if let Ok(canon) = fs::canonicalize(&cur_pom) {
        visited.insert(canon);
    }

    for _ in 0..6 {
        if *has_actuator && *has_devtools {
            break;
        }
        let Some(raw) = read_confined_bounded(&cur_pom, 128 * 1024, allowed_roots, is_root_user)
        else {
            break;
        };
        let stripped = crate::java_workspace::strip_xml_comments(&raw);
        let Some(parent_block) =
            crate::java_workspace::extract_first_tag_block(&stripped, "parent")
        else {
            break;
        };
        // Explicit empty `<relativePath/>` or `<relativePath></relativePath>` tells Maven
        // to resolve the parent exclusively from the repository, never from a local file.
        let has_self_closing_empty =
            parent_block.contains("<relativePath/>") || parent_block.contains("<relativePath />");
        if has_self_closing_empty {
            break;
        }
        let rel = match crate::java_workspace::extract_tag_text(parent_block, "relativePath") {
            Some(v) if v.is_empty() => break,
            Some(v) => v,
            None => "../pom.xml".to_string(),
        };
        let Some(cur_dir) = cur_pom.parent() else {
            break;
        };
        let cand = cur_dir.join(&rel);
        let next_pom = if cand.is_dir() {
            cand.join("pom.xml")
        } else {
            cand
        };
        let Ok(canon_next) = fs::canonicalize(&next_pom) else {
            break;
        };
        if !visited.insert(canon_next.clone()) {
            break;
        }
        let Some(parent_raw) =
            read_confined_bounded(&canon_next, 128 * 1024, allowed_roots, is_root_user)
        else {
            break;
        };
        let parent_stripped = crate::java_workspace::strip_xml_comments(&parent_raw);
        // Maven reads the local file only when it IS the declared parent; otherwise it
        // resolves the parent from the repository (BUG3-6).
        let own_coords = [
            "parent",
            "dependencyManagement",
            "dependencies",
            "build",
            "profiles",
        ]
        .iter()
        .fold(parent_stripped.clone(), |xml, tag| {
            crate::java_workspace::remove_all_tag_blocks(&xml, tag)
        });
        if crate::java_workspace::extract_tag_text(&own_coords, "artifactId")
            != crate::java_workspace::extract_tag_text(parent_block, "artifactId")
        {
            break;
        }
        let parent_deps =
            crate::java_workspace::remove_all_tag_blocks(&parent_stripped, "dependencyManagement");
        if parent_deps.contains("spring-boot-starter-actuator") {
            *has_actuator = true;
        }
        if parent_deps.contains("spring-boot-devtools") {
            *has_devtools = true;
        }
        cur_pom = canon_next;
    }
}
