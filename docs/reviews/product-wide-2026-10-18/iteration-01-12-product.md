# E12-product — iteration 01 engineering review

**Initial verdict: Block — 2 blockers, 4 major findings, 1 minor finding.** Initial source-review score **7.2/10**, below the 9.8 target. Allocated fixes are now implemented in source; validation remains pending. See the source-frozen handoff below. No revised 9.8 claim is made.

## Provenance and constraints

- Reviewer: E12-product. Date: 2026-10-18.
- Branch: `fix/db-parity-session-consistency`; starting and concluding HEAD: `c009da20cc113c8fb79f3bf0600442d0c6afa358` (PR111 context supplied by coordinator).
- Reviewed existing subsystem behavior, not merely changed lines. Findings may predate this PR.
- Read repository AGENTS.md and correctness-review, test-review, architecture-review, performance-review skills; applied separate lenses without manufacturing a finding for every lens.
- Initial inspection was read-only. Subsequent explicit allocation authorized the narrow implementation listed below. No Cargo, builds, tests, live product mutations, commits or git mutations were performed. Concurrent edits outside the allocation were preserved.
- Early confirmed findings were delivered using `otto_send_message` to coordinator session `01M57PYK75FC1F2AFS4C73BBF7` with reviewer ID.
- All findings below are **confirmed by concrete source trace; not runtime reproduced**. Test descriptions are proposed checks, not claimed results.

## Ranked findings

### E12-01 — Blocker: Saved draft requirements disappear from agent context

**Locations:** `crates/otto-product/src/service.rs:1096` and `:951`; `crates/otto-product/src/refine.rs:292`; `crates/otto-product/src/swarm.rs:133`.

**Input and trace:** Create a local draft, then PATCH its draft body to “Refunds require manager approval above 100.” Creation stores only a `kind="draft"` version (`service.rs:1439`); `update_draft_body` updates that revision (`:1527`). `build_agent_context` selects only suggested/source, so it emits an empty story body. Analysis, rewrite, test generation and plan generation call that builder (`run.rs:509`, `:1596`, `:1909`, `:2313`). Refinement independently selects suggested/source and falls back to the title; swarm goal selection does the same. Injection independently omits the Story section when neither kind exists. None rejects draft stories upstream.

**Actual versus expected:** The UI can display saved requirements, but the agent performing refinement or implementation never receives them. Its resulting suggested version may omit all original constraints. Expected: the saved draft participates in content selection, as it already does in discovery chat (`chat.rs:441`) and publication (`service.rs:1324`).

**Fix direction:** Centralize the existing suggested → draft → source selection policy and reuse it across these callers. Preserve explicit distinctions between an unapplied suggestion and editable source where the caller requires them; do not change the source editor to display suggestions implicitly.

**Required regression:** Create an actual draft/version in an isolated DB, save a unique requirement, capture the host prompt for analysis/refinement, inspect the inject bundle and swarm goal, and assert the requirement survives. Cover source-only and suggested-over-draft precedence too. `ui/e2e/product-refine.spec.ts:59` seeds a draft but its final assertions only check the user message; deleting all story body text from the prompt would still pass that test.

**Architecture impact:** This is demonstrated policy duplication, not a naming preference. Another revision kind currently requires changes in multiple independent selectors. One shared content-selection function gives locality across existing callers and prevents the divergence already observed.

### E12-02 — Blocker: Concurrent attachment saves defeat the advertised conflict protection

**Locations:** `crates/otto-product/src/media.rs:518-552`; `crates/otto-state/src/product_attachments.rs:175-196`; contract `docs/contracts/api.md:1742`.

**Input and trace:** Two browser editors load attachment revision T. Both send PUT `/product/attachments/{aid}/content` with `base_updated_at=T`. Both handlers load the same row and pass `check_base_updated_at` before either writes. Each then writes the same filesystem path directly and updates metadata using unconditional `WHERE id = ?`. There is no attachment ownership guard or conditional update covering the operation.

**Actual versus expected:** Both saves can return success and one user's accepted edit is lost. The API promises a 409 for stale saves. Additionally, direct truncating writes expose incomplete bytes to reads; overlapping differently sized writes can leave bytes inconsistent with whichever metadata update runs last. The lost-update finding does not depend on that secondary interleaving.

**Fix direction:** Serialize every writer for an attachment, re-read/check under ownership, stage bytes to a sibling file and atomically publish them, and keep row revision/size/hash consistent with the published bytes. Include agent-assist and delete paths in the same ownership model; a CAS applied only after overwriting the shared file does not restore lost bytes.

**Required regression:** Barrier both same-base requests before publication; assert exactly one succeeds and one conflicts, served bytes equal the winning payload, and row size/hash agree. Also verify a failed staged write preserves previous content. The pure comparison test `media.rs:894` cannot detect this race: it passes even with no ownership around writes.

### E12-03 — Major: Same-timestamp comments are repeatedly reconciled forever

**Locations:** `crates/otto-product/src/service.rs:441-447`, `:1864-1877`; `crates/otto-product/src/watcher.rs:285-286`.

**Input and trace:** Existing cursor is `T|A`. A later fetch reveals comment B with the same timestamp T. `list_new_comments` excludes A and returns only B. The watcher builds its next cursor from that filtered batch alone, storing `T|B`. On the next poll A is now unseen, so it returns A and stores `T|A`. Subsequent polls alternate indefinitely until a newer timestamp arrives.

**Actual versus expected:** Duplicate comment events, repeated reconciliation/new questions, repeated notifications and repeated provider work for unchanged input. Expected: retain the union of IDs already consumed at the current timestamp, so the third poll is empty.

**Fix direction:** Build the next cursor using both the prior cursor and successful batch. Union IDs when timestamps represent the same instant; reset the set only when the high-water timestamp advances.

**Required regression:** Exercise three or more polling rounds against a fake issue endpoint: A@T, then A+B@T, then unchanged A+B@T. Assert results [A], [B], [], and stable cursor containing both IDs. The current `service.rs:1922` test checks one cursor and one filter but never rebuilds the cursor after the filtered tie.

**Cost model:** Two comments suffice; each poll can spend one reconcile call (up to three attempts) plus a self-improvement pass. With the five-minute minimum cadence this can repeat up to twelve cycles per hour per affected story. Cost grows with elapsed time despite zero new information.

### E12-04 — Major: Reconcile failure permanently acknowledges comments before their answers are applied

**Locations:** `crates/otto-product/src/watcher.rs:285-298`, `:348-374`, `:391-438`.

**Input and trace:** An open question Q receives comment C answering it. The watcher records only a 120-character event snippet and advances its cursor before invoking the reconcile agent. All three provider attempts fail, so the error branch returns success with the advanced cursor. Next poll filters C out. Malformed output likewise consumes C; individual question-write failures are only logged after acknowledgement.

**Actual versus expected:** Q remains open indefinitely even after the provider/DB recovers. No durable pending batch retains the full answer for automatic retry. A malformed response can even result in a “reconciled” event despite no parsed result. Expected: recoverable failures preserve pending work.

**Fix direction:** Separate fetched/observed position from successfully reconciled position, or durably persist a pending batch before acknowledging ingestion. Commit successful question changes and batch completion consistently; make retry side effects idempotent. Merely moving the cursor update below the agent call without handling partial DB writes risks duplicate questions/events.

**Required regression:** First cycle fails all provider attempts; second succeeds with no new remote comments. Assert Q becomes answered once. Repeat with malformed JSON and with one failed question write, proving no false success event and no duplicate inserted questions.

### E12-05 — Major: Deleting a story leaves its discovery/refinement agents running

**Locations:** `crates/otto-server/src/modules.rs:630-673`; `crates/otto-product/src/chat.rs:259`; `crates/otto-product/src/refine.rs:399`; `crates/otto-server/src/product_host.rs:485-513`.

**Input and trace:** Start a long discovery-chat or refinement turn and DELETE its story. The deletion handler calls `stop_story_agents` (`http.rs:491`) before deleting rows. That implementation signals the Product analysis/story-run cancel registry, then kills sessions selected by metadata path `work.story_id`. Chat and refinement create metadata with top-level `story_id`, not `work.story_id`; these turns use the studio host and do not register with that cancellation registry. `SessionsRepo::list_live_by_meta` (`crates/otto-state/src/sessions.rs:981`) uses the exact JSON path, so the top-level ID does not match.

**Actual versus expected:** DELETE succeeds while these agents continue running/spending against a now-deleted story, and eventual message/version writes fail after the work. Expected: deletion cancels/drains all active turns belonging to the story before cleanup. This is a lifecycle bug and does not require restricting privileged users.

**Fix direction:** Give studio turns the same story-owned cancellation lifecycle and publish session identity when created, before the long turn. Drain active owners during deletion and bar new admissions while deleting. Matching both metadata layouts is a useful migration measure but alone leaves startup/admission races.

**Required regression:** Use a controllable host that blocks during a chat/refinement turn; delete the story and assert cancellation observed, provider work stopped, no late message/version publication, and no fresh session created afterward. Cover deletion arriving during startup. Offline canned-reply E2Es cannot establish process cancellation.

### E12-06 — Major: Jira comment pagination silently loses a backlog above the cap

**Locations:** `crates/otto-issues/src/jira.rs:1003-1067`; downstream `crates/otto-product/src/watcher.rs:285`.

**Input and trace:** A watched Jira issue has cursor T, then accumulates 2,001 comments after T during an outage. The incremental client requests newest-first pages of 100 and stops after 20 pages. It returns the newest 2,000 as a successful complete vector without a truncation indicator. The watcher stores the newest timestamp from that vector. The oldest unseen comment is now older than the cursor and will never be requested as new.

**Actual versus expected:** At least one real reply is silently skipped. Expected: either continue the backlog safely or visibly report an incomplete fetch without acknowledging beyond the unseen gap.

**Fix direction:** Return completion/continuation state and do not advance a high-water cursor over an unconsumed interval. An explicit error at the cap is safer than silent success until durable paged catch-up is implemented. Keep bounded resource use; do not simply remove all limits.

**Required regression:** Fake 21 pages and a saved cursor predating every comment; assert either all 2,001 unique comments are eventually consumed or a visible incomplete result leaves the cursor safe. Adjacent residual: Confluence `walk_comments` discards `get_all_pages` truncation at `confluence.rs:689`; Linear also returns a capped comment list. Those provider-specific ordering/catch-up cases need separate fixtures before claiming full parity.

### E12-07 — Minor: Retrying a lens discards its selected model

**Locations:** `crates/otto-product/src/run.rs:1262-1266`; original path `:572` and `:629`.

**Input and trace:** Start a lens with an explicit model M different from the provider default D. The initial path persists `agent.model=M` and supplies M to the host. Stop/fail the lens and Retry. The retry loads that row but explicitly passes `None` for model. The host constructs a fresh session using only that argument, so the retry uses D; the persisted agent row still identifies M. Restart recovery uses the same retry entry point.

**Actual versus expected:** The model/cost/capability changes silently on retry while attribution remains misleading. Expected: retry preserves a nonempty stored model unless the user explicitly changes it.

**Fix direction:** Pass trimmed nonempty `agent.model` into `run_agent_with_recovery`, retaining `None` only for legacy empty rows.

**Required regression:** A fake host captures retry arguments: stored M must yield Some(M); empty model must yield None. Cover orphan reaper reuse as well.

## Coverage and lens assessment

| Area | Inspection performed | Result / boundary |
|---|---|---|
| Analysis | Admission, budget/fanout guards, original lens ownership, stop/retry, status publication and restart reaper entry points | Existing exclusivity mechanism is substantive; model retry defect remains. Every parser branch was not enumerated. |
| Story content | Draft create/edit, context and inject builders, refinement body choice, discovery context, swarm goal selection | Confirmed inconsistent draft selection. |
| Studio lifecycle | Chat/refinement admission, user-message persistence, session host adapter, story deletion and metadata query | Confirmed deletion cancellation gap. |
| Attachments | Upload/type/path guards, serve, content replacement, metadata update/delete, optimistic timestamp guard and tests | Confirmed non-atomic save; comprehensive annotation and 3D semantics not audited. |
| Issue integration | Tracker provider dispatch, Jira/Confluence/Linear comment paging, watcher cursor/filter/reconcile, publication snapshots and description/update guards | Confirmed watcher/pagination defects. No live Atlassian/Linear account or network mutation. |
| Publication | Reviewed-content identity, captured publication bytes, draft rebinding, Confluence version conflict and Jira lossy-node refusal | These protections are real. Cross-system idempotency and every partial-commit branch remain unverified. |
| Tests | Cursor helper, attachment timestamp helper, turn/agent ownership tests; refinement, discovery and attachment E2E source | Existing tests cover useful local properties but miss the multi-step failures described above; no suite executed. |
| Performance | Bounded fanout, watcher concurrency/cadence, pagination caps, repeated tie processing | Concrete cost amplification in E12-03. No throughput benchmark or numeric latency claim. |
| Architecture | ProductRunHost/ProductStudioHost seams, server adapter, repeated content policy, state and filesystem save boundary | Host separation generally fits the crate direction; content policy and lifecycle ownership are the actionable shared responsibilities. |

**Scores:** correctness/recovery 6.8; test confidence from source 7.0; architecture 8.0; performance/resource behavior 7.8. Overall 7.2 is a judgment, not a measured average or target compliance. No 9.8 claim is supported while these defects remain.

## Explicit residual boundaries and next round

- This report covers E12's Product/Issues engineering slice, not the other 19 engineering or 20 design reviewers' scopes. It does not replace their reports.
- No full line-by-line review of every Jira field mapping/ADF transform, Linear mutation, extraction pipeline, design-format validator, 3D generator, annotations, mockup-assist runner, or every Product HTTP endpoint. UI was read only where it establishes tests/contracts, not visually reviewed.
- No runtime reproduction, test pass, build result, live-provider behavior, accessibility score, or production-state claim is implied.
- The E16 credential-rewrite investigation is outside E12 and was not performed.
- E12-01 through E12-07 were subsequently allocated and implemented as detailed below. Runtime validation and independent cross-review remain required.
- Preserve root/admin abilities while fixing ownership/lifecycle; none of these fixes needs a new privilege restriction.
- Available for coordinated fixes and independent cross-review in a subsequent turn.

## Source-frozen implementation handoff — 2026-10-18

Existing assignment only; no new sessions spawned. All edits remain uncommitted. Cargo was never acquired or started by E12; no Cargo process or slot is held by E12. Coordinate remaining validation through root/Ronaldo/Pele.

### Exact files and responsibilities

- `crates/otto-product/src/service.rs`: shared suggested/draft/source content selection, equal-time cursor ID union, real-persistence regressions for draft input, attachment CAS and watch acknowledgement rollback/replay.
- `crates/otto-product/src/refine.rs`, `swarm.rs`: use shared content selection. Refinement also holds story admission through publication.
- `crates/otto-product/src/run.rs`: retry supplies trimmed persisted model; empty legacy model remains provider default; focused regression.
- `crates/otto-product/src/chat.rs`, `http.rs`, `lib.rs`, new `story_lifecycle.rs`: studio admission, cancellation and deletion drain; deletion route regression and admission tests.
- `crates/otto-product/src/watcher.rs`: strict reconciliation parsing; provider/parse failures leave cursor unchanged; persist cursor, answers, questions and events in one transaction; notices only after commit. Parser regressions included.
- `crates/otto-product/src/media.rs`: stage complete immutable attachment bytes, then publish path/hash/size via database CAS; clean rejected candidate writes.
- `crates/otto-state/src/product.rs`: only child module declaration for new `product_watch.rs`; E14's `add_version` untouched. `product_watch.rs` atomically commits reconciliation and rejects stale cursors. `product_attachments.rs` adds shared content CAS, including previous path and timestamp checks.
- `crates/otto-issues/src/jira.rs`: incomplete capped pagination fails instead of returning a partial successful batch. Fake HTTP regression covers 2,001-comment backlog.
- `crates/otto-server/src/modules.rs`: only exact `stop_story_agents` function; query both top-level and nested story metadata. E13 review-retry and other owners' edits preserved.
- `crates/otto-server/src/mockup_assist.rs`: only final publication block; use shared attachment CAS. No whole-file formatting.
- `crates/otto-server/src/product_host.rs`: studio turns pass the story cancellation flag to the cancellable session wrapper.
- `crates/otto-server/src/agent_session.rs`, `agent_session_lifecycle.rs`: approved wrapper and tracked startup cleanup, plus regressions described below.

### Independent reviewer notes: session lifecycle

The old public `run_session_turn` delegates with cancellation `None`, preserving existing callers. The Product wrapper supplies its cancellation flag through existing `TurnOpts`. Explicit cancellation drains the tracked startup task before stopping the owned session and returning. Ordinary timeout behavior is unchanged.

Tracked creation awaits the existing creation helper. Tracked resume acquires the session lease before registering startup work; a dropped queued lease waiter owns nothing and cannot stop the predecessor. Once startup owns a session, it publishes the ownership slot before sending the result. If the receiver disappeared, the tracked task awaits cleanup itself. If the result was delivered, the outer guard owns cleanup after the drain. Created sessions are retired; cancelled resumed sessions are suspended and remain resumable. Story deletion holds the write guard through deletion, after admitted studio turns have drained.

New lifecycle tests: `tracked_startup_cancellation_drains_created_and_resumed_cleanup`, `queued_tracked_resume_cancellation_does_not_stop_the_owner`, and `cancellable_chat_wrapper_stops_its_resumed_turn_before_returning`. They use real isolated shell sessions. The first controls a barrier around owned startup using the shared tracker; it does not instrument the manager's internal create syscall. Central validation should examine both real create/resume startup error branches as well as successful startup cancellation.

### Actual verification and pending commands

Owned Rust files were parsed/formatted with standalone `rustfmt --edition 2021 --config skip_children=true`; shared `modules.rs` and `mockup_assist.rs` received only manual formatting of owned blocks. `git diff --check` passed. No compilation, unit test, runtime test, E2E or live-provider test has run in E12. Written tests are not claimed passing. The rollback regression deliberately supplies a mismatched story question after cursor/answer updates; this exercises rollback on validation failure without assuming a nonexistent users foreign key.

Root slot requested for `cargo test -p otto-product --lib` and `cargo test -p otto-issues --lib comment_backlog_over_cap`. Follow with state tests, `cargo check -p otto-server --lib`, the three named lifecycle regressions via `cargo test -p otto-server --lib <filter>`, and central consumer gates for the changed shared lifecycle/state APIs. Run clippy on changed packages and all targets; use the repository gate to include dependent crates. No command is running or awaiting a tool result in E12.

### Residual risks and honest closure boundary

- Previous immutable attachment revisions intentionally remain until story-directory cleanup, preserving paths already handed to readers or agents. Individual attachment deletion removes its current file only. Repeated edits therefore retain disk space; bounded reclamation is not implemented. Independent review must accept this tradeoff or allocate safe reclamation without breaking in-flight absolute-path consumers.
- SQL-failure injection beyond the validation rollback case, retry host argument capture, exact-cap Jira success and live-provider pagination remain additional useful validation. The model regression checks the argument conversion, not an end-to-end retry session.
- All original unreviewed boundaries remain. Central compilation/test results and independent source review are required before claiming defects closed or raising the score. No claim of full product completion or 9.8 readiness.
