//! Opt-in cross-check against [ccusage](https://github.com/ryoppippi/ccusage).
//!
//! ccusage reads the same local transcripts Otto's tailer does and is the
//! community's reference for "what did my agents use". The check runs it ON
//! DEMAND through `npx` (no Otto dependency — npx fetches/caches it), bounded
//! by a timeout, and lines its `daily --json --breakdown` numbers up against
//! Otto's rows for the same dates, broken down by provider/model. Everything
//! here except [`run`] is pure, so parsing and diffing are unit-tested against
//! the exact shape ccusage 20 emits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::types::{CcusageDayRow, CcusageDiffRow, DailyModelUsage, TokenTotals};

/// How long ccusage may run (a first `npx` run downloads it).
pub const TIMEOUT: Duration = Duration::from_secs(120);

/// One (day, model) cell of ccusage's output.
#[derive(Debug, Clone, PartialEq)]
pub struct TheirCell {
    pub day: String,
    pub model: String,
    pub totals: TokenTotals,
}

/// The ccusage arguments for an inclusive `YYYY-MM-DD` range.
pub fn args(since: &str, until: &str) -> Vec<String> {
    vec![
        "--yes".into(),
        "ccusage".into(),
        "daily".into(),
        "--json".into(),
        "--breakdown".into(),
        "--since".into(),
        since.replace('-', ""),
        "--until".into(),
        until.replace('-', ""),
    ]
}

/// Locate `npx`: the daemon's `PATH` first, then the usual Homebrew / system
/// prefixes (a launchd-started daemon has a minimal `PATH`).
pub fn find_npx() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(
        ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
            .iter()
            .map(PathBuf::from),
    );
    dirs.into_iter()
        .map(|d| d.join("npx"))
        .find(|p| p.is_file())
}

/// Run ccusage via `npx` and return its parsed JSON. `Err` carries a
/// user-facing reason (missing npx, non-zero exit, timeout, bad JSON).
pub async fn run(npx: &Path, args: &[String]) -> Result<Value, String> {
    let mut cmd = tokio::process::Command::new(npx);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    // npx needs `node` next to it on PATH.
    if let Some(dir) = npx.parent() {
        let mut path = vec![dir.to_path_buf()];
        if let Some(p) = std::env::var_os("PATH") {
            path.extend(std::env::split_paths(&p));
        }
        if let Ok(joined) = std::env::join_paths(path) {
            cmd.env("PATH", joined);
        }
    }
    let out = match tokio::time::timeout(TIMEOUT, cmd.output()).await {
        Err(_) => return Err(format!("ccusage timed out after {}s", TIMEOUT.as_secs())),
        Ok(Err(e)) => return Err(format!("could not run npx: {e}")),
        Ok(Ok(o)) => o,
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: String = err.lines().rev().take(5).collect::<Vec<_>>().join(" | ");
        return Err(format!("ccusage exited with {}: {tail}", out.status));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("ccusage output is not JSON: {e}"))
}

fn num(v: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|k| v.get(*k))
        .and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f.max(0.0) as u64)))
        .unwrap_or(0)
}

fn cost(v: &Value) -> f64 {
    ["cost", "totalCost", "costUSD"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_f64))
        .unwrap_or(0.0)
}

fn totals_of(v: &Value) -> TokenTotals {
    let input_tokens = num(v, &["inputTokens", "input_tokens"]);
    let output_tokens = num(v, &["outputTokens", "output_tokens"]);
    let cache_read_tokens = num(v, &["cacheReadTokens", "cache_read_tokens"]);
    let cache_write_tokens = num(v, &["cacheCreationTokens", "cache_creation_tokens"]);
    TokenTotals {
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        total_tokens: input_tokens + output_tokens + cache_read_tokens + cache_write_tokens,
        cost_usd: cost(v),
    }
}

/// Flatten `ccusage daily --json --breakdown` into (day, model) cells. A day
/// without a model breakdown becomes one `"(unknown)"` cell, so totals still
/// add up. Tolerates the older `date` key and snake_case fields.
pub fn parse_daily(v: &Value) -> Vec<TheirCell> {
    let mut out = Vec::new();
    let Some(days) = v.get("daily").and_then(Value::as_array) else {
        return out;
    };
    for d in days {
        let Some(day) = ["period", "date", "day"]
            .iter()
            .find_map(|k| d.get(*k).and_then(Value::as_str))
        else {
            continue;
        };
        let day = day.get(..10).unwrap_or(day).to_string();
        let breakdowns = d
            .get("modelBreakdowns")
            .or_else(|| d.get("models"))
            .and_then(Value::as_array);
        match breakdowns {
            Some(models) if !models.is_empty() => {
                for m in models {
                    let model = ["modelName", "model", "name"]
                        .iter()
                        .find_map(|k| m.get(*k).and_then(Value::as_str))
                        .unwrap_or("(unknown)")
                        .to_string();
                    out.push(TheirCell {
                        day: day.clone(),
                        model,
                        totals: totals_of(m),
                    });
                }
            }
            _ => out.push(TheirCell {
                day,
                model: "(unknown)".into(),
                totals: totals_of(d),
            }),
        }
    }
    out
}

/// Provider for a model ccusage reports that Otto has no rows for.
fn infer_provider(model: &str) -> String {
    let m = model.to_lowercase();
    if ["claude", "opus", "sonnet", "haiku", "fable", "mythos"]
        .iter()
        .any(|k| m.contains(k))
    {
        "claude".into()
    } else {
        // e.g. Hermes' gpt-* models: counted by ccusage, not tracked by Otto.
        "untracked".into()
    }
}

fn totals_from(r: &DailyModelUsage) -> TokenTotals {
    TokenTotals {
        input_tokens: r.input_tokens,
        output_tokens: r.output_tokens,
        cache_read_tokens: r.cache_read_tokens,
        cache_write_tokens: r.cache_write_tokens,
        total_tokens: r.total_tokens,
        cost_usd: r.cost_usd,
    }
}

/// Side-by-side result of [`diff`].
#[derive(Debug, Default)]
pub struct Diff {
    pub rows: Vec<CcusageDiffRow>,
    pub daily: Vec<CcusageDayRow>,
    pub totals_ours: TokenTotals,
    pub totals_theirs: TokenTotals,
}

/// Line Otto's (day, provider, model) rows up against ccusage's cells. Rows
/// are keyed by model (ccusage has no provider); a model only ccusage saw gets
/// an inferred provider. Rows sort by the larger side's total, biggest first.
pub fn diff(ours: &[DailyModelUsage], theirs: &[TheirCell]) -> Diff {
    let mut provider_of: BTreeMap<String, String> = BTreeMap::new();
    let mut by_model: BTreeMap<String, (TokenTotals, TokenTotals)> = BTreeMap::new();
    let mut by_day: BTreeMap<String, (TokenTotals, TokenTotals)> = BTreeMap::new();
    let mut out = Diff::default();
    for r in ours {
        let t = totals_from(r);
        provider_of
            .entry(r.model.clone())
            .or_insert_with(|| r.provider.clone());
        by_model.entry(r.model.clone()).or_default().0.add(&t);
        by_day.entry(r.day.clone()).or_default().0.add(&t);
        out.totals_ours.add(&t);
    }
    for c in theirs {
        by_model
            .entry(c.model.clone())
            .or_default()
            .1
            .add(&c.totals);
        by_day.entry(c.day.clone()).or_default().1.add(&c.totals);
        out.totals_theirs.add(&c.totals);
    }
    out.rows = by_model
        .into_iter()
        .map(|(model, (ours, theirs))| CcusageDiffRow {
            provider: provider_of
                .get(&model)
                .cloned()
                .unwrap_or_else(|| infer_provider(&model)),
            model,
            ours,
            theirs,
        })
        .collect();
    out.rows.sort_by(|a, b| {
        let ka = a.ours.total_tokens.max(a.theirs.total_tokens);
        let kb = b.ours.total_tokens.max(b.theirs.total_tokens);
        kb.cmp(&ka).then_with(|| a.model.cmp(&b.model))
    });
    out.daily = by_day
        .into_iter()
        .map(|(day, (ours, theirs))| CcusageDayRow { day, ours, theirs })
        .collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed real output of `ccusage daily --json --breakdown` (v20.0.26).
    const SAMPLE: &str = r#"{"daily":[{"agent":"all","cacheCreationTokens":25245942,
        "cacheReadTokens":581555584,"inputTokens":9758414,"metadata":{"agents":["claude","codex","hermes"]},
        "modelBreakdowns":[{"cacheCreationTokens":25245942,"cacheReadTokens":451398656,"cost":260.93,
        "inputTokens":4628,"modelName":"claude-opus-5-5","outputTokens":1870352},
        {"cacheCreationTokens":0,"cacheReadTokens":130156928,"cost":22.86,"inputTokens":9753786,
        "modelName":"gpt-6.1-sol","outputTokens":589168}],"modelsUsed":["claude-opus-5-5","gpt-6.1-sol"],
        "outputTokens":2459520,"period":"2026-10-02","totalCost":283.79,"totalTokens":619053643},
        {"date":"2026-10-03","inputTokens":10,"outputTokens":20,"cacheReadTokens":0,
        "cacheCreationTokens":0,"totalCost":0.5}],
        "totals":{"inputTokens":9758424}}"#;

    #[test]
    fn args_use_compact_dates() {
        let a = args("2026-09-26", "2026-10-03");
        assert_eq!(a[1], "ccusage");
        assert!(a.windows(2).any(|w| w == ["--since", "20260926"]));
        assert!(a.windows(2).any(|w| w == ["--until", "20261003"]));
        assert!(a.contains(&"--breakdown".to_string()));
    }

    #[test]
    fn parse_daily_flattens_model_breakdowns() {
        let cells = parse_daily(&serde_json::from_str(SAMPLE).unwrap());
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0].day, "2026-10-02");
        assert_eq!(cells[0].model, "claude-opus-5-5");
        assert_eq!(cells[0].totals.output_tokens, 1_870_352);
        assert_eq!(cells[0].totals.cache_write_tokens, 25_245_942);
        assert_eq!(
            cells[0].totals.total_tokens,
            4628 + 1_870_352 + 451_398_656 + 25_245_942
        );
        assert!((cells[0].totals.cost_usd - 260.93).abs() < 1e-9);
        // A day without a breakdown is one "(unknown)" cell (older `date` key).
        assert_eq!(cells[2].model, "(unknown)");
        assert_eq!(cells[2].day, "2026-10-03");
        assert_eq!(cells[2].totals.total_tokens, 30);
        assert!(parse_daily(&serde_json::json!({"nope": 1})).is_empty());
    }

    #[test]
    fn diff_lines_models_up_and_flags_untracked_ones() {
        let theirs = parse_daily(&serde_json::from_str(SAMPLE).unwrap());
        let ours = vec![DailyModelUsage {
            day: "2026-10-02".into(),
            provider: "claude".into(),
            model: "claude-opus-5-5".into(),
            input_tokens: 4628,
            output_tokens: 600_000,
            cache_read_tokens: 451_398_656,
            cache_write_tokens: 25_245_942,
            total_tokens: 4628 + 600_000 + 451_398_656 + 25_245_942,
            cost_usd: 230.0,
        }];
        let d = diff(&ours, &theirs);
        let opus = d
            .rows
            .iter()
            .find(|r| r.model == "claude-opus-5-5")
            .unwrap();
        assert_eq!(opus.provider, "claude");
        assert_eq!(
            opus.theirs.output_tokens - opus.ours.output_tokens,
            1_270_352
        );
        let gpt = d.rows.iter().find(|r| r.model == "gpt-6.1-sol").unwrap();
        assert_eq!(gpt.provider, "untracked");
        assert_eq!(gpt.ours, TokenTotals::default());
        assert_eq!(d.daily.len(), 2);
        assert_eq!(d.daily[0].ours.output_tokens, 600_000);
        assert_eq!(d.totals_theirs.input_tokens, 4628 + 9_753_786 + 10);
        assert_eq!(d.rows[0].model, "claude-opus-5-5", "biggest first");
    }
}
