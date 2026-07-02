//! dora-metrics library — pure metric/suggestion engines + HTTP routing,
//! exposed as a lib so the integration tests (`tests/`) can drive them; the
//! binary (`main.rs`) is a thin tiny_http loop over [`routes::handle`].

pub mod config;
pub mod metrics;
