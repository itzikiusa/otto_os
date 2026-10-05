# Concrete acceptance matrix

Each row requires an exact test/command, source revision, outcome and evidence path before it can become accepted. Source traces alone do not mark a row green. Expand this matrix from remaining reviewer reports. Tests use isolated data/servers only.

| Partition | Critical journey/state | Proposed evidence | Status |
|---|---|---|---|
| 1 | History open on working/running/idle does not restart; stale inactive snapshot cannot kill now-live session | Deferred/browser route assertions + session status authority | Resume Rust3/3 and session UI regressions pass; current merged mounted live/stale/warmed-preference and departed-resume cases GREEN |
| 1 | Reconnectable/exited/on-disk History resume/import retains intended behavior | Browser/API regression | Resume Rust3/3 passes; current merged mounted stale-shell resume and import-failure/retry cases GREEN |
| 1 | Open in Chat overrides cached Terminal preference without reload | Browser/store regression | Actual UI handler/store regression and current merged mounted warmed Terminal preference case GREEN |
| 1 | Disconnect/reconnect and earlier paging maintain bounded reachable history | Existing focused tests + affected browser rerun | Transcript lifecycle tests pass in session39/39 batch; current browser/load rerun pending |
| 2 | Replay binary key/value/header, tombstone, present-empty record exactly | Raw-producer tests + isolated Kafka consume assertion | Final isolated Kafka replay6/6 passes, including nullable headers and present-empty records |
| 2 | UTF-8 preview truncation at boundaries never panics | Rust regression including 63 ASCII + é | Actual Kafka replay Unicode regression passes; part of original5 green cases |
| 2 | Guarded query/managed write A→B switch prompts actual origin; cancellation/access loss suppresses retry | Deferred UI unit test matrix | Actual store tests pass in data-tools80/80 batch; mounted confirmation pending |
| 2 | API workspace A→B→A loads/mutations/activation reject stale publication | Deferred UI store tests | Actual store ownership tests pass in data-tools80/80 batch; environment A→B→A mounted control GREEN; broader collection/automation matrix remains bounded |
| 2 | Environment save preserves newer draft and does not reseed another selected environment | Mounted browser/component regression | Handler ownership/recovery regressions pass in data-tools80/80 batch; newer-value/double-secret-rename and clean A→B→A mounted cases GREEN |
| 3 | Concurrent Design metadata patches preserve independent fields; approval survives concurrent title patch | Rust barrier/transaction tests, aliased services | Final Design128 passed/1 ignored includes all five original and adjacent writer regressions |
| 3 | Browser reader/live tab creation after workspace switch preserves current workspace | Deferred UI store regressions | Actual store ownership regressions pass; browser/Vault follow-up18/18; mounted pending |
| 3 | Browser A navigation after selecting B persists request-local A title; reversed same-tab navigation order | Deferred UI store regressions | Actual navigation ownership/reversal regressions pass; mounted pending |
| 3 | Excalidraw restore reapplies persisted background/grid and next save retains them | Editor/component regression | Canvas11/11 passes including grid-only autosave; mounted 3MB save/version restore and Excalidraw failure recovery GREEN; native editor matrix not certified |
| 4 | Concurrent workflow graph saves publish atomic graph/version/snapshot; admitted run sees matching snapshot | Rust deterministic concurrency tests | State3/3 and actual server8/8 pass publication/pinning/rollback cases; independent source approval |
| 4 | Retimed one-shot schedule survives old execution completion | Rust scheduler deferred/CAS regression | Current actual server30/30 includes all six pause/settlement cases; grouped state consumers106/106 green |
| 4 | Retimed fired workflow trigger rearms exactly once | Rust trigger/cadence tests | Current actual server30/30 includes stale admission, overlap and rollback; grouped state consumers106/106 green |
| All | Light/dark rendered states, keyboard/focus, phone/tablet overflow on changed surfaces | Inspected screenshots + affected Playwright cases | Publication dialog desktop/phone light/dark captures inspected at64a850e6; full changed-surface/native matrix remains pending |
| Performance | CPU/RAM/latency at N=0/1/3/5; sustained comparable view cycles; real-app read-only sample | Isolated load harness + current-source measurements | Scale N=0/1/3/5 and recovery complete; read-only installed-app sample complete; 15-minute sustained run safety-aborted at 414 seconds, so full acceptance remains incomplete |

No native Tauri/assistive technology/physical-device acceptance is inferred from Chromium tests. Full desktop suite failures must be triaged against baseline and reported faithfully. Environment failures do not count as product passes.
# Additional iteration4 acceptance cases

These are required checks, not executed results; VERIFICATION.md records actual runs.

| Repair | Required behavior and failure coverage | Current execution |
|---|---|---|
| Data dialogs | Import never runs an unsent SQL draft; partial collection creation retries the same ID; Recovery Refresh retries history; environment Save retains newer edits and selected scope | Final data-tools80/80 covers handler/store follow-ups, including canceled collection creation and environment A→B→A; mounted cases pending |
| Database guarded writes | Both entrypoints name and target origin A after selecting B; rejection, cancellation and access revocation prevent retry; agent production gate remains typed | 10 production-store cases green; mounted confirmation pending |
| API client ownership | Collections/environments/automations, errors and loading states stay with their initiating visit, including A→B→A; late activation cannot alter B's next Send | 21 new production-store cases green plus 26 existing controls; mounted scope transitions pending |
| Publication preview | Jira and Confluence receive exactly the reviewed version plus title/reference payload, or reject stale preview for rereview; account/destination retries retain form and ignore old scope | Product HTTP24/24 and server approval cases pass; current merged mounted Jira/RFC409 rereview, destination retry, departed success and reopened-dialog ABA cases GREEN |
| Canvas assist | Failure retains prompt; accepted request only clears its unchanged originating draft; scene changes/remounts never resurrect or erase another scene's draft | Canvas11/11 passes prompt ownership and persistence controls; all three mounted assist formats pass failure/empty/accepted prompt recovery |
| Vault lookup | Pending/failed lookup cannot masquerade as successful empty or trigger ordinary Enter-to-create; retry current query, ignore stale results; retain last good tags with error feedback | Vault26/26 and browser/Vault18/18 controls pass; mounted backlinks failure/Retry and failed open/save recovery GREEN; complete keyboard lookup matrix remains bounded |
| Scheduled Task and Goal Loop drafts | Sidebar/back/workspace navigation uses same dirty decision as local Back; Keep editing retains every field; discard leaves; pending save/launch completion owns its view | Pending |
| Scheduled Task newer edits | Save only acknowledges submitted draft; later edits survive; create followed by more edits adopts created ID for PATCH; failures preserve current inputs | Mounted existing-task deep link, failed Save, newer typing, Keep editing and final persistence GREEN; create-ID path covered by handler tests |
| Insights immediate save | Slow schedule PUT followed by model edits drains latest model; errors preserve honest pending/error/recovery state; provider switch cannot replay obsolete model | Pending |
| One-time token reveal | Click/Enter cannot mint again until explicit dismissal; clipboard failure retains secret; dismiss permits next mint; revocation targets intended token | Platform26/26 covers token/provider ownership and recovery; mounted reveal journey pending; raw secrets are never persisted |
| Account picker | Pending/failure/retry without losing session form; provider/unmount fencing; selecting explicit account is preserved; list retry never signs in or creates accounts | Pending |

## Latest integration dependencies

- Product HTTP24/24, reviewed-publication UI17/17 and workflow preview UI8/8 pass. The final server30/30 includes displayed-preview version CAS, denied/stale/forged approvals and exact isolated publication. Grouped106/106 reruns the latest Product byte projections. Mounted human workflows remain pending.
- Recap client10/10, current backend revision5/5 and authenticated HTTP1/1 pass. Cold unchanged polls read no event/draft bodies or index. Modal archive identity now has corrected mounted WebKit RED→keyed repair→1/1 GREEN and independent source approval.
- Workbench UI, actual exclusive-cursor HTTP and explicit50k bounded-history checks pass. MCP handler/catalog checks pass; current merged mounted305revision paging/comparison/oldest restore GREEN.
- Git all five raw/render/capture budget regressions pass in grouped106/106. Exact omitted-file counts still incur linear disk reads; representative CPU/RAM remains pending.
- Current automation server30/30 and grouped state consumers106/106 pass. The earlier broad state timing failure is preserved in VERIFICATION.md; its exact rerun passed unchanged, and full affected checks remain queued.
- Merged UI guards/types check GREEN0errors/0warnings; unit1333/1333GREEN. Full affected Rust4694pass/2fail/86skip; both exact repaired checksGREEN. Current workspace all-target clippy/fmtGREEN; workspace doc-testsGREEN (33libraries,0cases). Scheduled atomic admission including0174index migration and prior controlsGREEN57/57. See VERIFICATION.md for exact source/log scope.

## Iteration5 added defect acceptance

| Defect | Current evidence | Remaining limit |
|---|---|---|
| History resume after departure steals navigation | Actual mounted RED then GREEN; current merged departed-resume and ordinary resume/import controls pass | Final independent rescore pending |
| Product publication after departure/reopen steals selected story/dialog | Actual mounted RED then GREEN; reopened-dialog A→B→A and ordinary Jira/RFC409 recovery pass,4 theme/viewport captures inspected | Final independent rescore pending |
| PersonalAgents A→B→A list response/error/finally overwrites latest visit |3 deferred production-store regressions RED→GREEN within full1333-pass unit run | Three mounted ABA controls GREEN (old success/error during loading; old success after current) |
| Scheduled scan admitted after disable/retime, or admitted twice |5 intended REDs then actual server/state57/57GREEN with disable/retimeABA,concurrent admission,replay,rollback and running-row index | Final independent rescore pending |

Latest complete review means before final evidence rescore: correctness8.96 (before four repairs), performance8.78, UX9.72 (iteration5). External design9.96/min9.80 is static. Internal bounded design repair review now9.98/min9.90; populations remain separate. These are reviewer judgments, not measured reliability percentages.

## Closing evidence and remaining scope

The latest executed results above supersede historical “pending” checkpoints in VERIFICATION.md. Final category/partition judgments are in [SCORES.md](SCORES.md) and [iteration-6-scores.md](iteration-6-scores.md); they are not test pass percentages.

The Product startup deep-link defect added during final acceptance is repaired and has three repeated mounted passes, including canceled and accepted dirty navigation. Final integrated UI gates are 0 errors / 0 warnings, 1,335 unit passes, successful production build and unchanged bundle budget.

Outstanding acceptance scope: the sustained run did not reach its 15-minute completion/recovery checkpoint; representative DB/Kafka/Git/content/scheduler workloads lack the same CPU/RAM coverage as synthetic agent streaming; native WebKit process attribution and physical-device/assistive-technology coverage are incomplete. These gaps are retained, not converted into passes. The three requested review iterations can close with these explicitly reported limits; the 9.8-every-partition target is not met.
