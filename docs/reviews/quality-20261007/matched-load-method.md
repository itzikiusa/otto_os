# Matched N=5 telemetry comparison

The separate `ui/e 2e/desktop-telemetry-matched-perf.spec.ts` addresses the earlier shared-browser memory confound without changing the original raw benchmark. It runs collection on first, then off; each phase launches a new Chromium process with the same 1280×800 viewport, authenticated storage, workspace and Agents page. There is no preceding module walk. The fixture deliberately avoids Playwright's automatic browser/page fixture so an unused browser cannot enter process totals.

Each phase creates five owned fake-CLI sessions, opens five real terminal sockets, and runs five independent session-list request loops with the original 50 ms pause. The exact original raw input is 1081bytes every 250 ms, or 4324bytes/s/session. This differs from the separately capped rendered workload (~861 bytes/s/session); preserving the original raw payload is necessary for comparison. Positive actual binary output and per-session input are asserted. Resource rows retain contributing PIDs, platform CPU estimates and RSS; the load driver is excluded.

Defaults are 90 seconds of load and 60one-second recovery observations per mode. Sampling costs extend recovery beyond 60 wall-clock seconds; elapsed workload seconds and sample timestamps remain available. All sessions are deleted before recovery. Chromium heap size is observed before load, after load and after recovery through `Runtime.getHeapUsage`, without forced GC. The browser closes completely before the next mode. A partial artifact is written even if the test fails.

The same daemon and ClickHouse remain alive across both modes, so backend cache warming is not controlled. One counterbalanced pair is a bounded diagnostic comparison, not a statistical confidence interval or long-duration leak proof. A fresh-browser result can distinguish prior browser state from a reproducible telemetry-associated difference; it cannot independently prove which allocation caused any retained memory.

The coordinator's full type/style check passed before execution. The full run subsequently passed; see [matched results](telemetry-matched-load-results.md) and its retained raw artifact for final evidence.
