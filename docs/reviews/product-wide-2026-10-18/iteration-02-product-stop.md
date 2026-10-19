# Product conversational Stop — closing UX finding07-05

The client-only Stop waiting action cannot stop a provider turn. Root is adding
exact-turn cancellation on the existing managed-session lifecycle; Ronaldo owns
the UI/store integration. No new sessions or blanket permission restrictions.

Design: optional fresh turn_id on Send; Editor-authorized /stop accepts the same
ID, signals only that active conversation generation and waits for its handler
to drop the admission guard after cleanup/publication. Stale IDs cannot cancel
replacement turns. A not-yet-admitted send returns stopped:false, which must not
be presented as cancellation. Existing accepted user messages remain; completed
writes are not rolled back. Cancellation during startup uses the existing tracked
startup drain, and story deletion continues to cancel all its studio turns.

Evidence so far: new registry tests first failed to compile because the
cancellation API did not exist (/tmp/otto-product-stop-red.log). After
implementation, five studio_turn tests passed (/tmp/otto-product-stop-green.log).
Server --tests check passed (/tmp/otto-product-stop-server-check.log). Additional
real-PTY host cleanup and owner/root/viewer/outsider handler tests are pending in
/tmp/otto-product-stop-server-tests.log. UI and mounted-browser proof remain
pending. This report does not mark07-05 closed yet.

## Executed follow-up

- Real-PTY host test `product_studio_stop_and_story_delete_drain_the_owned_session`
  passed for both explicit turn cancellation and story-deletion cancellation
  (0.57s, /tmp/otto-product-stop-server-tests.log). It observes ownership of the
  real session lease before stopping, verifies no live session remains and the
  lease is available before return. It does not claim every provider timing.
- Both Stop handlers preserve Editor admission: owner and root succeed; Viewer
  and outsider fail. `product_stop_preserves_editor_boundary_and_root_access`
  passed (0.23s, /tmp/otto-product-stop-auth-test.log).
- A further regression exposed HTTP-disconnect ownership loss: dropping the
  caller dropped the turn before cleanup. `/tmp/otto-product-stop-disconnect-red.log`
  records the failing assertion. Send now uses a managed Tokio task independent
  of its response lifetime, so a disconnected caller cannot release admission
  early. All six registry tests passed, including disconnect/Stop/drain/retry
  (/tmp/otto-product-stop-disconnect-green.log). These are in-process tasks, not
  additional visible Otto sessions.
- Swarm state tests exceeded the per-file LOC cap after the reservation changes.
  The complete unchanged test module moved to `src/swarm/tests.rs`; all32 tests
  passed in16.35s (/tmp/otto-state-swarm-extracted-tests.log). No baseline raised.
  LOC checked including untracked Rust files: server130677/130939, no violations.

Product/server all-target clippy passed with `-D warnings`
(/tmp/otto-product-stop-clippy.log).

## UI / Store Integration & Mounted Browser Closure (`07-05` — FIXED)

- **Store API (`ui/src/lib/stores/product.svelte.ts:10, 1118-1137, 1154-1173`)**:
  - `sendRefinementMessage` and `sendDiscoveryMessage` accept an optional `turnId?: string` and include `turn_id` in the POST payload.
  - Added `stopRefinementTurn(tid, turnId)` (`POST /product/refinement-threads/${tid}/stop`) and `stopDiscoveryTurn(cid, turnId)` (`POST /product/discovery-chats/${cid}/stop`) returning `ProductStopTurnResp` (`{ stopped: boolean }`).
- **Chat Components (`ui/src/modules/product/DiscoveryChat.svelte` & `RefineChat.svelte`)**:
  - Generate a fresh UUID `turnId = newTurnId()` per `Send`, track `activeTurnId`, `inFlightTurnId`, `stopping`, `stopNotice`, and `stopRequestedTurns`.
  - Render a primary `Stop` button (`Stopping…` while awaiting `/stop` ack, disabling new sends) alongside the `Stop waiting` network fallback inside `.thinking-row`.
  - When `res.stopped === true`, clear `sending`/`stopping`/`activeTurnId`, surface `'Turn stopped.'`, and reload the conversation (`loadChat`/`loadThread` with `preserveMessages` fallback so the accepted user bubble stays visible).
  - When `res.stopped === false` (not yet admitted / already finished / mismatch), never claim cancellation: surface `'Couldn’t confirm stop — the turn was not active on the server yet (or already finished).'` and keep `sending` true while the turn request remains in flight.
  - Suppress rollback/draft restoration in `send()` `catch` whenever `stopRequestedTurns.has(turnId)` so an accepted user message is never re-inserted into the composer or `drafts` map on late send failure or conversation switch.
- **Verification Evidence**:
  - **Unit (`ui/unit/productChatOwnership.test.ts`)**: `10/10 PASS` (`node --test unit/productChatOwnership.test.ts`), covering `DiscoveryChat` and `RefineChat` exact `turn_id` plumbing, `Stopping…` hold until `/stop` ack, `{ stopped: true }` reload + preserved user bubble with zero draft rollback, and `{ stopped: false }` truthfulness + cross-conversation switch safety.
  - **Static gates**: `node scripts/ui-guards.mjs` (`ok, 1086 files, 0 regressions`) and `npx svelte-check --tsconfig ./tsconfig.app.json` (`0 errors, 0 warnings`).
  - **Mounted Browser (`ui/e2e/desktop-product-vault-skills-closure.spec.ts` + `ui/e2e/desktop-product-chat-ownership.spec.ts` on isolated Slot 4)**: `7/7 PASS` (`9.7s`), verifying phone `light` + `dark` Discovery Chat `Send` -> `Stop` (`capturedStopTurnId === capturedSendTurnId`, disabled `Stopping…` state during `/stop`, `'Turn stopped.'` after `{ stopped: true }` reload, accepted user bubble preserved, composer empty, and `Stop waiting` fallback present) plus conversation-switch draft ownership.
