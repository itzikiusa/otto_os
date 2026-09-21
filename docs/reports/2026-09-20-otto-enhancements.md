# Otto enhancements — 2026-09-20

All work is local and uncommitted in `fix/workflow-summarizer-sessions`, based on main `11b564456f22bb2ec7a46c638980cdef9d49a9b6`. No commits, pushes, PRs, or real account/bastion logins were performed. The original main checkout remains unchanged. At the initial version check, origin/main and local main matched; the installed build had the same source tree under a different merge commit, so there was nothing to pull.

## What to try

| Area | Behavior and where to test |
|---|---|
| Workflow summarizer | Open a review workflow's embedded agents. The summarizer now appears as a managed session with provider, progress, elapsed time and terminal access. Cancellation, retries and deterministic fallback remain visible; review associations survive reload. |
| Git | Leave repositories open in Git tabs and work elsewhere in Otto. Auto-fetch runs across the authenticated app with two-minute open-tab and thirty-second selected-tab freshness, focus wakeups, two concurrent requests and error backoff. Hidden/unfocused windows pause it. Fetch refreshes tracking counts for every local branch without checkout. |
| Workspace context (corrected September 21) | The workspace itself is the shared project. Open Settings → Workspace context or right-click the workspace. Its goal, instructions, references, memory, decisions and artifacts apply automatically to agent sessions on launch/restart. The separate Projects section and session picker are removed; existing Swarm data stays intact. |
| Subscription accounts | New Session → Claude/Codex account → Add account → Sign in → Check sign-in. Each named profile has an isolated native home; restart preserves the account. Account labels are visible on sessions. Default CLI accounts still work. |
| Session tokens | Settings → Personal Access Tokens now separates personal, managed-session and legacy-label credentials. Durable credentials rotate/revoke on restart, deletion and failed spawn. Deleted-session credentials cannot authenticate. **Deleted session** filters historical candidates; bulk cleanup is explicit and retains failed revocations. Archive is distinct from deletion. |
| Personal Agents | Edit an agent's Memory or Context directly. Memory saves use optimistic concurrency and atomic file replacement; changing folders cannot make an old editor overwrite the new target. Context is snapshotted into new runs/chats. |
| Path selection | Personal Agents and other audited local path forms reuse the folder/file picker while retaining typed entry and optional defaults. Remote paths keep remote semantics. |
| Goal Loops | Check provider-aware embedded role sessions, preserved worktrees, explicit human criteria approval, questions/evidence ledger, research mode, retry/cancellation and repeated-failure blocking. Commits are disabled by default; terminal/recovery paths no longer force-delete working files. |
| Self-improvement | Enable the existing workspace/live policy or session Evolve option. Recent Claude/Codex evidence and recorded prompts/notes from other providers feed the existing proposal/allow-list/autonomy policy. Relevant skills can be read for proposals even when they are outside the auto-apply allowlist; that does not grant write permission. Review approve/decline actions feed deduplicated evidence. Failed analysis does not consume the successful checkpoint. |
| API client | The old **Local: on/off** label is now **Private addresses: allowed/blocked**. It controls the API client's access to localhost/private targets, not whether Otto's daemon is running. The default remains blocked and changes remain admin-gated. |
| SSH access | Select a workspace network profile in New Session. An existing SSH connection supplies the bastion; named TCP endpoints get managed localhost forwards and explicit host/port environment mappings. Session status shows the actual local endpoints and failures. |
| Vault | OKF 0.2 metadata is readable alongside v0.1. The note panel shows declared verification, sources, generation time, lifecycle and staleness; property editing preserves nested data. Index generation preserves the declared version. New concept templates start as drafts. The bundled authoring skill/validator is updated. |

## Design conclusions

[Claude's redesigned Projects](https://claude.com/blog/projects-redesigned) combines a coordinator, parallel session threads, shared memory and a library of inputs/results. It is more than a shared-context folder. Following the September 21 correction, Otto uses its existing Workspace as the provider-independent identity and context owner; Swarm supplies coordinated execution within it. Sessions inherit their workspace context automatically. Workspace context does not automatically launch coordinators or merge every transcript into shared memory.

The [current OKF specification](https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md) is **0.2**. Prioritize evidence provenance, review history, explicit stale deadlines and deprecation. A freshly indexed note is not necessarily factually fresh. A Markdown `human:` verifier is a declaration, not authenticated approval. Otto accepts Attested Computation concepts but does not execute or certify their referenced computations.

The `goal-loop` skill in `~/ai_skills` was found on `origin/feature/bo-skill`, not main. The native loop benefits from persistent state, visible provider sessions and enforced human verification. The changes address the significant work-preservation and lifecycle gaps. Portable export/import, richer task/file tracking, selected-skill availability preflight and a configurable stagnation threshold remain future extensions. Monetary caps are rejected while reliable per-role accounting is unavailable, rather than displaying an unenforced limit.

## Practical limits

- Real subscription login/token refresh across two actual accounts requires signing in; tests use isolated fake providers. Profiles do not inherit the default account's credentials. Incompatible enterprise routing or conflicting Codex auth overrides fail visibly.
- SSH forwarding changes the destination your program must use. Configure the displayed localhost host/port or mapped environment variables. This is not a transparent VPN. A bastion must itself be reachable without FortiClient to remove that dependency. TLS server identity and MongoDB/Kafka topology discovery require appropriate driver configuration. No office server or database was contacted during development.
- Historical label-only API tokens are shown as candidates, not automatically deleted. No live credentials were revoked during development.
- Gemini/agy native database/protobuf transcripts are not parsed by the learner; recorded user prompts, notes and skill activity supply its evidence. Automatic changes still follow existing policy.
- Vault metadata does not automatically rewrite user bundles, authenticate embedded review claims or create graph edges for frontmatter-only sources.

## Verification and installation

Feature tests use isolated SQLite databases, temporary files, fake provider/tunnel implementations and intercepted browser HTTP/WebSocket fixtures. Full workspace tests that would create Git fixture commits were not run, honoring the no-commit instruction. All feature implementations and independent reviews are complete. Final checks include:

- UI type check: zero errors and warnings; full unit suite: **141 passed**.
- Six isolated browser configurations: **9 passed**, covering all newly added mocked UI flows.
- Workspace Clippy, all targets: passed with warnings denied.
- Combined core/state/RBAC/context/Vault library suite: **481 passed**; four additional account-policy tests passed.
- Focused regressions cover account isolation/resume, session credentials, summarizer lifecycle, SSH authorization/cleanup, Goal Loop safety, Projects, personal-document conflicts and OKF compatibility.
- Self-improvement: **49 safe tests plus 3 persistence tests passed**. Two preexisting fixtures that write to the default user home were excluded.
- Bundled OKF authoring validator/audit: **30 tests passed**.
- Advisory `cargo fmt --all --check` reports formatting differences; no repository-wide reformat was applied. `git diff --check` passed.

The final install uses the frozen uncommitted source fingerprint and signed binary hashes. It stops the old app/daemon before backing up SQLite, preserves the previous app, then checks the new process IDs, signatures, exact installed bytes, six migrations, healthy API and embedded UI. The actual build/install outcome is recorded separately in `~/Library/Logs/Otto/uncommitted-20260920/installed.json`; the presence of a source/build receipt alone does not mean installation succeeded.
