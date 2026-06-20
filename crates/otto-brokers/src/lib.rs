//! Otto Message Brokers — Kafka cluster viewer engine.
//!
//! Layers: [`types`] (domain + DTOs) → [`decode`] / [`schema_registry`] /
//! [`metrics`] (rendering) → [`kafka`] (rdkafka driver) → `service` (façade +
//! client pool) → `http` (`api_router::<S: BrokersCtx>()`).

pub mod decode;
pub mod http;
pub mod kafka;
pub mod metrics;
pub mod schema_registry;
pub mod service;
pub mod types;

pub use http::{api_router, BrokersCtx};
pub use service::BrokersService;

/// Linked librdkafka version, e.g. `2.12.1 (0x020c01ff)`.
pub fn librdkafka_version() -> String {
    let (n, v) = rdkafka::util::get_rdkafka_version();
    format!("{v} (0x{n:08x})")
}
