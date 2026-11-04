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
/// Past [`MODELS_TTL`] a catalog is still served — and refreshed behind the
/// response — for this long. `/meta` is on the app's boot path, and `agy
/// models` is a process spawn; model lists change on CLI upgrades, not
/// between two launches.
const MODELS_MAX_STALE: Duration = Duration::from_secs(24 * 3600);

/// What discovery needs: each enabled provider's catalog family
/// (`ProviderRegistry::model_families`) and the resolved programs of the CLIs
/// whose catalog comes from a `models` subcommand. Also the cache key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelProbe {
    /// `(provider, family)` — a custom provider wrapping `claude` lists
    /// Claude's models under its own name.
    pub families: Vec<(String, String)>,
    pub agy_program: Option<String>,
    pub grok_program: Option<String>,
    pub opencode_program: Option<String>,
    pub pi_program: Option<String>,
}

type ModelsCache = Option<(
    Instant,
    ModelProbe,
    HashMap<String, Vec<ProviderModelOption>>,
)>;

static CACHE: tokio::sync::Mutex<ModelsCache> = tokio::sync::Mutex::const_new(None);
static REFRESHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Discover selectable models per provider at most once per [`MODELS_TTL`];
/// a stale catalog for the same inputs is served while ONE background
/// refresh replaces it.
pub async fn probe_provider_models_cached(
    probe: &ModelProbe,
) -> HashMap<String, Vec<ProviderModelOption>> {
    use std::sync::atomic::Ordering;
    let mut slot = CACHE.lock().await;
    if let Some((at, cached, models)) = slot.as_ref() {
        if cached == probe {
            let age = at.elapsed();
            if age < MODELS_TTL {
                return models.clone();
            }
            if age < MODELS_MAX_STALE {
                if !REFRESHING.swap(true, Ordering::AcqRel) {
                    let probe = probe.clone();
                    tokio::spawn(async move {
                        // Cleared even if discovery panics.
                        struct Done;
                        impl Drop for Done {
                            fn drop(&mut self) {
                                REFRESHING.store(false, Ordering::Release);
                            }
                        }
                        let _done = Done;
                        let fresh = discover_models(&probe).await;
                        *CACHE.lock().await = Some((Instant::now(), probe, fresh));
                    });
                }
                return models.clone();
            }
        }
    }
    let out = discover_models(probe).await;
    *slot = Some((Instant::now(), probe.clone(), out.clone()));
    out
}

async fn discover_models(probe: &ModelProbe) -> HashMap<String, Vec<ProviderModelOption>> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));

    // One discovery per FAMILY, shared by every provider of it.
    let mut by_family: HashMap<&str, Vec<ProviderModelOption>> = HashMap::new();
    for (_, family) in &probe.families {
        if by_family.contains_key(family.as_str()) {
            continue;
        }
        let models = match family.as_str() {
            "claude" => discover_claude_models(&home).await,
            "codex" => discover_codex_models(&codex_home).await,
            "agy" => discover_agy_models(probe.agy_program.as_deref()).await,
            "grok" => discover_grok_models(probe.grok_program.as_deref()).await,
            "opencode" => {
                let out = run_models(probe.opencode_program.as_deref(), &["models"]).await;
                parse_opencode_models_output(&out)
            }
            "pi" => {
                let out =
                    run_models(probe.pi_program.as_deref(), &["--offline", "--list-models"]).await;
                parse_pi_models_output(&out)
            }
            _ => continue,
        };
        by_family.insert(family.as_str(), models);
    }
    probe
        .families
        .iter()
        .filter_map(|(provider, family)| {
            let models = by_family.get(family.as_str())?;
            (!models.is_empty()).then(|| (provider.clone(), models.clone()))
        })
        .collect()
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

/// `<program> <args…>`'s stdout + stderr (3 s cap), `""` when absent / failed.
async fn run_models(program: Option<&str>, args: &[&str]) -> String {
    let Some(prog) = program else {
        return String::new();
    };
    let Ok(Ok(out)) = timeout(
        Duration::from_secs(3),
        Command::new(prog)
            .args(args)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    else {
        return String::new();
    };
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}

/// `opencode models`: one `provider/model` per line.
pub fn parse_opencode_models_output(raw: &str) -> Vec<ProviderModelOption> {
    raw.lines()
        .map(str::trim)
        .filter(|l| {
            l.contains('/')
                && !l.contains(' ')
                && l.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        })
        .map(|id| ProviderModelOption {
            id: id.to_string(),
            label: id.to_string(),
            description: String::new(),
            default_effort: None,
            supported_efforts: Vec::new(),
        })
        .collect()
}

/// `pi --list-models`: a `provider model context max-out thinking images`
/// table; ids are `provider/model`, thinking models take efforts.
pub fn parse_pi_models_output(raw: &str) -> Vec<ProviderModelOption> {
    let mut out = Vec::new();
    let mut in_table = false;
    for line in raw.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.first() == Some(&"provider") && cols.get(1) == Some(&"model") {
            in_table = true;
            continue;
        }
        if !in_table || cols.len() < 5 {
            continue;
        }
        let id = format!("{}/{}", cols[0], cols[1]);
        if out.iter().any(|m: &ProviderModelOption| m.id == id) {
            continue;
        }
        let thinking = cols[4] == "yes";
        out.push(ProviderModelOption {
            label: cols[1].to_string(),
            description: format!("{} · {} context", cols[0], cols[2]),
            default_effort: thinking.then(|| "medium".to_string()),
            supported_efforts: if thinking {
                vec!["low".into(), "medium".into(), "high".into(), "xhigh".into()]
            } else {
                Vec::new()
            },
            id,
        });
    }
    out
}

/// `grok models` — answers even signed out (it lists the public catalog).
async fn discover_grok_models(program: Option<&str>) -> Vec<ProviderModelOption> {
    let Some(prog) = program else {
        return Vec::new();
    };
    let Ok(Ok(out)) = timeout(
        Duration::from_secs(3),
        Command::new(prog)
            .arg("models")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    else {
        return Vec::new();
    };
    // The list may land on either stream depending on the auth banner.
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    parse_grok_models_output(&text)
}

/// Parse `grok models`: the bullet lines under `Available models:`
/// (`  * grok-4.6 (default)`, `  - grok-4.5`). Deduped, in order.
pub fn parse_grok_models_output(raw: &str) -> Vec<ProviderModelOption> {
    let mut out: Vec<ProviderModelOption> = Vec::new();
    let mut in_list = false;
    for line in raw.lines() {
        let t = line.trim();
        if t.eq_ignore_ascii_case("available models:") {
            in_list = true;
            continue;
        }
        if !in_list {
            continue;
        }
        let Some(rest) = t.strip_prefix("* ").or_else(|| t.strip_prefix("- ")) else {
            if t.is_empty() {
                continue;
            }
            in_list = false;
            continue;
        };
        let id = rest.split_whitespace().next().unwrap_or("");
        if id.is_empty() || out.iter().any(|m| m.id == id) {
            continue;
        }
        out.push(ProviderModelOption {
            id: id.to_string(),
            label: id.to_string(),
            description: if rest.contains("(default)") {
                "Default model".into()
            } else {
                String::new()
            },
            default_effort: None,
            supported_efforts: Vec::new(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `/meta` answers from a stale catalog at once; one background refresh
    /// replaces it (no provider here, so the refresh is an empty map).
    #[tokio::test]
    async fn stale_catalog_is_served_then_refreshed_in_the_background() {
        use std::sync::atomic::Ordering;
        let marker = HashMap::from([("stale".to_string(), Vec::new())]);
        let aged = Instant::now()
            .checked_sub(Duration::from_secs(600))
            .unwrap();
        *CACHE.lock().await = Some((aged, ModelProbe::default(), marker));
        let served = probe_provider_models_cached(&ModelProbe::default()).await;
        assert!(served.contains_key("stale"), "served stale");
        for _ in 0..200 {
            if !REFRESHING.load(Ordering::Acquire) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(probe_provider_models_cached(&ModelProbe::default())
            .await
            .is_empty());
    }

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

        let grok = parse_grok_models_output(
            "You are not authenticated.\n\nDefault model: grok-4.6\n\nAvailable models:\n  * grok-4.6 (default)\n  - grok-4.5\n\nAvailable models:\n  * grok-4.6 (default)\n",
        );
        assert_eq!(
            grok.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["grok-4.6", "grok-4.5"]
        );

        let pi = parse_pi_models_output(
            "provider      model                context  max-out  thinking  images\nopenai-codex  gpt-5.5              272K     128K     yes       yes   \nx  tiny  8K  1K  no  no\n",
        );
        assert_eq!(pi[0].id, "openai-codex/gpt-5.5");
        assert_eq!(pi[0].supported_efforts.len(), 4);
        assert!(pi[1].supported_efforts.is_empty());

        let oc = parse_opencode_models_output(
            "anthropic/claude-sonnet-4\nopenai/gpt-5\nsome log line\n",
        );
        assert_eq!(oc.len(), 2);
    }

    /// Every provider of a family gets that family's list under its own name.
    #[tokio::test]
    async fn custom_providers_share_their_family_catalog() {
        let probe = ModelProbe {
            families: vec![
                ("claude".into(), "claude".into()),
                ("claude-work".into(), "claude".into()),
            ],
            ..Default::default()
        };
        let got = discover_models(&probe).await;
        assert!(!got["claude-work"].is_empty());
        assert_eq!(got["claude-work"], got["claude"]);
    }
}
