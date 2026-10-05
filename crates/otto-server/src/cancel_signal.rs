//! Re-export: [`CancelSignal`] lives in `otto_core::cancel_signal` so the
//! extracted engines (otto-insights) share the one implementation.

pub use otto_core::cancel_signal::CancelSignal;
