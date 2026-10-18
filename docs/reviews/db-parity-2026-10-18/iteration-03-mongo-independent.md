# Independent Mongo review — iteration 03

Read-only review of the Mongo worker's current implementation. No Mongo source changes, engine mutation, or Cargo commands were performed by this reviewer during this pass. Findings were handed to the owning worker before scoring.

| Lens | Score at reviewed snapshot | Evidence |
|---|---:|---|
| L01 statement/fallback coverage | 9.2 | Native parse failures fall back before execution, so failed writes are never retried. JSON/translated SQL/explain errors remain explicit. A multiline or comment-prefixed single getSiblingDB expression still takes raw script mode and loses its return value (`mongodb.rs:585` at review time). |
| L02 options/explain/SQL semantics | 9.5 | Strict chain/arity and JSON-field validation prevent ignored options; unsupported SQL features fail clearly. Explain bypasses the caller's timeout/comment path (`mongodb.rs:1008`, `:3637`) so a configured deadline is not applied to planning. |
| L03 BSON/results/export fidelity | 9.4 | Typed sparse JSON exports preserve per-document columns and rare sentinels. Extreme BSON dates still serialize to an invalid Extended JSON string (`mongodb.rs:3213`), breaking roundtrip/export. |
| L04 resource/cancellation/isolation | 8.7 | Cursor/result/output caps and shell kill-on-drop are sound on traced paths. The cancel tag derives solely from client query_id (`mongodb.rs:2867`) while registry identity is separately scoped by user/connection (`service.rs:2270`). Two users/connections with the same query_id on one Mongo server share a tag; allUsers currentOp matches both, so cancelling one can kill the other. |

## Confirmed findings

1. **Major: cancellation crosses execution boundaries.** A user-supplied `query_id="x"` becomes `comment="otto:x"`. Two distinct service registry entries can use that same request id. `current_op_pipeline` matches only the shared comment and `cancel` kills every returned opid. The service's user check protects entry lookup but does not rewrite the request passed to the driver. Fix: generate a fresh unpredictable execution tag and record it in `CancelToken`; leave the client id solely for service lookup. Test two concurrent queries with the same client id and cancel only one.
2. **Major: formatted shell expressions lose their output.** `db.getSiblingDB('db')\n.collection.find({})` satisfies script detection, but the expression branch additionally requires no newline; a leading comment also defeats `starts_with("db.")`. The raw `--file` body has no print, so the result is discarded. Preserve expression evaluation independently of whitespace/comments; continue to leave actual scripts unchanged.
3. **Minor: extreme BSON dates cannot roundtrip.** `DateTime::from_millis(i64::MAX)` cannot render RFC3339. `bson_to_json_typed` falls back to `dt.to_string()`, whose BSON implementation emits the numeric millisecond value as text when its time conversion fails. Result: `{"$date":"9223372036854775807"}`. `decode_date` accepts RFC3339/date-only strings, not numeric strings, and fails. Use canonical `{"$date":{"$numberLong":"..."}}` on the fallback path.
4. **Minor: explain ignores the configured deadline and cancellation tag.** `run_one` computes timeout, then returns `explain_plan` before applying it or the comment. `mongo_explain_value` sends only explain and verbosity. Pass timeout/comment through the explain command or enforce the caller's wall clock.

The owner is implementing the reported changes; scores intentionally describe the inspected snapshot, not unverified future fixes. No 9.8 claim is made from a static review with these gaps still present.

## Fix re-review

The owner applied all four corrections during this round. Re-read confirms fresh `otto_core::new_id()` execution tags, a conservative property/call-chain expression recognizer across comments/newlines, canonical Extended JSON fallback for extreme dates, and timeout/comment fields passed into explain commands. No new blocker emerged from this focused re-review. Conditional post-fix scores are **L01 9.8, L02 9.8, L03 9.8, L04 9.8**, contingent on the owner's targeted regression/live gate passing. This reviewer did not execute those pending tests and does not substitute source inspection for their result.
