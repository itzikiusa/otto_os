# E17 — Channels engineering review, iteration 01

Verdict: **Block the 9.8 readiness claim.** Five confirmed source-traced defects: two P1/high and three P2/medium. No P0 claim. Overall assessed readiness **7.4/10**, not a measured reliability statistic. Correctness 7.0; regression coverage 7.0; architecture 8.0; performance/recovery 8.0. No lens is certified at 9.8 by a static pass.

## Provenance and constraints

- Reviewer: E17-channels; date: 2026-10-18.
- Branch: `fix/db-parity-session-consistency`; HEAD at start and finish: `c009da20cc113c8fb79f3bf0600442d0c6afa358` (PR111 context supplied by coordinator).
- Reviewed the existing subsystem, not just changed lines. No local modifications were present in `crates/otto-channels`, `run_channels.rs`, or `improve_channels.rs` at the final check. Other reviewers were editing unrelated/shared consumers; their changes are not attributed to this review.
- Read AGENTS.md and correctness-review, test-review, architecture-review, performance-review skills. Memory registry search returned no relevant entries; findings rely on current source.
- **No Cargo, builds, tests, network-provider probes, commits, or production edits.** All confirmation below is hand tracing, not executed reproduction. This report is the only authored file.
- Early findings were delivered with `otto_send_message` to coordinator session `01M57PYK75FC1F2AFS4C73BBF7` under E17-channels.

## Confirmed findings

### E17-01 — P1/high: Codex sessions launch successfully but the mirror reads a Claude transcript

**Location:** `crates/otto-channels/src/mirror.rs:394`; provider selection at `crates/otto-channels/src/bridge.rs:1034` and session creation at `:1062`; parser at `crates/otto-channels/src/transcript.rs:348`.

**Intent:** The feature guide explicitly allows `preferred_cli=codex` and promises working output and the final answer back in the originating thread (`docs/features/channels-slack-telegram.md`, settings table and relay description). The bridge honors explicit, workspace, and global provider defaults.

**Trace:** Configure an enabled channel with the installed Codex provider; send an allowed user's question. Bridge creates a Codex session and submits the prompt. `wait_for_psid` returns only cwd and the provider session ID (`mirror.rs:672-674`). `run_tailer` always calls `claude_pty::session_jsonl_path`, which constructs `$HOME/.claude/projects/<encoded-cwd>/<psid>.jsonl` (`crates/otto-orchestrator/src/claude_pty.rs:469-485`). Codex writes a `rollout-...-<psid>.jsonl` under its sessions tree; the already-existing provider-aware lookup proves the distinction (`crates/otto-sessions/src/manager.rs:4732-4769`). The mirror polls the wrong file, and its parser additionally requires Claude's `message.role/content/stop_reason` envelope. No Codex final can reach the adapter. The agent is explicitly instructed not to post replies itself (`bridge.rs:1135-1154`), so there is no alternate delivery path.

**Actual vs expected:** Agent performs work, but channel gets no final answer; expected relay of the answer for the provider actually selected. The same hard-coded path also ignores configured Claude account homes and canonicalized cwd, unlike the session manager's resolver; those are additional affected variants of the same defect.

**Fix direction:** Use the session/provider-aware artifact resolver and normalized provider transcript events. Reuse a real provider seam rather than copying more home/path rules into channels. Providers without readable reply events must have an explicit supported alternative or an actionable unsupported-provider result, not a silently successful launch. Preserve the owner's ability to choose/use providers.

**Regression needed:** With fixture session metadata and filesystem transcripts, launch/attach Codex and account-backed Claude; append a tool event and final, and assert the correct destination receives exactly one final. Existing Claude parser fixtures do not exercise provider selection or artifact resolution.

### E17-02 — P1/high: A permanent Telegram notice failure blocks every later update for the bot

**Location:** `crates/otto-channels/src/telegram.rs:706-711`; contiguous-prefix control at `:676-681`, poll/offset update at `:642-655`.

**Trace:** Update 10 is an allowed user's bare document/photo. Before Otto replies, that user blocks the bot (or the bot is removed from that group). Telegram returns the permanent `Forbidden: bot was blocked by the user` error. The adapter preserves that error (`telegram.rs:294-299`; fixture coverage at `:903` onward), but `handle_update` reduces **every** error to `false`. `dispatch_prefix` stops without advancing past update 10. Update 11, an ordinary message from another allowed chat on the same bot, is never dispatched. The next successful poll returns the same prefix; the code retries the impossible notice after one second, indefinitely until external state changes or the update expires. Restarting the listener does not solve the condition. `health.connected()` is called after each successful poll, despite the wedged delivery queue.

**Actual vs expected:** One permanently undeliverable explanatory notice stalls all conversations sharing the bot. Expected: record a terminal failure for that notice and continue to later updates; retain/retry only genuinely retryable failures. A busy notice has the same unclassified-error behavior at `telegram.rs:768-774`, though the bare-file path alone establishes the indefinite stall.

**Cost:** O(number of repeated polls) failed notice requests, while useful throughput for every later chat is zero. With prompt API responses the loop approaches one failed notice per second. This is a recovery/correctness defect, not a hypothetical large-scale optimization.

**Fix direction:** Return a typed disposition: handled, terminally rejected, or retryable. Reuse permanent/transient/rate-limit classification already present in `mirror.rs:735` instead of treating all notice errors alike. Surface the terminal rejection to the owner, advance the prefix for it, and honor retry timing for transient rejection delivery.

**Regression needed:** Exercise the actual notice handler with update 10's send returning 403 and update 11 belonging to another chat. Assert update 11 is dispatched and the cursor advances. Separately assert a 429/temporary failure preserves the update and respects its cooldown. The current `failed_admission_retains_suffix_and_retry_does_not_duplicate_prefix` test (`telegram.rs:789`) supplies booleans directly; it proves prefix retention, but cannot distinguish permanent errors from retryable ones.

### E17-03 — P2/medium: Final-reply retries exhaust before a valid Retry-After expires

**Location:** `crates/otto-channels/src/mirror.rs:1059-1064`; constants at `:1024-1029`; Slack decoding at `crates/otto-channels/src/slack.rs:202-218`.

**Trace:** First final-reply attempt at t=0 returns HTTP 429 with Retry-After 120 seconds. Slack's adapter deliberately preserves that hint (capped at 120). The mirror parses it, then truncates the wait to `REPLY_RETRY_MAX=30s`. Second attempt at t≈30 remains rate-limited with 90 seconds remaining; it waits another 30 seconds. The third/final attempt at t≈60 still occurs before t=120 and fails. `post_reply` logs and returns unit; the final is subsequently marked posted at `mirror.rs:576`. There is no attempt when the service actually permits delivery.

**Actual vs expected:** A recoverable cooldown loses the final answer and causes two requests the server explicitly asked the client not to make. Expected: honor the server's not-before time, retaining a bounded pending delivery if the caller cannot wait that long.

**Fix direction:** Do not clamp a server minimum delay downward. Bound total delivery lifetime separately, honor the advertised deadline, and retain an explicit retryable delivery result when a budget is exhausted. A maximum attempt count alone does not justify retrying before eligibility.

**Regression needed:** Use a controllable clock and an adapter that rejects until t=120, then succeeds. Assert no second request before that time and eventual delivery. Existing `a_rate_limited_final_reply_is_re_posted` (`mirror.rs:1340`) uses zero-second hints; `retries_are_bounded_and_only_for_rate_limits` (`:1354`) checks counts, not timing. Both allow this defect.

### E17-04 — P2/medium: Slack attachment I/O reorders messages within one conversation

**Location:** `crates/otto-channels/src/slack.rs:939-947`, `:1133`, `:1193`; bridge serialization begins at `crates/otto-channels/src/bridge.rs:964-968` and turn enqueue at `:1100`.

**Trace:** Event A arrives in thread T: a document plus “Use this document.” The socket loop admits/ACKs it and spawns `handle_event`. That task waits in `collect_attachments`. Event B then arrives in the same T: “Summarize the document above.” B has no files, so its handler reaches `Bridge::handle` first, creates/reuses the session and enqueues its turn. A only enters the bridge after its download finishes. The agent processes B then A. The download client permits a 120-second request, so this is a practical network scheduling case rather than a narrow lock race.

**Actual vs expected:** Follow-up reaches the agent before its prerequisite document/context. Bridge comments promise arrival-order turn queues, but the mutex orders entry into the bridge after preparation, not original inbound arrival. Separate conversations should stay concurrent.

**Fix direction:** Reserve ordering for normal messages per conversation at ingress, before spawning attachment work. Preparation can remain parallel, but dispatch must consume the reserved sequence. Keep `/stop` and other control messages on their explicit reserved path so ordering does not make cancellation wait behind a download or running turn.

**Regression needed:** Hold A's attachment future behind a barrier; receive B in T and C in another thread. Verify C proceeds, B cannot dispatch before A, then release A and assert A/B order. Bridge's direct turn-lock test (`bridge.rs:1628`) starts after this reorder point and cannot catch it.

### E17-05 — P2/medium: A long answer claims an attachment was delivered when upload failed

**Location:** `crates/otto-channels/src/mirror.rs:1077-1087`, `:563-576`; Slack upload starts at `crates/otto-channels/src/slack.rs:519-524`.

**Trace:** A final answer exceeds 1,800 characters. `post_reply` sends only a 1,500-character head with “full reply attached as investigation.md.” The subsequent upload fails, for example on the first `files.getUploadURLExternal` call with a definite HTTP 429 before any file was uploaded. Unlike the text send, upload has no retry here. The function only logs the error and returns `()`. The caller sets `last_posted_final=Some(joined)` regardless of the failed upload, suppressing repeated Final events. The omitted body is never delivered and the visible attachment claim is false. Explicit file uploads have a similar swallowed-result boundary, but this finding is established by the automatic long-answer path alone.

**Actual vs expected:** The user sees a truncated answer claiming to have a full attachment which does not exist. Expected: successful full-answer delivery, or a visible partial-delivery result with a recoverable attachment; no success dedupe for an undelivered body.

**Fix direction:** Make reply/attachment delivery return a structured outcome. Track text and file parts separately, retry definite pre-delivery rejection safely, and state attachment success only after upload succeeds. Preserve failed parts for retry without reposting the already delivered head. Treat uncertain upload timeouts separately from definite rejections to avoid duplicate files.

**Regression needed:** Adapter succeeds on the head, rejects the first upload before acceptance, then recovers. Assert the full content eventually arrives once, the head is not duplicated, and the failed upload does not mark the entire final as delivered. Also test terminal upload failure yields an honest visible status. The reviewed mirror tests exercise `send_formatted` retries, extraction, and feed health, not this partial-delivery result.

## Architecture and test assessment

The transport `Adapter` and injected Run/Workflow/Swarm/Improvement hooks are useful existing seams; they keep channel wiring independent of server-owned engines. Admission separates control capacity from normal work and bounds normal accepted tasks at 256 globally, 64 per integration, and 8 per conversation. Do not replace these with a new generic task framework.

Two concrete structural costs are already visible in the findings: provider path/format knowledge is duplicated outside the provider-aware session layer (E17-01), and unit-returning `post_reply` erases the outcome needed for dedupe and recovery (E17-03/05). Extending another provider or adding safe attachment retries currently forces the caller to guess state. Small changes at these seams are justified by present variants, not speculative abstraction.

Good reviewed coverage includes Telegram non-2xx envelope decoding and Markdown fallback, cancel during held polling, cross-generation poller locking and offsets; Slack FIFO dedupe eviction, human-edit identity, rejection-notice acknowledgement; feed permanent-error shutdown and bounded text retry; bridge turn locks, detach metadata, and idle-release plumbing. These tests provide useful evidence but do not cover the end-to-end compositions above. They were read, not run.

Performance is reasonably bounded in several important places: ingress permits, 2,000-entry Slack event dedupe windows, three concurrent attachment downloads per message, 50 MiB inbound attachment cap, 250 activity lines/3,500 displayed characters, shared HTTP pools, adaptive transcript polling, and bounded socket dial/write/watchdog timings. No blanket N+1 or “large file” finding is asserted. The actionable recovery cost found in this pass is E17-02's bot-wide stall.

## Coverage and residual boundaries

Deep source tracing covered Slack ingress filtering/dedupe/ACK/attachment ordering, reconnect/backoff/socket watchdog, send/edit/upload; Telegram polling/cursor/generation cancellation, admission and notices, send/format/edit/upload; bridge allow-list, provider choice, conversation lookup/turn serialization/submit/control routes; mirror attachment, transcript lookup, event handling, feed health, final retries and upload outcomes; proactive event classification/delivery; manager generation/pending-token supervision; integration HTTP configuration and persistence.

Caller/callee checks included `otto-server` boot/build and channel-manager wiring, run trigger, self-improvement glue, webhook admission/adapter, session artifact lookup, integration state repository, feature guide and channel API contract. Slack directory limits/cache and attachment guard/seed/email entrypoints were inspected for context, not exhaustively audited.

**Explicitly not certified:** real Slack/Telegram behavior or credentials, UI rendering/accessibility (design reviewers own it), SMTP/email OTP, full webhook auth/SSRF behavior, every redaction pattern, complete provider parsers, deep workflow/swarm/automation execution, native macOS sleep/resume, durable crash recovery, live token rotation with active mirrors, or sustained load. Notification broadcasting is explicitly documented best-effort and drops lagged events; this pass does not invent an exactly-once notification guarantee. In-memory inbound ACK/dedupe is not a durable inbox; stronger crash-recovery semantics require an agreed contract and targeted restart tests. These are residual boundaries, not unproven blockers.

No claim that the five findings exhaust all defects in the subsystem. Required next step: coordinate ownership with the root, fix the proven delivery paths, run targeted failing regression cases plus the affected-crate gate when allocated, then independently cross-review the fixes. E17 remains available for that work; no production ownership was assumed in this pass.

## Allocated fix pass — implementation in progress

Root allocated all five channel fixes after the read-only pass. Changes are confined to `crates/otto-channels` and this report; no session manager/server glue edits. Root approved adding the existing `otto-transcript` workspace dependency and reserved its single Cargo.lock dependency-list update for the allocated Cargo run.

Implemented for verification:

- E17-01: the mirror asks `SessionManager::activity_artifact` for the provider/account-aware path; Codex content uses `otto-transcript` normalization and is emitted as a final only on the matching `task_complete`. Duplicate/stale completions and aborted turns do not emit replies. Unsupported or unavailable transcript sources produce a channel notice instead of silently polling a fictitious Claude file. The cold-start artifact wait is 120 seconds.
- E17-02: Telegram notice results distinguish terminal rejection from retryable failure. A permanent rejection settles its update so later chats proceed. Rate limits retain the update and honor the cooldown; cancellation interrupts the wait. Structured Telegram `parameters.retry_after` is authoritative over prose. A cancelled generation does not start another update from its fetched batch.
- E17-03: removed downward Retry-After caps from both the Slack adapter and mirror. Final replies rejected by rate limits stay in a bounded in-memory delivery queue, with their original destination, rather than exhausting three attempts before eligibility. Queue waits do not block transcript processing and survive the agent's exit. This is not a durable outbox across daemon restarts.
- E17-04: Slack and Telegram reserve a conversation FIFO position at ingress before spawned work/attachment preparation. Explicit control commands bypass this queue. Different conversations remain independent.
- E17-05: automatic long-answer delivery retains per-part state, uploads the full answer before claiming the attachment exists, and does not repeat an accepted upload when only the head was rejected. Rate-limited parts stay pending. Permanent/ambiguous transport failures are surfaced visibly; uncertain accepted writes are not blindly repeated.

Focused regressions added for matching/stale/duplicate/aborted Codex completions and both transcript eras; 90/120-second cooldown preservation; permanent Telegram notice prefix progress; the actual Telegram bare-file handler against a local 403 fixture; authoritative structured retry delay; ingress ordering; failed long-answer upload; and successful upload followed by rate-limited head. Existing reply retry tests now exercise the production delivery helper, not a test-only copy.

Verification so far: owned-file `rustfmt --edition 2021 --check` and `git diff --check` passed. An isolated Python syntax check parsed 17 JSON fixture literals; this is explicitly not execution of Rust regressions. **Cargo tests, compilation, and clippy remain unrun awaiting the root's shared slot.** Requested commands: `cargo test -p otto-channels --lib`, `cargo clippy -p otto-channels --all-targets -- -D warnings`, then affected consumer `cargo check -p otto-server --lib` if allocated. No real Slack/Telegram requests, commits, pushes, or deployment.

Static re-review round 2 additionally corrected retained deliveries being discarded on agent exit, structured-vs-prose retry timing, cancellation during notice backoff, and tests accidentally targeting superseded helper logic. Readiness score is **not raised** before compiled/runtime evidence. Residuals to scrutinize in the next pass: provider fixture coverage at the actual tail loop, partial upload timeout ambiguity, explicit file-directive uploads (distinct from automatic long-answer attachments), and daemon restart durability.

## E17 final source-frozen handoff — 2026-10-18

All five allocated fixes are implemented for validation. No new assignments or sessions were opened. Source is frozen at this handoff; edits are preserved, uncommitted. Exact owned source files and SHA-256 at freeze:

- `crates/otto-channels/Cargo.toml` — `a00310539cd39520ab0ecf99547cb689e97a3d2649fc400124b621b2a3ca30f2`
- `crates/otto-channels/src/admission.rs` — `1f9afbb45385e22475610d662f6e6e5e9971cccf2e08939f2204cef2eb246239`
- `crates/otto-channels/src/bridge.rs` — `326b081b5a08d903034973716d926e9ee1f14d0dfde3a125e6a899537bf9c5f7`
- `crates/otto-channels/src/mirror.rs` — `4e042cbdd407087408c6c628dbcd5b1eaa3853c476869baea757e208275de783`
- `crates/otto-channels/src/slack.rs` — `9735dae662aee62d1c14c8f7196725834c600a40a1463bdcdad811923a3338f0`
- `crates/otto-channels/src/telegram.rs` — `8ce3f6ffb02930a8c8bd0539a55df5b42cce611d00e2f159d329837721d41172`
- `crates/otto-channels/src/transcript.rs` — `7aa77fbff5f29cf001d920495e5cc10567a26f50e98d99928ae16a31b4ffeba6`

The only other edited file is this report. No server glue, session manager, or Cargo.lock edits were made by E17. The existing workspace `otto-transcript` dependency addition was approved by root; only its channels dependency-list lockfile refresh remains reserved.

Final verification: owned-file `rustfmt --edition 2021 --check` and `git diff --check -- crates/otto-channels` both passed again at freeze. Earlier lightweight JSON fixture syntax check passed (17 literals). No Cargo/build/Rust tests/clippy were run; no real Slack/Telegram sends, commits, stashes, reversions, pushes, or deployment. E17 never acquired the shared Cargo slot and has no Cargo process: Cargo is released/free from E17.

Pending allocated checks: `cargo test -p otto-channels --lib`; `cargo clippy -p otto-channels --all-targets -- -D warnings`; affected consumer `cargo check -p otto-server --lib`. Root was sent the queue request. These checks must compile and execute the new regressions before calling the fixes verified. The final additions include the actual Codex tail partial-line regression and preserving HTTP Retry-After on Slack raw upload bytes.

Residual boundaries: automatic long-answer retries are covered in source/tests, while explicit file-directive uploads remain best-effort; pending replies are memory-only across daemon restart; ambiguous accepted-write failures are visible but not blindly resent. Codex normalization is per record, so duplicate tool activity across transcript eras still deserves independent review. No claim of 9.8+ or passing Rust tests is made; the pre-fix 7.4 readiness score is not replaced with an unsupported post-fix score. Remaining work belongs to validation and independent cross-review of this allocation.
