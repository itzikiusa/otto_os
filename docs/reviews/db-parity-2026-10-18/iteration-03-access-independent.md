# Iteration 03 — independent L17–L20 review

Reviewer: root. Read complete policy diff, new native_owner_tests, workflow route
and location helper, and added run_repo_scope tests; traced ui_bridge::is_human.
No Cargo run for this review; the coordinated integration gate remains pending.

| Lens | Score | Assessment |
|---|---:|---|
| L17 messaging | 9.8 | Policy behavior unchanged; corrected comments match same-user non-shell delivery. Actual authenticated messages to both named sibling sessions delivered in this task. No new blocking finding. |
| L18 workflow cwd | 9.5 | Human root exception is applied at manual route, not workflow owner or chat hook. Existing canonical-directory predicate preserves repo behavior and rejects missing paths/files/root. Error still incorrectly implies root has no host-folder exception; requested fix. Route test for root agent credential exclusion should accompany human/role distinction. |
| L19 DB execution | 9.6 | Resolved.profile_id derives from actual resolution; owner bypass requires logical connection equality, alternate profile/non-owner still inspect grants. New tests exercise both refusals and retained read-only marker. Await execution. |
| L20 auth compatibility | 9.7 | No changes to share-token root stripping, human-required administration, or cross-user messaging. Existing credential-class predicates reused. Await route regression execution. |

No additional blanket access restriction is recommended. Remaining gaps are
specific guidance/test evidence, not hypothetical privilege tightening.
