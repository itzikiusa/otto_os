//! Live model-catalog discovery for `/api/v1/meta` (`MetaResp.provider_models`).
//!
//! Reads each CLI's local model cache / `models` command behind a 60 s
//! single-flight cache so `/meta` stays fast, and falls back to curated model
//! catalogs when a CLI's local cache is absent or unreachable.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use otto_core::api::ProviderModelOption;
use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;

const MODELS_TTL: Duration = Duration::from_secs(60);

type ModelsCache = Option<(
    Instant,
    Vec<String>,
    Option<String>,
    HashMap<String, Vec<ProviderModelOption>>,
)>;

/// Discover selectable models per provider at most once per [`MODELS_TTL`].
pub async fn probe_provider_models_cached(
    providers: &[String],
    agy_program: Option<String>,
) -> HashMap<String, Vec<ProviderModelOption>> {
    static CACHE: tokio::sync::Mutex<ModelsCache> = tokio::sync::Mutex::const_new(None);
    let mut slot = CACHE.lock().await;
    if let Some((at, cached_providers, cached_agy, models)) = slot.as_ref() {
        if at.elapsed() < MODELS_TTL
            && cached_providers.as_slice() == providers
            && *cached_agy == agy_program
        {
            return models.clone();
        }
    }

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));

    let want_claude = providers.iter().any(|p| p == "claude");
    let want_codex = providers.iter().any(|p| p == "codex");
    let want_agy = providers.iter().any(|p| p == "agy");

    let mut out = HashMap::new();
    if want_claude {
        out.insert("claude".to_string(), discover_claude_models(&home).await);
    }
    if want_codex {
        out.insert(
            "codex".to_string(),
            discover_codex_models(&codex_home).await,
        );
    }
    if want_agy {
        let agy = discover_agy_models(agy_program.as_deref()).await;
        out.insert("agy".to_string(), agy);
    }

    *slot = Some((Instant::now(), providers.to_vec(), agy_program, out.clone()));
    out
}

fn default_efforts() -> Vec<String> {
    vec![
        "low".into(),
        "medium".into(),
        "high".into(),
        "xhigh".into(),
        "max".into(),
    ]
}

async fn discover_claude_models(home: &Path) -> Vec<ProviderModelOption> {
    let dir = home.join(".claude/cache/model-catalog");
    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
        let mut candidates: Vec<(u64, String)> = Vec::new();
        while let Ok(Some(e)) = entries.next_entry().await {
            let p = e.path();
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with("-cc.json") && !name.ends_with("-ccd.json") {
                continue;
            }
            let Ok(raw) = tokio::fs::read_to_string(&p).await else {
                continue;
            };
            let fetched = serde_json::from_str::<Value>(&raw)
                .ok()
                .and_then(|v| v.get("fetchedAt").and_then(Value::as_u64))
                .unwrap_or(0);
            // Prefer `-cc.json` (CLI surface) slightly over `-ccd.json` when timestamps tie.
            let bonus = u64::from(name.ends_with("-cc.json"));
            candidates.push((fetched.saturating_mul(2).saturating_add(bonus), raw));
        }
        candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
        for (_, raw) in candidates {
            let parsed = parse_claude_catalog_json(&raw);
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }
    fallback_claude_models()
}

pub fn parse_claude_catalog_json(raw: &str) -> Vec<ProviderModelOption> {
    let Ok(root) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let Some(models) = root
        .pointer("/catalog/config/models")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut main = Vec::new();
    let mut overflow = Vec::new();
    for item in models {
        let Some(id) = item.get("id").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let label = item
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(id)
            .to_string();
        let description = item
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let default_effort = item
            .pointer("/runtime/default_effort")
            .and_then(Value::as_str)
            .map(str::to_string);
        let supported_efforts: Vec<String> = item
            .pointer("/runtime/effort_levels")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let opt = ProviderModelOption {
            id: id.to_string(),
            label,
            description,
            default_effort,
            supported_efforts,
        };
        if item.get("section").and_then(Value::as_str) == Some("main") {
            main.push(opt);
        } else {
            overflow.push(opt);
        }
    }
    main.extend(overflow);
    main
}

fn fallback_claude_models() -> Vec<ProviderModelOption> {
    let efforts = default_efforts();
    vec![
        ProviderModelOption {
            id: "opus".into(),
            label: "Claude Opus".into(),
            description: "Most capable model for complex reasoning and everyday coding".into(),
            default_effort: Some("high".into()),
            supported_efforts: efforts.clone(),
        },
        ProviderModelOption {
            id: "sonnet".into(),
            label: "Claude Sonnet".into(),
            description: "Balanced speed and intelligence for daily engineering".into(),
            default_effort: Some("medium".into()),
            supported_efforts: efforts.clone(),
        },
        ProviderModelOption {
            id: "haiku".into(),
            label: "Claude Haiku".into(),
            description: "Fastest responses for lightweight edits and questions".into(),
            default_effort: Some("low".into()),
            supported_efforts: vec!["low".into(), "medium".into(), "high".into()],
        },
    ]
}

async fn discover_codex_models(codex_home: &Path) -> Vec<ProviderModelOption> {
    let cache = codex_home.join("models_cache.json");
    if let Ok(raw) = tokio::fs::read_to_string(&cache).await {
        let parsed = parse_codex_models_cache_json(&raw);
        if !parsed.is_empty() {
            return parsed;
        }
    }
    fallback_codex_models()
}

pub fn parse_codex_models_cache_json(raw: &str) -> Vec<ProviderModelOption> {
    let Ok(root) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let Some(models) = root.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in models {
        let vis = item
            .get("visibility")
            .and_then(Value::as_str)
            .unwrap_or("list");
        if vis != "list" {
            continue;
        }
        let Some(slug) = item.get("slug").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if slug.is_empty() {
            continue;
        }
        let label = item
            .get("display_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(slug)
            .to_string();
        let description = item
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let default_effort = item
            .get("default_reasoning_level")
            .and_then(Value::as_str)
            .map(str::to_string);
        let supported_efforts: Vec<String> = item
            .get("supported_reasoning_levels")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.get("effort").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        out.push(ProviderModelOption {
            id: slug.to_string(),
            label,
            description,
            default_effort,
            supported_efforts,
        });
    }
    out
}

fn fallback_codex_models() -> Vec<ProviderModelOption> {
    let efforts = vec![
        "low".into(),
        "medium".into(),
        "high".into(),
        "xhigh".into(),
        "max".into(),
    ];
    vec![
        ProviderModelOption {
            id: "gpt-5.4".into(),
            label: "GPT-5.4".into(),
            description: "Flagship reasoning model for coding and agentic workflows".into(),
            default_effort: Some("medium".into()),
            supported_efforts: efforts.clone(),
        },
        ProviderModelOption {
            id: "gpt-5.3-codex".into(),
            label: "GPT-5.3 Codex".into(),
            description: "Specialized Codex model tuned for software engineering".into(),
            default_effort: Some("medium".into()),
            supported_efforts: efforts,
        },
    ]
}

async fn discover_agy_models(program: Option<&str>) -> Vec<ProviderModelOption> {
    if let Some(prog) = program {
        if let Ok(Ok(out)) = timeout(
            Duration::from_secs(2),
            Command::new(prog)
                .arg("models")
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        {
            if out.status.success() {
                let parsed = parse_agy_models_output(&String::from_utf8_lossy(&out.stdout));
                if !parsed.is_empty() {
                    return parsed;
                }
            }
        }
    }
    fallback_agy_models()
}

pub fn parse_agy_models_output(raw: &str) -> Vec<ProviderModelOption> {
    let efforts = default_efforts();
    let mut out = Vec::new();
    for line in raw.lines() {
        let Some((id, label)) = line.split_once('\t') else {
            continue;
        };
        let id = id.trim();
        let label = label.trim();
        if id.is_empty() || label.is_empty() {
            continue;
        }
        let default_effort = if id.ends_with("-high") {
            Some("high".into())
        } else if id.ends_with("-low") {
            Some("low".into())
        } else {
            Some("medium".into())
        };
        out.push(ProviderModelOption {
            id: id.to_string(),
            label: label.to_string(),
            description: String::new(),
            default_effort,
            supported_efforts: efforts.clone(),
        });
    }
    out
}

fn fallback_agy_models() -> Vec<ProviderModelOption> {
    let efforts = default_efforts();
    vec![
        ProviderModelOption {
            id: "gemini-4-argon-high".into(),
            label: "Gemini 4 Argon (High)".into(),
            description: "High-reasoning Gemini model".into(),
            default_effort: Some("high".into()),
            supported_efforts: efforts.clone(),
        },
        ProviderModelOption {
            id: "gemini-4-argon-medium".into(),
            label: "Gemini 4 Argon (Medium)".into(),
            description: "Balanced Gemini model".into(),
            default_effort: Some("medium".into()),
            supported_efforts: efforts.clone(),
        },
        ProviderModelOption {
            id: "claude-opus-5-5-high".into(),
            label: "Claude Opus 5.5 (High)".into(),
            description: "Anthropic Opus via Antigravity".into(),
            default_effort: Some("high".into()),
            supported_efforts: efforts,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_codex_and_agy_catalogs() {
        let claude = parse_claude_catalog_json(
            r#"{"catalog":{"config":{"models":[
                {"id":"claude-old","name":"Old","section":"overflow","runtime":{"default_effort":"high","effort_levels":["low","high"]}},
                {"id":"claude-opus-5-5","name":"Opus 5.5","description":"Everyday tasks","section":"main","runtime":{"default_effort":"medium","effort_levels":["low","medium","high","xhigh","max"]}}
            ]}}}"#,
        );
        assert_eq!(claude.len(), 2);
        assert_eq!(
            claude[0].id, "claude-opus-5-5",
            "main section sorts ahead of overflow"
        );
        assert_eq!(claude[0].default_effort.as_deref(), Some("medium"));
        assert_eq!(claude[0].supported_efforts.len(), 5);

        let codex = parse_codex_models_cache_json(
            r#"{"models":[
                {"slug":"gpt-hidden","display_name":"Hidden","visibility":"hide"},
                {"slug":"gpt-6.1-sol","display_name":"GPT-6.1-Sol","description":"Workhorse","visibility":"list","default_reasoning_level":"low","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"},{"effort":"max"}]}
            ]}"#,
        );
        assert_eq!(codex.len(), 1);
        assert_eq!(codex[0].id, "gpt-6.1-sol");
        assert_eq!(codex[0].supported_efforts, vec!["low", "high", "max"]);

        let agy = parse_agy_models_output(
            "Fetching available models...\ngemini-4-argon-high\tGemini 4 Argon (High)\ngemini-4-argon-low\tGemini 4 Argon (Low)\n",
        );
        assert_eq!(agy.len(), 2);
        assert_eq!(agy[0].id, "gemini-4-argon-high");
        assert_eq!(agy[0].default_effort.as_deref(), Some("high"));
        assert_eq!(agy[1].default_effort.as_deref(), Some("low"));
    }
}
