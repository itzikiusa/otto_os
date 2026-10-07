# Fix and validation plan

Baseline `196048df`; all changes stay on `fix/quality-20261007`.
The user delegated decisions and requested no questions. Repairs preserve the product's existing workflows, data ownership, and module boundaries. We use the existing shared primitives and store patterns instead of introducing a new UI framework or broad redesign.

## Decisions and ownership

1. **Telemetry contract and trace UI** — align endpoint span naming with a bounded, privacy-preserving ingress contract. Fix trace empty/error/retry states. Own `otto-server/src/routes/telemetry.rs`, telemetry contract docs/tests, `ui/src/modules/usage/OttoUsage.svelte`, and relevant telemetry tests. Prove real producer batches are accepted and malformed/raw identifiers still rejected.
2. **Telemetry sampling** — separate bounded sampling cadence from exporter waits; track only the owned collector's lifetime. Own `crates/otto-telemetry/`. Test a blocked flush, cancellation, disabling and process identity. Corroborate with isolated external resource samples.
3. **Scheduled recovery and report identity** — persist immutable admission context through an append-only nullable schema addition; use it at recovered completion. Legacy runs must not consume an unproven new schedule or deliver to an unproven destination. Include unique run IDs in report paths. Own automation/state scheduled-task code and migration/tests; coordinate any public type change. Test retime/restart, edited destination, same-instant distinct reports and pruning.
4. **Data editor safety** — refuse unsafe Redis list deletion unless an atomic exact-index operation fits the existing reviewed execution contract; never preserve the fixed-marker algorithm. Add environment leave guards using existing approval/draft patterns; guard create/selection/module exits without storing secrets in browser persistence. Own Redis edit and API environment components/tests.
5. **School interactions** — isolate stage shortcuts from native controls, remove invisible focus stops while retaining accessible actions, correct LoadState integration, preserve initial room restoration, and separate complete list membership from capped geometry. Own School model/widget/scene and matching tests. Validate overflow sessions, delayed loads, room remount, keyboard actions, no-WebGL, light/dark/phone/RTL. Also dispose cloned skeletons (shared geometry/materials remain owned by templates) and pause/abort screen polling while the document is hidden; validate reveal and teardown.
6. **Team Performance plugin** — preserve settings drafts and newer edits across refresh/save; make nested overlay keyboard ownership explicit; show stale scan status and provide cooperative Stop without discarding completed results. Own plugin UI/server and tests. Cancellation must reach pacing/backoff and stop new dispatch; it cannot merely stop polling. Existing active network requests may finish, with bounded cancellation documented. Fence account/scope requests against stale success/error, including scan callbacks. Fix worker-lifetime deployment-tag cache with bounded persisted results and compact earlier-tag digests; retain divergent-branch exclusion semantics and test separate worker processes.
7. **Archive candidate query** — bound hourly stale-session candidate enumeration, keeping existing live/attached race checks. Own state session query and session-manager sweep. Prove irrelevant archived payloads are not materialized, candidates are paged and races remain safe.

8. **Authentication boundary coverage** — exercise the real login handler with username/IP throttling and spoofed forwarding headers; do not duplicate its implementation in a test helper. Own focused server auth tests.

New-surface and test reviewers may add proven defects or refine these tasks before implementation. Each owner reads the current files and applicable skills before edits. Workers do not stage, commit, publish, build or run suites without coordination.

## Validation sequence

- [x] Write meaningful regressions and execute a failing baseline where practical; retain exact logs and test counts.
- [ ] Implement minimal repairs and pass focused regressions (original repairs verified; final All time plugin finding remains in progress).
- [ ] Independently check spec compliance, then code quality and neighboring branches.
- [x] Run `scripts/check.sh --base 196048df --check` with the same reduced-debug profile, Cargo jobs 2 and bounded nextest threads; browser workers 1 (broad Rust gates and subsequent UI continuation passed; later private-helper changes received focused checks).
- [ ] Run plugin unit/server/browser tests separately; they are outside `ui/`'s default check.
- [x] Run UI acceptance journeys, inspect actual light/dark/phone screenshots and evaluate keyboard/overflow/contrast (bounded browser matrix; packaged native zoom/VoiceOver unverified).
- [x] Run fresh telemetry off/on N=1/3/5 fixture-agent load, rendered-terminal load, and recovery samples, with no build concurrent. Attribute daemon/collector/ClickHouse/browser separately and preserve throughput (both full workloads passed; fresh-browser matched follow-up is in progress).
- [x] Sample real app read-only; distinguish short observation from controlled load and long-term leak evidence.
- [ ] Publish local evidence ledger and five final scores with explicit scope limits; iterate on unresolved material findings.

## Execution limits

Two active worker agents maximum. One heavy command at a time, owned by coordinator. No paid provider workloads, external publication, production mutation, unrelated process termination, shared cross-worktree Cargo target, or destructive git. All fixture processes and files must have demonstrable ownership before cleanup.

## Review refinements

Test reviewer requests: snapshot tests include legacy/malformed data and prior-reader rollback compatibility; keep a per-finding baseline-failure/fixed-pass ledger; identify which CI gate selects each key regression. Existing telemetry browser integration already catches C4 and failed freshly on this baseline. New tests complement it rather than claiming it never existed.

## Additional validation toward the remaining category targets

Performance and test quality reached a bounded independent 9.8; design, UX and correctness remain 9.6. The next checks address concrete remaining evidence rather than repeating passed suites:

- Extend the existing nonpersistent full-SPA Tauri probe to exercise actual WKWebView zoom in/out/reset, layout, focus and draft continuity. Build only isolated examples in this worktree's separate desktop target; never launch normal installed-app setup.
- Capture and review the real plugin report viewer, nested Download dialog, failed-scan Retry and stopping states in light/dark and narrow/wide layouts, using its existing isolated browser fixtures.
- Exercise scheduled workflow admission and recovery across actual disposable daemon process termination/restart. Edit the schedule and delivery destination between admission and crash; prove completion retains the admitted identity/destination and leaves the new schedule generation untouched. Use human approval as the waiting workflow boundary; no paid provider or external delivery.

The coordinator owns serial build/runtime execution. No global VoiceOver toggle, physical display action, installed-app state mutation or external publication is part of these checks. Native assistive-technology and hardware limits remain explicit unless directly observed.


## Execution closure

All listed repair work and bounded acceptance checks have executed. New native acceptance exposed a shared Modal focus dependency; the corrected mount-only lifecycle passes its browser breakpoint regression, existing nested return cases, and actual native zoom/draft acceptance. Actual scheduled workflow crash/recovery passed four process boots. Final new-source gates are recorded in `evidence/verification-log-manifest.json`; the individual independent reviews own the final scores. All work remains local to this single repair branch. No PR, deployment or production-data mutation was performed.
