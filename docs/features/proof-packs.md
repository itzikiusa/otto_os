# Proof Packs (the evidence layer)

> An agent may not declare a task "done" on assertion alone. Every meaningful unit
> of agent work carries a **Proof Pack** — a bundle of inspectable evidence whose
> **status is derived from the evidence, not claimed by the agent.** Otto assembles
> what it can automatically; agents and humans add the rest through a small API.

This document is the authoritative end-user + operator reference for Proof Packs.
Where it describes wire-level behaviour, the source of truth is the contract files
in `docs/contracts/` (`api.md` #115–125, `ws.md` `proof_pack_updated` /
`proof_pack_exported`). The **pure** domain logic lives in
`crates/otto-core/src/proof.rs`; persistence in `otto_state::proof`
(`ProofRepo`); the assembly engine + gates in `crates/otto-server/src/proof.rs`;
and the UI in `ui/src/modules/proof/ProofPage.svelte`.

---

## 1. What it is

A proof pack belongs to exactly one **work item**
(`UNIQUE(work_item_kind, work_item_id)`): a `session`, `goal_loop`, `review`,
`workflow_run`, `task`, or a `manual` pack. It holds **artifacts** (the evidence)
and a **derived status**:

| Status | Meaning |
|--------|---------|
| `missing` | no evidence at all |
| `partial` | some evidence, but the required set for this work-item kind isn't met |
| `passed` | required evidence present and nothing failed |
| `failed` | at least one artifact failed (e.g. a test command exited non-zero) |
| `waived` | a human explicitly waived the requirement (`waived_by` recorded) |

Status, `risk_score` (0–100), and the badge set are computed by **pure functions
in `otto-core::proof`** — `derive_status(&pack, &artifacts)`,
`compute_risk(&artifacts)`, and `compute_badges(&pack, &artifacts)` — the single
source of truth, recomputed on **every** mutation. The agent never sets the
status; it can only attach evidence.

`derive_status` is short and deliberate:

1. `waived_by` set → **`waived`** (short-circuits everything).
2. no artifacts → **`missing`**.
3. any artifact `failed` → **`failed`**.
4. the work-item kind's required set is met → **`passed`**, else **`partial`**.

### The "work item" + parent rollup

A pack carries `work_item_kind`, `work_item_id`, a `title`, a `summary`, the
derived `status` + `risk_score`, an optional `parent_pack_id` (a goal-loop pack
parents the session/review/workflow packs it spawns), and the waiver fields
(`waived_by`, `waived_reason`). The unique `(work_item_kind, work_item_id)` index
is the **ensure-or-create gate key** — re-running the same work item updates its
one pack rather than creating duplicates.

---

## 2. Required evidence (per work-item kind)

`required_kinds(kind)` maps each kind to a `RequiredSpec`; `passed` is reached
only when that spec is satisfied **and** nothing failed:

- **session / goal_loop / task → `CodeChange`** — a `diff` **and** ≥1 *passing
  recognized test* command. A `command` artifact counts as a test only when its
  **title** matches a real runner (see §3.1); a trivial green no-op like `true`
  does **not** count, so `passed` is not gameable.
- **review → `Review`** — ≥1 `review` artifact present (and, by the global rule,
  none failed). A `review` artifact that is `failed` makes the pack `failed`.
- **workflow_run → `WorkflowRun`** — ≥1 artifact present, none failed, and every
  `approval` artifact that *is* present is `passed` (a `pending` approval keeps it
  `partial`).
- **manual → `Lenient`** — ≥1 artifact whose status isn't `failed`.

---

## 3. Artifacts

An artifact is one piece of evidence inside a pack:

```
ProofArtifact { id, proof_pack_id, workspace_id, kind, title,
                content_ref, status, metadata, created_by, created_at, updated_at }
```

**Kinds** (`ProofArtifactKind`):
`command · log · screenshot · diff · ci · api · db · review · approval ·
self_review`.

**Statuses** (`ProofArtifactStatus`): `passed · failed · pending · info` —
`info` is neutral evidence (a diff, a logged sample) that is neither pass nor
fail and is the default when none is supplied.

`content_ref` holds inline text (capped, see §4), a URL, or a file reference; the
flavour is recorded in `metadata.ref_kind ∈ inline | url | file`. The engine also
stamps `metadata` with fields like `redactions`, `truncated`, `test_kind`
(`test`/`build`/`lint`), `exit_code`, `duration_ms`, and — for a diff —
`files_changed`, `additions`, `deletions`, `risky_files`.

### 3.1 Recognized test commands (non-gameable `passed`)

`looks_like_test_command(cmd)` does a case-insensitive substring match against a
fixed list of real runners. Today that list is:

```
cargo test · cargo nextest · go test · npm test · npm run test · npm run check ·
yarn test · pnpm test · jest · vitest · playwright test · pytest ·
python -m pytest · go vet · cargo clippy · svelte-check · ctest · gradle test ·
mvn test · rspec · phpunit
```

Build/lint runners (`go vet`, `cargo clippy`, `svelte-check`) are recognized too —
they all gate quality — and `classify_test_kind` records which it was in
`metadata.test_kind`. A `command` artifact is "test evidence" only if its **title**
matches one of these needles, which is what keeps `passed` honest.

---

## 4. Risk score

`compute_risk(&artifacts)` returns a clamped `0..100`:

| Signal | Contribution |
|---|---|
| Change size (`additions + deletions` on the diff) | `loc / 20`, capped at **40** |
| Risky files touched (count from the diff's `risky_files`) | `count × 8`, capped at **32** |
| A `migrations/` or `*.sql` file in `risky_files` | **+10** |
| A failing **test** artifact | **+25** |
| A failed **review** artifact (unresolved findings) | **+15** |
| A diff present but **no test command at all** (untested change) | **+10** |

A pack with `risk_score ≥ 50` (`RISKY_THRESHOLD`) **or** any risky file earns the
`risky_change` badge.

### Which files are "risky"

`is_risky_file(path)` matches by path **segment / extension / basename** — never a
naive substring — so `author.rs` and `tokenizer.rs` are **not** false-flagged:

- **Path segment**: `migrations/`, `.github/`.
- **Extension / basename**: `*.sql`, `Cargo.lock`, `package-lock.json`,
  `yarn.lock`, `pnpm-lock.yaml`.
- **Security whole-words in the basename** (split on non-alphanumeric, matched as
  whole words): `auth`, `rbac`, `keychain`, `netguard`, `policy`, `secret(s)`,
  `password(s)`, `crypto`, `token(s)`, `credential(s)`.

So `auth.rs`, `policy.rs`, `oauth_config.ts` hit; `author.rs`/`tokenizer.rs` do
not.

---

## 5. Badges

`compute_badges` derives the chip set shown in the UI. A pack may carry several:

| Badge | When |
|---|---|
| `no_proof` | zero artifacts and not waived |
| `tests_passed` | ≥1 test artifact, all passed |
| `tests_failed` | ≥1 test artifact failed |
| `human_approved` | ≥1 `approval` artifact `passed` |
| `risky_change` | `risk_score ≥ 50` **or** a risky file present |
| `ci_missing` | a `diff` present but no `ci` artifact |
| `db_api_verified` | a `db`/`api` artifact explicitly marked `passed` (not the default `info`) |
| `review_unresolved` | a `review` artifact `failed` |
| `waived` | the pack was waived |

---

## 6. Evidence integrity (redaction + cap + preview)

A trust layer must not itself leak secrets, so **all persisted artifact content is
redacted before storage**. `prepare_content` runs `otto_core::redact` over the
text first; `redact` strips a small set of high-confidence shapes — **JWTs**, **AWS
access keys**, **PEM blocks**, **`Bearer` tokens**, **emails**, and values under
**sensitive JSON keys** — replacing each with `[redacted]`. The number of hits is
recorded in `metadata.redactions`.

Content is stored in full up to **2 MiB** (`STORE_CAP`); larger content is
truncated at a char boundary with a trailing `…(truncated)` note and
`metadata.truncated = true`. List/detail responses carry an **8 KiB**
(`PREVIEW_CAP`) inline preview; the full stored content is fetched on demand via
`GET /proof-artifacts/{id}/content`.

---

## 7. Enforcement — "no done without evidence"

Otto packages a pack and derives a status whenever a work item reaches "done"; the
status and badges are the visible, non-bypassable signal. There are **five**
integration points. Two add **hard teeth**; three **package** evidence and surface
badges without a hard block.

### 7.1 Goal Loops — machine-checked (hard teeth, opt-in)

A goal loop already ground-truths declared `verify_kind=command` acceptance
criteria (an "achieved" verdict with an unmet command criterion is coerced to
"continue"). On success it **packages** each verify command (with captured
output) as a `command` artifact, the worktree diff as a `diff`, the evaluator's
feedback/rationale as a `self_review`, and a criteria summary as a `review`.

With **`OTTO_PROOF_REQUIRE_GOAL_LOOP=1`** the loop additionally **refuses to
finalize "achieved"** without a passing machine-checked test — bounded by the
iteration cap, accepting a `partial` pack on the last iteration so a genuine goal
is never failed purely for lack of a command. Default **off** (`require_goal_loop_proof()`
reads `1`/`true`).

### 7.2 PR creation — hard `409` gate (default ON)

Opening a PR whose `CreatePrReq` carries a `proof_pack_id` is **rejected with
`409 Conflict`** when that pack is not `passed`/`waived` — *unless* the caller
passes `allow_unproven: true`, which is recorded as an audit `approval` artifact
on the pack (`metadata {"override": true, "kind": "pr_override"}`) so the override
is itself evidence. The error message names the pack and its status:

```
proof pack <id> is '<status>', not passed — provide evidence or open with
allow_unproven to override
```

Disable the gate with **`OTTO_PROOF_REQUIRE_PR=0`** (or `false`); default **on**.
A PR with **no** linked pack is not gated — Otto can't enforce evidence it can't
locate — and an *unknown* pack id is allowed through rather than blocking the user.

### 7.3 AI review → review pack (package, lifecycle-driven)

Each completed AI review (see [`./code-review.md`](./code-review.md)) upserts its
summarized findings into the persistent `review_findings` store (fingerprinted),
and the review's pack gets a `review` artifact — `failed` while findings are
unresolved (which drives the `review_unresolved` badge), `passed` when clean.
Fix-attempt linkage is set via the finding-state endpoints — semi-automatic, not a
fully closed loop. The full tracked-finding workflow is documented in
[`./review-findings.md`](./review-findings.md).

### 7.4 Workflows — node + approval evidence (package)

On run completion each node's output becomes a `log` artifact (status from the
node status), each `human_approval` node an `approval` artifact (passed iff
approved, with approver/note/timestamp — drives `human_approved`), and the budget
gate a `log`.

### 7.5 Sessions — the all-done edge (package; tests opt-in)

When an agent marks every task complete, `gate_session` packages a pack: the
working-tree **diff always**, the repo's **tests opt-in** via
**`OTTO_PROOF_AUTO_TEST=1`** (running a repo's suite in the user's live cwd is
disruptive, so default off). Goal-loop-spawned session packs are linked to the
loop's pack via `parent_pack_id`. If the resulting proof is incomplete, Otto
surfaces a one-shot Notice ("Tasks done — proof incomplete").

### What's auto vs. what an agent/human supplies (honest)

| Proof item | Auto | Via API |
|---|---|---|
| Diff summary | ✅ goal-loop + session | — |
| Test commands + output | ✅ goal-loop (verify cmds); session (opt-in) | ✅ `/assemble {commands}` or a `command` artifact |
| Build / lint | partial (classified from the command) | ✅ `command` artifact |
| Screenshots / video | — | ✅ `screenshot` artifact (`content_url`) |
| API evidence | — | ✅ `api` artifact |
| DB evidence | — | ✅ `db` artifact |
| CI status | — | ✅ `ci` artifact (`content_url`) |
| Review findings | ✅ review gate | — |
| Agent self-review | ✅ goal-loop (evaluator) | ✅ `self_review` artifact |
| Human approval | ✅ workflow approval node + PR override | ✅ `approval` artifact |

Auto-capturing API-Client requests / DB-Explorer queries, and video, are
follow-ups; the artifact API supports all of them today.

---

## 8. The Proof module (UI)

`ui/src/modules/proof/ProofPage.svelte` is a two-pane viewer:

- **Left rail** — a status filter (`all · passed · failed · partial · missing ·
  waived`) over the workspace's packs, each row showing its title (or
  `work_item_id`), a `ProofStatusChip` (status + risk), the `work_item_kind` tag,
  and its `ProofBadges`. A **`+`** creates a `manual` pack (a random `work_item_id`)
  for ad-hoc verification ("Release 1.4 verification").
- **Right detail** — the open pack's badges + summary, then its **artifacts
  grouped by kind**. Each artifact shows a status dot, its title, and the status
  label; previews are expandable and **"Load full"** pulls the uncapped content via
  `GET /proof-artifacts/{id}/content`. A **waived** pack shows its reason and who
  waived it. Any **child packs** (rollups) are listed and clickable.
- **Pack actions** — **Assemble** (prompts for a working directory and re-runs
  diff + commands), **Add artifact** (a modal with kind/title/status/content),
  **Waive** (prompts for a reason), and **Delete** (cascades artifacts).

Live updates arrive over the `proof_pack_updated` WS event (the store re-fetches
the affected pack and refreshes the workspace summary). The cheap
`GET /proof-summary` roll-up powers compact status chips elsewhere in the app
(e.g. alongside a session) without loading every pack.

---

## 9. API & contract reference

Authoritative: `docs/contracts/api.md` **#115–125** and `docs/contracts/ws.md`
(`proof_pack_updated`). All routes are **`Feature::ProofPack`-gated** (`policy.rs`):
View = reads, Edit = writes, and each handler additionally checks the caller's
workspace role.

| # | Method & path | Auth | Body / query | Response |
|---|---|---|---|---|
| 115 | `GET /workspaces/{id}/proof-packs` | ws viewer · ProofPack View | `?status & work_item_kind & work_item_id` | `ProofPackResp[]` |
| 116 | `POST /workspaces/{id}/proof-packs` | ws editor · ProofPack Edit | `CreateProofPackReq {work_item_kind, work_item_id, title?, parent_pack_id?}` | `ProofPackResp` (ensure-or-create) |
| 117 | `GET /workspaces/{id}/proof-summary` | ws viewer · ProofPack View | — | `ProofSummaryResp {rows:[{work_item_kind, work_item_id, proof_pack_id, status, risk_score, badges[]}]}` |
| 118 | `GET /proof-packs/{id}` | ws viewer · ProofPack View | — | `ProofPackDetailResp {pack, badges[], artifacts[], children[]}` |
| 119 | `PATCH /proof-packs/{id}` | ws editor · ProofPack Edit | `{title?, summary?}` | `ProofPackResp` |
| 120 | `DELETE /proof-packs/{id}` | ws editor · ProofPack Edit | — | `{ok:true}` (cascades artifacts) |
| 121 | `POST /proof-packs/{id}/artifacts` | ws editor · ProofPack Edit | `AddArtifactReq {kind, title, content?, content_url?, status?, metadata?}` | `ProofPackResp` |
| 122 | `POST /proof-packs/{id}/assemble` | ws editor · ProofPack Edit | `AssembleReq {cwd?, base?, commands?:[{cmd, kind?}]}` | `ProofPackResp` |
| 123 | `POST /proof-packs/{id}/waive` | ws editor · ProofPack Edit | `WaiveReq {reason}` | `ProofPackResp` |
| 124 | `DELETE /proof-artifacts/{id}` | ws editor · ProofPack Edit | — | `{ok:true}` |
| 125 | `GET /proof-artifacts/{id}/content` | ws viewer · ProofPack View | — | `{content, ref_kind, kind, status, metadata}` (full stored content) |

All routes are under `/api/v1`.

### WebSocket event

`proof_pack_updated` (`/ws/events`, workspace-scoped) is emitted by
`otto_server::proof::recompute_and_emit` whenever a pack is created, (re)assembled,
gains an artifact, or is waived:

```json
{ "type": "proof_pack_updated", "workspace_id": "<Id>", "proof_pack_id": "<Id>",
  "work_item_kind": "session", "work_item_id": "<id>",
  "status": "passed", "risk_score": 12 }
```

(`proof_pack_exported` — a *review's* Proof Pack snapshot was persisted — belongs
to the Review Findings workflow; see [`./review-findings.md`](./review-findings.md).)

---

## 10. Configuration (env)

| Var | Default | Effect |
|---|---|---|
| `OTTO_PROOF_REQUIRE_PR` | **on** | Block PRs over an unproven linked pack (`0`/`false` disables) |
| `OTTO_PROOF_REQUIRE_GOAL_LOOP` | **off** | Hard-require a passing test before a goal loop finalizes "achieved" (`1`/`true` enables) |
| `OTTO_PROOF_AUTO_TEST` | **off** | Auto-run the repo's tests on the session all-done edge (`1`/`true` enables) |

---

## 11. Data model

`crates/otto-state/migrations/0078_proof_packs.sql`:

- **`proof_packs`** — one row per work item. ULID `id`, `workspace_id`,
  `work_item_kind`, `work_item_id`, `title`, `status` (default `missing`),
  `summary`, `risk_score` (default 0), `parent_pack_id`, `waived_by`,
  `waived_reason`, `created_by`, `created_at`, `updated_at`. A **`UNIQUE`** index
  on `(work_item_kind, work_item_id)` enforces one pack per work item; further
  indexes on `(workspace_id, status)` and `parent_pack_id`.
- **`proof_artifacts`** — `id`, `proof_pack_id` (`REFERENCES proof_packs(id) ON
  DELETE CASCADE`), `workspace_id`, `kind`, `title`, `content_ref`, `status`
  (default `info`), `metadata_json` (default `{}`), `created_by`, `created_at`,
  `updated_at`; indexed by `(proof_pack_id, created_at)`.

Repo: `otto_state::ProofRepo`. Engine + gates: `otto_server::proof`.

---

## 12. Capabilities & limitations

**Can:**

- Attach a one-per-work-item evidence pack to sessions, goal loops, reviews,
  workflow runs, tasks, and ad-hoc manual checks.
- Derive `status`, a `0..100` `risk_score`, and a badge set purely from the
  artifacts — recomputed on every mutation, never claimed by the agent.
- Keep `passed` non-gameable: a code change needs a real diff **and** a recognized
  passing test command (a green no-op like `true` won't do).
- Auto-package diffs (goal-loop + session), verify commands (goal-loop), review
  findings, workflow node/approval outputs; and accept agent/human-supplied
  commands, screenshots, api/db/ci evidence, self-review, and approvals over the
  API.
- **Hard-gate PR creation** on a linked pack (with an audited `allow_unproven`
  override) and **optionally** hard-gate goal-loop "achieved".
- Redact secrets and cap content before storage; roll child packs up under a
  parent (e.g. a goal loop).

**Limitations / honest caveats:**

- **No video capture**, and **no auto-capture** of API-Client requests or
  DB-Explorer queries into artifacts yet — the artifact API supports them, but the
  capture wiring is a follow-up.
- **No on-disk large-binary artifact store** — content lives inline (≤2 MiB,
  redacted/truncated) or as a URL/file reference.
- **The review fix→verify loop isn't fully closed** — re-running a review verifies
  a fix; there's no single automatic close.
- **The PR gate enforces only when the PR request links a pack** (`proof_pack_id`).
  A PR with no linked pack — or an unknown pack id — is not blocked.
- **Session auto-test is off by default** (it would run a suite in your live cwd);
  the diff is always packaged, but tests are opt-in.

---

## 13. Security & permissions

- **Feature + workspace RBAC.** Every route is `Feature::ProofPack`-gated (View for
  reads, Edit for writes) **and** re-checks the caller's workspace role. See
  [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md).
- **Redaction before storage.** A trust layer that itself leaked secrets would be
  worse than none, so artifact content is redacted (JWT/AWS/PEM/Bearer/email/
  sensitive-JSON-key) before it is persisted, with the hit count recorded.
- **Content cap.** Stored content is capped at 2 MiB; previews at 8 KiB. The full
  content is fetched through an explicit, RBAC-gated endpoint.
- **Waive is the only human status override**, and it is attributed (`waived_by` +
  `waived_reason`); the PR `allow_unproven` override is recorded as an audit
  `approval` artifact rather than silently bypassing the gate.
- **Loopback by default.** Like the rest of the daemon, these endpoints are only
  reachable over loopback unless you explicitly enable remote access.

---

## 14. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| Pack stuck at `partial` after a session | A `CodeChange` kind needs a diff **and** a passing *recognized* test command. Add a `command` artifact whose title is a real runner (§3.1), or `/assemble` with `commands`. A passing `true` won't satisfy it. |
| `passed` but you expected `failed` | No artifact is `failed`. A neutral diff/log is `info`, not `failed`; mark a `db`/`api`/`command` artifact `failed` explicitly to flip the pack. |
| PR creation returns `409 ... not passed` | The linked pack isn't `passed`/`waived`. Add evidence until it passes, **Waive** it, or re-open with `allow_unproven: true` (recorded as an audit artifact). Or disable globally with `OTTO_PROOF_REQUIRE_PR=0`. |
| Session all-done didn't run tests | Expected — auto-test is off. Set `OTTO_PROOF_AUTO_TEST=1`, or add the test `command` via `/assemble`. The diff is always packaged. |
| Goal loop finalized "achieved" with no test | The hard block is opt-in — set `OTTO_PROOF_REQUIRE_GOAL_LOOP=1` (bounded by the iteration cap). |
| `risky_change` badge on a benign change | A risky **path** was touched (`migrations/`, `.github/`, a lockfile, `*.sql`, or a security-word basename) **or** `risk_score ≥ 50`. Inspect the diff artifact's `risky_files`. |
| Artifact content shows `[redacted]` | The redactor matched a JWT/AWS-key/PEM/Bearer/email/sensitive-key shape; `metadata.redactions` is the hit count. This is by design. |
| Artifact ends `…(truncated)` | Content exceeded the 2 MiB store cap. The preview is 8 KiB; "Load full" returns the (still capped) stored content. |
| No `proof_pack_updated` event | The pack didn't change, or you're not subscribed to the workspace event stream. The UI also re-fetches on focus. |

---

## 15. Related docs

- [`./code-review.md`](./code-review.md) — the multi-agent AI review that feeds a
  `review` pack (and the persisted findings).
- [`./review-findings.md`](./review-findings.md) — the tracked review-findings
  workflow, its own (namespaced) review proof pack, and memory ingest.
- [`./goal-loops.md`](./goal-loops.md) — the bounded Plan→Execute→Evaluate loop
  whose verify commands + diff become proof, with the opt-in hard gate.
- [`./workflows.md`](./workflows.md) — node/approval outputs become workflow
  artifacts.
- [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md) — the `ProofPack` feature roles.
- `docs/contracts/api.md` (#115–125), `docs/contracts/ws.md` — authoritative
  endpoint + WS contract.
</content>
</invoke>
