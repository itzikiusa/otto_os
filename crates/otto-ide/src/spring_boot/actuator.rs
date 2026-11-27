//! Loopback-only Spring Boot Actuator client (Health, Endpoints/Mappings, Beans,
//! redacted Env) and confined Spring Boot DevTools live restart trigger.

use std::path::Path;
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

fn is_sensitive_env_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "secret",
        "token",
        "apikey",
        "api_key",
        "api-key",
        "private",
        "credential",
        "auth",
        "cookie",
        "jwt",
        "cert",
        "key.store",
        "keystore",
        "truststore",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

async fn fetch_json_bounded(client: &reqwest::Client, url: &str) -> Option<Value> {
    let mut resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if buf.len().saturating_add(chunk.len()) > MAX_ACTUATOR_BODY_BYTES {
            return None;
        }
        buf.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&buf).ok()
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

    // Dedicated loopback-only client with redirects disabled so a local endpoint cannot redirect off-host.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(ACTUATOR_TIMEOUT)
        .timeout(ACTUATOR_TIMEOUT)
        .user_agent("otto-ide-spring-actuator/0.7")
        .build()
        .map_err(|e| Error::Internal(format!("build actuator http client: {e}")))?;

    let health_url = format!("{base_url}/health");
    let mappings_url = format!("{base_url}/mappings");
    let beans_url = format!("{base_url}/beans");
    let env_url = format!("{base_url}/env");
    let (root_v, health_v, mappings_v, beans_v, env_v) = tokio::join!(
        fetch_json_bounded(&client, &base_url),
        fetch_json_bounded(&client, &health_url),
        fetch_json_bounded(&client, &mappings_url),
        fetch_json_bounded(&client, &beans_url),
        fetch_json_bounded(&client, &env_url),
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
            for (k, vobj) in pobj {
                if props.len() >= MAX_ENV_PROPS {
                    return (active_profiles, props);
                }
                let redacted = is_sensitive_env_key(k);
                let value = if redacted {
                    "******".to_string()
                } else {
                    match vobj.get("value") {
                        Some(Value::String(s)) => s.clone(),
                        Some(other) => other.to_string(),
                        None => String::new(),
                    }
                };
                props.push(IdeSpringEnvProp {
                    key: k.clone(),
                    value,
                    source: src_name.clone(),
                    redacted,
                });
            }
        }
    }
    (active_profiles, props)
}

/// Touch or create the Spring Boot DevTools restart trigger file strictly inside `canon_root`.
pub async fn trigger_devtools_restart(
    canon_root: &Path,
    trigger_file_override: Option<&str>,
) -> Result<IdeSpringDevtoolsRestartResp> {
    let trigger_name = trigger_file_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(".reloadtrigger");

    // Reject path traversal or absolute paths outside the workspace root.
    if trigger_name.contains("..") || trigger_name.starts_with('/') || trigger_name.contains('\\') {
        return Err(Error::Invalid(format!(
            "invalid DevTools trigger file name '{trigger_name}'"
        )));
    }

    // Prefer compiled classpath output dirs (`target/classes`, `build/classes/java/main`,
    // `build/classes/kotlin/main`, `build/resources/main`) when present so Spring DevTools
    // sees the classpath change immediately; otherwise write into `src/main/resources`.
    let target_dir = [
        "target/classes",
        "build/classes/java/main",
        "build/classes/kotlin/main",
        "build/resources/main",
        "src/main/resources",
    ]
    .into_iter()
    .map(|rel| canon_root.join(rel))
    .find(|d| d.is_dir())
    .unwrap_or_else(|| canon_root.join("src/main/resources"));

    if !target_dir.starts_with(canon_root) {
        return Err(Error::Forbidden(
            "DevTools trigger path escaped workspace root".into(),
        ));
    }
    tokio::fs::create_dir_all(&target_dir)
        .await
        .map_err(|e| Error::Internal(format!("create DevTools trigger dir: {e}")))?;

    let canon_dir = tokio::fs::canonicalize(&target_dir)
        .await
        .map_err(|e| Error::Internal(format!("canonicalize DevTools trigger dir: {e}")))?;
    if !canon_dir.starts_with(canon_root) {
        return Err(Error::Forbidden(
            "DevTools trigger directory escaped workspace root".into(),
        ));
    }

    let trigger_path = canon_dir.join(trigger_name);
    if let Ok(meta) = tokio::fs::symlink_metadata(&trigger_path).await {
        if meta.file_type().is_symlink() {
            return Err(Error::Forbidden(
                "DevTools trigger file must not be a symlink".into(),
            ));
        }
    }
    let now = Utc::now().to_rfc3339();
    let body = format!("# Otto IDE Spring Boot DevTools restart trigger\nrestarted_at={now}\n");
    tokio::fs::write(&trigger_path, body)
        .await
        .map_err(|e| Error::Internal(format!("write DevTools trigger file: {e}")))?;

    Ok(IdeSpringDevtoolsRestartResp {
        ok: true,
        trigger_path: trigger_path.to_string_lossy().into_owned(),
        restarted_at: now,
        message: format!(
            "Triggered Spring Boot DevTools restart via {}",
            trigger_path.display()
        ),
    })
}
