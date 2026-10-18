# Database parity and session consistency review

Requested outcome: one reviewed PR, merge to main after required checks pass,
then rebuild, reinstall and replace the running app. Advisory checks do not
block release. The user authorized implementation and release without questions.

Starting commit: `191824b35d695c3593bae06b31c20f86a5403ee0`.
Starting working changes: Mongo distinct support and stable New Session sizing,
already verified before this wider review. Preserve these changes.

## Method

Each iteration reviews all 24 lenses, fixes evidenced defects, and records
verification. Up to 10 iterations; early completion requires every lens to reach
the 9.8 target with evidence. A score is an assessment of the reviewed surface,
not proof of universal compatibility. Unverified behavior and known functional
limitations must remain explicit and lower the relevant score. Do not manufacture
scores to meet the target.

Root and privileged users should retain broad legitimate capabilities. Fix real
authorization errors; do not add blanket restrictions to ordinary native database
commands, cross-session messaging, or authorized workflow folders. Saved read-only
settings and actual cross-user access boundaries remain meaningful.

## Lens ownership

| ID | Lens | Initial owner |
|---|---|---|
| L01 | Mongo native method coverage | Mongo worker |
| L02 | Mongo parser and BSON semantic fidelity | Mongo worker |
| L03 | Mongosh fallback and full shell coverage | Mongo worker |
| L04 | Mongo result, timeout and cancellation behavior | Mongo worker |
| L05 | MySQL native statement coverage | MySQL worker |
| L06 | MySQL batches, procedures and multiple results | MySQL worker |
| L07 | MySQL value and metadata fidelity | MySQL worker |
| L08 | MySQL session state, timeout and cancellation | MySQL worker |
| L09 | ClickHouse statement coverage | ClickHouse worker |
| L10 | ClickHouse transport, FORMAT and SETTINGS parity | ClickHouse worker |
| L11 | ClickHouse values, metadata and errors | ClickHouse worker |
| L12 | ClickHouse limits, state and cancellation | ClickHouse worker |
| L13 | Redis command and module coverage | Ronaldo (agy) |
| L14 | Redis argument and batch parsing | Ronaldo (agy) |
| L15 | Redis response types and errors | Ronaldo (agy) |
| L16 | Redis connection state and blocking/cancellation | Ronaldo (agy) |
| L17 | Privileged cross-session messaging | Pele (grok) |
| L18 | Workflow folders outside the workspace | Pele (grok) |
| L19 | Database capability and execution policy usability | Pele (grok) |
| L20 | Authorization compatibility without blanket restrictions | Pele (grok) |
| L21 | Query editor routing and execution ergonomics | Root |
| L22 | Session dialog layout and accessibility | Root |
| L23 | Contracts, discoverability and error guidance | Root |
| L24 | Regression coverage and release compatibility | Root |

## Execution plan

- [x] Iteration 01: establish actual support, reproduce gaps, implement fixes.
- [x] Subsequent iterations: independent review of fixes, regression and native
      engine validation; repeat all lenses, up to iteration 10.
- [x] Run scoped repository gate, relevant named browser tests, and native engine
      integration tests when locally available. Coordinate Cargo commands.
- [x] Consolidate supported native/fallback behavior and limitations in docs.
- [ ] Create one PR; wait for all required CI checks and address failures.
- [ ] Merge and push main, rebuild/install, verify replacement daemon/app.

Review reports in this directory are evidence records; source and tests remain
the authoritative behavior. Shared code edits are coordinated through root.

## Consolidated outcome

See [final verification](final-verification.md) for all 24 lens assessments and
exact evidence boundaries. All 5,806 Rust nextest cases passed (124 skipped), plus
the targeted live Mongo/MySQL/ClickHouse/Redis checks and four session-sizing
browser cases. Documentation, UI, plugin and filter checks also passed. CI/release steps are tracked separately; this is
not a claim that every lens reached 9.8 or every database command is supported.
