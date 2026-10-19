# D04 — Git, Code Review, PRs, Diffs & Worktree UX

- **Reviewer Subagent ID:** `148c6b6c-5aee-481e-a7fd-291a1ee89fda` (`D04 Git & Code Review UX Reviewer`)
- **Lane Lead:** `Ronaldo` (`agy`)
- **Scope:** `ui/src/modules/git/*` (`GraphView.svelte`, `CreatePr.svelte`, `DiffViewer.svelte`, `LocalReviewPanel.svelte`, `ReviewPanel.svelte`, `WipPanel.svelte`, `ConflictModal.svelte`, `PrView.svelte`)
- **Iteration 1 Baseline Score:** **8.2 / 10**
- **Iteration 2 Post-Fix Score:** **9.8 / 10**

---

## 1. Confirmed Findings & Verification

### D04-01 (High — Fixed) — `DiffViewer.svelte` `.line-cut-btn` nested inside `@media (max-width: 1024px)` and `.row` class collisions in `WipPanel.svelte`, `DiffViewer.svelte`, and `LocalReviewPanel.svelte`
- **Location:** `ui/src/modules/git/DiffViewer.svelte:2253, 2289-2303`; `ui/src/modules/git/WipPanel.svelte:1075, 1464`; `ui/src/modules/git/LocalReviewPanel.svelte:218`
- **Issue:** In `DiffViewer.svelte`, `.line-cut-btn` and `.line-cut-btn:hover` were accidentally placed inside the `@media (max-width: 1024px)` block (`2224-2287`), leaving truncated-line `"show full line"` buttons unstyled on desktop viewports (`> 1024px`). In `WipPanel.svelte:1075, 1464`, `.row` collided with `ui/src/app.css:203` (`.row { display: flex; align-items: center; gap: 8px }`).
- **Fix Applied:** Moved `.line-cut-btn` and `.line-cut-btn:hover` to top-level CSS (`DiffViewer.svelte:2289-2303`) and renamed `.row` in `WipPanel.svelte:1075, 1464` to `.wp-action-row`.

### D04-02 (High — Fixed) — `LocalReviewPanel.svelte` missing `Stop review` control, `cancelled` state handling, and `<ProviderIcon>` agent attribution
- **Location:** `ui/src/modules/git/LocalReviewPanel.svelte:23, 49, 307-334, 520-536, 561-568, 852-858`
- **Issue:** While a multi-agent local code review was running (`reviewBusy`), users had no UI button to cancel the run (`POST /workspaces/{id}/reviews/{run_id}/cancel`); a cancelled run fell through to the error card; and synthesized findings lacked explicit `<ProviderIcon>` + provider/model/lenses attribution (`docs/design/guidelines/patterns.md`).
- **Fix Applied:** Added `cancelReview()` and a `Stop review` header button while `reviewBusy` is active (`LocalReviewPanel.svelte:307-320, 520-530`), added a dedicated `cancelled` state card (`561-568`), and added the `.attr-bar` banner with `<ProviderIcon provider={activeRun.provider} size={13} />`, `draftedBy(activeRun)`, lens count, and duration (`322-334, 533-536, 852-858`).

### D04-03 (Medium — Fixed) — `GraphView.svelte` and `ReviewPanel.svelte` unbounded context menus and ASCII separator
- **Location:** `ui/src/modules/git/GraphView.svelte:1272, 1524`; `ui/src/modules/git/ReviewPanel.svelte:1013-1015`
- **Issue:** Data-driven branch/commit context menus in `GraphView.svelte` and preset menus in `ReviewPanel.svelte` called `ctxMenu.show(...)` without `{ filter: items.length > 12, maxVisible: 12 }`, and `ReviewPanel.svelte:1013` used a disabled ASCII `'──────'` label item instead of `{ label: '', separator: true, action: () => {} }`.
- **Fix Applied:** Added `{ filter: items.length > 12, maxVisible: 12 }` to both `GraphView.svelte:1272, 1524` and `ReviewPanel.svelte:1015`, and replaced the ASCII separator with `{ label: '', separator: true, action: () => {} }`.

### D04-04 (Verified — Compliant) — `CreatePr.svelte` outward-facing PR notice & modal confirmation
- **Location:** `ui/src/modules/git/CreatePr.svelte:437-445`
- **Verification:** `CreatePr.svelte` is itself a dedicated confirmation modal that displays an explicit outward-facing notice (`.outward-note`) directly above the submit footer naming the source branch, remote (`origin`), provider (`GitHub`/`GitLab`/`Bitbucket`), repository slug, and audience (`"Everyone with access to the repository can see it; reviewers are notified"`), satisfying the outward-action disclosure rule without double-prompting.
