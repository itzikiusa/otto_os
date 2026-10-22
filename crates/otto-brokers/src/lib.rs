//! Otto Message Brokers — Kafka cluster viewer engine.
//!
//! Layers: [`types`] (domain + DTOs) → [`decode`] / [`schema_registry`] /
//! [`metrics`] (rendering) → [`kafka`] (rdkafka driver) → `service` (façade +
//! client pool) → `http` (`api_router::<S: BrokersCtx>()`).

pub mod decode;
pub mod http;
pub mod kafka;
pub mod metrics;
pub mod proxy;
mod replay;
pub mod schema_registry;
pub mod service;
pub mod types;

pub use http::{api_router, BrokersCtx};
pub use service::BrokersService;
