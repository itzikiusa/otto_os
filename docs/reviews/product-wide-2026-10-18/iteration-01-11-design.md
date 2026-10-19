# Iteration 01-11 — Design / Canvas UI integration

UI integration for E11’s Rust Design and Canvas fixes. No Rust file was edited. No Cargo run. `docs/contracts/api.md` and `ui/src/lib/events.svelte.ts` were left as they are: the contract file is already dirty (E08 owns the workspace paragraph), and E20 owns the activity lifecycle hooks in the events store. The canvas document bus still delivers every push. The canvas store drops a stale one.

This is not a 9.8 score. The browser was not opened.

## What the server now does (read only)

Stop loads the scene’s current portable document, writes that source back over the agent scratch file, and returns it with the note `Stopped — your saved changes were preserved.` (`crates/otto-canvas/src/assist.rs`, `stopped_document`). A turn that conflicts with a newer saved source keeps that source and notes `Kept your manual edits saved during the turn; the agent's version was NOT applied.`

A live file poll can still broadcast an intermediate agent source after the user has saved document B. If the UI applied that push once `dirty` was false, the next autosave would store A again.

Design import rehydrates canvas images into the durable `otto-canvas` JSON (`imported_canvas_images_survive_source_deletion_and_fork` in `crates/otto-design/src/import.rs`). New imports do not need a client marker resolver. Historical documents can still contain `otto-canvas-file:` refs. The live Canvas editor still resolves those refs for its own files (`ui/src/modules/canvas/canvasFileRefs.ts`). That path was not changed.

`PinnedRef` poster fields are final: `poster_pinned?: boolean` and `poster_blob?: string | null`. Older manifests omit them and still parse. A new embed pin sets `poster_pinned` even when `poster_blob` is null. A historical pin (`poster_pinned` omitted or false) renders a stand-in and does not take a newer thumbnail (`crates/otto-design/src/site/http.rs`). The live editor, which is not a publish replay, still uses the current thumbnail.

## UI changes

Canvas store (`ui/src/lib/stores/canvas.svelte.ts`):

- A successful PUT records the saved source and increments a save epoch (`noteLandedSave`).
- `assist()` captures that epoch and clears the pin, so a new turn can preview again.
- While the epoch has moved, or after the turn pins the saved source, `ingestDoc` / `ingestPush` accept only that source. An unsaved dirty draft still wins, because the dirty check returns first.
- `finishAssistTurn()` runs when the Ask Otto request ends and keeps the pin until the next `assist()`. A later user save updates the pin to the newer source.
- `assistKeptSaved()` is true when this client saved during the turn, or when the note starts with `Stopped` or contains the server’s kept / preserved wording.

Ask Otto (`MermaidCanvas.svelte`, `D2Canvas.svelte`, `ExcalidrawCanvas.svelte`):

- `assistKeptSaved(res.note)` is read before `finishAssistTurn()`.
- The returned source is passed through `ingestDoc`. A mismatch or a dirty draft is not applied.
- A kept or stopped turn toasts “Saved changes kept” and does not call `toastAgentEdit` or “Drawn on canvas”. Undo restores the newest `agent` version, which is the pre-turn board, so that action is not offered.
- `finishAssistTurn()` runs in `finally`, including a failed request.

Stop copy (`ConversationPanel.svelte`): “Stopping Ask Otto” / “Saved changes stay on the board. A drawing that was only previewed is dropped.”

Design Hall (`ui/src/modules/product/design/DesignBoard.svelte`): `initialData` and `loadScene` pass Excalidraw only files whose `dataURL` starts with `data:`. Marker URLs and files with no data URL are omitted. No resolver was added.

`DesignPinnedRef` in `ui/src/lib/api/types.ts` keeps optional `poster_blob?: string | null` and `poster_pinned?: boolean`. Those names are final. Omitted fields stay valid on old JSON. `poster_pinned` false or omitted is a historical publish and shows a stand-in. `poster_pinned` true with `poster_blob` null pins that absence, also a stand-in. `poster_pinned` true with a blob is that captured poster. `settings_patch` and the gRPC DTOs were not rewritten.

## Contract left stale

`docs/contracts/api.md` row 107a (currently around line 3809) still says assist/stop resets the backing file and re-broadcasts the pre-turn document. The Rust stop path returns and broadcasts the current stored document. The sentence was not edited here.

## Verification

`cd ui && node --test unit/canvasRecovery.test.ts unit/canvasLivePush.test.ts unit/designBoardFiles.test.ts`

21 passed, 0 failed. Coverage includes: a save of B during a turn rejects a later push of A and still rejects it after the turn; a preview still applies when nothing was saved; a stop note is treated as kept; Mermaid, D2, and Excalidraw do not offer undo on that note; Design Board initial data and reload hand Excalidraw only the `data:` file.

`npm run check` was not run. The running app was not clicked through. Cargo was not run.

## Allocated fix pass — source-frozen handoff (2026-10-18)

The later allocation authorized production fixes, superseding the initial read-only boundary above. No sessions were spawned, no Cargo/build command was run, and no commit, staging, stash, reset, push or deployment was performed. All edits remain in the shared worktree. Cargo was never acquired by E11 and is not held.

Exact E11 production/test files:

- `crates/otto-state/src/canvas.rs`: `portable_scene` reads the scene and referenced image bytes in one SQLite read snapshot, returning self-contained content; missing images fail explicitly.
- `crates/otto-design/src/import.rs`: imports materialize Canvas image bytes, mark portable imports and upgrade unchanged legacy current heads on the next import; regression imports, forks, deletes the source (including Canvas GC), and checks both copies retain image bytes.
- `crates/otto-canvas/src/assist.rs`: cancellation restores/returns/broadcasts freshly saved content and its current format instead of the pre-turn document.
- `crates/otto-canvas/src/assist_regressions.rs`: regression saves a different format during the turn and checks the cancellation result and scratch file preserve it.
- `crates/otto-design/src/store.rs`: pruning rechecks head/approval/link/signal/publish protections inside an immediate transaction; returns actual deleted IDs and bytes. Approval atomically verifies the target version; link/signal/publish writers verify referenced versions under the same write transaction. Published poster hashes are blob GC roots. Tests pause a stale prune plan while adding every protection, reject references to a deleted candidate, and race approval against prune.
- `crates/otto-design/src/service.rs`: uses actual deletion results and canonicalizes explicit version pins to durable version IDs.
- `crates/otto-design/src/blobs.rs`: metadata and bounded-read helpers; regression checks oversize reads fail and exact-boundary reads succeed.
- `crates/otto-design/src/site/export.rs`: additive poster pin fields, move asset buffers into archives, reject duplicate output names, count ZIP overhead against the cap; collision and boundary regressions.
- `crates/otto-design/src/site/http.rs`: retain captured posters through publish insertion, replay only captured posters (including captured absence), avoid resolving formerly missing references to newly available content, enforce aggregate unique asset budgets before blob reads. Regressions check thumbnail replacement/GC, captured absence, budget rejection and actual bounded blob reads.
- `crates/otto-design/src/site/validate.rs`: IDs/slugs share a route namespace; reserve home `index`; regression covers cross-ID/slug and output collisions.
- `docs/contracts/api.md`: targeted Canvas cancellation and Design publish/export contract updates only; preserve other owners' edits.
- This report.

No change was necessary in `otto-design-assist`. No migration was added; E14 was notified. Poster pins use existing JSON storage: `poster_pinned?: boolean`, `poster_blob?: string|null`. Pele owns frontend functional integration and shared TypeScript fields (the fields are present on source inspection). E20 is authorized to replace the Design preference GET-merge-PATCH writer with an exact-key `settings_patch`; E11 did not edit UI files.

### Actual verification

- PASS: `rustfmt --edition 2021` on the ten owned Rust files, repeated for the final site test edits.
- PASS: scoped `git diff --check` after final source changes.
- PASS: isolated Python in-memory SQLite execution of the actual current prune DELETE and approval UPDATE strings, with the real Design migration schema and prerequisite fixture table: preserved head, approval, link, signal, direct publish and JSON publish-pin roots; deleted only the unprotected candidate; stale approval affected zero rows. An earlier fixture attempt failed because `user_feature_grants` was absent; the corrected fixture passed. This is SQL evidence, not Rust test execution.
- NOT RUN: Rust compilation, tests, clippy, UI checks, browser/PTY scenarios or memory benchmarks. Cargo slot requested from root; no grant received at source freeze. Regression tests are authored, not claimed passing.

Pending shared-slot commands:

```sh
cargo test -p otto-design -p otto-canvas --lib
cargo test -p otto-state --lib canvas::
cargo clippy -p otto-design -p otto-canvas -p otto-design-assist --all-targets -- -D warnings
```

Include affected consumers (`otto-design-assist`, `otto-server`) and normal integration gates under the root's coordinated run. Pele should check the Canvas stop result's current format and protection against a newer local draft/save arriving after the server snapshot, plus named UI scenarios and shared type checks.

### Remaining limits and honest status

All six findings have source repairs and regression coverage written. Independent source re-review checked the opposite prune/reference race ordering, missing/captured-absent posters, publish GC fencing, duplicate route namespaces, budget boundaries and current-format cancellation. Publication pin resolution was tightened to match both URI and role; the thumbnail regression uses returned publish IDs rather than timestamp ordering.

Legacy imported *historical* versions and pre-existing forks containing Canvas-only markers are not rewritten. Current imported heads upgrade while source images remain available; already deleted source bytes cannot be reconstructed. Older published manifests did not capture poster hashes and now show a stand-in rather than silently substituting today's thumbnail. These compatibility limits require explicit release/recovery handling by the coordinating owners. No claim is made that all past data can be repaired.

The final user clarification permits completing existing allocations and prohibits new visible sessions; E11 opens none. Source is ready for root/Ronaldo/Pele validation and cross-review. Verification remains pending, so this report does **not** raise the original 7.4 scope score to 9.8 or certify merge readiness.

## UI follow-up — final poster fields and current-format stop

No Rust file was edited.

`DesignPinnedRef.poster_pinned` and `poster_blob` stay optional. Omitted JSON is a historical publish. `pinnedEmbedPoster` / `resolveEmbed(uri, pin)` return a stand-in for a historical pin and for a pin that recorded no poster, and they do not call the artifact’s current thumbnail. The live editor still resolves the current thumbnail when no pin is passed. Publish history names those rows as poster stand-ins.

Mermaid, D2, and Excalidraw build the applied doc from `res.format` and that format’s source field. A note that starts with `Stopped` does not ingest the response body. `canvas.reloadAfterStop` waits for the scene’s in-flight save, reads the stored scene, and drops that read if a newer save landed during it. While the reload is in progress, live pushes are ignored. After it, only the reloaded source (or the save that won) is applied.

`node --test unit/canvasRecovery.test.ts unit/canvasLivePush.test.ts unit/siteResolution.test.ts` — 27 passed. `npm run check` was not run. The app was not clicked through. Cargo was not run.
