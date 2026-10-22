//! Local, explicitly opted-in application telemetry — the instrumentation
//! leaf. Library code wraps phases in [`context::measure`] /
//! [`context::measure_result`]; spans go to whatever [`SpanSink`] the
//! enclosing [`context::scope`] was given (the daemon's
//! `otto_telemetry_collector::TelemetryService`). No ClickHouse, no
//! collector, no otto-usage dependency here.
pub mod context;
mod types;
pub use types::*;

/// Where a scope's spans are recorded. Implemented by the collector crate's
/// `TelemetryService`; a scope with a disabled sink records nothing.
pub trait SpanSink: Send + Sync {
    /// Is recording on? Checked before a span is even built.
    fn enabled(&self) -> bool;
    /// Buffer one finished span (dropped silently when full or disabled).
    fn record(&self, span: SpanRecord);
}
