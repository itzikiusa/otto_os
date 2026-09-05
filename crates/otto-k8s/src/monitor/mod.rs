//! Monitoring: user-defined pod probes → ClickHouse samples, restart
//! classification, dashboard queries and the `k8s_health` aggregate.
//! See `docs/superpowers/specs/2026-09-05-k8s-monitoring-dashboard-design.md`.
//!
//! | module | responsibility |
//! |---|---|
//! | [`schema`] | ClickHouse DDL + per-cluster purge |
//! | [`probes`] | config model, validation, globs, exclusions, presets |
//! | [`parse`] | prometheus text / JSON-mapping / health parsers → `Sample`s |
//! | [`scrape`] | transport pick (API-server proxy vs port-forward) + HTTP fetch |
//! | [`classify`] | pod snapshots, diff, restart / churn classification |
//! | [`collector`] | one cycle + the per-cluster loop |
//! | [`queries`] | ClickHouse SQL builders for the dashboard |
//! | [`health`] | the compact `k8s_health` digest |
//! | [`http`] | `/k8s/monitor/*` + `/k8s/clusters/{id}/monitor*` routes |
pub mod parse;
pub mod probes;
pub mod schema;
