# Otto responsiveness implementation

This implements the twelve findings in the [performance review](2026-09-13-performance-review.md). The objective is bounded waiting and less repeated work as histories, workflows and Vaults grow. It does not claim measured improvements in production frame rate or release-mode latency.

## Changes

| Area | Result | Practical limit |
|---|---|---|
| API scripts | Pre-request and post-response scripts run in dedicated, cancellable Workers; each send retains its originating workspace, tab and environment. | Five-second deadline, bounded logs/tests and input/output messages. Worker isolation does not impose a hard JavaScript heap limit or provide a security sandbox. |
| Agents | Only viewed conversations resync; closed views cannot resume providers. Transcript folds are reused for unchanged source versions with bounded cold-work admission and retained caches. | A cold or changed transcript still requires a complete fold. Active views and separately owned drafts remain available. |
| Connections | Database drivers and SSH tunnels initialize per key, so a blocked new connection does not hold up a different warm connection. Closing retires the old lifecycle and owns its cleanup. | The selected remote endpoint can still be slow; independent keys no longer share its initialization wait. |
| Git | Worktree status uses bounded subprocesses, four probes per listing and eight globally, with a ten-second optional-probe phase. | Failed or skipped status is explicitly unknown. Fresh unknown status rejects forced removal; normal removal retains Git's safety checks. |
| Folder picker | Filesystem browsing runs off the async executor with four admitted jobs, server/client deadlines and cancellation. Large listings use bounded rendered rows and complete keyboard navigation. | A blocked filesystem syscall cannot be forcibly interrupted; it retains admission until it exits. Search still covers the complete returned listing. |
| Vault saves | Existing-note saves update that note's metadata, tags, links and search index transactionally, without walking every file. | Structural changes still require global link resolution to preserve ambiguity and incoming-link behavior. |
| Vault directories | Compact per-Vault indexes serve direct children; UI refreshes only visible open branches with bounded concurrency. | Cold hydration and periodic filesystem discovery still scale with total Vault size. |
| Vault large files | Automatic indexing streams exact hashes and limits content parsing/retention to 4 MiB, exposing size_limited metadata. | Oversized source files remain intact and available to explicit raw reads; hashing still reads every byte. |
| API history | Summary lists omit stored request/response bodies; selected details load on demand and stale selection results are ignored. Direct/event refreshes coalesce. | Legacy full-detail routes remain compatible. Explicit detail/replay still loads its stored body. |
| Agents History | An additive cursor endpoint pages persisted metadata before transcript resolution; UI preserves continuation through filtered candidate windows. | The legacy array endpoint retains its original behavior. An empty filtered page can still have more candidates. |
| Workflow progress | Revisioned summaries replace repeated full checkpoint-body polling; run lists use summaries and node/checkpoint details load on expansion. | Full recovery data remains persisted. Explicitly expanded details can be large. |
| SFTP upload progress | Progress probes target the exact staging file with bounded output and failure backoff. | Remote probe failure can temporarily suppress intermediate progress; completion remains determined by the actual transfer. |

## Review and verification

Design and implementation plan were written and independently reviewed before implementation. Separate implementation compliance and correctness reviews cover each lane. Review-triggered regression fixes include API environment ownership, nested folder-picker keyboard focus, and Vault publication-failure recovery. Full-suite verification exposed three checkpoint tests that hand-built an obsolete schema; their fixture now uses the real migration bootstrap while preserving retry and no-repeat assertions. A held-file timestamp regression also verifies nanosecond precision through the portable macOS/Linux stat conversion.

Verified local gates include 2,932 Rust unit/integration tests (zero failures, 66 ignored), all 32 documentation-test targets, 119 UI unit tests, zero UI type errors/warnings, a production frontend build, 16 browser integration tests against the newly built isolated daemon (13 combined UI cases plus three real HTTP/WebSocket authorization and live-tail cases), workspace formatting, strict workspace Clippy across all targets, and the six mocked deployment test groups. All independent implementation reviews are approved; see the [review record](../superpowers/specs/2026-09-13-responsiveness-implementation-review.md). The full workspace test command and strict Clippy both exited successfully. The delivery PR must pass its independent GitHub Actions checks before merge and installation.

The browser suite must use an explicitly built test daemon and temporary state. For parallel local work, disable global orphan-process sweeping while retaining teardown of the run's own fixture:

```sh
# From the repository root, after the source is stable:
CARGO_BUILD_JOBS=2 cargo build -p ottod
# Then from ui/ (the binary path is resolved before Playwright starts):
OTTO_E2E_BIN="$(cd .. && pwd)/target/debug/ottod" \
OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_SLOT=perf-root \
OTTO_E2E_PORT=7894 OTTO_E2E_PW_PORT=5294 OTTO_SECRETS=file \
npx playwright test --project=desktop-browser --workers=1 \
  desktop-folder-picker.spec.ts desktop-api-script-worker.spec.ts \
  desktop-git-worktree-status.spec.ts desktop-vault-performance.spec.ts \
  desktop-performance-agents-workflows.spec.ts desktop-api-history-performance.spec.ts \
  desktop-workflow-progress-auth.spec.ts desktop-transcript-cache-live.spec.ts
```

Earlier focused browser runs used a private wrapper around the same setup with the global sweep removed and a baseline test daemon. They validate current frontend behavior and mocked routes; the final 13-case combined run used the newly built daemon and passed. Three additional cases use real routes to verify workflow detail/version access, History cursor scoping, cache-hit authorization after role/membership changes, and first-open live transcript delivery. All three passed separately against the same isolated build.
The existing test daemon startup also performs model-catalog discovery and reads host usage/transcript metadata into its temporary database. Test scenarios launch no real agent turns and write no production state; startup is not described as fully offline.

## Delivery scope

Deliver one PR, wait for all required checks, then merge to main using the user's admin approval. The user's subsequent instruction explicitly authorizes rebuilding, reinstalling and replacing the running app only after that merge. Local development and browser verification use the isolated worktree and temporary daemon state.
