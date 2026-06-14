# Otto API Contract — /api/v1 (FROZEN)

All DTO names refer to types in `crates/otto-core/src/api.rs` (Rust) mirrored in
`ui/src/lib/api/types.ts` (TS). JSON: snake_case fields, RFC3339 timestamps, ULID ids.
Auth: `Authorization: Bearer <token>` unless marked public. Errors: HTTP status per
`otto_core::Error` variant + body `Problem{code,message}`.

Roles: `root` = global; workspace roles `viewer < editor < admin`. Root passes every check.
"member" below means any authenticated user; workspace-scoped routes require at least the
listed role IN THAT WORKSPACE. Sessions/connections/repos/PRs inherit their workspace.

| # | Method & path | Auth | Request | Response |
|---|---|---|---|---|
| 1 | GET /api/v1/health | public | — | `{"ok":true}` |
| 2 | GET /api/v1/meta | public | — | MetaResp |
| 3 | POST /api/v1/onboarding/root | public, only while 0 users exist (else 409) | OnboardRootReq | LoginResp |
| 4 | POST /api/v1/auth/login | public | LoginReq | LoginResp (401 on bad creds/disabled) |
| 5 | POST /api/v1/auth/logout | member | — | 204 |
| 6 | GET /api/v1/auth/me | member | — | User |
| 7 | GET /api/v1/users | root | — | `User[]` |
| 8 | POST /api/v1/users | root | CreateUserReq | User (409 dup username) |
| 9 | PATCH /api/v1/users/{id} | root | UpdateUserReq | User |
| 10 | DELETE /api/v1/users/{id} | root | — | 204 (soft: sets disabled; root user cannot be disabled → 400) |
| 11 | GET /api/v1/workspaces | member | — | `WorkspaceWithRole[]` (root sees all as admin) |
| 12 | POST /api/v1/workspaces | member | CreateWorkspaceReq | Workspace (creator becomes admin member) |
| 13 | PATCH /api/v1/workspaces/{id} | ws admin | UpdateWorkspaceReq | Workspace |
| 14 | DELETE /api/v1/workspaces/{id} | ws admin | — | 204 (archives) |
| 15 | GET /api/v1/workspaces/{id}/members | ws admin | — | `MemberEntry[]` |
| 16 | PUT /api/v1/workspaces/{id}/members | ws admin | SetMembersReq | `MemberEntry[]` |
| 17 | GET /api/v1/workspaces/{id}/sessions | ws viewer | — | `Session[]` |
| 18 | POST /api/v1/workspaces/{id}/sessions | ws editor | CreateSessionReq | Session |
| 19 | GET /api/v1/sessions/{id} | ws viewer | — | Session |
| 20 | PATCH /api/v1/sessions/{id} | ws editor | UpdateSessionReq | Session |
| 21 | DELETE /api/v1/sessions/{id} | ws editor | — | 204 (kills PTY, removes row) |
| 22 | POST /api/v1/sessions/{id}/restart | ws editor | — | Session (respawn; uses resume args when provider_session_id set) |
| 23 | POST /api/v1/workspaces/{id}/orchestrate | ws editor | OrchestrateReq | OrchestrateResp |
| 24 | POST /api/v1/workspaces/{id}/orchestrate/execute | ws editor | ExecutePlanReq | `{"results":[{"action_index":0,"ok":true,"detail":"...","session_ids":["..."]}]}` |
| 25 | GET /api/v1/workspaces/{id}/connections | ws viewer | — | `Connection[]` (includes global ones; secret never present) |
| 26 | POST /api/v1/workspaces/{id}/connections | ws editor | UpsertConnectionReq | Connection |
| 27 | PATCH /api/v1/connections/{id} | ws editor (global: root) | UpsertConnectionReq (all fields optional semantics: absent secret = keep) | Connection |
| 28 | DELETE /api/v1/connections/{id} | ws editor (global: root) | — | 204 (deletes Keychain secret too) |
| 29 | POST /api/v1/connections/{id}/open | ws editor | `{"title":null}` optional | Session |
| 30 | POST /api/v1/connections/{id}/test | ws editor | — | TestConnectionResp |
| 31 | GET /api/v1/git/accounts | member | — | `GitAccount[]` (own accounts only; token never present) |
| 32 | POST /api/v1/git/accounts | member | CreateGitAccountReq | GitAccount |
| 33 | DELETE /api/v1/git/accounts/{id} | member (owner) | — | 204 |
| 34 | GET /api/v1/workspaces/{id}/repos | ws viewer | — | `Repo[]` |
| 35 | POST /api/v1/workspaces/{id}/repos | ws editor | AddRepoReq | Repo (clone runs async; Notice events report progress/done) |
| 36 | DELETE /api/v1/repos/{id} | ws editor | — | 204 (unregisters; never deletes files) |
| 37 | GET /api/v1/repos/{id}/status | ws viewer | — | RepoStatusResp |
| 38 | GET /api/v1/repos/{id}/branches | ws viewer | — | `BranchInfo[]` |
| 39 | GET /api/v1/repos/{id}/log?limit=50&skip=0 | ws viewer | — | `CommitInfo[]` |
| 40 | GET /api/v1/repos/{id}/diff?target=worktree\|staged\|commit:<sha>\|range:<a>..<b> | ws viewer | — | DiffResp |
| 41 | POST /api/v1/repos/{id}/stage | ws editor | StagePathsReq | RepoStatusResp |
| 42 | POST /api/v1/repos/{id}/unstage | ws editor | StagePathsReq | RepoStatusResp |
| 43 | POST /api/v1/repos/{id}/commit | ws editor | CommitReq | `{"sha":"..."}` |
| 44 | POST /api/v1/repos/{id}/push | ws editor | — | `{"output":"..."}` |
| 45 | POST /api/v1/repos/{id}/pull | ws editor | — | `{"output":"..."}` |
| 46 | POST /api/v1/repos/{id}/checkout | ws editor | CheckoutReq | RepoStatusResp |
| 47 | POST /api/v1/repos/{id}/stash | ws editor | `{"op":"save"\|"pop"}` | RepoStatusResp |
| 48 | GET /api/v1/repos/{id}/prs?state=open\|merged\|declined\|all | ws viewer | — | `PrSummary[]` |
| 49 | POST /api/v1/repos/{id}/prs | ws editor | CreatePrReq | PrSummary |
| 50 | GET /api/v1/repos/{id}/prs/{number} | ws viewer | — | PrDetail |
| 51 | GET /api/v1/repos/{id}/prs/{number}/diff | ws viewer | — | DiffResp |
| 52 | PATCH /api/v1/repos/{id}/prs/{number} | ws editor | UpdatePrReq | 204 |
| 53 | POST /api/v1/repos/{id}/prs/{number}/comments | ws editor | NewPrCommentReq | PrComment |
| 54 | POST /api/v1/repos/{id}/prs/{number}/approve | ws editor | — | 204 |
| 55 | POST /api/v1/repos/{id}/prs/{number}/merge | ws editor | MergePrReq | 204 |
| 56 | POST /api/v1/repos/{id}/prs/{number}/decline | ws editor | — | 204 |
| 57 | GET /api/v1/settings | root | — | `{ "<key>": <value_json>, ... }` |
| 58 | PUT /api/v1/settings | root | same shape | same shape |

Notes:
- Session create with kind=connection requires `connection_id`; provider is set server-side
  to the connection kind. Title defaults: agent → "<provider> #N", connection → conn name.
- PR routes resolve the provider + account from the repo row (`provider`, `git_account_id`);
  if the repo has no provider/account → 400 `invalid`.
- `/orchestrate` never executes; it only returns a plan. Execution is the separate call #24.
- Settings keys used in v1: `network_listener` `{enabled:bool, port:u16}`, `providers`
  (provider registry overrides), `default_provider` (string).
