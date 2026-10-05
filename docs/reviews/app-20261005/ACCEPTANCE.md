# Concrete acceptance matrix

Each row requires an exact test/command, source revision, outcome and evidence path before it can become accepted. Source traces alone do not mark a row green. Expand this matrix from remaining reviewer reports. Tests use isolated data/servers only.

| Partition | Critical journey/state | Proposed evidence | Status |
|---|---|---|---|
| 1 | History open on working/running/idle does not restart; stale inactive snapshot cannot kill now-live session | Deferred/browser route assertions + session status authority | Pending |
| 1 | Reconnectable/exited/on-disk History resume/import retains intended behavior | Browser/API regression | Pending |
| 1 | Open in Chat overrides cached Terminal preference without reload | Browser/store regression | Pending |
| 1 | Disconnect/reconnect and earlier paging maintain bounded reachable history | Existing focused tests + affected browser rerun | Pending |
| 2 | Replay binary key/value/header, tombstone, present-empty record exactly | Raw-producer tests + isolated Kafka consume assertion | Pending |
| 2 | UTF-8 preview truncation at boundaries never panics | Rust regression including 63 ASCII + é | Pending |
| 2 | Guarded query/managed write A→B switch prompts actual origin; cancellation/access loss suppresses retry | Deferred UI unit test matrix | Pending |
| 2 | API workspace A→B→A loads/mutations/activation reject stale publication | Deferred UI store tests | Pending |
| 2 | Environment save preserves newer draft and does not reseed another selected environment | Mounted browser/component regression | Pending |
| 3 | Concurrent Design metadata patches preserve independent fields; approval survives concurrent title patch | Rust barrier/transaction tests, aliased services | Pending |
| 3 | Browser reader/live tab creation after workspace switch preserves current workspace | Deferred UI store regressions | Pending |
| 3 | Browser A navigation after selecting B persists request-local A title; reversed same-tab navigation order | Deferred UI store regressions | Pending |
| 3 | Excalidraw restore reapplies persisted background/grid and next save retains them | Editor/component regression | Pending |
| 4 | Concurrent workflow graph saves publish atomic graph/version/snapshot; admitted run sees matching snapshot | Rust deterministic concurrency tests | Pending |
| 4 | Retimed one-shot schedule survives old execution completion | Rust scheduler deferred/CAS regression | Pending |
| 4 | Retimed fired workflow trigger rearms exactly once | Rust trigger/cadence tests | Pending |
| All | Light/dark rendered states, keyboard/focus, phone/tablet overflow on changed surfaces | Inspected screenshots + affected Playwright cases | Pending; Claude coordinates design changes |
| Performance | CPU/RAM/latency at N=0/1/3/5; sustained comparable view cycles; real-app read-only sample | Isolated load harness + current-source measurements | Pending |

No native Tauri/assistive technology/physical-device acceptance is inferred from Chromium tests. Full desktop suite failures must be triaged against baseline and reported faithfully. Environment failures do not count as product passes.
