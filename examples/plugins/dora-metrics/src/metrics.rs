//! DORA metric engine — pure computation over a stream of git commits.
//!
//! Signals (git-only heuristics, documented in the UI methodology note):
//! deploy = tag containing the configured pattern; merge = ≥2-parent commit
//! whose subject carries a configured branch prefix (hotfix > release >
//! feature). Lead time pairs each merge with the first subsequent deploy —
//! merges with none are censored (`unshipped_merges`). A deploy is *failed*
//! iff a hotfix merge lands before the next deploy; recovery = time to that
//! next deploy (`unrecovered` when there is none).

use std::process::Command;

use serde_json::{json, Value};

use crate::config::Config;

pub struct Commit {
    pub ts: i64,
    pub subject: String,
    pub parents: usize,
    pub refs: Vec<String>,
    pub repo: String,
}

/// Load up to `depth` commits (all refs) with timestamp/parents/subject/refs.
pub fn load_commits(repo_path: &str, repo_name: &str, depth: usize) -> Vec<Commit> {
    let out = Command::new("git")
        .args([
            "-C",
            repo_path,
            "log",
            "--all",
            "-n",
            &depth.to_string(),
            "--pretty=%ct\x1f%P\x1f%s\x1f%D",
        ])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    out.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\x1f').collect();
            if f.len() < 4 {
                return None;
            }
            let ts = f[0].trim().parse::<i64>().ok()?;
            let parents = f[1].split_whitespace().count();
            let refs = if f[3].is_empty() {
                vec![]
            } else {
                f[3].split(", ").map(|s| s.to_string()).collect()
            };
            Some(Commit {
                ts,
                subject: f[2].to_string(),
                parents,
                refs,
                repo: repo_name.to_string(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tiers (DORA performance levels; thresholds shown in the UI methodology note)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Elite,
    High,
    Medium,
    Low,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Elite => "elite",
            Tier::High => "high",
            Tier::Medium => "medium",
            Tier::Low => "low",
        }
    }
}

/// Deployment frequency: Elite ≥ daily, High ≥ weekly, Medium ≥ monthly.
pub fn tier_df(per_week: f64) -> Tier {
    if per_week >= 7.0 {
        Tier::Elite
    } else if per_week >= 1.0 {
        Tier::High
    } else if per_week >= 7.0 / 30.0 {
        Tier::Medium
    } else {
        Tier::Low
    }
}

/// Lead time for changes (median hours): Elite <1d, High <1w, Medium <1mo.
pub fn tier_lead(median_h: f64) -> Tier {
    if median_h < 24.0 {
        Tier::Elite
    } else if median_h < 168.0 {
        Tier::High
    } else if median_h < 720.0 {
        Tier::Medium
    } else {
        Tier::Low
    }
}

/// Change-failure rate: Elite ≤5%, High ≤10%, Medium ≤15%.
pub fn tier_cfr(rate: f64) -> Tier {
    if rate <= 0.05 {
        Tier::Elite
    } else if rate <= 0.10 {
        Tier::High
    } else if rate <= 0.15 {
        Tier::Medium
    } else {
        Tier::Low
    }
}

/// Failed-deployment recovery time (median hours): Elite <1h, High <1d,
/// Medium <1w.
pub fn tier_mttr(median_h: f64) -> Tier {
    if median_h < 1.0 {
        Tier::Elite
    } else if median_h < 24.0 {
        Tier::High
    } else if median_h < 168.0 {
        Tier::Medium
    } else {
        Tier::Low
    }
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Percentile with linear interpolation over a pre-sorted slice.
pub(crate) fn percentile(sorted: &[f64], q: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let pos = (sorted.len() - 1) as f64 * q / 100.0;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    Some(sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64))
}

/// Epoch-aligned UTC Monday 00:00 of the week containing `ts`
/// (epoch + 4 days = Mon 1970-01-05).
pub(crate) fn week_start(ts: i64) -> i64 {
    ts - (ts - 345_600).rem_euclid(604_800)
}

/// Merge classification by configured branch prefixes (hotfix > release >
/// feature), case-insensitive substring on the subject.
pub(crate) fn classify(subject: &str, cfg: &Config) -> Option<&'static str> {
    let s = subject.to_lowercase();
    let hit = |list: &[String]| list.iter().any(|p| s.contains(&p.to_lowercase()));
    if hit(&cfg.branch_prefixes.hotfix) {
        Some("hotfix")
    } else if hit(&cfg.branch_prefixes.release) {
        Some("release")
    } else if hit(&cfg.branch_prefixes.feature) {
        Some("feature")
    } else {
        None
    }
}

/// First tag on the commit matching the configured deploy pattern.
pub(crate) fn deploy_tag(refs: &[String], pattern: &str) -> Option<String> {
    let pat = pattern.to_lowercase();
    for r in refs {
        if let Some(tag) = r.trim().strip_prefix("tag: ") {
            if tag.to_lowercase().contains(&pat) {
                return Some(tag.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Computation
// ---------------------------------------------------------------------------

struct Deploy {
    ts: i64,
    tag: String,
    repo: String,
    failed: bool,
}

struct MergeRec {
    ts: i64,
    kind: &'static str,
    subject: String,
    repo: String,
}

struct WindowStats {
    deploys: Vec<Deploy>,
    merges: Vec<MergeRec>,
    /// (deploy_ts, lead hours) per shipped merge.
    leads: Vec<(i64, f64)>,
    unshipped: u32,
    /// (failed_deploy_ts, recovery hours) per recovered failure.
    recoveries: Vec<(i64, f64)>,
    unrecovered: u32,
    df_per_week: f64,
    lead_median: Option<f64>,
    lead_p90: Option<f64>,
    cfr: Option<f64>,
    mttr: Option<f64>,
    batch_median: Option<f64>,
}

/// All window math over the closed interval `[from, to]`. `span_days` is the
/// nominal window length used for the deploys/week rate.
fn window_stats(
    commits: &[Commit],
    from: i64,
    to: i64,
    span_days: i64,
    cfg: &Config,
) -> WindowStats {
    let mut deploys: Vec<Deploy> = vec![];
    let mut merges: Vec<MergeRec> = vec![];
    for c in commits {
        if c.ts < from || c.ts > to {
            continue;
        }
        if let Some(tag) = deploy_tag(&c.refs, &cfg.deploy_tag_pattern) {
            deploys.push(Deploy {
                ts: c.ts,
                tag,
                repo: c.repo.clone(),
                failed: false,
            });
        }
        if c.parents >= 2 {
            if let Some(kind) = classify(&c.subject, cfg) {
                merges.push(MergeRec {
                    ts: c.ts,
                    kind,
                    subject: c.subject.clone(),
                    repo: c.repo.clone(),
                });
            }
        }
    }
    deploys.sort_by_key(|d| d.ts);
    merges.sort_by_key(|m| m.ts);

    // Lead time: each merge pairs with the first subsequent deploy; merges
    // with none are censored (counted, excluded from the median).
    let mut leads: Vec<(i64, f64)> = vec![];
    let mut unshipped = 0u32;
    for m in &merges {
        match deploys.iter().find(|d| d.ts >= m.ts) {
            Some(d) => leads.push((d.ts, (d.ts - m.ts) as f64 / 3600.0)),
            None => unshipped += 1,
        }
    }

    // Failure + recovery: a deploy is failed iff a hotfix merge lands before
    // the next deploy (or window end); recovery = time to that next deploy.
    let mut recoveries: Vec<(i64, f64)> = vec![];
    let mut unrecovered = 0u32;
    for i in 0..deploys.len() {
        let wend = deploys.get(i + 1).map(|d| d.ts).unwrap_or(to);
        let failed = merges
            .iter()
            .any(|m| m.kind == "hotfix" && m.ts > deploys[i].ts && m.ts <= wend);
        deploys[i].failed = failed;
        if failed {
            match deploys.get(i + 1) {
                Some(next) => {
                    recoveries.push((deploys[i].ts, (next.ts - deploys[i].ts) as f64 / 3600.0))
                }
                None => unrecovered += 1,
            }
        }
    }

    // Batch size: merges landing between consecutive deploys.
    let mut batches: Vec<f64> = vec![];
    let mut prev = from - 1;
    for d in &deploys {
        batches.push(
            merges
                .iter()
                .filter(|m| m.ts > prev && m.ts <= d.ts)
                .count() as f64,
        );
        prev = d.ts;
    }
    batches.sort_by(|a, b| a.total_cmp(b));

    let mut lead_hours: Vec<f64> = leads.iter().map(|l| l.1).collect();
    lead_hours.sort_by(|a, b| a.total_cmp(b));
    let mut rec_hours: Vec<f64> = recoveries.iter().map(|r| r.1).collect();
    rec_hours.sort_by(|a, b| a.total_cmp(b));

    let failed_count = deploys.iter().filter(|d| d.failed).count();
    WindowStats {
        df_per_week: deploys.len() as f64 * 7.0 / span_days.max(1) as f64,
        lead_median: percentile(&lead_hours, 50.0),
        lead_p90: percentile(&lead_hours, 90.0),
        cfr: if deploys.is_empty() {
            None
        } else {
            Some(failed_count as f64 / deploys.len() as f64)
        },
        mttr: percentile(&rec_hours, 50.0),
        batch_median: percentile(&batches, 50.0),
        deploys,
        merges,
        leads,
        unshipped,
        recoveries,
        unrecovered,
    }
}

/// Mean gap (hours) from each target to the latest source at or before it.
fn avg_gap(targets: &[i64], sources: &[i64]) -> Option<f64> {
    let gaps: Vec<f64> = targets
        .iter()
        .filter_map(|&t| {
            sources
                .iter()
                .filter(|&&s| s <= t)
                .max()
                .map(|&s| (t - s) as f64 / 3600.0)
        })
        .collect();
    if gaps.is_empty() {
        None
    } else {
        Some(gaps.iter().sum::<f64>() / gaps.len() as f64)
    }
}

fn opt(v: Option<f64>) -> Value {
    v.map(|x| json!(x)).unwrap_or(Value::Null)
}

fn tier_json(t: Option<Tier>) -> Value {
    t.map(|x| json!(x.as_str())).unwrap_or(Value::Null)
}

/// Full `/metrics` payload (spec §A3), suggestions included. Pure — `now`
/// injected.
pub fn compute(commits: &[Commit], days: i64, label: &str, now: i64, cfg: &Config) -> Value {
    let days = days.max(1);
    let from = now - days * 86_400;
    let cur = window_stats(commits, from, now, days, cfg);
    let prev = window_stats(commits, from - days * 86_400, from - 1, days, cfg);

    // Tiers: null when the metric has no signal; overall = worst present.
    let has_deploys = !cur.deploys.is_empty();
    let t_df = has_deploys.then(|| tier_df(cur.df_per_week));
    let t_lead = cur.lead_median.map(tier_lead);
    let t_cfr = cur.cfr.map(tier_cfr);
    let t_mttr = cur.mttr.map(tier_mttr);
    let overall = [t_df, t_lead, t_cfr, t_mttr].into_iter().flatten().max();

    // Deltas vs the previous window; null when the previous has no signal.
    let d_df = (!prev.deploys.is_empty()).then_some(cur.df_per_week - prev.df_per_week);
    let d_lead = cur.lead_median.zip(prev.lead_median).map(|(a, b)| a - b);
    let d_cfr = cur.cfr.zip(prev.cfr).map(|(a, b)| a - b);
    let d_mttr = cur.mttr.zip(prev.mttr).map(|(a, b)| a - b);

    // Weekly buckets (epoch-aligned UTC Mondays); edge buckets are partial.
    let mut weekly: Vec<Value> = vec![];
    let mut w = week_start(from);
    while w <= week_start(now) {
        let wend = w + 604_800;
        let in_bucket = |ts: i64| ts >= w && ts < wend;
        let deps: Vec<&Deploy> = cur.deploys.iter().filter(|d| in_bucket(d.ts)).collect();
        let mut bl: Vec<f64> = cur
            .leads
            .iter()
            .filter(|(dts, _)| in_bucket(*dts))
            .map(|(_, h)| *h)
            .collect();
        bl.sort_by(|a, b| a.total_cmp(b));
        let mut br: Vec<f64> = cur
            .recoveries
            .iter()
            .filter(|(fts, _)| in_bucket(*fts))
            .map(|(_, h)| *h)
            .collect();
        br.sort_by(|a, b| a.total_cmp(b));
        let kind_count = |k: &str| {
            cur.merges
                .iter()
                .filter(|m| m.kind == k && in_bucket(m.ts))
                .count()
        };
        let failed = deps.iter().filter(|d| d.failed).count();
        weekly.push(json!({
            "week_start": w,
            "deploys": deps.len(),
            "lead_median_h": opt(percentile(&bl, 50.0)),
            "cfr": if deps.is_empty() { Value::Null } else { json!(failed as f64 / deps.len() as f64) },
            "mttr_h": opt(percentile(&br, 50.0)),
            "feature": kind_count("feature"),
            "release": kind_count("release"),
            "hotfix": kind_count("hotfix"),
        }));
        w = wend;
    }

    let count = |k: &str| cur.merges.iter().filter(|m| m.kind == k).count();
    let feat_ts: Vec<i64> = cur
        .merges
        .iter()
        .filter(|m| m.kind == "feature")
        .map(|m| m.ts)
        .collect();
    let rel_ts: Vec<i64> = cur
        .merges
        .iter()
        .filter(|m| m.kind == "release")
        .map(|m| m.ts)
        .collect();
    let dep_ts: Vec<i64> = cur.deploys.iter().map(|d| d.ts).collect();

    let mut recent: Vec<&MergeRec> = cur.merges.iter().collect();
    recent.sort_by_key(|m| std::cmp::Reverse(m.ts));
    recent.truncate(50);

    let sugg = crate::suggest::suggestions(&crate::suggest::SuggestInput {
        deploy_tag_pattern: cfg.deploy_tag_pattern.clone(),
        has_deploys,
        df_per_week: cur.df_per_week,
        lead_median_h: cur.lead_median,
        lead_p90_h: cur.lead_p90,
        lead_count: cur.leads.len(),
        cfr: cur.cfr,
        mttr_h: cur.mttr,
        tier_df: t_df,
        tier_lead: t_lead,
        tier_mttr: t_mttr,
        batch_median: cur.batch_median,
        feat_to_rel_h: avg_gap(&rel_ts, &feat_ts),
        rel_to_dep_h: avg_gap(&dep_ts, &rel_ts),
        deploy_ts: dep_ts.clone(),
        failed_tags: cur
            .deploys
            .iter()
            .filter(|d| d.failed)
            .map(|d| d.tag.clone())
            .collect(),
        unrecovered: cur.unrecovered,
        unshipped_merges: cur.unshipped,
    });

    json!({
        "repo_name": label,
        "window_days": days,
        "df_per_week": cur.df_per_week,
        "lead_median_h": opt(cur.lead_median),
        "lead_p90_h": opt(cur.lead_p90),
        "cfr": opt(cur.cfr),
        "mttr_h": opt(cur.mttr),
        "unrecovered": cur.unrecovered,
        "unshipped_merges": cur.unshipped,
        "tiers": {
            "df": tier_json(t_df),
            "lead": tier_json(t_lead),
            "cfr": tier_json(t_cfr),
            "mttr": tier_json(t_mttr),
            "overall": tier_json(overall),
        },
        "deltas": {
            "df_per_week": opt(d_df),
            "lead_median_h": opt(d_lead),
            "cfr": opt(d_cfr),
            "mttr_h": opt(d_mttr),
        },
        "weekly": weekly,
        "counts": { "feature": count("feature"), "release": count("release"), "hotfix": count("hotfix") },
        "batch_median": opt(cur.batch_median),
        "avg_feature_to_release_hours": opt(avg_gap(&rel_ts, &feat_ts)),
        "avg_release_to_deploy_hours": opt(avg_gap(&dep_ts, &rel_ts)),
        "deployments": cur.deploys.iter().map(|d| json!({"ts": d.ts, "tag": d.tag, "failed": d.failed, "repo": d.repo})).collect::<Vec<_>>(),
        "recent_merges": recent.iter().map(|m| json!({"ts": m.ts, "kind": m.kind, "subject": m.subject, "repo": m.repo})).collect::<Vec<_>>(),
        "suggestions": sugg.iter().map(|s| s.to_json()).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    const D: i64 = 86_400;
    const NOW: i64 = 1_770_000_000;

    fn mk(ts: i64, subject: &str, parents: usize, refs: &[&str], repo: &str) -> Commit {
        Commit {
            ts,
            subject: subject.into(),
            parents,
            refs: refs.iter().map(|s| s.to_string()).collect(),
            repo: repo.into(),
        }
    }

    fn cfg() -> Config {
        Config::default()
    }

    fn f(v: &Value, k: &str) -> f64 {
        v.get(k)
            .and_then(Value::as_f64)
            .unwrap_or_else(|| panic!("missing f64 {k}: {v}"))
    }

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    #[test]
    fn classify_order_and_custom_prefixes() {
        let c = cfg();
        assert_eq!(
            classify("Merge branch 'HOTFIX/x' into feature/y", &c),
            Some("hotfix")
        );
        assert_eq!(classify("Merge branch 'feature/a'", &c), Some("feature"));
        assert_eq!(
            classify("Merge pull request #5 from x/release/1.2", &c),
            Some("release")
        );
        assert_eq!(classify("plain commit", &c), None);
        let mut custom = cfg();
        custom.branch_prefixes.feature = vec!["feat/".into()];
        assert_eq!(classify("Merge branch 'feat/z'", &custom), Some("feature"));
        assert_eq!(classify("Merge branch 'feature/z'", &custom), None);
    }

    #[test]
    fn deploy_tag_pattern_matching() {
        let refs = vec!["tag: v1-deployed".to_string(), "origin/main".to_string()];
        assert_eq!(deploy_tag(&refs, "deploy"), Some("v1-deployed".into()));
        assert_eq!(deploy_tag(&refs, "prod-"), None);
        let refs2 = vec!["tag: PROD-42".to_string()];
        assert_eq!(deploy_tag(&refs2, "prod-"), Some("PROD-42".into()));
        assert_eq!(deploy_tag(&[], "deploy"), None);
    }

    #[test]
    fn percentile_interpolates() {
        assert_eq!(percentile(&[], 50.0), None);
        approx(percentile(&[5.0], 90.0).unwrap(), 5.0);
        approx(percentile(&[48.0, 120.0], 50.0).unwrap(), 84.0);
        approx(percentile(&[48.0, 120.0], 90.0).unwrap(), 112.8);
        approx(percentile(&[1.0, 2.0, 3.0], 50.0).unwrap(), 2.0);
    }

    #[test]
    fn week_start_is_epoch_aligned_monday() {
        // 345600 = Mon 1970-01-05 00:00 UTC.
        assert_eq!(week_start(345_600), 345_600);
        assert_eq!(week_start(345_600 + 604_799), 345_600);
        assert_eq!(week_start(345_600 + 604_800), 950_400);
        assert_eq!((week_start(NOW) - 345_600) % 604_800, 0);
        assert!(NOW - week_start(NOW) < 604_800);
    }

    #[test]
    fn tier_boundaries() {
        assert_eq!(tier_df(7.0), Tier::Elite);
        assert_eq!(tier_df(6.99), Tier::High);
        assert_eq!(tier_df(1.0), Tier::High);
        assert_eq!(tier_df(7.0 / 30.0), Tier::Medium);
        assert_eq!(tier_df(0.2), Tier::Low);
        assert_eq!(tier_lead(23.99), Tier::Elite);
        assert_eq!(tier_lead(24.0), Tier::High);
        assert_eq!(tier_lead(168.0), Tier::Medium);
        assert_eq!(tier_lead(720.0), Tier::Low);
        assert_eq!(tier_cfr(0.05), Tier::Elite);
        assert_eq!(tier_cfr(0.10), Tier::High);
        assert_eq!(tier_cfr(0.15), Tier::Medium);
        assert_eq!(tier_cfr(0.151), Tier::Low);
        assert_eq!(tier_mttr(0.99), Tier::Elite);
        assert_eq!(tier_mttr(1.0), Tier::High);
        assert_eq!(tier_mttr(24.0), Tier::Medium);
        assert_eq!(tier_mttr(168.0), Tier::Low);
        // Ord: worst = max.
        assert!(Tier::Low > Tier::Medium && Tier::Medium > Tier::High && Tier::High > Tier::Elite);
    }

    /// 28-day window: 2 deploys, censored merges, one failure without recovery.
    #[test]
    fn compute_core_scenario() {
        let commits = vec![
            mk(NOW - 25 * D, "Merge branch 'feature/one'", 2, &[], "r"),
            mk(NOW - 20 * D, "release v1", 1, &["tag: v1-deployed"], "r"),
            mk(NOW - 12 * D, "Merge branch 'release/1.0'", 2, &[], "r"),
            mk(NOW - 10 * D, "release v2", 1, &["tag: v2-deployed"], "r"),
            mk(NOW - 8 * D, "Merge branch 'hotfix/fix'", 2, &[], "r"),
            mk(NOW - 5 * D, "Merge branch 'feature/two'", 2, &[], "r"),
        ];
        let m = compute(&commits, 28, "r", NOW, &cfg());
        assert_eq!(m["repo_name"], "r");
        assert_eq!(m["window_days"], 28);
        approx(f(&m, "df_per_week"), 0.5);
        approx(f(&m, "lead_median_h"), 84.0);
        approx(f(&m, "lead_p90_h"), 112.8);
        approx(f(&m, "cfr"), 0.5);
        assert!(m["mttr_h"].is_null());
        assert_eq!(m["unrecovered"], 1);
        assert_eq!(m["unshipped_merges"], 2);
        assert_eq!(m["tiers"]["df"], "medium");
        assert_eq!(m["tiers"]["lead"], "high");
        assert_eq!(m["tiers"]["cfr"], "low");
        assert!(m["tiers"]["mttr"].is_null());
        assert_eq!(m["tiers"]["overall"], "low");
        assert_eq!(m["counts"]["feature"], 2);
        assert_eq!(m["counts"]["release"], 1);
        assert_eq!(m["counts"]["hotfix"], 1);
        approx(f(&m, "batch_median"), 1.0);
        approx(f(&m, "avg_feature_to_release_hours"), 312.0);
        approx(f(&m, "avg_release_to_deploy_hours"), 48.0);
        let deps = m["deployments"].as_array().unwrap();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0]["failed"], false);
        assert_eq!(deps[1]["failed"], true);
        assert_eq!(deps[0]["repo"], "r");
        // Previous window is empty → all deltas null.
        for k in ["df_per_week", "lead_median_h", "cfr", "mttr_h"] {
            assert!(m["deltas"][k].is_null(), "delta {k} should be null");
        }
        // Weekly buckets: Monday-aligned, zero-filled, totals match.
        let weekly = m["weekly"].as_array().unwrap();
        assert!(
            weekly.len() == 4 || weekly.len() == 5,
            "got {}",
            weekly.len()
        );
        let mut dep_total = 0i64;
        let mut hot_total = 0i64;
        for w in weekly {
            assert_eq!((w["week_start"].as_i64().unwrap() - 345_600) % 604_800, 0);
            dep_total += w["deploys"].as_i64().unwrap();
            hot_total += w["hotfix"].as_i64().unwrap();
        }
        assert_eq!(dep_total, 2);
        assert_eq!(hot_total, 1);
        // Suggestions slot exists (filled by suggest module at the route layer).
        assert!(m["suggestions"].is_array());
    }

    #[test]
    fn compute_mttr_recovery() {
        let commits = vec![
            mk(NOW - 20 * D, "v1", 1, &["tag: v1-deployed"], "r"),
            mk(NOW - 18 * D, "Merge branch 'hotfix/oops'", 2, &[], "r"),
            mk(NOW - 16 * D, "v2", 1, &["tag: v2-deployed"], "r"),
        ];
        let m = compute(&commits, 28, "r", NOW, &cfg());
        approx(f(&m, "cfr"), 0.5);
        approx(f(&m, "mttr_h"), 96.0);
        assert_eq!(m["unrecovered"], 0);
        assert_eq!(m["tiers"]["mttr"], "medium");
    }

    #[test]
    fn compute_deltas_vs_previous_window() {
        let commits = vec![
            // previous window [NOW-56d, NOW-28d)
            mk(NOW - 42 * D, "Merge branch 'feature/p'", 2, &[], "r"),
            mk(NOW - 40 * D, "v0", 1, &["tag: v0-deployed"], "r"),
            // current window
            mk(NOW - 21 * D, "Merge branch 'feature/c'", 2, &[], "r"),
            mk(NOW - 20 * D, "v1", 1, &["tag: v1-deployed"], "r"),
        ];
        let m = compute(&commits, 28, "r", NOW, &cfg());
        approx(f(&m, "df_per_week"), 0.25);
        approx(m["deltas"]["df_per_week"].as_f64().unwrap(), 0.0);
        approx(m["deltas"]["lead_median_h"].as_f64().unwrap(), 24.0 - 48.0);
        approx(m["deltas"]["cfr"].as_f64().unwrap(), 0.0);
        assert!(m["deltas"]["mttr_h"].is_null());
    }

    #[test]
    fn compute_no_deploys_all_null_tiers() {
        let commits = vec![mk(NOW - 3 * D, "Merge branch 'feature/x'", 2, &[], "r")];
        let m = compute(&commits, 14, "r", NOW, &cfg());
        approx(f(&m, "df_per_week"), 0.0);
        assert!(m["lead_median_h"].is_null());
        assert!(m["cfr"].is_null());
        assert!(m["mttr_h"].is_null());
        for k in ["df", "lead", "cfr", "mttr", "overall"] {
            assert!(m["tiers"][k].is_null(), "tier {k} should be null");
        }
        assert_eq!(m["unshipped_merges"], 1);
        assert!(m["deployments"].as_array().unwrap().is_empty());
    }

    #[test]
    fn compute_multi_repo_labels() {
        let commits = vec![
            mk(NOW - 9 * D, "va", 1, &["tag: a-deployed"], "repo-a"),
            mk(NOW - 4 * D, "vb", 1, &["tag: b-deployed"], "repo-b"),
        ];
        let m = compute(&commits, 14, "All repos", NOW, &cfg());
        assert_eq!(m["repo_name"], "All repos");
        let repos: Vec<&str> = m["deployments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["repo"].as_str().unwrap())
            .collect();
        assert!(repos.contains(&"repo-a") && repos.contains(&"repo-b"));
    }

    #[test]
    fn compute_custom_deploy_pattern() {
        let mut c = cfg();
        c.deploy_tag_pattern = "prod-".into();
        let commits = vec![
            mk(NOW - 9 * D, "x", 1, &["tag: v1-deployed"], "r"),
            mk(NOW - 4 * D, "y", 1, &["tag: prod-42"], "r"),
        ];
        let m = compute(&commits, 14, "r", NOW, &c);
        let deps = m["deployments"].as_array().unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0]["tag"], "prod-42");
    }
}
