# Coordinator findings and verification

Baseline `7cff9e538`, clean before this campaign. Source and checks are coordinated with the named independent reports. This file records additional findings and historical checkpoints, not a whole-app score. [Final exact-source validation](VALIDATION.md) supersedes pending handoff notes below.

## Goal Loop settings rejection persisted a partial write

Confirmed at baseline `crates/otto-server/src/routes/goal_loops.rs:304–344`: PATCH writes `name` before validating limits/config, then writes limits before validating config. Sending a new name with `max_iterations: 0` returns an error but persists the name. A draft config patch also bypasses the creation-time nonempty-executor requirement. The regression invokes the actual router with an isolated migrated SQLite database: expected `Original`, received `Must not persist` (see `evidence/goal-patch-red.log`).

Repair validates the entire candidate first and persists optional fields in a single SQL UPDATE. The shared validation now rejects zero executors for both creation and updates. The same regression exercises invalid limits, mode changes and empty executors, an injected storage failure on config update, and a successful combined update. API contract and TypeScript request documentation describe atomic rejection. The actual router regression passed; all four Goal Loop integration cases also passed.

## Proof deep-link loading raced workspace initialization

The UI reviewer reproduced the existing real-daemon reload case: selected pack A is visible, reload ends at `Proof packs` with no detail. Source trace: the route effect starts `proof.open(id)` before workspace initialization; `loadPacks` later changes scope and invalidates the detail request; the route itself does not change and auto-selection explicitly skips a deep link.

Repair makes the route effect depend on both workspace readiness and the Proof store's matching scope. Original reload regression and an independently added delayed-workspace case pass in the 12-case browser run. The delayed case asserts zero detail requests before workspace resolution, exactly one afterward, correct pack and retained URL. This is real UI behavior against isolated data, not a source-string assertion.

## Browser guard regression found during independent iteration 2

The initial bounded guard implementation rejected an already-vetted public request after 64 permits, before spawned workers could run. Independent production-handler/real-pipe test sends 128 cached requests and reproduced 64 Continue / 64 Fail responses. The correction handles cached-positive GET/HEAD/OPTIONS inline, preserving the existing origin key/TTL and session route. Uncached, denied, expired and outward methods retain guarded admission and approval behavior; the proxy independently vets each TCP connection. No admission limit was increased. Both coordinated cached-burst positive/negative tests passed after the refinement; final whole-workspace verification remains pending.

## Integration checkpoint

- Baseline UI units: 1,823 passed on Node 26.10.0.
- Post-repair UI units: 1,823 passed; UI guards, Svelte and all TypeScript checks passed.
- Fresh UI production build passed via native reviewer; bundle budgets passed without updates.
- UI real-browser named checks: 12/12 passed; shared earlier baseline group also 12/12. These selections overlap, so counts are not added as unique coverage.
- Browser package initial repaired snapshot: 125 unit + 3 integration tests and Clippy passed. Independent burst refinement passed its two positive/negative tests; final consumer verification remains pending.
- Evaluator initial repairs: 26 tests passed including malformed output/proof, origin attribution and concurrent rating/retry publication. Independent iteration 3 confirmed and repaired initial scoring's analogous publication race, incomplete multi-pass success and stale winner selection; its seven output/publication and six scoring tests passed.
- Native lifecycle/visibility probes passed; strict AX tree acceptance could not run successfully under the current process permissions. Native School passed three hidden/resume cycles but an earlier modal resume failure remains unexplained. The original detached-frame assertion was insufficient; iteration 5 strengthens it and will record a separate result. No installed app or live user profile was modified.

## Final independent finding: retry badges omitted failed validators

Iteration 5 traced the retry callback's legacy aggregate through the persisted iteration to the UI badge. It filtered to `status=done`, so one clean validator and one error produced 100 rather than 50. The new regression reached the actual guarded publisher with a mixed-state matrix and failed with `left:100.0, right:50.0` (`evidence/final/retry-badge-red.log`). A first test compile attempt incorrectly treated the numeric score as optional; that test-only type error was corrected before the meaningful red run.

The aggregate is now published under the same update guard as the proof/composite, with every configured validator in the denominator and incomplete states contributing zero. The old unguarded callback write is removed. All seven output/publication regressions pass, including the persisted badge and post-publication review signal. The independent reviewer accepted the repair at source level; final integration is recorded separately.
