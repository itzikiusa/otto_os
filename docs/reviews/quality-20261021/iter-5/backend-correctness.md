# Backend correctness — quality iteration 5 (2026-10-22)

Lens: logic bugs in the Rust daemon crates (`crates/**`): wrong conditions,
off-by-one errors, broken state transitions, unhandled edge cases, wrong error
mapping, and panics that input can reach. The review was read-only. No cargo,
npm or tests were run. Every finding was confirmed by reading the code at the
cited lines.

## Score

Start at 10.0. Open findings: 2 major (-0.3 each) and 3 minor (-0.1 each).

- backend-correctness-5-01: major, -0.3
- backend-correctness-5-02: major, -0.3
- backend-correctness-5-03: minor, -0.1
- backend-correctness-5-04: minor, -0.1
- backend-correctness-5-05: minor, -0.1

**10.0 - 2 x 0.3 - 3 x 0.1 = 9.10 / 10**

The drop from 9.74 comes from going deeper, not from regressions in fixed
code. Two of the findings sit in code merged after iteration 4: the Jira and
Confluence Markdown pipeline, and MCP profiles. Two more sit in parsers that
older passes never fed adversarial nesting. The last is in the order of the
Slack admission checks.

## Previous findings (iteration 4)

All four are fixed. None was invalid.

| id | status | evidence in current code |
|----|--------|--------------------------|
| 4-01 (undoing a memory forget re-activated a memory that was already inactive) | **fixed** | Migration `0192_memory_prev_active.sql` adds `memories.prev_active`. `otto-state/src/memory.rs:916` saves `prev_active = CASE WHEN forgotten_at IS NULL THEN active ELSE prev_active END`. `:963` restores `active = COALESCE(prev_active, …)`. |
| 4-02 (an SSH tunnel through an existing ControlMaster never became ready) | **fixed** | `otto-ssh/src/lib.rs:386-393`: `base_args` passes `ControlMaster=no` and `ControlPath=none`. `:576` asserts both. |
| 4-03 (ClickHouse batch panicked on non-object `params`) | **fixed** | `otto-dbviewer/src/config.rs:103-115` turns non-object `params` into `{}`. Regression at `:171`. |
| 4-04 (swarm and product callers took the first JSON span) | **fixed** | Each caller now uses `extract_json_where` with a shape predicate: `verify.rs:44-59` (`is_verdict`), `run.rs:131` (`is_turn_result`), `recruiter.rs:115-121` (`is_plan`), and `otto-product/src/swarm.rs:703` (`parse_planner_tasks`). |

## Areas covered this iteration

- **Re-verified** the four iteration-4 findings above.
- **Code merged since the iteration-4 reports (`b0bbe22b8..HEAD`, about 8,300 lines), read as new code:**
  - `otto-issues`: `markdown.rs` (block and inline parser, `DEPTH_MAX` / `NEST_MAX` bounds, closer memo, slicing at every byte index), `md_adf.rs`, `adf_check.rs`, `storage_check.rs` (finding 5-02), `md_storage.rs`, `issue_create.rs`, `jira/create.rs` (createmeta paging, type, priority, epic and parent checks, required fields, read-back `compare`), `confluence/verify.rs`, and `http.rs` `resolve_body` / `create_issue`.
  - MCP profiles:
    - `otto-state/src/mcp_profiles.rs` (exactly-one-default transactions, locked Full, `resolve`);
    - `otto-mcp/src/profiles/{mod,routes}.rs` (`canonical`, `ProfileDef::allows`, union semantics, `normalize_fields`, `fields_from`);
    - `ottod/src/mcp_tools/{profile,creds}.rs` and the `tools/list` / `tools/call` hooks;
    - `ottod/src/mcp_server.rs` (finding 5-05);
    - `otto-sessions` `MCP_PROFILES_META` PATCH validation and `otto_tools_env.rs`;
    - `mcp_http.rs` `?profile=` and `mcp_outward.rs` `otto_server_enabled`.
  - `otto-mcp/src/outward/exec.rs`: the `issue_route` split, including `create_issue` argument mapping. Also `auto_approve.rs` `validate_target`, which was moved.
- **Areas iteration 4 listed as not covered:**
  - `workflow_engine.rs`:
    - `spawn_run` (scope persistence, park-until-permit-or-cancel, the Pending re-read, definition pinning);
    - `resume_queued_runs` and `reconcile_interrupted_runs`;
    - `classify_resume` (approval re-entry, mid-flight kinds, the between-nodes topo resume, Finish);
    - `otto-state/src/workflows.rs` `prepare_retry`, `prepare_resume`, `queued_run_ids` and `interrupted_runs`.

    The Running transition always writes initialized node states, so the `'[]'` queue key cannot collide with a resumed row. No defect found.
  - `otto-usage/src/engine.rs`: `totals_window_day`, `remember_totals`, `session_totals_for_budget` (fail-closed flush, parked last-known gated on day and generation), `session_totals_background`, and `FlushSync`.
  - `otto-channels`:
    - `slack.rs`: dedup window, `decode_slack_text`, `is_human_edit` / `dedup_ts`, the `events_api` admission branch (finding 5-04), and `handle_event`;
    - `admission.rs` and `bridge.rs` `reserve_ingress` / `handle`;
    - `telegram.rs`: `run`, `dispatch_prefix`, `notice_disposition` and `handle_update`.
  - `otto-dbviewer` MongoDB:
    - `mongo_parse.rs` in full (finding 5-01);
    - `parse_command`, `parse_json_command` and the shorthand argument split;
    - `statement_is_read_only` and `types.rs` `mongo_is_write` / `looks_like_mongosh_script`.
  - `otto-server/src/state_archive/schema.rs`: `scrub_string`, `has_credentials` and `scrub_json`.
  - `otto-server/src/game_rooms/registry.rs`: create, join and authenticate, `maintain` / `depart`, `apply` (rate buckets, epochs, packet sequence, rematch), and slot semantics against the UI's slot lookups.
  - `otto-core/src/expr.rs`: parser recursion (finding 5-03).
- **Sweeps:**
  - `unwrap()` / `expect()` / `[0]` / `len() - 1` in non-test code added since iteration 4. Every site is guarded, except `storage_check.rs:159` (finding 5-02).
  - Hand-written recursive-descent parsers in `crates/**`, checked for a depth bound. `otto-issues/src/markdown.rs` has one; `mongo_parse.rs`, `storage_check.rs` (tree drop and `outline`) and `expr.rs` do not.

## Areas NOT covered yet (next iteration)

- `otto-product/src/run.rs` `run_analysis` fan-out and summarizer, `run_generate_plan`, and `chat.rs` action parsing (still open from iteration 4).
- `otto-server/src/policy.rs` past the exempt block, and the `mcp_outward.rs` route map in full (only the issue routes were read).
- `otto-sessions/src/ws.rs` flow-control credits (`Credits::take`, `track_stall`) and `http.rs`, other than the MCP-profile PATCH.
- `otto-aws/src/s3.rs` and the `otto-k8s` actions.
- `otto-server/src/state_archive` import: `insert_row`, `files.rs` `publish_using` / `validate_files`.
- `otto-dbviewer` MongoDB executor beyond parsing: aggregation paging, `json_to_bson` / `decode_ejson`, and the mongosh script runner.
- `otto-issues/src/confluence.rs` `storage_to_markdown` (iterative, so no recursion risk; its list and table state machine was not traced).
- Rooms (non-game) media and classroom code under `otto-server/src/rooms/`.

## Findings

### backend-correctness-5-01 — major — a deeply nested Mongo statement overflows the stack and aborts the daemon

- **Where:** `crates/otto-dbviewer/src/drivers/mongo_parse.rs:113-195`. `Parser::value` → `object` / `array` / `constructor` → `value` recurses with no depth limit.
- **How input reaches it:**
  - `mongodb.rs:2467` (`parse_json_command`);
  - `:2944` / `:2949` (`parse_doc_arg` / `parse_pipeline_arg`, from `parse_shorthand`);
  - the read-only guard `mongodb.rs:2444` `statement_is_read_only` → `parse_command`, called by `types.rs:1549` (`mongo_is_write`).
- **Evidence:**
  - `value()` dispatches `{` → `object()` and `[` → `array()`. Each of those calls `self.value()` for every member and keeps no depth counter. The only other bound is the input itself. The axum default body limit is 2 MB, and the dbviewer routes add no statement cap: `grep DefaultBodyLimit` finds none for them.
  - Example: `db.c.find(` + `[` × 200,000 + `]` × 200,000 + `)`. This is a single line, so `looks_like_mongosh_script` is false and the shorthand split only counts depth with an `i32`. The input is about 400 KB and needs about 400k nested frames.
  - The default tokio worker stack is 2 MiB, and ottod sets no `thread_stack_size`. A stack overflow is not a panic: Rust aborts the whole process (SIGABRT). Every session's websocket, the HTTP API and all background engines go down until launchd restarts the daemon.
  - The guard runs the same parser before execution. So even a read-only MCP `otto_db_query` from an agent session, or a classification-only path, crashes the daemon. No query reaches MongoDB first.
- **Fix:**
  - Add `depth: usize` to `Parser`. Increment it on entry to `object` / `array` / `constructor` and decrement on exit. Refuse past a fixed bound (for example 128, serde_json's default) with `err("nesting is too deep")`.
  - Add a test that feeds 10,000 nested `[` and expects `Err`, not a crash.

### backend-correctness-5-02 — major — Confluence `body_html` with deep nesting aborts the daemon; `</#root>` panics the request

- **Where:**
  - `crates/otto-issues/src/storage_check.rs:105-192` (`parse`);
  - `:253-255` (`check_well_formed`);
  - `:418-473` (`storage_outline` / `outline`);
  - `:483-490` (`storage_plain_text` / `collect_text`);
  - reachable from `crates/otto-issues/src/http.rs:547-556` (`resolve_body`, used by page create `:605`, page update `:669` and comment `:717`, and by the MCP `create_confluence_page`, `update_confluence_page` and `comment_confluence_page` tools).
- **Evidence (abort):**
  - `parse` is iterative, but the tree it builds is as deep as the input. Each `Node` owns `children: Vec<Node>`.
  - `check_well_formed` is `parse(xhtml).map(|_| ())`. On well-formed input (`<b>` × 100,000 then `</b>` × 100,000, about 700 KB, under the 2 MB body limit), dropping the root `Node` runs recursive drop glue 100,000 levels deep.
  - The verification read-back also walks the tree recursively with no bound: `outline` and `collect_text` recurse into each child.
  - As in 5-01, a stack overflow aborts ottod. `resolve_body` runs this on any `body_html` a user with an authorized issue account, or an agent through MCP, sends.
- **Evidence (panic):**
  - A close tag's name is never validated (`:148-160`).
  - For a top-level `</#root>`, `stack.pop()` removes the synthetic `#root` frame. Its name equals `#root`, so the mismatch check passes, and `stack.last_mut().unwrap()` (`:159`) panics on the now-empty stack.
  - The panic unwinds: the request's connection drops with no response instead of a 400.
- **Also affected:** the read-back verification. `body_checks` (`confluence/verify.rs:104-117`) runs `storage_outline` / `storage_plain_text` on the stored body, so a page that already nests that deeply in Confluence crashes the verification. The depth bound below covers this too: an over-deep body becomes `["unparseable"]`.
- **Fix:**
  - Keep a depth counter in `parse`: `stack.len()` is the depth. Refuse past a bound such as 256 with "the body nests too deeply". This bounds every later recursive walk and the drop.
  - In the close-tag branch, refuse when `stack.len() == 1` ("closes nothing") before popping, or check the name with `is_name_start` / `is_name_char` like open tags.
  - Add tests for `</#root>` and for 10,000 levels of nesting.

### backend-correctness-5-03 — minor — workflow expressions recurse without a bound, so a long `!!!…` or `((((…` condition aborts the daemon

- **Where:**
  - `crates/otto-core/src/expr.rs:509-561`: `parse_unary` recurses once per `!` or `-`, and `parse_primary` → `parse_expr` once per `(`;
  - `eval_ast` (`:601`) recurses the same depth;
  - called from `otto-workflows/src/validation.rs:47` (edge conditions) and `:142` (node expression params) on every workflow save, and from `workflow_engine.rs:3824` at run time.
- **Evidence:**
  - `fn parse_unary` is `Some(Tok::Bang) => { self.pos += 1; Ok(Ast::Unary('!', Box::new(self.parse_unary()?))) }`, with no depth argument and no counter.
  - A branch condition of 200,000 `!` followed by `true` lexes fine. It then needs 200,000 nested frames on a 2 MiB tokio stack, and the overflow aborts the daemon during `validate`, before the workflow is even stored. Dropping the boxed `Ast` chain recurses as well.
  - It takes Editor rights on a workspace's workflows, which is why this is minor and not major.
- **Fix:**
  - Add `depth: usize` to `Parser`. Increment it in `parse_unary` (per prefix operator) and in `parse_primary` (on `(` and on a call's arguments). Return `ExprError::Parse("expression nests too deeply")` past 64.
  - Optionally cap `src.len()` (for example 4 KB) in `lex`.

### backend-correctness-5-04 — minor — Slack admits events it will drop (bot posts and edits, membership, deletions), so the bot's own feed edits fill a thread's queue and bounce the next human message as "busy"

- **Where:**
  - `crates/otto-channels/src/slack.rs:854-944` (the `events_api` branch: admission, `reserve_ingress`, spawn);
  - the drop filters only run later in `handle_event` (`:1074-1124`);
  - `crates/otto-channels/src/bridge.rs:1005-1008` (`handle` awaits the session's turn lock while the ingress ticket is held);
  - `admission.rs:7-9` (`PER_CONVERSATION = 8`, `PER_INTEGRATION = 64`).
- **Evidence:**
  - Every `events_api` envelope that is not a dedup repeat calls `bridge.admission.admit_chat(…)`. That includes `reaction_*`, bot messages, `message_deleted`, `channel_join` and non-human `message_changed` events. On `Work`, the branch takes a `WorkPermit` and an ingress ticket on the `(chat, thread)` queue, then spawns `_ordered = q.acquire().await; handle_event(…)`.
  - Only inside `handle_event` are bot messages (`bot_id`), deletions, membership notices and unfurls dropped. By then the permit has been held the whole time the task waited in the queue.
  - A human follow-up sent during a running turn holds its ingress ticket inside `Bridge::handle` (`turn.acquire_or_notify(...)`) until that turn ends.
  - Meanwhile the mirror posts and edits its "working…" feed in the same thread. `handle_event`'s own comment says "each edit emits a message_changed event authored by the bot". Each such event takes a permit for that conversation and queues behind the follow-up.
  - After about 6 bot events, `counts.conversations[key]` reaches 8. The person's next message in the thread then gets `Admission::Busy`, and they are told "Otto is busy … This message was not started", although only one real message is waiting.
  - The `Busy` path filters its notice less strictly than `handle_event`: it checks only `bot_id` and type. So an admitted user's `channel_join` (which Slack refuses to thread onto) or an unfurl is answered with a busy notice instead of being skipped.
- **Fix:**
  - Run the same cheap drop predicate (type `message` / `app_mention`; no `bot_id` at either level; not `message_deleted` or a membership subtype; not a non-human `message_changed`) in the `events_api` branch before `admit_chat`. Ack and insert into `seen` without admission.
  - Factor the predicate out of `handle_event` so the two cannot drift.

### backend-correctness-5-05 — minor — `ottod mcp-server --profile` with a space or other stray character silently runs unrestricted

- **Where:** `crates/ottod/src/mcp_server.rs:249-256` (profile parsing in `run`). Compare `crates/ottod/src/mcp_tools/profile.rs:21-35` (the session bridge, which sanitizes each id) and `otto-mcp/src/profiles/mod.rs:416-424` `parse_ids` (the HTTP path, which trims each id).
- **Evidence:**
  - The stdio connection keeps the value only if `p.chars().all(|c| c.is_ascii_alphanumeric() || "-_,".contains(c))`. Anything else (`--profile "database, issues"`, a trailing newline inside the value, a typo with `.`) makes the whole value `None`.
  - `profiles: None` means no `?profiles=` on the enable-list read, so `filter_in` returns `None`, and both `tools_list` and `tools_call` apply no filter. Nothing is logged.
  - The person asked for a narrowed connection ("Otto · Database") and gets every enabled tool without being told. The same list over HTTP (`?profile=database,%20issues`) resolves both profiles, because `parse_ids` trims.
- **Fix:**
  - Parse with `otto_mcp::profiles::parse_ids`, so each id is trimmed and blanks are dropped.
  - Fail closed when a requested value has invalid characters: exit with an error naming the bad id, or at least `eprintln!` and use a deny-all filter. Never fall back to "no profile".
  - Add a test for `"database, issues"`.
