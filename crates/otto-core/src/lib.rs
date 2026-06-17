//! otto-core — domain types, identifiers, errors, events and API DTOs.
//!
//! This crate is the contract of record for the Otto workspace. It has no I/O
//! and no async; every other crate depends on it and codes against these types.

pub mod api;
pub mod auth;
pub mod domain;
pub mod error;
pub mod event;
pub mod hooks;
pub mod id;
pub mod provider;
pub mod secrets;
pub mod workflows;

pub use error::{Error, Result};
pub use id::{new_id, Id};
