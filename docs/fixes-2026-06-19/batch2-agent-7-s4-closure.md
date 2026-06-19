# S4 — Cross-user credential leak: full closure (batch 2, agent 7)

Date: 2026-06-19

## Summary

A prior fix guarded only `otto-issues/src/http.rs`. The same leak was still open in
**otto-server** and **otto-product** (issue accounts) and an analogous one existed
for **git** accounts (push/PR/clone). With the TLS listener live this was the last
critical hole. Every reachable use-path that resolves an account id and then acts
with that account's third-party token now enforces: a credential may be **used**
only by its **owner or root**, otherwise `403 Forbidden`.

## Approach — one canonical guard

Added a single shared chokepoint in **otto-core** (`crates/otto-core/src/auth.rs`):

- `trait OwnedCredential { fn owner_id(&self) -> &Id; }` implemented for
  `GitAccount` and `IssueAccount`.
- `pub fn authorize_owner<C: OwnedCredential>(account: &C, user: &User) -> Result<()>`
  — returns `Forbidden("not the account owner")` unless `user` owns the account
  or `user.is_root`. This is the same semantics otto-issues already used.

Every crate now routes ownership decisions through `authorize_owner`, so the rule
is defined once. Request/response shapes are unchanged (no api.md edit needed);
the legitimate owner and root behave exactly as before.

## Call sites guarded

### otto-issues (`crates/otto-issues/src/http.rs`)
- `authorize_account` refactored to delegate to `authorize_owner` (kept the
  `authorize_account` / `load_authorized_account` wrappers so all existing read/use
  handlers and tests are unchanged). All issue read/use handlers already funnelled
  through `load_authorized_account`, so they stay covered.

### otto-product
Two flavors, both authorized at the **HTTP boundary** (where the authenticated
`user` lives), via two new `ProductService` methods
(`crates/otto-product/src/service.rs`):
- `authorize_account_id(account_id, user)` — loads the account and authorizes.
- `authorize_story_account(story_id, user)` — loads the story, authorizes
  `story.account_id`; an empty/draft `account_id` carries no credential so it
  passes (no leak).

Handlers updated in `crates/otto-product/src/http.rs`:

| Handler (route) | Flavor | Guard |
|---|---|---|
| `import_story` (`POST /workspaces/{ws}/product/stories`) | `req.account_id` | `authorize_account_id` — also rejects binding a story to an account the caller doesn't own |
| `publish_as_rfc` (`POST /product/stories/{sid}/publish-as-rfc`) | `req.account_id` | `authorize_account_id` |
| `publish_as_story` (`POST /product/stories/{sid}/publish-as-story`) | `req.account_id` | `authorize_account_id` |
| `refresh_story` (`POST /product/stories/{sid}/refresh`) | `story.account_id` | `authorize_story_account` |
| `post_questions` (`POST /product/stories/{sid}/questions/post`) | `story.account_id` | `authorize_story_account` |
| `publish_version` (`POST /product/versions/{vid}/publish`) | `story.account_id` | `authorize_story_account(version.story_id)` |
| `publish_tests` (`POST /product/testcase-runs/{rid}/publish`) | `story.account_id` | `authorize_story_account(run.story_id)` |

Notes on the two service methods that also resolve `story.account_id`
(`build_agent_context`, `list_new_comments`): these are reached **only** from
server-internal background jobs (`product_run.rs`, `product_watcher.rs`) acting as
the system on behalf of `story.created_by` — not user-supplied cross-user
requests — so no boundary guard is added there. `publish_as_story`'s non-draft
"convert" path reuses the single account loaded from `req.account_id` (incl. the
cross-link comment), so authorizing `req.account_id` once covers all of its
credential use.

### otto-git (`crates/otto-git/src/http.rs`)
A repo binds exactly one account via `repo.git_account_id`, and a workspace can
have many members, so the existing Editor role-check on the repo did NOT stop
member B from acting through member A's hosting token. Added:
- `authorized_repo_account(s, user, repo)` — resolves the bound account and
  `authorize_owner`s it; returns `None` when no account is bound (ssh-via-agent).
- `optional_token` reworked to `Result<Option<String>>` and to go through
  `authorized_repo_account` (callers updated: `repo_fetch`, `repo_push`,
  `repo_pull`, `repo_collections_pull`, `repo_collections_push`).
- `provider_ctx` reworked to take `user` and go through `authorized_repo_account`
  (all 11 PR handlers updated to pass `&user`).
- `resolve_account` (repo register/clone binding): when an explicit
  `git_account_id` is supplied, `authorize_owner`s it so a caller can never bind a
  repo to a credential they don't own.

### otto-server (`crates/otto-server/src/modules.rs`)
- `resolve_provider_remote(ctx, user, repo)` — now takes the caller and
  `authorize_owner`s the repo's bound git account before building the provider.
  Callers updated: `run_pr_review_inner` and `draft_pr`.
- `start_review` (`POST /repos/{id}/prs/{number}/review`): the caller (`user`) is
  now carried into the spawned `run_pr_review_inner`. Inside it, the user-supplied
  `req.issue_account_id` is `authorize_owner`'d before its token is read to build
  the `JiraClient` (the path the trace flagged at ~line 1434). The git account
  used for the diff is guarded by `resolve_provider_remote`. The two
  `get_account` sites in this file (git ~943, issue ~1446) are now the only ones,
  and both are guarded.

## Tests added (non-owner ⛔ / owner ✅ / root ✅)

- **otto-core** `auth::tests` — `authorize_owner` for a generic credential, plus
  `GitAccount` and `IssueAccount` specifically (owner ok, stranger forbidden,
  root ok).
- **otto-product**
  - `service::tests::authorize_account_id_owner_root_ok_stranger_forbidden`
  - `service::tests::authorize_story_account_guards_bound_credential`
    (incl. accountless draft passes)
  - `http::tests::publish_as_story_rejects_non_owner_account` — HTTP-boundary
    test with `AllowAll` roles proving the **ownership guard** (not the workspace
    role-check) returns `403` for a non-owner and lets the owner through.
- **otto-git**
  - `http::tests::repo_credential_use_is_owner_or_root_only` — real in-memory
    `GitStore`/`GitCtx`, `AllowAll` roles: owner/root get the bound account+token,
    a non-owner is `Forbidden` (not silently `None`), and an unbound repo yields
    `None` for everyone. (Added `sqlx` dev-dependency to otto-git for the
    migration-backed harness.)
- **otto-server `start_review`**: a full `ServerCtx` (≈25 fields incl. session
  manager, orchestrator, spawner) is impractical to construct in a unit test, so
  per the brief's fallback the authorize check is the factored, shared
  `otto_core::auth::authorize_owner`, directly unit-tested in otto-core for both
  `GitAccount` and `IssueAccount`. The modules.rs wiring (single
  `authorize_owner(&account, user)?` before each credential use) is verified to
  compile and is positioned before any token read.

## Build / test status

- `cargo check --workspace` — clean.
- `cargo clippy -p otto-core -p otto-issues -p otto-product -p otto-git` — no
  warnings.
- `cargo test -p otto-server -p otto-product -p otto-issues -p otto-git` — all
  pass: otto-git 29, otto-issues 62, otto-product 23, otto-server 122 + 5
  (auth_security) + 2 (route_inventory). 0 failures.

## Paths judged not to need a boundary guard (with reason)

- otto-product `build_agent_context`, `list_new_comments`: reachable only from
  server-internal background jobs (`product_run.rs`, `product_watcher.rs`) running
  as the system on behalf of the story owner, never from a user-supplied
  cross-user request.
- git ssh-via-agent remotes (no bound account): `authorized_repo_account` returns
  `None`, so there is no stored credential to leak — unchanged behavior.

## Constraints honored

Did not run `cargo fmt`; did not `git commit`. Touched only the assigned files
(plus `otto-core/src/auth.rs` for the shared helper, and `otto-git/Cargo.toml` for
the test dev-dependency, both within the allowed scope). Did not touch
`otto-usage/src/pricing.rs`, `otto-server/src/routes/usage.rs`,
`ottod/src/usage_tailer.rs`, or `otto-improve/src/prompt.rs`.
