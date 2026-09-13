# Otto performance and responsiveness review

**Verdict: Approve with fixes — 0 blockers, 11 major findings, 1 minor finding.** The worthwhile work is preventing stuck operations and avoiding work that grows with accumulated data. This review does not recommend chasing small speed gains.

Reviewed the merged tree of [PR #45](https://github.com/itzikiusa/otto_os/pull/45), merge commit `f2b885b016bd49fb776658a07a3f595e50e98962`, on 2026-09-13. The local review tree has the same Git tree hash, `141c58d09d13043335fbb43f6f7aae42b9fd0f66`. Scope includes Agents, Connections/DB Explorer, Git, Vault, API, Workflows, the shared folder picker, and backup UI. It includes older reachable code, not only changes introduced by PR #45.

Three parallel reviewers covered the six feature areas; the primary reviewer traced shared paths, checked the strongest findings, and consolidated this report. Findings below are confirmed source-level work/lock paths. Small isolated measurements are labeled separately. **No production workload, actual user data sizes, installed-app frame timings, or release-mode latency was measured.** Example sizes are scenarios, not observations about this user's data. No performance fixes, app rebuild, deployment, installation, restart, or provider launch were performed during this sweep.

## What to fix first

| Order | Change | Why it matters |
|---|---|---|
| 1 | Run interactive API scripts in a cancellable worker | One accidental loop can freeze the entire webview, including Cancel. |
| 2 | Resync only conversations with active viewers | Returning to the app can reload historical chats and resume suspended agents whose views were closed. |
| 3 | Move remote database initialization outside shared cache locks | One bad connection can delay an unrelated healthy connection. |
| 4 | Bound Git worktree probes and folder browsing | Slow filesystem work can leave operations waiting without an application deadline. |
| 5 | Update Vault indexes per changed file; refresh only affected folders | Ordinary saves currently cause whole-Vault work and repeated full-list queries. |
| 6 | Separate list/progress metadata from large bodies | API history, Agents History, and workflow polling do much more work than their displayed results require. |

“Major” here means a concrete issue worth fixing on an ordinary failure condition or a plausible larger dataset; it does not mean the running app was observed failing. No production-scale evidence supports a blanket release-blocking performance verdict. The findings are ordered by severity and source area below.

## Findings

### [Major] Slow database setup blocks unrelated cached connections

**Location:** `crates/otto-dbviewer/src/drivers/mysql.rs:1533`; `crates/otto-dbviewer/src/service.rs:749`. Equivalent driver patterns appear in `drivers/postgres.rs:1283`, `drivers/redis.rs:512`, and `drivers/mongodb.rs:1347`.

**What and cost:** Global cache mutexes remain held while a new connection performs remote setup. Existing cache hits must acquire those same mutexes. With Q unrelated initializations, a healthy lookup can wait behind the sum of their setup delays instead of an O(1) lookup. **Two connections suffice:** an unreachable new endpoint and a working existing endpoint. The shared SSH-tunnel map extends this coupling across tunneled database engines; SSH readiness has a 12-second deadline (`crates/otto-ssh/src/lib.rs:159`). Other library-default timeouts were not assumed or measured.

**Evidence and bounds:** Source tracing shows `tunnel_for` holding the lock across `SshTunnel::open`/`open_socks` at `service.rs:767`; MySQL holds its map guard across pool setup at `mysql.rs:1537`. These async waits yield an executor thread, but retain the mutex. Pooling and idle reaping exist and do not isolate different keys during initialization.

**Fix and payoff:** Use the global lock only to look up or install a per-key initialization handle; connect outside it. Preserve per-key single-flight behavior, configuration revision checks, and authorization-scoped keys. Healthy keys stop waiting for another endpoint's timeout. Verify with a fake pending initializer for A while cached B remains immediately usable, and two A callers share one initialization.

### [Major] Git worktree listing bypasses subprocess deadlines

**Location:** `crates/otto-git/src/local.rs:855`, especially `path_has_changes` at `:865`.

**What and cost:** Listing worktrees runs one bounded list command, then sequentially awaits one raw `git status` subprocess per non-prunable worktree. Those probes bypass the shared timeout and process-group cleanup runner. For W worktrees, latency is list time plus the sum of W status times. Illustratively, 20 half-second probes add 10 seconds; one stuck filesystem/fsmonitor probe can leave the request pending indefinitely. Output is also fully buffered. W has no count cap.

**Evidence and bounds:** `http.rs:1781` reaches this helper. `ui/src/modules/git/GraphView.svelte:423` loads worktrees separately from the initial graph, so the initial graph need not wait; its refresh at `:514` does wait. Null stdin and `GIT_TERMINAL_PROMPT=0` prevent credential prompts, not stuck processes. Most other Git operations already have bounded runners (`local.rs:404`).

**Fix and payoff:** Use the shared bounded runner, a small concurrency limit, and an overall listing deadline. Return available worktrees with explicitly unknown dirtiness for timed-out probes; retain removal's independent safety checks. Serial W-probe latency becomes bounded batches, and abandoned requests do not leave unbounded child lifetimes. Verify with a sleeping Git shim and child-cleanup assertions.

### [Major] API history refresh downloads complete response bodies for a summary list

**Location:** `crates/otto-server/src/routes/api_client.rs:885`; full-row query at `crates/otto-state/src/api_client.rs:584`; UI reload at `ui/src/lib/stores/apiClient.svelte.ts:493`.

**What and cost:** The sidebar needs method, URL, status, and timing, but receives each entry's complete request/response JSON. H defaults to 100, is capped at 500, and each retained response text can be 512 KiB. Response text alone therefore approaches **50 MiB per default page**, or 250 MiB at the endpoint cap. Request bodies and escaping add more. Each refresh pays O(H × body bytes) database reads, decoding, serialization, transfer, and browser parsing. Send refreshes directly (`apiClient.svelte.ts:988`); the history event separately schedules a reload (`:506`), allowing duplicate work.

**Evidence and bounds:** Full rows are decoded at `otto-state/src/api_client.rs:617`; the route returns them intact. `HistoryList.svelte:96` renders summary fields. The workspace/time index, entry limit, 512 KiB text limit, removal of stored base64, and virtualized DOM are useful safeguards, but do not make this payload small. A detail endpoint already exists at `routes/api_client.rs:907`.

**Fix and payoff:** Add a summary projection/DTO and fetch full detail only for the selected row; coalesce direct/event refreshes or append the new summary by ID. Cost becomes O(H × small metadata), plus one selected body, rather than tens of MiB per reload. Preserve exact detail for replay and update contracts/types together. Verify that 100 stored large bodies are absent from the list response and one selected detail remains complete.

### [Major] Folder browsing can leave navigation waiting while blocking a daemon worker

**Location:** `crates/otto-server/src/routes/fs.rs:352`; `ui/src/lib/components/FolderPicker.svelte:58`.

**What and cost:** The async browse handler synchronously checks/canonicalizes the path, enumerates every entry, and probes `.git` under every child directory. N entries and D directories cost O(N) enumeration, D serial metadata probes, O(N log N) sorting, and O(returned entries) UI rows. There is no entry cap or paging. A permitted slow/unresponsive mount can occupy **one** async runtime worker for the duration of a blocking syscall; multiple such requests can occupy more workers. Thousands of entries also generate thousands of DOM rows.

**Evidence and bounds:** `fs.rs:378` iterates the whole directory; `:404` probes each child's `.git`. The picker calls `api.get` without a deadline/signal (`FolderPicker.svelte:66`); the shared client supplies neither by default (`api/client.ts:84`). Navigation disables until `finally`; destruction only invalidates the result (`FolderPicker.svelte:136`). **Cancel still closes the modal** (`:239`), but does not cancel the pending request. Authentication, allowed roots, and omission of ordinary files unless requested constrain access, not latency/count. Router middleware adds no request timeout.

**Fix and payoff:** Give each browse operation a deadline and cancellation on close/replacement. Move filesystem work into a bounded blocking pool, retaining its concurrency permit until the actual operation exits: timing out an await does not interrupt an underlying filesystem syscall. Reject excess queued work instead of accumulating it. Virtualize large listings while preserving current filter completeness; add paging only with explicit search/completeness semantics. Verify bounded waiting, usable Cancel/retry, and bounded rendered rows with an isolated delayed browse fixture.

### [Major] Agents History applies its page limit after resolving all sessions

**Location:** `crates/otto-server/src/routes/transcript.rs:952`, `:1005`, `:1036`, and `:1125`.

**What and cost:** A 100-row page first loads all visible workspace sessions, global claimed transcript sets, resolves every agent transcript path, and performs an awaited metadata lookup per resolved session. Filtering/cursor/truncation happen afterward. For N resolvable sessions and M globally claimed sessions, an admin page performs approximately five base data queries plus N serial indexed lookups, O(N) filesystem resolutions, O(M) claimed-set work, and O(N log N) sorting; authorization queries are additional. A workspace with 1,000 retained sessions incurs 1,000 lookups per search/page. Search fires after a 250 ms debounce, and Load more repeats the work.

**Evidence and bounds:** `crates/otto-state/src/sessions.rs:142` has no LIMIT for this session fetch. Final default 100/max 1,000 limits and the separate unclaimed-file SQL limit do not cap session-side processing. Path lookups are indexed and filesystem resolution is offloaded; the request nevertheless awaits all of it. Non-admins skip the unclaimed-file page query but still build the claimed sets.

**Fix and payoff:** Join and paginate persisted transcript/session metadata with authorization/filter/cursor predicates before resolving paths; batch the candidate page's metadata and use indexed exclusion for claimed files. Resolve/cache missing paths only for the bounded page. N serial lookups become a fixed small query count and page-sized filesystem work. Verify query counts for 100 displayed entries from 1,000 and 10,000 synthetic sessions.

### [Major] Workflow polling repeatedly transfers all checkpoint bodies

**Location:** `crates/otto-server/src/routes/workflows.rs:702`; `ui/src/modules/workflows/WorkflowsPage.svelte:224`.

**What and cost:** A viewed pending/running workflow is polled every 2.5 seconds, even with healthy WebSocket updates. Every response reloads all checkpoints, including full inputs, outputs, and logs (`crates/otto-state/src/workflows.rs:138`). Let S be total checkpoint JSON bytes: every poll costs O(S) database read, parse, serialization, and transfer even when nothing changed. A scenario with 10 iterations × 3 steps × 512 KiB combined input/output is roughly **15 MiB per poll**, or **360 MiB/minute** at 24 completed polls/minute, excluding duplicated node output. Slow requests reduce that rate because polling is coalesced.

**Evidence and bounds:** Complete checkpoint contents are persisted at `workflow_engine.rs:5840`. The loop cap is 10 and nested loops are prohibited, but checkpoint byte size is not bounded by that count. One request is in flight; queued refreshes coalesce. The 32 KiB node-event cap can cause fallback full GETs. Directly opened checkpoint details also format bodies while their nested disclosure is closed (`RunSteps.svelte:190`).

**Fix and payoff:** Poll a revision/ETag-backed summary; return checkpoint summaries with lazy/paginated detail endpoints and render bodies only on expansion. Preserve full server-side recovery data. Unchanged polls become O(1), and body transfer is limited to explicitly opened details. Verify payload size with 30 large synthetic checkpoints and unchanged revisions.

### [Major] Large external Markdown is read and parsed before the apparent size threshold

**Location:** `crates/otto-vault/src/engine.rs:298`.

**What and cost:** Every changed Markdown file is read into a byte vector, copied into a string, and synchronously parsed on the async runtime thread. The 4 MiB threshold at `:335` only omits the FTS body afterward. For S bytes of externally created Markdown, memory includes roughly 2S source buffers plus parsed data, with multiple O(S) passes. A 50 MiB generated note therefore creates roughly 100 MiB of source buffers before link/frontmatter structures. Link-rich files also grow accumulated link lists and individual database writes. S has no scanner read/parse ceiling.

**Evidence and bounds:** `scan.rs:51` does not reject by size; `parse.rs:85` handles YAML, links, and word counting before the FTS check. Enumeration is already offloaded, but this parsing is not. Files are processed serially within a scan, limiting concurrent source buffers per scan; different Vault scans can still run concurrently. Editor limits do not constrain external files discovered during registration/refresh.

**Fix and payoff:** Apply a bounded read policy before allocation; explicitly index oversized files as metadata-only without altering their source bytes. Use bounded/streaming hashing where needed and a bounded blocking parser for accepted files. Memory and CPU become proportional to a configured ceiling, and async workers remain available. Verify an oversized temporary file skips full parsing/link extraction and remains byte-identical.

### [Major] Vault tree refresh repeats full-Vault reads for every loaded folder

**Location:** `crates/otto-vault/src/engine.rs:609`; `ui/src/modules/vault/vault.svelte.ts:419`.

**What and cost:** Each directory listing fetches all notes and file paths for the Vault, builds metadata, then filters by directory. UI refresh recursively awaits this for every previously loaded directory, including collapsed ones. D loaded directories and N total notes/files yield approximately 2D full-list queries, D × N row visits, and D serial HTTP round trips. N=20,000 and D=100 implies around 200 full-list queries and 2 million row visits, despite returning only each directory's immediate entries.

**Evidence and bounds:** The queries at `store.rs:551` and `:590` filter by Vault, not parent directory. Refresh recurses on `loaded`, not `open`. Existing `(vault_id,path)` keys do not narrow a query that omits the directory predicate. Unopened folders are lazy and overlap is guarded, but work within one refresh remains O(D × N).

**Fix and payoff:** Build a directory-child index once per changed generation, or persist indexed parent-directory membership. Refresh visible/changed branches and invalidate collapsed caches for lazy reload. Repeated O(D × N) materialization becomes one O(N) rebuild plus returned-entry lookups; incremental events can narrow it further. Verify row/query counts and that collapsed branches do not each issue refresh requests.

### [Major] A single Vault save waits for whole-Vault reconciliation

**Location:** `crates/otto-vault/src/engine.rs:771`, with invalidation at `:281`.

**What and cost:** After the guarded durable write/history operation, the save awaits a full scan while retaining the note write guard. The scan walks/sorts all files, loads signatures, rebuilds resolution data, and reloads/resolves all links whenever any note content changed. It then rebuilds the switcher. For N files and E links, each save costs whole-tree O(N log N) work plus O(E) resolution rather than work for the edited note. At 20,000 files/200,000 links, every 800 ms typing pause can schedule that scale of reconciliation. Periodic visible-Vault freshness checks also rescan unchanged trees.

**Evidence and bounds:** `structure_changed` treats modified content as a changed path set (`:281`); global resolution follows at `:381`. The UI serializes saves and background kicks coalesce; unchanged bodies are not reparsed and filesystem walking is offloaded. These safeguards reduce overlap but do not remove broad work from each save.

**Fix and payoff:** Update the changed file's metadata/tags/outgoing links/switcher entry directly, preserving locking, conflict hashes, recovery history, and read-after-write behavior. Distinguish added/removed paths from content-only changes; reserve global resolution for relevant path changes and coalesce external reconciliation. Ordinary save cost becomes proportional to changed content/links instead of N+E. Verify saves do not enumerate unrelated files while adds/removals still repair incoming links.

### [Major] Interactive API scripts can freeze the entire webview

**Location:** `ui/src/lib/api/scripts.ts:79`; callers at `ui/src/lib/stores/apiClient.svelte.ts:942` and `:994`.

**What and cost:** Pre/post scripts run synchronously through `new Function` on the UI event loop. The pre-script runs before sending state and the HTTP AbortController are established. Script work N and accumulated log bytes have no execution budget; one long loop blocks rendering/navigation/Cancel for its entire duration, and a nonterminating loop never yields. HTTP timeout/cancellation and `try/catch` cannot interrupt it.

**Evidence and bounds:** The actual `runPreRequest` helper delayed a scheduled timer by about 80 ms for a deliberately finite 80 ms script in an isolated Node fixture. No infinite-loop browser test was run. Server automation scripts have separate Boa loop/recursion limits (`crates/otto-server/src/api_scripts.rs:181`), which do not protect this frontend runner.

**Fix and payoff:** Execute scripts in a dedicated Worker with elapsed-time and log/test-output budgets, terminating on timeout, Cancel, or owner disposal. Marshal request/variable mutations back as structured results; set running state before launching. UI input remains schedulable and script lifetime is bounded. Verify finite slow scripts leave controls responsive and an isolated worker can be terminated for nontermination.

### [Major] Returning to the app can reload and resume previously closed conversations

**Location:** `ui/src/lib/stores/transcript.svelte.ts:403`; resume sink at `crates/otto-server/src/routes/transcript.rs:471`.

**What and cost:** The conversation cache retains every visited session; `forget` has no call sites. Visibility return and WebSocket reconnect iterate that entire cache without a mounted/workspace filter or concurrency cap. Each resync fetches a transcript then touches the session. Touch resumes reconnectable provider sessions with saved provider IDs. H visited conversations and R resumable sessions can produce roughly 2H requests plus one workspace touch and up to R resumed agents. **Thirty visited chats, twenty suspended, and one visible pane can mean sixty requests and twenty resumes.** Cache memory also retains previously loaded pages.

**Evidence and bounds:** Callers are `ui/src/shell/App.svelte:18` and `ui/src/lib/events.svelte.ts:339`; `Conversation.resync` calls touch at `transcript.svelte.ts:229`. Lifecycle locking prevents duplicate resumes of the same ID, not simultaneous distinct IDs. The 300-turn mounted DOM limit, 64 tail slots, two-minute tail expiry, and nominal 60-turn/2 MiB response page do not bound historical cache size or process resumes.

**Amplifier:** Transcript GETs call a whole-file read/parse/fold before applying the page limit (`routes/transcript.rs:279`, `:426`; `otto-transcript/src/records.rs:41`). B bytes per transcript still costs O(B) work/transient memory for a 60-turn result. Sixty 11 MiB histories therefore process roughly 660 MiB on one reconnect. The fold is offloaded, but this load still consumes memory/CPU; live incremental tail state is not reused by these GETs.

**Fix and payoff:** Track visible/mounted consumers; resync only those sessions with bounded/coalesced requests. Background catch-up must not resume closed sessions. Evict unused conversation caches without discarding separately owned drafts; make cache policy explicit. H historical requests/resumes becomes O(visible panes). Then share version-aware incremental folded state or turn offsets to make warm transcript reads page-sized. Verify visibility/reconnect never touches closed sessions using fake providers only.

### [Minor] Upload progress lists the entire remote parent every second

**Location:** `crates/otto-connections/src/transfers.rs:266`; `crates/otto-ssh/src/sftp.rs:204`.

**What and cost:** Progress needs one staging file's size, but runs `ls -la` on its entire parent and searches the parsed listing. With U uploads, duration T seconds, and N directory entries, work is O(U × T × N), approximately U × T extra SFTP commands when probes finish in time. A scenario with 10,000 entries at 100 bytes each is roughly 1 MB per tick, or 60 MB of metadata for one 60-second upload.

**Evidence and bounds:** Each probe has a 500 ms timeout; copy phases have a 1–600 second deadline, with final publication awaited separately; transfer tracking has a 128-entry cap, cancellation, and subprocess cleanup. Probes that exceed 500 ms are repeatedly restarted, potentially wasting partial scans. These bounds make this lower priority than the stuck-operation findings.

**Fix and payoff:** Stat/long-list the exact staging file, or obtain progress from the copy process, retaining literal-path handling and authorization/cancellation. Back off failing probes. Cost becomes O(U × T), independent of parent size. Verify the fake transport receives only single-file probes.

## Measurements and deliberately lower-priority work

These are small synthetic probes, not production benchmarks or percentile claims:

| Probe | Fixture | Result and interpretation |
|---|---|---|
| Actual transcript fold library, existing debug build | 200 / 2,000 / 10,000 synthetic turns; 0.22 / 2.23 / 11.18 MB; always return 60 turns | One warm sample each: 13 / 44 / 220 ms. Supports whole-input cost; does not establish release-mode endpoint latency. |
| Transcript snapshot cloning | Same sizes, five samples each | Medians 0.09 / 0.35 / 2.32 ms. Not a separate urgent finding at these sizes. |
| Actual interactive pre-script helper in Node | Finite 80 ms CPU loop | Scheduled timer delayed 79.8 ms. Confirms synchronous event-loop blocking, not WebKit frame timing. |
| API history-shaped JSON in Node | 10 / 100 responses, each 512 KiB; five parses each | Payloads 5 / 50 MiB; parse medians 2.01 / 18.36 ms. Excludes storage, Rust work, transport, and rendering. The avoidable payload is the primary finding. |
| Backup JSON parse → stringify → Blob in isolated Chromium 149 | Single-file synthetic archive, 16 / 64 MiB; one sample each | Synchronous work 67 / 175 ms; timer delay 69 / 175 ms. Same primitives as `FullBackup.svelte:26`, not the installed Tauri WebKit. Occasional large-export work is lower priority than the findings above. |

For large backups, a raw Blob download can avoid the current parse/re-serialize pass; import parsing can move to a Worker. This is an optional follow-up, not another major finding. Archive records already stream through a per-row size check (`state_archive/schema.rs:560`); this review rejected the initial suspicion of an unrestricted whole-table fetch.

Other safeguards checked: Git graph rows and API history rows are virtualized; ordinary Git commands have timeouts; Git auto-fetch has visibility/overlap/backoff guards; Vault graph physics runs in a worker; workflow provider concurrency defaults to two; workflow history is indexed and limited; stream/result caches have explicit bounds; Redis browsing uses bounded SCAN/pipelining. These should be preserved, not replaced wholesale.

API automation report rewriting/polling is a secondary candidate: reports grow with up to 1,000 executions and are repeatedly serialized, but omit response bodies. Measure that bounded case before choosing a new persistence structure. Idle transfer polling and individual small clones similarly do not justify priority work without stronger evidence.

## Implementation and verification sequence

1. **Prevent stuck work:** script worker budgets; active-view resync; per-key connection initialization; bounded Git probes and folder requests. Use fake scripts/transports/providers and assert healthy controls/keys remain usable while another task stalls. Do not substitute larger timeouts for cancellation or isolate a lock incorrectly.
2. **Make ordinary work proportional to what changed:** Vault delta indexing, directory lookups, and oversized-file policy. Preserve atomic writes, recovery history, auth, and source files. Count unrelated reads and parser calls in synthetic fixtures rather than asserting fragile millisecond targets.
3. **Reduce payload and history costs:** metadata-only API lists, page-before-resolve Agents History, checkpoint summaries/revisions, then shared transcript caches. Keep exact detail/replay/recovery available on demand and update contracts and UI types together.
4. **Improve transfer progress:** targeted metadata probes and failure backoff. Benchmark optional backup/report refinements only if users still see delays afterward.

No new feature is required for its own sake. Useful additions are operational: visible progress with a bounded wait, meaningful Cancel/Retry, and an explicit “unknown” state when an optional probe times out. Proposed tests should prove those behaviors and bounded work, rather than aim for arbitrary “lightning fast” targets.

## Separate correctness follow-up

`ui/src/modules/workflows/WorkflowsPage.svelte:146` merges run/node fields in `applyRunSnapshot` but omits checkpoints. Depending on the opening path, polling may not populate/update the checkpoint panel. This is not included in the 12 performance findings and was not changed during the sweep.

## Delivery status

PR #45 merged after all ten checks were green on `9d6cac07701d5fa495ab1da277e357e087fcb1b2`. Final Linux workspace results: **2,849 passed, 0 failed, 66 intentionally ignored**, with strict linting and UI checks passed. CI exposed and the PR corrected Git 2.55 bisect-marker compatibility and a pre-existing test-only DashMap guard deadlock. Fourteen CodeQL annotations were individually traced and dismissed with evidence; scanner settings were unchanged.

The original feature delivery and its limits remain in [the feature review report](2026-09-13-feature-review.md). This performance report is a local review artifact, not a second PR. The installed app and daemon were left running with their original process IDs/start times; no rebuild, reinstall, deployment, or restart followed the merge.
