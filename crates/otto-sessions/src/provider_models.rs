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

#[path = "provider_models_catalog.rs"]
mod provider_models_catalog;
pub use provider_models_catalog::*;

const MODELS_TTL: Duration = Duration::from_secs(60);
/// Past [`MODELS_TTL`] a catalog is still served — and refreshed behind the
/// response — for this long. `/meta` is on the app's boot path, and `agy
/// models` is a process spawn; model lists change on CLI upgrades, not
/// between two launches.
const MODELS_MAX_STALE: Duration = Duration::from_secs(24 * 3600);
/// Cold-cache inline wait budget on `/meta` before serving local disk/fallback
/// catalogs and letting slow CLI subprocesses (`opencode models`) finish behind.
const MODELS_COLD_BUDGET: Duration = Duration::from_millis(250);

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

impl ModelProbe {
    /// Build a deterministic cache key from `model_families` and configured
    /// `tool_specs` without running synchronous PATH/filesystem `stat`s on
    /// `/meta` cache hits. Executable checks happen off-thread on refresh.
    pub fn from_tool_specs(
        families: Vec<(String, String)>,
        specs: &std::collections::BTreeMap<String, String>,
    ) -> Self {
        let program_of = |family: &str| {
            families
                .iter()
                .filter(|(_, f)| f == family)
                .find_map(|(p, _)| specs.get(p).cloned())
        };
        Self {
            agy_program: program_of("agy"),
            grok_program: program_of("grok"),
            opencode_program: program_of("opencode"),
            pi_program: program_of("pi"),
            families,
        }
    }
}

type ModelsCache = Option<(
    u64,
    Instant,
    ModelProbe,
    HashMap<String, Vec<ProviderModelOption>>,
)>;

static CACHE: tokio::sync::Mutex<ModelsCache> = tokio::sync::Mutex::const_new(None);
static CACHE_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static REFRESHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Synchronously invalidate `/meta`'s in-memory model catalog cache (for one
/// `provider` or all providers when `None`) so `POST /providers/models/refresh`
/// forces immediate re-discovery on the next `/meta` call.
pub fn invalidate_provider_models_cache(provider: Option<&str>) {
    use std::sync::atomic::Ordering;
    let _ = provider;
    CACHE_GEN.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut guard) = CACHE.try_lock() {
        *guard = None;
    }
}

/// Discover selectable models per provider at most once per [`MODELS_TTL`];
/// a stale catalog for the same inputs is served while ONE background
/// refresh replaces it.
pub async fn probe_provider_models_cached(
    probe: &ModelProbe,
) -> HashMap<String, Vec<ProviderModelOption>> {
    use std::sync::atomic::Ordering;
    let mut slot = CACHE.lock().await;
    let cur_gen = CACHE_GEN.load(Ordering::Acquire);
    if slot.as_ref().is_some_and(|(g, _, _, _)| *g != cur_gen) {
        *slot = None;
    }
    if let Some((_, at, cached, models)) = slot.as_ref() {
        if cached == probe {
            let age = at.elapsed();
            if age < MODELS_TTL {
                return models.clone();
            }
            if age < MODELS_MAX_STALE {
                if !REFRESHING.swap(true, Ordering::AcqRel) {
                    let probe = probe.clone();
                    let start_gen = cur_gen;
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
                        let mut guard = CACHE.lock().await;
                        if CACHE_GEN.load(Ordering::Acquire) == start_gen {
                            *guard = Some((start_gen, Instant::now(), probe, fresh));
                        }
                    });
                }
                return models.clone();
            }
        }
    }
    if REFRESHING.load(Ordering::Acquire) {
        if let Some((_, _, cached, models)) = slot.as_ref() {
            if cached == probe {
                return models.clone();
            }
        }
        return fast_models(probe).await;
    }
    let probe_bg = probe.clone();
    let start_gen = cur_gen;
    let (tx, rx) = tokio::sync::oneshot::channel::<HashMap<String, Vec<ProviderModelOption>>>();
    REFRESHING.store(true, Ordering::Release);
    tokio::spawn(async move {
        struct Done;
        impl Drop for Done {
            fn drop(&mut self) {
                REFRESHING.store(false, Ordering::Release);
            }
        }
        let _done = Done;
        let fresh = discover_models(&probe_bg).await;
        let _ = tx.send(fresh.clone());
        let mut guard = CACHE.lock().await;
        if CACHE_GEN.load(Ordering::Acquire) == start_gen {
            *guard = Some((start_gen, Instant::now(), probe_bg, fresh));
        }
    });
    match timeout(MODELS_COLD_BUDGET, rx).await {
        Ok(Ok(out)) => {
            if CACHE_GEN.load(Ordering::Acquire) == start_gen {
                *slot = Some((start_gen, Instant::now(), probe.clone(), out.clone()));
            }
            out
        }
        _ => {
            let provisional = fast_models(probe).await;
            let stale_at = Instant::now()
                .checked_sub(MODELS_TTL + Duration::from_secs(1))
                .unwrap_or_else(Instant::now);
            if CACHE_GEN.load(Ordering::Acquire) == start_gen {
                *slot = Some((start_gen, stale_at, probe.clone(), provisional.clone()));
            }
            provisional
        }
    }
}

async fn fast_models(probe: &ModelProbe) -> HashMap<String, Vec<ProviderModelOption>> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let mut by_family: HashMap<&str, Vec<ProviderModelOption>> = HashMap::new();
    for (_, family) in &probe.families {
        if by_family.contains_key(family.as_str()) {
            continue;
        }
        let models = match family.as_str() {
            "claude" => discover_claude_models(&home).await,
            "codex" => discover_codex_models(&codex_home).await,
            "agy" => fallback_agy_models(),
            "gemini" => fallback_gemini_models(),
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

async fn discover_models(probe: &ModelProbe) -> HashMap<String, Vec<ProviderModelOption>> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));

    // One discovery per FAMILY, spawned concurrently and shared by every provider of it.
    let mut seen_families = Vec::<String>::new();
    for (_, family) in &probe.families {
        if !seen_families.iter().any(|f| f == family) {
            seen_families.push(family.clone());
        }
    }
    let mut tasks = Vec::with_capacity(seen_families.len());
    for family in seen_families {
        let home = home.clone();
        let codex_home = codex_home.clone();
        let agy_program = probe.agy_program.clone();
        let grok_program = probe.grok_program.clone();
        let opencode_program = probe.opencode_program.clone();
        let pi_program = probe.pi_program.clone();
        tasks.push(tokio::spawn(async move {
            let models = match family.as_str() {
                "claude" => discover_claude_models(&home).await,
                "codex" => discover_codex_models(&codex_home).await,
                "agy" => {
                    let prog = resolve_cli_program(agy_program).await;
                    discover_agy_models(prog.as_deref()).await
                }
                "gemini" => fallback_gemini_models(),
                "grok" => {
                    let prog = resolve_cli_program(grok_program).await;
                    discover_grok_models(prog.as_deref()).await
                }
                "opencode" => {
                    let prog = resolve_cli_program(opencode_program).await;
                    let out = run_models(prog.as_deref(), &["models"], 20).await;
                    parse_opencode_models_output(&out)
                }
                "pi" => {
                    let prog = resolve_cli_program(pi_program).await;
                    let out =
                        run_models(prog.as_deref(), &["--offline", "--list-models"], 15).await;
                    parse_pi_models_output(&out)
                }
                _ => Vec::new(),
            };
            (family, models)
        }));
    }
    let mut by_family: HashMap<String, Vec<ProviderModelOption>> = HashMap::new();
    for task in tasks {
        if let Ok((family, models)) = task.await {
            by_family.insert(family, models);
        }
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

async fn resolve_cli_program(program: Option<String>) -> Option<String> {
    let prog = program?;
    tokio::task::spawn_blocking(move || {
        crate::tool_probe::executable_in_path_or_fs(&prog).then_some(prog)
    })
    .await
    .ok()
    .flatten()
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
            Duration::from_secs(20),
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

fn strip_ansi_line(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn is_agy_model_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.' | '_'))
}

fn effort_rank(effort: &str) -> usize {
    match effort {
        "low" => 0,
        "medium" => 1,
        "high" => 2,
        "xhigh" => 3,
        "max" => 4,
        "ultra" => 5,
        _ => 6,
    }
}

/// Split an `agy models` tier-suffixed entry (`gemini-4-carbon-medium` /
/// `"Gemini 4 Carbon (Medium)"`) into `(base_id, clean_label, effort)`.
/// Non-tier parentheticals like `"Orchestrator (Carbon)"` or
/// `"Claude Sonnet 4.6 (Thinking)"` return `None` and stay untouched.
fn split_agy_tier<'a>(id: &'a str, label: &'a str) -> Option<(&'a str, String, &'static str)> {
    const ID_SUFFIXES: [(&str, &str); 7] = [
        ("-minimal", "low"),
        ("-low", "low"),
        ("-medium", "medium"),
        ("-high", "high"),
        ("-xhigh", "xhigh"),
        ("-max", "max"),
        ("-ultra", "ultra"),
    ];
    let (base_id, effort) = ID_SUFFIXES.iter().find_map(|(suf, eff)| {
        let base = id.strip_suffix(suf)?;
        (!base.is_empty()).then_some((base, *eff))
    })?;
    if label == id {
        return Some((base_id, base_id.to_string(), effort));
    }
    let trimmed = label.strip_suffix(')')?;
    let (base_label, paren) = trimmed.rsplit_once('(')?;
    let is_tier_label = matches!(
        paren.trim().to_ascii_lowercase().as_str(),
        "minimal"
            | "low"
            | "medium"
            | "high"
            | "xhigh"
            | "x-high"
            | "extra high"
            | "extra"
            | "max"
            | "ultra"
    );
    if !is_tier_label {
        return None;
    }
    let clean = base_label.trim();
    (!clean.is_empty()).then(|| (base_id, clean.to_string(), effort))
}

pub fn parse_agy_models_output(raw: &str) -> Vec<ProviderModelOption> {
    enum Entry {
        Plain {
            id: String,
            label: String,
        },
        Family {
            base_id: String,
            first_full_id: String,
            label: String,
            first_effort: &'static str,
            efforts: Vec<&'static str>,
        },
    }

    let mut entries: Vec<Entry> = Vec::new();
    let mut seen_ids = std::collections::HashSet::<String>::new();
    for seg in raw.split(['\n', '\r']) {
        let cleaned = strip_ansi_line(seg);
        let line = cleaned.trim();
        if line.is_empty() {
            continue;
        }
        let (id, label) = match line.split_once('\t').or_else(|| line.split_once("  ")) {
            Some((i, l)) => (i.trim(), l.trim()),
            None => (line, line),
        };
        if !is_agy_model_id(id) || label.is_empty() || !seen_ids.insert(id.to_string()) {
            continue;
        }
        if let Some((base_id, clean_label, effort)) = split_agy_tier(id, label) {
            if let Some(Entry::Family { efforts, .. }) = entries
                .iter_mut()
                .find(|e| matches!(e, Entry::Family { base_id: b, .. } if b == base_id))
            {
                if !efforts.contains(&effort) {
                    efforts.push(effort);
                }
            } else {
                entries.push(Entry::Family {
                    base_id: base_id.to_string(),
                    first_full_id: id.to_string(),
                    label: clean_label,
                    first_effort: effort,
                    efforts: vec![effort],
                });
            }
        } else {
            entries.push(Entry::Plain {
                id: id.to_string(),
                label: label.to_string(),
            });
        }
    }

    entries
        .into_iter()
        .map(|entry| match entry {
            Entry::Plain { id, label } => ProviderModelOption {
                id,
                label,
                description: String::new(),
                default_effort: None,
                supported_efforts: Vec::new(),
            },
            Entry::Family {
                base_id,
                first_full_id,
                label,
                first_effort,
                mut efforts,
            } => {
                efforts.sort_by_key(|e| effort_rank(e));
                // Multi-tier families (`gemini-4-carbon`, `gemini-4-argon`,
                // `gemini-4-hydrogen`, `claude-opus-5-5`, …) register their
                // base slug in `agy` (`--model` and `session/set_model`); a
                // single-tier model (`gpt-oss-120b-medium`) only registers its
                // full ID while still displaying its clean family label.
                let id = if efforts.len() >= 2 {
                    base_id
                } else {
                    first_full_id
                };
                let default_effort = if first_effort != "low" {
                    first_effort
                } else if efforts.contains(&"medium") {
                    "medium"
                } else {
                    first_effort
                };
                ProviderModelOption {
                    id,
                    label,
                    description: String::new(),
                    default_effort: Some(default_effort.to_string()),
                    supported_efforts: efforts.into_iter().map(str::to_string).collect(),
                }
            }
        })
        .collect()
}

fn fallback_agy_models() -> Vec<ProviderModelOption> {
    let std_efforts = vec!["low".into(), "medium".into(), "high".into()];
    vec![
        ProviderModelOption {
            id: "gemini-4-carbon".into(),
            label: "Gemini 4 Carbon".into(),
            description: "Balanced flagship Gemini model".into(),
            default_effort: Some("medium".into()),
            supported_efforts: std_efforts.clone(),
        },
        ProviderModelOption {
            id: "gemini-4-argon".into(),
            label: "Gemini 4 Argon".into(),
            description: "High-capability Gemini reasoning model".into(),
            default_effort: Some("medium".into()),
            supported_efforts: std_efforts.clone(),
        },
        ProviderModelOption {
            id: "gemini-4-hydrogen".into(),
            label: "Gemini 4 Hydrogen".into(),
            description: "Deep-reasoning Gemini model".into(),
            default_effort: Some("high".into()),
            supported_efforts: vec!["low".into(), "medium".into(), "high".into(), "xhigh".into()],
        },
        ProviderModelOption {
            id: "gemini-pro-orchestrator".into(),
            label: "Orchestrator (Carbon)".into(),
            description: "Multi-agent orchestrator on Gemini Carbon".into(),
            default_effort: None,
            supported_efforts: Vec::new(),
        },
        ProviderModelOption {
            id: "gemini-pro-orchestrator-argon".into(),
            label: "Orchestrator (Argon)".into(),
            description: "Multi-agent orchestrator on Gemini Argon".into(),
            default_effort: None,
            supported_efforts: Vec::new(),
        },
        ProviderModelOption {
            id: "gemini-3.8-flash".into(),
            label: "Gemini 3.8 Flash".into(),
            description: "Fast low-latency Gemini model".into(),
            default_effort: Some("medium".into()),
            supported_efforts: std_efforts.clone(),
        },
        ProviderModelOption {
            id: "claude-opus-5-5".into(),
            label: "Claude Opus 5.5".into(),
            description: "Anthropic Opus via Antigravity".into(),
            default_effort: Some("medium".into()),
            supported_efforts: std_efforts.clone(),
        },
        ProviderModelOption {
            id: "claude-sonnet-5-5".into(),
            label: "Claude Sonnet 5.5".into(),
            description: "Anthropic Sonnet via Antigravity".into(),
            default_effort: Some("medium".into()),
            supported_efforts: std_efforts,
        },
        ProviderModelOption {
            id: "gpt-oss-120b-medium".into(),
            label: "GPT-OSS 120B".into(),
            description: "Open-weights 120B model via Antigravity".into(),
            default_effort: Some("medium".into()),
            supported_efforts: vec!["medium".into()],
        },
    ]
}

fn fallback_gemini_models() -> Vec<ProviderModelOption> {
    let efforts = vec!["low".into(), "medium".into(), "high".into()];
    vec![
        ProviderModelOption {
            id: "gemini-2.5-pro".into(),
            label: "Gemini 2.5 Pro".into(),
            description: "Flagship Gemini reasoning and coding model".into(),
            default_effort: Some("medium".into()),
            supported_efforts: efforts.clone(),
        },
        ProviderModelOption {
            id: "gemini-2.5-flash".into(),
            label: "Gemini 2.5 Flash".into(),
            description: "Fast multimodal Gemini model".into(),
            default_effort: Some("medium".into()),
            supported_efforts: efforts,
        },
    ]
}

/// `<program> <args…>`'s stdout + stderr (`secs` cap), `""` when absent / failed.
async fn run_models(program: Option<&str>, args: &[&str], secs: u64) -> String {
    let Some(prog) = program else {
        return String::new();
    };
    let Ok(Ok(out)) = timeout(
        Duration::from_secs(secs),
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
        Duration::from_secs(15),
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

    static TEST_CACHE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// `/meta` answers from a stale catalog at once; one background refresh
    /// replaces it (no provider here, so the refresh is an empty map).
    #[tokio::test]
    async fn stale_catalog_is_served_then_refreshed_in_the_background() {
        let _serial = TEST_CACHE_LOCK.lock().await;
        use std::sync::atomic::Ordering;
        let marker = HashMap::from([("stale".to_string(), Vec::new())]);
        let aged = Instant::now()
            .checked_sub(Duration::from_secs(600))
            .unwrap();
        let epoch = CACHE_GEN.load(Ordering::Acquire);
        *CACHE.lock().await = Some((epoch, aged, ModelProbe::default(), marker));
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

    #[tokio::test]
    async fn invalidate_provider_models_cache_forces_immediate_rediscovery() {
        let _serial = TEST_CACHE_LOCK.lock().await;
        use std::sync::atomic::Ordering;
        let epoch = CACHE_GEN.load(Ordering::Acquire);
        let marker = HashMap::from([("cached-only".to_string(), Vec::new())]);
        *CACHE.lock().await = Some((epoch, Instant::now(), ModelProbe::default(), marker));
        assert!(probe_provider_models_cached(&ModelProbe::default())
            .await
            .contains_key("cached-only"));

        invalidate_provider_models_cache(Some("claude"));
        assert!(
            probe_provider_models_cached(&ModelProbe::default())
                .await
                .is_empty(),
            "invalidated cache must not serve pre-refresh snapshot"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cold_discovery_returns_within_cold_budget_then_populates_slow_cli() {
        let _serial = TEST_CACHE_LOCK.lock().await;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::Ordering;

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("slow-opencode");
        std::fs::write(
            &script,
            "#!/bin/sh\nsleep 0.45\nprintf 'openai/gpt-slow-test\\n'\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        *CACHE.lock().await = None;
        let probe = ModelProbe {
            families: vec![
                ("claude".into(), "claude".into()),
                ("opencode".into(), "opencode".into()),
            ],
            opencode_program: Some(script.to_string_lossy().into_owned()),
            ..Default::default()
        };

        let t0 = Instant::now();
        let provisional = probe_provider_models_cached(&probe).await;
        let elapsed = t0.elapsed();
        assert!(
            elapsed < Duration::from_millis(420),
            "cold /meta exceeded inline budget: {elapsed:?}"
        );
        assert!(
            provisional.get("claude").is_some_and(|m| !m.is_empty()),
            "provisional catalog serves local/fallback claude models immediately"
        );
        assert!(
            !provisional.contains_key("opencode"),
            "slow CLI has not finished yet"
        );

        for _ in 0..100 {
            if !REFRESHING.load(Ordering::Acquire) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let refreshed = probe_provider_models_cached(&probe).await;
        assert_eq!(
            refreshed
                .get("opencode")
                .map(|v| v.iter().map(|m| m.id.as_str()).collect::<Vec<_>>()),
            Some(vec!["openai/gpt-slow-test"])
        );
    }

    #[test]
    fn agy_parser_strips_ansi_csi_spinners_and_colors() {
        let raw = "\x1b[?25l\x1b[2K\r\x1b[36m⠋ Fetching models...\x1b[0m\r\x1b[2K\
                   \x1b[32mgemini-4-carbon-low\x1b[0m\tGemini 4 Carbon (Low)\n\
                   \x1b[32mgemini-4-carbon-high\x1b[0m\tGemini 4 Carbon (High)\n\
                   \x1b[?25h";
        let got = parse_agy_models_output(raw);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "gemini-4-carbon");
        assert_eq!(got[0].label, "Gemini 4 Carbon");
        assert_eq!(got[0].supported_efforts, vec!["low", "high"]);
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
            "\r⠋ Fetching available models...\rgemini-4-carbon-medium         Gemini 4 Carbon (Medium)\n\
             gemini-4-carbon-high           Gemini 4 Carbon (High)\n\
             gemini-4-carbon-low            Gemini 4 Carbon (Low)\n\
             gemini-4-hydrogen-high         Gemini 4 Hydrogen (High)\n\
             gemini-4-hydrogen-xhigh        Gemini 4 Hydrogen (XHigh)\n\
             gemini-pro-orchestrator        Orchestrator (Carbon)\n\
             claude-opus-5-5-low            Claude Opus 5.5 (Low)\n\
             claude-opus-5-5-medium         Claude Opus 5.5 (Medium)\n\
             claude-opus-5-5-high           Claude Opus 5.5 (High)\n\
             gpt-oss-120b-medium            GPT-OSS 120B (Medium)\n",
        );
        assert_eq!(
            agy.iter()
                .map(|m| (m.id.as_str(), m.label.as_str()))
                .collect::<Vec<_>>(),
            [
                ("gemini-4-carbon", "Gemini 4 Carbon"),
                ("gemini-4-hydrogen", "Gemini 4 Hydrogen"),
                ("gemini-pro-orchestrator", "Orchestrator (Carbon)"),
                ("claude-opus-5-5", "Claude Opus 5.5"),
                ("gpt-oss-120b-medium", "GPT-OSS 120B"),
            ]
        );
        assert_eq!(agy[0].default_effort.as_deref(), Some("medium"));
        assert_eq!(agy[0].supported_efforts, vec!["low", "medium", "high"]);
        assert_eq!(agy[1].default_effort.as_deref(), Some("high"));
        assert_eq!(agy[1].supported_efforts, vec!["high", "xhigh"]);
        assert_eq!(agy[2].default_effort, None);
        assert!(agy[2].supported_efforts.is_empty());
        assert_eq!(agy[3].default_effort.as_deref(), Some("medium"));
        assert_eq!(agy[3].supported_efforts, vec!["low", "medium", "high"]);
        assert_eq!(agy[4].default_effort.as_deref(), Some("medium"));
        assert_eq!(agy[4].supported_efforts, vec!["medium"]);

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

    /// Verbatim output of the installed CLIs (pi 1.5.0 `--offline
    /// --list-models`, opencode 1.19.5 `models`, stdout then stderr as
    /// `run_models` joins them).
    #[test]
    fn parses_real_pi_1_5_and_opencode_1_19_output() {
        let pi = parse_pi_models_output(
            "provider      model          context  max-out  thinking  images\n\
             openai-codex  gpt-5.6-luna   272K     128K     yes       yes   \n\
             openai-codex  gpt-6.1-terra  500K     128K     yes       yes   \n\
             opencode      big-pickle     200K     32K      yes       no    \n\
             \n",
        );
        assert_eq!(
            pi.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            [
                "openai-codex/gpt-5.6-luna",
                "openai-codex/gpt-6.1-terra",
                "opencode/big-pickle"
            ]
        );
        assert_eq!(pi[1].label, "gpt-6.1-terra");
        assert_eq!(pi[1].description, "openai-codex · 500K context");
        assert_eq!(pi[0].default_effort.as_deref(), Some("medium"));

        let oc = parse_opencode_models_output(
            "opencode/beam-free\nopencode/big-pickle\ngithub-copilot/claude-haiku-4.5\n\
             google/gemini-3.8-flash\nopenai/gpt-5.6-luna-fast\n\
             \n[auto-update-checker] Package not found, nothing to invalidate: x\n",
        );
        assert_eq!(
            oc.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            [
                "opencode/beam-free",
                "opencode/big-pickle",
                "github-copilot/claude-haiku-4.5",
                "google/gemini-3.8-flash",
                "openai/gpt-5.6-luna-fast"
            ]
        );
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
