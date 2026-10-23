//! API-client secret handling now lives in [`otto_apiclient::secrets`] (the
//! REST execute engine's crate). This re-export keeps `crate::api_secrets`
//! paths (state-archive's `secret_members`) compiling; there is one module and
//! one `request_guard` lock map.

pub use otto_apiclient::secrets::*;
