//! SSH-tunnelled Kafka access.
//!
//! librdkafka has no SOCKS support and no way to override the *advertised*
//! broker addresses it learns from `Metadata`/`FindCoordinator`. So to reach a
//! private cluster (e.g. AWS MSK in a VPC) through a bastion we run a small,
//! Kafka-aware reverse proxy:
//!
//! - One `ssh -D` SOCKS5 tunnel per cluster (via [`otto_ssh::SshTunnel`]).
//! - librdkafka talks **plaintext to a local listener**; the proxy dials each
//!   real broker through SOCKS (remote DNS) and, when the cluster uses TLS,
//!   originates TLS to the broker itself with the correct SNI.
//! - The proxy rewrites the broker host/port in `Metadata` and
//!   `FindCoordinator` responses to `127.0.0.1:<local>`, spinning up a local
//!   listener per broker on demand, and clamps `ApiVersions` so those two
//!   responses stay on the simple non-flexible wire format.
//!
//! [`protocol`] is the pure (sync, unit-tested) wire-format logic; [`runtime`]
//! is the async tunnel + listener plumbing built on top.

pub mod protocol;
pub mod runtime;

pub use runtime::BrokerTunnel;
