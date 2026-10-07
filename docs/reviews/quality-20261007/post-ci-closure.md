# Publication follow-up — PR 94

All repairs remain in [one PR](https://github.com/itzikiusa/otto_os/pull/94), based on main `196048df5`. The original five 9.8 scores apply to the bounded review recorded at 11:21 UTC. Broader publication checks uncovered additional defects and obsolete test assumptions; those scores are not retroactive evidence for these changes.

## Why publication needed another pass

The first PR run at `c78d239f5` had 58 functional failures, 1596 passes and 25 skips across four advisory shards. Main already had 58 functional failures, but the PR must still meet the user's green-check condition. No failure-count ceiling was raised, no assertion was disabled and no new skip was introduced. CodeQL also reported test-password literals and an erased CF-to-Objective-C pointer conversion in the native probe.

## Repairs and evidence

- Grid Enter could commit an edit and immediately reopen it when the same event reached the grid handler. The handler now respects the consumed event. Kubernetes monitor drafts now survive same-cluster metadata refreshes while loading the correct state when the selected cluster changes. Both regressions failed before repair and passed afterward.
- Compact result footers wrap their paging controls and edit guidance. First-time Explain reserves output space and replaces the idle placeholder until a result/error/loading state needs the shared pane. Smaller desktop light/dark regressions and the live MySQL Raw JSON/Close journey failed before the final repair and pass afterward.
- Enforced Mongo browsing retains only schema names/types from collection metadata, excluding sampled document values. Completion enriches only an authorized collection already present in its graph, with separately credential/policy-scoped field entries, 60-second TTL and 128-entry caps. Tests cover revocation, policy changes, refresh, unknown names, bounded eviction and existing SQL behavior. The actual bulk-schema path is exercised, not replaced by a permissive mock.
- Browser fixtures now use the current confirmation, draft-discard, shared-modal, tab and accessibility contracts. Assertions still inspect exact statements, persisted values, navigation destinations and error/refusal outcomes. Mongo workers own separate scratch collections; School checks target their own classroom.
- MCP fixtures obtain real managed-session tokens without storing them in shared configuration, preserve the shared tool catalog, and verify approval before execution. Approval still cannot bypass the workspace SSRF guard; the loopback listener receives no request and one agent-sourced failure is recorded. Token scope, bad-token and write-refusal checks remain intact.
- Team Performance tests follow Overview/People/Flow/Estimates and developer detail views, retaining exact person/ticket counts, phase data, predictions, evidence and saved-goal assertions. DORA checks remain unchanged.
- Auth fixtures generate per-test passwords. The native accessibility probe reads guarded CFStrings through public Core Foundation APIs into a bounded UTF-16 buffer, avoiding construction of an Objective-C reference from an erased pointer. Final static/native verification is tracked below.

## Executed checkpoints

| Check | Observed result |
| --- | --- |
| Entire database library, including new cache regressions |456 passed, 6 ignored|
| Rebuilt daemon |Passed; existing compact-unwind linker warning retained|
| Desktop database batch |42 passed: JSON edits, fat-document structures/rendering, index/DDL, Explain, compact paging and shortcuts|
| Live Mongo completion + MCP API + plugin/DORA |18 passed|
| Mobile database + MCP transport batch |26 passed after scroll repair; prior failures retained below|
| LocalStack account identity and permission chips |1 passed|
| UI type/style check |0 errors, 0 warnings|
| Earlier guarded-workflow/School batch |15 passed|
| Broad Rust gate |4,622 passed, 91 skipped; doc-tests passed. It then stopped at the service source-size limit.|
| Structural extraction |Both completion methods moved verbatim to a child module; fresh database 456/6, strict Clippy, workspace fmt and LOC ratchet passed.|
| UI/plugin continuation |Type/style checks, 1,698 UI tests, production build/bundle budget, 491 plugin passes and 1 intentional skip.|
| Native CFString change |Example build/strict Clippy passed; actual-SPA continuity/zoom passed; public AX name/value/action checks passed at 100% and 200%.|
| Grid keyboard and same-query rerun |2 passed; rerun regression failed before repair (scroll reset from 400 to zero)|
| Small phone and tablet scroll regressions |4 passed (iPhone SE and iPad portrait)|
| Final grid performance |100k × 30: layout p95 6 ms, painted p95 18 ms, jump 13 ms; 20k × 300: layout p95 4 ms, painted p95 11 ms, horizontal p95 13 ms; RTL p95 15 ms. All original budgets passed.|
| Final UI after grid repair |1,698 unit tests, production build, zero-error/warning type/style check and bundle budget passed.|
| Remote checks |Pending on the complete pushed batch|

The original scroll failures remain in the evidence. A diagnostic listener that read scroll position masked the problem; removing all diagnostic reads reproduced four failures in four cases. A setter trace without geometry reads recorded only the test's initial scroll write, followed by a native reset to zero. Disabling scroll anchoring, touch scrolling or containment did not fix it. Moving the virtual-window update from the scroll-event microtask to a coalesced animation frame fixed both Redis and ClickHouse. The tests issue one scroll and require the last data row to remain visible in the viewport. No layout read or second scroll was added to the production fix.

A separate reset effect accidentally tracked fresh column objects as well as their shape signature. Reading column objects untracked when applying a shape reset now preserves the offset on a same-shape rerun. The regression verifies that the second response's distinct row values are rendered before comparing the offset, so stale first-response rows cannot satisfy it.

The ClickHouse query helper now waits for the exact connection and statement response before inspecting rows; connection selectors use each fixture's own ID. The full mobile follow-up passed 26 cases, including Redis and ClickHouse read/write/edit/scroll, database import/export and MCP transport. Grid performance samplers now include the actual scheduled rendering work and require the target row to be mounted after each sample, while retaining existing cost budgets. One harness run exposed a missed RTL helper rename (three passed, one ReferenceError); the corrected RTL case passed afterward.

Evidence and inspected light/dark database screenshots are in [evidence/post-ci](evidence/post-ci/manifest.json); hashes distinguish original logs from trailing-whitespace normalization. Test fixtures use throwaway daemon state and owned database containers. One local heavy command runs at a time, Cargo jobs 2, test threads 2, browser worker 1. No production deployment or user-data mutation is included.
