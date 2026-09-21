# Goal Loops

Goal Loops runs bounded Plan → Execute → Evaluate → Digest iterations toward concrete acceptance criteria. Build loops use an isolated `goal-loop/<id>` git worktree; research loops use a dedicated directory and produce `findings.md` with cited evidence. Your original checkout is not the executor working directory.

## Define and launch

Open **Goal Loops → New goal loop**, choose Build or Research, select the definer provider/model, and describe the goal. Build mode requires a repository; Research does not. **Define with AI** creates a managed definer session and returns an editable draft. Refine the draft with feedback before launching.

Each criterion specifies how it is verified:

- **Command:** Otto runs the command and uses its exit status as ground truth.
- **Agent assessment:** the evaluator must provide evidence. Existing `manual` criteria retain this meaning.
- **Human verification:** the model cannot mark it accepted. Once work is ready, the loop blocks; record what you personally verified and Resume.

Set iteration, active-time, phase-time and executor-attempt limits. Select executor and planner/evaluator/digester providers/models. Optional source/spec/plan links and skill names are included in agent context. Skill availability is checked by the executing agent, not guaranteed by preflight.

**Allow local commits** defaults off. The executor is instructed to retain uncommitted files for review; local commits require this explicit opt-in. Push, publishing and external messages are not part of automatic completion. **Require independent completion review** adds a separate managed review turn; unresolved findings prevent success.

## Monitor and decisions

The detail view shows status, phase, criterion progress, elapsed budget, retained work path, next action and iteration history. **Agents and roles** lists executor, planner, evaluator, digester and completion-reviewer sessions with provider and state. Open a session inline to inspect its work. Sessions remain identifiable after their live PTYs are released.

The ledger persists questions, answers, verifier identity/evidence, next action and review results alongside the narrative context digest. A blocked question must receive a recorded answer before Resume. Answers do not automatically restart the loop.

Two iterations producing identical unmet criteria and evidence block with a request for a new approach. A human criterion can be accepted only through the verification action while paused, blocked or exhausted. Further executor work invalidates previous human approvals. After final human acceptance, Resume rechecks the existing iteration without rerunning executors or consuming an additional iteration; runtime and phase limits still apply.

## Preservation and recovery

Pause banks active time. Stop releases execution resources and retains the worktree. Success, failure and exhaustion also retain tracked and untracked work. Restart pauses active loops and preserves blocked decisions, so you can Resume explicitly. Deleting loop history does not delete the working directory or branch; the detail view shows the retained path before deletion.

Build proof includes the actual working contents: committed differences from the launch base, staged changes, unstaged changes and untracked files. Verification command output and evaluator evidence are attached to a proof pack. Research reports are attached as evidence. `OTTO_PROOF_REQUIRE_GOAL_LOOP=1` requires passing machine proof on every completion attempt, including the last permitted iteration. Exhausting the budget never relaxes this requirement.

## Limits and troubleshooting

- Executors run sequentially in one working directory.
- Iteration limits gate new work; active-time limits are checked at iteration boundaries, with per-role phase deadlines and a controller backstop.
- Per-loop monetary accounting is not wired. Non-null `max_cost_usd` is rejected; older loops with this setting block until it is removed. Workspace usage-budget checks still gate Define/Launch/Resume.
- If a loop is exhausted, raise iteration/runtime limits through PATCH before Resume. If human acceptance is the only remaining work, it does not require another execution iteration.
- If a provider fails or times out, inspect its retained session and iteration evidence. Missing/noncompliant structured outputs fail the role rather than silently satisfying criteria.
- Agent policy instructions do not constitute an OS permission boundary. Existing session sandbox and tool governance remain responsible for confinement.

## API and implementation

See [API contract](../contracts/api.md#goal-loops) and [WS contract](../contracts/ws.md) for HTTP requests and `goal_loop_updated`. List/detail require workspace Viewer; definition, mutation, verification and answers require Editor. Identity is derived from authentication, not an agent-supplied verifier field.

The controller is `crates/otto-server/src/goal_loop.rs`; managed roles are in `goal_loop_roles.rs`; pure verification decisions are in `goal_loop_policy.rs`; working-directory/evidence helpers are in `goal_loop_workspace.rs`. State lives in `GoalLoopsRepo`, with the ledger added by migration `0133_goal_loop_ledger.sql`. UI lives in `ui/src/modules/loops/`.

## Walkthrough: build, inspect, and finish

1. Open a workspace with a usable repository and an installed, authenticated agent provider. Otto creates a separate worktree from the repository's launch commit. A build goal needs an existing commit; research mode works without Git.
2. Describe an outcome and boundaries: for example, “Add validation to the import screen; keep the existing CSV format; verify the failing-import test and inspect the error message.” Choose the provider/model used to draft the goal, then define it.
3. Review the generated title, objectives, constraints, out-of-scope items and acceptance criteria. Make command criteria concrete commands that can run unattended in the isolated directory. Use Human verification for checks requiring personal inspection; Agent assessment is an automated judgment.
4. Choose bounded limits and executor roles. Executors share one directory and run sequentially. Different providers/models can fill each role. Optional links and named skills guide the work; they do not grant access to unavailable sources or install skills.
5. Create and start. Expand each iteration to inspect its plan, executor summaries, evaluations, digest and sessions. The provider/model on the definer controls drafting; executor and role choices control the subsequent loop.
6. When the loop requests a decision, read the question and record an answer. When automated work is ready but a human criterion is pending, inspect the retained files or running product and record specific evidence. Resume is a separate action.
7. Review the final proof and retained files. Integrating, committing, pushing or publishing the result remains a separate operation. You can use the displayed directory in your editor or Git tools.

## Research workflow

Choose Research to investigate a question without requiring a repository. Add useful source links and request citations in the goal. Otto creates a dedicated directory and requires a nonempty `findings.md`; the report becomes a proof artifact. Add separate acceptance criteria for source quality, confidence, unresolved questions or a human decision. A nonempty report alone does not establish that its claims are correct.

Build/Research mode is fixed when the loop is created, because it determines the working-directory and report-verification requirements. Create another loop to change mode. Research does not automatically crawl private links or enable credentials; agent tools retain their existing access rules.

## Lifecycle controls

| State | Meaning and available action |
| --- | --- |
| Draft | Definition/configuration exists; Start begins execution. |
| Running | A controller or explicit executor retry owns the work. Pause or Stop can interrupt it. |
| Paused | Active time is banked. Once in-flight work has stopped, inspect, verify, raise limits or Resume. |
| Blocked | A decision, repeated failure, human criterion or completion-review finding needs attention. Read the explanation before Resume. |
| Exhausted | A hard limit was reached. Raise the applicable limit, or finish a pending human-only verification continuation. |
| Succeeded | All acceptance criteria and configured completion gates passed. Files remain available. |
| Failed / Stopped | Execution ended; inspect the error or retained output. These are terminal history states. |

Pause interrupts managed roles and verification commands and releases executor sessions. A short “work is still active or stopping” response means cleanup has not finished yet; wait for it to settle before Resume or recording an approval. Stop is asynchronous while cleanup is in progress. Neither operation resets the worktree.

A blocked loop can retry one executor from its **current** iteration using its saved prompt. The retry becomes Running, owns a cancellation handle and clears stale results, approvals and evaluation. It returns to Blocked when finished. Resume then performs fresh loop work/evaluation. This prevents old approvals from being reused for changed files. Two retries cannot run concurrently in the same loop.

## Limits and evidence details

The iteration counter measures iterations started, including interrupted iterations. Pause does not refund elapsed time. Raising limits does not erase previous work or progress. Each managed role has an absolute deadline including session startup and publication. Verification commands have a timeout and are stopped with their process group on cancellation or timeout. Executor recovery remains bounded by attempts and its per-attempt waiting rules.

The narrative digest is auxiliary context; the retained working directory is the source of truth. The ledger preserves questions/answers, identity and evidence for human checks, the latest next action, repeated-failure state and completion-review results. It is not a complete task/file dependency tracker. The failure threshold is currently two matching unsuccessful evaluations and is not configurable.

Human approvals are tied to the exact criterion revision, not only its ID. Revising a criterion or executing further work invalidates earlier approval. Authentication supplies the verifier identity. Session-managed agent credentials cannot call the human verification or question-answer endpoints; the person uses the UI or their own user credential.

The optional completion review is a separate managed session that receives the goal, evaluation and verification evidence. An unavailable, malformed or rejecting review blocks completion. It does not claim independent human approval and does not automatically implement its own findings.

## HTTP and WebSocket surface

All paths below are under `/api/v1`. The authoritative request/response fields are in the [Goal Loops API contract](../contracts/api.md#goal-loops).

| Method and path | Purpose |
| --- | --- |
| `GET /workspaces/{id}/goal-loops` | List workspace loops. |
| `POST /workspaces/{id}/goal-loops/define` | Draft/refine a definition with a managed definer. |
| `POST /workspaces/{id}/goal-loops` | Save the configured goal. |
| `GET /goal-loops/{id}` | Read loop, ledger and iteration details. |
| `PATCH /goal-loops/{id}` | Rename, adjust limits, or edit draft configuration. |
| `DELETE /goal-loops/{id}` | Stop and remove history while retaining files. |
| `POST /goal-loops/{id}/start` | Start a draft. |
| `POST /goal-loops/{id}/pause` | Interrupt work and bank active time. |
| `POST /goal-loops/{id}/resume` | Continue eligible paused/blocked/exhausted work. |
| `POST /goal-loops/{id}/stop` | Cancel work and retain files. |
| `POST /goal-loops/{id}/iterations/{idx}/agents/{agent}/retry` | Retry a current executor while blocked. |
| `POST /goal-loops/{id}/criteria/{criterion}/verify` | Record human evidence using a person's credential. |
| `POST /goal-loops/{id}/questions/{question}/answer` | Record a decision without automatically resuming. |

`goal_loop_updated` signals changes to status, phase, current iteration and progress. Clients fetch detail for the updated ledger, session associations and evidence. Role session IDs are stored before the controller waits for role completion, so reload can rediscover an ongoing role.

## Troubleshooting reference

| Symptom | What to inspect or do |
| --- | --- |
| Build cannot create its worktree | Confirm the repository has a commit and the retained path/branch is not occupied by unrelated work. Otto refuses destructive replacement. |
| Definer/role fails during startup | Inspect provider authentication/configuration and the managed session. The phase deadline includes startup. |
| A role stays running after its chat reply | The role must finish its validated result file; a chat reply alone does not satisfy that protocol. |
| Human verification is rejected | Wait for work to stop, use a person’s credential, provide nonempty evidence, and verify a criterion whose kind is Human. |
| Resume asks for an answer | Record every pending decision question first; answers persist and do not auto-resume. |
| Retry is unavailable | Retry is limited to an executor in the current iteration of a blocked loop, with a retained prompt. Role sessions are inspectable but are not executor retry slots. |
| Completion blocks on proof | Inspect command evidence and report artifacts. Required machine proof is not waived on the last iteration. |
| Completion blocks on review | Read the review findings and use them to decide the next action. Unavailable or malformed review output also blocks. |
| Loop paused after daemon restart | This is intentional recovery. Inspect retained work and Resume explicitly. |
| A skill or source is missing | Make it available through the provider's normal configuration or revise the goal. There is no automatic skill installation or source-access escalation. |

Goal templates/export/import and pre-launch skill-availability resolution are not currently implemented. The comparison proposal describes these as further design opportunities; it is not the API contract.

## Views, fields and defaults

Open the app's **Goal Loops** section. The list shows one card per loop with its name, status pill, progress bar, `iter N/max`, and current phase while running. The empty state offers **Define your first goal**. **New goal loop** opens the definition form.

The AI definition response contains an editable `definition`, `suggested_limits` and `suggested_config`. Refinement sends the previous draft as `context` and your requested changes as `feedback`. Defining creates a managed session but does not create a loop row; creation happens on launch/save.

The form edits Name, acceptance criteria, role configuration and budgets. Each criterion needs a unique ID, a nonempty description and verification instructions; Command also needs a shell command. At least one criterion and executor are required. “What this checks” documents the purpose of a command separately from its executable text.

| Field | Stored setting | Default |
| --- | --- | --- |
| Max iterations | `limits.max_iterations` | 5 |
| Max minutes | `limits.max_runtime_secs`, converted from minutes | 30 |
| Per-phase minutes | `limits.per_phase_timeout_secs`, converted from minutes | 10 |
| Executor recovery attempts | `limits.max_attempts_per_executor` | 3 |
| Controller lifetime backstop | Internal constant | 4 hours |
| Allow local commits | `config.allow_commits` | Off |
| Completion review | `config.require_review` | Off |
| Monetary limit | `limits.max_cost_usd` | Unset; setting it is rejected |

**Launch loop** creates with `autostart:true` and navigates to detail. API clients can save with autostart disabled, edit draft configuration, then use Start. Once execution begins, configuration stays fixed; limits may change while not Running and the name may change while nonterminal.

Detail has a progress bar, the Plan → Execute → Evaluate → Digest stepper, current iteration/active-time totals, retained path/branch, criteria and expandable iteration history. A Waiting executor appears under Executing. Expand an iteration for its plan, Agents and roles, assessment/evidence, and context carried forward. Open a waiting executor session to inspect its prompt or respond. Detail follows WebSocket updates with polling as a fallback.

## Controller and directory reference

`start_loop` provisions the directory, marks Running, registers its exclusive control handle and starts the controller. Each cycle checks limits; creates an iteration; plans; executes sequentially; evaluates command/agent/human criteria; digests; and decides whether to continue, block or assemble proof. A failed planner supplies fallback instructions so executors can work toward the criteria directly. A malformed evaluator result never satisfies omitted criteria. Progress is recomputed from the actual accepted criterion count.

For build goals the directory is `<data>/goal-loops/<id>/work`, on `goal-loop/<id>`. Fresh launch captures HEAD as `base_commit`. Resume reuses a registered worktree unchanged. If someone has removed the directory but the branch survives, Otto can reattach the branch without resetting it; already deleted uncommitted files cannot be reconstructed from that branch. An unrecorded occupied path fails rather than being overwritten. Research uses the dedicated directory without provisioning a Git worktree.

| Layer | Location |
| --- | --- |
| Controller, lifecycle and recovery | `crates/otto-server/src/goal_loop.rs` |
| Managed provider role turns | `crates/otto-server/src/goal_loop_roles.rs` |
| Command cancellation and process cleanup | `crates/otto-server/src/goal_loop_commands.rs` |
| Human verification/progress decisions | `crates/otto-server/src/goal_loop_policy.rs` |
| Worktree provisioning and working-content capture | `crates/otto-server/src/goal_loop_workspace.rs` |
| Tolerant JSON extraction | `crates/otto-server/src/goal_loop_parse.rs` |
| HTTP handlers | `crates/otto-server/src/routes/goal_loops.rs` |
| Domain types/default role prompts | `crates/otto-core/src/domain.rs` |
| Request DTOs | `crates/otto-core/src/api.rs` |
| Persistence | `crates/otto-state/src/goal_loops.rs`; migrations `0065` and `0133` |
| Boot recovery | `crates/ottod/src/main.rs` |
| UI | `ui/src/modules/loops/` |

## Related features

- [Agent sessions](./agent-sessions.md): opening and attaching managed terminals.
- [Code review](./code-review.md): shared agent-run/session plumbing.
- [Agent Swarm](./agent-swarm.md): related bounded multi-agent coordination.
- [Mission Control](./mission-control.md): Goal Loops as work-graph items.
- [API contract](../contracts/api.md) and [WebSocket contract](../contracts/ws.md): authoritative transport definitions.
