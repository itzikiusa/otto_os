//! Spring Boot `application.properties` / `application.yml` configuration intelligence:
//! curated property metadata (shared with the UI from `docs/contracts/spring-boot-properties.json`),
//! hover docs, `@Value("${...}")` / `@ConfigurationProperties` bidirectional navigation,
//! and fast diagnostics (with replacement Quick Fixes in `.properties`).

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;

use crate::types::{
    IdeDiagnostic, IdeHoverDefinedIn, IdeHoverResp, IdeQuickFix, IdeReferenceLocation, IdeTextEdit,
};

#[derive(Debug, Clone, Deserialize)]
pub struct SpringPropMeta {
    pub key: String,
    pub value_type: String,
    #[serde(default)]
    pub default_value: Option<String>,
    pub description: String,
    #[serde(default)]
    pub deprecated_replacement: Option<String>,
    #[serde(default)]
    pub allowed_values: Vec<String>,
}

const SPRING_PROPS_CATALOG_JSON: &str =
    include_str!("../../../../docs/contracts/spring-boot-properties.json");

pub fn spring_boot_properties() -> &'static [SpringPropMeta] {
    static CATALOG: OnceLock<Vec<SpringPropMeta>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(SPRING_PROPS_CATALOG_JSON)
            .expect("docs/contracts/spring-boot-properties.json must be valid JSON")
    })
}

/// Verify that canonicalized `canon` resides strictly within `allowed_roots`,
/// never touches denied secret paths, and respects non-root confinement.
pub(crate) fn is_confined_spring_path(
    canon: &Path,
    allowed_roots: &[PathBuf],
    is_root_user: bool,
) -> bool {
    let within = allowed_roots.iter().any(|r| {
        #[allow(clippy::disallowed_methods)]
        let cr = fs::canonicalize(r).unwrap_or_else(|_| r.clone());
        canon.starts_with(&cr)
    });
    if !within {
        return false;
    }
    if otto_core::secret_paths::is_denied_file(canon)
        || otto_core::secret_paths::is_denied_dir(canon)
    {
        return false;
    }
    if !is_root_user && crate::target::enforce_non_root_path(canon).is_err() {
        return false;
    }
    true
}

#[allow(clippy::disallowed_methods)]
fn read_confined_text(
    candidate: &Path,
    allowed_roots: &[PathBuf],
    is_root_user: bool,
) -> Option<(PathBuf, String)> {
    let canon = fs::canonicalize(candidate).ok()?;
    if !canon.is_file() || !is_confined_spring_path(&canon, allowed_roots, is_root_user) {
        return None;
    }
    let text = fs::read_to_string(&canon).ok()?;
    Some((canon, text))
}

pub fn is_spring_config_filename(fname: &str) -> bool {
    let lower = fname.to_ascii_lowercase();
    (lower.starts_with("application") || lower.starts_with("bootstrap"))
        && (lower.ends_with(".properties") || lower.ends_with(".yml") || lower.ends_with(".yaml"))
}

pub fn is_spring_config_path(path: &str) -> bool {
    let base = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    is_spring_config_filename(base)
}

pub fn lookup_spring_property(key: &str) -> Option<&'static SpringPropMeta> {
    let catalog = spring_boot_properties();
    if let Some(exact) = catalog.iter().find(|p| p.key == key) {
        return Some(exact);
    }
    if key.starts_with("logging.level.") {
        return catalog.iter().find(|p| p.key == "logging.level.root");
    }
    None
}

/// Extract flattened `dotted.key -> value` pairs from a `.properties` or `.yml`/`.yaml` file.
pub fn extract_config_map(path: &str, content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for (key, val, _line) in extract_config_entries(path, content) {
        map.insert(key, val);
    }
    map
}

/// Extract `(dotted_key, value, 1-based line)` entries from `.properties` or `.yml`/`.yaml`.
pub fn extract_config_entries(path: &str, content: &str) -> Vec<(String, String, u32)> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".properties") {
        let mut out = Vec::new();
        for (idx, raw) in content.lines().enumerate() {
            let t = raw.trim();
            if t.is_empty() || t.starts_with('#') || t.starts_with('!') {
                continue;
            }
            if let Some(sep) = t.find(['=', ':']) {
                let k = t[..sep].trim();
                let v = t[sep + 1..].trim();
                if !k.is_empty() {
                    out.push((k.to_string(), v.to_string(), (idx + 1) as u32));
                }
            }
        }
        return out;
    }

    // YAML dotted-path stack by indentation width.
    let mut out = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    for (idx, raw) in content.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" {
            if trimmed == "---" {
                stack.clear();
            }
            continue;
        }
        if trimmed.starts_with("- ") {
            continue;
        }
        let indent = raw.chars().take_while(|c| *c == ' ').count();
        let Some((lhs, rhs)) = trimmed.split_once(':') else {
            continue;
        };
        let key_part = lhs.trim().trim_matches(['"', '\'']);
        if key_part.is_empty() {
            continue;
        }
        while stack.last().is_some_and(|(ind, _)| *ind >= indent) {
            stack.pop();
        }
        stack.push((indent, key_part.to_string()));
        let full_key = stack
            .iter()
            .map(|(_, seg)| seg.as_str())
            .collect::<Vec<_>>()
            .join(".");
        let val = rhs
            .split(" #")
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches(['"', '\''])
            .to_string();
        if !val.is_empty() {
            out.push((full_key, val, (idx + 1) as u32));
        } else {
            // Also record prefix node so navigation to `@ConfigurationProperties(prefix = "...")` matches.
            out.push((full_key, String::new(), (idx + 1) as u32));
        }
    }
    out
}

/// Extract the full Spring Boot dotted property key on `line_1based` of a config file.
pub fn extract_config_key_at_line(path: &str, content: &str, line_1based: u32) -> Option<String> {
    let entries = extract_config_entries(path, content);
    entries
        .into_iter()
        .find(|(_, _, ln)| *ln == line_1based)
        .map(|(k, _, _)| k)
}

/// Extract a Spring Boot property key from Java/Kotlin `@Value("${foo.bar:default}")`
/// or `@ConfigurationProperties(prefix = "foo.bar")` on `line_1based`.
pub fn extract_spring_key_from_code_line(line_text: &str) -> Option<String> {
    static RE_VAL: OnceLock<Regex> = OnceLock::new();
    static RE_CFG: OnceLock<Regex> = OnceLock::new();
    let re_val =
        RE_VAL.get_or_init(|| Regex::new(r#"\$\{\s*([A-Za-z0-9_.-]+)(?::[^}]*)?\}"#).unwrap());
    if let Some(cap) = re_val.captures(line_text) {
        return Some(cap[1].to_string());
    }
    let re_cfg = RE_CFG.get_or_init(|| {
        Regex::new(
            r#"@ConfigurationProperties\s*\(\s*(?:(?:prefix|value)\s*=\s*)?"([A-Za-z0-9_.-]+)""#,
        )
        .unwrap()
    });
    re_cfg.captures(line_text).map(|cap| cap[1].to_string())
}

/// Hover documentation for a Spring Boot property key in `application*.{properties,yml}`
/// or inside `@Value("${...}")` / `@ConfigurationProperties` in Java/Kotlin.
pub fn hover_spring_config(
    path: &str,
    content: &str,
    line_1based: u32,
    project_roots: &[PathBuf],
    is_root_user: bool,
) -> Option<IdeHoverResp> {
    let key = if is_spring_config_path(path) {
        extract_config_key_at_line(path, content, line_1based)?
    } else if path.ends_with(".java") || path.ends_with(".kt") {
        let raw_line = content
            .lines()
            .nth(line_1based.saturating_sub(1) as usize)?;
        extract_spring_key_from_code_line(raw_line)?
    } else {
        return None;
    };

    let meta = lookup_spring_property(&key);
    let mut doc_lines = Vec::new();
    let sig = if let Some(m) = meta {
        doc_lines.push(m.description.clone());
        if let Some(ref def) = m.default_value {
            doc_lines.push(format!("Default: `{def}`"));
        }
        if !m.allowed_values.is_empty() {
            doc_lines.push(format!(
                "Allowed values: `{}`",
                m.allowed_values.join("`, `")
            ));
        }
        if let Some(ref repl) = m.deprecated_replacement {
            doc_lines.push(format!("Deprecated — use `{repl}` instead."));
        }
        format!("{key}: {}", m.value_type)
    } else {
        format!("{key} (Spring Boot property)")
    };

    // If hovering inside Java/Kotlin code, also show configured value(s) from confined config files.
    if !is_spring_config_path(path) {
        let locs = find_property_in_config_files(&key, project_roots, is_root_user);
        if locs.is_empty() && meta.is_none() {
            return None;
        }
        for (cfg_path, _ln, val) in locs.into_iter().take(4) {
            let fname = Path::new(&cfg_path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&cfg_path);
            doc_lines.push(format!("Configured in `{fname}`: `{val}`"));
        }
    }

    let full = doc_lines.join("\n");
    Some(IdeHoverResp {
        found: true,
        symbol: Some(key),
        kind: Some("property".into()),
        package_name: Some("Spring Boot".into()),
        signature: Some(sig),
        summary: doc_lines.first().cloned(),
        full_doc: (!full.is_empty()).then_some(full),
        defined_in: Some(IdeHoverDefinedIn {
            path: path.to_string(),
            line: line_1based,
        }),
    })
}

#[allow(clippy::disallowed_methods)]
fn collect_spring_search_roots(project_roots: &[PathBuf], is_root_user: bool) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for root in project_roots {
        if !out.contains(root) {
            out.push(root.clone());
        }
        let Ok(rd) = fs::read_dir(root) else {
            continue;
        };
        let mut children: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        children.sort();
        for child in children {
            let Ok(meta) = fs::symlink_metadata(&child) else {
                continue;
            };
            if meta.file_type().is_symlink() || !meta.is_dir() {
                continue;
            }
            if !child.join("pom.xml").is_file()
                && !child.join("build.gradle").is_file()
                && !child.join("build.gradle.kts").is_file()
            {
                continue;
            }
            if let Ok(canon) = fs::canonicalize(&child) {
                if is_confined_spring_path(&canon, project_roots, is_root_user)
                    && !out.contains(&canon)
                {
                    out.push(canon);
                }
            }
        }
    }
    out
}

/// Find definitions of a Spring Boot property key across confined `application*.{properties,yml,yaml}` files.
#[allow(clippy::disallowed_methods)]
pub fn find_property_in_config_files(
    key: &str,
    project_roots: &[PathBuf],
    is_root_user: bool,
) -> Vec<(String, u32, String)> {
    let mut out = Vec::new();
    let search_roots = collect_spring_search_roots(project_roots, is_root_user);
    for root in &search_roots {
        for rel in ["src/main/resources", "config"] {
            let dir = root.join(rel);
            let Ok(rd) = fs::read_dir(&dir) else {
                continue;
            };
            let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            files.sort();
            for f in files {
                let Some(fname) = f.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if !is_spring_config_filename(fname) {
                    continue;
                }
                let Some((canon, raw)) = read_confined_text(&f, project_roots, is_root_user) else {
                    continue;
                };
                let f_str = canon.to_string_lossy().into_owned();
                for (k, v, ln) in extract_config_entries(&f_str, &raw) {
                    if k == key || k.starts_with(&format!("{key}.")) {
                        out.push((f_str.clone(), ln, v));
                        break;
                    }
                }
            }
        }
    }
    out
}

/// Navigate between Java/Kotlin `@Value` / `@ConfigurationProperties` and confined `application*.{properties,yml}`.
#[allow(clippy::disallowed_methods)]
pub fn navigate_spring_config(
    path: &str,
    content: &str,
    line_1based: u32,
    project_roots: &[PathBuf],
    is_root_user: bool,
) -> Option<(String, Vec<IdeReferenceLocation>)> {
    if path.ends_with(".java") || path.ends_with(".kt") {
        let raw_line = content
            .lines()
            .nth(line_1based.saturating_sub(1) as usize)?;
        let key = extract_spring_key_from_code_line(raw_line)?;
        let hits = find_property_in_config_files(&key, project_roots, is_root_user);
        if hits.is_empty() {
            return None;
        }
        let locations = hits
            .into_iter()
            .map(|(p, ln, val)| IdeReferenceLocation {
                path: p,
                line: ln,
                column: 1,
                preview: if val.is_empty() {
                    key.clone()
                } else {
                    format!("{key} = {val}")
                },
            })
            .collect();
        return Some((key, locations));
    }

    if is_spring_config_path(path) {
        let key = extract_config_key_at_line(path, content, line_1based)?;
        let mut locations = Vec::new();
        let search_roots = collect_spring_search_roots(project_roots, is_root_user);
        for root in &search_roots {
            for src_rel in ["src/main/java", "src/main/kotlin"] {
                let dir = root.join(src_rel);
                if dir.is_dir() {
                    let mut scanned = 0usize;
                    search_code_for_spring_key(
                        &dir,
                        project_roots,
                        is_root_user,
                        &key,
                        0,
                        &mut scanned,
                        &mut locations,
                    );
                }
            }
        }
        if !locations.is_empty() {
            return Some((key, locations));
        }
    }
    None
}

#[allow(clippy::disallowed_methods)]
fn search_code_for_spring_key(
    dir: &Path,
    allowed_roots: &[PathBuf],
    is_root_user: bool,
    key: &str,
    depth: usize,
    scanned: &mut usize,
    out: &mut Vec<IdeReferenceLocation>,
) {
    if depth > 7 || *scanned >= 120 || out.len() >= 16 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for entry in entries {
        if *scanned >= 120 || out.len() >= 16 {
            break;
        }
        let Ok(meta) = fs::symlink_metadata(&entry) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            search_code_for_spring_key(
                &entry,
                allowed_roots,
                is_root_user,
                key,
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
        let Some((canon, raw)) = read_confined_text(&entry, allowed_roots, is_root_user) else {
            continue;
        };
        let p_str = canon.to_string_lossy().into_owned();
        for (idx, line) in raw.lines().enumerate() {
            if let Some(code_key) = extract_spring_key_from_code_line(line) {
                if code_key == key || key.starts_with(&format!("{code_key}.")) {
                    out.push(IdeReferenceLocation {
                        path: p_str.clone(),
                        line: (idx + 1) as u32,
                        column: 1,
                        preview: line.trim().to_string(),
                    });
                }
            }
        }
    }
}

/// Fast diagnostics for Spring Boot `application*.{properties,yml,yaml}`:
/// - deprecated Spring Boot properties (with concrete Quick Fix replacement edit in `.properties`)
/// - duplicate keys in `.properties`
/// - invalid integer / boolean / enum literals on known typed keys (ignoring `${ENV_VAR}` placeholders)
pub fn analyze_spring_config_diagnostics(path: &str, content: &str) -> Vec<IdeDiagnostic> {
    if !is_spring_config_path(path) {
        return Vec::new();
    }
    let lines: Vec<&str> = content.lines().collect();
    let is_props = path.to_ascii_lowercase().ends_with(".properties");
    let mut diags = Vec::new();
    let mut seen_props: HashMap<String, u32> = HashMap::new();

    for (key, val, ln) in extract_config_entries(path, content) {
        let raw_line = lines.get((ln - 1) as usize).copied().unwrap_or("");
        let end_col = (raw_line.encode_utf16().count() as u32).max(1);

        if is_props {
            if let Some(&prev_ln) = seen_props.get(&key) {
                diags.push(IdeDiagnostic {
                    line: ln,
                    col: 1,
                    end_line: ln,
                    end_col,
                    severity: "warning".into(),
                    code: "spring_duplicate_property".into(),
                    source: "otto-spring".into(),
                    message: format!(
                        "Duplicate Spring property '{key}' (previously defined on line {prev_ln})"
                    ),
                    quick_fixes: Vec::new(),
                });
            } else {
                seen_props.insert(key.clone(), ln);
            }
        }

        if let Some(meta) = lookup_spring_property(&key) {
            if let Some(ref repl) = meta.deprecated_replacement {
                let mut fixes = Vec::new();
                if is_props {
                    if let Some(pos) = raw_line.find(&key) {
                        let start_col = (raw_line[..pos].encode_utf16().count() + 1) as u32;
                        let key_end_col = start_col + (key.encode_utf16().count() as u32);
                        let replaced_line = raw_line.replacen(&key, repl, 1);
                        fixes.push(IdeQuickFix {
                            title: format!("Replace deprecated '{key}' with '{repl}'"),
                            line: ln,
                            end_line: ln,
                            replacement: replaced_line,
                            delete_line: false,
                            is_preferred: true,
                            edits: vec![IdeTextEdit {
                                path: path.to_string(),
                                start_line: ln,
                                start_col,
                                end_line: ln,
                                end_col: key_end_col,
                                new_text: repl.clone(),
                            }],
                        });
                    }
                }
                diags.push(IdeDiagnostic {
                    line: ln,
                    col: 1,
                    end_line: ln,
                    end_col,
                    severity: "warning".into(),
                    code: "spring_deprecated_property".into(),
                    source: "otto-spring".into(),
                    message: format!("Property '{key}' is deprecated; use '{repl}' instead"),
                    quick_fixes: fixes,
                });
            }

            // Skip validation for placeholder expressions (`${PORT:8080}`) or empty parent YAML nodes.
            if val.is_empty() || val.contains("${") {
                continue;
            }
            match meta.value_type.as_str() {
                "integer" if val.parse::<i64>().is_err() => {
                    diags.push(IdeDiagnostic {
                        line: ln,
                        col: 1,
                        end_line: ln,
                        end_col,
                        severity: "error".into(),
                        code: "spring_invalid_type".into(),
                        source: "otto-spring".into(),
                        message: format!("Spring property '{key}' expects an integer, got '{val}'"),
                        quick_fixes: Vec::new(),
                    });
                }
                "boolean" if !matches!(val.to_ascii_lowercase().as_str(), "true" | "false") => {
                    diags.push(IdeDiagnostic {
                        line: ln,
                        col: 1,
                        end_line: ln,
                        end_col,
                        severity: "error".into(),
                        code: "spring_invalid_type".into(),
                        source: "otto-spring".into(),
                        message: format!(
                            "Spring property '{key}' expects a boolean (true/false), got '{val}'"
                        ),
                        quick_fixes: Vec::new(),
                    });
                }
                "enum"
                    if !meta.allowed_values.is_empty()
                        && !meta
                            .allowed_values
                            .iter()
                            .any(|v| v.eq_ignore_ascii_case(&val)) =>
                {
                    diags.push(IdeDiagnostic {
                        line: ln,
                        col: 1,
                        end_line: ln,
                        end_col,
                        severity: "warning".into(),
                        code: "spring_invalid_enum".into(),
                        source: "otto-spring".into(),
                        message: format!(
                            "Unexpected value '{val}' for '{key}' (expected one of: {})",
                            meta.allowed_values.join(", ")
                        ),
                        quick_fixes: Vec::new(),
                    });
                }
                _ => {}
            }
        }
    }

    diags
}
