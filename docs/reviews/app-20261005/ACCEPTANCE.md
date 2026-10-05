# Concrete acceptance matrix

Each row requires an exact test/command, source revision, outcome and evidence path before it can become accepted. Source traces alone do not mark a row green. Expand this matrix from remaining reviewer reports. Tests use isolated data/servers only.

| Partition | Critical journey/state | Proposed evidence | Status |
|---|---|---|---|
| 1 | History open on working/running/idle does not restart; stale inactive snapshot cannot kill now-live session | Deferred/browser route assertions + session status authority | Resume Rust3/3 and session UI39/39 pass; mounted route assertions pending |
| 1 | Reconnectable/exited/on-disk History resume/import retains intended behavior | Browser/API regression | Resume Rust3/3 passes named supported/inactive/archived cases; mounted import/resume pending |
| 1 | Open in Chat overrides cached Terminal preference without reload | Browser/store regression | Actual UI handler/store regression passes in session39/39 batch; mounted pending |
| 1 | Disconnect/reconnect and earlier paging maintain bounded reachable history | Existing focused tests + affected browser rerun | Transcript lifecycle tests pass in session39/39 batch; current browser/load rerun pending |
| 2 | Replay binary key/value/header, tombstone, present-empty record exactly | Raw-producer tests + isolated Kafka consume assertion | Final isolated Kafka replay6/6 passes, including nullable headers and present-empty records |
| 2 | UTF-8 preview truncation at boundaries never panics | Rust regression including 63 ASCII + é | Actual Kafka replay Unicode regression passes; part of original5 green cases |
| 2 | Guarded query/managed write A→B switch prompts actual origin; cancellation/access loss suppresses retry | Deferred UI unit test matrix | Actual store tests pass in data-tools80/80 batch; mounted confirmation pending |
| 2 | API workspace A→B→A loads/mutations/activation reject stale publication | Deferred UI store tests | Actual store ownership tests pass in data-tools80/80 batch; mounted transitions pending |
| 2 | Environment save preserves newer draft and does not reseed another selected environment | Mounted browser/component regression | Handler ownership/recovery regressions pass in data-tools80/80 batch; mounted pending |
| 3 | Concurrent Design metadata patches preserve independent fields; approval survives concurrent title patch | Rust barrier/transaction tests, aliased services | Final Design128 passed/1 ignored includes all five original and adjacent writer regressions |
| 3 | Browser reader/live tab creation after workspace switch preserves current workspace | Deferred UI store regressions | Actual store ownership regressions pass; browser/Vault follow-up18/18; mounted pending |
| 3 | Browser A navigation after selecting B persists request-local A title; reversed same-tab navigation order | Deferred UI store regressions | Actual navigation ownership/reversal regressions pass; mounted pending |
| 3 | Excalidraw restore reapplies persisted background/grid and next save retains them | Editor/component regression | Canvas11/11 passes including grid-only autosave; mounted/backend persistence pending |
| 4 | Concurrent workflow graph saves publish atomic graph/version/snapshot; admitted run sees matching snapshot | Rust deterministic concurrency tests | State3/3 and actual server8/8 pass publication/pinning/rollback cases; independent source approval |
| 4 | Retimed one-shot schedule survives old execution completion | Rust scheduler deferred/CAS regression | Current actual server30/30 includes all six pause/settlement cases; grouped state consumers106/106 green |
| 4 | Retimed fired workflow trigger rearms exactly once | Rust trigger/cadence tests | Current actual server30/30 includes stale admission, overlap and rollback; grouped state consumers106/106 green |
| All | Light/dark rendered states, keyboard/focus, phone/tablet overflow on changed surfaces | Inspected screenshots + affected Playwright cases | Pending; Claude coordinates design changes |
| Performance | CPU/RAM/latency at N=0/1/3/5; sustained comparable view cycles; real-app read-only sample | Isolated load harness + current-source measurements | Pending |

No native Tauri/assistive technology/physical-device acceptance is inferred from Chromium tests. Full desktop suite failures must be triaged against baseline and reported faithfully. Environment failures do not count as product passes.
# Additional iteration4 acceptance cases

These are required checks, not executed results; VERIFICATION.md records actual runs.

| Repair | Required behavior and failure coverage | Current execution |
|---|---|---|
| Data dialogs | Import never runs an unsent SQL draft; partial collection creation retries the same ID; Recovery Refresh retries history; environment Save retains newer edits and selected scope | Final data-tools80/80 covers handler/store follow-ups, including canceled collection creation and environment A→B→A; mounted cases pending |
| Database guarded writes | Both entrypoints name and target origin A after selecting B; rejection, cancellation and access revocation prevent retry; agent production gate remains typed | 10 production-store cases green; mounted confirmation pending |
| API client ownership | Collections/environments/automations, errors and loading states stay with their initiating visit, including A→B→A; late activation cannot alter B's next Send | 21 new production-store cases green plus 26 existing controls; mounted scope transitions pending |
| Publication preview | Jira and Confluence receive exactly the reviewed version plus title/reference payload, or reject stale preview for rereview; account/destination retries retain form and ignore old scope | Product HTTP24/24, publication UI17/17 and current server30/30 pass, including actual isolated Jira/Confluence payload assertions; authored mounted cases pending |
| Canvas assist | Failure retains prompt; accepted request only clears its unchanged originating draft; scene changes/remounts never resurrect or erase another scene's draft | Canvas11/11 passes prompt ownership and persistence controls; mounted hosts pending |
| Vault lookup | Pending/failed lookup cannot masquerade as successful empty or trigger ordinary Enter-to-create; retry current query, ignore stale results; retain last good tags with error feedback | Vault26/26 and browser/Vault18/18 controls pass; integrated presentation and mounted journeys pending |
| Scheduled Task and Goal Loop drafts | Sidebar/back/workspace navigation uses same dirty decision as local Back; Keep editing retains every field; discard leaves; pending save/launch completion owns its view | Pending |
| Scheduled Task newer edits | Save only acknowledges submitted draft; later edits survive; create followed by more edits adopts created ID for PATCH; failures preserve current inputs | Pending |
| Insights immediate save | Slow schedule PUT followed by model edits drains latest model; errors preserve honest pending/error/recovery state; provider switch cannot replay obsolete model | Pending |
| One-time token reveal | Click/Enter cannot mint again until explicit dismissal; clipboard failure retains secret; dismiss permits next mint; revocation targets intended token | Platform26/26 covers token/provider ownership and recovery; mounted reveal journey pending; raw secrets are never persisted |
| Account picker | Pending/failure/retry without losing session form; provider/unmount fencing; selecting explicit account is preserved; list retry never signs in or creates accounts | Pending |

## Latest integration dependencies

- Product HTTP24/24, reviewed-publication UI17/17 and workflow preview UI8/8 pass. The final server30/30 includes displayed-preview version CAS, denied/stale/forged approvals and exact isolated publication. Grouped106/106 reruns the latest Product byte projections. Mounted human workflows remain pending.
- Recap client10/10, current backend revision5/5 and authenticated HTTP1/1 pass. Cold unchanged polls read no event/draft bodies or index. Modal archive identity now has corrected mounted WebKit RED→keyed repair→1/1 GREEN and independent source approval.
- Workbench UI11/11, actual exclusive-cursor HTTP and explicit50k bounded-history checks pass. MCP catalog/handler2/2 passes; mounted history remains pending.
- Git all five raw/render/capture budget regressions pass in grouped106/106. Exact omitted-file counts still incur linear disk reads; representative CPU/RAM remains pending.
- Current automation server30/30 and grouped state consumers106/106 pass. The earlier broad state timing failure is preserved in VERIFICATION.md; its exact rerun passed unchanged, and full affected checks remain queued.
- Combined UI check passed before the newest workflow/recap/contract additions. That earlier result does not cover current combined source or Claude integration.
