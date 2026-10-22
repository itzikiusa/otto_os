# Quality iteration 2 — lens: ux-design

Reviewer: ux-design (static, read-only; no builds, tests or e2e run). Base: `fix/quality-loop-20261021` @ `ab0a86f05`.
Scope excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`,
`ui/src/modules/home/boxes/ClassroomsBox.svelte`, `target/`, `node_modules/`, `docs/reviews/`.

## Score

Rubric: start 10.0; blocker −1.0, major −0.3, minor −0.1, nit −0.03.

| Severity | Count | Deduction |
|---|---:|---:|
| blocker | 0 | 0.00 |
| major | 1 | 0.30 |
| minor | 6 | 0.60 |
| nit | 2 | 0.06 |
| **Total** | 9 | **0.96** |

**Score: 10.0 − 0.30 − 0.60 − 0.06 = 9.04 / 10**

## Previous-finding status (iteration 1)

| Id | Status | Evidence |
|---|---|---|
| ux-design-1-01 | Fixed | `AttachIssue.svelte:18-37` stores `provider`, `url`, `status` from the picker; `InfoPanel.svelte:207-226` renders plain text when `url` is empty and omits the empty chip; `AttachedIssue.provider` is `IssueProviderKind` (`types.ts:4415`). |
| ux-design-1-02 | Fixed | "Attach issue" (AttachIssue Modal, InfoPanel), "Attach story…" (ReviewPanel 1171/1308/1778), "Convert to issue…" / "Creating the issue…" (FindingActions 70, 274). Only identifiers/comments still say Jira. |
| ux-design-1-03 | Fixed | Spot-checked all 13 sites: names are stable with `aria-pressed` (e.g. RoomScreens "Pin …", WorkflowsPage "Context panel" / "Maximize run detail", Studio3D "Turntable/Play/Source", SiteCanvas "Hidden on mobile") or `aria-pressed` was dropped (RoomMedia, Splits, ReaderView, FileList, S3Preview). |
| ux-design-1-04 | Fixed | `RoomPage.svelte:153-154`: separate "Connecting to the room…" status (no action) and "Connection interrupted" error with Retry; in-room reconnecting strip at 157-158. |
| ux-design-1-05 | Fixed | `RoomRecap.svelte:70`: "Retry speech capture" only when `captureStatus.speechIssue`, paired with the reason. |
| ux-design-1-06 | Fixed | `DiffViewer.svelte:1629-1632`: `max-height: var(--nav-max, 480px)` measured from the scroll area, not `100vh`. |
| ux-design-1-07 | Fixed | `RoomRecapsPage.svelte:18`: `crumbs={[{label: 'Rooms', …}]}`; the header button is gone. |

All 7 previous findings are fixed; none rejected.

## Areas covered this iteration

- Re-verification of all iteration-1 findings (table above), plus the UX of code the iteration-1 fixes
  added (App capabilities banner, ConflictResolverView load error, AttachIssue/InfoPanel).
- Database Explorer deep states: ImportDialog, ExportDialog validation, StructureView
  loading/error/empty, MultiRunDialog guarded runs, the store's typed write gate
  (`database.svelte.ts` `confirmGuardedWrite`), VerticalView field add/rename.
- Git: CreatePr (where/who line, draft-with-agent replace confirm, AgentByline), PrDetail outward
  confirms (request changes, update PR, approve/decline) — all conform.
- Scheduled tasks (list states, Run-and-deliver confirm, convert-to-workflow choice), Goal loops
  (list → detail, restore selection, delete/stop confirms), Swarm GoalsPanel states.
- Workflows TriggersPanel (load/error, delete confirm, spec copy) and the header primary anatomy.
- Product: DesignArena states/selection fallback and Blender render feedback, RefineChat
  attribution, PublishDialog and TestCasesTab Confluence publish (where/what/who present).
- Settings: Providers, AccessGroups, Channels (load/error, validation, open-to-everyone confirm,
  test-message confirm).
- Kubernetes PodHttpPanel prod/read-only guard; AWS Athena/Logs/S3/EC2 states and confirmProd use;
  Logs list/detail initial selection.
- Personal agents (AgentPage, PersonalAgentsPage, AgentEditSheet delivery confirm) traced end to
  end into `otto-assistant/src/personal_agents_engine.rs` delivery.
- Home boxes (Sessions, Usage, Insights, K8s, Mission Control, DB dashboards) and HomeToday;
  Assistant page states; PluginFrame states.
- Cross-cutting greps: validation toasts, raw-error toast titles (none), "OK/Submit/Yes" buttons
  (none), `100vh` (only own-window pages and documented comments), bare "Loading…" regions
  (none), `confirmer.ask` calls without `confirmLabel` (all are real deletes except one nit),
  local relative-time formatters, "pick one" empty detail panes.

## Areas NOT covered yet (next iteration)

- Workflows editor inspector panels in depth (node forms in the 3.5k-line WorkflowsPage), RunAgents.
- Product DiscoveryChat, AnalysisTab/RewriteTab/QuestionsTab flows, MockupsTab.
- Kubernetes ClusterWorkspace/ResourceDrawer/ExecView/monitor states (only PodHttpPanel checked).
- AWS SqsView, RdsView, EksView (EksView uses plain `confirmer.ask` — check prod wording).
- Vault (NoteView, DocsAgentsView), Canvas, Design Hall studios (Site, 3D), Snip.
- Mission Control, Usage, Insights pages (only their Home boxes were checked), Proof detail.
- Settings: Users, PersonalAccessTokens, SecretsStore, BackupRestore, Onboarding copy.
- Agents SessionView/Composer/ConversationView (core path, heavily changed), Navigator, Palette.
- Every module's phone/tablet layout — static review only, no screenshots taken.

---

## Findings

### ux-design-2-01 — major — Personal agent "Run now" / "Run once" posts to Slack/Telegram/email/webhook without confirming where

- **Files:**
  - `ui/src/modules/personal-agents/AgentPage.svelte:262-272` (`runNow`), call sites `:385` ("Run now", the page primary), `:389` ("Run once"), `:588` (per-schedule "Run now")
  - `ui/src/modules/personal-agents/PersonalAgentsPage.svelte:103-114` (`runNow`), call sites `:129` (row menu), `:261`, `:265`
- **Evidence:**
  - Both `runNow` functions call `personalAgents.runNow(...)` directly; there is no `confirmOutward` anywhere in `modules/personal-agents/` (grep finds none).
  - A manual run is a directed run (`personal_agents_engine.rs:673`, `RunPlan::directed()`), and on completion the engine delivers it: `personal_agents_engine.rs:983-991` calls `ctx.deliver_destination(... &agent.delivery, &out.summary, &out.report)` unless the report is unchanged or the run is proactive.
  - The agent's `delivery` is Slack / Telegram / email / webhook (`types.ts:8575-8577`; AgentPage `deliveryLabel()` at `:344-353`).
  - The same action in Scheduled Tasks confirms first: `ScheduledTasksPage.svelte:716-729` runs `confirmOutward({ verb: 'Run and deliver', where: savedDestWhere(t), what, who: destWho(...) })` when a destination is set.
  - AGENTS.md design rule: "Anything outward-facing (PR, Jira/Confluence, Slack/Telegram, publish, prod) confirms where it goes, what is sent and who sees it." Two modules that run the same delivery engine now disagree, and the personal-agent one posts silently.
- **Fix:** Extract `savedDestWhere` / `destWho` from ScheduledTasksPage into a shared helper (e.g. `lib/deliveryCopy.ts`) and, in both personal-agent `runNow` functions, when `agent.delivery.type` is not `none`, `await confirmOutward({ verb: 'Run and deliver', title: 'Run and deliver now?', where, what: \`The report of “${agent.name}”, produced by a run that starts now.\`, who })` before calling the store. Keep "Run once" wording but route it through the same confirm.

### ux-design-2-02 — minor — The database write gate calls every guarded connection "production" and reports a mistyped name as a plain cancel

- **Files:**
  - `ui/src/lib/stores/database.svelte.ts:3292-3317` (`confirmGuardedWrite`; title at `:3311`), callers `:3163-3170`, `:3178-3183`
  - `ui/src/modules/database/ImportDialog.svelte:175-183` (copy of the same gate), `:113-122`
- **Evidence:**
  - `label = conn.environment === 'prod' ? 'PRODUCTION' : 'read-only'` — the body distinguishes the two, but the dialog title is always `'⚠ Confirm production write'` (store `:3311`, ImportDialog `:181`). A write on a read-only *dev* connection is titled "Confirm production write". MultiRunDialog gets it right ("production or read-only", `MultiRunDialog.svelte:618`).
  - The title also uses a `⚠` glyph as an icon (guidelines: no glyph characters as icons; the Modal's danger styling already carries the tone).
  - A typed name that doesn't match returns `false` (store `:3317`, ImportDialog `:183`); the callers then toast only "Write canceled" (`:3166`, `:3180`) or "Import canceled" (ImportDialog `:121`). The person who typed a near-miss is not told why nothing ran. The shared prod gate explains it: `confirmProd.ts:66-68` → "Nothing was changed — What you typed didn’t match, so the action was canceled."
- **Fix:** Title by environment (`'Confirm production write'` vs `'Confirm write on a read-only connection'`), drop the `⚠` glyph, and on a non-null mismatch toast the `confirmProd` wording. Better: have `ImportDialog` reuse the store's gate (make it public) instead of a second copy, so both stay aligned.

### ux-design-2-03 — minor — Form field errors are shown as toasts instead of inline under the field

- **Files:**
  - `ui/src/modules/database/ImportDialog.svelte:99-106` ("Missing field", "Invalid batch size")
  - `ui/src/modules/database/ExportDialog.svelte:141-143` ("Invalid row limit")
  - `ui/src/modules/browser/CredentialsPanel.svelte:90-92` ("Missing fields")
  - `ui/src/modules/settings/Channels.svelte:260-265` ("Add an allowed user before enabling …" while the edit Modal stays open)
  - `ui/src/modules/product/TestCasesTab.svelte:433-436` ("Note required — Please add a review note…")
  - `ui/src/modules/mcp/PoliciesTab.svelte:164-166` ("Pick a server and enter a tool name")
  - `ui/src/modules/api/RequestBuilder.svelte:392` ("Add a token URL first")
  - `ui/src/modules/database/VerticalView.svelte:257-259`, `:276-278` ("Invalid field name" after a `promptText`)
- **Evidence:** Each handler validates a field, raises `toasts.error/warn(...)` and `return`s with the form still open. `components.md:100-102`: "Validation is inline. Put the message under the field in `--danger` … Don't use a toast for a field error." and the table at `:403` ("A form field is invalid | Inline, under the field"). The toast fades in a few seconds while the offending field is unmarked; screen readers get no `aria-invalid`/`aria-describedby` on the field.
- **Fix:** Keep a per-form `fieldErrors` map, render `<p class="field-error" id=…>` under the field with `aria-invalid` + `aria-describedby`, and disable the primary while required fields are empty (as `FindingActions` already does at `:384`). For `promptText`-based edits (VerticalView), add an optional `validate(value) => string | null` to `confirmer.promptText` so ConfirmDialog shows the error inline and keeps the dialog open.

### ux-design-2-04 — minor — Three local cadence formatters show the same schedule three different ways ("every 1440 min")

- **Files:**
  - `ui/src/modules/personal-agents/cadence.ts:64-75` (`cadenceLabel`, shown as the schedule's primary `<strong>` at `AgentPage.svelte:579` and in aria-labels `:588-591`)
  - `ui/src/modules/workflows/TriggersPanel.svelte:192-204` (`describeSpec`, shown at `:363` and inside the delete confirm `:177`)
  - `ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte:885-907` (`everyLabel` / `cadenceLabel`, the only humane one)
- **Evidence:**
  - Personal agents and workflow triggers render `\`every ${every_min} min\`` → a daily interval reads "every 1440 min"; weekly reads "weekly Mon at 09:00 UTC" (lower-case lead, abbreviated day).
  - Scheduled Tasks renders the same spec as "Every day"/"Every 2 hours" and "Mondays at 09:00 · UTC".
  - `content.md` §6: "Cadences read as people say them: 'Weekly (Mondays 09:00)', 'Every 15 minutes', not 'every 10080 min'"; §7: no new local formatters. Three modules that share one schedule shape (the personal-agent type comment says "Existing cadence format") disagree.
- **Fix:** Move Scheduled Tasks' `everyLabel` + `cadenceLabel` into `lib/` (e.g. `lib/cadence.ts`, taking `(schedule, timezone)`), and use it in AgentPage and TriggersPanel; delete the two local versions.

### ux-design-2-05 — minor — Kafka message inspector always lands on "Pick a message" after a Peek

- **File:** `ui/src/modules/brokers/TopicDetail.svelte:441` (`selected = null` on every consume), `:260`, `:856`
- **Evidence:** Each Peek/consume clears `selected`; with messages loaded the detail pane renders `<EmptyState title="Pick a message" body="N messages loaded. Select one…">`. `layout.md:303`: "Never open on an empty 'pick one' pane when there are items. Restore the last selection or fall back to the first item." Brokers is a listed list/detail module (`layout.md:276`), and its sibling `GroupsTab` auto-selects.
- **Fix:** After a non-tail consume, on desktop/tablet select the first (or previously selected offset if still present) message via `selectMessage(result.messages[0])`; keep the selection unchanged on live-tail appends. Keep the EmptyState only for "no messages".

### ux-design-2-06 — minor — Local relative-time formatters bypass the shared `rel()` (some never tick)

- **Files:**
  - `ui/src/modules/home/HomeToday.svelte:57-63` (`when()`: "in 2h"/"3m ago", re-implements what `rel()` already does for future times)
  - `ui/src/modules/home/boxes/SessionsBox.svelte:17,64` and `HomeToday.svelte:13` (import Mission Control's private `relTime`)
  - `ui/src/shell/NotificationBell.svelte:286-296` (`relative()`), `ui/src/modules/git/FocusView.svelte:269-276` (`ago()`, `Date.now()` — not reactive), `ui/src/modules/database/DatabasePage.svelte:939-946` (`fmtAgo`), `ui/src/modules/canvas/ConversationPanel.svelte:101`, `ui/src/modules/product/LinkedCanvases.svelte:131`, `ui/src/modules/mission-control/WorkItemDetail.svelte:317`
- **Evidence:** Each is a hand-written seconds/minutes/hours ladder with its own rounding (`Math.round` in NotificationBell/FocusView vs `Math.floor` in DatabasePage/relTime) and suffix rules ("3m" vs "3m ago" vs "now" under 45 s). `content.md` §7: "Use the shared helpers. Don't write another local `timeAgo`" — `rel(ts)` from `lib/stores/now.svelte.ts:64` is reactive and handles past and future. The Home dashboard shows "3m" in the Agents box and "3m ago" in the Today cards for the same kind of timestamp.
- **Fix:** Replace these with `rel()` (or `<RelTime iso=… />`); if a compact "3m" form is wanted on dense rows, add a `compact` option to `rel()` once and use it everywhere, then delete the local functions.

### ux-design-2-07 — minor — The permissions-failure banner floats over the unified toolbar with no way to dismiss it

- **File:** `ui/src/App.svelte:167-172` (markup), `:213-230` (`.caps-banner { position: fixed; inset-block-start: 8px; inset-inline: 16px; max-width: 560px; z-index: var(--z-toast) … }`)
- **Evidence:**
  - Added by the iteration-1 fix for error-handling-1-05. It is pinned 8 px from the top of the window, which is where `PageHeader`'s single toolbar row (title, leading back button, primary action) sits, and at the toast layer, so it sits on top of those controls.
  - On phone width it spans the full width minus 16 px gutters, covering the leading back button and title. There is no close control, and it stays while `auth.capabilitiesError` is true — the store retries on a 2 s → 60 s backoff indefinitely (`auth.svelte.ts:90`, `:178-181`), so a daemon that keeps failing `/auth/capabilities` leaves the toolbar covered.
  - App-level connection state already has a non-overlapping home: the shell `StatusBar` (`shell/App.svelte:1384-1386`).
- **Fix:** Render the capabilities error inside the shell's StatusBar (or as an in-flow strip under the toolbar inside `PageBody`), not as a fixed overlay; if it must float, place it bottom-centred like Toasts and add a dismiss that hides it until the next failed retry.

### ux-design-2-08 — nit — Copy slips: brand nouns as verbs, "unknown error", "(s)" plurals and "1 rows"

- **Files and evidence:**
  - `ui/src/modules/product/design/DesignArena.svelte:778` "`${n} file(s) attached`", `:781` "Couldn’t blender render" with fallback `'unknown error'` (the catch at `:784` already says "Couldn’t render with Blender").
  - `ui/src/lib/stores/apiClient.svelte.ts:1150` "Couldn’t postman sync".
  - `ui/src/modules/database/ResultsGrid.svelte:211` "`${n} row(s) affected`"; `ui/src/modules/database/StructureView.svelte:673` "`${row_count} rows`" (reads "1 rows").
  - `ui/src/modules/database/DatabasePage.svelte:1143`, `ui/src/modules/settings/BrowserSettings.svelte:216` `'unknown error'`; `ui/src/modules/product/AnalysisTab.svelte:855` `'Unknown error'`.
  - `ui/src/modules/agents/NewSession.svelte:399` "Could not create session" next to `:404` "Couldn’t create session".
  - `content.md` §1/§7: say what happened and why; use `plural()`/`formatCount`.
- **Fix:** "Couldn’t render with Blender" / "Couldn’t sync with Postman"; `plural(n, 'file')`, `plural(n, 'row')`; replace "unknown error" fallbacks with a cause-specific sentence ("The job ended without a reason. Check the Blender log."); use "Couldn’t" consistently.

### ux-design-2-09 — nit — Delete/Remove vocabulary and raw enum in trigger and field confirms

- **Files and evidence:**
  - `ui/src/modules/workflows/TriggersPanel.svelte:176-189`: the confirm is "Delete trigger" / "Delete this `${t.kind}` trigger", but the toasts say "Trigger removed" / "Couldn’t remove the trigger"; `t.kind` is the raw enum ("Delete this chat trigger") although `KIND_LABEL` (`:132`) maps it to "Chat binding".
  - `ui/src/modules/database/VerticalView.svelte:284`: body "Remove “path” from this document?" with title "Delete field" and the default "Delete" button.
  - `content.md` §2/§6: Delete = destroys, Remove = detaches; one word per concept; map enum values to words.
- **Fix:** TriggersPanel: "Trigger deleted" / "Couldn’t delete the trigger", and `${(KIND_LABEL[t.kind] ?? t.kind).toLowerCase()}` in the message. VerticalView: "Delete “path” from this document?" (or title/button "Remove field" if the field is only unset).
