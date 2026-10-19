# Iteration 01 — automation lane (Grok)

Product-wide review, this lane only. No score is 9.8. Cargo was not run. The new tests are written and unexecuted. Doc tests and the UI were not part of this pass.

## Reviewers actually started

Live Otto sessions on this workspace when the lane opened (68 sessions listed, 3 live):

| ID | Provider | Title | Status | Role here |
|---|---|---|---|---|
| `01M57PYK75FC1F2AFS4C73BBF7` | codex | root | working | Owns sessions/PTY, workflow engine, git. Received both findings messages. |
| `01M57PYTS624JJ0SDSJ4RKSJPS` | agy | Ronaldo | working | Already working. No second task was queued onto this session. |
| `01M57PYXXY6TK198E7FN6AGXMS` | grok | Pele | working | This lane. |

59 reconnectable and 6 exited sessions were not counted as reviewers.

Grok subagents started for this lane. All six were still reading when this report was written. Their notes are not folded in yet.

| ID | Coverage |
|---|---|
| `01a14ff8-76af-77e0-9d76-76fee1df78f1` | `otto-automation` scheduled-task engine, cadence, report delivery |
| `01a14ff8-76af-77e0-9d76-7706483b43ff` | goal-loop engine, policy, commands, workspace |
| `01a14ff8-76af-77e0-9d76-77179e4a3fdb` | `otto-assistant` chat, tasks, personal-agent engine and scheduler |
| `01a14ff8-76af-77e0-9d76-77206722c554` | `otto-swarm` runtime |
| `01a14ff8-76af-77e0-9d76-7730654b009c` | UI: scheduled tasks, goal loops, workflow screens |
| `01a14ff8-76af-77e0-9d76-774de2177cb5` | UI: swarm, personal agents, assistant |

This is 3 live sessions plus 6 subagents. It is not 20 engineering reviewers and not 10 design reviewers.

## Scores

| Surface | Score | Why it is not 9.8 |
|---|---:|---|
| Scheduled-task supervisor | 9.0 | Tick, in-flight claim, and startup reap were traced. The run engine and delivery path were not finished in this pass. |
| Goal loops | 8.6 | Human criteria cannot be marked met by the model. The controller, kill, and evidence capture were not finished. |
| Personal-agent scheduler | 9.1 | Per-agent guard, cursor, and proactive budget fail-closed were traced. The run engine was not. |
| Assistant chat and tasks | 7.8 | Not traced end to end. The assistant reviewer is still reading. |
| Swarm orchestration | 8.4 | Two fail-open starts are fixed in owned files. The tests have not run. The rest of the runtime is still under review. |
| Scheduled-tasks UI | 8.7 | Named browser cases are in the 2026-10-19 section. Not every `02-ST` id has its own assertion. |
| Goal-loops UI | 8.9 | Ronaldo owns this UI and reported 15 workflows/loops cases. Not repeated here. |
| Workflow UI | 8.2 | Ronaldo owns this UI and reported 15 workflows/loops cases. Not repeated here. |
| Swarm UI | 8.5 | Named browser cases are in the 2026-10-19 section. Score stays under 9.8. |
| Personal-agents UI | 8.5 | Named browser cases are in the 2026-10-19 section. Score stays under 9.8. |
| Assistant UI | 8.6 | Named browser cases are in the 2026-10-19 section. Score stays under 9.8. |

## Confirmed bugs fixed in owned files

### Channel launch started a swarm when its status could not be read

`crates/otto-swarm/src/runtime/channels.rs`, `channel_start_refused`. The load was `get_swarm(...).await.ok()?`. `None` means "start is allowed". `launch` then calls `set_swarm_status(..., "active")` (`channels.rs` around the start block after the refusal check). A database error or a missing row therefore skipped the aborted and pause checks.

The load now returns `status_load_refusal(Err(...))`, which is a refusal string: "its status could not be read (...) — not started". The pure table is `status_load_refusal`. The existing `start_refusal` cases are unchanged.

Regression, not executed: `status_load_refusal(Err("db down"))` is `Some` and contains "could not be read"; aborted is still refused; active with no pause is still allowed. Same test module as `a_channel_never_restarts_an_aborted_or_auto_paused_swarm`.

### Scheduled swarm tick treated a failed busy read as an open slot

`crates/otto-swarm/src/runtime/scheduler.rs` `tick`. `active_run_count` used `unwrap_or(0)` and `agent_has_active_run` used `unwrap_or(false)`. Either error let the tick continue into `reserve_scheduled_run` while the board might already be full or the agent might already be running.

A failed count or a failed busy read now `continue`s, with a warning. Over cap still skips before the busy query. `schedule_slot_open` is the decision used once both reads succeed, and its `None` arms stay closed.

Regression, not executed: `failed_cap_or_busy_read_does_not_open_a_scheduled_slot`.

`rustfmt --edition 2021` was run on both files. Cargo was not.

## Traced and left as they are

Scheduled tasks (`scheduled_tasks_scheduler.rs` `tick`, lines 76–108): a task that is not due is skipped; a task that cannot claim `in_flight` is skipped without advancing the cursor; the claim is held across the spawned `run_task`. `reap_interrupted` (lines 45–58) runs on the boot path, and a workflow handoff is resumed instead of reaped. This matches the comment at the top of the file.

Personal agents (`personal_agents_scheduler.rs` `tick`, lines 54–86) use the same claim-before-spawn rule, one guard per agent. `proactive_tick` (lines 105–113) returns without spending a run when the 24-hour count query fails.

Goal-loop policy (`goal_loop_policy.rs` `reconcile_human`, lines 23–52): a `verify_kind == "human"` criterion is met only from `ledger.verification`. A model `achieved` verdict with that criterion unmet becomes `blocked`. Test `model_cannot_satisfy_human_criterion` asserts that. Not re-run.

Goal-loops UI (`LoopsPage.svelte` lines 145–157): no workspace, loading, error with retry, and empty each have a state. Scheduled tasks (`ScheduledTasksPage.svelte` `loadDestReady`, lines 128–137): a failed integration or sender read offers every destination (`chan` falls open to `true` when the list is `null`). That is a form convenience, not a send. The daemon still rejects an unconfigured send later. Not changed.

Swarm channel pause (`channels.rs` `start_refusal`): `start_refusal("paused", None)` is `None`, and the existing test locks that. A paused swarm with an empty pause reason can still be started from a channel. Not changed.

## Shared server, not edited

`crates/otto-server/src/routes/usage.rs` `check_budget` (around lines 659–667) says an engine error reads as not exceeded. `crates/otto-server/src/swarm_host.rs` `budget_blocked` (lines 104–107) returns `verdict.blocked.then_some(...)`, so that error becomes "not blocked". A channel launch or a scheduled swarm can pass a spend cap when the spend read fails. Root owns this file. A comment in `check_budget` already says failing open on a cap can let a blocked run through.

## Still outside this score

Workflow engine, session spawn, and git stay with root. ClickHouse and database parity are a different review. The six subagents above had not returned when this file was first written. Named browser evidence for the owned UI lane is the 2026-10-19 section below. Workflows and goal loops are Ronaldo’s.

## Named browser evidence (2026-10-19)

No Cargo, no new reviewers, no Canvas retest, no workflows or loops specs. Owned UI is frozen after the remainder below. Binary `target/debug/ottod`. Slot 18, daemon `7816`, UI `5214`, projects `desktop-browser` and `iphone-portrait`. **64 passed, 0 failed, 25.1s.**

Design-grok closure inventory OPEN count for these four modules is 0. A row below is “this run” only when that case asserted the behaviour. Other FIXED ids stay source-closed in `design-grok-closure-crosscheck.md` and were not given a new browser assertion here.

| Finding IDs | This run |
|---|---|
| `02-ST-01` | `page renders the seeded task; Run now produces a report row` |
| `02-ST-03` | `scheduled task form: "Next fires" comes from the daemon before Save` |
| `02-ST-04` | `scheduled task deep link and save retry preserve newer edits and the leave decision` |
| Webhook copy, `02-ST-02`, `02-ST-05`, `02-ST-06`, `02-ST-07`, `02-ST-08`, `02-ST-09` | No dedicated assertion. `create form exposes v2 controls` shows provider, cron, timezone, sandbox, and toggles. `report formatter load failure offers recovery and keeps plain text usable` covers the lazy report. |
| `03-07` | `composer: @codex routes one turn, the model chip pins the thread, the mic is honest` (no Dictate control) |
| `03-01` | `agent detail: arrow keys and End move focus with selection`; `agent detail: RTL arrow keys follow visual tab direction`. Phone `<select>` was not asserted. |
| `03-05` rooms surface | `rooms: live messages append from the WS event without a refetch`; `rooms: drafts belong to their conversation`; `rooms: a pending send neither duplicates nor clears newer text`; `rooms: a late previous-room reply cannot replace the selected conversation`; `rooms: opens the latest 200 messages and pages earlier evidence`; `rooms: failed messages never claim an empty conversation and Retry recovers`; `rooms: phone conversation keeps its header and composer in view`; `rooms: pending creation prevents duplicate Enter and preserves newer name` |
| `03-02`, `03-03`, `03-04`, `03-06`, `03-08`–`03-14`, `18-08`, `pw-rev` 10–18 | Inventory FIXED. This run did not assert delivery confirm, activity Approve/Stop, member remove, invite copy, share revoke, handover, or composer hit size. Related cases that did run: memory inspector; autonomy save; autonomy draft leave-guard; personal-agent failed save; three list A-B-A visits; schedule refresh Retry in light, dark, and phone; agent form keeps typed fields; Memory tab; Tasks limit; reminder cards; failed thread list Retry; phone thread Back. |
| `04-Swarm-02`, `18-05` | `all five tabs are reachable (switcher scrolls on a phone) and each fits` (`.seg-tabs` overflow, then Org, Graph, Board, Runs, Feed) |
| `04-Swarm-08` | `goals panel: a task’s goals render with status` (`Runs under 2 minutes`) |
| `04-Swarm-01` | `Kanban view: columns scroll horizontally`; `swarm board: pending creation`; `swarm board: project change clears selection`; `swarm board: keyboard assignment and move`. No 320px clip measurement of Stop. |
| `04-Swarm-04` | `settings: standing goals tab + the Triggers tab show seeded data` (seeded `@team`, not the reply confirm) |
| `04-Swarm-07` | `Graph view: the agent org graph renders`; org expansion in native/warm/pro-dark, light and dark, 375/834/1440 |
| `04-Swarm-03`, `04-Swarm-05`, `04-Swarm-06`, `pw-rev-03` | Inventory FIXED. This run did not fail a task load, open Stop verification, or read goal-chip case. |
| `04-MC-02` | `Mission Control: pending summary is not zero and load is explicit` |
| `04-MC-04` | `phone/tablet detail sheet sits above the BottomNav` |
| `04-MC-01` | `switches to the graph view`; `Mission Control: keyboard switches view tabs with focus`. No colour-plus-label pixel check. |
| `04-MC-03` | `renders a sized, non-overflowing page with seeded work`; `opening a work item reveals its traceable detail (timeline)` |

Also passed in the same 64, without a separate finding id: swarm list, header overflow into ⋯, rail collapse, Org tree scroll, Runs filters, Feed fit, session panel width, feed Retry, RTL tabs, phone composer width, feed draft keep, desktop-to-tablet rail, tablet picker focus. Workflow and goal-loop specs were not started.

### Remainder and snapshot

The 64 left named cases in the same lane unrun. Those ran next. Workflow and loop specs stayed out. No Cargo. Owned UI stayed frozen through the iPad rerun, then the two unit harness files below were the only reopen.

Command, from `ui/`, slot 19:

```bash
OTTO_E2E_BIN=/Users/itziklavon/otto_os/target/debug/ottod OTTO_E2E_SLOT=19 OTTO_E2E_PORT=7818 OTTO_E2E_PW_PORT=5218 OTTO_E2E_SWEEP_ORPHANS=0 \
  ./node_modules/.bin/playwright test --project=desktop-browser --project=iphone-portrait \
  --grep "opens on the latest thread|memory chip Undo|approval card shows where|purchases never offer|Permissions is an honest|Settings → Assistant saves|empty assistant invites|no critical axe|older selected thread stays open|slug input|keyboard tab selection moves focus|memory edits preserve notes|context stays separate|keeps the persistent rail|empty rail names the workspace|handover preserves the brief|rooms: the list shows when each room"
```

Count: **16 passed, 1 skipped, 0 failed, 22.8s.** The skip is `wide (≥641px)` on `iphone-portrait` (`phoneWidth`). Daemon data dir `/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-e2e-A6OJNI`.

That skip was then run on `ipad-portrait`. The first attempt (slot 20, daemon 7820, UI 5220) failed: auto-open collapsed the tablet rail, so `.swarm-item` was `aria-current` and not visible (`1 failed`, 45.2s timeout). `SwarmPage` now collapses the rail only when `viewport.isPhone`. Rerun:

```bash
OTTO_E2E_BIN=/Users/itziklavon/otto_os/target/debug/ottod OTTO_E2E_SLOT=21 OTTO_E2E_PORT=7822 OTTO_E2E_PW_PORT=5222 OTTO_E2E_SWEEP_ORPHANS=0 \
  ./node_modules/.bin/playwright test --project=iphone-portrait --project=ipad-portrait \
  --grep "keeps the persistent rail|Swarms rail collapses"
```

Count: **2 passed, 2 skipped, 0 failed, 5.4s.** Passed: iPhone rail collapse, iPad persistent rail. Skipped: wide case on iPhone, phone case on iPad.

New assertions from this remainder:

| Finding IDs | Case |
|---|---|
| `02-ST-05` | `create form: "Custom…" provider reveals a slug input and persists it` |
| `03-01` | `personal agents: keyboard tab selection moves focus to the selected tab` |
| `03-04` | `the approval card shows where / who / why / what and approves with a scoped always-allow` (assistant card, not the agent Activity row) |
| `03-13` | `handover preserves the brief and archives only after confirmed receipt` |
| Tablet rail | `wide (≥641px): keeps the persistent rail (no phone collapse) and fits` on `ipad-portrait` |

Still without a dedicated browser assertion: webhook copy, `02-ST-02`, `02-ST-06`, `02-ST-07`, `02-ST-08`, `02-ST-09`, `03-02`, `03-03`, `03-06`, `03-08`–`03-12`, `03-14`, `18-08`, `pw-rev` 10–18, `04-Swarm-03`, `04-Swarm-05`, `04-Swarm-06`, `pw-rev-03`. Inventory OPEN count for the four modules remains 0. Scores stay under 9.8.

Snapshot for both remainder commands: HEAD `18a21bfd00cf8724c4fe81d355af1a74d3247394`. `target/debug/ottod` sha256 `558f65b36e79719fe563e83935ed7ac7633259cd2f0f8a900f2a284896dc4ccf`, 511184376 bytes, mtime `2026-10-19 00:43:55`. The rail change is UI source served by Vite; the daemon binary was not rebuilt.

Slot 18 command for the 64, same binary and HEAD:

```bash
OTTO_E2E_BIN=/Users/itziklavon/otto_os/target/debug/ottod OTTO_E2E_SLOT=18 OTTO_E2E_PORT=7816 OTTO_E2E_PW_PORT=5214 OTTO_E2E_SWEEP_ORPHANS=0 \
  ./node_modules/.bin/playwright test --project=desktop-browser --project=iphone-portrait \
  --grep "report formatter load failure|Next fires|schedule refresh failure retains|memory inspector:|personal agent: failed save|session panel: the opened session|swarm feed: failed refresh|swarm tabs: RTL|swarm feed: phone composer|org expansion and editor focus|rooms: drafts belong|rooms: a pending send|rooms: live messages append|rooms: a late previous-room|rooms: opens the latest 200|goals panel:|Kanban view: columns|Runs view: filters|phone/tablet detail sheet|switches to the graph view|opening a work item reveals|renders a sized, non-overflowing|feed: the new verification|settings: standing goals|Mission Control: pending summary|Mission Control: keyboard switches|rooms: failed messages never claim|rooms: phone conversation keeps|agent detail: arrow keys|agent form: losing workspace|agent detail: RTL|desktop-to-tablet swarm|swarm board: pending creation|swarm board: project change|rooms: pending creation prevents|swarm board: keyboard assignment|tablet swarm: picker|all five tabs are reachable|Swarms rail collapses|swarm header keeps the title|Feed view: the board|Org view: the hierarchy|opening a swarm shows the header|Graph view: the agent org|swarm page loads|autonomy: proactive budget|autonomy draft blocks|scheduled task deep link|page renders the seeded task|swarm feed: finishing a post|personal agent list A-B-A|composer: @codex routes|phone: the list is the page|a failed thread list shows inline|Tasks tab decides a limit|Memory tab: profile save|reminder and browser cards|create form exposes v2|Custom… provider"
```

Count: **64 passed, 0 failed, 25.1s.** Daemon data dir `/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-e2e-NQuLvk`. Lane total of these three green commands: **82 passed, 3 skipped, 0 failed.** The slot 20 failure is superseded by slot 21.

### Unit harness (combined suite 2060 pass / 2 fail)

The combined log `/tmp/otto-product-wide-ui-unit-r4.log` failed two extracted-function bindings. Assertions were left as written.

- `orchestrationOwnership.test.ts` `extended budget survives Resume failure and retry skips repeat PATCH` calls `loadErrorText` inside `LoopDetail.extendAndResume`. The compile context passes the real helper from `ui/src/lib/loadError.ts`.
- `platformRecovery.test.ts` `template retry preserves the first created agent and schedule request key` calls `deliversReports` inside `AgentEditSheet.save`. The extract list includes the real declarations `save`, `buildDelivery`, and `deliversReports`. The same real `loadErrorText` is the catch binding. With `fDestType: 'none'`, `deliversReports` returns false before any destination confirm.

Command, from `ui/`:

```bash
node --test unit/platformRecovery.test.ts unit/orchestrationOwnership.test.ts
```

Count: **25 passed, 0 failed, 978 ms.** The extended-budget assertions still require `/Budget saved/`, one `updateLimits`, two `resume` calls, and `extendOpen` false on retry. The template-retry assertions still require the schedule error text, one create, two attempts, and the same agent id and `idempotency_key`.

No Cargo. Owned UI and these two harness files are frozen. Workflows and loops modules were not edited.

### Personal-agents page budget

`/tmp/otto-product-wide-ui-build-r4.log` (00:28) measured `page:personal-agents` at 74.7 KB gzip against the 71.9 KB budget. That build had no separate `AgentPage` chunk: the agent page, its documents, and their markdown stayed in `PersonalAgentsPage`’s static closure (`PersonalAgentsPage-CKvsMQUc.js`, 79.62 KB raw / 24.86 KB gzip).

`PersonalAgentsPage.svelte` now loads `AgentPage` and `AgentEditSheet` through `lazyComponent`. The list and agent channels stay on the page. Product and Usage were not edited. `scripts/bundle-budget.json` was not raised.

Fresh command, from `ui/`:

```bash
npm run build && node scripts/bundle-budget.mjs
```

`page:personal-agents` is **24.5 KB** gzip, budget 71.9 KB, status ok. `AgentPage-ZQp8MiL8.js` is its own chunk (54.80 KB raw / 17.62 KB gzip). Log: `/tmp/otto-pele-ui-build-pa.log`, budget table `/tmp/otto-pele-ui-budget-pa.log`. The script also notes the page shrank past the ratchet tolerance; `--update` was not run.

### Three owned page budgets

`/tmp/otto-bundle-current.log` (00:35) still lists the pre-split sizes: mission-control 33.8 vs 32.8, scheduled-tasks 44.7 vs 43.3, personal-agents 74.7 vs 71.9. Source saved after that log already moves the weight off the static page closure:

- `PersonalAgentsPage` loads `AgentPage` and `AgentEditSheet` with `lazyComponent`.
- `ScheduledTasksPage` loads `renderMarkdownGfm` with a dynamic `import('../../lib/md')` when a formatted report is open.
- `WorkItemDetail` loads the swarm store only on the swarm-kind open path. `WorkGraphView` stays on the mission-control page.

No file under `ui/src` is newer than `ui/dist/.vite/manifest.json` (01:36). That dist was not rebuilt again. `node scripts/bundle-budget.mjs` against it (`/tmp/otto-pele-bundle-owned.log`) reports all three ok, and the baseline file was not updated:

| Target | Gzip | Budget |
|---|---:|---:|
| `page:mission-control` | 28.5 KB | 32.8 KB |
| `page:scheduled-tasks` | 30.0 KB | 43.3 KB |
| `page:personal-agents` | 24.5 KB | 71.9 KB |

The script notes those three shrank past the ratchet tolerance, along with `page:usage` and `panel:canvas`. `--update` was not run. Product and Usage source were not edited.

Harness, from `ui/`:

```bash
node --test unit/platformRecovery.test.ts unit/orchestrationOwnership.test.ts
```

**25 passed, 0 failed, 957 ms.** Assertions unchanged.

Named browser, slot 22, daemon 7824, UI 5224, data `/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-e2e-X64uM6`. No Cargo. Existing `target/debug/ottod`.

```bash
OTTO_E2E_BIN=/Users/itziklavon/otto_os/target/debug/ottod OTTO_E2E_SLOT=22 OTTO_E2E_PORT=7824 OTTO_E2E_PW_PORT=5224 OTTO_E2E_SWEEP_ORPHANS=0 \
  ./node_modules/.bin/playwright test --project=desktop-browser --project=iphone-portrait \
  e2e/desktop-scheduled-tasks.spec.ts e2e/desktop-personal-schedule-recovery.spec.ts \
  e2e/desktop-ux-r2-automation.spec.ts e2e/desktop-ux-r3-automation.spec.ts \
  e2e/desktop-ux-automation.spec.ts e2e/mission-control.spec.ts \
  --grep "report formatter load failure|page renders the seeded task|schedule refresh failure retains|agent detail: arrow keys|personal agent: failed save|keyboard tab selection|Mission Control: pending summary|Mission Control: keyboard switches|renders a sized, non-overflowing|opening a work item reveals|switches to the graph view|phone/tablet detail sheet"
```

**12 passed, 0 failed, 10.2s.** Covers the formatted-report import, the lazy agent page and edit sheet, and Mission Control list, graph, timeline, and phone sheet.

Owned UI, the two harness files, and these three page budgets are frozen. Workflows and loops were not edited.

### Reconfirmed after the reservation-filter cancel

No Cargo. The state 476 / swarm 65 refund run was not repeated. `ui/src` is unchanged since the 01:36 dist.

Harness again: **25 passed, 0 failed, 956 ms.** Log `/tmp/otto-pele-harness-ack.log`.

Budget again, no `--update`: mission-control **28.5 / 32.8 KB**, scheduled-tasks **30.0 / 43.3 KB**, personal-agents **24.5 / 71.9 KB**. Log `/tmp/otto-pele-bundle-ack.log`. The 74.7 / 44.7 / 33.8 figures are the 00:35 table, from before these lazy splits.

Named browser slot 23, daemon 7826, UI 5226, data `/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-e2e-0eVCSC`: **23 passed, 2 skipped, 0 failed, 16.0s.** Skips are the phone rail case on iPad and the wide-rail case on iPhone. Desktop covers the report formatter, seeded scheduled task, agent schedule retry, agent detail keys, personal-agents tabs, failed agent save, assistant composer, and Mission Control pending summary plus keyboard tabs. iPhone and iPad cover Mission Control size, timeline, graph, and phone sheet, plus swarm tabs, goals, phone rail, and the iPad persistent rail.

### Harness files transferred

`ui/unit/orchestrationOwnership.test.ts` and `ui/unit/platformRecovery.test.ts` are Ronaldo’s. They were not edited in this pass.

The three page budgets were remeasured with `node scripts/bundle-budget.mjs` (`/tmp/otto-pele-bundle-own.log`). `ui/src` is unchanged since the dist. No `--update`:

| Target | Gzip | Budget |
|---|---:|---:|
| `page:personal-agents` | 24.5 KB | 71.9 KB |
| `page:scheduled-tasks` | 30.0 KB | 43.3 KB |
| `page:mission-control` | 28.5 KB | 32.8 KB |

Named browser slot 24, daemon 7832, UI 5232, data `/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-e2e-Yu52OZ`: **27 passed, 2 skipped, 0 failed, 15.9s.** Same phone/iPad rail skips. Added rooms cases: session-room last active, agent-channel drafts, failed messages with Retry, and pending channel creation. No Cargo.

### Deferred graph, model picker, and agent documents

Ronaldo owns the two harness files. This pass did not edit them.

- `MissionControlPage` loads `WorkGraphView` only after Graph is selected, with `LoadState` for the chunk’s loading and error/retry. `detailPane` remains `bind:this` on `WorkItemDetail`.
- `ScheduledTasksPage` loads `ModelPicker` only inside the create/edit agent form, with the same loading and retry states.
- `AgentPage` already loaded `AgentMemoryInspector` on the Memory tab. Context (and the memory document) now load `AgentDocuments` the same way.

Fresh `npm run build` then `node scripts/bundle-budget.mjs` (`/tmp/otto-pele-bundle-lazy3.log`). No `--update`:

| Target | Gzip | Budget |
|---|---:|---:|
| `page:mission-control` | 27.1 KB | 32.8 KB |
| `page:scheduled-tasks` | 30.2 KB | 43.3 KB |
| `page:personal-agents` | 24.4 KB | 71.9 KB |

The shrink notice is informational. Exit 0.

Named browser slot 25, daemon 7834, UI 5234, data `/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-e2e-Ve8uYc`: **30 passed, 2 skipped, 0 failed, 20.7s.** Graph view passed on iPhone and iPad. Create form, memory inspector, and context document passed on desktop, with the earlier scheduled, assistant, rooms, and swarm cases. No Cargo.

### Personal-agents budget only

`MissionControlPage` and `ScheduledTasksPage` stay with Ronaldo. They were not edited in this pass. The full UI unit run stays **2062 / 2062** and was not repeated.

`AgentPage` loads `AgentMemoryInspector` only on the Memory tab (`lazyComponent`, `LoadState` loading and Retry). `PersonalAgentsPage` still loads `AgentPage` itself the same way, so the memory inspector is outside the page’s static closure. Current dist, source unchanged: `page:personal-agents` **24.4 KB** gzip, budget 71.9 KB, status ok. Baseline not updated.

Named browser slot 26, daemon 7836, UI 5236, data `/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-e2e-ui9VOL`: **17 passed, 2 skipped, 0 failed, 12.1s.** Assistant composer, session rooms, agent channels, swarm tabs/goals/rails, memory inspector, context document, agent detail, and failed save. No Cargo.

### Hold for Ronaldo’s production build

No production build was started from this lane. No `vite build` was running during a 70s wait, and `ui/dist/.vite/manifest.json` stayed at 01:45:54. The full UI unit run **2062 / 2062** was not repeated.

Focused browser after that wait, slot 27, daemon 7838, UI 5238, data `/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-e2e-iHapNu`: **8 passed, 0 failed, 7.9s.** Create form (scheduled model picker), work-item timeline and phone detail sheet (`WorkItemDetail`), memory inspector, context document, schedule refresh, agent detail keys, and failed agent save (`PersonalAgentsPage`). Owned UI is frozen. No Cargo.

### Named browser 27 and 11 (not re-run)

Source is frozen. Backend remains `18a21bfd0`. These two commands were not repeated for this appendix. The 11-pass desktop run is a subset of the 27-pass run. Root’s later full UI unit count **2062 / 2062** predates the lazy imports and is not repeated here.

**27 passed, 0 failed, 15.7s.** Slot 9, daemon 7796, UI 5176, data `/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-e2e-BBYvNS`. Log: `/Users/itziklavon/.grok/sessions/%2FUsers%2Fitziklavon%2Fotto_os/cdc2308c-5f31-481f-a1a2-b6452631a09c/terminal/call-fa2cfa71-d2a5-4298-9b03-174366a96b41-213.log`.

```bash
cd ui && OTTO_E2E_BIN="/Users/itziklavon/otto_os/target/debug/ottod" OTTO_E2E_SLOT=9 OTTO_E2E_PORT=7796 OTTO_E2E_PW_PORT=5176 OTTO_E2E_SWEEP_ORPHANS=0 \
  ./node_modules/.bin/playwright test --project=desktop-browser --project=iphone-portrait \
  e2e/desktop-ux-r3-automation.spec.ts e2e/desktop-personal-schedule-recovery.spec.ts \
  e2e/desktop-review5-personal-agent-ownership.spec.ts e2e/desktop-journeys.spec.ts e2e/mission-control.spec.ts \
  --grep "rooms:|swarm |personal agent|org expansion|schedule refresh|scheduled task form|work item|backfill|approval gate|graph view|BottomNav"
```

| Spec | Test |
|---|---|
| `e2e/mission-control.spec.ts:60` | backfill projected the seeded session and swarm into work items |
| `e2e/mission-control.spec.ts:89` | approval gate: request → needs_approval, decide → cleared |
| `e2e/mission-control.spec.ts:118` | graph view returns nodes (and edges when present) |
| `e2e/mission-control.spec.ts:139` | opening a work item reveals its traceable detail (timeline) |
| `e2e/mission-control.spec.ts:154` | phone/tablet detail sheet sits above the BottomNav |
| `e2e/mission-control.spec.ts:172` | switches to the graph view |
| `e2e/desktop-journeys.spec.ts:98` | run approval gate: evidence links open THIS run’s review and branch (S20-301) |
| `e2e/desktop-journeys.spec.ts:166` | scheduled task form: "Next fires" comes from the daemon before Save |
| `e2e/desktop-personal-schedule-recovery.spec.ts:5` | schedule refresh failure retains cadence and Retry recovers in light, dark and phone layouts |
| `e2e/desktop-review5-personal-agent-ownership.spec.ts:26` | personal agent list A-B-A preserves the current visit: old-success-while-loading |
| `e2e/desktop-review5-personal-agent-ownership.spec.ts:26` | personal agent list A-B-A preserves the current visit: old-error-while-loading |
| `e2e/desktop-review5-personal-agent-ownership.spec.ts:26` | personal agent list A-B-A preserves the current visit: old-success-after-current |
| `e2e/desktop-ux-r3-automation.spec.ts:28` | rooms: drafts belong to their conversation |
| `e2e/desktop-ux-r3-automation.spec.ts:39` | rooms: a pending send neither duplicates nor clears newer text |
| `e2e/desktop-ux-r3-automation.spec.ts:60` | rooms: live messages append from the WS event without a refetch (perf R2/R3) |
| `e2e/desktop-ux-r3-automation.spec.ts:85` | swarm feed: failed refresh retains posts and offers Retry |
| `e2e/desktop-ux-r3-automation.spec.ts:104` | swarm tabs: RTL arrows follow the visible direction |
| `e2e/desktop-ux-r3-automation.spec.ts:161` | swarm feed: phone composer has usable message width |
| `e2e/desktop-ux-r3-automation.spec.ts:176` | personal agent: failed save preserves form and a retry creates the paused agent |
| `e2e/desktop-ux-r3-automation.spec.ts:226` | swarm feed: finishing a post keeps the next message draft |
| `e2e/desktop-ux-r3-automation.spec.ts:269` | rooms: a late previous-room reply cannot replace the selected conversation |
| `e2e/desktop-ux-r3-automation.spec.ts:292` | rooms: opens the latest 200 messages and pages earlier evidence |
| `e2e/desktop-ux-r3-automation.spec.ts:335` | org expansion and editor focus: native light 1440 |
| `e2e/desktop-ux-r3-automation.spec.ts:335` | org expansion and editor focus: warm light 375 |
| `e2e/desktop-ux-r3-automation.spec.ts:335` | org expansion and editor focus: pro-dark dark 1440 |
| `e2e/desktop-ux-r3-automation.spec.ts:335` | org expansion and editor focus: warm dark 834 |
| `e2e/desktop-ux-r3-automation.spec.ts:335` | org expansion and editor focus: native dark 1440 |

**11 passed, 0 failed, 8.6s.** Slot 8, daemon 7798, UI 5178, data `/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-e2e-Kr3X61`. Log: `/Users/itziklavon/.grok/sessions/%2FUsers%2Fitziklavon%2Fotto_os/cdc2308c-5f31-481f-a1a2-b6452631a09c/terminal/call-8b451825-8c9a-4b7b-9dca-c485a79b06a7-107.log`.

```bash
cd ui && OTTO_E2E_BIN="/Users/itziklavon/otto_os/target/debug/ottod" OTTO_E2E_SLOT=8 OTTO_E2E_PORT=7798 OTTO_E2E_PW_PORT=5178 OTTO_E2E_SWEEP_ORPHANS=0 \
  ./node_modules/.bin/playwright test --project=desktop-browser \
  e2e/desktop-ux-r3-automation.spec.ts e2e/desktop-personal-schedule-recovery.spec.ts \
  e2e/desktop-review5-personal-agent-ownership.spec.ts e2e/desktop-journeys.spec.ts \
  --grep "personal agent|schedule refresh|scheduled task form|rooms:"
```

| Spec | Test |
|---|---|
| `e2e/desktop-personal-schedule-recovery.spec.ts:5` | schedule refresh failure retains cadence and Retry recovers in light, dark and phone layouts |
| `e2e/desktop-journeys.spec.ts:166` | scheduled task form: "Next fires" comes from the daemon before Save |
| `e2e/desktop-review5-personal-agent-ownership.spec.ts:26` | personal agent list A-B-A preserves the current visit: old-success-while-loading |
| `e2e/desktop-review5-personal-agent-ownership.spec.ts:26` | personal agent list A-B-A preserves the current visit: old-error-while-loading |
| `e2e/desktop-review5-personal-agent-ownership.spec.ts:26` | personal agent list A-B-A preserves the current visit: old-success-after-current |
| `e2e/desktop-ux-r3-automation.spec.ts:28` | rooms: drafts belong to their conversation |
| `e2e/desktop-ux-r3-automation.spec.ts:39` | rooms: a pending send neither duplicates nor clears newer text |
| `e2e/desktop-ux-r3-automation.spec.ts:60` | rooms: live messages append from the WS event without a refetch (perf R2/R3) |
| `e2e/desktop-ux-r3-automation.spec.ts:176` | personal agent: failed save preserves form and a retry creates the paused agent |
| `e2e/desktop-ux-r3-automation.spec.ts:269` | rooms: a late previous-room reply cannot replace the selected conversation |
| `e2e/desktop-ux-r3-automation.spec.ts:292` | rooms: opens the latest 200 messages and pages earlier evidence |
