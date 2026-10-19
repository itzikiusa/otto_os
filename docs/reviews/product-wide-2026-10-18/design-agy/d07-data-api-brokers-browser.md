# D07 — Connections, Database Explorer, Message Brokers, API Client, Browser & MCP UX

- **Reviewer Subagent ID:** `3e03d181-dc7e-49b9-bef3-5c86dd3d036a` (`D07 Data API Brokers & Browser UX Reviewer`)
- **Lane Lead:** `Ronaldo` (`agy`)
- **Scope:** `ui/src/modules/{connections,database,brokers,mcp}/*` (owned by `Ronaldo/agy`), plus cross-review of `api/`, `browser/`
- **Iteration 1 Baseline Score:** **7.6 / 10**
- **Iteration 2 Post-Fix Score:** **9.8 / 10**

---

## 1. Confirmed Findings & Verification

### D07-01 (High — Fixed) — Destructive `confirmer.ask()` calls missing `danger: true` across Database Explorer and Message Brokers
- **Location:** `ui/src/modules/database/DatabasePage.svelte:1639`, `ui/src/modules/database/Dashboards.svelte:45`, `ui/src/modules/database/WidgetCard.svelte:120`, `ui/src/modules/brokers/BrokersPage.svelte:263`, `ui/src/modules/brokers/LagAlertsPanel.svelte:80`
- **Issue:** Five destructive deletion confirmations (deleting a saved DB query, deleting a DB dashboard, removing a dashboard widget, deleting a Kafka cluster, and deleting a consumer-lag alert rule) omitted `danger: true`, rendering primary accent buttons instead of `--danger` destructive buttons.
- **Fix Applied:** Added `danger: true` (and `confirmLabel: 'Delete'` where missing) across all 5 `confirmer.ask(...)` calls.

### D07-02 (High — Fixed) — Global `.row` class collisions in `QueryBuilder.svelte`, `ClusterForm.svelte`, and `ConnectionImportDialog.svelte`
- **Location:** `ui/src/modules/database/QueryBuilder.svelte:1096, 1193, 1359, 1922, 1929`, `ui/src/modules/brokers/ClusterForm.svelte:132, 154, 181, 208, 232, 275, 279, 345, 352`, `ui/src/modules/connections/ConnectionImportDialog.svelte:319, 566, 572, 575, 594`
- **Issue:** Scoped `.row` classes in `QueryBuilder.svelte`, `ClusterForm.svelte`, and `ConnectionImportDialog.svelte` collided with `ui/src/app.css:203` (`.row { display: flex; align-items: center; gap: 8px }`), causing unintended flex alignment and spacing inheritance on table rows and multi-column form grids.
- **Fix Applied:** Renamed `.row` to `.qb-row` in `QueryBuilder.svelte`, `.cf-row` in `ClusterForm.svelte`, and `.import-row` in `ConnectionImportDialog.svelte`.

### D07-03 (Medium — Verified) — Read-only/production safety indicators and schema/autocomplete popover clamping
- **Location:** `ui/src/modules/database/DatabasePage.svelte:1081-1230, 4180`, `ui/src/modules/connections/ConnectionsPage.svelte`
- **Verification:** Verified prod/read-only badge rendering, destructive change-request confirmation in `DatabaseChanges.svelte`, and viewport clamping (`max-height` + `overflow-y: auto`) on schema/autocomplete popovers.
