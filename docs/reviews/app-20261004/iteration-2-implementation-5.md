# Iteration 2 implementation — partition 5

Addressed R2-5-01 (remaining C5-02 execution ownership) from `iteration-2-partition-5.md`. The pre-fix defect was confirmed by tracing the production tool caller, update handler, stored execution ID and exact-ID Stop lookup. No observed pre-fix failing test is claimed. This role did not run builds, tests, servers, commits or live-service operations.

The tool handler now passes its already-resolved caller thread into `agent_update`. Its production mutation helper (`crates/otto-server/src/assistant/tasks.rs`, `apply_agent_update`) first loads the task under the caller's user identity and verifies the exact owning thread. A foreign thread, a threadless caller targeting a thread task, or another user cannot update or rebind it. An explicit running update requires a current execution for a thread-bound task. It commits that execution ID together with the running state, result and cleared needs-you payload in one SQL update through `AssistantRepo::set_task_state_with_execution`. Task locking continues to serialize action/update operations, and the existing terminal-state conditional still rejects revival.

The Stop implementation remains unchanged: an untouched E1 task cannot signal a replacement E2. Only an explicit running update from the authorized owning thread moves its binding to E2. User-scoped tasks without a thread retain their existing user-scoped behavior. Public JSON shape is unchanged; API documentation and the TypeScript task-thread comment describe the ownership rule.

Added regressions in `crates/otto-server/src/assistant/threads.rs`:

- `explicit_task_resume_rebinds_stop_to_the_new_execution`: creates a real SQLite task bound to E1, ends E1, claims E2, confirms stale E1 Stop does not signal E2, invokes the production task-update mutation, verifies persisted E2 binding and cleared question, and requires Stop to signal/finish the E2 control before canceled settlement.
- `foreign_thread_or_owner_cannot_rebind_an_agent_task`: rejects another thread, an absent caller thread, another user and a same-thread update after its driver ends; persisted ownership stays unchanged and live controls remain unsignaled.

These tests use migrated in-memory SQLite and actual in-memory turn cancellation controls. They do not launch a provider process or exercise the full HTTP/auth/session-resolution stack. Existing tool identity tests and the original replacement-turn regression remain applicable.

Rustfmt succeeded on the four changed Rust files. Root reported 1,968 seven-package library tests passing before this follow-up; that result does not certify this change. Central execution requested:

```sh
cargo test -p otto-server --lib assistant::
cargo test -p otto-state --lib assistant::
```

Root also owns the final clippy/integration gates. Production source and tests are frozen and ready for execution and independent re-review.

## Closing browser follow-up — live metadata released Assistant history

The rendered r4 Assistant incremental-turn regression exposed a production lifecycle defect introduced by the acquire/release change. `assistant_turn` merges a fresh thread row before appending the turn. ChatView's acquire effect read `thread.id` directly, so replacing that prop object invalidated the effect despite an unchanged ID. Cleanup evicted the just-updated history; reacquisition fetched the older fixture snapshot, losing the live turn. Waiting for initial history did not resolve this.

Root recorded an observed pre-fix failure in `/tmp/otto-review-assistant-acquire-red.log`: history GET #1 returned at 1745 ms, the initial “Three good fits” assertion ran at 1918 ms, the live event/text assertion at 1940 ms, and the unexpected GET #2 returned at 1945 ms. “Research started.” never remained visible. This is runtime evidence, distinct from the source-only ownership finding above.

The minimal fix derives the primitive thread ID before the acquire effect. Metadata changes for that same ID now preserve the acquisition; changing threads still releases it. No store or WebSocket behavior was changed. Root strengthened the existing rendered browser test with a history GET counter and asserts that each live update changes the visible text without refetching history. Central post-fix browser/UI verification is pending; no local test command was run by this role.

## Closing Rooms fixture correction — tail and older-page contract

Claude reported the old r3 automation test failing repeatedly because “Evidence note 201” never appeared. Source review established a stale fixture, not a production paging defect. The authoritative contract at `docs/contracts/api.md:4298` specifies that `tail=true` without a cursor returns the newest `limit` messages, in chronological order; `before` pages older history, while `after` takes precedence. `personalAgentsApi.messagesBefore` supplies `tail=true&limit=200` for cold loads. The route selects `AgentRoomsRepo::list_messages_before`, whose SQL reads rowid descending with LIMIT and reverses the page. The store deliberately returns after loading that tail, and RoomsView offers “Show earlier messages.”

The old mock ignored `tail` and returned the first 200 of 201 messages, then required an automatic after-cursor request. That expectation contradicted both the API and the bounded rendering behavior. Only the affected fixture/test was changed: it now honors tail, before, after precedence and limit; asserts note 201 visible, note 1 absent, exactly 200 initial rows and the exact tail request; then clicks Show earlier and asserts 201 rows, note 1 visible and the actual before cursor equal to the first tail message. Its name now describes opening the latest 200 messages and paging earlier evidence. Other route mocks and production sources were unchanged. Central browser execution is pending; this role ran no test or server.


### Central Rooms fixture execution

The corrected newest-200/earlier-page case passed three consecutive runs in 4.2 seconds and passed again in the combined post-main browser group. Logs: `/tmp/otto-review-rooms-repeat.log`, `/tmp/otto-review-merged-browser.log`. Production Rooms sources were unchanged.
