# Iteration 06 — root L21–L24: export contract audit

Read ClickHouse export_to_writer, export_native and HTTP format selection against
both API and feature documentation. Found stale claims that native transport and
HTTP JSON arrays buffer the whole result. Corrected the feature matrix, export
walkthrough and limitations, plus the API explanation. Corrected the browser
export explanation to distinguish streamed rerun from displayed-grid download.
These are documentation corrections; no endpoint or TypeScript shape changed.

| Lens | Score | Evidence |
|---|---:|---|
| L21 routing | 9.6 | Export remains a separate execution path with native find/aggregate restrictions for Mongo. |
| L22 dialog | 9.8 | No layout change in this pass. |
| L23 contracts | 9.7 | Streaming descriptions now match both CH transports; native JSON/Dynamic limitation remains explicit. |
| L24 release | 9.1 | CH 11 live cases passed; combined Rust gate and deployment pending. |
