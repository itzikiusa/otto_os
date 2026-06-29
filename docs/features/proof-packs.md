# Proof Packs (the trust layer)

> An agent may not declare a task "done" on assertion alone. Every meaningful unit
> of agent work carries a **Proof Pack** — a bundle of inspectable evidence whose
> **status is derived from the evidence, not claimed by the agent.** Otto assembles
> what it can automatically; agents and humans add the rest through a small API.
> **Proof Packs v2 — "the trust layer"** layers on tamper-evident snapshots, CI
> status, screenshot/video/API/DB/Kafka evidence, a PR-description consistency
> check, an explainable "done contract" readiness score, per-repo policy, an
> exportable report, and an accountable human waiver.

This document is the authoritative end-user + operator reference for Proof Packs.
Where it describes wire-level behaviour, the source of truth is the contract files
in `docs/contracts/` (`api.md` #115–137, `ws.md` `proof_pack_updated` /
`proof_pack_exported`). The **pure** domain logic lives in
`crates/otto-core/src/proof.rs`; persistence in `otto_state::proof`
(`ProofRepo`); the assembly engine + gates in `crates/otto-server/src/proof.rs`;
the REST surface in `crates/otto-server/src/routes/proof.rs`; and the UI in
`ui/src/modules/proof/ProofPage.svelte`.

> **v2 is additive and never weakens v1.** A repo with no config behaves exactly
> as v1 did; every new pack column has a default; per-repo policy can only
> *strengthen* (never relax) the requirement. The regression invariant is encoded
> in `proof.rs` (`policy_default_equals_legacy`): with the default policy,
> `derive_status_with_policy == derive_status` for every fixture.

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

Status, `risk_score` (0–100), the `done_score` (0–100), and the badge set are
computed by **pure functions in `otto-core::proof`** — `derive_status`,
`compute_risk`, `compute_badges`, `compute_done_contract` — the single source of
truth, recomputed on **every** mutation. The agent never sets the status; it can
only attach evidence.

`derive_status` is short and deliberate:

1. `waived_by` set → **`waived`** (short-circuits everything).
2. no artifacts → **`missing`**.
3. any artifact `failed` → **`failed`**.
4. the work-item kind's required set is met → **`passed`**, else **`partial`**.

**Policy capping (v2).** `derive_status_with_policy(pack, arts, policy)` runs
`derive_status` and then — *only* if the result would be `passed` — caps it to
`partial` when a repo-opted-in extra requirement lacks a passing artifact (see
§7). It never relaxes `failed`/`missing`. The engine always recomputes through
the policy-aware path, so a repo's "require a green CI / passing test / consistent
PR / resolved review" rules are enforced as a hard ceiling on `passed`.

### The "work item" + parent rollup

A pack carries `work_item_kind`, `work_item_id`, a `title`, a `summary`, the
derived `status`, `risk_score`, and `done_score`, an optional `parent_pack_id` (a
goal-loop pack parents the session/review/workflow packs it spawns), an optional
`repo_id` and `pr_number` (set once the work is repo-/PR-linked, driving per-repo
policy and CI/report lookups), and the waiver fields (`waived_by`,
`waived_reason`, `waived_at`). The unique `(work_item_kind, work_item_id)` index
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

These are the **kind-intrinsic** requirements. A repo can layer additional
strengthen-only requirements on top via its proof policy (§7) — those cap a
`passed` down to `partial`, they never relax these.

---

## 3. Artifacts

An artifact is one piece of evidence inside a pack:

```
ProofArtifact { id, proof_pack_id, workspace_id, kind, title,
                content_ref, status, metadata, content_sha256,
                created_by, created_at, updated_at }
```

**Kinds** (`ProofArtifactKind`):
`command · log · screenshot · video · diff · ci · api · db · kafka · review ·
approval · pr_check · self_review`. The v2 additions are `video` (screencast of a
working UI), `kafka` (consumed-message read evidence), and `pr_check`
(PR-description consistency result). `screenshot`/`video` are **media** kinds —
their content is a binary **blob**, not inline text (`kind.is_media()`).

**Statuses** (`ProofArtifactStatus`): `passed · failed · pending · info` —
`info` is neutral evidence (a diff, a logged sample) that is neither pass nor
fail and is the default when none is supplied.

`content_ref` holds inline text (capped, see §10), a URL, a `blob:<id>` reference
(media), or nothing; the flavour is recorded in
`metadata.ref_kind ∈ inline | url | blob | none`. The engine also stamps
`metadata` with fields like `redactions`, `truncated`, `test_kind`
(`test`/`build`/`lint`), `exit_code`, `duration_ms`, `evidence`
(`ci`/`api`/`db`/`kafka`/`pr_check`), `mime`/`size_bytes`/`sha256` (media), and —
for a diff — `files_changed`, `additions`, `deletions`, `risky_files`.

**`content_sha256` (v2 tamper-evidence).** Every artifact with inline content
records the SHA-256 (hex) of its **full** stored content at write time. It is the
chip shown next to the artifact in the UI, it is preserved into snapshots (which
copy only a capped preview), and it survives even when the snapshot truncates the
content — so a snapshot stays small while remaining verifiable. It is `None` for
url/blob/none refs.

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
| `ci_passed` | a `ci` artifact `passed` (and none failed/pending) |
| `ci_failed` | a `ci` artifact `failed` |
| `ci_pending` | a `ci` artifact `pending` (CI still running) |
| `db_api_verified` | a `db`/`api`/`kafka` artifact explicitly marked `passed` (not the default `info`) |
| `ui_verified` | a `screenshot`/`video` artifact present (any non-failed status) |
| `pr_inconsistent` | a `pr_check` artifact `failed` (PR text inconsistent with the change) |
| `review_unresolved` | a `review` artifact `failed` |
| `waived` | the pack was waived |

The CI badges follow how a human reads a check-run summary: **failed wins, then
pending, then passed.** `kafka` joins the `db`/`api` "data verified" family.

---

## 6. The "done contract" (explainable readiness, 0..100)

Beyond pass/fail, every pack carries a **done contract**: a weighted, *itemized*,
*explainable* readiness score. `compute_done_contract(pack, arts, policy)` returns
a `DoneContract { score, satisfied, required, items[] }` where each `ContractItem`
has a `key`, `label`, `required`, `satisfied`, `weight`, and a human `detail`
string. The score is the **single source of truth for "how ready is this, and
what's missing"**, deterministic and pure.

| Item | Weight | Required when |
|---|---|---|
| `diff` — Code diff captured | 15 | a code-change kind |
| `tests` — Tests passed | 25 | a code-change kind **or** repo `require_test` |
| `no_failures` — No failed evidence | 20 | always |
| `ci` — CI green | 10 | repo `require_ci` |
| `review` — Review resolved | 10 | a `review` kind **or** repo `require_review` |
| `pr_consistency` — PR matches change | 10 | repo `require_pr_consistency` |
| `ui_evidence` — UI screenshot/video | 5 | optional |
| `data_evidence` — API/DB/Kafka verified | 5 | optional |
| `self_review` — Agent self-review | 5 | optional |
| `human_approval` — Human approved | 5 | optional |

`score = round(satisfied required weight ÷ total required weight × 100)`. A
**waived** pack scores **100**. `no_failures` (weight 20) is required for every
kind, so the denominator is always ≥ 20. The score is **persisted** on the pack
(`done_score`) for cheap list/summary sorting, but the **detail view recomputes
the full contract live** (`live_contract`) so the meter is accurate even for packs
created before v2 (whose persisted score may be stale). A pack at/above
`DONE_READY_THRESHOLD` (**80**) earns the "release-ready" treatment in the UI —
this is presentational; the **gate** uses `status`, not the score.

The UI surfaces this as a `DoneContractMeter` in pack detail (score + the
itemized checklist).

---

## 7. Per-repo proof policy ("test command required")

A repository can **strengthen** (never relax) what its work needs to be `passed`,
stored as JSON on `repos.proof_config_json` (`RepoProofConfig`). Every flag
defaults to `false`, so a repo with no config behaves exactly as v1.

| Field | Effect |
|---|---|
| `require_test` | a passing recognized test command is required |
| `test_cmd` | the repo's canonical test command (used by session auto-test, §11.5) |
| `require_ci` | a green `ci` artifact is required |
| `require_pr_consistency` | a passing `pr_check` artifact is required |
| `require_review` | a resolved `review` artifact is required |

`policy_for_pack` resolves the policy: the work-item-kind defaults (which
reproduce the legacy status) `.with_repo(cfg)`-strengthened by the linked repo's
opt-ins. The lookup is **best-effort** — an unknown/unlinked repo yields the
default policy, so a pack never gets *weaker* than v1 and a lookup failure can't
wedge recompute. `derive_status_with_policy` then caps a would-be `passed` to
`partial` whenever an opted-in extra requirement lacks a passing artifact (a
*failing* required artifact already makes the legacy status `failed`). The same
policy expands the **done contract**'s required-item set (§6).

A pack is linked to a repo two ways: explicitly at creation
(`CreateProofPackReq.repo_id`, #116) or via `repos/{id}/proof-config` GET/PUT
(#137), and implicitly by `gate_session` (longest-prefix match of the session cwd
against registered repos) and `after_pr_created` / `ci-refresh`. Linking is
**strengthen-only and best-effort** — an unresolvable repo just leaves the pack
unlinked.

---

## 8. Evidence capture (CI, UI, API, DB, Kafka, PR-check)

All capture paths flow through the **one redaction + cap boundary**
(`prepare_content` → `otto_core::redact`, §10) and recompute-and-emit afterwards.
Each has a dedicated, RBAC-gated endpoint (§13) and a pure status mapper in
`otto-core::proof` so the policy is testable and non-gameable.

### 8.1 CI status

`record_ci_artifact` upserts a `ci` artifact from a `CiSummary
{state, total, passed, failed, url}`. `ci_artifact_status(state)` maps the
provider's aggregate state → artifact status:

- `success`/`passed`/`passing`/`ok`/`green` → **passed**
- `failure`/`failed`/`failing`/`error`/`red`/`canceled`/`cancelled` → **failed**
- `pending`/`running`/`in_progress`/`queued`/`expected`/`waiting` → **pending**
- `none`/unknown → **info** (neutral)

CI is **auto-captured on PR creation** (`after_pr_created` in
`crates/otto-server/src/modules.rs`, which also stamps `repo_id`/`pr_number`), and
re-fetchable on demand via **`POST /ci-refresh`** (#135) — which resolves the git
provider for the linked repo/PR, fetches live CI, persists the link, and records
the artifact. The CI badges (`ci_passed`/`ci_failed`/`ci_pending`) and the
`ci_missing` badge (diff present, no CI yet) make CI visible at a glance.

### 8.2 UI evidence — screenshots & video

`attach_media` stores a binary **blob** (`proof_blobs`, content-addressed by
sha256) and creates the owning `screenshot`/`video` artifact (`ref_kind=blob`,
`content_ref=blob:<id>`, status `info`). The endpoint (**`POST /media`**, #129)
takes base64 (`AttachMediaReq {kind, title, mime, data_base64, metadata?}`) and
enforces:

- **MIME allow-list** (`ALLOWED_MEDIA_MIMES`): `image/png`, `image/jpeg`,
  `image/gif`, `image/webp`, `image/svg+xml`, `video/mp4`, `video/webm`. Anything
  else → **`415 Unsupported Media`**.
- **Size cap** `MEDIA_CAP` = **25 MiB**. A larger decoded blob → **`413 Payload
  Too Large`**. Empty/invalid base64 → `400`.

Any non-failed media artifact earns the `ui_verified` badge. The raw bytes are
served back inline via **`GET /proof-artifacts/{id}/blob`** (#130), and the UI
renders the image/`<video>` directly in pack detail.

### 8.3 API request/response evidence

`attach_api_evidence` records an HTTP request/response as an `api` artifact
(**`POST /evidence/api`**, #131, `ApiEvidenceReq {title, method, url, status,
duration_ms?, request?, response?, metadata?}`). `http_evidence_status(code)`:
`0` (no response / network error) → **failed**; `2xx` → **passed**; `≥400` →
**failed**; `1xx`/`3xx` → **info**. An explicitly `passed` `api` artifact
contributes to `db_api_verified`.

### 8.4 DB / Kafka read evidence

- `attach_db_evidence` → `db` artifact (**`POST /evidence/db`**, #132,
  `{title, engine?, query?, columns?, row_count?, sample?, error?, metadata?}`).
- `attach_kafka_evidence` → `kafka` artifact (**`POST /evidence/kafka`**, #133,
  `{title, topic, message_count?, sample?, truncated?, error?, metadata?}`).

`read_evidence_status(has_error)` → an error fails it, otherwise verified. For
Kafka the nuance is honest: an **error** → `failed`; **messages present** →
`passed`; an **empty topic** → `info` (neutral — reading a topic that had nothing
isn't a failure, but it isn't positive proof either). A `passed` `db`/`kafka`
artifact contributes to `db_api_verified`.

### 8.5 PR description consistency check

`check_pr_consistency` runs a deterministic, weighted check that the PR's prose
matches the actual change — catching a misaligned (or dishonest) description
before the PR lands. Five checks:

| Check | Weight | Passes when |
|---|---|---|
| Description is substantive | 20 | ≥ 40 chars |
| Title is sane | 10 | 8–140 chars |
| Mentions the actual change | 25 | references a changed file/dir token |
| Has testing notes for a code change | 15 | mentions test/verif/ci/coverage when code changed |
| **No false "tests pass" claim** (HARD) | 30 | doesn't claim tests pass while they fail/are absent |

`passed = !hard_fail && score ≥ PR_CONSISTENCY_THRESHOLD (70)`; `hard_fail` is set
when the description claims "tests pass" / "all green" / "CI passes" etc. while
the pack's test evidence is failing or absent. `pr_check_artifact_status` then
maps the report to the recorded `pr_check` artifact status — the nuance that keeps
an **auto-run-on-every-PR** check non-disruptive:

- **hard fail** (a *dishonest* PR) → **failed** → fails the pack, earns
  `pr_inconsistent`.
- **passed** → **passed** → positive evidence that can satisfy a repo's
  `require_pr_consistency`.
- otherwise (an **honest-but-thin** description) → **info** → neutral; it neither
  fails an otherwise-green pack nor satisfies a *required* consistency check.

`run_pr_check` redacts title + description first (one trust boundary; the
heuristics still work on redacted text) and derives `files_changed`/LOC from the
actual `base..HEAD` diff. It runs **automatically on PR creation**
(`after_pr_created`, with `base = target_branch`) and is re-runnable on demand via
**`POST /pr-check`** (#134, `{title, description, base?, cwd?}`). The UI renders
the per-check breakdown under the `pr_check` artifact.

---

## 9. Immutable snapshots + exportable report

### 9.1 Snapshots (tamper-evident, append-only)

`make_snapshot` freezes a pack's current evidence into an **immutable**
`proof_snapshots` row (append-only; only ever removed by the cascade when the pack
is deleted). Each snapshot captures:

- a frozen `bundle_json` — `{pack, badges, done_contract, artifacts, generated_at}`
  with each artifact's content **capped to `SNAPSHOT_ARTIFACT_CAP` (64 KiB)** but
  its full **`content_sha256` preserved**, so the snapshot stays small while
  remaining verifiable;
- a **`sha256`** — `bundle_sha256` over the *canonical* (key-order-independent)
  bundle, the snapshot's tamper-evidence key;
- the pack's `status`, `done_score`, `risk_score`, a per-pack `seq` (1, 2, 3…),
  an optional `note`, and the creating principal;
- **frozen rendered reports** — `report_md` and `report_html` are rendered at
  snapshot time and stored, so the report reflects the evidence *as it was*.

Create with **`POST /snapshot`** (#126, `{note?}` → `ProofSnapshotResp {meta,
bundle, report_md, report_html}`); list with **`GET /snapshots`** (#127, newest
first); fetch one with **`GET /proof-snapshots/{id}`** (#128). The detail response
(#118) also includes the snapshot metadata list. The UI lists snapshots with
per-snapshot report download.

### 9.2 Exportable report (Markdown / HTML)

`render_report_md` / `render_report_html` produce a **self-contained** evidence
report (the HTML has inline CSS and no external assets, and escapes content). It
includes the status, done-contract score + checklist, risk, badges, the waiver
note (if any), and each artifact with its `sha256` and a capped preview (media is
referenced, not embedded). Fetch the **live** report via **`GET /report?format=md|html`**
(#136, `text/markdown` or `text/html`; default Markdown). The **same renderer**
freezes the per-snapshot reports above.

---

## 10. Evidence integrity (redaction + cap + hashing + preview)

A trust layer must not itself leak secrets, so **all persisted artifact text is
redacted before storage**. `prepare_content` runs `otto_core::redact` over the
text first; `redact` strips a small set of high-confidence shapes — **JWTs**, **AWS
access keys**, **PEM blocks**, **`Bearer` tokens**, **emails**, and values under
**sensitive JSON keys** — replacing each with `[redacted]`. The number of hits is
recorded in `metadata.redactions`. This boundary is shared by every auto path
(goal loop / review / workflow / session) and every artifact/evidence endpoint —
including `pr_check`, which redacts the title/description before the heuristics
run.

| Cap | Value | Applies to |
|---|---|---|
| `STORE_CAP` | **2 MiB** | inline artifact content stored in full; larger ⇒ truncated at a char boundary with a trailing `…(truncated)` note + `metadata.truncated = true` |
| `PREVIEW_CAP` | **8 KiB** | the inline preview returned in list/detail responses |
| `MEDIA_CAP` | **25 MiB** | a single media blob (screenshot/video); larger ⇒ `413` |
| `SNAPSHOT_ARTIFACT_CAP` | **64 KiB** | per-artifact content copied into a snapshot (full `content_sha256` still embedded) |

The full stored content is fetched on demand via **`GET /proof-artifacts/{id}/content`**
(#125). **Hashing**: `content_sha256(text)` stamps each inline artifact (§3);
`bytes_sha256(bytes)` hashes media blobs; `bundle_sha256(value)` is the stable,
key-order-independent hash that makes a snapshot tamper-evident (§9).

---

## 11. Enforcement — "no done without evidence"

Otto packages a pack and derives a status whenever a work item reaches "done"; the
status and badges are the visible, non-bypassable signal. There are **five**
integration points. Two add **hard teeth**; three **package** evidence and surface
badges without a hard block.

### 11.1 Goal Loops — machine-checked (hard teeth, opt-in)

A goal loop already ground-truths declared `verify_kind=command` acceptance
criteria (an "achieved" verdict with an unmet command criterion is coerced to
"continue"). On success it **packages** each verify command (with captured
output) as a `command` artifact, the worktree diff as a `diff`, the evaluator's
feedback/rationale as a `self_review`, and a criteria summary as a `review`.

With **`OTTO_PROOF_REQUIRE_GOAL_LOOP=1`** the loop additionally **refuses to
finalize "achieved"** without a passing machine-checked test — bounded by the
iteration cap, accepting a `partial` pack on the last iteration so a genuine goal
is never failed purely for lack of a command. Default **off**.

### 11.2 PR creation — hard `409` gate (default ON) + auto evidence

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

**After** a PR is created (`after_pr_created`), Otto — best-effort, never
surfacing failures to the PR caller — stamps `repo_id`/`pr_number` on the linked
pack, **auto-captures CI** (§8.1) and **auto-runs the PR-consistency check** (§8.5,
`base = target_branch`). A dishonest description flips the pack to `failed`; an
honest-but-thin one is neutral.

### 11.3 AI review → review pack (package, lifecycle-driven)

Each completed AI review (see [`./code-review.md`](./code-review.md)) upserts its
summarized findings into the persistent `review_findings` store (fingerprinted),
and the review's pack gets a `review` artifact — `failed` while findings are
unresolved (which drives the `review_unresolved` badge), `passed` when clean.
Fix-attempt linkage is set via the finding-state endpoints — semi-automatic, not a
fully closed loop. The full tracked-finding workflow is documented in
[`./review-findings.md`](./review-findings.md).

### 11.4 Workflows — node + approval evidence (package)

On run completion each node's output becomes a `log` artifact (status from the
node status), each `human_approval` node an `approval` artifact (passed iff
approved, with approver/note/timestamp — drives `human_approved`), and the budget
gate a `log`.

### 11.5 Sessions — the all-done edge (package; tests opt-in)

When an agent marks every task complete, `gate_session` packages a pack: it links
the pack to a registered repo (longest-prefix match of the session cwd) so the
repo's proof policy applies on recompute, assembles the working-tree **diff
always**, and runs the repo's **tests opt-in** via **`OTTO_PROOF_AUTO_TEST=1`**
(running a repo's suite in the user's live cwd is disruptive, so default off).
When auto-test is on it prefers the repo's configured `test_cmd`, falling back to
a detected runner (`cargo test` / `go test ./...` / `npm test` if a `test` script
exists). Goal-loop-spawned session packs are linked to the loop's pack via
`parent_pack_id`. If the resulting proof is incomplete, Otto surfaces a one-shot
Notice ("Tasks done — proof incomplete").

### What's auto vs. what an agent/human supplies (honest)

| Proof item | Auto | Via API |
|---|---|---|
| Diff summary | ✅ goal-loop + session | ✅ `/assemble {cwd}` |
| Test commands + output | ✅ goal-loop (verify cmds); session (opt-in) | ✅ `/assemble {commands}` or a `command` artifact |
| Build / lint | partial (classified from the command) | ✅ `command` artifact |
| CI status | ✅ on PR creation; `/ci-refresh` re-fetches | ✅ a `ci` artifact |
| PR consistency | ✅ on PR creation | ✅ `/pr-check` |
| Screenshots / video | — | ✅ `/media` (blob) |
| API evidence | — | ✅ `/evidence/api` |
| DB / Kafka evidence | — | ✅ `/evidence/db`, `/evidence/kafka` |
| Review findings | ✅ review gate | — |
| Agent self-review | ✅ goal-loop (evaluator) | ✅ `self_review` artifact |
| Human approval | ✅ workflow approval node + PR override | ✅ `approval` artifact / waiver |

The evidence endpoints exist for **all** of the above; the API-Client and
DB-Explorer do not yet auto-push their requests/queries into a pack — that capture
wiring is a follow-up, so today it's a manual `/evidence/*` call.

---

## 12. The Proof module (UI)

`ui/src/modules/proof/ProofPage.svelte` is a two-pane viewer:

- **Left rail** — a status filter (`all · passed · failed · partial · missing ·
  waived`) over the workspace's packs, each row showing its title (or
  `work_item_id`), a `ProofStatusChip` (status + risk), the `work_item_kind` tag,
  and its `ProofBadges`. A **`+`** creates a `manual` pack (a random `work_item_id`)
  for ad-hoc verification ("Release 1.4 verification").
- **Right detail** — the open pack's `DoneContractMeter` (score + checklist),
  badges + summary, the PR-link tag (`PR #<n>`), the waiver note (with `waived_at`),
  the **snapshot list** (with per-snapshot report download), and its **artifacts
  grouped by kind**. Each artifact shows a status dot, its title, a
  `sha:<8 hex>…` chip, the status label; media (`screenshot`/`video`) render
  inline; a `pr_check` artifact renders its per-check breakdown; previews are
  expandable and **"Load full"** pulls the uncapped content.
- **Pack actions** — **Assemble** (diff + commands), **Add artifact**,
  **Add media** (screenshot/video, ≤25 MiB), **Add evidence** (api/db/kafka),
  **PR check**, **Refresh CI** (when repo-linked), **Requirements** (edit the
  repo's `RepoProofConfig`), **Export .md / .html**, **Waive** (prompts for a
  reason — min 10 chars), and **Delete** (cascades artifacts, snapshots, blobs).

Live updates arrive over the `proof_pack_updated` WS event (the store re-fetches
the affected pack and refreshes the workspace summary). The cheap
`GET /proof-summary` roll-up (now carrying `done_score` per row) powers compact
status chips elsewhere in the app (e.g. alongside a session) without loading every
pack.

---

## 13. API & contract reference

Authoritative: `docs/contracts/api.md` **#115–137** and `docs/contracts/ws.md`
(`proof_pack_updated`). All routes are **`Feature::ProofPack`-gated** (`policy.rs`):
View = reads, Edit = writes, and each handler additionally checks the caller's
workspace role.

| # | Method & path | Auth | Body / query | Response |
|---|---|---|---|---|
| 115 | `GET /workspaces/{id}/proof-packs` | ws viewer · ProofPack View | `?status & work_item_kind & work_item_id` | `ProofPackResp[]` |
| 116 | `POST /workspaces/{id}/proof-packs` | ws editor · ProofPack Edit | `CreateProofPackReq {work_item_kind, work_item_id, title?, parent_pack_id?, repo_id?}` | `ProofPackResp` (ensure-or-create; `repo_id` links policy, strengthen-only) |
| 117 | `GET /workspaces/{id}/proof-summary` | ws viewer · ProofPack View | — | `ProofSummaryResp {rows:[{work_item_kind, work_item_id, proof_pack_id, status, risk_score, done_score, badges[]}]}` |
| 118 | `GET /proof-packs/{id}` | ws viewer · ProofPack View | — | `ProofPackDetailResp {pack, badges[], artifacts[], children[], done_contract, snapshots[]}` (done_contract live) |
| 119 | `PATCH /proof-packs/{id}` | ws editor · ProofPack Edit | `{title?, summary?}` | `ProofPackResp` |
| 120 | `DELETE /proof-packs/{id}` | ws editor · ProofPack Edit | — | `{ok:true}` (cascades artifacts, snapshots, blobs) |
| 121 | `POST /proof-packs/{id}/artifacts` | ws editor · ProofPack Edit | `AddArtifactReq {kind, title, content?, content_url?, status?, metadata?}` | `ProofPackResp` |
| 122 | `POST /proof-packs/{id}/assemble` | ws editor · ProofPack Edit | `AssembleReq {cwd?, base?, commands?:[{cmd, kind?}]}` | `ProofPackResp` |
| 123 | `POST /proof-packs/{id}/waive` | ws editor (or Admin if `OTTO_PROOF_WAIVER_MIN_ROLE=admin`) · ProofPack Edit | `WaiveReq {reason}` (≥10 chars) | `ProofPackResp` |
| 124 | `DELETE /proof-artifacts/{id}` | ws editor · ProofPack Edit | — | `{ok:true}` |
| 125 | `GET /proof-artifacts/{id}/content` | ws viewer · ProofPack View | — | `{content, ref_kind, kind, status, metadata}` (full stored content) |
| 126 | `POST /proof-packs/{id}/snapshot` | ws editor · ProofPack Edit | `CreateSnapshotReq {note?}` | `ProofSnapshotResp {…meta, bundle, report_md, report_html}` (immutable) |
| 127 | `GET /proof-packs/{id}/snapshots` | ws viewer · ProofPack View | — | `ProofSnapshotMeta[]` (newest first) |
| 128 | `GET /proof-snapshots/{id}` | ws viewer · ProofPack View | — | `ProofSnapshotResp` |
| 129 | `POST /proof-packs/{id}/media` | ws editor · ProofPack Edit | `AttachMediaReq {kind:screenshot\|video, title, mime, data_base64, metadata?}` (≤25 MiB) | `ProofPackResp` — `415` bad mime, `413` oversize |
| 130 | `GET /proof-artifacts/{id}/blob` | ws viewer · ProofPack View | — | raw bytes (`Content-Type` = blob mime, `Content-Disposition: inline`) |
| 131 | `POST /proof-packs/{id}/evidence/api` | ws editor · ProofPack Edit | `ApiEvidenceReq {title, method, url, status, duration_ms?, request?, response?, metadata?}` | `ProofPackResp` |
| 132 | `POST /proof-packs/{id}/evidence/db` | ws editor · ProofPack Edit | `DbEvidenceReq {title, engine?, query?, columns?, row_count?, sample?, error?, metadata?}` | `ProofPackResp` |
| 133 | `POST /proof-packs/{id}/evidence/kafka` | ws editor · ProofPack Edit | `KafkaEvidenceReq {title, topic, message_count?, sample?, truncated?, error?, metadata?}` | `ProofPackResp` |
| 134 | `POST /proof-packs/{id}/pr-check` | ws editor · ProofPack Edit | `PrCheckReq {title, description, base?, cwd?}` | `ProofPackResp` (stores a `pr_check` artifact) |
| 135 | `POST /proof-packs/{id}/ci-refresh` | ws editor · ProofPack Edit | `CiRefreshReq {repo_id?, pr_number?}` (default from pack) | `ProofPackResp` (fetches live CI → `ci` artifact) |
| 136 | `GET /proof-packs/{id}/report` | ws viewer · ProofPack View | `?format=md\|html` | rendered report (`text/markdown` or `text/html`) |
| 137 | `GET\|PUT /repos/{id}/proof-config` | ws viewer (GET) / editor (PUT) · ProofPack View/Edit | `RepoProofConfig {require_test?, test_cmd?, require_ci?, require_pr_consistency?, require_review?}` | `RepoProofConfigResp {repo_id, config}` |

All routes are under `/api/v1`.

### WebSocket event

`proof_pack_updated` (`/ws/events`, workspace-scoped) is emitted by
`otto_server::proof::recompute_and_emit` whenever a pack is created, (re)assembled,
gains an artifact, is waived, or captures CI/evidence:

```json
{ "type": "proof_pack_updated", "workspace_id": "<Id>", "proof_pack_id": "<Id>",
  "work_item_kind": "session", "work_item_id": "<id>",
  "status": "passed", "risk_score": 12, "done_score": 85 }
```

`done_score` (0..100, the done-contract readiness) was added in v2.

(`proof_pack_exported` — a *review's* Proof Pack snapshot was persisted — belongs
to the Review Findings workflow; see [`./review-findings.md`](./review-findings.md).)

---

## 14. Configuration (env)

| Var | Default | Effect |
|---|---|---|
| `OTTO_PROOF_REQUIRE_PR` | **on** | Block PRs over an unproven linked pack (`0`/`false` disables) |
| `OTTO_PROOF_REQUIRE_GOAL_LOOP` | **off** | Hard-require a passing test before a goal loop finalizes "achieved" (`1`/`true` enables) |
| `OTTO_PROOF_AUTO_TEST` | **off** | Auto-run the repo's tests on the session all-done edge (`1`/`true` enables) |
| `OTTO_PROOF_WAIVER_MIN_ROLE` | `edit` | Set to `admin` to require workspace **Admin** to waive (closes the service-principal self-waive path) |

Per-repo strengthening (`require_test`/`require_ci`/`require_pr_consistency`/
`require_review`/`test_cmd`) is configured per repository via `RepoProofConfig`
(#137), not an env var.

---

## 15. Data model

Base schema: `crates/otto-state/migrations/0078_proof_packs.sql`. The v2 additions
(`crates/otto-state/migrations/0085_proof_packs_v2.sql`) are **additive only** —
new columns (all defaulted/nullable) + two new tables — so existing v1 rows keep
working.

- **`proof_packs`** — one row per work item. ULID `id`, `workspace_id`,
  `work_item_kind`, `work_item_id`, `title`, `status` (default `missing`),
  `summary`, `risk_score` (default 0), `parent_pack_id`, `waived_by`,
  `waived_reason`, `created_by`, `created_at`, `updated_at`. **v2 columns:**
  `done_score` (INTEGER, default 0), `repo_id` (TEXT), `pr_number` (INTEGER),
  `waived_at` (TEXT, RFC3339). A **`UNIQUE`** index on
  `(work_item_kind, work_item_id)` enforces one pack per work item; further
  indexes on `(workspace_id, status)` and `parent_pack_id`.
- **`proof_artifacts`** — `id`, `proof_pack_id` (`REFERENCES proof_packs(id) ON
  DELETE CASCADE`), `workspace_id`, `kind`, `title`, `content_ref`, `status`
  (default `info`), `metadata_json` (default `{}`), `created_by`, `created_at`,
  `updated_at`; indexed by `(proof_pack_id, created_at)`. **v2 column:**
  `content_sha256` (TEXT — SHA-256 of the full stored content).
- **`repos`** — **v2 column:** `proof_config_json` (TEXT, NOT NULL, default `{}`)
  — the `RepoProofConfig`.
- **`proof_snapshots`** (v2, append-only) — `id`, `proof_pack_id` (`ON DELETE
  CASCADE`), `workspace_id`, `seq` (1,2,3… per pack), `sha256` (canonical-bundle
  hash), `status`, `done_score`, `risk_score`, `bundle_json` (frozen
  pack+capped-artifacts+contract+badges), `report_md`, `report_html`, `note`,
  `created_by`, `created_at`. Index on `(proof_pack_id, seq)`. Never UPDATEd or
  DELETEd except the cascade when the owning pack is removed.
- **`proof_blobs`** (v2) — media bytes for `screenshot`/`video` artifacts: `id`,
  `artifact_id` (`ON DELETE CASCADE`), `workspace_id`, `sha256`, `mime`,
  `size_bytes`, `data` (BLOB), `created_at`. Index on `artifact_id`. Capped at
  `MEDIA_CAP` (25 MiB) by the engine.

Repo: `otto_state::ProofRepo`. Engine + gates: `otto_server::proof`.

---

## 16. Capabilities & limitations

**Can:**

- Attach a one-per-work-item evidence pack to sessions, goal loops, reviews,
  workflow runs, tasks, and ad-hoc manual checks.
- Derive `status`, a `0..100` `risk_score`, a `0..100` `done_score` (the
  itemized, explainable done contract), and a badge set purely from the
  artifacts — recomputed on every mutation, never claimed by the agent.
- Keep `passed` non-gameable: a code change needs a real diff **and** a recognized
  passing test command (a green no-op like `true` won't do).
- **Strengthen per repo** (`require_test`/`require_ci`/`require_pr_consistency`/
  `require_review`) — capping a would-be `passed` to `partial` until met, never
  weakening v1.
- Auto-package diffs (goal-loop + session), verify commands (goal-loop), review
  findings, workflow node/approval outputs, **CI status and a PR-consistency check
  on PR creation**; and accept agent/human-supplied commands, **screenshots/video
  (≤25 MiB blobs)**, **api/db/kafka reads**, self-review, and approvals over the
  API.
- **Hard-gate PR creation** on a linked pack (with an audited `allow_unproven`
  override) and **optionally** hard-gate goal-loop "achieved".
- Freeze **immutable, content-hashed snapshots** and export a self-contained
  **Markdown/HTML report**.
- Redact secrets and cap content before storage; hash artifacts + bundles for
  tamper-evidence; roll child packs up under a parent (e.g. a goal loop).
- Record an **accountable human waiver** (the authenticated approver + reason +
  timestamp + an immutable approval artifact), optionally Admin-gated.

**Limitations / honest caveats:**

- **No auto-capture from the API-Client or DB-Explorer yet** — the
  `/evidence/api`, `/evidence/db`, `/evidence/kafka` and `/media` endpoints exist
  and the UI can push evidence, but those tools don't yet auto-attach their
  requests/queries/screenshots to a pack; that capture wiring is a follow-up.
- **No on-disk large-binary artifact store** — text lives inline (≤2 MiB,
  redacted/truncated) or as a URL/file ref; media lives as a DB blob capped at
  **25 MiB** (`proof_blobs`), not a file/object store, so very large screencasts
  must be trimmed or referenced by URL.
- **The review fix→verify loop isn't fully closed** — re-running a review verifies
  a fix; there's no single automatic close.
- **The PR gate enforces only when the PR request links a pack** (`proof_pack_id`).
  A PR with no linked pack — or an unknown pack id — is not blocked. The CI +
  PR-consistency auto-capture likewise only fire when a pack is linked.
- **CI auto-capture reflects the moment of PR creation** — checks that finish
  later need a `/ci-refresh` (or a fresh snapshot) to update the `ci` artifact.
- **Session auto-test is off by default** (it would run a suite in your live cwd);
  the diff is always packaged, but tests are opt-in.

---

## 17. Security & permissions

- **Feature + workspace RBAC.** Every route is `Feature::ProofPack`-gated (View for
  reads, Edit for writes) **and** re-checks the caller's workspace role. See
  [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md).
- **Redaction before storage.** A trust layer that itself leaked secrets would be
  worse than none, so all artifact text — including captured CI/API/DB/Kafka
  output and the redacted PR title/description — is redacted
  (JWT/AWS/PEM/Bearer/email/sensitive-JSON-key) before it is persisted, with the
  hit count recorded.
- **Tamper-evidence.** Each inline artifact carries a `content_sha256` of its full
  content; each snapshot carries a `bundle_sha256` over a canonical bundle, so a
  frozen evidence set can be verified later.
- **Content caps.** Text is capped at 2 MiB (preview 8 KiB); media at 25 MiB with
  a MIME allow-list (`415`/`413` on violation). The full content is fetched through
  an explicit, RBAC-gated endpoint.
- **Waive is the only human status override**, and it is accountable: the approver
  is **always the authenticated request principal** (never a client field), a
  reason of **≥10 chars** is required, the `waived_at` timestamp is recorded, and
  an immutable `approval` artifact is written. `OTTO_PROOF_WAIVER_MIN_ROLE=admin`
  raises the bar to workspace Admin (closes the service-principal self-waive
  path). The PR `allow_unproven` override is likewise recorded as an audit
  `approval` artifact rather than silently bypassing the gate.
- **Loopback by default.** Like the rest of the daemon, these endpoints are only
  reachable over loopback unless you explicitly enable remote access.

---

## 18. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| Pack stuck at `partial` after a session | A `CodeChange` kind needs a diff **and** a passing *recognized* test command. Add a `command` artifact whose title is a real runner (§3.1), or `/assemble` with `commands`. A passing `true` won't satisfy it. |
| `passed` you expected, but it's `partial` | A repo policy is **strengthening** the requirement (require CI / PR-consistency / review). Open **Requirements** (#137) or add the missing passing artifact; the done-contract checklist shows exactly which item is unmet. |
| `passed` but you expected `failed` | No artifact is `failed`. A neutral diff/log is `info`, not `failed`; mark a `db`/`api`/`command` artifact `failed` explicitly to flip the pack. |
| PR creation returns `409 ... not passed` | The linked pack isn't `passed`/`waived`. Add evidence until it passes, **Waive** it, or re-open with `allow_unproven: true` (recorded as an audit artifact). Or disable globally with `OTTO_PROOF_REQUIRE_PR=0`. |
| `pr_inconsistent` badge / pack `failed` after opening a PR | The auto PR-consistency check hard-failed — the description claims "tests pass"/"CI green" while the pack's tests fail or are absent. Fix the description (or add the passing tests) and re-run `/pr-check`. |
| CI badge looks stale | CI is captured at PR-open time. Hit **Refresh CI** (`/ci-refresh`) to re-fetch live status. |
| Adding media returns `415` / `413` | `415` = the MIME isn't in the allow-list (png/jpeg/gif/webp/svg, mp4/webm); `413` = the decoded blob exceeds 25 MiB. Trim it or reference it by URL via a `screenshot` artifact's `content_url`. |
| Session all-done didn't run tests | Expected — auto-test is off. Set `OTTO_PROOF_AUTO_TEST=1`, or add the test `command` via `/assemble`. The diff is always packaged. |
| Goal loop finalized "achieved" with no test | The hard block is opt-in — set `OTTO_PROOF_REQUIRE_GOAL_LOOP=1` (bounded by the iteration cap). |
| `risky_change` badge on a benign change | A risky **path** was touched (`migrations/`, `.github/`, a lockfile, `*.sql`, or a security-word basename) **or** `risk_score ≥ 50`. Inspect the diff artifact's `risky_files`. |
| Waive rejected | A reason of **≥10 chars** is required, and if `OTTO_PROOF_WAIVER_MIN_ROLE=admin` you need workspace Admin. |
| Artifact content shows `[redacted]` | The redactor matched a JWT/AWS-key/PEM/Bearer/email/sensitive-key shape; `metadata.redactions` is the hit count. This is by design. |
| Artifact ends `…(truncated)` | Content exceeded the 2 MiB store cap (snapshots cap each artifact at 64 KiB). The preview is 8 KiB; "Load full" returns the (still capped) stored content; the `content_sha256` is of the full content. |
| No `proof_pack_updated` event | The pack didn't change, or you're not subscribed to the workspace event stream. The UI also re-fetches on focus. |

---

## 19. Related docs

- [`./code-review.md`](./code-review.md) — the multi-agent AI review that feeds a
  `review` pack (and the persisted findings).
- [`./review-findings.md`](./review-findings.md) — the tracked review-findings
  workflow, its own (namespaced) review proof pack, and memory ingest.
- [`./goal-loops.md`](./goal-loops.md) — the bounded Plan→Execute→Evaluate loop
  whose verify commands + diff become proof, with the opt-in hard gate.
- [`./workflows.md`](./workflows.md) — node/approval outputs become workflow
  artifacts.
- [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md) — the `ProofPack` feature roles.
- `docs/contracts/api.md` (#115–137), `docs/contracts/ws.md` — authoritative
  endpoint + WS contract.
