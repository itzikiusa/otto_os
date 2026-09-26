# Round 1: data workbenches

Scope: Database Explorer, Connections/SFTP/network profiles, API client, Kafka brokers. Worktree: /Users/itziklavon/otto-ux-audit-20260925. Source review against AGENTS.md and design guidelines; browser evidence is explicitly distinguished. No edits to original checkout, no commits.

## Pre-fix scores

| Dimension | Score / 10 |
|---|---:|
| Layout | 8.4 |
| Interaction | 7.9 |
| Accessibility | 8.0 |
| States | 7.9 |
| Responsive | 7.3 |
| **Mean** | **7.90** |

These are evidence-bounded audit estimates, not a claim that all main flows were verified. 9.5 is not justified: real query execution, Kafka consumption, live SSH transfer and all five theme combinations were not exercised. Sources generally show tokenized styling, explicit guarded action confirmations, shared Modal/LoadState and useful toolbar states. Smaller panels still have material gaps.

## Findings

1. **P2, browser reproduced: SFTP name buttons ignore click and Space.** `ui/src/modules/connections/SftpBrowser.svelte:292` originally bound only double-click and Enter to a native button. Opening Browse files and clicking a directory, or focusing it and pressing Space, leaves `/home` unchanged; directories have no separate Open action. Failed browser assertions show crumbs remain `// home`. A single click handler uses native mouse, touch, Enter and Space activation. Fixed using native click activation; durable regressions in `ui/e2e/desktop-ux-data.spec.ts`.

2. **P2, browser reproduced: SFTP phone layout has no narrow adaptation.** `SftpBrowser.svelte:392` toolbar cannot wrap and its filter is 180px; `:452` listing reserves 324px for metadata before name/actions. At 375px the modal's content cannot expose directory names and actions together. Remedy: wrap toolbar/filter and stack each narrow row's metadata below name/actions. Do not drop permissions or modified date entirely.

3. **P2, browser reproduced: API response controls share an unbounded row.** `ui/src/modules/api/ResponseViewer.svelte` `.rtabs` styles combine response tabs and Pretty/Raw/Tree in a single flex row without wrapping or scrolling. A JSON response containing headers, cookies and trace needs more width than a phone. Remedy: wrap the row and reserve body-mode controls on a new row when needed; preserve arrow-key response navigation.

4. **P2, source confirmed, residual: saved query/history loads look empty after failure.** `ui/src/lib/stores/database.svelte.ts:3368` and `:3493` only toast failed loads. `ui/src/modules/database/DatabasePage.svelte:1519` and `:1573` then render “No saved queries” / “No query history yet” without Retry. Repro: reject `/saved-queries` or selected connection `/history` then open relevant sidebar. Remedy: explicit loading/error state scoped to workspace/connection, inline LoadState + Retry, preserve existing lists as stale data. Store ownership authorization received but deferred for next pass.

5. **P2, source confirmed, residual: Kafka replay accepts an empty timestamp and silently substitutes now.** `ui/src/modules/brokers/ReplayPanel.svelte:37` only validates topic names and range order; `:54` converts blank/invalid date to `Date.now()`. Select “Since timestamp”, leave From blank, enter distinct topics, click Replay: confirmation succeeds and selection means now rather than a user-selected time. Count/partition numeric constraints are also not enforced because this is not a submitted form. Remedy: validate selector-specific finite integer/range and timestamp input inline; disable Replay until valid and never substitute current time silently.

6. **P2, source confirmed, residual: slower Kafka group detail can overwrite the selected group.** `ui/src/modules/brokers/GroupsTab.svelte:117` changes selected immediately, then accepts every response at `:125`. Click group A then B with A delayed; A's later response populates detail while B stays selected, and offset reset still targets selected B. Remedy: generation/selection guard on success, catch and finally; invalidate on cluster changes. Browser deferred-response regression needed.

7. **P3, source confirmed, residual: database sidebar and query tabs omit composite keyboard navigation.** `DatabasePage.svelte:975` tablist uses click handlers only; `QueryEditor.svelte:740` gives every query tab tabindex 0 and handles Enter/Space but not arrows/Home/End. Main content tabs already implement a keyboard helper, so behavior differs within one workbench. Remedy: reuse roving tabindex/arrow-key pattern while retaining close/rename access.

8. **P3, source confirmed, residual: network profile SSH fetch has no truthful loading/retry state.** `ui/src/modules/connections/NetworkProfiles.svelte:16` catches failure into raw error, while `:59` immediately says “Create an SSH connection…” whenever connections is empty (including loading/failure). Remedy: track pending/failed load separately, suppress empty guidance until successful load, inline Retry.

## Browser verification notes

Isolated daemon port 7813 and Vite 5313, OTTO_E2E_SLOT=uxdata, current target/debug/ottod, orphan sweep disabled. SFTP and API upstream transports mocked; no real host/request is contacted. Test-fixture initial failures due to duplicate global connection names and conditional response tabs were corrected and are not product findings.

Resizing an already-open SFTP modal from desktop to phone remounts the shell and closes it; test now opens directly at phone dimensions. This observed shared-shell behavior needs separate assessment (no shared-shell edits).

## Implemented changes and verification

- SFTP: native click activation (also restores Space/Enter); responsive wrapping and stacked rows retaining metadata/actions; LTR path/code containers.
- API: response navigation wraps within available width.
- Browser red evidence: filter right edge 457px; API body-mode right edge 450px, both in a 375px viewport. Red artifacts: `/tmp/otto-ux-data-phone-red`.
- `npx tsc --noEmit -p tsconfig.e2e.json`: passed. `git diff --check`: passed.
- Full `npm run check` and multi-theme screenshot sweep belong to aggregate verification by the lead. This pass did not verify dark/Warm, RTL rendered screenshots, production SSH/Kafka/DB operations, or all main-flow states.
- Post-fix numerical re-scoring deferred until remaining defects and broader verification are complete; original 7.90 score retained for this audit round.
- Browser suite: **4 passed (52.5s)** in `/tmp/otto-ux-data-green` after functional fixes. After local class rename for guard compliance, final two phone checks remain running in exec session **89104**, artifacts `/tmp/otto-ux-data-final`; lead can poll session. `node ui/scripts/ui-guards.mjs`: passed (802 files, no regressions). No baseline changes.

Final follow-up phone run completed; /tmp/otto-ux-data-final/.last-run.json reports passed, failedTests=[].
