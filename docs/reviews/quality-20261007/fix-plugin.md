# Team Performance repair checkpoint

Scope: D3, UX-02/05/06, NS1/NS2 from the round-1 review. Plugin files only;
no production Jira calls, live plugin mutation, publication, staging or commits.

## Implemented

- Account-owned in-memory settings drafts preserve field values, time off,
  workers and project-specific controls across refresh and navigation. Save
  acknowledges a captured revision; newer edits and failed saves remain drafts.
- Account, people, overview and scan requests commit only while their captured
  generation/scope is current. Old successes, failures and completion refreshes
  cannot replace the selected scope.
- Overlay stack gives the topmost dialog/popover Escape ownership, marks covered
  dialogs inert and restores focus to the surviving trigger.
- Scan polling labels unavailable status, preserves last known progress and
  provides Retry. Stop reaches the server and cancellation reaches sockets,
  pacing/backoff, PR dispatch, estimator lanes and owned git process groups.
  Partial Jira work and completed estimates/PRs are retained; the watermark does
  not advance after a stopped partial scan. Restart is admitted after cleanup.
- Deployment range cache persists across actual worker invocations, with compact
  prefix digests and a 5,000-entry / 16 MiB bound. It retains exclusions against
  every earlier deployment, including divergent release branches.

API and limits: [SCAN-CONTRACT.md](../../../examples/plugins/team-performance/SCAN-CONTRACT.md)
(the repository-relative path is `examples/plugins/team-performance/SCAN-CONTRACT.md`).
Already accepted remote agent runs lack a host cancellation handle and may finish
there after the local request closes. Local synchronous analysis cancels at its
next event-loop boundary. Cold deployment-tag walks retain their previous
quadratic exclusion-input cost; oversized cache working sets may recompute.
Drafts survive navigation within the frame, not a frame/browser reload.

## Evidence ledger

Coordinator ran heavy/browser suites sequentially. Worker ran only the expressly
authorized lightweight cancellation RED suite.

| Check | Baseline | Fixed checkpoint |
| --- | --- | --- |
| Browser overlay/draft/stale overview/status recovery; separate git workers; pacer backoff | 6 expected failures, `/tmp/otto-quality-20261007-plugin-red.log` | included below |
| Jira active I/O; feature-estimate dispatch; pacer backoff | 3 expected failures in 1.01 s, `/tmp/otto-quality-20261007-plugin-cancel-red.log` | included below |
| Real sidecar Stop mid-changelog then restart | expected 404 vs 200, `/tmp/otto-quality-20261007-plugin-server-red.log` | included below |
| Focused regressions (browser 5, git, pacer, estimates, Jira, sidecar) | as above; late-account case also added | 10/10 pass, 9.08 s, `/tmp/otto-quality-20261007-plugin-green.log` |
| Follow-up edge regressions | pending coordinator run | pending |
| Full plugin suite | initial 453 pass / 2 intentional skips | pending |

Current follow-up checks: cancel a queued account behind another active account;
Tab trapping in popovers above modals; submission of status-map drafts across
projects. Added bounded-cache reload/corruption tests. Full-suite and rendered
light/dark/phone review remain pending; this checkpoint is not a complete quality
claim. Plugin tests run separately from `ui/`'s default gate with
`node --test --test-concurrency=1 test/*.test.js` in the plugin directory.

## Estimate correction contract follow-up

Coverage review confirmed that the browser correction test's skip concealed a
production mismatch: `flow.estimateAccuracy` returned histogram totals while the
Estimates view required `bins` and `worst`. No real ticket could expose Correct.

Repaired the producer additively: existing histogram/fraction/median fields stay
compatible; the UI receives ordered bins, count, fraction and at most 30 actual
ticket misses ranked by absolute log ratio. The same eligible population drives
all fields. Candidate rows preserve corpus identity and the effective estimate,
including lead corrections. Zero actual is retained, invalid estimates excluded,
and empty populations remain explicit. No fabricated overview response is used.
See `examples/plugins/team-performance/METRICS-CONTRACT.md` for the contract.

The browser fixture now explicitly estimates all dates via the real scan and
selects All time in the actual toolbar. Its correction test must find TP-1 and
verify persisted 3.5-day correction/reason; absence fails instead of skipping.

- Pure RED: 3/3 failed for missing producer fields, coordinator log
  `/tmp/otto-quality-20261007-estimate-contract-red.log` (174 ms).
- Pure GREEN: 3/3 passed, worker authorized bounded run, log
  `/tmp/otto-quality-20261007-estimate-contract-green.log` (186 ms).
- Required browser correction and full plugin suite: pending coordinator gates.

## All time period follow-up (R2-T3)

The mandatory real-browser correction exposed another production mismatch:
All time completion counts kept June history while overview/person metrics
silently substituted a 90-day start. Browser RED is recorded at
`/tmp/otto-quality-20261007-estimate-browser-red.log`.

Added pure `lib/view-window.js`: explicit positive cutoffs remain exact; All time
starts at the first valid activity in the whole scoped record set,
rounded to the UTC boundary used by current capacity. Empty history has a zero
window, never an epoch fallback. Both overview and person paths share the same
scope/day observation window. Metric cache keys now preserve exact caller bounds;
explicit report windows do not go through the daily view-window resolver.

Added five pure edge cases and two real-server regressions for old-history
accuracy/flow, exact team/person window agreement, explicit cutoff exclusion and
bounded empty data. The browser test retains actual All time selection. No tests
were run during the coordinator's matched performance measurement; those gates
remain queued. Production and tests are frozen for independent review.


### Independent window-scope correction

Review caught that `scope.side` holds global PR/deployment caches. The final
resolver uses authoritative selected-record timestamps only, including
`first_commit_at`, `first_deployed_at` and `deployed_at`. Raw side timestamps are
excluded even when keys or repo/tag names appear to match: two Jira accounts can
share a key, and aggregate record repo lists cannot prove deployment ownership.
Actual included records remain consistent with the metric population; no
window-only feature exclusion was added.

- Initial pure scope RED reproduced a 2000 start instead of June 2026:
  `/tmp/otto-quality-20261007-view-scope-red.log`.
- Final pure RED proved missing first-deployment history and matching-looking
  global cache contamination: `/tmp/otto-quality-20261007-view-records-red.log`.
- Final pure GREEN: all five resolver cases passed, including owned deployment
  history, account-key/repo-tag collisions, empty scope, cutoffs and invalid dates:
  `/tmp/otto-quality-20261007-view-records-green.log`, 80 ms.
- Source frozen; mandatory browser and full suite remain coordinator-owned.
