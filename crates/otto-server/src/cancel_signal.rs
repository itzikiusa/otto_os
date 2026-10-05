//! Re-export: [`CancelSignal`] lives in `otto_core::cancel_signal` so leaf
//! crates (the product watcher in `otto-product`) can share it.

pub use otto_core::cancel_signal::CancelSignal;
