//! Loopback-only Spring Boot Actuator client (Health, Endpoints/Mappings, Beans,
//! redacted Env) and confined Spring Boot DevTools live restart trigger.

use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use otto_core::{Error, Result};
use reqwest::Url;
use serde_json::Value;

use crate::types::{
    IdeSpringActuatorReq, IdeSpringActuatorResp, IdeSpringBean, IdeSpringDevtoolsRestartResp,
    IdeSpringEndpoint, IdeSpringEnvProp, IdeSpringHealthComponent,
};

const ACTUATOR_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_ACTUATOR_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_ENDPOINTS: usize = 500;
const MAX_BEANS: usize = 800;
const MAX_ENV_PROPS: usize = 200;

/// Validate that `raw_url` is strictly an `http://` loopback URL (`127.0.0.1`,
/// `localhost`, or `[::1]`) with no embedded credentials, preventing SSRF.
pub fn validate_loopback_actuator_url(raw_url: &str) -> Result<String> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return Err(Error::Invalid("actuator base_url must not be empty".into()));
    }
    let parsed = Url::parse(trimmed)
        .map_err(|e| Error::Invalid(format!("invalid actuator URL '{trimmed}': {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(Error::Forbidden(
            "actuator URL must use http or https scheme".into(),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(Error::Forbidden(
            "actuator URL must not contain embedded credentials".into(),
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| Error::Invalid("actuator URL missing host".into()))?;
    let is_loopback = host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "[::1]"
        || host == "::1";
    if !is_loopback {
        return Err(Error::Forbidden(format!(
            "Spring Boot Actuator inspector only allows loopback hosts (127.0.0.1, localhost, [::1]), rejected '{host}'"
        )));
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

async fn fetch_json_bounded(
    client: &reqwest::Client,
    url: &str,
    allow_service_unavailable: bool,
) -> Option<Value> {
    let mut resp = client.get(url).send().await.ok()?;
    let st = resp.status();
    if !st.is_success()
        && !(allow_service_unavailable && st == reqwest::StatusCode::SERVICE_UNAVAILABLE)
    {
        return None;
    }
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if buf.len().saturating_add(chunk.len()) > MAX_ACTUATOR_BODY_BYTES {
            return None;
        }
        buf.extend_from_slice(&chunk);
    }
    let parsed: Value = serde_json::from_slice(&buf).ok()?;
    if st == reqwest::StatusCode::SERVICE_UNAVAILABLE
        && parsed.get("status").and_then(Value::as_str).is_none()
    {
        return None;
    }
    Some(parsed)
}

/// Build the dedicated loopback-only Actuator HTTP client (redirects disabled, ambient proxies bypassed).
pub(crate) fn build_actuator_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(ACTUATOR_TIMEOUT)
        .timeout(ACTUATOR_TIMEOUT)
        .user_agent("otto-ide-spring-actuator/0.7")
        .build()
        .map_err(|e| Error::Internal(format!("build actuator http client: {e}")))
}

/// Query a locally running Spring Boot Actuator server (`127.0.0.1` / `localhost` / `[::1]` only)
/// for Health, HTTP Mappings/Endpoints, Beans, and redacted Environment properties.
pub async fn inspect_actuator(req: &IdeSpringActuatorReq) -> Result<IdeSpringActuatorResp> {
    let raw_base = req
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let port = req.port.unwrap_or(8080);
            let bp = req
                .base_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("/actuator");
            let bp_clean = if bp.starts_with('/') {
                bp.trim_end_matches('/').to_string()
            } else {
                format!("/{}", bp.trim_end_matches('/'))
            };
            format!("http://127.0.0.1:{port}{bp_clean}")
        });

    let base_url = validate_loopback_actuator_url(&raw_base)?;

    let client = build_actuator_http_client()?;

    let health_url = format!("{base_url}/health");
    let mappings_url = format!("{base_url}/mappings");
    let beans_url = format!("{base_url}/beans");
    let env_url = format!("{base_url}/env");
    let (root_v, health_v, mappings_v, beans_v, env_v) = tokio::join!(
        fetch_json_bounded(&client, &base_url, false),
        fetch_json_bounded(&client, &health_url, true),
        fetch_json_bounded(&client, &mappings_url, false),
        fetch_json_bounded(&client, &beans_url, false),
        fetch_json_bounded(&client, &env_url, false),
    );

    let reachable = root_v.is_some()
        || health_v.is_some()
        || mappings_v.is_some()
        || beans_v.is_some()
        || env_v.is_some();

    let available_links = root_v
        .as_ref()
        .and_then(|v| v.get("_links"))
        .and_then(|v| v.as_object())
        .map(|m| m.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();

    let (status, health_components) = health_v
        .as_ref()
        .map(parse_actuator_health)
        .unwrap_or((None, Vec::new()));

    let endpoints = mappings_v
        .as_ref()
        .map(parse_actuator_mappings)
        .unwrap_or_default();

    let beans = beans_v
        .as_ref()
        .map(parse_actuator_beans)
        .unwrap_or_default();

    let (active_profiles, env_properties) =
        env_v.as_ref().map(parse_actuator_env).unwrap_or_default();

    Ok(IdeSpringActuatorResp {
        base_url,
        reachable,
        status,
        active_profiles,
        available_links,
        health_components,
        endpoints,
        beans,
        env_properties,
    })
}

pub fn parse_actuator_health(val: &Value) -> (Option<String>, Vec<IdeSpringHealthComponent>) {
    let status = val
        .get("status")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let mut comps = Vec::new();
    if let Some(obj) = val.get("components").and_then(|v| v.as_object()) {
        for (name, cval) in obj {
            let st = cval
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN")
                .to_string();
            let detail = cval.get("details").and_then(|d| {
                if d.is_object() {
                    serde_json::to_string(d).ok()
                } else {
                    None
                }
            });
            comps.push(IdeSpringHealthComponent {
                name: name.clone(),
                status: st,
                detail,
            });
        }
    }
    (status, comps)
}

pub fn parse_actuator_mappings(val: &Value) -> Vec<IdeSpringEndpoint> {
    let mut out = Vec::new();
    let Some(contexts) = val.get("contexts").and_then(|v| v.as_object()) else {
        return out;
    };
    for (_ctx_name, ctx_val) in contexts {
        let Some(mappings) = ctx_val.get("mappings").and_then(|v| v.as_object()) else {
            continue;
        };
        // Spring MVC (`dispatcherServlets`) and Spring WebFlux (`dispatcherHandlers`)
        for group_key in ["dispatcherServlets", "dispatcherHandlers"] {
            let Some(servlets) = mappings.get(group_key).and_then(|v| v.as_object()) else {
                continue;
            };
            for (_sname, list_val) in servlets {
                let Some(arr) = list_val.as_array() else {
                    continue;
                };
                for item in arr {
                    if out.len() >= MAX_ENDPOINTS {
                        return out;
                    }
                    let handler = item
                        .get("handler")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let cond = item
                        .pointer("/details/requestMappingConditions")
                        .or_else(|| item.pointer("/details/handlerMethod"));
                    let patterns: Vec<String> = cond
                        .and_then(|c| c.get("patterns"))
                        .and_then(|p| p.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_else(|| {
                            item.get("predicate")
                                .and_then(|v| v.as_str())
                                .map(|p| vec![p.to_string()])
                                .unwrap_or_default()
                        });
                    let methods: Vec<String> = cond
                        .and_then(|c| c.get("methods"))
                        .and_then(|m| m.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(str::to_string))
                                .collect()
                        })
                        .filter(|m: &Vec<String>| !m.is_empty())
                        .unwrap_or_else(|| vec!["ANY".to_string()]);
                    let produces: Vec<String> = cond
                        .and_then(|c| c.get("produces"))
                        .and_then(|p| p.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| {
                                    x.get("mediaType")
                                        .and_then(|m| m.as_str())
                                        .or_else(|| x.as_str())
                                        .map(str::to_string)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let consumes: Vec<String> = cond
                        .and_then(|c| c.get("consumes"))
                        .and_then(|p| p.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| {
                                    x.get("mediaType")
                                        .and_then(|m| m.as_str())
                                        .or_else(|| x.as_str())
                                        .map(str::to_string)
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    for path in patterns {
                        for method in &methods {
                            if out.len() >= MAX_ENDPOINTS {
                                return out;
                            }
                            out.push(IdeSpringEndpoint {
                                method: method.clone(),
                                path: path.clone(),
                                handler: handler.clone(),
                                produces: produces.clone(),
                                consumes: consumes.clone(),
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

pub fn parse_actuator_beans(val: &Value) -> Vec<IdeSpringBean> {
    let mut out = Vec::new();
    let Some(contexts) = val.get("contexts").and_then(|v| v.as_object()) else {
        return out;
    };
    for (ctx_name, ctx_val) in contexts {
        let Some(beans_obj) = ctx_val.get("beans").and_then(|v| v.as_object()) else {
            continue;
        };
        for (bean_name, bval) in beans_obj {
            if out.len() >= MAX_BEANS {
                return out;
            }
            let bean_type = bval
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let scope = bval
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("singleton")
                .to_string();
            let dependencies = bval
                .get("dependencies")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            out.push(IdeSpringBean {
                name: bean_name.clone(),
                bean_type,
                scope,
                context: ctx_name.clone(),
                dependencies,
            });
        }
    }
    out
}

pub fn parse_actuator_env(val: &Value) -> (Vec<String>, Vec<IdeSpringEnvProp>) {
    let active_profiles = val
        .get("activeProfiles")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let mut props = Vec::new();
    if let Some(sources) = val.get("propertySources").and_then(|v| v.as_array()) {
        for src in sources {
            let src_name = src
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("propertySource")
                .to_string();
            let Some(pobj) = src.get("properties").and_then(|v| v.as_object()) else {
                continue;
            };
            for (k, _vobj) in pobj {
                if props.len() >= MAX_ENV_PROPS {
                    return (active_profiles, props);
                }
                // Redact all Actuator `/env` values by default so credentials embedded in
                // connection URIs or custom property names are never exposed over the API.
                props.push(IdeSpringEnvProp {
                    key: k.clone(),
                    value: "******".to_string(),
                    source: src_name.clone(),
                    redacted: true,
                });
            }
        }
    }
    (active_profiles, props)
}

/// Touch (or create empty if absent) the Spring Boot DevTools restart trigger file strictly
/// inside a compiled classpath output directory of one of the authorized writable roots
/// (`allowed_roots`), resolving optional `module_root_override` against all authorized roots.
pub async fn trigger_devtools_restart(
    allowed_roots: &[PathBuf],
    trigger_file_override: Option<&str>,
    module_root_override: Option<&str>,
) -> Result<IdeSpringDevtoolsRestartResp> {
    let primary_root = allowed_roots
        .first()
        .ok_or_else(|| Error::Forbidden("no authorized writable workspace root".into()))?;
    let trigger_name = trigger_file_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(".reloadtrigger");

    // Reject path traversal, directory separators, NULs, or reserved dot names.
    if trigger_name == "."
        || trigger_name == ".."
        || trigger_name.contains("..")
        || trigger_name.contains('/')
        || trigger_name.contains('\\')
        || trigger_name.contains('\0')
    {
        return Err(Error::Invalid(format!(
            "invalid DevTools trigger file name '{trigger_name}'"
        )));
    }

    let (matched_root, effective_root) = match module_root_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(mr) => {
            let cand = tokio::fs::canonicalize(mr).await.map_err(|_| {
                Error::Invalid(format!("Spring Boot module root '{mr}' does not exist"))
            })?;
            let owner = allowed_roots
                .iter()
                .find(|r| cand.starts_with(r))
                .cloned()
                .ok_or_else(|| {
                    Error::Forbidden("Spring Boot module root escaped workspace roots".into())
                })?;
            if !cand.is_dir() {
                return Err(Error::Invalid(format!(
                    "Spring Boot module root '{mr}' is not a directory"
                )));
            }
            (owner, cand)
        }
        None => (primary_root.clone(), primary_root.clone()),
    };

    // Only touch inside an existing compiled classpath output directory so DevTools sees the
    // classpath change and we never pollute `src/main/resources` outside the runtime classpath.
    let target_dir = [
        "target/classes",
        "build/classes/java/main",
        "build/classes/kotlin/main",
        "build/resources/main",
    ]
    .into_iter()
    .map(|rel| effective_root.join(rel))
    .find(|d| d.is_dir())
    .ok_or_else(|| {
        Error::Invalid(
            "no compiled classpath output directory found (build or run the Spring Boot app with DevTools first)"
                .into(),
        )
    })?;

    let canon_dir = tokio::fs::canonicalize(&target_dir)
        .await
        .map_err(|e| Error::Internal(format!("canonicalize DevTools trigger dir: {e}")))?;
    if !canon_dir.starts_with(&matched_root) {
        return Err(Error::Forbidden(
            "DevTools trigger directory escaped workspace root".into(),
        ));
    }

    let trigger_path = canon_dir.join(trigger_name);
    let trigger_path_c = trigger_path.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        match std::fs::symlink_metadata(&trigger_path_c) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(Error::Forbidden(
                        "DevTools trigger file must not be a symlink".into(),
                    ));
                }
                if !meta.is_file() {
                    return Err(Error::Invalid(
                        "DevTools trigger path must be a regular file".into(),
                    ));
                }
                // Touch mtime in-place without truncating or modifying existing resource bytes.
                let file = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&trigger_path_c)
                    .map_err(|e| Error::Internal(format!("open DevTools trigger file: {e}")))?;
                file.set_modified(std::time::SystemTime::now())
                    .map_err(|e| Error::Internal(format!("touch DevTools trigger file: {e}")))?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&trigger_path_c)
                    .map_err(|e| Error::Internal(format!("create DevTools trigger file: {e}")))?;
            }
            Err(e) => {
                return Err(Error::Internal(format!("stat DevTools trigger file: {e}")));
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| Error::Internal(format!("DevTools trigger task failed: {e}")))??;

    let now = Utc::now().to_rfc3339();
    Ok(IdeSpringDevtoolsRestartResp {
        ok: true,
        trigger_path: trigger_path.to_string_lossy().into_owned(),
        requested_at: now,
        restarted_at: None,
        message: format!(
            "Updated DevTools trigger file {} (restart check requested)",
            trigger_path.display()
        ),
    })
}
