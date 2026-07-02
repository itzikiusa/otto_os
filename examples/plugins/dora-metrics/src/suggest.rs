//! Deterministic suggestions engine — evidence-carrying delivery advice
//! derived from the computed window stats. Always available (no AI); the
//! `/analyze` agent run builds on top of these.

use serde_json::{json, Value};

use crate::metrics::Tier;

/// Everything the rules need, precomputed by `metrics::compute`.
pub struct SuggestInput {
    pub deploy_tag_pattern: String,
    pub has_deploys: bool,
    pub df_per_week: f64,
    pub lead_median_h: Option<f64>,
    pub lead_p90_h: Option<f64>,
    pub lead_count: usize,
    pub cfr: Option<f64>,
    pub mttr_h: Option<f64>,
    pub tier_df: Option<Tier>,
    pub tier_lead: Option<Tier>,
    pub tier_mttr: Option<Tier>,
    pub batch_median: Option<f64>,
    pub feat_to_rel_h: Option<f64>,
    pub rel_to_dep_h: Option<f64>,
    pub deploy_ts: Vec<i64>,
    pub failed_tags: Vec<String>,
    pub unrecovered: u32,
    pub unshipped_merges: u32,
}

#[derive(Debug)]
pub struct Suggestion {
    pub severity: &'static str, // "critical" | "warn" | "info"
    pub title: String,
    pub detail: String,
}

impl Suggestion {
    pub fn to_json(&self) -> Value {
        json!({ "severity": self.severity, "title": self.title, "detail": self.detail })
    }
}

fn fmt_h(h: f64) -> String {
    if h < 48.0 {
        format!("{h:.1}h")
    } else {
        format!("{:.1}d", h / 24.0)
    }
}

/// Ordered rules (critical → warn → info). Each fires only when its signal
/// exists; numbers are quoted so every suggestion carries its evidence.
pub fn suggestions(inp: &SuggestInput) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = vec![];
    let mut push = |severity: &'static str, title: String, detail: String| {
        out.push(Suggestion {
            severity,
            title,
            detail,
        })
    };

    // 1. No deploy signal at all — nothing else can be measured.
    if !inp.has_deploys {
        push(
            "critical",
            "No deployments detected".into(),
            format!(
                "No tag containing \"{}\" was found in the window. Tag each production \
                 deploy (e.g. `v1.2-deployed`) or set your own pattern in the plugin \
                 settings (⚙) so the four DORA keys can be measured.",
                inp.deploy_tag_pattern
            ),
        );
        return out;
    }

    // 2. Change-failure rate (replaces its tier-gap rule at warn/critical).
    if let Some(cfr) = inp.cfr {
        let pct = cfr * 100.0;
        let tags = if inp.failed_tags.is_empty() {
            String::new()
        } else {
            let sample: Vec<&str> = inp.failed_tags.iter().take(3).map(String::as_str).collect();
            format!(" Failed deploys: {}.", sample.join(", "))
        };
        if cfr > 0.15 {
            push(
                "critical",
                format!("Change-failure rate is {pct:.0}%"),
                format!(
                    "More than 15% of deploys needed a hotfix.{tags} Strengthen \
                     pre-merge verification (tests, review, staging soak) before \
                     chasing speed — Elite teams stay at or below 5%."
                ),
            );
        } else if cfr > 0.10 {
            push(
                "warn",
                format!("Change-failure rate is {pct:.0}%"),
                format!(
                    "Above the 10% High-tier bar.{tags} Add a pre-deploy smoke \
                     gate or expand test coverage on the release path."
                ),
            );
        } else if cfr > 0.05 {
            push(
                "info",
                format!("Change-failure rate is {pct:.0}% — one tier from Elite"),
                "Elite teams keep failures at or below 5% of deploys.".into(),
            );
        }
    }

    // 3. Unrecovered failures — a hotfix landed but never went out.
    if inp.unrecovered > 0 {
        push(
            "warn",
            format!(
                "{} failed deploy{} not yet recovered",
                inp.unrecovered,
                if inp.unrecovered == 1 { "" } else { "s" }
            ),
            "A hotfix merged after the deploy but no subsequent deploy shipped it. \
             Deploy the fix — recovery time is still running."
                .into(),
        );
    }

    // 4. Pipeline bottleneck: which half dominates.
    if let (Some(fr), Some(rd)) = (inp.feat_to_rel_h, inp.rel_to_dep_h) {
        if rd > 2.0 * fr && rd > 0.0 {
            push(
                "warn",
                "Release→deploy is the bottleneck".into(),
                format!(
                    "Releases wait {} on average to deploy vs {} from feature to \
                     release. Automate the deploy step (CD) — the code is ready \
                     long before it ships.",
                    fmt_h(rd),
                    fmt_h(fr)
                ),
            );
        } else if fr > 2.0 * rd && fr > 0.0 {
            push(
                "warn",
                "Feature→release is the bottleneck".into(),
                format!(
                    "Features wait {} on average to enter a release vs {} from \
                     release to deploy. Cut release batching — smaller, more \
                     frequent release branches.",
                    fmt_h(fr),
                    fmt_h(rd)
                ),
            );
        }
    }

    // 5. Batch size.
    if let Some(b) = inp.batch_median {
        if b > 5.0 {
            push(
                "warn",
                format!("Large deploy batches (median {b:.0} merges/deploy)"),
                "Big batches raise failure blast radius and slow recovery. \
                 Deploy smaller changes more often."
                    .into(),
            );
        }
    }

    // 6. Long-tail lead times.
    if let (Some(med), Some(p90)) = (inp.lead_median_h, inp.lead_p90_h) {
        if inp.lead_count >= 4 && med > 0.0 && p90 > 3.0 * med {
            push(
                "warn",
                "Long-tail lead times".into(),
                format!(
                    "p90 lead time ({}) is more than 3× the median ({}). A few \
                     branches sit unmerged or undeployed far longer than the rest — \
                     find and split the stragglers.",
                    fmt_h(p90),
                    fmt_h(med)
                ),
            );
        }
    }

    // 7. Tier gaps (df / lead / mttr — CFR handled above).
    if let Some(t) = inp.tier_df {
        let gap = match t {
            Tier::Low => Some(("Medium", "at least monthly (≥0.23/week)")),
            Tier::Medium => Some(("High", "at least weekly (≥1/week)")),
            Tier::High => Some(("Elite", "on demand — daily or more (≥7/week)")),
            Tier::Elite => None,
        };
        if let Some((next, needs)) = gap {
            push(
                if matches!(t, Tier::High) {
                    "info"
                } else {
                    "warn"
                },
                format!("Deployment frequency is {} — path to {next}", t.as_str()),
                format!(
                    "Currently {:.2} deploys/week; {next} needs {needs}.",
                    inp.df_per_week
                ),
            );
        }
    }
    if let (Some(t), Some(med)) = (inp.tier_lead, inp.lead_median_h) {
        let gap = match t {
            Tier::Low => Some(("Medium", "under 1 month (720h)")),
            Tier::Medium => Some(("High", "under 1 week (168h)")),
            Tier::High => Some(("Elite", "under 24h")),
            Tier::Elite => None,
        };
        if let Some((next, needs)) = gap {
            push(
                if matches!(t, Tier::High) {
                    "info"
                } else {
                    "warn"
                },
                format!("Lead time is {} — path to {next}", t.as_str()),
                format!(
                    "Median merge→deploy is {}; {next} needs {needs}.",
                    fmt_h(med)
                ),
            );
        }
    }
    if let (Some(t), Some(m)) = (inp.tier_mttr, inp.mttr_h) {
        let gap = match t {
            Tier::Low => Some(("Medium", "under 1 week")),
            Tier::Medium => Some(("High", "under 1 day")),
            Tier::High => Some(("Elite", "under 1 hour")),
            Tier::Elite => None,
        };
        if let Some((next, needs)) = gap {
            push(
                if matches!(t, Tier::High) {
                    "info"
                } else {
                    "warn"
                },
                format!("Recovery time is {} — path to {next}", t.as_str()),
                format!(
                    "Median failed-deploy recovery is {}; {next} needs {needs}.",
                    fmt_h(m)
                ),
            );
        }
    }

    // 8. Cadence regularity (needs ≥3 deploys → ≥2 gaps).
    if inp.deploy_ts.len() >= 3 {
        let mut ts = inp.deploy_ts.clone();
        ts.sort_unstable();
        let gaps: Vec<f64> = ts.windows(2).map(|w| (w[1] - w[0]) as f64).collect();
        let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
        if mean > 0.0 {
            let var = gaps.iter().map(|g| (g - mean).powi(2)).sum::<f64>() / gaps.len() as f64;
            let cv = var.sqrt() / mean;
            if cv > 1.0 {
                push(
                    "info",
                    "Irregular deploy cadence".into(),
                    format!(
                        "Deploy gaps vary widely (CV {cv:.1}). A steady rhythm — even \
                         a fixed weekly slot — makes delivery predictable and \
                         failures easier to attribute."
                    ),
                );
            }
        }
    }

    // 9. Day-of-week clustering (needs ≥4 deploys).
    if inp.deploy_ts.len() >= 4 {
        let mut byday = [0usize; 7];
        for &ts in &inp.deploy_ts {
            byday[((ts / 86_400 + 4).rem_euclid(7)) as usize] += 1;
        }
        let (day, max) = byday
            .iter()
            .enumerate()
            .max_by_key(|(_, c)| **c)
            .map(|(d, c)| (d, *c))
            .unwrap();
        if max * 2 > inp.deploy_ts.len() {
            const DAYS: [&str; 7] = [
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
                "Monday",
                "Tuesday",
                "Wednesday",
            ];
            push(
                "info",
                format!("Deploys cluster on one day ({})", DAYS[day]),
                format!(
                    "{max} of {} deploys land on the same weekday — a batch-day \
                     pattern. Spreading deploys reduces risk concentration.",
                    inp.deploy_ts.len()
                ),
            );
        }
    }

    // 10. Merged but never deployed.
    if inp.unshipped_merges > 0 && inp.df_per_week > 0.0 {
        push(
            "info",
            format!("{} undeployed merge(s) in window", inp.unshipped_merges),
            "Work is merged but no deploy has shipped it yet — it accrues lead \
             time until the next deploy."
                .into(),
        );
    }

    let rank = |s: &str| match s {
        "critical" => 0,
        "warn" => 1,
        _ => 2,
    };
    out.sort_by_key(|s| rank(s.severity));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> SuggestInput {
        SuggestInput {
            deploy_tag_pattern: "deploy".into(),
            has_deploys: true,
            df_per_week: 7.5,
            lead_median_h: Some(10.0),
            lead_p90_h: Some(20.0),
            lead_count: 10,
            cfr: Some(0.02),
            mttr_h: Some(0.5),
            tier_df: Some(Tier::Elite),
            tier_lead: Some(Tier::Elite),
            tier_mttr: Some(Tier::Elite),
            batch_median: Some(2.0),
            feat_to_rel_h: Some(24.0),
            rel_to_dep_h: Some(24.0),
            deploy_ts: vec![0, 86_400, 172_800, 259_200],
            failed_tags: vec![],
            unrecovered: 0,
            unshipped_merges: 0,
        }
    }

    fn titles(v: &[Suggestion]) -> Vec<String> {
        v.iter().map(|s| s.title.to_lowercase()).collect()
    }

    #[test]
    fn quiet_elite_dataset_yields_nothing() {
        let s = suggestions(&base());
        assert!(s.is_empty(), "unexpected: {:?}", titles(&s));
    }

    #[test]
    fn no_deploys_is_critical() {
        let inp = SuggestInput {
            has_deploys: false,
            df_per_week: 0.0,
            lead_median_h: None,
            lead_p90_h: None,
            lead_count: 0,
            cfr: None,
            mttr_h: None,
            tier_df: None,
            tier_lead: None,
            tier_mttr: None,
            batch_median: None,
            deploy_ts: vec![],
            ..base()
        };
        let s = suggestions(&inp);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].severity, "critical");
        assert!(s[0].detail.contains("deploy"), "mentions the tag pattern");
    }

    #[test]
    fn tier_gap_severities() {
        // High lead → info; Medium df → warn.
        let inp = SuggestInput {
            df_per_week: 0.5,
            tier_df: Some(Tier::Medium),
            lead_median_h: Some(100.0),
            tier_lead: Some(Tier::High),
            ..base()
        };
        let s = suggestions(&inp);
        let df = s
            .iter()
            .find(|x| x.title.to_lowercase().contains("deployment frequency"))
            .unwrap();
        assert_eq!(df.severity, "warn");
        let lead = s
            .iter()
            .find(|x| x.title.to_lowercase().contains("lead time"))
            .unwrap();
        assert_eq!(lead.severity, "info");
        assert!(
            lead.detail.contains("24"),
            "cites the Elite threshold: {}",
            lead.detail
        );
    }

    #[test]
    fn cfr_thresholds() {
        let mut inp = base();
        inp.cfr = Some(0.2);
        inp.failed_tags = vec!["v1-deployed".into(), "v2-deployed".into()];
        let s = suggestions(&inp);
        let c = s
            .iter()
            .find(|x| x.title.to_lowercase().contains("failure"))
            .unwrap();
        assert_eq!(c.severity, "critical");
        assert!(c.detail.contains("v1-deployed"));
        inp.cfr = Some(0.12);
        let s = suggestions(&inp);
        assert_eq!(
            s.iter()
                .find(|x| x.title.to_lowercase().contains("failure"))
                .unwrap()
                .severity,
            "warn"
        );
        inp.cfr = Some(0.07);
        let s = suggestions(&inp);
        assert_eq!(
            s.iter()
                .find(|x| x.title.to_lowercase().contains("failure"))
                .unwrap()
                .severity,
            "info"
        );
    }

    #[test]
    fn bottleneck_release_to_deploy() {
        let inp = SuggestInput {
            feat_to_rel_h: Some(10.0),
            rel_to_dep_h: Some(30.0),
            ..base()
        };
        let s = suggestions(&inp);
        let b = s
            .iter()
            .find(|x| x.title.to_lowercase().contains("bottleneck"))
            .unwrap();
        assert_eq!(b.severity, "warn");
        assert!(b.detail.contains("release"), "{}", b.detail);
    }

    #[test]
    fn batch_size_rule() {
        let inp = SuggestInput {
            batch_median: Some(6.0),
            ..base()
        };
        let s = suggestions(&inp);
        assert!(titles(&s).iter().any(|t| t.contains("batch")));
    }

    #[test]
    fn long_tail_rule_needs_four_leads() {
        let mut inp = base();
        inp.lead_median_h = Some(10.0);
        inp.lead_p90_h = Some(40.0);
        inp.lead_count = 4;
        inp.tier_lead = Some(Tier::Elite);
        let s = suggestions(&inp);
        assert!(titles(&s).iter().any(|t| t.contains("tail")));
        inp.lead_count = 3;
        let s = suggestions(&inp);
        assert!(!titles(&s).iter().any(|t| t.contains("tail")));
    }

    #[test]
    fn irregular_cadence_rule() {
        // Gaps 1d, 1d, 20d → CV > 1.
        let inp = SuggestInput {
            deploy_ts: vec![0, 86_400, 172_800, 1_900_800],
            ..base()
        };
        let s = suggestions(&inp);
        assert!(
            titles(&s).iter().any(|t| t.contains("cadence")),
            "{:?}",
            titles(&s)
        );
    }

    #[test]
    fn weekday_clustering_rule() {
        // 4 deploys, 3 on the same weekday (Mondays: 345600 + k*604800).
        let inp = SuggestInput {
            deploy_ts: vec![345_600, 950_400, 1_555_200, 431_999],
            ..base()
        };
        let s = suggestions(&inp);
        assert!(
            titles(&s).iter().any(|t| t.contains("day")),
            "{:?}",
            titles(&s)
        );
    }

    #[test]
    fn unrecovered_and_unshipped_rules() {
        let inp = SuggestInput {
            unrecovered: 1,
            unshipped_merges: 3,
            ..base()
        };
        let s = suggestions(&inp);
        let unrec = s
            .iter()
            .find(|x| x.title.to_lowercase().contains("recover"))
            .unwrap();
        assert_eq!(unrec.severity, "warn");
        let unshipped = s
            .iter()
            .find(|x| x.title.to_lowercase().contains("undeployed"))
            .unwrap();
        assert_eq!(unshipped.severity, "info");
    }

    #[test]
    fn ordered_critical_first() {
        let inp = SuggestInput {
            cfr: Some(0.5),
            unrecovered: 2,
            unshipped_merges: 1,
            failed_tags: vec!["t".into()],
            ..base()
        };
        let s = suggestions(&inp);
        assert!(s.len() >= 3);
        let sev_rank = |x: &str| match x {
            "critical" => 0,
            "warn" => 1,
            _ => 2,
        };
        let ranks: Vec<i32> = s.iter().map(|x| sev_rank(x.severity)).collect();
        let mut sorted = ranks.clone();
        sorted.sort();
        assert_eq!(ranks, sorted, "not ordered by severity: {ranks:?}");
    }
}
