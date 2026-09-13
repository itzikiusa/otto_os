# Otto feature review — 13 September 2026

Review scope: **Agents, Connections, Git, Vault, API client, and Workflows**. Six separate feature agents reviewed the implementation; the coordinating review checked shared code, reproduced selected failures, and consolidated priorities. Repository baseline: `013aa77d3cbc66b446f4b486fdbf877fa4c1fc66`.

This is a code-grounded product assessment, not a production incident report or an exhaustive security audit. Findings distinguish **reproduced** behavior in isolated harnesses from **static traces** of concrete execution paths. Reproductions using extracted functions or mocked HTTP do not establish full browser behavior. Product opportunities are proposals, not requirements or defects. The findings below record the original baseline. The implementation delivery section at the end records the subsequent approved changes and verification.

**Recommended direction:** prioritize preserving user work, correct target selection, and reliable recovery before expanding the feature set. Otto already has substantial breadth; the gaps below mostly appear when users switch contexts, edit existing configuration, retry work, or resume an interrupted operation.

Priority: **P1** = address first because of lost work, unintended execution, credential exposure, or wrong-target operations; **P2** = next reliability/correctness work; **P3** = polish. These are implementation priorities, not CVSS scores. Product additions are ordered separately.

## Shared infrastructure

| ID / priority | Finding and evidence | Change and acceptance check |
|---|---|---|
| S1 · P1 | **Workspace responses can overwrite the newly selected workspace.** Select A, then B; let B finish first and A finish last. The extracted production methods yielded `currentId: B`, A's sessions, and A's restored layout. Both the response assignment and post-load layout restoration lack a generation check. [Selection](../../ui/src/lib/stores/workspace.svelte.ts#L430), [session refresh](../../ui/src/lib/stores/workspace.svelte.ts#L512). **Reproduced with deferred HTTP responses.** | Capture a selection generation and workspace ID; reject obsolete assignments, loading-state updates, and layout restores. Verify reversed responses, A→B→A, and scratch transitions. This affects the shared shell and is counted once. |
| S2 · P2 | **The API helper treats empty errors as success and discards JSON on 202.** An empty 403 with `content-length: 0` and a JSON 202 both resolved to `undefined` in the actual transpiled helper. [client.ts](../../ui/src/lib/api/client.ts#L97). A real affected path is stopping a Git review agent: the server returns 202 plus the updated Review, while the UI adopts the missing result. [Server](../../crates/otto-server/src/modules.rs#L5635), [caller](../../ui/src/modules/git/ReviewAgents.svelte#L82), [state adoption](../../ui/src/modules/git/ReviewPanel.svelte#L92). **Helper reproduced; integration statically traced.** | Check unsuccessful status before handling empty bodies; parse successful nonempty bodies regardless of 202. Preserve support for genuinely bodyless accepted responses. Test empty 403/500, JSON 202, empty 202, and 204; verify Stop keeps the review visible. |

## Agents

**Already present:** multiple providers, batch launch, scratch sessions, persistent splits/tiles, terminal/chat views, history/import/resume, attachments, tasks, outputs, handover, sharing, context preview, and attention queues. The opportunity is to make session identity and lifecycle transitions dependable.

### Fix

| ID / priority | Trigger, impact, and evidence | Recommended fix |
|---|---|---|
| A1 · P1 | **Switching chat sessions can overwrite the destination draft.** Type in A, then open B in the same pane. The unkeyed component chain reuses a composer whose text was initialized from A; its effect saves that text under B's new ID. Attachments also remain in the reused instance. [Composer](../../ui/src/modules/agents/conversation/Composer.svelte#L39), [mount](../../ui/src/modules/agents/conversation/ConversationView.svelte#L475), [session replacement](../../ui/src/lib/stores/workspace.svelte.ts#L619). **Static trace.** | Key composer state by session, preserving drafts and attachments per session. Guard delayed upload/send completions. Verify A→B→A with distinct drafts and an upload in flight. |
| A2 · P1 | **Handover can submit a brief to a plain shell.** The existing-target filter and server validation accept `kind: agent`, which includes shell sessions; delivery writes the brief and Enter into the PTY. Command lines can execute, and source archival can follow a successful write even though no agent received the handover. [Picker](../../ui/src/modules/agents/Handover.svelte#L34), [validation](../../crates/otto-server/src/routes/handover.rs#L177), [delivery](../../crates/otto-server/src/routes/handover.rs#L571). **Static trace.** | Reject plain-shell targets in both existing/new target APIs and the picker. Supporting an agent nested inside a shell requires positive identification and a supported delivery path. Add negative target tests. |
| A3 · P2 | **History pagination appends obsolete results after a filter/workspace change.** Request Claude's next page, switch to Codex, finish the new load, then finish the old page. The actual store ended with Codex selected and both Codex and old Claude rows. First-page loads have a generation guard; pagination does not. [History store](../../ui/src/modules/agents/history/history.svelte.ts#L166). **Reproduced with the transpiled store and deferred HTTP.** | Apply the same generation/scope checks to pagination, including catch/finally updates. Prevent Load more during first-page replacement. |
| A4 · P2 | **Archive can race with restart and leave a hidden running session.** Restart checks archival while holding its resume lock, then awaits setup/spawn. Archive uses no equivalent lock, can mark the row archived, and return before restart inserts a running PTY. [Archive](../../crates/otto-sessions/src/manager.rs#L3717), [restart publication](../../crates/otto-sessions/src/manager.rs#L3922). **Static concurrency trace; not reproduced.** | Serialize lifecycle mutations with a shared per-session lock and recheck terminal state before publishing a handle. Verify with a spawn barrier and concurrent archive; audit kill/remove under the same invariant. |

Also address shared finding **S1**.

### Improve or add

| Proposal | Why it helps / current gap | Completion criterion |
|---|---|---|
| **Reliable handover delivery and recovery** | Success currently establishes PTY writes, not receipt of a turn by the target. [Handover worker](../../crates/otto-server/src/routes/handover.rs#L219). | Preserve the brief; expose preparing, sent, acknowledged, and failed states; allow retry; tie optional source archival to the chosen delivery guarantee. |
| **Scratch-session parity** | History uses `ws.currentId`; scratch-only use has no history load. Split broadcasting hides when scratch panes exist. [History](../../ui/src/modules/agents/history/HistoryPage.svelte#L34), [broadcast](../../ui/src/modules/agents/Splits.svelte#L128). | Offer a No workspace history scope and same-scope scratch broadcasts, with explicit treatment of mixed workspaces. |
| **Consistent provider readiness** | The first-run coach recognizes Claude/Codex while launch supports registry providers. [Coach](../../ui/src/modules/agents/FirstRunCoach.svelte#L31). | Reuse executable/readiness checks in launch, onboarding, and handover; show actionable setup failures for custom providers too. |
| **Ordinary controls for saved views** | Mission saved views exist, but authoring starts from JSON. [MissionControl](../../ui/src/modules/agents/MissionControl.svelte#L57). | Provide provider/repository/status filters; retain JSON as an advanced option. |

## Connections

**Already present:** a unified hub for SSH, MySQL, PostgreSQL, Redis, MongoDB, ClickHouse, Custom and Kafka; sections/search/filters; DB workbench integration; SSH terminals/SFTP; TLS and tunnels; imports; resource access controls. Governed DB/Custom terminal opening is deliberately refused in current code, despite older documentation describing broader terminal access.

### Fix

| ID / priority | Trigger, impact, and evidence | Recommended fix |
|---|---|---|
| C1 · P1 | **MongoDB URI passwords are persisted in ordinary profile parameters.** Importing a URI containing dummy userinfo leaves the full URI in `conn_string`, with no separate secret; those params are serialized into SQLite. [Parser](../../ui/src/modules/connections/ConnectionForm.svelte#L272), [persistence](../../crates/otto-state/src/connections.rs#L102). **Actual form parsing reproduced; storage path traced.** | Normalize embedded credentials server-side as well as in import UI. Store passwords in Keychain and persist references/placeholders; cover direct form/API input too. Plan a careful migration for existing profiles without losing connectivity. |
| C2 · P1 | **Editing a profile drops valid configuration, including TLS shorthand.** The form rebuilds params from `{}`. Renaming a profile with `secure: true` removes the flag; supported custom placeholders and the `database` alias also disappear. [Builder](../../ui/src/modules/connections/ConnectionForm.svelte#L185), [save](../../ui/src/modules/connections/ConnectionForm.svelte#L345). **Actual builder reproduced; TLS interpretation traced.** | Preserve unedited parameters and normalize supported aliases. Remove only explicitly cleared fields. Verify a rename is lossless for TLS, custom params, and database selection. |
| C3 · P2 | **Secure URI import drops TLS.** `rediss` and `clickhouse+https` are recognized but do not set TLS fields. A new `rediss` profile stays at `tlsMode: disabled`, selecting ordinary TCP. [Import](../../ui/src/modules/connections/ConnectionForm.svelte#L256), [Redis driver](../../crates/otto-dbviewer/src/drivers/redis.rs#L431). **Parser reproduced; driver branch traced.** | Map secure schemes and supported query options to TLS settings. Surface unsupported options instead of silently dropping them. Test scheme/port/database/credential round trips. |
| C4 · P2 | **Initial SFTP failure immediately retries itself.** The effect loads while `!loaded && !loading`; failure leaves `loaded` false and clears `loading`, enabling another attempt and clearing the visible error. [Effect](../../ui/src/modules/connections/SftpBrowser.svelte#L42), [store](../../ui/src/lib/stores/sftp.svelte.ts#L87). **Static state-transition trace.** | Separate attempted from successfully loaded state; retain the error and provide explicit Retry or bounded backoff. Verify an authentication failure causes one initial attempt. |
| C5 · P2 | **SFTP forces port 22 over an SSH-config alias.** An omitted port becomes 22 and is always passed as `-P 22`. A terminal can honor alias port 2222 while SFTP fails. [Default](../../crates/otto-connections/src/http.rs#L542), [arguments](../../crates/otto-ssh/src/sftp.rs#L106). **Argument path traced; `ssh -G` reproduced inherited 2222 versus forced 22 without networking.** | Preserve an optional port and emit an override only when explicitly set. Test alias-based terminal/SFTP parity and examine tunnel defaults. |

### Improve or add

| Proposal | Why it helps / current gap | Completion criterion |
|---|---|---|
| **Test edits using the stored credential** | The edit form requires re-entering an unchanged password to test. [Form](../../ui/src/modules/connections/ConnectionForm.svelte#L320). | Test proposed settings server-side using the authorized saved secret, without returning it to the UI. |
| **Transfer progress, cancellation, and timeout** | SFTP awaits whole subprocess output and has a connection timeout, rather than full transfer lifecycle controls. [SFTP execution](../../crates/otto-ssh/src/sftp.rs#L132). | Long transfers report progress; cancel terminates the operation; stalled transfers end with a readable error. |
| **Reuse SFTP connections across browsing requests** | Each request creates a fresh session, so current multiplexing mainly helps commands within that request. [Construction](../../crates/otto-connections/src/http.rs#L569). | Use bounded session reuse with idle cleanup and authorization checks; measure browse latency across repeated navigation. |
| **Duplicate profiles and reconcile repeat imports** | Existing menus cover open/edit/delete/access; imports create rows without an update/skip matching preview. | Offer secret-safe duplication and a reviewable import preview for create/update/skip. |
| **Refresh the feature guide** | It references removed page components and outdated terminal availability. | Describe the unified hub and actual governed capabilities; verify examples against current code. |

## Git

**Already present:** persistent repo tabs, clone/register, graph/search/history/blame, file/folder/hunk/line staging, signing/amend, stashes, branches/tags/worktrees/submodules, conflict resolution, rebase/cherry-pick/revert, remotes, three PR providers, comments/CI/readiness, and agent drafting/review. Prioritize the correctness of destructive and recovery paths.

### Fix

| ID / priority | Trigger, impact, and evidence | Recommended fix |
|---|---|---|
| G1 · P1 | **Merge-and-delete can delete the wrong branch for a fork PR.** A PR from `contributor/project:feature` into `company/project` reads only `head.ref`, then deletes `company/project:feature`. An unrelated same-name base branch is at risk; the actual source remains. [GitHub merge](../../crates/otto-git/src/providers/github.rs#L764), [delete target](../../crates/otto-git/src/providers/github.rs#L778). **Static trace; no remote operation attempted.** | Carry the head repository identity and apply cleanup only to the intended source under a supported policy. Safely skip unsupported fork cleanup. Test equal branch names across different repositories. |
| G2 · P1 | **Hunk actions can affect newer content the user never reviewed.** Change `reviewed` to `unreviewed` without changing line counts after opening a diff. Both versions have the same `@@` header, so Otto accepts the old selection and builds an operation from the fresh content. The isolated Git reproduction staged `unreviewed`; discard uses the same freshness check, though it creates a backup stash. [Header comparison](../../crates/otto-git/src/patch.rs#L117), [fresh patch](../../crates/otto-git/src/patch.rs#L449). **Underlying Git behavior reproduced; implementation traced.** | Bind mutations to a fingerprint of the exact reviewed diff/preimage, including line selection. Reject changed content with 409 and require refresh. |
| G3 · P2 | **Unstaging a rename leaves its source deletion staged.** The UI sends only the destination path. The actual Git command changes `R old.txt → new.txt` to staged deletion of `old.txt` plus untracked `new.txt`; Unstage all follows the same path-list behavior. [WIP action](../../ui/src/modules/git/WipPanel.svelte#L402), [unstage](../../crates/otto-git/src/local.rs#L1394). **Reproduced in a disposable repository.** | Resolve renamed rows against fresh status and unstage both `path` and `orig_path`. Verify the index is clean after individual and bulk unstage. |
| G4 · P2 | **Reopening a fully resolved operation disables Complete.** Resolve/stage the last conflict, then reopen the resolver while merge/rebase is still active. `allResolved` requires a nonempty file list, so zero remaining conflicts cannot be completed. [Gate](../../ui/src/modules/git/ConflictResolverView.svelte#L110). **Static trace.** | Enable completion from live operation state plus absence of unresolved paths. Test reload and externally resolved conflicts. |
| G5 · P2 | **A rebase's next conflict appears as a failure without refreshing the resolver.** Continuing after one resolved commit can stop at a second conflicting commit. Nonzero Git exit propagates before returning the supported `conflicts` outcome; the UI retains the previous resolved rows. [Continuation](../../crates/otto-git/src/local.rs#L2462), [UI](../../ui/src/modules/git/ConflictResolverView.svelte#L181). **Static trace.** | Inspect operation state after continuation and return the next conflict set as an ordinary outcome. Reset resolution state even if the same file conflicts again. |

Also address shared finding **S2**, which affects stopping review agents.

### Improve or add

| Proposal | Why it helps / current gap | Completion criterion |
|---|---|---|
| **Complete non-text conflict resolution** | Current save gating expects text conflict segments; checkout/add cannot select a deleted side. [Pane](../../ui/src/modules/git/ConflictFilePane.svelte#L84), [backend](../../crates/otto-git/src/local.rs#L2400). | Provide explicit keep/delete/take-side controls for deletion-vs-modification, binary, and marker-free cases, with representative tests. |
| **Recovery history** | No general reflog browser/recovery endpoint was found; current reflog use relates to stashes. | Inspect reflog entries and create a recovery branch without rewriting existing history. |
| **Interactive rebase controls** | Ordinary rebase and continue/abort exist; dedicated skip/reorder/squash/edit planning was not found. | Preview a commit plan and expose safe continuation/abort, including successive conflicts. |
| **Bisect assistant** | No Git bisect routes/UI were found. | Select good/bad revisions in the graph, show the current candidate, and support resume/abort. This is a later addition after recovery fixes. |

## Vault

**Already present:** portable Markdown folders, Obsidian links/embeds, tabs/autosave, search/tags/backlinks, graph filters, OKF validation/index generation, attachment/OpenAPI viewers, MCP access, and durable multi-provider author/reviewer runs. Draft preservation and safe concurrent editing deserve immediate attention.

### Fix

| ID / priority | Trigger, impact, and evidence | Recommended fix |
|---|---|---|
| V1 · P1 | **Navigation can discard a draft after a failed or conflicted save.** Edit A, get 409, then open B: saving absorbs the error and navigation replaces the draft, clearing dirty/conflict state. An existing conflict bypasses saving entirely. Closing the last dirty tab also removes the save target. [Navigation](../../ui/src/modules/vault/vault.svelte.ts#L421), [save](../../ui/src/modules/vault/vault.svelte.ts#L627), [close](../../ui/src/modules/vault/vault.svelte.ts#L478). **Actual transpiled store reproduced:** B open, dirty/conflict false, A draft not preserved. | Keep drafts by vault/path; return explicit save outcomes and await in-flight saves. Preserve failed drafts until successfully saved or deliberately resolved/discarded. Cover note/vault switches and closing the last tab during debounce. |
| V2 · P1 | **Concurrent Markdown writes can both pass the same hash guard.** Both writers read the same old bytes before writing; serialization begins after the file mutation. Two updates with one `if_hash` both succeeded in 1 of 20 synchronized attempts, losing one accepted update. [Markdown write](../../crates/otto-vault/src/engine.rs#L693). **Reproduced against the built VaultEngine in a temporary vault.** | Serialize the read-check-write boundary and use atomic replacement, adapting the existing [text-artifact writer](../../crates/otto-vault/src/engine.rs#L752). Include rename/delete in the mutation policy. Require exactly one successful update and one conflict for competing same-base writes. |
| V3 · P1 | **Soft deletion can overwrite an earlier trashed version.** Repeated deletion of the same path within one second reuses a timestamp suffix after the base trash name exists; rename replaces that destination. [Trash allocation](../../crates/otto-vault/src/engine.rs#L787). **Reproduced against VaultEngine with the legitimate collision state pre-seeded:** deleting version three replaced saved version two. | Allocate collision-resistant names and use no-replace semantics with retry. Retain original path/deletion metadata. Verify every deleted version survives repeated same-path deletions. |
| V4 · P2 | **A read can return current content with an old hash and metadata.** External editing before the next scan makes `note()` combine fresh disk bytes with indexed metadata. Saving against the returned hash immediately conflicts despite no intervening change. [Read](../../crates/otto-vault/src/engine.rs#L659). **Reproduced against VaultEngine:** new raw, old hash, then 409. | Derive raw, hash, metadata, and links from one content version, or synchronously refresh changed content. Test external edit→read→save without an intervening scan. |
| V5 · P2 | **Scans do not refresh the displayed note reliably.** Polling refreshes tree/tags/backlinks but never reloads the open note; unchanged counts can also hide content-only changes if scanning occurs between polls. [Polling](../../ui/src/modules/vault/vault.svelte.ts#L272), [reader](../../ui/src/modules/vault/NoteView.svelte#L62). **Store harness reproduced zero note reads and old displayed content after an observed scan transition; count limitation traced.** | Publish a content/index generation or changed-path signal. Refresh clean notes and affected caches; preserve dirty drafts and show external-change state. Test title/tag/body/attachment changes with unchanged counts. |

### Improve or add

| Proposal | Why it helps / current gap | Completion criterion |
|---|---|---|
| **Trash browser and Restore** | Soft deletion exists; no restore route/UI workflow was found. | Browse deletion time/original path and restore without overwriting a newer live file. Build on V3. |
| **Reviewable agent edit history** | Agent runs preserve sessions, findings, and changed paths, but that is not content version history. | Inspect before/after diffs and restore individual edits with concurrency protection. |
| **Expose the local graph** | The graph component/backend already support neighborhoods; the page mounts only the full graph. [Mount](../../ui/src/modules/vault/VaultPage.svelte#L352). | Offer Graph around this note and full/local switching using the existing implementation. |
| **Editable structured properties** | The current Properties panel is read-only. | Edit common YAML fields while preserving unknown properties and providing a source preview. Use the repaired save lifecycle. |

## API client

**Already present:** HTTP/GraphQL, SSE/WebSocket, unary/server-streaming gRPC, Keychain auth/environments, scripts, import/export and Postman account sync, SSH tunneling, request history, automations, and saved-request MCP tools. **Local/private target opt-in already exists**; it is not a missing feature. The older blanket localhost limitation in the guide is stale.

### Fix

| ID / priority | Trigger, impact, and evidence | Recommended fix |
|---|---|---|
| API1 · P1 | **Saving A can attach A's saved identity and auth to tab B.** Save A, switch to B before completion. The handler updates the current draft, leaving B's URL/body but adding A's ID/auth/name. Saving B then overwrites saved A. [Save completion](../../ui/src/lib/stores/apiClient.svelte.ts#L786). **Actual transpiled store reproduced with deferred HTTP:** B's URL plus `requestId: saved-a` and A's secret reference. | Capture workspace and stable tab identity before awaiting; update the originating tab only. Apply ownership checks to related async loads/responses. Verify delayed Save A→switch B→Save B preserves both requests. |
| API2 · P1 | **Runtime variables cross workspace boundaries.** A login script in workspace A sets `token` or `base_url`; switching to B restores tabs but retains singleton runtime variables. They override B's environment on send, risking wrong credentials or target selection. [Restore](../../ui/src/lib/stores/apiClient.svelte.ts#L350), [send](../../ui/src/lib/stores/apiClient.svelte.ts#L928), [override precedence](../../crates/otto-server/src/routes/api_client.rs#L1765). **Store retention reproduced; send/override traced.** | Scope runtime variables by workspace and confine delayed script results to their originating scope. Verify identical variable names in A and B never cross. |
| API3 · P2 | **Clearing the last script or nondefault setting does not clear the saved value.** The UI sends `extras: null`; PATCH treats that as omitted and retains the old extras. A deleted script reappears on reopen and remains active in automation. [Serialization](../../ui/src/lib/stores/apiClient.svelte.ts#L150), [PATCH](../../crates/otto-server/src/routes/api_client.rs#L638), [DTO](../../crates/otto-core/src/api.rs#L2649). **Serializer reproduced; persistence semantics traced.** | Send an explicit empty extras object or distinguish absent from explicitly cleared PATCH values. Test script deletion and returning the last timeout/TLS/redirect setting to default. |
| API4 · P2 | **Streaming ignores configured auth, variables, and other visible request fields.** Connect a WebSocket with Bearer auth or a `{{base_url}}` URL. The relay receives literal URL/headers/body but no auth/environment resolution; query rows, settings, and SSH selection are also omitted. [Builder](../../ui/src/modules/api/RequestBuilder.svelte#L533), [client](../../ui/src/lib/stores/apiStream.svelte.ts#L78), [wire DTO](../../crates/otto-server/src/routes/api_stream.rs#L49). **Complete static caller-to-relay trace.** | Make streams workspace-aware and reuse server request preparation, including Keychain/environment resolution. Hide or clearly mark unsupported transport settings. Verify authenticated SSE/WS with variable/query expansion in isolated fixtures. |
| API5 · P2 | **A delayed old socket close disables a new stream.** Reconnect before the previous socket's close callback arrives; that callback unconditionally nulls the shared socket and marks the replacement closed. [Callback](../../ui/src/lib/stores/apiStream.svelte.ts#L102). **Actual store reproduced with two fake sockets.** | Guard all callbacks by socket identity/generation and detach old handlers. Verify old close/error/message events cannot alter the current stream. |
| API6 · P2 | **The streaming relay authenticates but does not enforce workspace Editor authorization.** The root-mounted handler accepts a valid token, discards the returned identity, and upgrades; neither its state nor open message carries workspace authorization context. A viewer can request outbound streaming operations contrary to the editor-only contract. [Handler](../../crates/otto-server/src/routes/api_stream.rs#L87), [mount](../../crates/otto-server/src/modules.rs#L7419), [contract](../../docs/contracts/api.md#L2230). **Security path statically traced; no daemon exploit attempted.** Existing destination guards still restrict targets. | Require workspace context and Editor authorization before opening an upstream connection; enforce relevant API feature policy too. Verify no upstream attempt for a viewer/nonmember and a successful authorized fixture. |
| API7 · P2 | **Script-result metadata bypasses secret scrubbing.** Saved execution resolves environment secrets for scripts. A test named from `pm.environment.get('secret')`, or an error containing that value, enters `script_tests`/`warnings` verbatim; only the response object is scrubbed before the final result, including agent-shaped results. [Script output](../../crates/otto-server/src/routes/api_client.rs#L1679), [result](../../crates/otto-server/src/routes/api_client.rs#L1740). **Security source-to-result trace; no real secret used.** | Scrub the entire returned result, including test names, warnings, and script errors, using the resolved secret set. Verify with a dummy secret in each metadata/error field for human and agent result shapes. |

Also address shared finding **S2**. API6 and API7 are focused security findings beyond the API reviewer's correctness pass, checked during consolidation; this was not a comprehensive authorization audit.

### Improve or add

| Proposal | Why it helps / current gap | Completion criterion |
|---|---|---|
| **Durable automation reports** | The runner returns in-memory results, steps bypass ordinary history recording, and UI keeps only `lastRun`. [Runner](../../crates/otto-server/src/routes/api_client.rs#L2529), [UI](../../ui/src/lib/stores/apiClient.svelte.ts#L1154). | Persist run/step IDs, status, selected environment, and provenance; inspect failures after reload with secrets redacted. |
| **Automation execution controls** | Execution uses the active environment and attempts each step without run options. [Route](../../crates/otto-server/src/routes/api_client.rs#L2506). | Select environment explicitly, support stop-on-failure and cancellation; add dataset iterations later if needed. |
| **Reusable saved gRPC requests** | Current extras omit the proto/schema and selected method; loading resets them. [Extras](../../ui/src/lib/stores/apiClient.svelte.ts#L132), [load](../../ui/src/lib/stores/apiClient.svelte.ts#L1194). | Persist method and a reusable schema attachment/reference so reopen restores a runnable request. |
| **OAuth authorization-code flow with PKCE** | Current grant choices cover client credentials, password, and refresh token. [Builder](../../ui/src/modules/api/RequestBuilder.svelte#L1069). | Support browser-based user consent, callback handling, and Keychain persistence for APIs requiring interactive login. |

## Workflows

**Already present:** graph authoring, templates/AI generation, conditional branches, bounded loops, agent/review/product/git nodes, retries, approvals, schedule/event/webhook/chat triggers, queued execution, restart recovery, context handoffs, live sessions, history, and proof packs. Previously stubbed product/review nodes are wired. The highest-value fixes are in continuation semantics.

### Fix

| ID / priority | Trigger, impact, and evidence | Recommended fix |
|---|---|---|
| W1 · P1 | **Restart recovery can skip unfinished sibling branches and report success.** For Start→A and Start→B, restart while A runs and B is pending. Recovery scopes execution to A's descendants; B becomes outside-scope/Skipped, and finalization can still report Success. The between-node recovery path has the same limitation. [Recovery](../../crates/otto-server/src/workflow_engine.rs#L572), [scope](../../crates/otto-server/src/workflow_engine.rs#L1337), [skip](../../crates/otto-server/src/workflow_engine.rs#L1754), [finish](../../crates/otto-server/src/workflow_engine.rs#L2305). **Static trace.** | Preserve the original execution scope and resume all its unfinished nodes while adopting completed work and branch decisions. Separate restart continuation from Run from here. Test restart with pending siblings and joins. |
| W2 · P1 | **Restarting a loop can replay completed external actions.** A loop sends a notification or mutating HTTP request, then waits. The outer loop is classified restart-safe, but restart begins again at iteration 1, repeating the action. [Allowlist](../../crates/otto-server/src/workflow_engine.rs#L446), [iteration reset](../../crates/otto-server/src/workflow_engine.rs#L3868), [inner execution](../../crates/otto-server/src/workflow_engine.rs#L3940). This contradicts the [no-replay contract](../../docs/contracts/api.md#L1671). **Static trace; no external action executed.** | Persist inner step/iteration checkpoints and classify the interrupted inner operation. Until that exists, conservatively stop automatic replay of loops containing external mutations. Test with a local recorded side effect, never real notifications. |
| W3 · P2 | **Retry loses the adopted predecessor's output.** Retry a failed consumer after a successful fetch. The fetch output is adopted, but its incoming edge is filtered out because the fetch is outside the execution scope; the consumer receives original run input instead. [Edge filter](../../crates/otto-server/src/workflow_engine.rs#L1784), [retry entry](../../crates/otto-server/src/routes/workflows.rs#L604). **Static trace.** | Separate execution scope from input/control dependencies. Existing-run retries must consume adopted outputs and reconstructed edge conditions; keep run-input fallback for genuinely fresh partial runs. Test differing run input and upstream output. |
| W4 · P2 | **Max retries = 0 actually enables the agent default.** The UI serializes zero as `retry: null`; the backend interprets absence as two retries for agent steps. [UI](../../ui/src/modules/workflows/WorkflowsPage.svelte#L1168), [policy](../../crates/otto-server/src/workflow_engine.rs#L5673). **Static trace.** | Preserve an explicit zero policy; represent Use default separately and display the effective count. Verify zero gives one initial attempt with no retries. |
| W5 · P2 | **The default Event trigger cannot be saved.** The form supplies `ReviewChanged`; validation accepts `review_changed` and other snake_case identifiers. [Default](../../ui/src/modules/workflows/TriggersPanel.svelte#L60), [validator](../../crates/otto-server/src/routes/workflows.rs#L1348). **Static trace.** | Use supported event choices with readable labels and canonical values. Verify the default form saves and fires for a matching isolated event. |

### Improve or add

| Proposal | Why it helps / current gap | Completion criterion |
|---|---|---|
| **Run version pinning** | Retry/restart load the current workflow even though snapshots exist. [Retry](../../crates/otto-server/src/routes/workflows.rs#L595), [restart](../../crates/otto-server/src/workflow_engine.rs#L648). | Default continuation to the run's original definition; make applying a new version an explicit, reviewable choice. Pair with W1–W3. |
| **Preflight graph validation** | Create/update persist graphs without structural/required-parameter/expression validation; output schemas are warn-only. [Create/update](../../crates/otto-server/src/routes/workflows.rs#L56). | Validate before execution, highlighting broken nodes/edges and actionable errors. Distinguish errors from intentional dynamic values. |
| **Complete trigger authoring** | Backend cron, timezones, filters, and result destinations exceed the current form's interval/daily/weekly/minimal event fields. [Form](../../ui/src/modules/workflows/TriggersPanel.svelte#L54). | Edit existing specs, expose supported controls, and preview next fire times. Extend the repaired W5 control. |
| **Inner-step retries and visibility** | Loop steps deserialize retry policy but execute directly, bypassing the outer retry wrapper. [Loop](../../crates/otto-server/src/workflow_engine.rs#L3920). | Reuse attempt execution semantics and show inner attempts/checkpoints in the run view. Coordinate with loop replay safety. |

## Consolidated delivery order

The report contains **33 distinct fix findings: 14 P1 and 19 P2**, plus product opportunities. Shared defects are counted once. The number is not a measure of feature quality: review depth, feature size, and reproduction opportunities differ.

| Order | Work package | Findings / dependency | Exit criterion |
|---|---|---|---|
| **1** | **Prevent lost work and wrong-target actions** | A1/A2, S1, C1/C2, G1/G2, V1/V2/V3, API1/API2, W1/W2. These form the first delivery tranche and can be split by feature. | Every reported trigger has a failing-before/passing-after regression. No draft moves to another session/tab; no profile rename changes TLS; no wrong-repository deletion; no accepted Vault write/trash version disappears; no restart silently skips/replays work. |
| **2** | **Make recovery and execution semantics consistent** | S2, A3/A4, C3/C4/C5, G3/G4/G5, V4/V5, API3/API4/API5/API6/API7, W3/W4/W5. Pin run versions alongside workflow continuation fixes. | Retry consumes the same inputs; zero retries means zero; errors remain visible; streaming honors authorized configuration; reload/continuation preserve coherent state. |
| **3** | **Expose recovery and validation to users** | Vault Trash/Restore and agent diffs; Git non-text conflict support and recovery history; workflow preflight; durable API automation reports; reliable handover status. | Users can understand a failure, inspect what happened, and recover through the UI without guessing at hidden state. |
| **4** | **Expand useful breadth** | Trigger-editor parity, credential-aware connection testing, cancellable SFTP, scratch/provider parity, local Vault graph/properties, reusable gRPC, OAuth PKCE, then interactive rebase/bisect as demand warrants. | Each addition completes a documented user journey and includes realistic failure behavior, rather than only exposing a new control. |

Within order 1, **G1, A2, C1, V1, and W2** deserve early attention because they respectively risk unrelated branch deletion, shell execution, credential persistence, draft loss, and repeated external actions. Order 2 permission/redaction fixes should ship with the related API work; the table is a dependency-oriented grouping, not a reason to delay a small important fix.

Three shared engineering changes would prevent recurrence:

1. **Carry stable operation identity across awaits.** Use workspace/tab/session identity plus generations in the shell, agent History, API saves, and streams. Check success, error, and finalization paths. Do not make unrelated domain stores one global abstraction.
2. **Treat save and retry outcomes explicitly.** Distinguish success, conflict, failure, cancellation, and unknown outcome. Preserve the user's draft and the operation's original inputs until the outcome is resolved.
3. **Test interruptions and recovery as normal user journeys.** Use deferred HTTP/socket responses, temporary repositories/vaults, and local recorded side effects. Add restart barriers and concurrent-write checks, rather than relying on happy-path rendering tests.

## Baseline review validation and limits

| Verification | Result | What it establishes |
|---|---|---|
| `npm run check` in `ui/` | Passed; Svelte reported **0 errors, 0 warnings**, and all chained TypeScript checks exited successfully. | Current static UI/type baseline. It does not detect the state-machine and request-ordering defects above. |
| `npm run test:unit` in `ui/` | **25 passed, 0 failed.** | Existing session ordering and split-layout logic. This small suite is not comprehensive feature coverage. |
| Connections/SSH targeted Rust tests | **81 unique tests passed:** 48 Connections unit, 15 SSH unit, 18 Connections integration. | Existing tested behavior in those crates. No real connection or SFTP transfer was performed. |
| `cargo test -p otto-vault --lib --tests` | **43 passed:** 25 unit, 16 engine, 2 HTTP. | Existing Vault behavior; additional isolated engine reproductions exposed concurrency, trash, and freshness defects. |
| Deterministic UI harnesses | Reproduced shared API response handling and workspace ordering; agent History pagination; API tab-save/runtime-variable/socket races; Vault navigation/freshness issues. | Executed actual transpiled code or extracted production methods with mocked transports/reactivity. These are not full Svelte browser reproductions. |
| Disposable Git/config fixtures | Reproduced rename unstage and unchanged-header hunk behavior; `ssh -G` confirmed alias-port override. | Actual local command semantics without touching user repositories or connecting remotely. |
| Vault engine fixtures | Reproduced concurrent same-base writes, content/hash mismatch, and trash collision using temporary files and an in-memory database. | Actual engine behavior under the stated conditions; concurrency reproduction was observed once in 20 synchronized attempts. |

At the original review stage, not yet run: full workspace build/test/clippy, production UI build, desktop/browser E2E, actual provider launches, live Keychain transport flows, real SSH/DB/streaming connections, or real daemon restart experiments. Workflow recovery findings remain static traces. That review stage made no remote writes, production mutations, commits, or application-source edits. The subsequent authorized implementation and its verification are recorded below.

Recommendations come from the current implementation and inspected feature/contracts/tests, not competitor research or usage analytics. Before assigning dates, size the selected work packages and choose which product additions match actual usage. Whenever a fix changes endpoint payloads or events, update authoritative contracts and TypeScript types together.


## Implementation delivery

Implementation branch: `feature/review-improvements-20260913`. All 33 findings and the 25 feature proposals above have corresponding local implementations. Local integration verification is complete; the final results and operating limits are recorded below.

| Area | Fixes delivered | Added or improved user flows |
|---|---|---|
| Shared infrastructure | Workspace selection/refresh generations prevent obsolete sessions and layouts from publishing; HTTP errors are checked before empty bodies and JSON 202 responses retain their result. | Shared request-ownership regression harness and atomic session metadata replacement. |
| Agents | Session-owned drafts, images, and in-flight send claims; shell handover rejection; History pagination ownership; serialized restart/archive/kill/remove/suspend/unarchive. | Scratch History/broadcast; consistent executable readiness including custom templates; normal saved-view filters with advanced JSON; durable handover brief, failure/retry panel, and explicit receipt confirmation before source archival. |
| Connections | Mongo URI credentials move to secret storage; edit forms retain unknown fields and TLS; secure URI import semantics; explicit SFTP retry; omitted SSH ports inherit SSH config. | Credential-aware proposed-edit testing; transfer progress/cancel/timeouts and finalization state; bounded authorized SFTP reuse; credential-free duplication and import reconciliation; updated hub/governance documentation. |
| Git | Fork cleanup scope, byte-exact hunk fingerprints, rename unstaging, empty conflict completion, successive rebase conflicts. | Non-text conflict controls; reflog recovery branches; interactive reorder/squash/edit/skip; persistent bisect assistant. |
| Vault | Dirty drafts survive failed navigation saves; atomic guarded writes; collision-safe trash; coherent note reads; generation-based refresh. | Trash browser/restore; before/after history and guarded restore; agent edit review; full/local graph switch; structured YAML properties editing. |
| API client | Saved-tab ownership, workspace-scoped runtime values, nullable extras clearing, prepared authenticated streams, socket identity, stream authorization, and diagnostic secret redaction. | Durable automation run/step reports and request history; explicit environment, dataset, stop-on-failure and cancel controls; saved gRPC schema/method; browser OAuth authorization-code + PKCE with Keychain persistence. |
| Workflows | Continuation preserves original scope and adopted inputs; interrupted loop side effects use durable checkpoints; explicit zero retries and canonical event values. | Run version pinning; graph preflight; complete trigger editing/preview; inner-step attempt/checkpoint visibility and atomic retry preparation. |

Independent integration review also repaired metadata replacement losing its durable object between SQL writes, duplicate sends after composer remount, stale Mission Control responses, Vault recovery results crossing vault selections, Git mutation locks dropping on HTTP cancellation, SFTP transport/profile snapshot races, and OAuth credential-write/callback races.

### Shared folder picker addition

New Session uses the same `FolderPicker` component as the other folder/file selectors. That component now includes Back, Forward, Up, and clickable ancestors, while retaining current-listing search and hidden-file controls. Successful navigation alone advances history; failed loads expose Retry and prevent selecting stale results. Long paths scroll inside the picker. Favorites support adding/removing folders (including unavailable favorites), and Recents retain the last 20 distinct successful folder visits. Shortcuts persist locally per daemon/user and share the same permission-checked browse path.

### Backup, Git sync, and connection export additions

Settings → Backup & Restore now separates the legacy settings-only transfer from three new flows:

| Flow | Delivered behavior | Boundaries |
|---|---|---|
| Otto data archive | Saved feature records, workflow definitions/versions, scheduled tasks, connection profiles, API configuration, session/history records, managed assets, and registered Vault documents in format-2 JSON. Preview lists conflicts, exclusions, and reconnect requirements before an explicitly confirmed restore. | Credentials/auth grants, running processes, external repository working trees, external database contents, and derived caches are excluded. Restore adds missing items without overwriting existing ones; imported automatic activity stays inactive. |
| Git snapshots | Deterministic readable `.otto-sync/` configuration/documents; preview and write; snapshot-only commits preserving unrelated staged work; explicit fetch, fast-forward pull, and push; previewed additive import. | Git snapshots exclude stored credentials and runtime history. Push sends the current branch history. Multi-file publication is not crash-atomic; inspect uncommitted changes after interruption. Existing Otto records are not updated by Git import. |
| Connection export | All connection types in JSON/CSV, or compatible MySQL Workbench, DBeaver MySQL/MongoDB, NoSQLBooster URI list, and RedisInsight formats. All workspaces or selected workspaces plus global connections. | Passwords are opt-in. Workbench receives a separate credentials JSON because its connection XML does not transport passwords. Unsupported native options are listed as skipped; prepared credential files are never sent to Git automatically. |

Full archives are limited to 256 MiB encoded, with 64 MiB individual assets, and currently require matching schema migration levels. Database rows share one snapshot; files use individual change detection rather than a filesystem-wide snapshot. Imported users remain disabled, and relocated Vault files are reindexed. Documents and historical text remain private content even when structured credentials are filtered.

See [Backup and restore](../features/backup-restore.md), the [archive inventory](../features/state-archive.md), and [connection formats and primary references](../features/connections-ssh-sftp.md#exporting-connections-to-another-tool).

### Scope and operating limits

- Legacy Mongo URI credentials normalize lazily when Connections profiles are read/listed; this is not a startup bulk migration.
- SFTP transfer jobs are held in memory and do not resume after daemon restart. Copy timeouts are 1–600 seconds; publication is not cancellable, uncertain publication is reported as `outcome_unknown`, and partial-file cleanup is best effort. Saved-secret tests and governed local transfer paths require root.
- Interactive rebase planning supports linear history; merge-containing ranges are rejected before mutation.
- Vault history begins with the new guarded write paths and link rewrites. It does not reconstruct older versions or capture every unmanaged filesystem edit. Agent edit review filters by run paths and time; it is not exclusive actor attribution.
- Handover “sent” means PTY submission. The operator confirms receipt before requested source archival. An interrupted submission with an unknown outcome is inspected before retry.
- Workflow nested loops remain unsupported and preflight explains that limit. Unknown interrupted external mutations stop for explicit operator retry; continuation uses the original saved workflow version. To use the latest definition, start a fresh run; an existing run is never repinned.
- SSE uses the HTTP preparation path, including supported SSH and TLS settings. WebSocket settings that the transport cannot honor are rejected explicitly rather than ignored.
- API automation cancellation cannot retract a request already received upstream. Completed steps persist; daemon restart marks unfinished API runs interrupted without automatic replay. Dataset values stay in memory; reports carry row indices and redacted request provenance.
- OAuth browser flows expire after ten minutes; restarting the daemon requires beginning authorization again. Received tokens are retained in Keychain, not browser flow state.

### Verification

`cargo test --workspace --no-fail-fast` passed **2,849 tests with 0 failures and 66 ignored tests** across 86 runtime test groups. All 32 documentation-test targets succeeded (no runnable examples). Strict workspace/all-targets Clippy, `cargo fmt --all --check`, and `git diff --check` passed. The ignored cases require explicitly provisioned fixtures or are manual benchmarks.

Completed UI checks report **0 errors and 0 warnings**, **70/70 unit tests**, and a successful production build. Route inventory and authorization coverage pass **3/3** checks. Archive backend tests pass **7/7**; connection-export serializers pass **11/11** and isolated credential-store tests **2/2**. Git snapshot backend regressions pass **12/12**, including real registered-Vault idempotence, byte preservation under inherited Git attributes, stale previews, symlink rejection, staged-work isolation, and local bare-remote synchronization.

The original feature delivery passed **52 distinct isolated browser cases** across Agents, API, Connections, conversation/session flows, folder navigation, Git, Vault, and Workflows. All three added archive/Git/connection-export browser journeys passed together, bringing coverage to **55 distinct browser cases**. Connection export also passed a mobile prepare/download flow; the shared folder picker passed mobile viewport and cross-window shortcut checks.

A synthetic SFTP subprocess benchmark measured 20 samples per path. New/cached acquisition median was 2.000/1.888 ms (p95 2.254/2.071 ms); browse median was 17.850/17.305 ms (p95 19.513/19.382 ms). This fixture does not include SSH handshakes or real network latency and does not establish a real-host speedup.

Actual vendor-application imports, live SSH/DB connections, provider launches, real remote authentication/publishing, and installed macOS application packaging were not exercised. Native connection formats are checked against primary vendor documentation/source and isolated serialization fixtures.

 Focused Rust/UI regressions and isolated browser journeys were run throughout implementation. Failures found during integration were corrected before the final green gates above. No production database, user SSH host, or remote repository is used by the new fixtures.
