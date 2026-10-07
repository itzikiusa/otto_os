# Archive candidate selection and login boundary coverage

Baseline `196048df`, branch `fix/quality-20261007`. Local changes only; coordinator owns all Cargo execution.

## Archive repair

`SessionManager::auto_archive_stale` now reads pages of at most 128 IDs from `SessionsRepo::auto_archive_candidate_ids`, selecting only unarchived agents older than the cutoff. The storage API caps requested pages at 256. The ID cursor advances even for pinned, attached, live, failed, or newly active candidates. ID ordering stays valid when archival changes last-active timestamps. Every mutation still goes through the unchanged `archive_if_stale` resume lock and fresh row/liveness/attachment/pin checks.

The isolated SQLite experiment applied every real migration and explained the candidate query. Before the new index: `SEARCH sessions USING INDEX sqlite_autoindex_sessions_1 (id>?)`, requiring table reads. With the proposed index: `SEARCH sessions USING COVERING INDEX idx_sessions_auto_archive (id>?)`. Migration `0180_session_auto_archive_index.sql` adds only this partial `(id,last_active_at)` index for unarchived agents. No existing migration was changed. The sweep scans only narrow active-agent index entries; it may still scan recent unarchived agents to find older candidates, but it does not read archived history or materialize that history's payloads.

Baseline regression `quality_auto_archive_ignores_irrelevant_history_and_reaches_later_candidates` failed as intended: 0 archived versus 10 expected (`/tmp/otto-quality-20261007-archive-red.log`, coordinator execution). It seeds 130 archived 8 KB payloads with an unreadable legacy timestamp, more than a page of pinned candidates, one attached candidate, and ten later eligible IDs. The old full-row mapping lets irrelevant legacy data abort the entire sweep.

Additional authored checks: `quality_auto_archive_pages_are_bounded_complete_and_covering` compares all page results with independently seeded expected IDs, mutates rows between pages, checks cutoff/kind/archive exclusions and the actual query's covering-index plan. The existing `auto_archive_rechecks_staleness_under_the_lock` test now also checks live, attached and newly pinned refusal before finally archiving a stale unpinned row.

## Login coverage (T8)

No production auth logic changed. New tests in `auth_routes.rs` call the real private `handle_login` using isolated `AttemptStore` instances and the existing full `ServerCtx` fixture, or route through the real public login handler and `host_guard_with_settings` middleware.

- `quality_login_handler_rotating_peers_lock_username_but_desktop_can_recover`: real existing user's bad passwords produce 401 then 429 despite peer rotation; valid remote/tunnel credentials remain gated; desktop recovery issues an authenticatable token and clears the account tally; desktop's own per-client threshold still applies.
- `quality_login_handler_client_bucket_stops_username_spraying`: varying usernames on one remote peer exhausts the independent client bucket; another peer can still authenticate the actual fixture user.
- `quality_login_router_ignores_spoofed_forwarding_headers_and_scopes_local_exemption`: actual POST requests carry changing X-Forwarded-For/X-Real-IP headers and real ConnectInfo. The assertion inspects the real peer's recorded production bucket, so a global username lock cannot conceal trusting spoofed headers. Loopback to the share DNS host is still gated; loopback to localhost can recover. Test-owned global keys are cleared afterward.

429 assertions check positive bounded Retry-After and the actual problem response code. Tests use fixture users, isolated local databases and temporary key stores, with no network requests, provider workloads or live-state writes.

## Verification status and required commands

Formatting of the three edited Rust files and `git diff --check` passed. No build or test suite was run by this worker. Green execution remains pending with the coordinator:

```sh
cargo test -p otto-state --lib quality_auto_archive
cargo test -p otto-state --test migration_compat
cargo test -p otto-sessions --lib auto_archive
cargo test -p otto-server --lib routes::auth_routes::tests::quality_login
```

Use the coordinator's jobs=2 / debug=0 / incremental=0 environment. These tests are part of the affected package lib/integration gates. Auth tests cover the request/middleware boundary in process, not an actual listening TCP socket, TLS proxy or live deployment. The index experiment is a query-plan check; it is not a throughput, CPU or memory load measurement. Existing helper-level auth tests remain useful and were retained.
