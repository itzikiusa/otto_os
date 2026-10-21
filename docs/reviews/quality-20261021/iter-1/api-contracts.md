# Quality iteration 1 — lens: api-contracts

Reviewer scope: parity between `docs/contracts/{api,ws,rooms}.md`, the axum
routes / WS events in `crates/`, `ui/src/lib/api/types.ts` and the UI callers,
plus `docs/features/` drift. Read-only; no builds or tests were run.

## Score

Start 10.0. Open findings: 0 blockers, 1 major, 9 minor, 3 nits.

`10.0 − 1×0.3 − 9×0.1 − 3×0.03 = 10.0 − 0.3 − 0.9 − 0.09 = ` **8.71**

## Previous-finding status

None. This is the first iteration of this loop for this lens.

## Method / areas covered

All checks were mechanical cross-references (scratch scripts over the source),
and every flagged hit was then confirmed by reading the code by hand.

- **Route inventory.** 1059 `.route("…")` registrations, with test modules
  stripped. Every non-test path appears in `docs/contracts/*.md`. Method by
  method (1291 method/path pairs), every pair is documented, including the
  table-style sections (`/access/*`, `/telemetry/*`).
- **UI callers to routes.** Every `api.get/post/put/patch/del(...)` and direct
  `fetch(.../api/v1/...)` in `ui/src` (excluding `rooms/games`) was matched
  against the method+path inventory. One caller hits a route that does not
  exist (finding 01).
- **WS events.** All 80 `Event` variants are documented in `ws.md` and present
  in the TS `OttoEvent` union. Field sets match. `ws.md` JSON examples match the
  Rust fields. `ui/unit/eventContract.test.ts` guards this.
- **Terminal and room sockets.** Client frames in `/ws/term` (`ClientFrame`,
  including the new `claim{cols,rows}` and the `grid` feed) and in the room
  terminal (`rooms/terminal.rs` `Frame`) match `ws.md` and `rooms.md`. Room
  HTTP routes and status codes also match.
- **REST DTOs.** Same-named Rust `Serialize`/`Deserialize` structs were compared
  with TS interfaces across all crates (field sets, and `skip_serializing_if`
  against TS required-ness). So were unit enums against TS string unions.
- **Status codes.** Handlers returning 201/204 were compared with the
  documented responses.
- **Auth column.** Rows documented as `root` were compared with the handler's
  `require_root` / policy class.
- **Recent contract-touching commits (3 weeks).** Git landed marks (#113),
  workbench scope, the terminal grid feed (#108), agent-to-agent messaging
  (#110), Linear tracker (#107), JQL/CQL (#106), the channel-bridge owner rule,
  proof assemble cwd confinement (#109), and workspace `settings_patch`. All of
  these are consistent except where listed below.

## Areas NOT covered yet (next iteration)

- Per-field request/response parity for DTOs whose Rust and TS names differ
  (the script only pairs identical names). Priority: Product, Swarm, Design
  Hall and the API client.
- Documented 4xx semantics (409/404/400 claims) against handler bodies. Only
  201/204 were checked mechanically.
- Ownership and visibility claims in `api.md` §RBAC compared with
  `policy.rs` / `feature_guard.rs`, beyond the root rows.
- `docs/contracts/game-rooms.md`, `room-recaps.md`, `product.md`, and
  `ui-commands.json` against `crates/otto-mcp/src/outward/ui_commands.rs`.
- `docs/features/*` tables other than git, proof-packs and workbench.
- MCP outward catalog (`otto-mcp/src/outward/catalog.rs`) arguments against
  the REST routes they self-call.

## Findings

### api-contracts-1-01 — major — Proof page calls `GET /repos/{id}`, a route that does not exist

- **Where:** `ui/src/modules/proof/ProofPage.svelte:402-410` (`packRepoPath`),
  which is used by Assemble (`:415`) and PR check (`:375`). The route is
  registered at `crates/otto-git/src/http.rs:106-108`.
- **Evidence:** The UI does
  `api.get<{ path: string }>(\`/repos/${encodeURIComponent(rid)}\`)`. The only
  registration for `/repos/{id}` is
  `.route("/repos/{id}", patch(update_repo::<S>).delete(delete_repo::<S>))`,
  so a GET answers 405. `api.md` documents no `GET /repos/{id}` either. The
  `catch { return ''; }` hides the failure.
- **Impact:** The feature from commit 31a0b648c ("default folders to the pack's
  repo") never works. The Assemble prompt and the PR-check cwd always start
  empty. Since #109, a non-root caller also has to type a path that is inside
  the pack's workspace, repos or session.
- **Fix:** Pick one of these:
  - Add `GET /repos/{id}` (ws viewer, returns `Repo`) to `otto-git` http.rs, and
    document it in `api.md` next to `PATCH /repos/{id}`.
  - Resolve the path on the client from the already-loaded repo list
    (`GET /workspaces/{id}/repos`), filtered by `pack.repo_id`.

  In either case, stop swallowing a 4xx silently.

### api-contracts-1-02 — minor — TS `ReviewFinding` declares `fingerprint` and `state`, which the server never sends; the UI keys on them

- **Where:** `ui/src/lib/api/types.ts:3958-3961`;
  `ui/src/modules/git/ReviewAgents.svelte:198,203`; Rust
  `crates/otto-core/src/domain.rs:1124-1138`.
- **Evidence:** The Rust `ReviewFinding` has only `path`, `line`, `line_end`,
  `severity`, `body` and `lens`. Nothing injects `fingerprint` or `state` into
  `agents_json` (`rg '"fingerprint"'` finds nothing in otto-server or
  otto-review). Those fields live on `ReviewFindingRow`
  (`/reviews/{id}/findings`). The agent list renders
  `{#each agent.findings as f (f.fingerprint ?? f.body)}` and
  `{#if f.state && f.state !== 'open'}`.
- **Impact:** The lifecycle chip can never render. The each-key falls back to
  `body`, so two findings with the same text from one agent (the same issue at
  two lines) produce duplicate keys in a keyed `{#each}`.
- **Fix:** Remove `fingerprint` and `state` from `ReviewFinding`. Key the list
  on `${f.path}:${f.line}:${i}` or on the index. If the chip is wanted, join on
  `ReviewFindingRow` by fingerprint.

### api-contracts-1-03 — minor — `ReviewAgentCfg.skill` is missing from TS, and saving an agent as a preset drops it

- **Where:** Rust `crates/otto-core/src/domain.rs:1082-1089` (`pub skill: String`,
  set to `correctness-review` / `security-review` on the default agents at
  `crates/otto-review/src/engine.rs:238,251`). TS
  `ui/src/lib/api/types.ts:4043-4053` has no `skill`.
  `ui/src/modules/git/ReviewPanel.svelte:953-959` (`saveAsPreset`).
- **Evidence:** `saveAsPreset` builds
  `{ name, provider, providers, model, prompt }` field by field, so a preset
  made from a default agent loses its `skill`. That preset then runs without the
  inlined skill method. The UI cannot show or edit the field.
- **Fix:** Add `skill?: string` to `ReviewAgentCfg` in `types.ts`. Copy it in
  `saveAsPreset`. Optionally expose it in the agent editor.

### api-contracts-1-04 — minor — `POST /state/restore` is documented to return `{restored}` but answers 204 with no body

- **Where:** `docs/contracts/api.md:3711`;
  `crates/otto-server/src/routes/backup.rs:345` (`ApiResult<StatusCode>`) and
  `:401` (`Ok(StatusCode::NO_CONTENT)`).
- **Evidence:** The doc row reads `| POST /state/restore | root | {backup, confirm:true} | {restored} |`.
  The handler returns `StatusCode::NO_CONTENT`. The accepted keys are written
  only to the audit row.
- **Fix:** Document `204` (root, person credential only; `require_human`). The
  alternative is to return `{accepted: string[]}`, the shape
  `/settings/import` uses.

### api-contracts-1-05 — minor — `GET /settings/pr-review` is documented as root-only but is open to any member with Git View

- **Where:** `docs/contracts/api.md:3070`;
  `crates/otto-server/src/modules.rs:5777-5782` (`get_review_config` takes
  `CurrentUser(_user)` and runs no role check);
  `crates/otto-server/src/policy.rs:734-737` (`Require(Git, View)` for GET).
- **Evidence:** The contract row says `root`, but the handler and policy allow
  any member with Git View. The presets row (`:3072`) correctly says
  `member (Git View)`. Only `PUT` is root plus human (`modules.rs:5790-5791`).
- **Fix:** Change the row to `GET … | member (Git View) |` and
  `PUT … | root (person credential) |`.

### api-contracts-1-06 — minor — Contract #16a lists the scratch-workspace routes but leaves out Workbench, which the guard allows

- **Where:** `docs/contracts/api.md:76` (#16a). Code:
  `crates/otto-server/src/routes/mod.rs:807-836` (`scratch_path_allowed`).
- **Evidence:** #16a says "**Only the session routes exist under `scratch`**
  (`…/sessions…`, plus `…/broadcast`, `…/activity/summary` and `…/members`…):
  every other `/workspaces/scratch/…` route family answers **404**". The guard
  also admits `tail == "workbench" || tail.starts_with("workbench/")`, and the
  Workbench section of the same file says "Also served under the `scratch`
  workspace".
- **Impact:** The contract contradicts itself on who can reach which family
  under the implicit-Editor workspace.
- **Fix:** Add `…/workbench…` (owner-scoped) to the #16a allow-list sentence.

### api-contracts-1-07 — minor — TS `UpdateWorkflowReq` and `WorkflowVersion` lack `on_restart`, which the contract names

- **Where:** `ui/src/lib/api/types.ts:6130-6135` (`UpdateWorkflowReq`) and
  `:6106-6117` (`WorkflowVersion`). Rust `crates/otto-core/src/workflows.rs:440-442`
  and `:515-517`. Docs `api.md:2364` and `:2449-2450`.
- **Evidence:** The contract says the policy is "settable via
  `UpdateWorkflowReq.on_restart`" and that `WorkflowVersion` includes
  `on_restart`. The UI sends it untyped
  (`WorkflowsPage.svelte:799-801`, `api.patch<Workflow>(…, { on_restart: … })`).
- **Fix:** Add `on_restart?: 'resume' | 'fail' | null` to `UpdateWorkflowReq`
  and `on_restart: 'resume' | 'fail'` to `WorkflowVersion`. Type the PATCH body.

### api-contracts-1-08 — minor — `docs/features/proof-packs.md` assemble row omits the cwd confinement (403)

- **Where:** `docs/features/proof-packs.md:624` against
  `docs/contracts/api.md:4242`. Code: `require_assemble_cwd` in
  `crates/otto-server/src/routes/proof.rs`, added in #109.
- **Evidence:** The contract row adds "`cwd` outside the pack workspace root,
  its registered repos, the calling agent session cwd and the pack session cwd
  → 403 unless the caller is the root owner on their own credential". The
  feature-doc copy of row 122 stops at the `commands` rule, and the
  troubleshooting section has no 403 entry.
- **Fix:** Bring the feature-doc row in line with the contract, and add a
  troubleshooting line for "cwd must be inside this pack's workspace…".

### api-contracts-1-09 — minor — `docs/features/git.md` does not document the merged/landed branch marks or the cleanup-base routes

- **Where:** `docs/features/git.md:604` (the only `/refs` mention). The
  contract has `api.md:1443-1445` (`merged_into_base`, `merged_into`,
  `GET|PUT /repos/{id}/cleanup-base`).
- **Evidence:** `grep -n 'cleanup-base\|merged_into\|safe to delete' docs/features/*.md`
  finds nothing. The feature (#113: "develop · main" marks next to each branch)
  and the per-repo base override have no user-facing walkthrough or route rows.
- **Fix:** Add a short "Merged branches" subsection: what the check and the
  names mean, how the base is chosen and overridden, and that it is
  indicator-only. Add `GET|PUT /repos/{id}/cleanup-base` to the route table.

### api-contracts-1-10 — minor — `api.md` names 23 request types that exist in neither Rust nor TS

- **Where:** For example, `api.md:1869` `DoTransitionReq`, `:1873`
  `AddCommentReq`, `:1330` `CreateSavedQueryReq`, `:2625`
  `CreateCollectionReq`, `:3043` `InstallServersReq`, `:3562`
  `UpsertLagAlertReq`, `:1638` `PatchStoryReq`, `:1668` `CreateTranscriptReq`.
- **Evidence:** `rg '\b<Name>\b' ui/src crates` finds no definition for any of
  them. The full list is AddCommentReq, AssignReq, CreateAutomationReq,
  CreateCollectionReq, CreateEnvironmentReq, CreateLearningReq, CreateNoteReq,
  CreateQuestionReq, CreateRequestReq, CreateSavedQueryReq, CreateSectionReq,
  CreateTranscriptReq, DoTransitionReq, ExecuteRequestReq, InstallServersReq,
  PatchStoryReq, RenameSectionReq, UpdateAutomationReq, UpdateCollectionReq,
  UpdateEnvironmentReq, UpdateRequestReq, UpsertLagAlertReq and
  UpsertTriggerReq.
- **Impact:** A reader cannot find the request shape these rows promise.
- **Fix:** Replace each name with the real Rust/TS type name, or with an inline
  `{field, …}` shape.

### api-contracts-1-11 — nit — TS `ImprovementTrigger` lacks `'live'`

- **Where:** `ui/src/lib/api/types.ts:4567` against
  `crates/otto-core/src/domain.rs:2161-2166`.
- **Evidence:** The in-loop evolver creates runs with `ImprovementTrigger::Live`
  (`otto-improve/src/engine.rs:540,598,1475`). TS has
  `'scheduled' | 'manual'` only.
- **Fix:** Add `| 'live'`.

### api-contracts-1-12 — nit — `ws.md` says "There are 79 variants"; `Event` has 80

- **Where:** `docs/contracts/ws.md:586`; `crates/otto-core/src/event.rs`
  (`type_name()` lists 80 arms).
- **Fix:** Change the count to 80, or drop the number (the unit test only
  asserts `>= 79`).

### api-contracts-1-13 — nit — TS `K8sCluster` / `ImportK8sClusterReq` drift

- **Where:** `ui/src/lib/api/types.ts:9390-9407` and `:9423-9429` against
  `crates/otto-state/src/k8s_clusters.rs:60-90` and
  `crates/otto-k8s/src/clusters.rs` (import req `color`).
- **Evidence:**
  - Rust always serializes `params` (no skip); TS omits it.
  - Rust `created_by: Option<Id>` is serialized as `null`; TS says `created_by: Id`.
  - The import request accepts `color`; the TS `ImportK8sClusterReq` lacks it.
- **Fix:** Add `params: Record<string, unknown>`, make it
  `created_by: Id | null`, and add `color?: string | null` to
  `ImportK8sClusterReq`.
