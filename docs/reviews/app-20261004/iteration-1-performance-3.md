# Iteration 1 — performance partition 3

**Verdict: Approve with fixes.** Blocker 0 · major 1 · minor 2 · nit 0. Current-app audit at `a16f4c71`, not a branch-diff review. Scope: Vault, Canvas, Design Hall, Product, Browser, Snip; desktop responsiveness, local I/O, SQLite work, and retained memory. No visual findings.

**Scope sized:** hub-note fan-in of 1,000–10,000 links; vaults of 1k/10k/100k notes with five links per note; Product histories of 30/300 distinct 500 KiB JSON source versions. These are explicit synthetic sizes, not claims about the user's actual data. Parent owns concurrent-agent CPU/RAM and running-app measurements. This report does not certify runtime behavior under load.

## Ranked findings

### [major] Opening a hub note reads every backlink source before showing the first page — `crates/otto-vault/src/engine.rs:1740`

**What:** `backlinks_hashed` returns the complete incoming-link list, and each context-cache miss awaits a full source-file read serially (`engine.rs:1757`). The UI initially shows only 100 entries (`ui/src/modules/vault/RightPanel.svelte:15`), but server work is not paginated. The client also waits for the complete response (`ui/src/modules/vault/vault.svelte.ts:794`).

**Cost:** N = distinct incoming source/kind rows for the selected note; B = average source body bytes. A cold hub open does one backlink query plus N serial file reads and O(N·B) scanning, returning O(N) contexts before the first 100 can display. At 1,000 sources averaging 64 KiB, that is approximately 62.5 MiB read; at 10,000, 625 MiB. An average 1 ms file-read/scheduling latency would alone contribute 1/10 seconds respectively; this is a cost illustration, not a measured latency. Warm identical opens benefit from the cache, but new targets and edited source hashes miss it. The 20,000-entry cache clears wholesale on overflow, reintroducing the cold cost.

**Evidence: confirmed by tracing.** `crates/otto-vault/src/store.rs:754–760` has an indexed destination predicate but no limit; the loop awaits each read. Source links are normally extracted only from notes up to the 4 MiB indexing limit (`prepare.rs:184–189`), so the example's 64 KiB sources are within normal supported input. UI paging does not bound server work. No real-vault timing was performed.

**Fix/payoff:** Add keyset pagination of incoming `(src_path, kind)` rows and return total count separately; fetch the first 100 on open and subsequent pages on demand. Store bounded link context during note indexing, or resolve context for the requested page with a small bounded worker pool. Initial reads become at most 100 rather than N (10,000 → 100, 100× fewer file reads); indexing contexts removes the read fan-out altogether. Use bounded LRU eviction instead of clearing the entire context cache.

**Regression verification:** A synthetic 10k-source hub must return the first page without opening all source files; exercise next-page ordering, duplicate kinds, edited-source invalidation, rename/delete behavior, and cancellation during navigation. Preserve total backlink count in the UI.

### [minor] Slim version lists still parse all historical JSON bodies — `crates/otto-state/src/product.rs:842`

**What:** The response omits `body_md` and reduces `raw_json` to its version number, but `SLIM_RAW_JSON` (`product.rs:389`) invokes `json_valid`, `json_type`, and `json_extract` on every complete saved JSON blob. There is no list limit. The comment documents Confluence bodies of 500 KiB; Rewrite uses this list initially and during generation polling (`ui/src/modules/product/RewriteTab.svelte:61,80,130`). Polling is already bounded to two minutes, pauses when hidden, and normally uses a 15-second event safety interval; it is not unconditional idle 3-second polling.

**Cost:** V = saved versions of a story; B = average raw JSON size. Each list reads/parses O(V·B) bytes even though its response is small: 30 × 500 KiB ≈ 14.6 MiB, 300 × 500 KiB ≈ 146.5 MiB per request. This is a warm history/navigation/generation path. Repeated windows or agent readers multiply the work. Ranked minor because measured query CPU is modest at these sizes and this is not an always-running hot loop.

**Evidence: confirmed SQL shape and synthetic query timing.** Python SQLite 3.53.4, in-memory database, distinct JSON version number per row, five repeated query timings: 30 versions median 3.175 ms; 300 versions 31.835 ms. Reading a scalar version from the same wide table was 0.163/4.058 ms; reading a separate thin summary was 0.006/0.057 ms. This isolates query shape, not SQLx, disk cache, HTTP, or app latency. An earlier identical-blob run was discarded for comparison because JSON parser reuse underestimates distinct-history cost.

**Fix/payoff:** Persist the upstream numeric source version at insertion in a scalar metadata column (append-only migration/backfill), and construct the compatible slim `raw_json` output from that scalar. Page the history endpoint; polling can request a latest/version-since summary instead of all history. Normal list cost becomes O(page size), independent of saved body bytes; source JSON is parsed once at import rather than on each read. A thin summary or covering metadata projection avoids touching overflow-heavy rows.

**Regression verification:** Preserve numeric, missing, malformed, and nonnumeric version behavior; verify history pagination and newest-source/rewrite selection. Seed distinct large JSON bodies and assert the listing path does not invoke JSON extraction on historical blobs. Repeat measurements through the actual endpoint.

### [minor] The Vault status cache still recounts notes and links twice per idle poll — `crates/otto-vault/src/store.rs:134`

**What:** `VAULT_COLS` contains correlated note/link `COUNT(*)` subqueries. Every `get_vault` uses that projection (`store.rs:155`), including `get_scoped`. `VaultEngine::status` first calls `get_scoped` (`engine.rs:837`) and then `status_with`, which calls `get_vault` again (`store.rs:257`). The generation cache covers unresolved links/tags/attachments only; note and link counts bypass it. Other scoped calls (directory, note, search, graph, backlinks) each incur these counts as well.

**Cost:** N = indexed notes, E = indexed links. An unchanged visible Vault status poll every five seconds (`ui/src/modules/vault/vault.svelte.ts:315–323`) scans approximately 2N + 2E index entries before returning cached status data. Indexes make these covering range scans, not constant-time counts. At 100k notes/500k links, this is approximately 1.2 million visited entries per poll; each scoped read adds N + E. Concurrency multiplies SQLite work, but measured local cost warrants minor severity.

**Evidence: confirmed trace + query plan.** Migration `0105_vault_docs.sql` provides the note primary key and link source/destination indexes. Synthetic Python SQLite 3.53.4, same correlated count shape and indexes, two reads per sample, median of nine: 1k notes/5k links 0.121 ms; 10k/50k 1.161 ms; 100k/500k 13.528 ms. `EXPLAIN QUERY PLAN` showed the vault PK seek plus two correlated covering-index searches. These are in-memory query-shape measurements, not real-app timings.

**Fix/payoff:** Split metadata/existence lookup from summary-with-counts; route `get_scoped` through the metadata lookup. Include note/link totals in the generation-keyed status cache and fetch vault metadata once in `status`. Steady-state polling becomes O(1) metadata lookup, with count work paid only after an index generation changes. Directory/search/asset requests stop paying whole-vault aggregate cost.

**Regression verification:** Instrument all five status aggregates rather than only `status_count_reads`; unchanged polls must not execute note/link counts. Verify all mutations and scan completion invalidate counts, including concurrent writes and unregister. Ensure list responses still expose accurate totals.

## Coverage and existing bounds

- **Vault:** Traced scan preparation/batching, metadata/status, lazy directory tree, note read, backlinks, graph construction/cache, graph worker, and UI refresh. Tree rendering is virtualized; graph assembly is offloaded; layout uses a worker/Barnes–Hut and transferred buffers. These existing protections are not findings. Full graph output still intentionally scales with the requested whole graph; no unsupported claim that every graph read is constant-time.
- **Canvas:** Inspected scene list/item routes, repository summaries, version handling, scene list UI, file cache. Request bodies cap at 25 MiB; large JSON parsing is offloaded; asset-reference responses avoid repeatedly inlining image blobs; file cache has a 64 MiB target and per-owner release. Scene lists remain unpaginated, but no current scene counts were available to rank the user-visible cost. Rendering every listed scene and inline thumbnail sizes are follow-up sizing work.
- **Design Hall:** Inspected library loading/event patching, search/store paths, detail/content, graph/link lookups, lobby. Library asks for at most 500 hits; known artifact changes patch at most 10 IDs per batch; reference counts are batched; detail text truncates at 256 KiB; thumbnail upload caps at 2 MiB. Did not report bounded per-card map lookups or small fixed serial detail reads as scaling failures.
- **Product:** Traced global story listing, selected story/detail and history versions, Rewrite polling, source-search cancellation. Global story list is unpaginated; realistic story totals and DOM costs need a later UI load run. Main finding is historical-body work despite the slim wire response.
- **Browser:** Inspected reader page acquisition/cache, byte and match caps, and live runtime/session bounds. Pages cap at 2 MiB, page cache at 32 entries/32 MiB, selector results at 500 matches/1 MiB, and live sessions/processes/viewers have limits. No new confirmed finding within sampled paths. Cancellation cleanup of page single-flight gates was not proven through an actual cancelling call chain and is not asserted as a leak.
- **Snip:** Inspected upload/image serving/listing and editor redraw/undo. Encoding is delegated; repaint is frame-coalesced; stable canvas layers avoid full annotation redraw during drags; undo is capped at 100 snapshots; unchanged annotated output skips upload. The list caps its response after scanning all sidecars; realistic retained snip count remains unmeasured. Compressed byte validation checks positive PNG dimensions without a pixel-count cap; whether a large imported image exceeds practical editor/GPU budgets needs a separate bounded fixture run (not asserted here as an observed freeze).

## Verification limits and next wave

Read-only source audit plus isolated in-memory SQLite query-shape experiments only. No builds, tests, daemon requests, servers, repository source edits, commits, or user-data reads/writes. The skill hotspot script was run against `a16f4c71` and correctly returned no changed files; manual full-current-path sampling supplied this audit. No external web research was needed for local code observations.

This is not exhaustive across site/3D studios, Product agent orchestration/media/watchers, Browser CDP streams/overlays, vault OKF/recovery, or Canvas export/render engines. Next wave should run the first-page backlink fixture, actual version-list endpoint timings, aggregate-count instrumentation, and parent-owned CPU/RSS measurements at N concurrent agents with memory sampled over time. Do not use this report alone to claim those runtime gates passed.
