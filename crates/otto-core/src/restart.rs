//! What a daemon restart keeps running and what it ends, counted by the
//! crates that own the work and merged by `GET /system/restart-impact`, so
//! the update / restart dialog tells the truth (D-016).

use serde::Serialize;

/// Names kept per item (session titles, run titles, room titles).
pub const DETAIL_MAX: usize = 5;

/// One kind of work on one side of a restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImpactItem {
    /// Stable id: `sessions`, `background_agents`, `connection_terminals`,
    /// `ide_runs`, `debug_sessions`, `container_runtime`,
    /// `broker_tunnels`, `session_rooms`, `live_browser`, `phones`.
    pub kind: &'static str,
    /// Noun phrase for `count` ("1 debug session", "2 debug sessions").
    pub label: &'static str,
    pub count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub detail: Vec<String>,
    #[serde(skip)]
    one: &'static str,
    #[serde(skip)]
    many: &'static str,
    #[serde(skip)]
    keeps: bool,
}

impl ImpactItem {
    /// `count` things of `kind`; `names` (may be fewer) become the detail.
    pub fn new(
        keeps: bool,
        kind: &'static str,
        (one, many): (&'static str, &'static str),
        count: usize,
        names: Vec<String>,
    ) -> Self {
        let mut detail: Vec<String> = names.into_iter().filter(|n| !n.trim().is_empty()).collect();
        detail.truncate(DETAIL_MAX);
        ImpactItem {
            kind,
            label: many,
            count,
            detail,
            one,
            many,
            keeps,
        }
    }

    /// One entry per name.
    pub fn named(
        keeps: bool,
        kind: &'static str,
        labels: (&'static str, &'static str),
        names: Vec<String>,
    ) -> Self {
        let n = names.len();
        Self::new(keeps, kind, labels, n, names)
    }

    /// A PTY or structured session, by the sidebar's buckets.
    pub fn session(keeps: bool, connection: bool, background: bool, title: String) -> Self {
        let (kind, labels) = if connection {
            (
                "connection_terminals",
                ("connection terminal", "connection terminals"),
            )
        } else if background {
            (
                "background_agents",
                ("background agent", "background agents"),
            )
        } else {
            (
                "sessions",
                ("agent or shell session", "agent and shell sessions"),
            )
        };
        Self::named(keeps, kind, labels, vec![title])
    }
}

/// The endpoint's body: items merged by kind, empty ones dropped.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct RestartImpact {
    pub keeps: Vec<ImpactItem>,
    pub ends: Vec<ImpactItem>,
}

impl From<Vec<ImpactItem>> for RestartImpact {
    fn from(items: Vec<ImpactItem>) -> Self {
        let mut out = RestartImpact::default();
        for it in items.into_iter().filter(|i| i.count > 0) {
            let side = if it.keeps {
                &mut out.keeps
            } else {
                &mut out.ends
            };
            match side.iter_mut().find(|x| x.kind == it.kind) {
                Some(x) => {
                    x.count += it.count;
                    x.detail.extend(it.detail);
                    x.detail.truncate(DETAIL_MAX);
                }
                None => side.push(it),
            }
        }
        for it in out.keeps.iter_mut().chain(out.ends.iter_mut()) {
            it.label = if it.count == 1 { it.one } else { it.many };
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_merge_by_kind_and_side_and_label_their_count() {
        let impact = RestartImpact::from(vec![
            ImpactItem::session(true, false, false, "fix the bug".into()),
            ImpactItem::session(true, false, false, "shell".into()),
            ImpactItem::session(false, true, false, "prod-db".into()),
            ImpactItem::session(false, false, true, "".into()),
            ImpactItem::session(false, false, true, "workflow".into()),
            ImpactItem::new(
                false,
                "debug_sessions",
                ("debug session", "debug sessions"),
                0,
                vec![],
            ),
            ImpactItem::new(
                false,
                "live_browser",
                ("live browser session", "live browser sessions"),
                1,
                vec![],
            ),
        ]);
        let v = serde_json::to_value(&impact).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "keeps": [{"kind": "sessions", "label": "agent and shell sessions", "count": 2, "detail": ["fix the bug", "shell"]}],
                "ends": [
                    {"kind": "connection_terminals", "label": "connection terminal", "count": 1, "detail": ["prod-db"]},
                    {"kind": "background_agents", "label": "background agents", "count": 2, "detail": ["workflow"]},
                    {"kind": "live_browser", "label": "live browser session", "count": 1},
                ],
            })
        );
    }

    #[test]
    fn detail_is_capped() {
        let names = (0..9).map(|i| format!("s{i}")).collect();
        let it = ImpactItem::named(false, "sessions", ("a", "b"), names);
        assert_eq!((it.count, it.detail.len()), (9, DETAIL_MAX));
        let merged = RestartImpact::from(vec![it.clone(), it]);
        assert_eq!(
            (merged.ends[0].count, merged.ends[0].detail.len()),
            (18, DETAIL_MAX)
        );
    }
}
