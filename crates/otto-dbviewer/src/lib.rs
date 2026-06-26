//! otto-dbviewer — native data-access layer for the DB Explorer.
//!
//! A TablePlus/Navicat-class viewer for MySQL, Redis, MongoDB and ClickHouse
//! built on top of the existing `otto-connections` profiles. This crate owns:
//!
//! - [`types`] — the engine-agnostic contract (config, schema tree, results,
//!   autocomplete, capabilities).
//! - [`driver::Driver`] — the per-engine trait, implemented in [`drivers`].
//! - [`service::DbViewerService`] — resolves a profile + secret, opens any SSH
//!   tunnel, dispatches to the driver, records history.
//! - [`http`] — the REST router (`api_router`) + [`http::DbViewerCtx`].

pub mod complete;
pub mod config;
pub mod driver;
pub mod drivers;
pub mod export;
pub mod http;
pub mod import;
pub mod nl;
pub mod registry;
pub mod service;
pub mod tls;
pub mod types;

pub use driver::Driver;
pub use export::{ExportCounts, ExportFormat};
pub use http::{api_router, DbViewerCtx};
pub use import::{build_insert_statements, parse_rows, sql_string_literal, ImportFormat, ParsedTable};
pub use nl::{
    drive_nl_to_sql, extract_sql, DraftContext, FailedAttempt, NlToSqlOutcome, SqlDrafter,
    SqlValidator,
};
pub use registry::Registry;
pub use service::DbViewerService;
pub use types::{
    Capabilities, Column, ColumnDef, CompletionContext, CompletionItem, CompletionKind,
    CompletionResponse, Engine, ForeignKey, GraphColumn, GraphEdge, GraphTable, IndexDef, NodeKind,
    NodePath, ObjectDetail, QueryRequest, QueryResult, QueryStats, ResolvedConfig, SchemaGraph,
    SchemaNode, SshTunnelConfig, TestResult, TlsConfig, TlsMode,
};
