# D08 — Usage, Settings, Share/Mobile Access, Help & Platform UX

- **Reviewer Subagent ID:** `ffb4d54a-0f63-4a1a-8692-437ee4a860cf` (`D08 Usage Settings & Platform UX Reviewer`)
- **Lane Lead:** `Ronaldo` (`agy`)
- **Scope:** `ui/src/modules/{usage,settings,share,help,snip}/*`
- **Iteration 1 Baseline Score:** **8.1 / 10**
- **Iteration 2 Post-Fix Score:** **9.8 / 10**

---

## 1. Confirmed Findings & Verification

### D08-01 (High — Fixed) — Cleartext secrets in `McpServers.svelte` and `Channels.svelte`, plus empty allow-list warning
- **Location:** `ui/src/modules/settings/McpServers.svelte:131, 352, 514-523, 581-589`; `ui/src/modules/settings/Channels.svelte:133-148, 233, 681, 693, 821-825`
- **Issue:** In `McpServers.svelte`, the `#mcp-secret-env` `<textarea>` rendered `KEY=VALUE` secrets in cleartext with no masking; in `Channels.svelte`, the webhook key `<input id="ch-bot">` used `type="text"` unconditionally, and enabling a Slack/Telegram integration with an empty allow-list (`!fOpenToAll && fAllowedUsers.trim() === ''`) gave no inline warning that the daemon fails closed and drops all messages.
- **Fix Applied:** Added `showSecrets` state and `.masked-secret { -webkit-text-security: disc; }` to `McpServers.svelte:514-523, 581-589`; masked the webhook key by default (`type={editChannel === 'webhook' && showWebhookKey ? 'text' : 'password'}`) with a `Show`/`Hide` toggle in `Channels.svelte:681, 694`; and added an inline `role="alert"` warning (`needsAllowedUser`) in `Channels.svelte:133-135, 821-825`.

### D08-02 (High — Fixed) — `OttoUsage.svelte` unstyled inputs, off-scale `Modal width={900}`, `60vh` table cap, and raw error strings
- **Location:** `ui/src/modules/usage/OttoUsage.svelte:11, 79, 99, 106, 112, 118, 124, 136-146, 166, 201, 206, 213, 238`
- **Issue:** `OttoUsage.svelte` formatted errors via `e?.message ?? String(e)` instead of `loadErrorText(e)`, used bare `<input type="number">` and `<select>` controls without `.input`, passed `width={900}` to `<Modal>` (outside the standard modal width scale), and capped `.events` with `max-block-size: 60vh`.
- **Fix Applied:** Switched all 6 error catch blocks to `loadErrorText(e)`, added `class="input"` to all 11 telemetry numeric inputs and the window `<select>`, resized the raw events modal to `width={680}`, and changed `.events` to `max-block-size: 420px`.

### D08-03 (High — Fixed) — `UsagePage.svelte` background poll clobbering mid-edit storage settings, missing range validation, and hand-rolled error banner
- **Location:** `ui/src/modules/usage/UsagePage.svelte:70-83, 204, 489, 643-650`
- **Issue:** Every 15 seconds, `loadAll()` overwrote `retention`, `interval`, and `chPath` even when the user was actively editing the Storage & retention form; out-of-range values (`retention < 1 || retention > 3650`, `interval < 5 || interval > 3600`) showed no inline validation message; and line 489 used a hand-rolled `.error-banner` instead of `<LoadState>`.
- **Fix Applied:** Added `storageDirty` and `storageError` derived state (`UsagePage.svelte:70-83`), guarded `loadAll()` with `if (!storageDirty)` (`204`), rendered an inline `storageError` alert and disabled `Save` when invalid (`643-650`), and replaced the hand-rolled error block at line 489 with `<LoadState what="usage metrics" error={err} empty={false} onretry={loadAll} />`.

### D08-04 (Medium — Fixed) — Standardized `<LoadState variant="compact">` and token z-index across Settings
- **Location:** `ui/src/modules/settings/McpServers.svelte:149-160`, `ui/src/modules/settings/BrowserSettings.svelte:22, 147-149`, `ui/src/modules/settings/ConnectionsExport.svelte:4, 87-91`, `ui/src/modules/settings/InsightsSettings.svelte:197`, `ui/src/modules/settings/TrustSafety.svelte:536`
- **Issue:** Settings subpages used bespoke error retry markup instead of `<LoadState variant="compact">`, and `TrustSafety.svelte:536` used raw `z-index: 1` instead of `var(--z-sticky)`.
- **Fix Applied:** Migrated `McpServers.svelte`, `BrowserSettings.svelte`, and `ConnectionsExport.svelte` to `<LoadState variant="compact">`, removed unused `.err-card` CSS in `BrowserSettings.svelte`, and replaced `z-index: 1` with `z-index: var(--z-sticky)` in `TrustSafety.svelte:536`.
