//! Shared model-catalog discovery, scraping, and fallback parsers used by
//! `otto-server`'s `/api/v1/providers/models` catalog refresher.
//!
//! Keeps all pure token-scanning, `models.dev` parsing, CLI model probing, and
//! per-provider fetch-error tracking in `otto-sessions` alongside `/meta`'s
//! live `provider_models` discovery so both surfaces stay in lockstep across
//! all seven built-in providers.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;
use tokio::time::timeout;

/// Built-in providers covered by the dynamic model catalog and refresher.
pub const KNOWN_CATALOG_PROVIDERS: [&str; 8] = [
    "claude", "codex", "agy", "gemini", "grok", "opencode", "pi", "muse",
];

/// A catalog older than this many hours is marked `stale` in `/providers/models`.
pub const CATALOG_STALE_AFTER_HOURS: i64 = 24;

#[derive(Clone, Default)]
struct FetchStatus {
    last_error: Option<String>,
}

fn status_map() -> &'static Mutex<HashMap<String, FetchStatus>> {
    static STATUS: OnceLock<Mutex<HashMap<String, FetchStatus>>> = OnceLock::new();
    STATUS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record (or clear when `None`) the last refresh error for `provider`.
pub fn set_catalog_fetch_status(provider: &str, err: Option<String>) {
    let mut m = status_map().lock().unwrap_or_else(|p| p.into_inner());
    m.insert(provider.to_string(), FetchStatus { last_error: err });
}

/// Read the last recorded refresh error for `provider`, if any.
pub fn catalog_last_error(provider: &str) -> Option<String> {
    let m = status_map().lock().unwrap_or_else(|p| p.into_inner());
    m.get(provider).and_then(|s| s.last_error.clone())
}

fn is_id_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.'
}

/// Every maximal `[a-z0-9.-]` token starting with `prefix` at a non-id-char
/// boundary, in order of appearance, deduped, trailing punctuation trimmed.
pub fn scan_tokens(text: &str, prefix: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(pos) = text[from..].find(prefix) {
        let start = from + pos;
        from = start + prefix.len();
        if start > 0 {
            let prev = bytes[start - 1] as char;
            if is_id_char(prev) {
                continue;
            }
        }
        let tail: String = text[start..]
            .chars()
            .take_while(|c| is_id_char(*c))
            .collect();
        let token = tail.trim_end_matches(['.', '-']).to_string();
        if token.len() > prefix.len() && !out.contains(&token) {
            out.push(token);
        }
    }
    out
}

fn all_numeric_segments(rest: &str) -> bool {
    !rest.is_empty()
        && rest
            .split('-')
            .all(|seg| !seg.is_empty() && seg.chars().all(|c| c.is_ascii_digit()))
}

/// Claude docs page -> API ids (`claude-<family>-<numeric version...>`).
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

/// Codex docs page -> `gpt-*` ids (`<code>gpt-...</code>` preferred, raw scan fallback).
pub fn extract_codex_ids(html: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(pos) = html[from..].find("<code") {
        let start = from + pos;
        let Some(gt) = html[start..].find('>') else {
            break;
        };
        let inner_start = start + gt + 1;
        let Some(end) = html[inner_start..].find("</code>") else {
            break;
        };
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

/// Gemini docs page -> `gemini-*` ids.
pub fn extract_gemini_ids(html: &str) -> Vec<String> {
    scan_tokens(html, "gemini-")
        .into_iter()
        .filter(|t| {
            t.chars()
                .nth("gemini-".len())
                .is_some_and(|c| c.is_ascii_digit())
                && !t.ends_with(".jpg")
                && !t.ends_with(".webp")
                && !t.ends_with(".png")
                && !t.contains(".com")
        })
        .collect()
}

/// xAI docs page -> `grok-*` ids.
pub fn extract_grok_ids(html: &str) -> Vec<String> {
    scan_tokens(html, "grok-")
        .into_iter()
        .filter(|t| {
            t.chars()
                .nth("grok-".len())
                .is_some_and(|c| c.is_ascii_digit())
                && !t.ends_with(".jpg")
                && !t.ends_with(".webp")
                && !t.ends_with(".png")
                && !t.contains(".com")
        })
        .collect()
}

/// `agy models` stdout -> `(id, label)` pairs for the persisted catalog.
pub fn parse_agy_catalog_output(stdout: &str) -> Vec<(String, String)> {
    super::parse_agy_models_output(stdout)
        .into_iter()
        .map(|m| (m.id, m.label))
        .collect()
}

/// `models.dev` `api.json` -> `(id, label)` pairs for one built-in provider.
pub fn parse_models_dev(json: &Value, provider: &str) -> Vec<(String, String)> {
    if matches!(provider, "opencode" | "pi") {
        let mut out: Vec<(String, String)> = Vec::new();
        for (vendor, family) in [
            ("anthropic", "claude-"),
            ("openai", "gpt-"),
            ("google", "gemini-"),
        ] {
            let Some(models) = json
                .get(vendor)
                .and_then(|p| p.get("models"))
                .and_then(Value::as_object)
            else {
                continue;
            };
            for (id, m) in models.iter().filter(|(id, _)| id.starts_with(family)) {
                let full_id = format!("{vendor}/{id}");
                let label = m
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(&full_id)
                    .to_string();
                out.push((full_id, label));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        return out;
    }
    let (key, family) = match provider {
        "claude" => ("anthropic", "claude-"),
        "codex" => ("openai", "gpt-"),
        "agy" | "gemini" => ("google", "gemini-"),
        "grok" => ("xai", "grok-"),
        _ => return Vec::new(),
    };
    let Some(models) = json
        .get(key)
        .and_then(|p| p.get("models"))
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = models
        .iter()
        .filter(|(id, _)| id.starts_with(family))
        .map(|(id, m)| {
            let label = m
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(id)
                .to_string();
            (id.clone(), label)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Public docs URL + token extractor for Source 2 (`scrape`), when supported.
pub type CatalogScrapeTarget = (&'static str, fn(&str) -> Vec<String>);

pub fn catalog_scrape_target(provider: &str) -> Option<CatalogScrapeTarget> {
    match provider {
        "claude" => Some((
            "https://platform.claude.com/docs/en/models/overview",
            extract_claude_ids,
        )),
        "codex" => Some(("https://learn.chatgpt.com/docs/models", extract_codex_ids)),
        "agy" | "gemini" => Some((
            "https://ai.google.dev/gemini-api/docs/models",
            extract_gemini_ids,
        )),
        "grok" => Some(("https://docs.x.ai/docs/models", extract_grok_ids)),
        _ => None,
    }
}

/// Source 1 — non-interactive CLI model probe (`agy`, `grok`, `opencode`,
/// `pi`, `muse`). Returns `None` when `provider` has no model-listing CLI
/// subcommand.
pub async fn probe_catalog_cli(
    provider: &str,
    program: &str,
    budget: Duration,
) -> Option<Result<Vec<(String, String)>, String>> {
    if provider == "muse" {
        let models = super::run_muse_model_list(program, budget)
            .await
            .map(|out| super::parse_muse_models_output(&out));
        return Some(match models {
            Ok(m) if m.is_empty() => Err("muse model/list: no models".into()),
            Ok(m) => Ok(m.into_iter().map(|m| (m.id, m.label)).collect()),
            Err(e) => Err(e),
        });
    }
    let args: &[&str] = match provider {
        "agy" | "grok" | "opencode" => &["models"],
        "pi" => &["--offline", "--list-models"],
        _ => return None,
    };
    let cmd_label = format!("{provider} {}", args.join(" "));
    let mut cmd = super::cli_models_command(program, args);
    let out = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("TERM", "dumb")
        .kill_on_drop(true)
        .output();
    let res = match timeout(budget, out).await {
        Err(_) => Err(format!("{cmd_label}: timed out")),
        Ok(Err(e)) => Err(format!("{cmd_label}: {e}")),
        Ok(Ok(o)) if !o.status.success() && provider != "grok" => {
            Err(format!("{cmd_label}: exit {}", o.status))
        }
        Ok(Ok(o)) => {
            let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
            if provider != "agy" {
                text.push('\n');
                text.push_str(&String::from_utf8_lossy(&o.stderr));
            }
            let models: Vec<(String, String)> = match provider {
                "agy" => parse_agy_catalog_output(&text),
                "grok" => super::parse_grok_models_output(&text)
                    .into_iter()
                    .map(|m| (m.id, m.label))
                    .collect(),
                "opencode" => super::parse_opencode_models_output(&text)
                    .into_iter()
                    .map(|m| (m.id, m.label))
                    .collect(),
                "pi" => super::parse_pi_models_output(&text)
                    .into_iter()
                    .map(|m| (m.id, m.label))
                    .collect(),
                _ => Vec::new(),
            };
            if models.is_empty() {
                if !o.status.success() {
                    Err(format!("{cmd_label}: exit {}", o.status))
                } else {
                    Err(format!("{cmd_label}: no ids in output"))
                }
            } else {
                Ok(models)
            }
        }
    };
    Some(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_catalog_providers_covers_all_eight_builtins() {
        assert_eq!(
            KNOWN_CATALOG_PROVIDERS,
            ["claude", "codex", "agy", "gemini", "grok", "opencode", "pi", "muse"]
        );
        for p in ["claude", "codex", "agy", "gemini", "grok"] {
            assert!(catalog_scrape_target(p).is_some(), "missing scrape for {p}");
        }
    }

    #[test]
    fn gemini_and_grok_parse_from_scrape_and_models_dev() {
        let html = r#"<code>grok-4.6</code> "grok-4.5-mini" not grok-api or grok-4.jpg"#;
        assert_eq!(
            extract_grok_ids(html),
            vec!["grok-4.6".to_string(), "grok-4.5-mini".to_string()]
        );

        let dev = serde_json::json!({
            "google": {
                "models": {
                    "gemini-2.5-pro": { "name": "Gemini 2.5 Pro" },
                    "gemma-3-27b": { "name": "Gemma 3" }
                }
            },
            "xai": {
                "models": {
                    "grok-4.6": { "name": "Grok 4.6" },
                    "aurora": { "name": "Aurora" }
                }
            }
        });
        assert_eq!(
            parse_models_dev(&dev, "gemini"),
            vec![("gemini-2.5-pro".into(), "Gemini 2.5 Pro".into())]
        );
        assert_eq!(
            parse_models_dev(&dev, "grok"),
            vec![("grok-4.6".into(), "Grok 4.6".into())]
        );
    }
}
