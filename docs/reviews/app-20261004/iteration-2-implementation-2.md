# Iteration 2 implementation — data tools

Follow-up to `iteration-2-correctness-2.md`. All three findings are accepted and repaired in source; parent owns verification. Existing iteration-1 fixes remain on the same branch. No builds, suites, servers, remote mutations or subagents were run by this implementation agent. Owned Rust files were formatted only.

## C2-R2-01 — PostgreSQL identifier provenance

`ui/src/modules/database/edit-sql.ts` now normalizes unquoted PostgreSQL schema/table identifiers to lowercase before catalog lookup and generated mutation quoting. Double-quoted names preserve exact case. PostgreSQL's accepted identifier grammar excludes MySQL backticks. Other engine behavior is retained.

`ui/unit/dbEditScope.test.ts` covers `app.Users`, `APP.users`, `app."Users"`, `"APP".users`, rejected PostgreSQL backticks, and actual UPDATE generation against the resolved target. Thus an unquoted `app.Users` query produces `UPDATE "app"."users"`, while quoted `app."Users"` produces `UPDATE "app"."Users"`. These are pure parser/builder regressions; no live PostgreSQL write is claimed.

Parent confirmed RED: the old parser returned `Users` instead of `users` in `/tmp/otto-review-r2-red.log`.

## C2-R2-02 — conservative SQL limit eligibility

`crates/otto-dbviewer/src/types.rs` replaces digit-after-space recognition and substring clause matching with the existing sqlparser tokenizer. Any real, unquoted LIMIT token preserves the original SQL, including ALL, expressions, newlines, comments and placeholders. FETCH/OFFSET/locking clauses, INTO and set operations also remain unchanged. Unknown tokenization returns the original statement. Quoted identifiers, string contents and comments are not mistaken for clauses. A trailing line comment gets a newline before an eligible injected LIMIT so it cannot swallow the clause. MySQL-style `#` ambiguity conservatively disables rewriting.

Tests cover explicit LIMIT ALL / (2) / newline / commented numeric limits, FETCH FIRST, FOR NO KEY UPDATE / KEY SHARE / newline UPDATE, plain SELECT eligibility, quoted/commented LIMIT mentions, and trailing line comments. New ignored MySQL/PostgreSQL integration tests exercise explicit-limit batches and verify the second statement runs on the same backend session. They require the existing disposable SQL fixtures and are not claimed as executed here.

Parent confirmed RED: `LIMIT ALL` became `LIMIT ALL LIMIT 3`, `/tmp/otto-review-r2-sql-red.log`.

## C2-R2-03 — workspace context leave decisions

`router.mayChangeWorkspace()` reuses the registered leave guards and their generation ownership without changing the hash. It passes an empty destination, so guards that exempt routes inside the current editor do not accidentally exempt replacing its workspace. No guards means a synchronous true result, preserving startup session-prefetch timing.

`WorkspaceStore.select` and private `selectNone` ask before modifying workspace identity, tabs, sessions, storage keys or the API context. A separate request counter cancels stale decisions, including when the person selects the current workspace while another choice is pending. Existing selection generation cancels an approval made stale by auth/reset or another applied context. Startup from no workspace and same-workspace retry skip leave prompts. Selection returns a boolean so dependent actions can stop.

Callers audited: Navigator filter/chip/row/menu, App command and host/guest events, FloatingBar space selection, Swarm/Canvas direct selection, cross-workspace session opening and notifications. Direct callers receive the central guard automatically. Navigator's Workspace context action, notifications' openRoute and cross-workspace session opening now stop after a canceled/superseded selection. No shell layout or visual changes were made.

Current-workspace archive received the parent-approved adjacent repair: run leave approval before DELETE; cancel sends no request. Once approved, fallback selection uses the internal already-approved path (including scratch when the last workspace is removed), so it does not ask to save into an archived workspace. A newer completed selection owns the current workspace and cannot be redirected by a late archive response. The Navigator suppresses the removed toast on cancellation.

Regression coverage:
- Cancel preserves current workspace, sessions and tabs before/after the asynchronous decision.
- Save remains scoped to the original workspace until approval resolves.
- Reselecting the current workspace cancels an older pending switch.
- A route navigation supersedes a pending workspace decision.
- A canceled notification workspace switch does not navigate in the old context.
- Canceling archive sends no DELETE; approved archive asks once and selects the fallback.
- Last-workspace archive asks once and selects scratch; a late archive response does not redirect a newer completed selection.
- Existing boot-prefetch, same-workspace retry and delayed-session-layout tests remain in the focused suite.

Parent confirmed behavioral RED: three workspace ownership failures (`/tmp/otto-review-r2-red.log`) and both archive preflight cases (`/tmp/otto-review-r2-archive-red.log`).

Correction after inspecting the actual logs: `/tmp/otto-review-r2-notification-red.log` failed during fixture construction because its workspace mock omitted `sessions`, before the notification navigation path executed. It is **not evidence of a behavioral red**. The later `/tmp/otto-review-r2-ui-green.log` had 35 passing tests and the same one fixture failure. Added the missing `sessions: []` to that mock only; production code is unchanged. A corrected notification test run remains pending with the parent.

## Verification handoff

```sh
cd ui
node --test unit/dbEditScope.test.ts unit/asyncOwnership.test.ts unit/routerGuard.test.ts unit/notificationsIngest.test.ts
npm run check
# root, parent-controlled build slot / consistent feature set
cargo test -p otto-dbviewer --lib existing_limit_forms_and_fetch_locking_are_never_rewritten
cargo test -p otto-dbviewer --lib quoted_or_commented_limit_tokens_do_not_disable_plain_preview
cargo test -p otto-dbviewer --lib injected_limit_is_not_swallowed_by_trailing_comment
```

Also rerun the existing injector tests. Isolated integration tests, when disposable fixtures are available:

```sh
OTTO_DBV_E2E=1 cargo test -p otto-dbviewer --test postgres_e2e postgres_batch_preserves_explicit_limit_forms -- --ignored
OTTO_DBV_E2E=1 cargo test -p otto-dbviewer --test mysql_e2e mysql_batch_preserves_explicit_limit_forms -- --ignored
```

Iteration-1 verification subsequently received from parent: full UI gate passed; 1,015 unit tests passed; one schema-history test, two Mongo budget tests and eight auto-stash tests passed (`/tmp/otto-review-data-rust-green.log`). These precede iteration-2 changes. Iteration-2 green checks and rendered navigator/command-palette automation Save/Discard/Cancel flows remain with the parent and must not be inferred from source-only inspection.
