# Agent 6 — Contract sync (api.md / ws.md) + route-inventory test

Hardening pass, 2026-06-19. Scope per assignment: reconcile route–contract drift
(`docs/deep-dive-improvements-2026-06-19.md` §1; product audit Cross-Cutting MH / P0 #3).
Files owned/touched (only these): `docs/contracts/api.md`, `docs/contracts/ws.md`,
`crates/otto-server/tests/route_inventory.rs` (new).

## [x] T11a — Document every registered REST route in api.md

Enumerated the **real** registered route set (paren-balanced extraction of every
`.route("PATH", …)` literal across all crates, plus the root-level WS/proxy routers and the
`build_router` composition in `crates/otto-server/src/lib.rs` / `crates/ottod/src/main.rs`).

- **261 distinct route paths** are registered. The pre-existing api.md documented ~89 of
  them (#1–#89). I added the remaining undocumented surface as a new "extended surface"
  section, preserving the FROZEN v1 framing (the #1–#89 tables are untouched).
- After edits, **all 261 registered paths appear verbatim in api.md** (verified: 0 missing).
- New sections added (method, path, brief purpose, auth/role where derivable from the
  handlers' `require_ws_role` / `require_root` / owner checks):
  - Activity trail & task tracker (`/workspaces/{wid}/sessions/{sid}/trail|tasks`, activity summary)
  - Sessions extras (archive/unarchive/input/handover/handover-brief/attach-product, `/app/kill-sessions`)
  - Connection sections
  - DB Explorer engine access (`/connections/{id}/db/*`, incl. explain-with-agent)
  - DB Explorer saved-queries / dashboards / widgets
  - Git repo & PR extras (refs, fetch, discard, merge/conflict, PR commits/request-changes,
    api-collections pull/push, pr/draft, `remote-repos`, `repos/detect`)
  - PR review agents (PR + local review, comment approve/decline, handoff, agent retry)
  - Orchestrator broadcast
  - Product (stories, versions, analyses, questions, notes, events, testcases, transcripts,
    learnings, drafts) + Product AI actions (analyze/rewrite/generate/plan, agent retry/stop)
  - Issue trackers (Jira/Confluence)
  - Channel integrations
  - Self-improvement engine (runs, edits approve/reject/rollback)
  - Skill evaluations (+ `/settings/skill-eval`)
  - Context library (skills/souls/context/default-soul, ws context + materialize)
  - Bundled skills
  - Workflow engine
  - API client ("Postman": collections/requests/environments/history/execute/grpc/oauth2/
    cookies/automations, import-curl)
  - Notifications
  - Usage tracking & metrics
  - Insights
  - LSP install + capabilities
  - Provider registry update
  - Filesystem & logs (operator tools)
  - PR-review config (`/settings/pr-review`)
  - Swarm lifecycle explicit paths (expands frozen #84's combined `start|pause|abort|resume`)
  - Root-level routers (`/ws/term`, `/ws/events`, `/ws/lsp`, `/ws/api-client/stream`,
    `/browser/proxy`) — documented as NOT under `/api/v1`, `?token=` auth
  - Ingest routes (session-token gated): `/ingest/{claude,codex,usage,swarm/board}`

## [x] T11b — Enumerate all Event variants in ws.md

Read `crates/otto-core/src/event.rs` and documented **all 16 `Event` variants** (was ~5),
each with its `type` tag and payload shape, grouped by area:

- Session lifecycle (4): `session_status`, `session_created`, `session_meta_updated`, `session_removed`
- Notices (2): `notice` (broadcast), `notification` (persisted)
- Activity (2): `trail_appended`, `tasks_updated`
- Self-improvement (4): `improvement_run_started`, `improvement_run_finished`,
  `improvement_edit_applied`, `improvement_approval_pending`
- Swarm (4): `swarm_status`, `swarm_run_updated`, `swarm_task_updated`, `swarm_message_posted`

Also captured delivery scoping (workspace-scoped vs. broadcast) and the JSON-embedded-row
note for swarm payloads.

Concurrent-change note: Agent 3 added `user_id: Option<Id>` to the `Notification` event
variant in `event.rs` mid-pass (it appeared after my first read). I documented the new
optional field in ws.md (`{"type":"notification","notice":{…},"user_id":"…"}`), including
the `skip_serializing_if = "Option::is_none"` omission behavior and its
single-user-targeting semantics.

## [x] T11c — Route-inventory drift test

`crates/otto-server/tests/route_inventory.rs` (auto-discovered; no mod decl needed).

- `every_registered_route_is_documented` — walks `crates/**/*.rs`, extracts every
  `.route("PATH")` literal with a hand-rolled `std`-only scanner (tolerant of
  whitespace/newlines between `.route(` and the path, and of `{id}`/`:id` params; **no new
  dev-dependency** — `regex`/`walkdir` are not available, so I deliberately avoided them),
  reads `docs/contracts/api.md`, and asserts every registered path appears verbatim. Fails
  with the explicit list of undocumented paths. Includes a sanity floor (≥100 routes) so a
  broken extractor fails loudly instead of passing a near-empty set.
- `documented_paths_are_well_formed` — asserts each extracted path is non-empty, absolute,
  and brace-balanced (guards against a future scanner regression).
- Resolves the repo root from `CARGO_MANIFEST_DIR` by walking up to the dir containing both
  `crates/` and `docs/contracts/api.md`, so it's CWD-independent.

## Test result

`cargo test -p otto-server --test route_inventory` **cannot run to completion right now**
because the `otto-server` **library** does not compile — Agent 3 is mid-edit adding a
`user_id` field to `NewNotice` / `Event::Notification`, producing 7 `E0063 missing field`
errors in **their** files (`src/monitor.rs`, `src/state.rs`, `src/routes/activity.rs`,
`src/skill_eval.rs`). **None** of the compile errors reference my test file.

To verify my test logic independently of the in-progress lib, I compiled and ran an exact
standalone copy of the test's extraction + assertion functions against the real repo:

```
registered=261 undocumented=0
PASS: every registered route documented; paths well-formed
```

So the test is correct and **will pass** under `cargo test -p otto-server --test
route_inventory` as soon as the crate compiles again (i.e., once Agent 3's `user_id` edits
land). The api.md I produced is complete enough to satisfy it (0 undocumented routes).
