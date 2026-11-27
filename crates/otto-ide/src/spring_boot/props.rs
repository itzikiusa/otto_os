//! Spring Boot `application.properties` / `application.yml` configuration intelligence:
//! curated property metadata, hover docs, `@Value("${...}")` / `@ConfigurationProperties`
//! bidirectional navigation, and fast diagnostics with Quick Fixes.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use crate::types::{
    IdeDiagnostic, IdeHoverDefinedIn, IdeHoverResp, IdeQuickFix, IdeReferenceLocation, IdeTextEdit,
};

#[derive(Debug, Clone, Copy)]
pub struct SpringPropMeta {
    pub key: &'static str,
    pub value_type: &'static str,
    pub default_value: Option<&'static str>,
    pub description: &'static str,
    pub deprecated_replacement: Option<&'static str>,
    pub allowed_values: &'static [&'static str],
}

pub static SPRING_BOOT_PROPERTIES: &[SpringPropMeta] = &[
    SpringPropMeta {
        key: "server.port",
        value_type: "integer",
        default_value: Some("8080"),
        description: "Server HTTP port.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "server.address",
        value_type: "string",
        default_value: None,
        description: "Network address to which the server should bind.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "server.servlet.context-path",
        value_type: "string",
        default_value: Some("/"),
        description: "Context path of the application.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "server.context-path",
        value_type: "string",
        default_value: None,
        description: "Deprecated Spring Boot 1.x context path property.",
        deprecated_replacement: Some("server.servlet.context-path"),
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "server.shutdown",
        value_type: "enum",
        default_value: Some("immediate"),
        description: "Type of shutdown that the server will support.",
        deprecated_replacement: None,
        allowed_values: &["immediate", "graceful"],
    },
    SpringPropMeta {
        key: "server.compression.enabled",
        value_type: "boolean",
        default_value: Some("false"),
        description: "Whether response compression is enabled.",
        deprecated_replacement: None,
        allowed_values: &["true", "false"],
    },
    SpringPropMeta {
        key: "spring.application.name",
        value_type: "string",
        default_value: None,
        description: "Application name (used by Actuator, tracing, and service discovery).",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.profiles.active",
        value_type: "string",
        default_value: None,
        description: "Comma-separated list of active Spring profiles.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.profiles.include",
        value_type: "string",
        default_value: None,
        description: "Unconditionally activate the specified comma-separated list of profiles.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.profiles",
        value_type: "string",
        default_value: None,
        description: "Deprecated in Spring Boot 2.4+. Use `spring.config.activate.on-profile`.",
        deprecated_replacement: Some("spring.config.activate.on-profile"),
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.config.activate.on-profile",
        value_type: "string",
        default_value: None,
        description: "Profile expression that should match for a multi-document config section to be active.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.config.import",
        value_type: "string",
        default_value: None,
        description: "Import additional configuration data (e.g., `optional:file:.env[.properties]`).",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.main.banner-mode",
        value_type: "enum",
        default_value: Some("console"),
        description: "Mode used to display the banner when the application runs.",
        deprecated_replacement: None,
        allowed_values: &["console", "log", "off"],
    },
    SpringPropMeta {
        key: "spring.main.web-application-type",
        value_type: "enum",
        default_value: None,
        description: "Flag to explicitly request a specific type of web application.",
        deprecated_replacement: None,
        allowed_values: &["none", "servlet", "reactive"],
    },
    SpringPropMeta {
        key: "spring.main.lazy-initialization",
        value_type: "boolean",
        default_value: Some("false"),
        description: "Whether initialization should be performed lazily.",
        deprecated_replacement: None,
        allowed_values: &["true", "false"],
    },
    SpringPropMeta {
        key: "spring.threads.virtual.enabled",
        value_type: "boolean",
        default_value: Some("false"),
        description: "Whether to use Java 21+ virtual threads for web & task executors.",
        deprecated_replacement: None,
        allowed_values: &["true", "false"],
    },
    SpringPropMeta {
        key: "spring.datasource.url",
        value_type: "string",
        default_value: None,
        description: "JDBC URL of the database.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.datasource.username",
        value_type: "string",
        default_value: None,
        description: "Login username of the database.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.datasource.password",
        value_type: "string",
        default_value: None,
        description: "Login password of the database.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.datasource.driver-class-name",
        value_type: "string",
        default_value: None,
        description: "Fully qualified name of the JDBC driver. Auto-detected from URL by default.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.datasource.hikari.maximum-pool-size",
        value_type: "integer",
        default_value: Some("10"),
        description: "Maximum number of connections in the HikariCP pool.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.jpa.hibernate.ddl-auto",
        value_type: "enum",
        default_value: Some("none"),
        description: "DDL mode (`none`, `validate`, `update`, `create`, `create-drop`).",
        deprecated_replacement: None,
        allowed_values: &["none", "validate", "update", "create", "create-drop"],
    },
    SpringPropMeta {
        key: "spring.jpa.show-sql",
        value_type: "boolean",
        default_value: Some("false"),
        description: "Whether to enable logging of SQL statements.",
        deprecated_replacement: None,
        allowed_values: &["true", "false"],
    },
    SpringPropMeta {
        key: "spring.jpa.open-in-view",
        value_type: "boolean",
        default_value: Some("true"),
        description: "Register OpenEntityManagerInViewInterceptor (recommended `false` for REST APIs).",
        deprecated_replacement: None,
        allowed_values: &["true", "false"],
    },
    SpringPropMeta {
        key: "spring.flyway.enabled",
        value_type: "boolean",
        default_value: Some("true"),
        description: "Whether to enable Flyway database migrations.",
        deprecated_replacement: None,
        allowed_values: &["true", "false"],
    },
    SpringPropMeta {
        key: "spring.liquibase.enabled",
        value_type: "boolean",
        default_value: Some("true"),
        description: "Whether to enable Liquibase database migrations.",
        deprecated_replacement: None,
        allowed_values: &["true", "false"],
    },
    SpringPropMeta {
        key: "spring.data.redis.host",
        value_type: "string",
        default_value: Some("localhost"),
        description: "Redis server host.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.data.redis.port",
        value_type: "integer",
        default_value: Some("6379"),
        description: "Redis server port.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.redis.host",
        value_type: "string",
        default_value: Some("localhost"),
        description: "Deprecated in Spring Boot 3.0+. Use `spring.data.redis.host`.",
        deprecated_replacement: Some("spring.data.redis.host"),
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.redis.port",
        value_type: "integer",
        default_value: Some("6379"),
        description: "Deprecated in Spring Boot 3.0+. Use `spring.data.redis.port`.",
        deprecated_replacement: Some("spring.data.redis.port"),
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.kafka.bootstrap-servers",
        value_type: "string",
        default_value: Some("localhost:9092"),
        description: "Comma-delimited list of host:port pairs for Kafka bootstrap connections.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.kafka.consumer.group-id",
        value_type: "string",
        default_value: None,
        description: "Unique string identifying the consumer group.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.jackson.serialization.indent-output",
        value_type: "boolean",
        default_value: Some("false"),
        description: "Pretty-print serialized JSON output.",
        deprecated_replacement: None,
        allowed_values: &["true", "false"],
    },
    SpringPropMeta {
        key: "spring.jackson.default-property-inclusion",
        value_type: "enum",
        default_value: Some("always"),
        description: "Controls inclusion of properties during JSON serialization.",
        deprecated_replacement: None,
        allowed_values: &["always", "non_null", "non_absent", "non_empty", "non_default"],
    },
    SpringPropMeta {
        key: "spring.devtools.restart.enabled",
        value_type: "boolean",
        default_value: Some("true"),
        description: "Whether to enable automatic restart via Spring Boot DevTools.",
        deprecated_replacement: None,
        allowed_values: &["true", "false"],
    },
    SpringPropMeta {
        key: "spring.devtools.restart.trigger-file",
        value_type: "string",
        default_value: None,
        description: "Name of a specific file that must be modified to trigger a DevTools restart check.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "spring.devtools.livereload.enabled",
        value_type: "boolean",
        default_value: Some("true"),
        description: "Whether to enable a LiveReload server.",
        deprecated_replacement: None,
        allowed_values: &["true", "false"],
    },
    SpringPropMeta {
        key: "management.server.port",
        value_type: "integer",
        default_value: None,
        description: "Management endpoint HTTP port (defaults to `server.port`).",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "management.endpoints.web.base-path",
        value_type: "string",
        default_value: Some("/actuator"),
        description: "Base path for Web Actuator endpoints.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "management.endpoints.web.exposure.include",
        value_type: "string",
        default_value: Some("health"),
        description: "Endpoint IDs exposed over HTTP (`*` or comma-separated list: `health,info,metrics,beans,mappings,env`).",
        deprecated_replacement: None,
        allowed_values: &["*", "health,info,metrics,beans,mappings,env", "health,info"],
    },
    SpringPropMeta {
        key: "management.endpoints.web.exposure.exclude",
        value_type: "string",
        default_value: None,
        description: "Endpoint IDs excluded from HTTP exposure.",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "management.endpoint.health.show-details",
        value_type: "enum",
        default_value: Some("never"),
        description: "When to show full health details.",
        deprecated_replacement: None,
        allowed_values: &["never", "when-authorized", "always"],
    },
    SpringPropMeta {
        key: "management.endpoint.health.probes.enabled",
        value_type: "boolean",
        default_value: Some("false"),
        description: "Whether to enable Kubernetes liveness and readiness probes.",
        deprecated_replacement: None,
        allowed_values: &["true", "false"],
    },
    SpringPropMeta {
        key: "logging.level.root",
        value_type: "enum",
        default_value: Some("INFO"),
        description: "Root logger severity level.",
        deprecated_replacement: None,
        allowed_values: &["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL", "OFF"],
    },
    SpringPropMeta {
        key: "logging.file.name",
        value_type: "string",
        default_value: None,
        description: "Log file name (for instance, `myapp.log`).",
        deprecated_replacement: None,
        allowed_values: &[],
    },
    SpringPropMeta {
        key: "logging.file",
        value_type: "string",
        default_value: None,
        description: "Deprecated in Spring Boot 2.2+. Use `logging.file.name`.",
        deprecated_replacement: Some("logging.file.name"),
        allowed_values: &[],
    },
];

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
    if let Some(exact) = SPRING_BOOT_PROPERTIES.iter().find(|p| p.key == key) {
        return Some(exact);
    }
    if key.starts_with("logging.level.") {
        return SPRING_BOOT_PROPERTIES
            .iter()
            .find(|p| p.key == "logging.level.root");
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
        doc_lines.push(m.description.to_string());
        if let Some(def) = m.default_value {
            doc_lines.push(format!("Default: `{def}`"));
        }
        if !m.allowed_values.is_empty() {
            doc_lines.push(format!(
                "Allowed values: `{}`",
                m.allowed_values.join("`, `")
            ));
        }
        if let Some(repl) = m.deprecated_replacement {
            doc_lines.push(format!("Deprecated — use `{repl}` instead."));
        }
        format!("{key}: {}", m.value_type)
    } else {
        format!("{key} (Spring Boot property)")
    };

    // If hovering inside Java/Kotlin code, also show configured value(s) from application config files.
    if !is_spring_config_path(path) {
        let locs = find_property_in_config_files(&key, project_roots);
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

/// Find definitions of a Spring Boot property key across `src/main/resources/application*.{properties,yml,yaml}`.
#[allow(clippy::disallowed_methods)]
pub fn find_property_in_config_files(
    key: &str,
    project_roots: &[PathBuf],
) -> Vec<(String, u32, String)> {
    let mut out = Vec::new();
    for root in project_roots {
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
                let Ok(raw) = fs::read_to_string(&f) else {
                    continue;
                };
                let f_str = f.to_string_lossy().into_owned();
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

/// Navigate between Java/Kotlin `@Value` / `@ConfigurationProperties` and `application*.{properties,yml}`.
#[allow(clippy::disallowed_methods)]
pub fn navigate_spring_config(
    path: &str,
    content: &str,
    line_1based: u32,
    project_roots: &[PathBuf],
) -> Option<(String, Vec<IdeReferenceLocation>)> {
    if path.ends_with(".java") || path.ends_with(".kt") {
        let raw_line = content
            .lines()
            .nth(line_1based.saturating_sub(1) as usize)?;
        let key = extract_spring_key_from_code_line(raw_line)?;
        let hits = find_property_in_config_files(&key, project_roots);
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
        for root in project_roots {
            for src_rel in ["src/main/java", "src/main/kotlin"] {
                let dir = root.join(src_rel);
                if dir.is_dir() {
                    let mut scanned = 0usize;
                    search_code_for_spring_key(&dir, &key, 0, &mut scanned, &mut locations);
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
        if entry.is_dir() {
            search_code_for_spring_key(&entry, key, depth + 1, scanned, out);
            continue;
        }
        let Some(ext) = entry.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        if ext != "java" && ext != "kt" {
            continue;
        }
        *scanned += 1;
        let Ok(raw) = fs::read_to_string(&entry) else {
            continue;
        };
        let p_str = entry.to_string_lossy().into_owned();
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
/// - deprecated Spring Boot properties (with concrete Quick Fix replacement edit)
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
            if let Some(repl) = meta.deprecated_replacement {
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
                                new_text: repl.to_string(),
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
            match meta.value_type {
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
