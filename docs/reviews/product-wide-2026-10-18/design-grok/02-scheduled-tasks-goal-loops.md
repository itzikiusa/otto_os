# Scheduled Tasks and Goal Loops — design review

Target 9.8. Reviewed light/dark tokens (no hex in these modules), phone/tablet, and the create, edit, run, pause, delivery, budget, progress, loading, empty, error, and retry paths.

| Module | Score | Defects |
|---|---|---|
| Scheduled Tasks | **8.5** | 9 |
| Goal Loops | **8.0** | 11 |
| **Total** | | **20** |

Both pages use one `PageHeader`, `PageBody`, `LoadState`, `EmptyState`, `Modal`, `confirmer` / `confirmOutward`, real buttons, and semantic tokens. The scores are below 9.8 because the live status, pause, and phone create flows still mislead.

## Scheduled Tasks — 8.5

1. **The row badge is the previous outcome, not this run.** `ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte:1152-1157`. `last_status` is written only when a run finishes, and a paused task replaces that badge with only “Paused”. After Run now, or during a scheduled run, the title still says Succeeded (or nothing) while the open run says Running; a paused failure disappears. A finished run with `delivery_error` still badges Succeeded beside “Delivery failed” (`1209`).
   **Fix:** One cluster on the title. If a loaded run (or the in-flight id from Run now / `scheduled_task_run_updated`) is running, show Running and hide the stale outcome. If paused, show Paused and the last outcome. If the newest loaded run has `delivery_error`, add a warning “Not delivered” and do not leave Succeeded as the only word.

2. **Pause and Resume are hidden; Run now reads as resume.** `ScheduledTasksPage.svelte:701` and `1168-1171`. Pause/Resume exist only in the ⋯ menu. On a paused task the visible verb is still “Run now”, which runs once and leaves the schedule paused.
   **Fix:** When the task is paused, the leading button is Resume. Rename the other action “Run once” (keep the outward confirm). Leave Pause in the menu only while the task is enabled, next to Runs.

3. **The default cadence lies until after save.** `ScheduledTasksPage.svelte:397`, `411`, `951-954`. Interval is the default, values under 5 are stored as 5 with no message, and the next-fire preview is skipped for interval (`previewKey` returns `''`). Daily, weekly, cron, and once show the real fires; the common case does not.
   **Fix:** On blur or submit, if the value is under 5, set an inline error “The shortest interval is 5 minutes” and do not clamp inside `buildSchedule`. Call the same preview for interval so the form shows the next fire before Create.

4. **A failed save is easy to miss.** `ScheduledTasksPage.svelte:487-516` and `859`. Name, workflow, email, webhook, timezone, cron, and “once” errors are one banner at the top of a long form. Create sits at the bottom; nothing scrolls to the banner or the field, and the fields are not marked except cron and timezone.
   **Fix:** Put the message under the invalid control (`aria-invalid` + `aria-describedby`), focus that control, and `scrollIntoView` it. Keep the banner only for a daemon failure, and send that through `toastError` / `loadErrorText` (`575-583`, `640`) instead of raw `e.message`.

5. **Shell commands and ids follow the UI direction.** `ScheduledTasksPage.svelte:917`, `931`, `1041`. The shell prompt, custom provider slug, and chat/channel id are `dir="auto"`. In RTL a command or channel id can reorder. Timezone (`990`) is an IANA id and should be `dir="ltr"` too.
   **Fix:** `dir="ltr"` on those four. Leave the agent prompt and the task name on `dir="auto"`.

6. **No workspace still invites a full form.** `ScheduledTasksPage.svelte:1132-1140` and `537-538`. Goal Loops blocks this with an empty state. Here the empty state offers New task, and the only failure is “Pick a workspace first” after the form is filled.
   **Fix:** Match Goal Loops: if `!ws.currentId`, page `EmptyState` “Add a workspace” and no New task. If the form is already open, show the same block at the top and disable Create.

7. **A dropped event socket leaves “Running” with no way out.** `ScheduledTasksPage.svelte:848-852` and `1183-1189`. Runs refresh only from the websocket (`scheduledTasks` store `applyEvent`). The header has no Refresh. An expanded running row never says the socket is offline, and Retry appears only after a failed load.
   **Fix:** A header Refresh (`data-overflow`) that reloads the list and any open runs. When `events.state !== 'connected'` and a run is running, a `role="status"` line: “Reconnecting — this run’s status may be stale.”

8. **⌘K can only create.** `ScheduledTasksPage.svelte:76-78`. Run once, Pause/Resume, and Edit are not registered, so the keyboard path for the verbs on the row does not exist.
   **Fix:** While a task is expanded (or is the only one), register “Run {name} once”, “Pause {name}” / “Resume {name}”, and “Edit {name}…” in the Scheduled Tasks group. Unregister them when the form is open.

9. **The Advanced hint widens a phone.** `ScheduledTasksPage.svelte:1334`. `.adv-hint` is `nowrap` with the default `min-width: auto`, so “skill, working dir, sandbox, retries, delivery rules” overflows the page instead of ellipsising.
   **Fix:** `.adv-hint { flex: 1; min-width: 0 }`.

## Goal Loops — 8.0

1. **The phase stepper is blank whenever the loop is not running.** `ui/src/modules/loops/LoopDetail.svelte:247-249`. `cur` is forced to `-1` unless `status === 'running'`. A paused, blocked, exhausted, or failed loop shows four inactive steps, including one that stopped mid-Execute. A succeeded loop does not mark the steps done. The list card does show the phase (`LoopsPage.svelte:184`); the detail, where you act, does not.
   **Fix:** Succeeded: every step `past`. Paused, blocked, exhausted, failed, stopped: `aria-current` on the real phase, no pulse. Keep the pulse only while running.

2. **The progress bar is “live” green for dead loops.** `LoopsPage.svelte:274-280` and `LoopDetail.svelte:428-435`. `--status-working` is the fill unless `succeeded`. Failed, exhausted, paused, and stopped cards use the working colour. The badge is a second signal, but the bar still reads as in progress. The list bar is `aria-hidden` (`180`), so the colour is doing the work.
   **Fix:** Map the fill from `loop.status`: `--info` running, `--warning` blocked/exhausted, `--danger` failed, `--success` succeeded, `--text-dim` paused/draft/stopped. Same classes on the list and the detail.

3. **Met, not met, and not checked look the same.** `LoopDetail.svelte:302-304`. Anything other than `met === true` is a dim `dot`. Not met vs not evaluated is only a `title`, which a phone cannot reveal. The evidence field is named with the id (`312`, `Evidence for ${c.id}`), so VoiceOver says “c1” instead of the criterion.
   **Fix:** Visible text on the row: “Met”, “Not met”, or “Not checked”, with `check` / `x` / `minus`. `aria-label={`Evidence for ${c.text}`}`.

4. **The next action and the primary button disagree.** `LoopDetail.svelte:201-206` and `278-280`. While a question is open, Resume (or Extend & resume) is the primary and disabled. Record answer is a quiet `.btn.small`, and the reason is a `title`. The extend sheet’s submit is also `disabled={unanswered}` (`386`) with only a title.
   **Fix:** While `unanswered`, Record answer is the only `.btn.primary` (in the decision, not the header). Header Resume stays a disabled secondary with a visible hint under the question: “Resume stays off until this answer is recorded.” Do not open Extend until that is done.

5. **Extend budget is an unstyled pair of inputs.** `LoopDetail.svelte:379-381`. They are not `.input`, so in dark mode they stay the native light control (white well, default focus) inside an otherwise tokenised sheet. They also miss the shared focus ring.
   **Fix:** `class="input"` on both, same as the create-form budget fields.

6. **Delete sits on the primary.** `LoopDetail.svelte:212-216`. Draft shows the trash icon immediately before Start. Succeeded/failed/stopped show only that trash (`217-219`), which is fine; the draft pair is the mis-tap.
   **Fix:** Use the same ⋯ menu as running/paused (Stop, Delete…) so Start is the only header button on a draft.

7. **The create form scrolls sideways on a phone.** `GoalDefineForm.svelte:236-244` (`.frow` at `421-428` does not wrap), `228-231` (`.repo-row` at `483-487`), and the Advanced hint at `445-450`. After a draft exists, Re-define + the refine field + Refine is one nowrap row. The hint “sources, skills, planner / evaluator / digester agents” cannot shrink. The list grid `minmax(260px, 1fr)` (`LoopsPage.svelte:209`) overflows a zoomed phone.
   **Fix:** `.frow { flex-wrap: wrap }`, `.grow { min-width: 0 }`, stack the refine controls under 640px. `.adv-hint { flex: 1; min-width: 0 }`. Grid: `minmax(min(100%, 260px), 1fr)`.

8. **Draft copy and direction.** `GoalDefineForm.svelte:239` and `242`. The first primary says “Define with AI” / “Re-define” (Otto’s word is not “AI”; Re-define and Refine both call `define`). The refine field is human language forced `dir="ltr"`, so a Hebrew note lays out left to right. Launch (`342-346`) stays disabled with the reason only in `title`, which touch does not show.
   **Fix:** “Draft goal”, then one “Update draft” that sends the feedback field. `dir="auto"` on that field. If `draft` exists and `!canLaunch()`, an inline hint under the criteria: “Each criterion needs a description and how it is verified.”

9. **Iteration agents and the embedded session.** `IterationRow.svelte:66-78` and `129`: done and running are both `--status-working`, and an empty note prints the raw status (`running`, `error`). `LoopDetail.svelte:335-343` and `671-676`: Open session mounts the full `SessionView` in a fixed 360px box, always `focused={true}`, with no Open in Agents. On a phone that pane is not a usable transcript and it takes session keys while you answer a question.
   **Fix:** `StatusDot` / `StatusBadge` with `runStatus(a.status)` and never the raw enum. On phone, `ws.navigateToSession(sid)` instead of embedding. On desktop, let the pane fill the remaining height, set `focused` only while it contains focus, and add “Open in Agents”.

10. **⌘K can only create.** `LoopsPage.svelte:113-116`. Pause, Resume, Extend budget, and Stop are missing while a loop is open.
    **Fix:** Register those four in the Goal Loops group for the open loop, and unregister them on the list.

11. **Lifecycle toasts show the raw exception.** `LoopDetail.svelte:79-90` and `103`. Pause, resume, stop, and delete pass `e.message` through. A down daemon surfaces as a TypeError or status string.
    **Fix:** `toastError`, as the define/launch form already does.
