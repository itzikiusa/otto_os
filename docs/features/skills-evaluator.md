# Skills Evaluator (benchmark & iterate a skill)

Otto's **Skills Evaluator** measures how good a *skill* actually is by putting it
to work: a coding agent uses the skill to implement a real task in an **isolated
git worktree**, a fleet of **validation agents** grades the result against the
quality dimensions you define, the round is **scored 0–100**, and (between rounds)
an **improver agent** edits a copy of the skill and re-runs — each iteration
scored so you can watch the skill get better (or not). You can then **compare
runs side by side** (A/B two skills, or the same skill across providers) and
**promote** the winning skill version into the shared Otto library.

A skill here is the same artifact as in the [Skills Library](./skills-library.md):
a `SKILL.md` (prose + frontmatter) that tells an agent *how* to do a class of
work. The Evaluator answers a different question than the library: not "is this
skill installed?" but **"does following this skill make an agent produce better
code, and which version should I keep?"**

This document is the authoritative end-user + operator reference. Where it
describes wire-level behaviour, the source of truth is the contract files in
`docs/contracts/` (`api.md` §"Skill evaluations", `ws.md` §"Skill-eval
completion"); the Rust engine in `crates/otto-server/src/skill_eval.rs` +
storage in `crates/otto-state/src/skill_evals.rs` (migrations `0017`, `0019`)
and the Svelte UI in `ui/src/modules/skills-eval/` implement it.

> **Not the Skills Library.** The library
> ([`./skills-library.md`](./skills-library.md)) is *where skills are stored and
> installed*; the Evaluator is *how you measure and improve one*. The two meet at
> **Promote** (Evaluator → Library) and at the **library skill source** (Library
> → Evaluator). They are separate features with separate RBAC capabilities
> (`SkillEval` vs `Skills`).

---

## 1. Overview

One **evaluation run** is a scored loop over your chosen skill:

```
   skill under test  +  task  +  validations
          │
          ▼   (per iteration, in a fresh disposable git worktree)
   ┌──────────────────────────────────────────────────────────┐
   │ 1. IMPLEMENT — one agent follows the skill to do the task  │
   │ 2. VALIDATE  — N agents grade the result (run concurrently)│  ← each is a
   │ 3. SCORE     — iteration score = mean of validation scores │     real, openable
   │ 4. IMPROVE   — improver edits a copy of the skill          │     coding-agent
   └──────────────────────────────────────────────────────────┘     session
          │  (next iteration tests the improved skill)
          ▼
   best iteration → Promote the winning SKILL.md to the library
```

Every implementation, validation, and improver agent is a **real coding-agent
session** (claude / codex / agy — the same kind you run from the Agents tab),
spawned by the daemon's session manager (run as a root user, tagged
`source: skilleval` so they stay out of the main session grid but remain openable
from the report). You can open any of them live, watch the terminal, respond if
one blocks on input, and retry a single validation.

### Where it lives

| Surface | Location |
|---|---|
| Main UI | **Skills Evaluator** section (`ui/src/modules/skills-eval/SkillsEvalPage.svelte`) — left list of runs + right report pane |
| Start form | `StartEvalForm.svelte` (the "New evaluation" form) |
| One run's report | `RunDetail.svelte` (header + per-iteration breakdown, polls while running) |
| Side-by-side compare | `CompareView.svelte` |
| Defaults / config | **Settings → Skill Eval** (`ui/src/modules/settings/SkillEvalSettings.svelte`) |
| REST client | `ui/src/lib/api/skillsEval.ts` |
| Backend engine | `crates/otto-server/src/skill_eval.rs` |
| Persistence | `crates/otto-state/src/skill_evals.rs`; tables `skill_evals`, `skill_eval_iterations` |
| WS event | `skill_eval_updated` (workspace-scoped) |

Runs are **workspace-scoped**: the list shows the current workspace's runs,
newest first. Each run's iterations run against that workspace's git repo (or a
scratch repo — see §5).

---

## 2. When and why to evaluate a skill

Reach for the Evaluator when you have a skill and want **evidence**, not a guess,
about its quality:

- **Authoring / hardening a skill.** Write a draft `SKILL.md`, evaluate it against
  a representative task, read the findings, and let the improver tighten it. Keep
  iterating until the score plateaus, then promote the winner.
- **Regression-checking an edit.** Changed a skill by hand? Run it and read the
  per-iteration **fixed / introduced** counts (§6) to confirm an edit fixed more
  than it broke.
- **A/B comparing two skills.** Evaluate skill A and skill B on the *same* task,
  then **Compare** their best iterations dimension-by-dimension (§7).
- **Comparing the same skill across providers.** Run the skill with `claude` and
  again with `codex` (different *Implementation CLI*), then compare — does the
  skill hold up regardless of which agent follows it?
- **Choosing which version to keep.** The run's **best iteration** and the
  per-dimension scores tell you which `SKILL.md` to promote into the library, and
  which the [Self-Improvement](./self-improvement.md) engine should adopt.

It is **not** a substitute for running your real test suite or for
[AI Code Review](./code-review.md) on production diffs — the implementation it
produces lives only in a throwaway worktree and is graded by *agents*, not by
your CI. Treat the score as a comparative signal about the **skill**, not a gate
on shippable code.

---

## 3. Starting an evaluation — every form field

Click **New** in the left rail (or the empty-state button) to open the start
form. It prefills from the saved defaults (**Settings → Skill Eval**, §8) and from
the skills it can discover in the current workspace.

The lede on the form states the contract plainly: *"A coding agent uses the skill
to implement your task in a fresh git worktree, validation agents grade the
result, and (between iterations) an improver edits a copy of the skill and
re-runs — each round scored."*

| Field | What it is | Notes / constraints |
|---|---|---|
| **Skill under test** | A dropdown of discovered sources, plus a **Custom path or archive** option. | Discovered sources are labelled `name · library` or `name · <provider>` (see §5). Picking one shows its description. |
| **Custom path or archive (.zip / .gz / .tgz)…** | When selected, a text field + **Browse** button to point at a skill folder, a `SKILL.md`, or an archive. | The folder picker (`FolderPicker`) lets you pick files. Archives are extracted server-side; the engine finds the `SKILL.md`/`.md` inside. |
| **Task to implement** | A free-text description of the work the implementing agent must do *using the skill*. | **Required.** This is the same task for every iteration, so the only variable across rounds is the skill. e.g. *"Add a new endpoint that returns a player's bonus balance history."* |
| **Implementation CLI** | Which agent CLI implements the task each iteration. | Drawn from `auth.meta.providers` minus `shell` (falls back to `claude, codex, agy`). Defaults to `claude` when available. |
| **Iterations** | How many implement→validate→score(→improve) rounds to run. | Integer **1–10**. Round 1 tests the original skill; later rounds test the improver's edits. A perfect-scoring round ends the run early (§5). |
| **Validation passes** | How many times *each* validation agent runs, with its scores **averaged**, to reduce grader noise. | Integer **1–3** (clamped server-side). Findings across passes are unioned/deduped, keeping the highest severity. |
| **Improver agent** | Which CLI edits the skill between iterations. | Same provider list. Defaults to the configured improver, else the Implementation CLI. With `Iterations = 1` the improver never runs. |
| **Validations** | One or more quality **dimensions** to grade against. **At least one is required.** | Each validation has a **name** (e.g. `logging`), **criteria** (free text: *what to check and how to judge it*, passed verbatim to the grading agent), and a set of **provider chips** — each selected CLI runs that dimension as its own agent. With no chip selected, it falls back to the Implementation CLI. |
| **Base git ref (optional)** | The git ref each iteration's worktree is created from. | Defaults to `HEAD`. If the workspace root isn't a git repo, Otto uses a scratch repo at `~/Otto/SkillsEvaluator` (created automatically) and ignores this field. |

**The agent-session estimate.** Above the **Start evaluation** button the form
shows `≈ N agent sessions` so you can see the scope before launching. The rough
worst case is:

```
iterations × (1 implementation + Σ over validations of (providers × passes))
            + (iterations − 1)        ← one improver between rounds
```

It is an *upper bound*: improver runs are skipped when a round scores perfectly,
which also ends the run early. Each session consumes real provider tokens, so
this number is your cost signal.

**Start gating.** The **Start evaluation** button is disabled until: the task is
non-empty, an Implementation CLI is chosen, a skill source is selected (custom
path non-empty, or at least one discovered source exists), and there is at least
one validation with both a name and criteria filled in.

On submit, the run is created (`status: running`), selected, and shown in the
report pane; you watch progress live there.

---

## 4. How a run executes (the engine)

The daemon runs the whole loop in the background (`run_skill_eval` →
`run_skill_eval_core`). Order of operations:

1. **Resolve the skill source** to a `(name, body)` pair (§5). A `library` skill
   comes from the Otto library; a `provider` skill is read from
   `~/.{provider}/skills/<name>/SKILL.md`; a `path` is a folder, a `SKILL.md`, or
   an archive that's extracted to a temp dir.
2. **Resolve the base repo.** If the workspace root is a git repo with commits,
   iterations branch from `base_ref` (default `HEAD`). Otherwise Otto uses a
   **scratch repo** at `~/Otto/SkillsEvaluator` (override with
   `OTTO_SKILLEVAL_DIR`), creating + `git init`-ing it with an initial commit on
   first use so worktrees can be made. When the scratch repo is used, `base_ref`
   is forced to `HEAD`.
3. **Pre-trust** the repo for every provider that will run (implementation,
   validation, improver) via `otto_sessions::trust::ensure_trusted`, so no agent
   stalls on a trust prompt.

Then, for each iteration `1..=iterations`:

4. **Create a disposable worktree** at
   `…/otto-skilleval/<eval_id>/iter<N>` (a `git worktree add --detach` from
   `base_ref`). Each iteration gets a *clean* checkout — implementations never
   contaminate each other.
5. **Install the skill copy** into the worktree at
   `.claude/skills/<skill_name>/SKILL.md` so the agent can discover it the same
   way it would in a normal session. (`skill_name` is uniquified per run/iter,
   e.g. `myskill-run-<tag>-iter1`.)
6. **Implementation phase.** Spawn one **Implementation CLI** agent in the
   worktree with a prompt that embeds the full skill body inline, states the task,
   instructs the agent to implement it fully *following the skill exactly*, and to
   write a 2–4 sentence summary file as its last act. Timeout: **40 min**. The
   captured summary becomes the iteration's `impl_summary`; its session id is kept
   so you can open the terminal and view the code diff.
7. **Validation phase (concurrent).** Otto pre-computes one `git diff HEAD` of the
   worktree and injects it (capped at 6 000 chars) into each validator's prompt
   so they don't each re-run `git diff`. Then it spawns **every (validation ×
   provider)** agent **in parallel** (a `tokio` `JoinSet`). Each validation agent
   is told it is **strictly read-only** (it must not edit the repo) and must emit a
   JSON array of findings. Per-validator timeout: **15 min**. When *Validation
   passes* > 1, that agent is run multiple times and its scores averaged.
8. **Score the iteration** (§6). The iteration score is the **mean of all
   validation-agent scores**.
9. **Improve phase (between iterations only).** If this isn't the last iteration
   *and* the round was not perfect, spawn the **Improver agent** (run from the
   base repo, not the worktree) with the task, every prior skill version + its
   score, the collected findings, and a built-in skill-authoring best-practices
   block — which explicitly forbids *gaming the validators* (no instructions that
   merely satisfy the named checks; edits must improve the skill *in general*).
   Timeout: **10 min**. It returns a JSON `{base_iter, skill, summary}`; Otto
   records the improved skill (`skill_after`), a unified diff (`skill_diff`), and
   the `improvement_summary`. The improver is told to base its edit on the
   best-scoring prior version (Otto defaults `base_iter` to the best so far if the
   agent's choice is invalid). The **next** iteration tests that improved skill,
   linked back via `base_iter`. If the improver produces nothing usable, the skill
   is carried forward unchanged.
10. **Early exit on a perfect round.** If a round produced **zero findings**, the
    run stops immediately — no further iterations, no improver.

After the loop: pick the **best iteration** (highest score; ties → latest), write
the run **summary**, set `status` to `done` (or `error`/`cancelled`), post a
completion notification, and broadcast the `skill_eval_updated` WS event (§9).

**Agent model & trust.** The model column shown per validation is for display;
the visible PTY session driver does not force a per-agent model override (it
mirrors the code-review reviewer-session path). All worktrees and the base/scratch
repo are pre-trusted for the providers that will run.

---

## 5. Skill sources

`GET /workspaces/{id}/skill-sources` populates the **Skill under test** dropdown.
Two `kind`s are auto-discovered:

- **`library`** — every skill in the shared Otto library
  (`ctx.context_library.list_skills()`), labelled `name · library`. Description
  comes from the library entry.
- **`provider`** — on-disk skills for each agent CLI, scanned at
  `~/.claude/skills`, `~/.codex/skills`, and `~/.agy/skills`. A directory counts
  if it has a `SKILL.md` and a safe name; the description is parsed from the
  skill's YAML frontmatter (`description:`). Labelled `name · <provider>`.

A third kind is available only via the **Custom path or archive** branch:

- **`path`** — a reference you type or browse to. It may be:
  - a **folder** containing a `SKILL.md`/`.md` (name = folder name),
  - a single **`SKILL.md`** (name = its parent folder) or another `.md` (name = its
    stem), or
  - an **archive** (`.zip`, `.gz`, `.tgz`, `.tar.gz`) — extracted to a temp dir,
    then the `SKILL.md`/`.md` inside is used (name = archive stem).

Provider/path names are validated (`is_safe_name` / sanitized) to keep filesystem
operations safe.

---

## 6. The run detail (metrics & outputs)

Selecting a run opens `RunDetail`, which polls every 2 s while the run is active
(backing off to 5 s after ~20 min) and refreshes immediately on the
`skill_eval_updated` WS event for this run.

### Header

- **Skill name**, a **status pill** (`running` / `done` / `error` / `cancelled`),
  and — once known — a **best-score badge** (`best NN · iter N`).
- **Actions:** **Cancel run** (while active), **Promote winning skill** (once a
  best iteration exists, §7/§8), **Delete** (with a confirm — removes the run, its
  sessions, and its worktrees).
- The **task**, then meta chips: `impl: <cli>`, target iteration count, and one
  `iter N: <score>` chip per iteration.
- A run **summary** sentence (e.g. *"Evaluated skill 'X' across 3 iterations. iter
  1: 72, iter 2: 88, iter 3: 91. Best: iteration 3 (score 91). Improvement of +19
  over the baseline."*) and any **error** line.

### Per iteration

Each iteration card shows:

- **Header:** iteration number; an `improved from iter N` chip if it tested an
  improver edit; the uniquified `skill_name`; a **regression** indicator vs the
  previous *done* iteration — `fixed F · introduced I` (turns amber if it
  introduced more than it fixed, computed by diffing the set of findings between
  rounds); the iteration **score badge**; and the iteration **status** (`pending`
  / `implementing` / `validating` / `improving` / `done` / `error`).
- **Implementation:** the impl provider, the agent's `impl_summary`, the worktree
  path, **View code diff** (lazily fetches the staged diff the agent produced;
  `+`/`-` colourised; may be marked truncated), and **Open session** to attach to
  the implementation agent's live terminal.
- **Validations:** a header `· N issues found`, then one card per validation
  agent showing its name, `provider · model`, a **passed/failed** pill and a
  **score badge** when done, **Open** (the agent's terminal), **Retry** (re-run
  just this validation agent — §7), and an **N issues** toggle. A `waiting` agent
  shows a *"looks blocked on input — Open the session to respond"* hint. Expanding
  the findings lists each one with its **severity** (`info` / `warn` / `fail`),
  optional **location**, the **Issue**, and the suggested **Fix**.
- **Skill (tested):** **Copy** / **Download** (`<name>.SKILL.md`) / **Promote**
  the exact skill body this iteration ran with.
- **Skill improvement** (if the improver ran): the `improvement_summary`, **Copy
  improved** / **Promote improved**, and **View skill diff** (the unified diff
  between the base and the improved `SKILL.md`).

### Scoring (the numbers)

Scores are 0–100, colour-coded **good ≥ 85 / ok ≥ 60 / bad < 60**:

- **Per validation agent:** start at 100, then subtract per finding by severity —
  **`fail`/`error`/`critical` = −25** (and marks the validation *failed*),
  **`warn`/`warning`/`major` = −8**, anything else (`info`, …) **= −2**; clamped to
  0–100. **Passed** = no `fail`-class findings.
- **Across passes** (when *Validation passes* > 1): the per-pass scores are
  **averaged**; findings are unioned and deduped by issue (highest severity wins).
- **Iteration score:** the **mean of all validation-agent scores** in that
  iteration (an iteration with no validation scores at all defaults to 100).
- **Best iteration / best score:** the iteration with the highest score (ties go
  to the *latest*). These drive the header badge and the **Promote winning skill**
  default.

---

## 7. Comparing runs (CompareView)

Toggle **Compare** in the left rail (enabled once you have ≥ 2 runs), tick **two
or more** runs, and the main pane renders a side-by-side table
(`CompareView.svelte`). It is purely client-side over already-loaded runs.

Columns are the picked runs (header = `source_skill` over its `impl_cli`). Rows
are the comparison **dimensions**:

| Row | What it shows |
|---|---|
| **Best score** | Each run's overall best score; the column-max is outlined as the **winner**. |
| **Iterations** | How many iterations each run took. |
| *one row per validation dimension* | The **mean score for that dimension in each run's best iteration** (only `done` validation agents counted). The per-row leader is outlined. |

Dimensions are the union of validation names seen across the compared runs (so an
A/B of two skills with overlapping validations lines them up; a dimension a run
never measured shows `—`). This is the view for **"skill A vs skill B"** and
**"same skill, claude vs codex"** decisions: read down a dimension to see which
skill/provider scored highest where it matters.

---

## 8. Configuration (Settings → Skill Eval)

**Settings → Skill Eval** edits the **defaults** the start form prefills. It is
**root-only** to read and write (`GET`/`PUT /settings/skill-eval`); the stored
blob is `SkillEvalConfig`:

| Setting | Meaning |
|---|---|
| **Iterations** | Default number of rounds (**1–10**). |
| **Validation passes** | Default passes to average (**1–3**, clamped). |
| **Improver agent** | Default CLI that edits the skill between rounds. |
| **Default validations** | The dimension list prefilled into the form — each with a **name**, **criteria**, and a set of **provider chips** (one agent per selected CLI). |

Out of the box the defaults ship five validations — **logging**,
**documentation**, **properties-config**, **variable-naming**, and
**type-naming** — with `iterations = 2` and `validator_passes = 1`; edit, remove,
or add to these freely.

Changing defaults here does **not** alter existing runs — each run captures the
full `StartSkillEvalReq` it launched with (stored as `config_json`), which is what
the per-validation **Retry** replays.

**Promotion** of a winning skill into the library is also **root-only** (it writes
to the shared library, like `PUT /library/skills`).

---

## 9. API & contract reference

Authoritative: `docs/contracts/api.md` §"Skill evaluations" and `docs/contracts/ws.md`
§"Skill-eval completion (A11)". RBAC: **reads = ws Viewer, run/mutations = ws
Editor, config + promote = root** (plus the `SkillEval` feature capability under
multi-user RBAC).

### REST

| Method & path | Auth | Request → Response |
|---|---|---|
| `POST /workspaces/{id}/skill-evaluations` | ws Editor | `StartSkillEvalReq` → `SkillEval` (status `running`) |
| `GET /workspaces/{id}/skill-evaluations` | ws Viewer | — → `SkillEval[]` (newest first) |
| `GET /workspaces/{id}/skill-sources` | ws Viewer | — → `SkillSourcesResp` (discovered library + provider skills) |
| `GET /skill-evaluations/{id}` | ws Viewer | — → `SkillEval` (with `iterations`) — poll while running |
| `POST /skill-evaluations/{id}/cancel` | ws Editor | — → `SkillEval` — stops the loop, **archives (kills) its agent sessions** |
| `DELETE /skill-evaluations/{id}` | ws Editor | — → `204` — archives sessions, **removes the worktrees**, drops the rows |
| `POST /skill-evaluations/{id}/promote` | **root** | `PromoteSkillReq` → `LibrarySkill` — saves the chosen version to the library |
| `GET /skill-evaluations/{id}/iterations/{iter_id}/diff` | ws Viewer | — → `ImplDiffResp` (`{diff, truncated}`) — the impl agent's staged code diff |
| `POST /skill-evaluations/{id}/iterations/{iter_id}/agents/{index}/retry` | ws Editor | — → `SkillEval` — **re-runs one validation agent** |
| `GET /settings/skill-eval` | **root** | — → `SkillEvalConfig` |
| `PUT /settings/skill-eval` | **root** | `SkillEvalConfig` → `SkillEvalConfig` |

**`StartSkillEvalReq`** (key fields): `source` (`{kind: 'library'|'provider'|'path',
reference, provider?}`), `task`, `impl_cli`, `iterations`, `validator_passes?`
(1–3), `validations[]` (`{name, criteria, providers[], model}`), `improver?`
(`{provider, model}`), `base_ref?`.

**`PromoteSkillReq`**: `{iteration_id, source: 'tested' | 'improved', name}` —
`tested` writes the iteration's `skill_before`; `improved` writes its `skill_after`
(errors if there is none). `name` must match `[A-Za-z0-9_-]+`; an existing library
skill of that name is **overwritten**.

### WebSocket — `skill_eval_updated`

Workspace-scoped; emitted once per run when it reaches a terminal state, by
`crates/otto-server/src/skill_eval.rs`:

```json
{
  "type": "skill_eval_updated",
  "workspace_id": "<Id>",
  "run_id": "<eval_id>",
  "status": "done|error|cancelled"
}
```

The UI routes it through `skillEvalBus`: `RunDetail` refreshes the open run on
demand instead of waiting for its 2 s timer, and the list updates the run's
status/score. (The timed poll remains a fallback if the socket is down.)

---

## 10. Capabilities & limitations

**Capabilities**

- End-to-end **implement → grade → improve** loop with real, openable agent
  sessions; live progress over WS + polling.
- Multi-dimension grading; each dimension fanned across one or more provider CLIs;
  multi-pass averaging to cut grader noise.
- Automated skill improvement between rounds, with a unified skill diff and a
  human-readable summary.
- A/B and N-way **Compare** across runs and providers; per-dimension winner
  highlighting.
- One-click **Copy / Download / Promote** of any iteration's tested or improved
  `SKILL.md`.
- Works **without a git workspace** (auto scratch repo).

**Limitations**

- **The implementation is throwaway.** Code lives only in a disposable worktree
  (deleted on run delete); the Evaluator does not run your tests/CI and does not
  produce a PR. The score grades the *skill*, not shippable code.
- **Grading is agent-judged**, so it is comparative and somewhat noisy — hence
  *Validation passes*. Treat absolute scores as relative signals.
- **Bounded prompts:** the injected diff each validator sees is capped at 6 000
  chars; the impl summary at ~400; very large changes may exceed these.
- **Timeouts** are fixed: implementation 40 min, validation 15 min, improver
  10 min. A round that needs longer will time out / error.
- **Early exit on a perfect round** ends the run — you won't get extra iterations
  past a 0-finding round.
- The improver runs only **between** iterations, so `Iterations = 1` never edits
  the skill.
- **No automatic library write** — promotion is an explicit, root-only action.

---

## 11. Security & permissions

- **RBAC.** Reads need ws **Viewer**; starting/cancelling/deleting/retrying need ws
  **Editor**; **config and promote are root-only** (promote writes the shared
  library). Under multi-user RBAC the `SkillEval` feature capability gates the UI
  and routes; see [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md).
- **Real agents in worktrees.** Implementation and improver agents *can edit
  files* — but only inside a disposable worktree / the scratch repo, never your
  working tree. Validation agents are instructed to be **strictly read-only**.
  Repos are pre-trusted only for the providers that run.
- **Provider tokens.** Every agent session consumes real provider tokens; the
  agent estimate on the form is your cost guard. Provider auth/keys are the CLIs'
  own; Otto secrets stay in the macOS Keychain and the daemon listens on loopback
  only.
- **Scratch repo location.** `~/Otto/SkillsEvaluator` (or `OTTO_SKILLEVAL_DIR`);
  worktrees under the OS temp dir. **Delete** cleans these up; **Cancel** kills
  the live sessions.

---

## 12. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| Start button stays disabled | Missing task, no Implementation CLI, no skill source selected/typed, or a validation with an empty name or criteria. Fill the required fields. |
| A validation agent stuck on `waiting` | It blocked on an input prompt. **Open** its session and respond, or **Retry** that validation. |
| Validation ends `error` / "validation did not complete" | The agent never wrote a usable findings file (timed out or produced no JSON). **Retry** it; check the CLI is installed/logged in. |
| Iteration shows score **100** with no validations | An iteration with no completed validation scores defaults to 100 — confirm the validations actually ran (open them). |
| `error` on the run with a `worktree:` message | `git worktree add` failed (bad `base_ref`, dirty/locked repo). Use a valid ref, or rely on the scratch repo by pointing at a non-git workspace root. |
| `NotFound` for a `provider`/`path` skill | The `SKILL.md` isn't at `~/.{provider}/skills/<name>/SKILL.md`, or the path/archive has no `SKILL.md`/`.md`. Check the location/contents. |
| Code diff shows "(worktree no longer available)" | The worktree was already removed (e.g. after **Delete**). The diff is only viewable while the worktree exists. |
| Promote fails: invalid name | Library names must be `[A-Za-z0-9_-]+`. Rename and retry. |
| Promote returns `403` | Promotion (and **Settings → Skill Eval**) is **root-only**. |
| Run never finishes / very slow | Large task or many validations × providers × passes; watch timeouts (40/15/10 min). Reduce passes or providers, or split the task. |
| The improver never ran | Expected when `Iterations = 1`, on the last iteration, or after a perfect-scoring round (it short-circuits). |

---

## 13. Related docs

- [`./skills-library.md`](./skills-library.md) — the shared skill library: where
  `library` sources come from and where **Promote** writes the winning `SKILL.md`.
- [`./self-improvement.md`](./self-improvement.md) — Otto's self-improvement
  engine, which acts on the kind of skill quality this feature measures (use eval
  results to decide which skill version it should adopt).
- [`./code-review.md`](./code-review.md) — the multi-agent code-review
  orchestrator; the Evaluator's validation phase uses the same "fan-out agents,
  parse findings, aggregate" pattern, but grades a *skill* rather than a diff.
- `docs/contracts/api.md` (§"Skill evaluations"), `docs/contracts/ws.md`
  (§"Skill-eval completion") — the authoritative endpoint + WS contract.
