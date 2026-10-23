# Architecture review: quality loop 2026-10-21, iteration 6 (final)

Lens: structure. That covers god files and functions that keep growing, dependencies that point the
wrong way, leaky abstractions, duplicated subsystems that should be shared, domain logic held in
otto-server, and UI modules reaching into each other's internals.

This is the final score for iteration 6. The review was read-only: `git diff origin/main..HEAD`,
`git show`, `rg`/`grep`, `sed`, `awk`, `wc`, a Python pass over the branch's added import lines,
the internal `Cargo.toml` edges, and a crude brace-counting function-length scan of the changed
Rust files. Nothing was built or tested. Every status was confirmed against
`fix/quality-iter6-20261023` at `2065f00e9` (the branch HEAD; `origin/main` is `02a64019b`).

Excluded: `target/`, `node_modules/`, `docs/reviews/`.

## Score

Rubric: 10.0 − 1.0 per blocker − 0.3 per major − 0.1 per minor − 0.03 per nit. Only open findings
count.

| Source | Blocker | Major | Minor | Nit |
|--------|---------|-------|-------|-----|
| Wave-3 findings still open (renumbered 6f-01..04) | 0 | 0 | 0 | 4 |
| New in the final pass (areas no earlier wave covered) | 0 | 0 | 2 (6f-05, 6f-06) | 0 |
| **Total** | 0 | 0 | 2 | 4 |

**Score: 10.0 − 0 × 1.0 − 0 × 0.3 − 2 × 0.1 − 4 × 0.03 = 10.0 − 0.20 − 0.12 = 9.68 / 10**

The wave-3 re-score was 9.88. The code has not changed since that report (HEAD is the same
commit), so its four nits are still open. The drop to 9.68 comes from two areas no earlier wave
judged, not from a regression: the OAuth token clients in the API-client routes (6f-05) and the
hand-rolled reorder controllers that wave 0 listed as "not judged for cost" (6f-06). Iteration 6
started at 7.11 (wave 0).

## Status of the wave-3 findings

| ID (wave 3 → final) | Sev | Status | Evidence now |
|---------------------|-----|--------|--------------|
| 6w3-01 → **6f-01** cwd-encoder / Claude-layout copies | nit | **Open** | `otto-server/src/routes/handover.rs:587-613` still builds `HOME/.claude/projects` by hand (`:591`) with its own encoder (`:593-596`), called from `gather_source_context` (`:567`). `otto-agent-run/src/session_turn.rs:1046-1050` (`claude_tasks_dir`) still has its own `is_ascii_alphanumeric() { c } else { '-' }`; the same file's test uses `roots::encode_cwd` (`:1281`). |
| 6w3-02 → **6f-02** automation run loop in an axum handler | nit | **Open** | `routes/api_automation_runs.rs:86` `start` still owns the `tokio::spawn` run loop (`:172`), `PRUNE_EVERY` (`:223`, `:292`) and `prune_history` (`:312`). `routes/api_client/automations.rs:83-99` still calls `start` with hand-built `Path`/`State`/`CurrentUser`/`CurrentAuthContext`/`Json`. |
| 6w3-03 → **6f-03** `RefChip` type vs `RefChip.svelte` | nit | **Open** | `git/ref-menus.ts:62` `interface RefChip`; `git/graph/RefChip.svelte:8`; `GraphView.svelte:24` (`type RefChip as Chip`) beside `:34`; `graph/CommitDetailPane.svelte:12-13`. |
| 6w3-04 → **6f-04** product per-lens retry copies `LensTurn::run` | nit | **Open** | `otto-product/src/run/retry.rs:38` `retry_analysis_agent_owned`, own `run_agent_with_recovery` (`:198`), own parse with the comment "mirrors the run_analysis per-agent logic" (`:221-222`). `LensTurn` (`run/analysis.rs:196`) keeps the richer error text (`:288`). |

None was fixed or partly fixed, and none is invalid. The earlier findings that waves 1–3 closed
(1-01r, 1-04, 1-05, 1-07r, 1-08r, 1-09, 1-10, 1-13, 2-06, 4-01r, 4-02, 4-04r, 5-02r, 5-03, 5-04,
5-05r, 6-01, 6-02, 6-03, 6-04, 6-05r, 6w1-01, 6w1-02, 6w2-01, 6w2-02) were spot-checked for
regression: `lib/` → `modules/` imports are still zero, `otto-rooms` still has no `ServerCtx`/`axum`/
`otto_server` reference, `git/GraphView.svelte` is 1,064 lines, `routes/api_client.rs` 917, and
no crate other than `ottod` depends on `otto-server`. No regression.

## Areas covered

The wave-0 areas, re-read fresh, plus the whole iteration diff (`git diff origin/main..HEAD`,
864 files, +74,339/−37,559):

- **Crate dependency graph.** Every `otto-*` edge in every `crates/*/Cargo.toml`. Only `ottod`
  depends on `otto-server`. The edges added this iteration (`otto-product` → `otto-agent-run`,
  `otto-context` → `otto-transcript`, `otto-apiclient` → `otto-state`/`otto-ssh`, `otto-server` →
  `otto-rooms`, `otto-canvas` → `otto-design` in place of `otto-design-assist`) all point leaf-ward. No cycle.
- **UI layering.** `lib/` → `modules/`: none. Cross-module imports added on the branch: three
  (`workflows/RunMonitor.svelte`, `RunContextSidebar.svelte` → `panels/FileTree`;
  `workflows/runSync.svelte.ts` → `product/types`). All three were moved out of
  `WorkflowsPage.svelte`, which already imported both on `origin/main` (`:3`, `:31`). No new edge.
- **Transcript discovery.** A sweep for the encoder (`is_ascii_alphanumeric() { c } else { '-' }`),
  `.claude/projects` and `join(".claude")`. Production copies outside `otto_transcript::roots`: the
  two in 6f-01. `otto-workflows::slug`, `otto-sandbox`, skills paths and `otto-usage`'s test-only
  `list_claude_files` are unrelated or deliberate.
- **Recent churn / big files.** Sizes of the large ratcheted files are unchanged since wave 3
  (`manager.rs` 6,502, `otto-git/local.rs` 5,533, `workflow_engine.rs` 4,615, `ottod/mcp_tools.rs`
  4,347 with tests from `:1853`). UI files that grew most on the branch are all extraction targets
  (`graph/*`, `stores/database/*`, `stores/apiClient/*`, `JiraIssueSection`, `runSync`). The
  `scripts/loc-baseline.json` function cap (`function_max` 400) holds for the new files; the largest
  new ones (`run_session_turn_inner`, `run_docs`, `engine::run_step`, `run_saved`, `refine`) are
  under it.
- **The moved engines.** `otto-rooms` + `otto-server/src/{rooms,game_rooms}` (two `terminal.rs`
  files are flow state vs socket, not copies); `otto-apiclient/src/engine/*` and what stayed in
  `routes/api_client.rs` (produced 6f-05); `otto-product/src/run/*` (6f-04);
  `otto-agent-run/src/{fanout,assist}`; `ottod/src/mcp_tools/{catalog,dispatch}.rs`;
  `otto-mcp/src/governance.rs` + `mcp_outward.rs::governed_invoke` (557 → ~395 lines, a gate
  pipeline over `governance::run`; no finding).
- **UI store splits.** `lib/stores/database/*` parts take `Pick<DatabaseStore, …>` host contracts
  (`connections.svelte.ts:55`); `lib/stores/apiClient/*`; `git/graph/*` host factories. No reach
  into private state.
- **Previously uncovered, now judged:** hand-rolled drag-to-reorder (produced 6f-06);
  `webhook_key_guard` (`routes/channel_webhook.rs:134`, one shared gate used by the three webhook
  routes; no finding); typed-client gaps in settings/workflows/git (95/33/93 raw `api.*` calls; the
  sampled repeated paths, e.g. `/repos/{id}/review-config` ×3 in `ReviewPanel.svelte:759,784,843`,
  use one response type each, so no confirmed drift and no finding); `vault_docs_agent` placement
  in otto-server (4,130 lines over `crate::agent_session`/`auth`/`modules`; otto-server is the
  documented host for orchestration engines and no cost beyond placement was confirmed, so no
  finding).

## Areas NOT covered

- otto-usage `engine.rs` (3,310) / `clickhouse.rs` internals.
- otto-dbviewer `service.rs` (4,177) / `types.rs` split.
- otto-git `local.rs` (5,533) internal cohesion.
- The ratcheted god functions as functions: `route_for` (1,536), `policy_for` (1,411),
  `controller_inner`, `bridge::handle`, `static_review`, `run_skill_eval_core`,
  `route_result_inner`, `run_review_core`.
- otto-design-assist `lib.rs` (2,743) internals beyond its size (`assemble` ≈ 307 lines).
- Large UI files: `Terminal.svelte` (3,532), `DatabasePage` (2,816), `ResultsGrid` (2,646),
  `ReviewPanel` (2,519), `Navigator` (2,517, only its drag code was read), `DiffViewer`,
  `QueryEditor`, `QueryBuilder`, `UsagePage`, `vault/GraphView` (1,980), `RequestBuilder`,
  `DocsAgentsView`, `FloatingBar`, `SessionView`, `workspace.svelte.ts` (2,129).

---

## Open findings

### architecture-6f-01: nit: two more cwd-encoder / Claude-layout copies outside `otto_transcript::roots`

(Was 6w3-01; unchanged.)

**Location:**
- `crates/otto-server/src/routes/handover.rs:587-613` (`find_claude_transcript`; `HOME/.claude/projects`
  at `:588-591`, encoder at `:593-596`), called at `:567` from `gather_source_context` (`:560`)
- `crates/otto-agent-run/src/session_turn.rs:1046-1050` (`claude_tasks_dir`)

**Evidence:** `otto_transcript::roots::{encode_cwd, default_claude_root, TranscriptRoots}`
(`roots.rs:46-57`) is the one layout. `find_claude_transcript` only looks under `$HOME`, so a
handover from a Claude session running under a provider account (or `OTTO_TRANSCRIPT_ROOTS`) misses
the transcript and falls back to scrollback. `claude_tasks_dir` re-implements `encode_cwd` in a
crate that already depends on `otto-transcript`.

**Fix:** in `gather_source_context`, call `crate::transcript::resolve_transcript_sync(&ctx.data_dir,
None, source)` (`transcript/resolve.rs:65`) inside the existing `spawn_blocking` and delete
`find_claude_transcript`. In `claude_tasks_dir`, use `otto_transcript::roots::encode_cwd`.

### architecture-6f-02: nit: the API automation run loop lives in an axum handler, and another handler calls it

(Was 6w3-02; unchanged.)

**Location:**
- `crates/otto-server/src/routes/api_automation_runs.rs:86-287` (`start`; the run loop in the
  `tokio::spawn` at `:172`; `PRUNE_EVERY` `:223`/`:292`; `prune_history` `:312`)
- `crates/otto-server/src/routes/api_client/automations.rs:83-117` (`run_automation` calls `start`
  with hand-built extractors at `:91-99`)

**Evidence:** the per-step engine moved to `otto_apiclient::engine` behind `ApiClientCtx`, but the
loop that drives a run (dataset rows, cancel registry, history rows and events, retention, final
status) did not. The synchronous route (reached by the MCP `api_run_automation` twin) re-enters the
async route handler. Adding an extractor to `start` breaks `run_automation`, and no non-HTTP caller
can start a run.

**Fix:** move the loop to `otto_apiclient::engine::automation::start_run(ctx: &impl ApiClientCtx,
…) -> RunHandle` (cancel/progress registry, `prune_history`, `runs_keep` through the ctx); have both
routes call it, and let `run_automation` await the handle.

### architecture-6f-03: nit: the `RefChip` type and `RefChip.svelte` share a name

(Was 6w3-03; unchanged.)

**Location:** `ui/src/modules/git/ref-menus.ts:62`; `ui/src/modules/git/graph/RefChip.svelte:8,11`;
`ui/src/modules/git/GraphView.svelte:24` (`type RefChip as Chip`) with `:34`;
`ui/src/modules/git/graph/CommitDetailPane.svelte:12-13`.

**Evidence:** the GraphView split named the new component after the existing data type, so the two
chip-rendering files alias the type to `Chip`, and grep/go-to-symbol for `RefChip` returns both.

**Fix:** rename the component `graph/RefChipView.svelte` (or the type `RefChipData`) and drop the
`as Chip` aliases.

### architecture-6f-04: nit: product's per-lens retry is a second copy of `LensTurn::run`

(Was 6w3-04; unchanged.)

**Location:** `crates/otto-product/src/run/retry.rs:38-260` (session run `:198`, parse/persist
`:221-259`); `crates/otto-product/src/run/analysis.rs:196-320` (`LensTurn`).

**Evidence:** the retry (also used by the boot reaper, `run/reap.rs`) repeats prompt + out-path,
`run_agent_with_recovery`, `set_agent_session`, `extract_json_as::<Findings>`, status, the
`SUPERSEDED` skip and the failure notice, and says it "mirrors the run_analysis per-agent logic"
(`:221`). It has already drifted: `LensTurn` reports "agent failed after {MAX_AGENT_ATTEMPTS}
attempts ({r})" (`analysis.rs:288`); the retry reports only "retry: session produced no output".

**Fix:** add `LensTurn::run_on_row(agent_id, spec, cwd)` for everything after the row exists; have
`LensTurn::run` and `retry_analysis_agent_owned` both call it.

### architecture-6f-05: minor: two OAuth token-endpoint clients in route files, with different guards

**Location:**
- `crates/otto-server/src/routes/api_client.rs:525-650` (`oauth2_token`: client_credentials /
  password / refresh_token), constants `:653-655`, `oauth_error_message` `:663`
- `crates/otto-server/src/routes/api_oauth.rs:258-408` (`exchange`: authorization_code)

**Evidence:** the REST engine moved to `otto-apiclient/src/engine/*` this iteration (1-07r), but
both token exchanges stayed in otto-server as hand-built HTTP clients that do the same job (POST a
form to `token_url`, cap the body, parse the token, scrub errors), and they already disagree:

| | `oauth2_token` | `exchange` |
|---|---|---|
| TLS rule | none (`check_url` only, `:595-597`) | `require_tls_or_loopback(token_url)` (`:298`) |
| Address pinning | `guarded_client_builder()` connect-time guard (`:606`) | own DNS lookup + `resolve_to_addrs` + `no_proxy` (`:299-320`) |
| Body cap | 256 KiB (`OAUTH_TOKEN_BODY_MAX`, `:655`) | 1 MiB (`:340`) |
| Error text | endpoint's `error_description` via `oauth_error_message`, secrets scrubbed (`:635-648`) | "Token endpoint returned HTTP {status}" (`:328-333`) |

`otto-apiclient/src/engine` has no token client (`engine/auth.rs` only applies an existing token,
`:9`), so the next grant type or guard change has to be made in two route files.

**Cost:** a security or policy fix to one grant (TLS, pinning, body cap) silently misses the other,
which is already the case for the TLS rule; neither path is reachable from the engine (a workflow
or automation step that needs a token has to call a route).

**Fix:** add `otto_apiclient::engine::oauth::{TokenRequest, request_token(ctx: &impl ApiClientCtx,
wid, token_url, form) -> Result<Value, String>}` holding the one client policy (TLS-or-loopback,
pinned resolve unless the workspace allows local, one body cap, `oauth_error_message` + scrub).
`oauth2_token` keeps the host-binding/marker resolution and builds its form; `exchange` keeps the
flow/fingerprint checks and builds its form; both call `request_token`.

### architecture-6f-06: minor: list reorder (drag state + keyboard move) is hand-rolled six times; the one shared controller sits in `product/`

**Location:**
- `ui/src/shell/Navigator.svelte:235-255` (agent rows), `:574-605` (modules/favorites), `:699-724`
  (sections)
- `ui/src/shell/TabBar.svelte:84-140` (⌥⇧←/→ + Move left/right menu), `:266-296` (drag)
- `ui/src/modules/git/GitTabs.svelte:89-137` (key + menu, "as in shell/TabBar", `:88`),
  `:139-167` (drag, "mirrors shell/TabBar")
- `ui/src/modules/agents/TiledView.svelte:228-259` ("Same HTML5 idiom as the tab bar")
- the existing controller: `ui/src/modules/product/caseOrder.svelte.ts:17` (`CaseOrder`: drag
  state, ⌥↑/↓ `onKey` `:121`, `announce` `:116`), used only by `TestCasesTab.svelte`

**Evidence:** each site declares its own `dragId`/`dragOverId` (and sometimes `dropSide`) state
and the same `onDragStart`/`onDragOver`/`onDragLeave`/`onDrop`/`onDragEnd` set
(`effectAllowed = 'move'`, `dropEffect = 'move'`, reset on end). Two files say outright that they
mirror `TabBar`. Only the pure index math is shared (`lib/sidebar.ts:364` `reorderAmong`). The
copies have drifted on the non-pointer path:

- `CaseOrder` announces the new position; no other site does.
- Tabs, git tabs and Navigator modules/favorites have ⌥-arrow moves; Navigator agent rows have only
  a "Move to top" menu item (`Navigator.svelte:1587`); tiles (`TiledView`) have no keyboard move.

**Cost:** every reorder behaviour change (keyboard alternative, announcement, touch fallback —
`Navigator.svelte:231` notes HTML5 DnD is inert on iOS) has to be made in six places, and the
drift above shows it is not.

**Fix:** generalise `CaseOrder` into `lib/listReorder.svelte.ts` (`ListReorder<Id>` with drag state,
`dropSide`, `onKey(e, id, ids, axis: 'vertical' | 'horizontal', rtl)`, `announce`, and a
`move(from, to)` host callback), keep `reorderAmong` as its index helper, and port TabBar, GitTabs,
TiledView, the three Navigator lists and TestCasesTab onto it.
