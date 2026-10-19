# Iteration 08 — root L21–L24: incremental execution policy

Reviewed the service-to-driver LOCAL INFILE boundary with the MySQL owner and an
independent ClickHouse reviewer. Preflight SQL parsing cannot predict later mode
changes. The service now conveys authenticated root host-file authority for the
whole request; the driver checks the actual incremental statement and grants
only its exact filename once. Saved/internal flags are cleared before resolving
current authority. Non-root requests retain runtime refusal even when preflight
cannot identify a later statement. Dedicated root connections trade setup cost
for correct same-batch state, without enabling the shared pool capability.

| Lens | Score | Evidence |
|---|---:|---|
| L21 routing | 9.6 | Stateful backend scanning owns SQL mode; UI does not guess server state. |
| L22 dialog | 9.8 | Four current browser cases remain green. |
| L23 contracts | 9.7 | Documented root connection cost and exact-file behavior; no public payload changes. |
| L24 release | 9.2 | Two updated policy tests and mixed-mode LOCAL live case passed per MySQL owner; final aggregate gate pending. |

No broader restriction on privileged host access was added. Independent review
found no inherited/cross-user flag path. Native read-only enforcement is retained.
