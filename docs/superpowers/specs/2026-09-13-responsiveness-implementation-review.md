# Responsiveness implementation review

Independent compliance and correctness/performance passes reviewed the actual callers and failure paths in each lane. All source findings are resolved; combined test and CI gates remain separate delivery requirements.

| Lane | Review outcome | Evidence and resolved findings |
|---|---|---|
| Shared scripts, picker and Git | Approved | API sends snapshot their environment before asynchronous pre-scripts. Nested dialogs and the virtual grid retain keyboard ownership. Unknown Git status cannot enable force in either UI or the fresh server check. Browser cancellation/deadline and unknown-status cases pass. |
| Vault | Approved | Delta saves and compact directory caches preserve structural link resolution. Publication-scoped repair guards cover errors/cancellation across note writes, artifact writes and scan additions/removals. Per-node ownership rejects older reopening results. Final crate suite: 78 passed, including portable timestamp precision; UI 14 passed; strict Clippy passed. |
| Agents and Workflows | Approved | Removed an old visibility listener bypassing the scheduler; unchanged mounted sources reacquire leases after identity reset. Conversation accounting is incremental and monotonic, so release is constant work and parent reload retains expanded-child charges. Same-generation checkpoint pages become visible during ongoing updates without replacing newer rows. Final focused review regressions: 19 passed. Actual HTTP/WebSocket fixtures also pass for cursor scope, cache authorization and live-tail startup; workflow routes pass current/revoked Viewer and encoded detail/version checks. |
| Connections, API history and SFTP | Approved | Real driver/tunnel call sites initialize per key; retired requests cannot acquire fresh resources and detached close owns its old resources. History projections preserve full replay details and guard late selection. Exact-file SFTP probes retain bounded output, elapsed time and cleanup. Independent quality pass found no additional defects. |
| Final installer | Approved | Build-only receipts bind clean source and signed artifacts; queueing reports queued, and installation records success only after hashes, signatures, process identity, health and embedded UI agree. SKIP_UI is rejected to prevent stale frontend receipts. Six mocked test groups pass; final macOS installation verification is still required. |

A suspected checkpoint 404 defect was explicitly rejected: the shared SQL error mapper already maps RowNotFound to NotFound, and an executable regression passed without changing production behavior.

Accepted implementation limits remain explicit: History resolves at most 4 × requested-limit candidates per window, which can exceed returned matches; checkpoint cursors use ordinary SQLite rowid plus reset generation; unchanged transcript GETs reuse versioned snapshots, while cold/changed files still fold completely and tail snapshots are not reused without a proven consumed stamp. Inactive conversation byte charging is conservative and may evict earlier after repeated reloads; active views and drafts are preserved.

The implementation report records the combined gate results. These reviews do not claim measured production latency, actual remote DB/SSH benchmarks or completed deployment.

The checkpoint retry fixture now uses the real migrated database instead of an obsolete hand-built schema. Independent review confirmed the production execution path and retry/no-repeat assertions are unchanged; all three formerly failing tests pass.
