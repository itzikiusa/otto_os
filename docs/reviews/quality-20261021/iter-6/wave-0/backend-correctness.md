# Backend correctness — quality iteration 6, wave 0 (2026-10-23)

Lens: logic bugs in the Rust daemon crates (`crates/**`): wrong conditions,
off-by-one errors, broken state transitions, unhandled edge cases, wrong error
mapping, and panics that input can reach. The review was read-only: no cargo,
npm or tests were run. Every finding below was confirmed by reading the code
at the cited lines.

## Score

Start at 10.0. Open findings: 2 major (-0.3 each), 2 minor (-0.1 each), 1 nit (-0.03).

| id | severity | deduction |
|----|----------|-----------|
| backend-correctness-6-01 | major | -0.30 |
| backend-correctness-6-02 | major | -0.30 |
| backend-correctness-6-03 | minor | -0.10 |
| backend-correctness-6-04 | minor | -0.10 |
| backend-correctness-6-05 | nit | -0.03 |

**10.0 - 2 x 0.3 - 2 x 0.1 - 1 x 0.03 = 9.17 / 10**

## Previous findings (iteration 5)

All five are fixed. None was invalid.

| id | status | evidence in current code |
|----|--------|--------------------------|
| 5-01 (deeply nested Mongo statement overflowed the stack) | **fixed** | `otto-dbviewer/src/drivers/mongo_parse.rs:49` `MAX_DEPTH = 128`, enforced by the nested-parse guard at `:75-85`. A test at `:568-571` covers exactly `MAX_DEPTH` and `MAX_DEPTH + 1`. |
| 5-02 (Confluence `body_html` nesting aborted the daemon; `</#root>` panicked) | **fixed** | `otto-issues/src/storage_check.rs:110` `MAX_DEPTH = 256`, checked before each push (`:193-199`). `:161` refuses a close tag when only `#root` is left, before popping. Tests at `:554-559`. |
| 5-03 (workflow expressions recursed without a bound) | **fixed** | `otto-core/src/expr.rs:386` `MAX_DEPTH = 64`, applied through `nested()` on `!`, `-`, `(`, call arguments and index (`:572-635`). `MAX_BINARY_OPS = 1024` also bounds left-associative chains. |
| 5-04 (Slack admitted events it would drop) | **fixed** | `otto-channels/src/slack.rs:1093` `drop_reason` is the one predicate. It runs before admission at `:899` and again in `handle_event` at `:1144`. Test at `:1626`. |
| 5-05 (`mcp-server --profile` with a stray character ran unrestricted) | **fixed** | `ottod/src/mcp_server.rs:302-315` `connection_profiles` parses with `parse_ids` (trimmed) and returns `Err` on a bad id. The process exits instead of running unfiltered. |

## Areas covered this iteration

- **Re-verified** the five iteration-5 findings above.
- **Iteration-5 fix code (commit `9a0639c6d`), read as new code:**
  - `otto-brokers` `replay.rs` (`Interrupted` drop guard, disarm before the final write, checkpoint cadence) and the route's own-task spawn in `http.rs`;
  - `otto-server/src/budget/mod.rs` (`read_config` / `try_config` fail-closed admission, `spend_unavailable`);
  - `otto-automation` `scheduled_tasks_engine.rs` (`SavedReport`, report-write failure becomes the run status, `pause_for_deleted_workflow`) and the `KeyedLocks` swap in `goal_loop.rs`;
  - `otto-git/src/collections.rs` (symlink refusal, `write_via_rename`, all-or-nothing validation);
  - `modules/review/retry.rs` `ClaimGuard` (restore on an error or a drop, disarm before spawn) and `summarize.rs` (`fallback_cause`, kept-comments read now failing the run);
  - `game_rooms` registry and types (shared `Capability` / `Digest` / `TokenBucket`, the catalog JSON, the `RATE_LIMITED` match in `socket.rs`).
- **Areas iteration 5 listed as not covered:**
  - `otto-product/src/run.rs`: `run_analysis` fan-out and summarizer, `retry_analysis_agent_owned`, and the `set_agent_status` merge semantics in `otto-state/src/product.rs:1155`. Also `chat.rs`: `send_message_inner`, `apply_action`, `split_actions` (finding 6-01), `canvas_doc_from_action` and `mermaid_from_nodes`. And `refine.rs` `parse_turn` (finding 6-04).
  - `otto-sessions/src/ws.rs` flow control: `CreditGate` (`take`, `push`, `reopen`, `ack`, `skip_on_input`, `forgive`, `track_stall`) and the `credit` / `ack` frame handlers. `room_authority.rs` in full, plus the manager's `begin` / `set` / `fence` / `end` room-authority wrappers.
  - `otto-aws/src/s3.rs`: normalizers, `is_texty` / `preview_kind`, `preview` / `ranged_get` / `RangedMeta::total_size`, `download` with `capped_child_stream` (the body is short against `Content-Length`, so a truncated transfer is visible), and the validators.
  - `otto-k8s/src/actions.rs` `plan` (every action), `restart_step` and `manual_job_name`.
  - `otto-server/src/state_archive.rs` `validate_archive`, `apply_rows` (user remap, data-root path rewrite, conflict signature), `plan_files`, and `state_archive/files.rs` in full (`validate_files`, `relative`, `existing`, `publish_using`, `remove_matching`, `design_blob_files`, `walk`).
  - `otto-dbviewer` Mongo executor beyond parsing: `keyset_filter` / `sort_is_id_asc` / `last_row_id` (finding 6-03), the `Find` branch's limit, probe and skip logic, and `json_to_bson` / `decode_ejson` with every sentinel decoder.
  - `otto-issues/src/confluence.rs` `storage_to_markdown`: the main loop's list, pre and blockquote state, `convert_table`, `convert_ac_macro` (finding 6-02) and `convert_ac_image`.
  - Rooms (non-game) under `otto-server/src/rooms/`: `registry.rs` (create, invite, join, authenticate, admit, connect / disconnect, `withdraw_media`, `end`), `actions.rs` (finding 6-05), and `socket.rs` (`run`, `maintain`, `disconnect`, `sync_authority`).
  - `otto-git` `refs_with_base` → `integration_bases` / `merged_sets_cached` (per-base memo key) and `resolve_cleanup_base` / `default_branch`.
- **Sweeps:**
  - `x.len() - 1` and `[..len - 1]` in non-test files changed since `389beaa9c` (405 files). Every site is guarded by a length check or a non-empty precondition. Examples: `review/retry.rs:422` (`agents.len() < 2`), `feature_guard.rs:546`, `split.rs:142`, `clickhouse.rs:2232`.
  - Every caller of `otto_core::text::extract_json` (the first-JSON-span helper behind iteration 4's finding 4-04). Four production callers still take the first span with no shape predicate: findings 6-01 and 6-04.

## Areas NOT covered yet (next iteration)

- `otto-server/src/policy.rs` past the exempt block, and the `mcp_outward.rs` route map in full.
- `otto-sessions/src/http.rs` (other than the MCP-profile PATCH) and the rest of `manager.rs`: `ensure_live`, respawn and `claim_input_authority` / `may_resize`.
- `otto-product/src/run.rs` `run_generate_plan`, `run_generate_tests` and `run_rewrite`; and `reap_orphaned_agents_on_startup`.
- `otto-server/src/workflow_engine.rs`: the iteration-5 `execute_node` split and `review_run` beyond the goals evaluation.
- `rooms/recap/*` (archive, media, consent), `rooms/terminal.rs`, `presentation.rs` and `annotations.rs`.
- `otto-vault` `engine.rs` / `parse.rs` link rewriting (`md_link` splice).
- `otto-apiclient` REST path extraction (`rest.rs` around `:860-900`) and the gRPC changes from iteration 5.

## Findings

### backend-correctness-6-01 — major — Discovery Chat drops everything after the first ```` ```json ```` fence and loses the proposed actions when an example block comes first

- **Where:** `crates/otto-product/src/chat.rs:660-686` (`split_actions`), called from `send_message_inner` at `:307`. The result is persisted at `:309-318` as the agent message body, with `meta_json: None`.
- **Evidence (prose truncation, independent of actions):**
  - `let cut = raw.find("```json").or_else(...).unwrap_or(raw.len()); let mut markdown = raw[..cut].trim().to_string();` cuts at the FIRST ```` ```json ```` fence, whether or not that fence is the actions block. It even cuts when `actions_json` is `None`; the `or_else` only matters when there is no fence at all.
  - A discovery reply that shows a payload example works like this: "The webhook body looks like ```` ```json {"event":"paid"} ``` ```` … then these edge cases: … ```` ```json {"actions":[…]} ``` ````. Everything from the example onwards (the edge cases and the actions block) is cut from the stored body.
  - The cut is the persisted `discovery_chat_messages.body`, and `meta_json` is `None`, so the UI never has the dropped text. This is a product-discovery chat, where JSON payload examples are routine.
  - Without a fence, the `or_else` cuts at the first `{` anywhere once an actions object was found. Prose such as "use the `{orderId}` path parameter" loses everything after it.
- **Evidence (lost actions):**
  - `otto_core::text::extract_json(raw)` (`otto-core/src/text.rs:142-167`) accepts the first ```` ```json ```` fence whose body parses (`accept` is `|_| true`), then the first balanced span that parses.
  - In the example above, the fence is `{"event":"paid"}`. It parses, is accepted, and `.get("actions")` is `None`. The real actions block is never considered, and the person gets no Apply buttons.
  - This is the iteration-4 finding 4-04 pattern ("callers took the first JSON span"), which was fixed for swarm and product run but not here.
- **Fix:**
  - Find the actions with `extract_json_where(raw, |v| v.get("actions").is_some_and(|a| a.as_array().is_some_and(|a| !a.is_empty())))`.
  - Remove only that block from the prose: locate the matching fence (or the balanced span) that produced it, and splice it out of `raw`. Never cut at the first ```` ```json ```` or `{`. Keep every other byte of the reply, including the example fences.
  - Add a test with an example ```` ```json ```` fence before the actions fence, and prose after both. Assert that the actions are found and that the example and the trailing prose survive.

### backend-correctness-6-02 — major — `storage_to_markdown` recurses once per nested panel macro, so a Confluence page with deeply nested info/note panels aborts the daemon

- **Where:** `crates/otto-issues/src/confluence.rs:1043-1061` (`convert_ac_macro`, the `"info" | "note" | "warning" | "tip"` arm calls `storage_to_markdown(body)` at `:1054`). Callers:
  - `storage_to_markdown_async` (`:791-801`), which runs inline on a tokio worker below 64 KiB (`OFF_RUNTIME_CONVERT_BYTES`, `:786`) and on a blocking-pool thread above it, both with 2 MiB stacks;
  - `otto-issues/src/http.rs:569` (page read, which also serves the MCP `otto_get_confluence_page` tool);
  - `otto-product/src/service.rs:186` (importing a Confluence-backed story).
- **Evidence:**
  - Iteration 5 recorded this function as "iterative, so no recursion risk". The panel arm is recursive.
  - For input `(<ac:structured-macro ac:name="info"><ac:rich-text-body>)×N x (</ac:rich-text-body></ac:structured-macro>)×N`:
    - `macro_src` ends at the FIRST `</ac:structured-macro>` (`:982`).
    - `extract_body_tag` (`:991-998`) takes the text from the first `<ac:rich-text-body` open to the first `</ac:rich-text-body>` close. That text holds N-1 nested macro opens.
    - `storage_to_markdown(body)` then repeats with N-1. The recursion depth is N, with no bound.
  - Each level also lowercases its whole remaining body twice (`input_lc` and `extract_body_tag`'s `src_lc`), so the work is quadratic in N.
  - Each level is about 70 bytes of storage, so a page of a few hundred KB nests thousands of levels deep. A stack overflow is not a panic: Rust aborts ottod (SIGABRT), taking down every session, the HTTP API and all background engines until launchd restarts it.
  - The trigger is reading a page. Anyone with edit rights on any Confluence page the user (or an agent through MCP, or the story watcher) opens can plant it, for example through the REST API with storage-format XHTML. `storage_check::parse`'s `MAX_DEPTH` (iteration 5's 5-02 fix) only guards the WRITE path.
- **Fix:**
  - Pass a depth into the conversion: make the recursive call a private `storage_to_markdown_depth(body, depth + 1)`, and past a small bound (for example 16 levels of nested panels) emit the panel body as plain text (`strip_tags` + `decode_entities`) without recursing.
  - Bound total work as well: lowercase `body` once per level, or reuse the caller's `input_lc` slice instead of re-lowercasing in `extract_body_tag`.
  - Add a test with 5,000 nested `info` macros that expects a string back, not an abort.

### backend-correctness-6-03 — minor — Mongo keyset paging silently ends the walk at the first `_id` type change, so collections with mixed `_id` types lose rows

- **Where:** `crates/otto-dbviewer/src/drivers/mongodb.rs:3046-3068` (`keyset_filter`), `:3088-3091` (`last_row_id`), and the `Find` branch at `:1059-1118`, which offers `next_cursor` only while `r.truncated`.
- **Evidence:**
  - An eligible find (no explicit limit, sort absent or `{_id: 1}`, no `_id` in the filter) pages with `{$and: [<filter>, {_id: {$gt: <last _id>}}]}`, sorted `{_id: 1}`.
  - MongoDB comparison operators use type bracketing: `{_id: {$gt: "abc"}}` matches only `_id` values of the same BSON type (numbers compare across numeric types only).
  - A collection with mixed `_id` types is a normal legacy shape. For example, string ids from an import and ObjectIds from the app; `{_id: 1}` sorts numbers < strings < objects < ObjectId < bool < date.
  - Page 1 ends inside the strings. Page 2's `$gt "<last string>"` returns only the remaining strings, and the page that exhausts them comes back short.
  - So `truncated` is false, `next_cursor` is `None`, and the pager stops. Every ObjectId-keyed document is never shown, with no error and no "more rows" hint.
  - The offset path the same query used before keyset paging (`skip`) did not have this gap. Nothing in the file handles type order: no `$type` or bracketing logic exists in `mongodb.rs`.
- **Fix:**
  - When building the cursor filter, add the later types: `{$or: [{_id: {$gt: last}}, {_id: {$type: [<every BSON type that sorts after last's type>]}}]}`, using the canonical sort order. MinKey, Null, numbers, symbol/string, object, binData, objectId, bool, date, timestamp, regex and MaxKey are the types an `_id` can take.
  - The simpler alternative: only enable keyset when the cursor's type is ObjectId, and fall back to `skip` otherwise.
  - Add a driver test that seeds string and ObjectId ids across a page boundary.

### backend-correctness-6-04 — minor — four agent-reply parsers still take the first parseable JSON span; the workflow `review_run` goals check then silently ignores the goals

- **Pattern:** `otto_core::text::extract_json` (`otto-core/src/text.rs:142`) accepts the first parseable ```` ```json ```` fence, else the first parseable balanced `{…}`. A `{}` or a quoted config snippet in a preamble shadows the object the caller wants. Iteration 4's finding 4-04 moved swarm and product-run callers to `extract_json_where` with a shape predicate. These callers were left:
  - `crates/otto-server/src/workflow_engine/nodes/review_run.rs:433-444`: the goals evaluation. The first span is taken; a missing `score` becomes `unwrap_or(review_score)` (`:438`) and missing `goals` becomes `[]`.
  - `crates/otto-product/src/refine.rs:572-593` (`parse_turn`): a first span without `reply` falls back to the raw text and DROPS `updated_story_md`, so the refinement's story update is lost.
  - `crates/otto-canvas/src/assist.rs:900`: a first span without `elements` or `nodes` makes the assist fall through as if no diagram came back.
  - `crates/otto-product/src/chat.rs:662`: the actions half of finding 6-01 (counted there, listed here so the class is fixed in one pass).
- **Evidence (the gate case):**
  - The goals prompt asks for `{"goals":[…],"score":N}`. A reply that first quotes, for example, `{"name":"svc","version":"1.0.0"}` from `package.json` while explaining an unmet goal yields `v` without `score`.
  - The goals score then equals the review score, and `score = (review_score + gs) / 2` (`:454`) equals the review score. Unmet goals cannot lower the run's score, so the threshold gate passes as if no goals were configured, and the node's output shows `goals: []` instead of "not parseable".
- **Fix:**
  - Use `extract_json_where` with the expected shape in each caller:
    - `v.get("score").is_some()` or `v.get("goals").is_some_and(Value::is_array)` for review_run;
    - `v.get("reply").is_some_and(Value::is_string)` for refine;
    - non-empty `elements` or `nodes` for canvas assist.
  - In review_run, treat "no object with `score`" like the not-parseable branch, recording a note, instead of defaulting to the review score.
  - Add one test per caller with a decoy object before the real one.

### backend-correctness-6-05 — nit — `RoomAction::Audio` indexes `members[&self.host]`, which panics if the host entry was pruned in the sub-second window before `maintain` ends the room

- **Where:** `crates/otto-server/src/rooms/actions.rs:138`. The prune happens at `crates/otto-server/src/rooms/registry.rs:242`.
- **Evidence:**
  - `join` (the public `/room-join` route) runs `room.members.retain(|_, m| m.valid())`. A host disconnected for at least `GRACE` (30 s) is no longer `valid()`, so the host's entry is removed.
  - `maintain` (`socket.rs:201-234`) ends such a room only on its next 1-second tick.
  - In that window, a connected admitted guest sending `Audio { joined: true }` reaches `!self.members[&self.host].view.audio_joined`. `HashMap` `Index` panics on a missing key, and the guest's room socket task dies mid-action.
  - Every other host lookup copes with a missing host. For example, `maintain` uses `.get(&room.host).is_none_or(…)`.
- **Fix:**
  - Use `self.members.get(&self.host).is_none_or(|h| !h.view.audio_joined)`, which refuses when the host is gone.
  - Optionally, never prune the host in `join`'s `retain` and leave ending the room to `maintain`.
