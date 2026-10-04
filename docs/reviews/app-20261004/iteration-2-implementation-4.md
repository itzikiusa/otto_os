# Iteration 2 implementation — Mission Control draft ownership

C4-R2-02 is repaired in source. `WorkItemDetail.svelte` previously reseeded the goal, result-summary and risk editor fields whenever a same-item GET completed. Approval actions invoke that GET while editing, and a read begun before Edit could also arrive after typing. The navigation guard did not protect either path.

The detail now keeps an explicit edit baseline independent of refreshed server data. Same-item reads update status, approvals and other persisted detail while retaining active draft fields and their baseline. Edit and Cancel explicitly seed from the current persisted detail. Read and view generations reject older reads, including A→B→A visits; success, error and loading writes share the same ownership check. Approval mutation callbacks capture their item/workspace/view, so they cannot refresh a later visit.

Save captures the submitted field snapshot. Its PATCH result updates persisted fields and the saved baseline; text entered after submission stays editable and dirty against that saved baseline. Reads begun before or during the PATCH are invalidated so a late response cannot undo the successful save. A failed Save retains the draft. Component destruction invalidates pending callbacks.

Only behavior was changed: script code and Edit/Cancel callback bindings. Existing visual markup, styles, ApprovalActions behavior and the restored `confirmer` import were preserved. No shared router or API contract changes were needed.

## Regression coverage

Nine cases were added to `ui/unit/orchestrationOwnership.test.ts`, extracting and executing the production loader, draft helpers, save and approval methods with deferred responses:

- Request approval, Approve and Deny refresh approval data while preserving all three edited fields and dirty state (three cases).
- A GET begun before Edit cannot overwrite later typing.
- A remote persisted-field update preserves the original edit baseline; Cancel/re-enter uses the latest persisted value.
- Successful Save wins over an older same-item GET released afterward.
- Typing after Save submission survives the PATCH and following GET; Cancel restores the submitted saved version.
- Failed Save retains fields, baseline and dirty state.
- A→B→A delayed reads cannot replace the newest visit; clean navigation seeds the next item's fields.

The existing selection/close dirty-guard test fixture now supplies the edit baseline and actual draft-comparison helpers. No test was executed by this implementer, and no observed red/green result is claimed.

## Verification handoff

`git diff --check -- ui/src/modules/mission-control/WorkItemDetail.svelte ui/unit/orchestrationOwnership.test.ts` passed. Root owns the central test slot and should run:

```sh
cd ui
node --test unit/orchestrationOwnership.test.ts
npm run check
```

Browser approval/edit interaction and light/dark/mobile rendering remain central verification. No builds, tests, servers, provider processes, commits or delegation were run. Source is stable and ready for central checks and the focused independent recheck.
