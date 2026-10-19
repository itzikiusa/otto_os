# D05 — Product Story Analysis, Mission Control, Swarm, Workflows, Loops & Scheduled Tasks UX

- **Reviewer Subagent ID:** `77042351-e9d7-4094-a75f-9008024ad480` (`D05 Product Swarm & Workflows UX Reviewer`)
- **Lane Lead:** `Ronaldo` (`agy`) — Cross-review handoff to `Pele` (`grok`) for workflow/automation/swarm/product module files
- **Scope:** `ui/src/modules/{product,mission-control,swarm,workflows,loops,scheduled-tasks}/*`
- **Iteration 1 Baseline Score:** **8.1 / 10**
- **Cross-Review Status:** Verified findings handed off to `Pele` (`grok`) who owns direct edits in `workflows/`, `scheduled-tasks/`, `loops/`, `swarm/`, `product/`.

---

## 1. Confirmed Findings & Cross-Lane Handoff

### D05-01 (High) — Outward Jira/Confluence mutations in `StoryDetail.svelte` without `confirmer.ask()`
- **Location:** `ui/src/modules/product/StoryDetail.svelte:403-411, 567-582, 1134-1143, 1285-1305`
- **Repro:** Trigger `transitionStory`, `postAnswersToJira`, `saveInlineEdit` (writes directly to Jira), or `createConfluenceDesign` (publishes a Confluence page and posts a Jira comment). None shows a confirmation dialog naming destination and audience.
- **Fix Direction (Coordinated with Pele):** Gate each outward write behind `confirmer.ask({ title, message, confirmLabel })` naming the Jira issue key / Confluence space.

### D05-02 (High) — Outward channel test sends in `ScheduledTasksPage.svelte` and `SwarmPage.svelte` without confirmation
- **Location:** `ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte:451-460`, `ui/src/modules/swarm/SwarmPage.svelte:531-539`
- **Repro:** Clicking "Send test" in Scheduled Tasks or "Test fire" in Swarm Triggers dispatches live messages/runs without confirmation.
- **Fix Direction (Coordinated with Pele):** Confirm destination channel before sending outward test payloads.

### D05-03 (High) — Global `.row`, `.card`, and `.chip` class collisions
- **Location:**
  - `ui/src/modules/loops/LoopsPage.svelte:296` (`<div class="card">`)
  - `ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte:265` (`<div class="row">`)
  - `ui/src/modules/swarm/SwarmScheduleSection.svelte:105`, `SwarmStageGatesSection.svelte:79`, `SwarmTriggersSection.svelte:98`
- **Fix Direction (Coordinated with Pele):** Prefix module classes (`.loop-card`, `.st-row`, `.sw-row`).

### D05-04 (Medium) — List/detail auto-selection when items exist in `LoopsPage.svelte` and `ScheduledTasksPage.svelte`
- **Location:** `ui/src/modules/loops/LoopsPage.svelte:90-98`, `ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte:163-171`
- **Repro:** Opening Goal Loops or Scheduled Tasks when items exist leaves `selectedId = null` and displays `"Select a goal loop on the left"` / `"Select a task on the left"`, violating `docs/design/guidelines/layout.md`.
- **Fix Direction (Coordinated with Pele):** Auto-select `items[0].id` after initial load when `selectedId === null`.
