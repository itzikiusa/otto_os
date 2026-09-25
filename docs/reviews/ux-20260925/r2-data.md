# Round 2 — Data workbenches (reviewer 3/10)

Worktree only: `/Users/itziklavon/otto-ux-audit-20260925`. Owned Database/API/Brokers/Connections and database-specific store (ownership notified to parent). No commits, full npm check, real connections, user data, broker writes or file transfers. Read prior R1 report, AGENTS.md and design guidelines. Used systematic-debugging/TDD instructions. All five audit rounds remain mandatory.

## Independent recheck and repairs

R1 SFTP click/Space, phone name/action layout and API phone response tabs/body modes were independently rerun unchanged: **4 passed, 54.5s**, `/tmp/otto-ux-r2-data-old.log`, output `/tmp/otto-ux-r2-data-old` (original debug daemon). They also pass in the final matching-release suite.

1. **P2 fixed — database saved/history failure looked empty.** `ui/src/lib/stores/database.svelte.ts:3379` / `:3514`, `ui/src/modules/database/DatabasePage.svelte:1535` / `:1592`. A 503 caused no Retry and rendered empty-list guidance. Red evidence: `/tmp/otto-ux-r2-data-red4.log`, Database test missing Retry. Added pending/error state, inline shared LoadState + Retry, request/scope guards and preservation of existing rows on failed refresh. Saved query requests are workspace-scoped; connection snapshots no longer restore another workspace's saved list. History resets on fresh connection selection and refreshes on restored connection. Browser regression verifies saved and history error → Retry → actual empty states.

2. **P2 fixed — invalid replay selectors were actionable.** `ui/src/modules/brokers/ReplayPanel.svelte:37`. Blank count was enabled (red3 Replay assertion, received enabled). All selector branches now validate safe whole numbers, count/limit 1–5000, nonnegative partition/offsets, offset order, and an explicit valid local timestamp. Removed silent current-time substitution. Event handler checks the same guard. Test covers blank, zero, fractional, oversized, negative and timestamp cases. A separate successful mocked replay verifies the confirmation shows both topics, sends the exact selector, and renders returned evidence.

3. **P2 fixed — late group details replaced the current group.** `ui/src/modules/brokers/GroupsTab.svelte:128`. Held alpha, opened beta, released alpha: beta selection displayed alpha (red4 Groups failure). Added cluster/request/selection guards on success, error and completion; list requests are guarded too. Selection clears prior preview state.

4. **P2 fixed — old reset preview appeared under another group.** `GroupsTab.svelte:205`. Held alpha reset-preview response, selected beta, released alpha: preview persisted (expanded2 red, expected 0 preview nodes, received 1). Preview now snapshots group/cluster/form values, rejects stale results/errors, and does not clear another selection's loading state. `:181` also rejects blank reset timestamps and invalid offsets instead of substituting now.

5. **P3 fixed — database sidebar/query tabs lacked composite keyboard navigation.** `DatabasePage.svelte:710`, `:976`, `:991`; `QueryEditor.svelte:748`. Added roving tab stops and arrows/Home/End to desktop/phone sidebar tabs and query tabs. Nested rename/close controls keep their own keyboard handling. Browser regression exercises sidebar Home and query Home/End with focus and selection checks.

6. **P2 fixed — NetworkProfiles falsely claimed no SSH connections while pending/failed.** `ui/src/modules/connections/NetworkProfiles.svelte:8`, `:69`. Held request displayed “Create an SSH connection…” (red3 Network failure expected zero, received one). Added request-lifetime loading/error state, inline Retry, truthful empty guidance only after success, and disabled connection selection/save until the list is usable. Existing network-profile save/conflict/workspace/stale-response tests pass too.

7. **P2 visual fix — tablet group detail was too narrow to read the full lag table.** `GroupsTab.svelte:760`. At 1024px RTL the shell rail, cluster sidebar and fixed group list left approximately 280px for detail; lag columns lay outside the visible detail area. Actual screenshot reviewed, then extended stacked group-list/detail layout to tablet widths ≤1100px. Refreshed `tablet-rtl-brokers.png` shows the complete table, group identifier and reset controls.

No confirmed feasible issue remains in the owned paths exercised here. Shared LSP issue below is an explicit cross-scope handoff, not dismissed.

## Deeper verification

New durable spec: `ui/e2e/desktop-ux-r2-data.spec.ts` (16 tests).

Verified realistic browser flows: query execution with loaded rows; editable SQL result primary-key probe; double-click cell edit → pending changes → Discard with zero writes; keyboard navigation; GET request → JSON response tree; SFTP directory activation and long-name metadata/actions; Kafka group lag/selection/preview races; topic filtering and failed-detail Retry; replay invalid-input prevention and confirmed mocked success/evidence; SSH form delayed save failure retaining the draft and re-enabling submission; network-profile pending/failure/retry and existing save/conflict handling.

All transports that would contact a real broker/SSH/DB/API are fixture interceptions. Global connection/cluster profiles require unique seed names and broad mocked overview/test/metrics routes because the page can initially auto-select the previous global cluster. Early fixture attempts missed this and tried to resolve `fixture.invalid` in the isolated daemon; those were stopped and corrected. No real host was contacted. Early screenshot attempts captured modal entrance opacity and were replaced with `animations:'disabled'` screenshots. Exact-label/role fixture mistakes were corrected; product assertions were not weakened.

Commands run from `ui/`, common prefix:

```sh
OTTO_E2E_SLOT=ux2data OTTO_E2E_PORT=7843 OTTO_E2E_PW_PORT=5343 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod
```

- `npx playwright test e2e/desktop-ux-r2-data.spec.ts e2e/desktop-ux-data.spec.ts e2e/desktop-network-profiles.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-data-final` → **19 passed (50.7s)**. Log `/tmp/otto-ux-r2-data-final.log`; `.last-run.json` passed, failedTests=[] (before three added deeper tests).
- `npx playwright test e2e/desktop-ux-r2-data.spec.ts --grep 'Loaded data' --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-data-visual` → **7 passed (25.6s)**. Stable-animation screenshot refresh; log `/tmp/otto-ux-r2-data-visual.log`.
- `npx playwright test e2e/desktop-ux-r2-data.spec.ts --grep 'SSH connection form|Kafka topic search' --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-data-extra` → **2 passed (12.3s)**, log `/tmp/otto-ux-r2-data-extra.log`.
- `npx playwright test e2e/desktop-ux-r2-data.spec.ts --grep 'Loaded data workbenches phone|Replay confirms' --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-data-last` → **2 passed (11.8s)**. Phone screenshot now exercises the readable Vertical result mode. Log `/tmp/otto-ux-r2-data-last.log`.
- Extra native-light assertion confirms the API saved-request sidebar finishes loading (statecheck log first test passed; phone attempt interrupted due wrong test role and rerun green above). Loading in early fast screenshots was transient, not a stuck list.
- `npx tsc --noEmit -p tsconfig.e2e.json` → passed, `/tmp/otto-ux-r2-data-tsc-final.log` empty.
- `node scripts/ui-guards.mjs` → passed, 803 files, no new debt; `/tmp/otto-ux-r2-data-guards-final.log`. No baseline changes.
- `git diff --check` → passed. Full npm check remains parent-owned.

## Rendered evidence

35 actual-view screenshots at `/tmp/otto-ux-r2-data-screenshots/{variant}-{database,api,brokers,replay,sftp}.png`.

Variants: native-light, native-dark, warm-light, warm-dark, pro-dark at 1440×900; phone at 375×812 native light; tablet-rtl at 1024×768 Warm dark. Every screenshot test asserts no page-level horizontal overflow. SFTP filter bounds are asserted separately; phone test independently verifies name and delete controls.

Viewed with view_image, not only bounds tests: native-light all five; native-dark database/API/SFTP; warm-light database/API/SFTP; warm-dark database/replay/SFTP; pro-dark API/brokers/SFTP; phone all five; tablet-rtl all five. Viewed final refreshed native-light SFTP, phone Vertical database and tablet RTL groups after the respective corrections. Untested native Tauri rendering and real service operations are not implied by browser screenshots.

## Scores / 10

These are scoped UX audit judgments of verified browser flows, not a fabricated whole-product score. Shared LSP keeps the API family below target until the next reviewer resolves it; NetworkProfiles theme variants lack a dedicated screenshot sweep.

| Page family | Layout/readability | Interaction | Accessibility | States/recovery | Responsiveness | Overall |
|---|---:|---:|---:|---:|---:|---:|
| Database query/results/saved/history | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 | 9.5 |
| API request/response main flow | 9.5 | 9.3 | 9.5 | 9.4 | 9.5 | 9.3 |
| Brokers groups/topics/replay | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 | 9.5 |
| Connections SSH/SFTP | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| NetworkProfiles manager | 9.4 | 9.5 | 9.5 | 9.6 | 9.4 | 9.4 |

| Important variant | Database | API | Brokers | SSH/SFTP |
|---|---:|---:|---:|---:|
| Native light desktop | 9.5 | 9.3 | 9.5 | 9.5 |
| Native dark desktop | 9.5 | 9.3 | 9.5 | 9.5 |
| Warm light desktop | 9.5 | 9.3 | 9.5 | 9.5 |
| Warm dark desktop | 9.5 | 9.3 | 9.5 | 9.5 |
| Pro Dark desktop | 9.5 | 9.3 | 9.5 | 9.5 |
| Phone native light | 9.5 | 9.3 | 9.5 | 9.5 |
| Tablet Warm dark RTL | 9.5 | 9.3 | 9.5 | 9.5 |

NetworkProfiles has desktop behavioral coverage but its remaining theme/device combinations are not individually scored without rendered evidence.

## Cross-scope handoff and next-round depth

**Shared CodeEditor LSP dependency path — parent explicitly owns handoff.** Vite warns `events.EventEmitter` is externalized. `ResponseViewer.svelte:409` renders CodeEditor with virtual `response.json`, readOnly=true and workspace root. `ui/src/lib/components/CodeEditor.svelte:500` attempts LSP whenever completionSource is absent (no readOnly exclusion). `:202` maps JSON to JSON; `:334–336` attaches only if `/lsp/capabilities` advertises an available JSON server. `:341` calls languageServer. Its dependency `@marimo-team/codemirror-languageserver` loads nested `@open-rpc/client-js` 1.8.1. RequestManager and TransportRequestManager instantiate Node `events.EventEmitter` (optimized bundle lines 144/300), which Vite externalizes; TypeError is swallowed by CodeEditor `:355`, preserving highlighted editor behavior but losing LSP. This is preexisting; none of this round's changes touches CodeEditor/dependencies. Parent instructed leaving the shared file unchanged and assigned a fresh shared-access reviewer to reproduce/repair and verify across editors. No warning was suppressed.

Next round should independently verify all repairs, complete NetworkProfiles theme/device screenshots, test DB saved/history workspace/connection races beyond the primary error/retry path, and deepen full long-running connection/transfer and API automation/stream states using fixtures. Actual prod DB writes, SFTP transfers and Kafka produce/reset were intentionally not performed; mocked replay confirmation/evidence and edit staging cover their UI boundaries.
