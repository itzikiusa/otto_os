# Iteration 4 browser execution queue

Source inventory only; no browser command was executed for this document. Root owns execution, the heavy-command lease and result recording in `VERIFICATION.md`. This queues the existing authored journeys without adding coverage or claiming acceptance.

## Authored desktop journeys

All paths below are relative to `ui/`. Use **desktop-browser**, one worker, no grep: each selected file contains only its focused authored cases.

| Implementation role | Spec | Cases | Mounted coverage |
|---|---|---:|---|
| 1 | `e2e/desktop-review4-session-recovery.spec.ts` | 5 | Child earlier/newer/collapse; mixed batch retry; live/stale History and warmed Terminal preference; inactive shell resume; import/conflict/retry |
| 2 | `e2e/desktop-review4-workbench-history.spec.ts` | 1 | Bounded revision pages, oldest selection, direct comparison and restore |
| 3, authored by role 1 | `e2e/desktop-review4-product-publication.spec.ts` | 3 | Jira and RFC same-version conflict/review reload; RFC destination retry and departed-account response |
| 3, authored by role 1 | `e2e/desktop-review4-product-transcripts.spec.ts` | 1 | Summary-only loading, chosen body expansion, older-history search and page restoration |
| 4 | `e2e/desktop-goal-draft-ownership.spec.ts` | 1 | Workspace Keep/Discard, persistent goal form and non-resurrection |
| 5 | `e2e/desktop-platform-settings-recovery.spec.ts` | 1 | Model-save failure, debounce, pending write and retry state |

Expected **12 desktop cases**. The publication file generates two cases from its story/RFC loop. No mounted Product workflow-approval spec was authored in this set; its handler/backend results do not add browser cases.

Run from `/Users/itziklavon/claude_ade-review/ui`, only after root has a current-tree daemon binary and the heavy slot. The ports below are an explicit proposed isolated allocation; root must ensure they are free and not another run's servers.

```sh
OTTO_E2E_SLOT=review05accept OTTO_E2E_PORT=7841 OTTO_E2E_PW_PORT=5341 \
OTTO_E2E_UI=http://localhost:5341 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_SECRETS=file \
OTTO_E2E_BIN=/Users/itziklavon/claude_ade-review/target/debug/ottod \
npx playwright test \
  e2e/desktop-review4-session-recovery.spec.ts \
  e2e/desktop-review4-workbench-history.spec.ts \
  e2e/desktop-review4-product-publication.spec.ts \
  e2e/desktop-review4-product-transcripts.spec.ts \
  e2e/desktop-goal-draft-ownership.spec.ts \
  e2e/desktop-platform-settings-recovery.spec.ts \
  --project=desktop-browser --workers=1 \
  --output=/tmp/otto-review05-browser-acceptance
```

## RoomRecap identity, separately

Role 5's exact case is **`open room recap replaces archive identity and discards the old pending page`**, in `e2e/room-recap.spec.ts`. Run only this case on **ipad-landscape**; expected **1 case**. `desktop-browser` only matches `desktop-*.spec.ts`; `desktop-light` is not a configured project despite stale skip text elsewhere in the file. This case remains a requested mounted regression, not a claimed green repair.

After the desktop invocation and teardown finish:

```sh
OTTO_E2E_SLOT=review05recap OTTO_E2E_PORT=7842 OTTO_E2E_PW_PORT=5342 \
OTTO_E2E_UI=http://localhost:5342 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_SECRETS=file \
OTTO_E2E_BIN=/Users/itziklavon/claude_ade-review/target/debug/ottod \
npx playwright test e2e/room-recap.spec.ts \
  --project=ipad-landscape --workers=1 \
  --grep 'open room recap replaces archive identity and discards the old pending page' \
  --output=/tmp/otto-review05-recap-identity
```

## Runner and fixture limits

- `playwright.config.ts` starts Vite at `OTTO_E2E_PW_PORT`; UI origin must match the storage-state origin. Setup creates a fresh temporary daemon/data directory, onboards root, writes `e2e/.auth-<slot>/{state,daemon}.json`, and supplies `otto_base` plus its token. `apiCtx()` reads that same slot. Never reuse another slot's auth metadata. The HTML report is `e2e/.report-<slot>`; failure traces/screenshots use the explicit output directory.
- Set `OTTO_E2E_BIN` explicitly: otherwise setup launches the installed binary against temporary data, and new routes can return misleading 404s. The existing transcript-route startup probe does not prove newer resume/recap/publication routes exist. These commands do not build the binary. Vite's configured `reuseExistingServer` can reuse an occupied port outside CI; verify server ownership instead of accidentally testing another checkout.
- Setup installs harmless provider shims and offline agent settings; secrets stay in its temporary file store. Teardown kills that daemon and its data-directory-matched ClickHouse children. The explicit orphan-sweep opt-out avoids global cleanup affecting another coordinated run. Inspect cleanup after abnormal startup/termination; do not broaden it to other agents' processes.
- Session/Product/Workbench fixtures use real isolated records plus bounded route responses. Publication requests are fulfilled locally, with no external send. History import is stubbed to an existing shell identity; this does not prove real provider import. Product stories are global, so fixtures select unique titles. Workbench and goal specs currently do not explicitly block service workers: if route assertions are bypassed, inspect interception/worker state before attributing a UI failure.
- RoomRecap mounts the actual host component through its Vite fixture module, intercepts the membership WebSocket, pauses the clock, and holds an obsolete page. Its corrected fixture preserves the real isolated owner token and blocks service workers so route fixtures cannot be bypassed; an earlier synthetic-token/service-worker attempt failed before reaching archive identity. The paused clock intentionally requires identity replacement immediately, without waiting for polling.
- Chromium/tablet route fixtures are not native macOS acceptance, real provider integration, measured CPU/RAM proof, or a full-app score. Preserve exact failure output and classify fixture failures before changing assertions or claiming regressions.

Inventory sources: the five `iteration-4-implementation-*.md` reports, `iteration-4-role5-ui-recheck.md`, current specs, `ui/playwright.config.ts`, and `ui/e2e/global-{setup,teardown}.ts`.
