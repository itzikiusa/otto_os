# E16-auth — iteration 01

**Verdict: Block — two major/P1 findings. Scope score: 8.0/10; 9.8 is not established.**

Reviewed 2026-10-18 on `fix/db-parity-session-consistency`, HEAD `c009da20cc113c8fb79f3bf0600442d0c6afa358`. This is an existing-subsystem source review, not a changed-lines-only verdict. Concurrent edits were present in Git local execution, sessions, workflows and swarm; citations refer to the working files read during this pass. No production edits, builds, tests, remote Git operations, or git mutations were performed. This report is the only file written by E16.

## E16-01 — P1: bound Git token follows an externally changed transport destination

**Confirmed by source trace; not executed.** Primary locations: `crates/otto-git/src/http.rs:349-358`, `crates/otto-git/src/local.rs:5258-5267`.

The account ownership check is necessary but does not constrain where its credential is sent. `optional_token` obtains the bound account through `authorized_repo_account` and immediately reads its secret. It neither refreshes nor validates a transport destination. `repo_push` calls this at `http.rs:1940`, then `push_with` constructs AskPass at `local.rs:3450-3454`. AskPass answers every non-username prompt with `OTTO_GIT_TOKEN`; its interface receives only a token, with no permitted destination. Its environment also resets other credential helpers (`local.rs:5284-5304`). `run_remote` and `run_remote_raw` repeat the same behavior (`local.rs:3753-3787`); fetch uses `git fetch --prune` (`local.rs:3617-3619`).

Concrete input and result:

1. A workspace repo is bound to its owner's GitHub account; origin initially points to `https://github.com/team/app.git`.
2. An external Git configuration edit changes origin to `https://other.example/team/app.git`, or leaves origin untouched and sets `remote.origin.pushurl` to that destination.
3. The owner or root invokes Otto Push. Workspace authorization and account ownership succeed. The bound account remains attached and its PAT is supplied to Git.
4. The destination challenges for HTTP credentials. The password prompt names the new host, but AskPass does not inspect that host and returns the original PAT. The credential can therefore be disclosed to the new destination even if the push ultimately fails.

This does not require bypassing the feature guard or impersonating another account. It is a destination-confusion bug on an authorized user's operation. Changing `url.*.insteadOf`, `pushInsteadOf`, or a branch's effective remote also means checking only the stored origin would be insufficient. `provider_ctx_with_account` refreshes origin and invokes a provider check (`http.rs:380-410`), but push/pull/fetch do not call that helper. A successful refresh alone also retains the binding (`http.rs:1451-1467`).

**Fix direction:** carry a destination-bound credential to the Git transport seam, resolve the effective target for the actual operation, and confine credential responses to approved scheme/authority/provider mappings. Cover push URLs, rewrites and alternate remotes, and prevent a configuration change between preflight and credential response from defeating the check. Preserve valid root/owner operations, user-selected accounts, SSH-agent operations and supported enterprise mappings. Do not solve this by banning remote edits or disabling privileged Git.

**Required regression:** isolated repository plus dummy token and a local challenge server (no real secret). Change origin, pushurl, rewrite and alternate push remote independently; assert the unrelated endpoint never receives the dummy token. Include same-destination positive controls, root and owner, non-owner refusal, SSH without an Otto token, and legitimate enterprise host mapping. Existing `http_tests.rs:280-333` verifies ownership/unbound handling, while `local.rs:6035` checks AskPass usernames/helper reset; those tests do not establish destination confinement. Removing a destination check would leave those existing assertions unchanged.

## E16-02 — P1: changed GitLab origin can send provider actions to the previous instance

**Confirmed by source trace; not executed.** Primary location: `crates/otto-git/src/providers/mod.rs:288-290`.

`check_remote_reachable` accepts every GitLab remote whenever the account has any nonblank API base. `provider_ctx_with_account` refreshes the remote but retains the bound account (`http.rs:380-410`). Detection converts the remote into an owner/repo pair, discarding its host (`providers/detect.rs:10-46`). The GitLab client then constructs requests exclusively from the account API base (`providers/gitlab.rs:54-71`).

Concrete input and result:

1. A bound account uses `https://gitlab.old.example`; the repo originally points there.
2. The user moves origin to `https://gitlab.new.example/team/app.git`, retaining the existing account binding. Both instances have a `team/app` project.
3. A provider read/action refreshes origin and detects GitLab. Account provider matches; the nonblank old API base causes the reachability check to return success.
4. The resulting client still sends `/api/v4/projects/team%2Fapp/...` to the old instance. For a comment on MR 7, `http.rs:3000-3011` resolves this client and calls its comment method. Create/update paths likewise build requests against the old base (`providers/gitlab.rs:547-593`). A valid token and matching project/MR can make the wrong-resource action succeed instead of returning a mismatch error.

This finding is wrong-project action routing, not a claim that this provider path sends its token to the new host. The token remains on its configured API origin. Same-origin redirect handling in the GitLab HTTP client is beneficial but cannot repair wrong resource identity before the request.

**Fix direction:** validate the remote-to-API forge identity before reading a secret or calling the provider. Preserve explicitly supported SSH/API hostname differences: the comment at `providers/mod.rs:280-282` deliberately allows these. Persist an explicit trusted remote/API mapping or bind identity at account selection, and require re-selection/reconfirmation when that identity changes. Blind hostname equality would regress legitimate installations; automatically sending the old credential to the new origin would create a disclosure bug.

**Required regression:** two fake GitLab instances with the same project path/MR number. After changing the live origin, assert provider resolution rejects the stale mapping and neither instance receives a write. Positive controls must include the original matching instance, public GitLab aliases, and an explicitly configured split SSH/API mapping. Current `providers/mod.rs:435-447` tests absent/matching/blank bases; it does not test a nonblank mismatched base. A guard that accepts any nonblank base therefore satisfies those assertions.

## Broader coverage and sound paths

- Read the RBAC role checker, deny-wins resource evaluation, auth cache read/publication/revocation structure, token authentication branches, password verification, server authentication middleware, central feature/scoped credential guard, user/PAT/grant and impersonation handlers, plus relevant API contracts and selected tests.
- The protected router applies authentication outside the feature guard (`otto-server/src/lib.rs:186-195`). Effective identity enters `AuthUser`; real identity remains available through `CurrentAuthContext`. Share and MCP restrictions precede the normal feature-policy path.
- Root remains an explicit bypass in resource authorization (`otto-rbac/src/resource_access.rs:99-104`) and workspace role checking. These are intended capabilities, not findings. Owner-or-workspace-admin session access is centralized; no recommendation here removes it.
- Resource policy evaluates current group membership, deny wins, and broad operations cannot evade child denies by passing no child. Parent discovery can expose a resource when an allowed child survives. Selected resource-access tests exercise actual policy/group storage rather than only a mocked permission result.
- Managed token authentication checks the originating session and owner and avoids caching that liveness decision (`otto-rbac/src/tokens.rs:298-319,514-521`). Share and impersonation return before ordinary cache insertion. Cache publication uses invalidation history to avoid repopulating revoked entries; storage is bounded at 4096 entries. These are meaningful protections.
- PAT minting rejects impersonation and non-human credentials (`routes/auth_routes.rs:283-297`). User disable/password-change paths revoke outstanding credentials. Impersonation start blocks self/nesting/root/admin targets; stop is intentionally self-scoped so exiting the overlay remains possible. Root's personal credential retains its administration paths.
- The Git account owner/root check remains required. Valid provider destination checks must complement it, not replace it. Submodule update intentionally avoids offering the parent account token (`local.rs:2199-2211`).

## Architecture, performance and verification assessment

The concrete structural issue underlying E16-01 is loss of destination information at `Option<String>`/`AskPass::new(token)`: adding transport calls inherits the same unscoped credential behavior. A small shared destination-bound credential type would concentrate the fix across the already existing push/fetch/pull/raw/clone call paths. This is remediation of E16-01, not an additional inflated finding.

No independent performance blocker was established. Auth cache cardinality is bounded; managed and resource checks intentionally trade database reads for current authorization. Policy/group growth and root administration password hashing were not benchmarked. No scale or passing-test claim is made. Tests were inspected for falsification, never executed under the read-only allocation.

## Residual and unreviewed boundaries

This pass does not certify every route's resource-specific ownership check, every policy-table entry, WebSocket reauthorization, all OTP throttling/recovery interleavings, MCP self-call forwarding, enterprise SSH alias behavior, redirects in the Git executable, clone/LFS/submodule recursive credential behavior, or OS-level Keychain/sandbox isolation. Adjacent sessions/workflows/automation/assistant/swarm/UI changes belong to their assigned reviewers. No browser usability assessment was performed. A scope-wide 9.8 score requires the two fixes, meaningful negative and positive regression results, and cross-review of those boundaries; source reading alone is insufficient.

Both confirmed findings were sent to root session `01M57PYK75FC1F2AFS4C73BBF7` through `otto_send_message`. E16 remains available for allocated fix work and cross-review; production ownership has not been assumed.

## Allocated fix pass — implementation and handoff

The text above records the original read-only review. Root subsequently allocated implementation; the following changes are now present and uncommitted. No edits were reverted, committed, stashed, pushed or deployed. No new reviewer/session was spawned.

### Exact E16 files and responsibility

- `crates/otto-git/src/transport.rs` (new): typed, redacted-debug `TransportCredential`, recorded destination construction, forge identity and automatic-account matching. The permitted authorities come from the selected account and recorded explicit split mapping, never from newly rewritten Git configuration.
- `crates/otto-git/src/askpass.sh` (new): answers only HTTP(S) username/password prompts for approved scheme/authority/port; rejects generic and SSH-key prompts and unrelated rewritten destinations. The token remains environment-only.
- `crates/otto-git/src/local.rs`: AskPass region plus mechanical credential parameter/import changes, clone credential parameter and existing helper test adaptation. Native Git reviewer's stash/worktree logic is preserved and is not E16-owned.
- `crates/otto-git/src/push.rs`, `crates/otto-git/src/ops.rs`: mechanical credential types/call adaptations only.
- `crates/otto-git/src/lib.rs`: export the transport module.
- `crates/otto-git/src/http.rs`: construct typed credentials for transport/clone; auto-select only accounts with matching forge identity; refresh origin and invalidate stale bound identity atomically. Explicit account selection and deliberate owner API-base reconfiguration remain usable.
- `crates/otto-git/src/providers/mod.rs`: document caller-side binding validation and intentional split SSH/API support. The permissive custom-base branch remains intentional for explicitly selected mappings; refresh/adoption now establish its precondition. Provider encoding edits elsewhere belong to native Git reviewer.
- `crates/otto-git/src/http_tests.rs`: split SSH/API owner/root positive test, changed-forge rejection with no extra old-API calls, stale concurrent-binding CAS test. Existing native reviewer tests retained.
- `crates/otto-git/tests/credential_destination.py` (new): isolated real-Git challenge-server regression suite using dummy credentials and loopback-only remotes.
- `crates/otto-state/src/git.rs`: `refresh_repo_binding` compare-and-swap helper updates remote/provider/account together; a stale reader cannot restore a concurrently cleared/rebound account.
- `crates/otto-review/src/worktree.rs`: credential parameter type only.
- `crates/otto-server/src/modules.rs`: materialization wrapper token constructor only; `run_engine.rs`: `best_effort_push` token constructor only; `run_service.rs`: publication token constructor only. E13's proof/retry changes retained. Shared files were not whole-file formatted.
- `docs/contracts/api.md`: Git credential/binding behavior prose; no DTO shapes changed.
- This report.

### Actual verification so far

- **Observed red before implementation:** extracted original production AskPass script, invoked with a dummy token and an unrelated HTTPS password prompt. It returned exit 0 and disclosed the dummy token; denial assertion failed as expected. No real token/network used.
- **Passed:** seven direct shell prompt checks (allowed username/password; unrelated host, downgrade, port mismatch, userinfo host spoof, generic prompt refusal).
- **Passed:** `python3 crates/otto-git/tests/credential_destination.py` — **8 tests**, 7.360 seconds. Real Git exercised matching credential authentication, changed origin, push URL, pushRemote, insteadOf, pushInsteadOf, cross-authority redirect, and same-authority alternate pushRemote positive control. Challenge endpoints received no authorization on denied destinations. These tests do not claim a successful push to a real forge; their oracle is the HTTP Authorization header received by the actual destination.
- **Passed:** `git diff --check` for touched source/contract paths. Rustfmt parsed/normalized owned files or exact E16 snippets; no Cargo command was started by E16.
- **Pending:** Rust tests and clippy. Root reported another reviewer's earlier baseline run of 393 passing/1 ignored Git tests, before this migration. Its intermediate clippy saw expected E0308 errors during signature migration; that is not a validated final result. The identified production callers are now migrated coherently by source inspection, but compiler validation is still required.

### Pending commands and residual issue

Root's shared Cargo scheduler must allocate:

1. `cargo test -p otto-git --lib`
2. `cargo clippy -p otto-git --all-targets -- -D warnings`
3. Shared consumer checks/tests for `otto-review`, `otto-server`, and changed `otto-state` helper (the CAS regression currently lives in Git HTTP tests). Root coordinates these with the other reviewers' changes; do not launch concurrent Cargo against the shared target.
4. Final formatting gate, with native Git/E13 coordination for shared files.

Final caller sweep found one **remaining existing-scope provider path**, reported to root for narrow allocation: `crates/otto-server/src/run_sources.rs:98` `provider_for_repo` directly constructs a provider from the stored remote and account; `run_service.rs` uses it for publication. It bypasses the new HTTP refresh/adoption validation and must use the shared live bound-provider validation before this fix is called complete. No edits to that unallocated wrapper yet. Do not introduce a new owner restriction on the trusted engine; preserve its existing authority while checking destination identity.

Existing split SSH/API mappings remain valid when origin is unchanged. Origin identity changes invalidate the old mapping, and matching-account adoption can recover supported new destinations; an intentionally split new mapping requires explicit account selection. Account owners are still free to deliberately reconfigure the API base and reuse credentials; no new-token requirement was added (user explicitly rejected that adjacent restriction).

**Status:** implementation reviewable, validation pending; not a 9.8 certification. E16 owns no Cargo slot and has no running Cargo process. Root/Ronaldo/Pele can continue from these preserved edits without restarting the review.

## Root autonomous caller integration

The remaining `run_sources::provider_for_repo` now delegates to public `otto_git::http::bound_provider_ctx`. This refreshes live origin and validates the existing bound account through the same `provider_for_bound_account` used by HTTP callers. It neither adopts a new account nor introduces an owner check into the already-authorized autonomous engine. The split-host/changed-forge regression now also exercises this resolver. Scoped formatting and whitespace checks passed; Rust execution remains pending with the Git/server consumer gate.
