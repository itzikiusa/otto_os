//! Dynamic **model catalog** (personal-agents design §3): discover each
//! provider's model ids at runtime — no hardcoded lists, no API keys.
//!
//! An hourly refresher (plus the manual `POST /providers/models/refresh`) runs a
//! per-provider source chain, first success wins:
//!
//!   1. **CLI probe** — `agy models` lists ids non-interactively (checked at
//!      impl time; `claude` and `codex` have no model-listing subcommand, so
//!      they skip straight to the scrape).
//!   2. **Docs scrape** — the provider's public models page, fetched through
//!      the `otto-netguard` SSRF check with a 15s timeout. Parsing is
//!      defensive id-shaped-token extraction (no DOM selectors): quoted
//!      `claude-<family>-<version>` strings, `<code>gpt-…</code>` cells,
//!      `gemini-…` tokens.
//!   3. **models.dev JSON** — the community catalog as a generic fallback,
//!      filtered to the provider's model family.
//!
//! A failed chain NEVER wipes the last good list ([`ProviderModelsRepo`]
//! enforces it); the failure is kept in-memory per provider and surfaced as
//! `last_error` + `stale` on the read route.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Utc};
use otto_state::{ProviderModel, ProviderModelsRepo};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::state::ServerCtx;

/// Providers the refresher knows how to source. Custom providers can still get
/// rows via the manual-add repo path; they just have no automatic source.
const KNOWN_PROVIDERS: [&str; 3] = ["claude", "codex", "agy"];

/// Outbound fetch budget per source attempt.
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
/// Background refresh cadence.
const REFRESH_EVERY: Duration = Duration::from_secs(60 * 60);
/// A list older than this is flagged `stale` in the read route (the hourly
/// refresher failing for a full day is worth surfacing).
const STALE_AFTER_HOURS: i64 = 24;

// ---------------------------------------------------------------------------
// In-memory per-provider fetch status (survives only for the daemon lifetime;
// the durable part — the rows — lives in SQLite).
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct FetchStatus {
    last_error: Option<String>,
}

fn status_map() -> &'static Mutex<HashMap<String, FetchStatus>> {
    static STATUS: OnceLock<Mutex<HashMap<String, FetchStatus>>> = OnceLock::new();
    STATUS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn set_status(provider: &str, err: Option<String>) {
    let mut m = status_map().lock().unwrap_or_else(|p| p.into_inner());
    m.insert(provider.to_string(), FetchStatus { last_error: err });
}

fn last_error_of(provider: &str) -> Option<String> {
    let m = status_map().lock().unwrap_or_else(|p| p.into_inner());
    m.get(provider).and_then(|s| s.last_error.clone())
}

// ---------------------------------------------------------------------------
// Defensive id extraction (no regex dep, no DOM paths — just token scanning).
// ---------------------------------------------------------------------------

fn is_id_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.'
}

/// Every maximal `[a-z0-9.-]` token starting with `prefix` at a non-id-char
/// boundary, in order of appearance, deduped, trailing punctuation trimmed.
fn scan_tokens(text: &str, prefix: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(pos) = text[from..].find(prefix) {
        let start = from + pos;
        from = start + prefix.len();
        // Reject a match glued to a preceding id char (e.g. `xgpt-4`).
        if start > 0 {
            let prev = bytes[start - 1] as char;
            if is_id_char(prev) {
                continue;
            }
        }
        let tail: String = text[start..].chars().take_while(|c| is_id_char(*c)).collect();
        let token = tail.trim_end_matches(['.', '-']).to_string();
        if token.len() > prefix.len() && !out.contains(&token) {
            out.push(token);
        }
    }
    out
}

/// True when every `-`-separated segment of `rest` is purely numeric — the
/// shape of a Claude version tail (`5`, `4-5`, `4-5-20251001`). Filters out
/// doc slugs like `…-system-card` or `…-and-claude-mythos-5`.
fn all_numeric_segments(rest: &str) -> bool {
    !rest.is_empty()
        && rest
            .split('-')
            .all(|seg| !seg.is_empty() && seg.chars().all(|c| c.is_ascii_digit()))
}

/// Claude docs page → API ids. The overview page carries ids as quoted strings
/// (`"claude-fable-5"`) amid many look-alike doc slugs, so the filter is the
/// id GRAMMAR: `claude-<family>-<numeric version…>`.
pub fn extract_claude_ids(html: &str) -> Vec<String> {
    const FAMILIES: [&str; 5] = ["fable", "mythos", "opus", "sonnet", "haiku"];
    scan_tokens(html, "claude-")
        .into_iter()
        .filter(|t| {
            let rest = &t["claude-".len()..];
            FAMILIES.iter().any(|f| {
                rest.strip_prefix(f)
                    .and_then(|r| r.strip_prefix('-'))
                    .is_some_and(all_numeric_segments)
            })
        })
        .collect()
}

/// Codex docs page → gpt ids. Model ids appear as `<code>gpt-…</code>` cells;
/// prefer those (clean), fall back to a filtered raw scan if the markup ever
/// changes (dropping asset/domain look-alikes like `gpt-5.4.jpg`).
pub fn extract_codex_ids(html: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(pos) = html[from..].find("<code") {
        let start = from + pos;
        let Some(gt) = html[start..].find('>') else { break };
        let inner_start = start + gt + 1;
        let Some(end) = html[inner_start..].find("</code>") else { break };
        from = inner_start + end;
        let inner = html[inner_start..inner_start + end].trim();
        if inner.starts_with("gpt-")
            && inner.len() > 4
            && inner.chars().nth(4).is_some_and(|c| c.is_ascii_digit())
            && inner.chars().all(is_id_char)
            && !out.contains(&inner.to_string())
        {
            out.push(inner.to_string());
        }
    }
    if !out.is_empty() {
        return out;
    }
    scan_tokens(html, "gpt-")
        .into_iter()
        .filter(|t| {
            t.chars().nth(4).is_some_and(|c| c.is_ascii_digit())
                && !t.ends_with(".jpg")
                && !t.ends_with(".webp")
                && !t.ends_with(".png")
                && !t.contains(".com")
                && !t.contains("localhost")
        })
        .collect()
}

/// Gemini docs page → gemini ids (raw token scan; the page carries them in
/// code spans and JSON blobs alike).
pub fn extract_gemini_ids(html: &str) -> Vec<String> {
    scan_tokens(html, "gemini-")
        .into_iter()
        .filter(|t| {
            t.chars().nth("gemini-".len()).is_some_and(|c| c.is_ascii_digit())
                && !t.ends_with(".jpg")
                && !t.ends_with(".webp")
                && !t.ends_with(".png")
                && !t.contains(".com")
        })
        .collect()
}

/// `agy models` stdout → (id, label) pairs. Lines are `id<TAB>Human Label`;
/// tolerate plain-space separation and skip banners ("Fetching …").
pub fn parse_agy_models_output(stdout: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (id, label) = match line.split_once('\t') {
            Some((i, l)) => (i.trim(), l.trim()),
            None => match line.split_once("  ") {
                Some((i, l)) => (i.trim(), l.trim()),
                None => (line, ""),
            },
        };
        // An id is a lone lowercase token; banner lines have spaces/uppercase.
        if id.is_empty() || !id.chars().all(is_id_char) {
            continue;
        }
        let label = if label.is_empty() { id } else { label };
        if !out.iter().any(|(i, _)| i == id) {
            out.push((id.to_string(), label.to_string()));
        }
    }
    out
}

/// models.dev `api.json` → (id, label) pairs for one of our providers.
/// Shape (verified at impl time): `{ "<provider>": { "models": { "<id>":
/// { "id", "name", … } } } }` — parsed defensively, unknown shapes yield [].
pub fn parse_models_dev(json: &Value, provider: &str) -> Vec<(String, String)> {
    let (key, family) = match provider {
        "claude" => ("anthropic", "claude-"),
        "codex" => ("openai", "gpt-"),
        "agy" => ("google", "gemini-"),
        _ => return Vec::new(),
    };
    let Some(models) = json.get(key).and_then(|p| p.get("models")).and_then(Value::as_object)
    else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = models
        .iter()
        .filter(|(id, _)| id.starts_with(family))
        .map(|(id, m)| {
            let label = m.get("name").and_then(Value::as_str).unwrap_or(id).to_string();
            (id.clone(), label)
        })
        .collect();
    // The JSON map has no meaningful order — sort for determinism.
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

// ---------------------------------------------------------------------------
// Source chain
// ---------------------------------------------------------------------------

/// Netguard-checked GET with the shared redirect policy + fetch budget.
async fn guarded_fetch_text(url: &str) -> Result<String, String> {
    otto_netguard::check_url(url).await?;
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .redirect(otto_netguard::redirect_policy())
        .user_agent("otto-model-catalog")
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("{url}: HTTP {}", resp.status()));
    }
    resp.text().await.map_err(|e| e.to_string())
}

/// Source 1 — CLI probe. Only `agy` can list models non-interactively
/// (`agy models`); `claude`/`codex` have no such subcommand (checked at impl
/// time), so they return None and the chain moves on. Never launches a TUI:
/// stdin is null and the process is hard-capped at the fetch budget.
async fn cli_probe(provider: &str) -> Option<Result<Vec<(String, String)>, String>> {
    if provider != "agy" {
        return None;
    }
    let run = async {
        let out = tokio::process::Command::new("agy")
            .arg("models")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env("TERM", "dumb")
            .kill_on_drop(true)
            .output();
        match tokio::time::timeout(FETCH_TIMEOUT, out).await {
            Err(_) => Err("agy models: timed out".to_string()),
            Ok(Err(e)) => Err(format!("agy models: {e}")),
            Ok(Ok(o)) if !o.status.success() => {
                Err(format!("agy models: exit {}", o.status))
            }
            Ok(Ok(o)) => {
                let models = parse_agy_models_output(&String::from_utf8_lossy(&o.stdout));
                if models.is_empty() {
                    Err("agy models: no ids in output".to_string())
                } else {
                    Ok(models)
                }
            }
        }
    };
    Some(run.await)
}

/// Source 2 — docs scrape (the primary source in practice for claude/codex).
async fn docs_scrape(provider: &str) -> Option<Result<Vec<(String, String)>, String>> {
    let (url, extract): (&str, fn(&str) -> Vec<String>) = match provider {
        "claude" => ("https://platform.claude.com/docs/en/models/overview", extract_claude_ids),
        "codex" => ("https://learn.chatgpt.com/docs/models", extract_codex_ids),
        "agy" => ("https://ai.google.dev/gemini-api/docs/models", extract_gemini_ids),
        _ => return None,
    };
    Some(match guarded_fetch_text(url).await {
        Err(e) => Err(e),
        Ok(html) => {
            let ids = extract(&html);
            if ids.is_empty() {
                Err(format!("{url}: no model ids extracted"))
            } else {
                Ok(ids.into_iter().map(|id| (id.clone(), id)).collect())
            }
        }
    })
}

/// Source 3 — models.dev community catalog (generic fallback).
async fn catalog_fallback(provider: &str) -> Result<Vec<(String, String)>, String> {
    let body = guarded_fetch_text("https://models.dev/api.json").await?;
    let json: Value = serde_json::from_str(&body).map_err(|e| format!("models.dev: {e}"))?;
    let models = parse_models_dev(&json, provider);
    if models.is_empty() {
        Err(format!("models.dev: no '{provider}' family models"))
    } else {
        Ok(models)
    }
}

/// Run the chain for one provider and persist the first success. On total
/// failure the table is untouched and the error is recorded for the routes.
pub async fn refresh_provider(ctx: &ServerCtx, provider: &str) {
    let repo = ProviderModelsRepo::new(ctx.pool.clone());
    let mut errors: Vec<String> = Vec::new();

    // Lazily walk the chain so a CLI hit skips the network sources entirely.
    for source in ["cli", "scrape", "catalog"] {
        let attempt = match source {
            "cli" => cli_probe(provider).await,
            "scrape" => docs_scrape(provider).await,
            _ => Some(catalog_fallback(provider).await),
        };
        match attempt {
            None => continue, // source not applicable to this provider
            Some(Err(e)) => errors.push(format!("{source}: {e}")),
            Some(Ok(models)) => {
                match repo.upsert_batch(provider, source, &models).await {
                    Ok(()) => {
                        info!(provider, source, count = models.len(), "model catalog refreshed");
                        set_status(provider, None);
                    }
                    Err(e) => {
                        warn!(provider, %e, "model catalog persist failed");
                        set_status(provider, Some(format!("persist: {e}")));
                    }
                }
                return;
            }
        }
    }
    let joined = errors.join("; ");
    warn!(provider, error = %joined, "model catalog refresh failed (keeping last good list)");
    set_status(provider, Some(joined));
}

/// Refresh every known provider (sequentially — three cheap fetches).
pub async fn refresh_all(ctx: &ServerCtx) {
    for p in KNOWN_PROVIDERS {
        refresh_provider(ctx, p).await;
    }
}

/// Hourly background refresher, started with the daemon. The first pass runs
/// shortly after boot so a fresh install has a catalog within seconds.
pub fn spawn_refresher(ctx: ServerCtx) -> JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        loop {
            refresh_all(&ctx).await;
            tokio::time::sleep(REFRESH_EVERY).await;
        }
    })
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct CatalogModel {
    pub id: String,
    pub label: String,
    pub source: String,
}

#[derive(Serialize)]
pub struct ProviderCatalog {
    pub models: Vec<CatalogModel>,
    /// RFC3339 of the newest fetched row (None = never fetched).
    pub fetched_at: Option<String>,
    pub stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Serialize)]
pub struct ModelsResp {
    pub providers: HashMap<String, ProviderCatalog>,
}

#[derive(Deserialize, Default)]
pub struct RefreshReq {
    /// Refresh just this provider; omitted = all known providers.
    #[serde(default)]
    pub provider: Option<String>,
}

fn group(rows: Vec<ProviderModel>) -> HashMap<String, ProviderCatalog> {
    let mut providers: HashMap<String, ProviderCatalog> = HashMap::new();
    // Known providers always appear (even empty) so the UI can show status.
    for p in KNOWN_PROVIDERS {
        providers.insert(
            p.to_string(),
            ProviderCatalog {
                models: Vec::new(),
                fetched_at: None,
                stale: true,
                last_error: last_error_of(p),
            },
        );
    }
    for row in rows {
        let entry = providers.entry(row.provider.clone()).or_insert_with(|| ProviderCatalog {
            models: Vec::new(),
            fetched_at: None,
            stale: true,
            last_error: last_error_of(&row.provider),
        });
        // Staleness tracks the fetched rows; manual rows never age out.
        if row.source != "manual"
            && entry.fetched_at.as_deref().is_none_or(|cur| row.fetched_at.as_str() > cur)
        {
            entry.fetched_at = Some(row.fetched_at.clone());
        }
        entry.models.push(CatalogModel {
            id: row.model_id,
            label: row.label,
            source: row.source,
        });
    }
    for cat in providers.values_mut() {
        cat.stale = match cat.fetched_at.as_deref().and_then(|t| {
            DateTime::parse_from_rfc3339(t).ok().map(|d| d.with_timezone(&Utc))
        }) {
            Some(at) => Utc::now() - at > chrono::Duration::hours(STALE_AFTER_HOURS),
            None => true,
        };
    }
    providers
}

async fn payload(ctx: &ServerCtx) -> ApiResult<ModelsResp> {
    let rows = ProviderModelsRepo::new(ctx.pool.clone()).list_all().await?;
    Ok(ModelsResp { providers: group(rows) })
}

/// `GET /api/v1/providers/models` — the full grouped catalog + staleness.
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<ModelsResp>> {
    Ok(Json(payload(&ctx).await?))
}

/// `POST /api/v1/providers/models/refresh` — run the source chain now
/// (optionally for one provider) and return the updated catalog.
pub async fn refresh(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
    body: Option<Json<RefreshReq>>,
) -> ApiResult<Json<ModelsResp>> {
    let req = body.map(|Json(r)| r).unwrap_or_default();
    match req.provider.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => refresh_provider(&ctx, p).await,
        None => refresh_all(&ctx).await,
    }
    Ok(Json(payload(&ctx).await?))
}

// ---------------------------------------------------------------------------
// Parser tests against saved snapshots of the real pages (fetched 2026-09-01,
// trimmed to the relevant chunks + representative noise).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE_HTML: &str = include_str!("../tests/fixtures/claude_models.html");
    const CODEX_HTML: &str = include_str!("../tests/fixtures/codex_models.html");
    const MODELS_DEV: &str = include_str!("../tests/fixtures/models_dev.json");
    const AGY_OUT: &str = include_str!("../tests/fixtures/agy_models.txt");

    #[test]
    fn claude_ids_from_docs_snapshot() {
        let ids = extract_claude_ids(CLAUDE_HTML);
        for expect in ["claude-fable-5", "claude-opus-5", "claude-sonnet-5", "claude-haiku-4-5"] {
            assert!(ids.contains(&expect.to_string()), "missing {expect} in {ids:?}");
        }
        // Doc slugs that share the prefix must NOT survive the grammar filter.
        for reject in ids.iter() {
            assert!(!reject.contains("system-card"), "slug leaked: {reject}");
            assert!(!reject.ends_with("-and"), "slug leaked: {reject}");
        }
    }

    #[test]
    fn codex_ids_from_docs_snapshot() {
        let ids = extract_codex_ids(CODEX_HTML);
        for expect in ["gpt-5.3-codex", "gpt-5.4", "gpt-5.6-luna"] {
            assert!(ids.contains(&expect.to_string()), "missing {expect} in {ids:?}");
        }
        assert!(
            ids.iter().all(|i| !i.ends_with(".jpg") && !i.ends_with(".webp")),
            "asset name leaked: {ids:?}"
        );
    }

    #[test]
    fn codex_raw_fallback_filters_assets() {
        // No <code> tags → the raw-scan fallback engages and still drops noise.
        let html = r#"see "gpt-5.4" and img gpt-5.4.jpg plus gpt-staging.com and gpt-5.5"#;
        let ids = extract_codex_ids(html);
        assert_eq!(ids, vec!["gpt-5.4".to_string(), "gpt-5.5".to_string()]);
    }

    #[test]
    fn agy_cli_output_parses() {
        let models = parse_agy_models_output(AGY_OUT);
        assert!(!models.is_empty());
        assert!(models.iter().any(|(id, _)| id.starts_with("gemini-")), "{models:?}");
        // Labels come from the second column, not the id.
        let (_, label) = models.iter().find(|(id, _)| id.contains("pro")).unwrap();
        assert!(label.contains(' '), "label should be human text: {label}");
    }

    #[test]
    fn gemini_ids_token_scan() {
        let html = r#"<code>gemini-3.1-pro-high</code> and "gemini-3.7-flash-low" but not gemini-api"#;
        let ids = extract_gemini_ids(html);
        assert_eq!(ids, vec!["gemini-3.1-pro-high".to_string(), "gemini-3.7-flash-low".to_string()]);
    }

    #[test]
    fn models_dev_snapshot_parses_per_family() {
        let json: Value = serde_json::from_str(MODELS_DEV).unwrap();
        let claude = parse_models_dev(&json, "claude");
        assert!(claude.iter().all(|(id, _)| id.starts_with("claude-")));
        assert!(!claude.is_empty());
        let agy = parse_models_dev(&json, "agy");
        // gemma-* entries in google's map must be filtered out by family.
        assert!(agy.iter().all(|(id, _)| id.starts_with("gemini-")), "{agy:?}");
        assert!(!agy.is_empty());
        assert!(parse_models_dev(&json, "unknown").is_empty());
        // Defensive: garbage shape → empty, not panic.
        assert!(parse_models_dev(&Value::Null, "claude").is_empty());
    }
}
