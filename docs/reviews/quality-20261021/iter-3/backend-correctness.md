# Backend correctness — quality iteration 3 (2026-10-22)

Lens: logic bugs in the Rust daemon crates (`crates/**`): wrong conditions,
off-by-one errors, broken state transitions, unhandled edge cases, wrong error
mapping, and panics that input can reach. The review was read-only. No cargo,
npm or tests were run. Every finding below was confirmed by reading the code
at the cited lines.

## Score

Start 10.0. Open findings: 5 minor (-0.1 each), 1 nit (-0.03).

- backend-correctness-2-01 (still open, otto-sessions half): minor, -0.1
- backend-correctness-3-01: minor, -0.1
- backend-correctness-3-02: minor, -0.1
- backend-correctness-3-03: minor, -0.1
- backend-correctness-3-04: minor, -0.1
- backend-correctness-3-05: nit, -0.03

**10.0 - 5 x 0.1 - 0.03 = 9.47 / 10**

## Previous findings (iteration 2)

| id | status | evidence in current code |
|----|--------|--------------------------|
| backend-correctness-2-01 (huge "keep N days" panics) | **partly fixed, still open** | The API-client half is fixed. `otto-state/src/api_client.rs:899-919` `prune_history` builds the cutoff with `TimeDelta::try_days` and `checked_sub_signed`, and treats `None` as "no age limit". It has a test with `99_999_999` and `i64::MAX` (`:1136-1143`). The `otto-sessions` half is still open. `otto-sessions/src/manager.rs:5450` still reads `chrono::Utc::now() - chrono::Duration::days(days as i64)`. `days` comes from `v.as_u64()` on the free-form `session_auto_archive_days` setting, so a value above about 1.07e11 panics. A value of 2^63 or more wraps to a negative `i64`. For example, `u64::MAX as i64 == -1`, which puts the cutoff one day in the FUTURE, so every eligible agent session gets archived. FIXES.md P03 lists this half as "Not done". |
| backend-correctness-2-02 (k8s `?window=` panics) | **fixed** | `otto-k8s/src/monitor/queries.rs:196-226` splits before the last char with `char_indices().last()`. It uses `Duration::try_minutes`, `try_hours` and `try_days`, mapping `None` to the "at most 90d" error. |
| backend-correctness-2-03 (workgraph LIKE wildcards) | **fixed** | `otto-state/src/workgraph.rs:324` adds a shared `like_escape` that escapes the backslash first. `:756-764` uses it with `title LIKE ? ESCAPE '\\'`. |

No iteration-2 finding was rejected.

## Areas covered this iteration

- **Re-verified** the three iteration-2 findings above.
- **Code added or moved by the iteration-2 fix commit (52edfdbe7)**, read as new code:
  - `otto-channels/src/gfm.rs`, the GFM to Slack mrkdwn and Telegram HTML converter:
    - the fence and table scan;
    - `inline`, `find_close` and `parse_link` index arithmetic, which has no panics;
    - Telegram HTML escaping;
    - the plain-text fallback in `telegram.rs` `send_formatted`, including `decode` for HTTP 400 parse errors.
  - `otto-dbviewer/src/sql_class.rs`: the shared first-keyword scanner, the run-path and write-gate tables, and `ReadClass::of`.
  - `otto-server/src/modules/review/{lifecycle,retry}.rs`:
    - `transition` and `announce`;
    - the cancel registry and `sweep_review_temp_files_if_idle`;
    - `retry_review_agent`, `retry_summarizer` and `stop_review_agent` state transitions, checked against `ReviewsRepo::claim_agent_retry`, `try_begin_rerun` and `fail_running` (`otto-state/src/reviews.rs`).
  - `otto-server/src/modules/session_input.rs`: `wait_session` (the event-driven long-poll), `send_input`, `session_message`, `delay_session_input` and `open_agent_session`.
  - `otto-server/src/modules/pr_draft.rs`: `jira_key_from_branch`, `parse_pr_draft` and `parse_commit_draft`.
  - `otto-server/src/routes/history_page.rs` with `otto-state/src/history_page.rs`. The keyset cursor over the merged session and transcript-index candidates is consistent: ULID keys sort below `path:` keys in both SQL and Rust, and `scanned < total` drives `next_cursor`.
- **Areas iteration 2 listed as not covered:**
  - `otto-sessions/src/manager.rs`:
    - `create` (the spec, title, account and network-prepare rollback);
    - `adopt_holders`, `adopt_one`, `adopt_deferred` and `retry_deferred_adoptions`;
    - `suspend_idle_unattached` and `enforce_live_cap`.
  - `otto-sessions/src/ws.rs`: the `InputQueue`, `DeferredInput` and `spawn_input_queue` budget and permit accounting, coalescing and revoke. No permit leak or ordering bug was found.
  - `otto-dbviewer`:
    - `types.rs` `inject_row_limit` and `sql_is_write_for`;
    - the postgres driver's read and batch paths;
    - `service/changes.rs` `approved_attempt` and the fingerprint;
    - `multirun/params.rs` (placeholder scan, substitution and literal rendering).
  - `otto-vault`: `engine.rs` `rewrite_moved_links` and `parse.rs` `rewrite_links`, `md_path_span` and `split_frontmatter`.
  - `otto-automation`: `goal_loop_parse.rs`, and `otto-workflows` `retry.rs` and `checkpoint.rs`.
  - `otto-assistant`: `assistant/limits.rs`, `assistant/router.rs` (header and keywords) and `personal_agents_scheduler.rs` (`next_proactive_goal`).
  - `otto-git`: `transport.rs` (askpass origins, `forge_identity`) and `http.rs` branch delete and rename, `url_host` and worktree remove and prune.
  - `ottod/src/usage_tailer.rs`: `tail_claude_file`, `tail_codex_file`, `release_held` and `read_new_lines`.
  - `otto-rbac/src/tokens.rs`: `authenticate` slide and touch rules and `purge_expired`.
- **Sweeps:**
  - byte-index string slicing (`[..n]`, `truncate(n)`) in non-test code. Every remaining site slices ASCII-only data or a char-boundary-checked index.
  - every caller of `otto_core::text::extract_json` and `find_json_object` (finding 3-03).

## Areas NOT covered yet (next iteration)

- `otto-server/src/workflow_engine.rs` node executors other than `human_approval` and `review_run`: loop and foreach checkpoints end to end, and `resume_queued_runs` after a crash mid-loop.
- `otto-swarm/src/runtime/engine.rs`: the turn runner's JSON candidate selection (`:3270-3320`) and the verifier retry loop.
- `otto-product/src/run.rs` (11 changes in 3 weeks), `refine.rs` and `swarm.rs`.
- `otto-dbviewer`:
  - the clickhouse native and batch paths (`drivers/clickhouse.rs:1040-1060`, `:2880-2900`, `:3060-3075`);
  - the mongodb driver's mongosh script path;
  - the MySQL `run_script` loop with DELIMITER and session-state close.
- `otto-server/src/policy.rs` (38 changes in 3 weeks) and `mcp_outward.rs` / `ottod/src/mcp_tools.rs`, checked for logic rather than authorization.
- `otto-pty` holder protocol (`lib.rs`, `holder.rs`): the reconnect and snapshot replay framing.
- `otto-usage/src/engine.rs` budget evaluation and day rollover.
- `otto-channels/src/bridge.rs`, the message chunking that runs before `send_formatted` (a split mid-fence).

## Findings

### backend-correctness-3-01 — minor — a failed or interrupted per-agent review Retry leaves the agent row `pending` for good, and nothing can retry or stop it

- **Where:**
  - the retry handler: `crates/otto-server/src/modules/review/retry.rs:214-297` (`retry_review_agent`);
  - `crates/otto-state/src/reviews.rs:204-228` (`claim_agent_retry`) and `:124-136` (`try_begin_rerun`);
  - `crates/otto-server/src/boot/recovery.rs:107-120` (`fail_orphaned_reviews`).
- **Evidence:**
  - The handler CLAIMS the agent row first (`:236`). The claim persists the element as `status: "pending"`, `note: "retrying…"`, `session_id: None`.
  - Only after the claim does it fall into three early returns: `Ok(None)` from `get_diff`, which returns 409 "this review has no stored diff…" (`:282-286`); a `get_diff` DB error (`:287-290`); and an `fs::write` failure of the diff (`:294-297`). None of them restores the row.
  - `Ok(None)` is reachable. `materialize_review_diff` (`modules/review/run.rs:207-209`) logs and IGNORES a failed `set_diff`. The run's end-of-review sweep then removes the temp `.diff`, so `diff_len == 0` and `get_diff` returns `None`, while the prompt row (`set_agent_prompt`) exists.
  - The same wedge happens without any error. If the daemon restarts while a retried agent is still `pending` (waiting on `reviewer_slots().acquire_owned()` in the spawned task, or before its session spawns), `fail_running` only touches reviews whose `status = 'running'`. A per-agent retry runs on a settled (`done` / `error`) review, so the agent row stays `pending`.
  - Once the row is stuck at `pending`:
    - Retry refuses: `agent_retry_block` returns "agent N is still pending — stop it before retrying" (`claim_agent_retry` also requires `done|error|skipped`).
    - Stop refuses: `stop_review_agent:626` allows only `running|waiting`.
    - Summarizer retry refuses: `try_begin_rerun`'s `NOT EXISTS … status IN ('pending','running','waiting')` fails, and the user is told "review is still running".
  - The review's retry and summarize actions are dead until a new review is started.
- **Fix:**
  - Do every fallible precondition (prompt, root user, and diff re-materialisation from the file or DB) BEFORE `claim_agent_retry`. Alternatively, on any early return after the claim, put back the original settled row with `set_agent_at(&review_id, index, &review.agents[index])`.
  - In boot recovery, add a `ReviewsRepo` sweep that rewrites any agent element in `pending|running|waiting` to `error` / "Interrupted by a daemon restart" for reviews whose status is not `running`.
  - Let `stop_review_agent` also settle a `pending` row that has no registered cancel flag.

### backend-correctness-3-02 — minor — Postgres: a data-modifying statement led by a CTE (`WITH … UPDATE/DELETE`) gets `LIMIT n` appended and fails with a syntax error

- **Where:**
  - `crates/otto-dbviewer/src/sql_class.rs:88`: the Postgres run-path `reads` include `"WITH"` unconditionally;
  - `crates/otto-dbviewer/src/drivers/postgres.rs:695-696` (single statement) and `:1789-1790` (batch): `is_read_statement` then calls `types::inject_row_limit`;
  - `crates/otto-dbviewer/src/types.rs:1217-1288`: `inject_row_limit` and its `SKIP` list at `:1249`.
- **Evidence:**
  - `inject_row_limit` rewrites any statement whose first word is `with` unless a token in `SKIP` (`LIMIT`, `FETCH`, `OFFSET`, `FORMAT`, `SETTINGS`, `INTO`, `FOR`, `UNION`, `INTERSECT`, `EXCEPT`, `LOCK`) appears.
  - `WITH t AS (SELECT id FROM x WHERE …) UPDATE y SET a = 1 FROM t WHERE y.id = t.id` contains none of them, so it runs as `… WHERE y.id = t.id LIMIT 1001`. The same happens to `WITH … DELETE FROM y USING t WHERE …`.
  - PostgreSQL has no `LIMIT` on UPDATE or DELETE, so the statement fails with `syntax error at or near "LIMIT"`.
  - On a guarded connection the write gate correctly requires `confirm_write`. The user confirms the write and still gets the syntax error. On an unguarded connection it simply never runs.
  - The module comment in `sql_class.rs:22-26` deliberately keeps `WITH` on the read path ("a data-modifying CTE still returns rows"). That is right for the shape, but the auto-LIMIT rewrite is not safe for it.
  - MySQL is unaffected, because `WITH` there is only a `provable_read`.
- **Fix:**
  - Add `"UPDATE"`, `"DELETE"`, `"INSERT"` and `"MERGE"` to `inject_row_limit`'s `SKIP` list. The list is conservative: the statement is left unrewritten and only loses the auto-LIMIT.
  - Add tests for `WITH … UPDATE …` and `WITH … DELETE …` returning `limited: false`.

### backend-correctness-3-03 — minor — the shared agent-JSON extractor gives up on every object after one unbalanced `{` in prose, and `parse_pr_draft` fails on any brace before the JSON

- **Where:**
  - `crates/otto-core/src/text.rs:78-121` (`balanced_object_spans`, behind `extract_json` and `find_json_object`). Callers include:
    - `otto-swarm` `recruiter.rs:113`, `runtime/run.rs:121`, `runtime/verify.rs:46` and `runtime/engine.rs:3277-3317`;
    - `otto-product` `refine.rs:572`, `swarm.rs:244,725` and `chat.rs:662`;
    - `otto-canvas` `assist.rs:1155`;
    - `workflow_engine/nodes/review_run.rs:690`;
    - `otto-automation/src/goal_loop_parse.rs:49`.
  - `crates/otto-server/src/modules/pr_draft.rs:89`, a separate first-`{`…last-`}` parser.
- **Evidence:**
  - A `{` at depth 0 sets `start` and increments `depth`. A later `}` only returns a span when `depth` drops back to 0, and the scan never restarts after a candidate that does not close.
  - So `"Changed the \`if x {\` guard. {\"approved\":true,\"findings\":[]}"` gives `depth` 1, then 2, then back to 1 at the object's `}`, and the iterator returns `None` at end of input. `extract_json` returns `None` even though a complete object follows.
  - The test `extract_json_none_for_prose_or_unclosed_objects` covers only "nothing after an unclosed brace". The documented contract ("a span that does not parse (`{placeholder}` in prose) is skipped, not fatal") does not hold for an UNBALANCED one. Agent replies about code mention a lone `{` often.
  - The parse then fails, so a swarm verifier verdict, recruiter, plan, goal-loop review or canvas assist turn is treated as "no JSON" (a retry, or a failed step).
  - `parse_pr_draft` slices `text[first '{' ..= last '}']`, so even a BALANCED `{}` in a preamble ("drafted from the `fn x() {}` change: {…}") makes the slice unparseable. The PR description then falls back to the whole raw reply, JSON included.
- **Fix:**
  - In `balanced_object_spans`, when the input ends with an open candidate (`depth > 0`), restart the scan at `start + 1` (reset `depth`, `in_str` and `escaped`) instead of returning `None`, so the inner balanced object is still found.
  - Make `parse_pr_draft` use `otto_core::text::extract_json`.
  - Add tests: a lone `{` in prose followed by a valid object, and a `{}` preamble before a PR draft object.

### backend-correctness-3-04 — minor — multi-run placeholders use a MySQL-style comment mask on PostgreSQL, so `#>>` / `#>` / `#` hide every placeholder after them, and `$$` bodies are substituted

- **Where:**
  - `crates/otto-dbviewer/src/multirun/params.rs:38-53`: `PlaceholderMode` has only `Sql` and `Line`, and `for_engine` maps Postgres to `Sql`;
  - `:69-126`: `code_mask` treats `#` as a line comment for every `Sql` engine, and treats `\` as an escape inside every quote;
  - used by `crates/otto-dbviewer/src/service/multirun.rs:248` and `:298`.
- **Evidence:**
  - In PostgreSQL `#` is an operator (bitwise XOR, and the jsonb path operators `#>` and `#>>`). For `SELECT data #>> '{a,b}' FROM t WHERE id = :id`, `code_mask` blanks everything from `#` to the end of the line. So `find_placeholders` does not see `:id`: it is not listed as a parameter, `substitute` leaves it in place, and every run of the multi-run fails on the literal `:id`.
  - The reverse case: a `:name` inside a dollar-quoted body (`DO $$ … :name … $$`) counts as code and IS substituted.
  - The editor this module claims to mirror already handles both cases. `ui/src/modules/database/sql-util.ts:30-41` `codeMask` has a dedicated `'pg'` mode using `pgSpanEnd` (dollar quotes, no `#` comments, standard strings). `QueryEditor.svelte:595` substitutes with `splitMode`.
  - So the same statement substitutes correctly in a single editor run but not in a multi-run. This breaks the module doc's promise that "a statement written for a single run sweeps over values unchanged".
- **Fix:**
  - Add a `PlaceholderMode::Postgres`, and make `for_engine(Engine::Postgres)` return it.
  - In that mode, `code_mask` should skip `#`, mask `$tag$ … $tag$` spans (reuse the dialect scanner `split.rs` already has for `SqlDialect::Postgres`), and treat `'…'` with only `''` doubling (backslash escapes only in `E'…'`).
  - Add tests for `#>>` and `$$`.

### backend-correctness-3-05 — nit — renaming a vault note rewrites the frontmatter fences of every note that links to it (CRLF to LF, `...` to `---`)

- **Where:** `crates/otto-vault/src/parse.rs:413-420` (`rewrite_links`), with `split_frontmatter` at `:31-49`. Called from `crates/otto-vault/src/engine.rs:1774-1812` (`rewrite_moved_links`).
- **Evidence:**
  - `split_frontmatter` accepts `---\r\n` as the opener (it strips the `\r`) and `---` or `...` (with any trailing whitespace or `\r`) as the closer. It returns only the YAML between them.
  - `rewrite_links` rebuilds the note with hard-coded `"---\n"` + yaml + `"---\n"`.
  - So for any linking note with a CRLF frontmatter or a `...` closer, a rename's link rewrite (written whenever one link changed) also changes the fence bytes and leaves mixed line endings. That contradicts the doc comment "preserving everything else byte-for-byte" and adds a spurious diff to a git-tracked vault.
- **Fix:** have `split_frontmatter` (or a sibling helper used by `rewrite_links`) also return the byte length of the opening and closing fence lines, and copy `content[..open_len]` and the original closing line verbatim. Add a CRLF / `...` case to `rewrite_noop_is_bytes_identical`.
