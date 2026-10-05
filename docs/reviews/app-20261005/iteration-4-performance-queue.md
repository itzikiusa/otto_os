# Iteration 4: serial performance acceptance queue

Prepared 2026-10-05 from `../app-20261004/PERFORMANCE.md`, `ui/scripts/loadtest/{README.md,loadtest.mjs,analyze.mjs}`, Playwright configuration/setup, and the existing targeted specs. **Commands below were prepared, not executed.** Source checkpoint supplied by the coordinator: `dea63948`; main integration is pending. No performance score changes or current CPU/RAM results are implied.

## Execution conditions

- Wait for the coordinator's affected gate and all other heavy jobs to finish. Run one measurement at a time, with no source edits/HMR updates during it. Do not run this queue concurrently with Cargo, another browser suite, or Claude's measurement.
- Use a daemon built from the exact source being accepted. A successful check/test gate does not establish that `target/debug/ottod` was rebuilt. The coordinator must establish binary provenance first; stop if it is unknown. This queue intentionally contains no build command and never falls back to the installed app binary.
- Keep headed Chromium, the default WebGL path, and the existing safety limits: load average 12, free memory floor 2 GiB, swap-growth ceiling 500 MiB. A safety abort/deadline is an incomplete run, not a pass. Do not raise thresholds to get a result. Prior headless runs hit SwiftShader CPU pressure, so mixing them with headed samples would not be a valid comparison.
- Use unused loopback ports 7814/5314. Inspect with `lsof` below; if occupied, coordinate a different pair throughout, never terminate the owner. Never use production port 7700.
- Save raw samples, analysis, page errors, driver log, exit status, and cleanup outcome. The harness creates isolated HOME/data/provider fixtures and owns teardown. Do not use broad process-kill or orphan-sweep commands against other agents' processes.

## 1. Setup and read-only live-app sample (about 2 minutes)

In the worktree shell, after the machine is quiet:

```sh
cd /Users/itziklavon/claude_ade-review
export PERF_RUN_ROOT="$(mktemp -d /tmp/otto-review05-perf.XXXXXX)"
export OTTO_E2E_BIN="$PWD/target/debug/ottod"
export OTTO_LOADTEST_UI_DIR="$PWD/ui"
export OTTO_E2E_PORT=7814
test -x "$OTTO_E2E_BIN"
git rev-parse HEAD > "$PERF_RUN_ROOT/source-head.txt"
git status --short > "$PERF_RUN_ROOT/source-status.txt"
shasum -a 256 "$OTTO_E2E_BIN" ui/package-lock.json > "$PERF_RUN_ROOT/input-sha256.txt"
lsof -nP -iTCP:7814 -sTCP:LISTEN
lsof -nP -iTCP:5314 -sTCP:LISTEN
```

For `lsof`, no rows/exit 1 means no listener was found; this is a manual preflight, not a command sequence to paste under unconditional `set -e`. A binary hash identifies the file, not its source revision; keep the build provenance with these artifacts.

The following bounded sampler records only process identity, `ps` CPU snapshots and RSS; it never reads sessions, database rows, transcript content, command-line arguments, environment variables, or application APIs. Run before the isolated daemon exists. Do not interact with or mutate real sessions to generate load. Record the installed app revision separately if already known; these measurements describe that running app, not automatically the reviewed source.

```sh
python3 - <<'PY'
import datetime, json, os, pathlib, subprocess, time
dest = pathlib.Path(os.environ['PERF_RUN_ROOT']) / 'real-app-process-samples.jsonl'
with dest.open('w') as out:
    for sample in range(25):
        rows = []
        raw = subprocess.check_output(['ps', '-axo', 'pid=,ppid=,%cpu=,rss=,comm='], text=True)
        for line in raw.splitlines():
            fields = line.strip().split(None, 4)
            if len(fields) != 5:
                continue
            pid, ppid, cpu, rss, executable = fields
            name = pathlib.Path(executable).name
            if name in {'otto-desktop', 'ottod'}:
                rows.append({'pid': int(pid), 'ppid': int(ppid), 'name': name,
                             'ps_cpu_percent': float(cpu), 'rss_kib': int(rss)})
        out.write(json.dumps({'sample': sample, 'utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                              'processes': rows}) + '\n')
        out.flush()
        if sample < 24:
            time.sleep(5)
print(dest)
PY
```

This reproduces the prior 25-sample/120-second observation shape. `ps %cpu` is a process CPU snapshot, not the isolated harness's interval-delta measurement. It excludes native WebKit/GPU helpers and is not total native-app RSS. Empty matches mean no qualifying running process, not zero resource usage. No workload throughput or content-scale conclusion follows from this sample.

## 2. Serve the frozen UI once

In one separately owned foreground terminal, start Vite from the same worktree. Keep it for both load runs and the optional targeted browser queue, then stop this terminal with Ctrl-C. The UI server is not another measured application workload.

```sh
cd /Users/itziklavon/claude_ade-review/ui
node --input-type=module -e 'import { createServer } from "vite"; const server = await createServer({ server: { host: "127.0.0.1", port: 5314, strictPort: true, hmr: false } }); await server.listen(); server.printUrls();'
```

## 3. Current-source N=0/1/3/5 scale run (normally 6–7 minutes; 10-minute deadline)

In the original shell, with the setup variables still present:

```sh
cd /Users/itziklavon/claude_ade-review/ui
node scripts/loadtest/loadtest.mjs --mode scale --steps 1,3,5 \
  --hold 80 --baseline 20 --recover 40 --sample 5 --stacks false \
  --headed true --deadline 600 --max-load 12 --min-free-gb 2 \
  --max-swap-growth-mb 500 --ui-url http://127.0.0.1:5314 \
  --out "$PERF_RUN_ROOT/scale"
node scripts/loadtest/analyze.mjs "$PERF_RUN_ROOT/scale" > "$PERF_RUN_ROOT/scale-analysis.txt"
```

Run the analyzer only after recording the harness exit status; analyzing partial output cannot turn an abort into a pass. N=0 is the harness baseline/recovery, so do not add zero to `--steps`. At each N=1/3/5 the scale mode visits tiled, focused, Home, and Git states for the 80-second hold; the harness also performs its existing profile sampling. Prior matching scale execution took 375.4 seconds. Compare per-state interval CPU, renderer/daemon/agent RSS, JS heap, DOM count, long tasks, jank, WS bytes and request counts. Preserve browser/daemon process groups separately and page errors/leftovers; do not sum overlapping group totals.

## 4. Fifteen-minute sustained run (normally 16–18 minutes; 20-minute deadline)

Only after scale has completed and its owned processes have exited:

```sh
node scripts/loadtest/loadtest.mjs --mode leak --n 3 --leak-min 15 \
  --state-s 20 --baseline 20 --recover 40 --sample 5 --stacks false \
  --headed true --deadline 1200 --max-load 12 --min-free-gb 2 \
  --max-swap-growth-mb 500 --ui-url http://127.0.0.1:5314 \
  --out "$PERF_RUN_ROOT/sustained"
node scripts/loadtest/analyze.mjs "$PERF_RUN_ROOT/sustained" > "$PERF_RUN_ROOT/sustained-analysis.txt"
```

This uses existing tiled/Home/Git/DB/focus cycles, with the harness's Home/forced-GC checkpoints. Prior matching execution took 986.8 seconds. Report first/last checkpoint and time slope for JS heap, renderer RSS and DOM counts, plus post-recovery values. Forced-GC heap is not process RSS. Fifteen minutes can expose growth; it cannot establish the absence of a slow leak. Existing session load does not certify imported transcript, large diff, workbench revision, or room recap content costs.

## 5. Existing targeted regressions (serial; roughly 3–10 minutes, failures may take longer)

These are separate acceptance evidence, not a substitute for content-workload CPU/RAM measurements. Skip rerunning focused unit tests if the current affected gate already ran the exact files on unchanged source. The short standalone command is:

```sh
node --test unit/sharedDiffBounds.test.ts unit/productTranscriptLoading.test.ts \
  unit/productTranscriptLifecycle.test.ts unit/asyncFindProvider.test.ts \
  unit/workbenchHistoryPaging.test.ts unit/historyPagination.test.ts \
  unit/recapPolling4.test.ts
```

For existing mounted-browser regressions, use the isolated daemon, one worker, synthetic transcript roots, file-backed secrets and no orphan sweep. `CI=` allows local reuse of the explicitly started Vite server. Keep the budget scale at 1; do not relax budgets for a busy host.

```sh
export OTTO_E2E_SLOT=review05perf
export OTTO_E2E_PW_PORT=5314
export OTTO_E2E_UI=http://127.0.0.1:5314
export OTTO_E2E_SWEEP_ORPHANS=0
export OTTO_SECRETS=file
export OTTO_TRANSCRIPT_ROOTS="$PERF_RUN_ROOT/empty-transcript-roots"
mkdir -p "$OTTO_TRANSCRIPT_ROOTS"
CI= OTTO_PERF_BUDGET_SCALE=1 npx playwright test \
  e2e/desktop-review4-workbench-history.spec.ts \
  e2e/desktop-history-pagination.spec.ts \
  e2e/desktop-performance-agents-workflows.spec.ts \
  e2e/desktop-api-history-performance.spec.ts \
  e2e/desktop-conversation-perf.spec.ts \
  e2e/desktop-diffviewer-huge-perf.spec.ts \
  --project=desktop-browser --workers=1 --retries=0 \
  --output="$PERF_RUN_ROOT/e2e-chromium"
```

Do not include `desktop-conversation-load-perf.spec.ts`: it requires an `OTTO_CONV_FIXTURE` from real transcript content and is outside this queue's data authorization. A skip without that variable is not load coverage. Do not run `room-recap.spec.ts` blindly under `desktop-browser`: it is not a desktop-prefixed suite matched by that project and includes separate synthetic media/protocol scenarios, not a sustained recap-poll performance measurement.

| Repaired workload | Existing evidence available in this queue | Remaining measurement gap |
|---|---|---|
| Shared `DiffView` large comparison | `sharedDiffBounds.test.ts`: bounded algorithm/rendering, reconstruction and repeated-line regressions | Mounted shared comparison: equal/one-edit/rewritten 10k+ lines, 500-row navigation and complete download; CPU, peak heap/RSS, long tasks. Git `desktop-diffviewer-huge-perf` exercises a different `DiffViewer` component (40k rewrite and huge single line), so is adjacent evidence only. |
| Product imported transcript | Loading/lifecycle/find unit tests exercise summary paging, lazy bounded body cache, fifth panel, oldest reveal and async cancellation with controlled transport; prior HTTP route tests remain separate daemon evidence | Actual Product mounted browser plus real isolated SQLite with 50/100/1,000 bodies, 256 KiB bodies and one huge body; summary bytes, five-panel retained heap, search/cancel CPU/RSS and time to first result. Session conversation spec's 300 synthetic turns and 70 KiB lazy tool output do not measure this path. |
| Workbench history | Mounted 305-revision route fixture checks 100-row page bounds, oldest selection, comparison and restore; current Rust scale test separately covers real 50k revisions | Actual 50k-revision browser workload, payload/heap over repeated page traversals and body comparison cost. Fixture correctness does not prove SQL latency. |
| History/API history/workflow summaries | Mounted controlled-route cases check cursor continuation, lazy detail and late-result guards | Real storage/index/content scaling and sustained memory, distinct from route fixtures. |
| Recap revision polling | `recapPolling4.test.ts` checks unchanged-revision behavior and lifecycle; archived-body backend regressions are separate | An isolated long recap with 15 unchanged polls: revision-query count, zero repeated body bytes/reads, changed-revision refresh, CPU/RSS over time. No existing listed load mode drives this workload. |

Additional existing adjacent specs (`desktop-docs-orch-perf`, `desktop-design-hall-scale-perf`) cover bounded document renderers, Vault graph idle polling, Design Hall metadata/index latency and large saves. They do not add shared-diff/Product/recap coverage; omit from the minimum queue unless that adjacent area changed again. A separate serial `desktop-webkit` run of the already compatible conversation performance spec can add browser-engine evidence, but still does not measure native Tauri/WebKit process cost.

## Reporting and duration

Minimum resource queue: approximately 25–28 minutes including the read-only sample and setup, before optional targeted browser regressions; budget roughly 30–40 minutes with them, excluding build/gate time. Deadlines bound the two load runs, not every possible test failure. Stop on safety failures and reconcile ownership/cleanup before another run.

Acceptance report must record exact source/binary provenance, mode, process groups, sample counts, exit/abort status, workload sizes, page errors, leftover processes and sustained slopes. Compare like-for-like prior headed measurements; hardware/foreground activity and browser-engine differences remain limitations. The queue provides current concurrent-session resource evidence and existing focused regression evidence. The missing content workloads above remain explicitly unmeasured until the coordinator runs suitable existing/manual isolated scenarios; no new benchmark framework or claim of measured improvement is introduced here.
