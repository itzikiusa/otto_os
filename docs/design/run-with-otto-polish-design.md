# Run with Otto — launcher polish, repo & model pickers, truthful CLI-update notices

Design doc, 2026-07-04. Follow-up to `run-with-otto-design.md` (the original
feature). Driven by direct user feedback on the shipped page.

## 1. Problems (user feedback, verbatim intent)

1. **"It is too plain"** — the page is a bare textarea + three controls. Nothing
   on it shows *how* a run executes, so the flagship pipeline reads as an empty
   form.
2. **"How it runs — it is pretty much what workflows does, no?"** — the product
   does not differentiate Run with Otto from Workflows anywhere in the UI. The
   answer exists only in docs.
3. **"Please add folder picker"** — the New goal loop form (`GoalDefineForm`)
   asks for an absolute repository path in a bare text input; the Run with Otto
   launcher has **no repo control at all** (it silently targets the workspace's
   first registered repo — `run_sources.rs` `resolve_repo` fallback).
4. **"Model to run"** — there is no way to pick the model anywhere in the flow,
   and the backend never plumbs one: single-agent runs call
   `orchestrator.run_agent(prompt, cwd, model: None, …)` (`run_engine.rs:241`),
   goal-loop-mode runs build `GoalLoopConfig::default()` (`run_engine.rs:299`),
   and goal-loop executor sessions never receive `meta.model` even though
   `SessionManager::create` already honors it (`manager.rs:57 model_args`).
5. **Daily CLI update notice bug** — "Updated CLIs: agy, claude, codex. …
   Note: exit 1: Error loading configuration: unknown variant `xhigh` …".
   Two defects: (a) machine-local — a stale Homebrew codex 0.57.0 shadowed the
   current codex in login shells and could not parse
   `model_reasoning_effort = "xhigh"` (fixed live: `brew upgrade --cask codex`
   → 0.142.5; verified `zsh -l -c 'codex update'` exits 0); (b) code — all
   provider updates run as ONE compound `cmd1; echo; cmd2; …` shell line
   (`cli_update.rs:266-272`), so the exit status is the *last* command's,
   failures are mis-attributed, and the notice claims "Updated CLIs: a, b, c"
   even when one failed.

## 2. Requirements checklist

- **R1** Page visually communicates how a run executes (not plain).
- **R2** In-product differentiation from Workflows.
- **R3** Folder picker for repository path — goal loop form AND launcher repo
  selection.
- **R4** Model picker on both forms, honored end-to-end (single-agent CLI flag,
  goal-loop roles + executor sessions).
- **R5** Focused E2E tests for exactly these changes (per the standing
  "run relevant only" directive).
- **R6** Worktree → branch → gates → local-main merge → rebuild + reinstall app.
- **R7** CLI-update notice reports per-provider truth (and the xhigh root cause
  is fixed on this machine).

## 3. Established facts (explored, file:line)

- Stage machine is pure and fixed: `otto-core/src/run.rs` `RunStatus`
  (Queued→ResolvingSource→BuildingContext→Provisioning→Executing→Proving→
  Reviewing→AwaitingApproval→[approve]→DraftingPr→Completed).
- Engine: `otto-server/src/run_engine.rs` — `advance()` CAS loop; stages at
  `:148/:200/:223/:350/:396/:455`. Single-agent executes via
  `otto-orchestrator` `run_agent` → `ClaudePty::run_prompt` which appends
  `--model <m>` only when `Some` (`claude_pty.rs:96`). **Single-agent execution
  is claude-only today; `run.provider` is stored but not honored there.**
- Goal-loop executors run as real sessions with `provider` honored
  (`goal_loop.rs:909-933`) but `exec.model` dropped (session `meta` lacks
  `model`); roles run headless with `model` honored, provider claude-only
  (`goal_loop.rs:557-566`).
- `LaunchRunReq` (`run.rs:301`) accepts `repo_id` today; **no `model` field
  anywhere** (req, `NewRun`, `otto_runs` table, `OttoRun`).
- Providers list: `GET /api/v1/meta` → `providers` + `default_provider`
  (`routes/meta.rs:39-44`). No models endpoint; models are free-text aliases
  ("opus"/"sonnet"/"haiku") by app convention (swarm `AgentEditor.svelte:130`).
- Folder picking: `GET /api/v1/fs/browse` (sandboxed, `routes/fs.rs:338`) +
  shared `FolderPicker.svelte` (`gitOnly` mode) used by 15+ features — but not
  by GoalDefineForm or RunLauncher. `POST /workspaces/{id}/repos/detect`
  (`otto-git/src/http.rs:512`) registers-or-returns the repo whose toplevel
  contains a given path — ideal Browse→repo_id bridge.
- Latest migration: `0097`. `otto_runs` created in `0087`.
- E2E: `desktop-run-with-otto.spec.ts` exists (API pipeline + one UI smoke);
  specs must be named `desktop-*.spec.ts` to run on the desktop-browser
  project.
- `src-{kind}` badge classes are referenced in all three run components but
  **never styled** — dead hooks ready for per-source colors.

## 4. Design

### 4.1 Backend — per-run model (small, additive)

1. `otto-core/src/run.rs`: `LaunchRunReq.model: Option<String>`;
   `OttoRun.model: String` ("" = provider default).
2. Migration `0098_run_model.sql`:
   `ALTER TABLE otto_runs ADD COLUMN model TEXT NOT NULL DEFAULT '';`
   (append-only; no CHECK constraints touched).
3. `otto-state/src/runs.rs`: `NewRun.model`, INSERT column, `row_to_run` read.
4. `run_service::launch`: `model: req.model.filter(non-empty-trim)…unwrap_or_default()`
   (mirrors `provider`).
5. `run_engine::execute_single_agent`: pass
   `(!run.model.is_empty()).then_some(run.model.as_str())` to `run_agent`.
6. `run_engine::execute_goal_loop`: start from `GoalLoopConfig::default()` and
   stamp every executor with the run's `provider`/`model` (when non-empty).
   Roles keep their tuned defaults (planner/evaluator "sonnet", digester
   "haiku") — they are pipeline bookkeeping, not the change-producing agent.
7. `goal_loop::run_executor_attempt`: include `"model": exec.model` in the
   session `meta` when non-empty → `model_args` already turns it into
   `--model` for claude/codex. This also makes the long-stored-but-ignored
   `GoalLoopAgentCfg.model` field real for ALL goal loops, not just run-mode
   ones.

**Provider semantics kept honest:** single-agent execution stays claude-only
(that is what the engine does); the UI fixes provider to `claude` in
single-agent mode with the model picker active, and offers the full
`/meta.providers` list only for goal-loop mode (executors honor it). No silent
lies.

### 4.2 Backend — truthful CLI-update run (R7)

Rework `cli_update.rs` `run()`/`run_updates()`:

- Run each provider's update **separately** (`$SHELL -l -c <cmd>`, same
  10-minute cap per provider), collecting `UpdateOutcome { name, ok, detail }`.
- Extract pure `fn build_body(outcomes, reload: Option<(u32,u32)>) -> String`
  producing e.g.
  `Updated CLIs: agy, claude. Failed: codex — exit 1: <stderr tail>. Reloaded 2 open session(s).`
  Unit-test it (all-ok, one-fail, all-fail, reload-off, reload-failures).
- Severity: `Warn` when any provider failed or any reload failed.
- Only reload sessions for providers whose update **succeeded** (don't re-exec
  onto a binary whose update just failed).

### 4.3 UI — launcher (RunLauncher.svelte) and page (RunWithOttoPage.svelte)

**Signature element — the pipeline rail.** One horizontal strip of the real
stage machine, rendered from a shared `STAGES` list (label + icon + the
matching `RunStatus` values):

`Source → Context → Branch → Execute → Proof → Review → Approve → PR draft`

- On the **launcher card** it is the static "how it runs" statement (dimmed
  chips, arrows, approval gate chip amber — the one human pause point).
- Under it, one quiet line answers R2 in-product:
  "A fixed, evidence-gated pipeline — proof pack, AI review, and your approval
  are always on. Want a custom shape instead? Build a Workflow →" (links to
  the Workflows page).
- On each **run row** it becomes a live mini-rail: filled segments up to the
  current stage, tone-colored (accent while active, green done, red failed,
  amber at the gate). Replaces nothing — augments the row.
- In **RunDetail** the full rail renders bound to `run.status` above the
  existing event timeline (current stage pulses; past stages green; failure
  marks the stage it died on).

**Source affordance.** A row of the 8 source chips (Jira, Confluence, GitHub
PR, GitHub issue, Product story, Finding, Failing test, Report) with icons;
clicking one inserts its prefix template (`jira:` / `finding:` …) into the
input and focuses it. The dead `src-{kind}` classes get real per-kind accent
colors used consistently on chips, detect badge, run rows, and detail header.

**Controls row (launch parameters):**
- **Repo**: `<select>` of workspace repos (name, path title) — default = first
  repo (matches engine fallback, labeled "(default)"), plus source-implied
  note stays automatic; **Browse…** opens `FolderPicker` (`gitOnly`) → picked
  path → `POST /workspaces/{ws}/repos/detect` → repo appears in the select,
  selected. Sends `repo_id` on launch (no backend change).
- **Provider/Model**: single-agent mode → fixed `claude` chip + model input
  with `<datalist>` (`opus`, `sonnet`, `haiku`; free text allowed,
  placeholder "default"); goal-loop mode → provider `<select>` from
  `auth.meta.providers` + the same model input. Sends `provider`, `model`.
- Mode segmented control, Auto-open PR, Run button unchanged.

### 4.4 UI — New goal loop form (GoalDefineForm.svelte)

- **Browse…** button beside Repository path → `FolderPicker` (`gitOnly`) fills
  `repoPath` (stays a path string — loops API takes `repo_path`).
- **Provider + Model** controls in the Budget block (provider select from
  meta, model input with datalist). Applied on launch to every executor
  (`{...base, provider, model}` before the count fan-out) so the AI-suggested
  config is overridden only when the user chose something.

### 4.5 Contracts & docs (lockstep rule)

- `docs/contracts/api.md`: runs launch req gains `model`; run DTO gains
  `model`; note single-agent = claude + `--model`, goal-loop = provider/model
  per executor. CLI-update notice format documented under its section if one
  exists.
- `ui/src/lib/api/types.ts`: `LaunchRunReq.model?`, `OttoRun.model`.
- `docs/features/run-with-otto.md`: launcher walkthrough updated (repo select,
  Browse→detect, model, provider semantics), plus a "Run with Otto vs
  Workflows" comparison block.

### 4.6 Focused E2E / tests (R5)

- **Rust**: `cli_update` `build_body` unit tests; `otto-state` runs test gains
  model round-trip; existing `run.rs`/engine tests untouched (no ordering
  change).
- **`desktop-run-with-otto.spec.ts`** (extend):
  - API: launch with `model: "sonnet"` → GET shows `model: "sonnet"`; launch
    without model → `""`.
  - API: `POST repos/detect` with a subdir of the seeded repo returns the same
    repo id (Browse bridge).
  - UI: pipeline rail visible on the launcher; repo select lists the seeded
    repo; model input present; Workflows link present.
- **`desktop-goal-loop-form.spec.ts`** (new, small): open Loops → New goal
  loop → Browse opens the folder picker modal (fs/browse renders), picking
  fills the repo path input; provider/model controls render. ("Define with
  AI" is NOT driven — needs a live CLI; covered by unit/integration layers.)
- Gates: `cargo build/test/clippy/fmt-check --workspace`, `npm run check`,
  `npm run build`, the two specs above on the isolated E2E daemon
  (`OTTO_E2E_BIN` + slot isolation).

## 5. Risks / gotchas

- **Migration race with concurrent agents**: renumber to the next free number
  at merge time (mission-control-merge-race protocol).
- `otto_runs` INSERT/SELECT column lists must stay aligned (runtime-bound sqlx,
  no offline cache involved).
- Playwright: new spec must match `desktop-*.spec.ts`.
- deploy.sh needs `~/.hermes/node/bin` on PATH.
- Svelte 5: keep run-list updates merge-in-place (don't reset `<details>`).

## 6. Requirements → design map (self-review)

| Req | Where met |
|-----|-----------|
| R1 not plain / shows how it runs | §4.3 pipeline rail (launcher static + row mini + detail live), source chips, per-kind colors |
| R2 vs Workflows | §4.3 one-liner + Workflows link; §4.5 feature-doc comparison |
| R3 folder picker | §4.3 Repo select + Browse→detect (launcher); §4.4 Browse→FolderPicker (goal loop form) |
| R4 model to run | §4.1 end-to-end plumbing (req→DB→engine→`--model`; executors via session meta); §4.3/§4.4 pickers |
| R5 focused E2E | §4.6 |
| R6 worktree/merge/deploy | process (worktree `rwo-polish` active; §5 merge protocol) |
| R7 CLI update | §4.2 truthful per-provider outcomes + live brew fix (verified) |

Self-review notes: (a) rejected a native Tauri dialog — no `tauri-plugin-dialog`
dependency exists and paths must be daemon-host paths, so the existing
`/fs/browse` picker is the correct tool; (b) rejected honoring arbitrary
providers in single-agent mode — would require a headless multi-provider
runner, out of scope, and the UI now states the truth instead; (c) rejected a
models endpoint — models remain free-text-with-suggestions app-wide; inventing
a registry for one form would drift from every other surface.
