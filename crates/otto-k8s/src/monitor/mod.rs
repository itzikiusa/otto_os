//! Monitoring: user-defined pod probes → ClickHouse samples, restart
//! classification, dashboard queries and the `k8s_health` aggregate.
//! See `docs/superpowers/specs/2026-09-05-k8s-monitoring-dashboard-design.md`.
//!
//! | module | responsibility |
//! |---|---|
//! | [`schema`] | ClickHouse DDL + per-cluster purge |
pub mod schema;
