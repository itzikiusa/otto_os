# Spec + Plan: Git graph context menus (commit + branch/tag actions)

Branch: `feat/git-context-menus` (off main). No migration. No policy change
(`/repos/...` POST is already `Git:Edit`; GET `View`).

## Requested features (verbatim) → where handled
| # | Request | Menu | Backend |
|---|---------|------|---------|
| 1 | Cherry-pick commit (right-click commit) | Commit menu → "Cherry-pick commit" | `POST /repos/{id}/cherry-pick {sha}` |
| 2 | Checkout specific commit | Commit menu → "Check out commit (detached)" | REUSE `POST /repos/{id}/checkout {branch:<sha>, create:false}` |
| 3 | Delete / rename (local + remote) | Branch menu → "Rename…", "Delete <b>", "Delete origin/<b>", "Delete both" | `POST .../branch/rename`, `.../branch/delete {name, remote?}` |
| 4 | Add tag to commit (+ push to remote) | Commit menu → "Create tag here…", "Create annotated tag here…" (push prompt) | `POST /repos/{id}/tag {name, sha, message?, push?}` |
| 5 | Copy branch name | Branch menu → "Copy branch name" | clipboard (FE only) |
| 6 | Copy commit hash | Commit menu → "Copy commit SHA" (+ "Copy short SHA", "Copy message") | clipboard (FE only) |
| 7 | Create branch from commit | Commit menu → "Create branch here…"; Branch menu → "Create branch from here…" | `POST /repos/{id}/branch {name, start_point?, checkout?}` |

Extras shown in the GitKraken menus (images) we also include because they're
trivial and expected: **Revert commit** (commit menu), **Checkout** (branch menu),
**Copy commit link is OUT** (no hosting-url builder — skip to avoid wrong links),
**Delete tag** (tag menu).

## Integration points (confirmed in code)
- Context menu: `ctxMenu.show(e, MenuItem[])`, `MenuItem {label?, icon?, danger?, separator?, action?}` — flat list; `{separator:true}` draws a divider → **sections**. (`ui/src/lib/contextmenu.svelte.ts`)
- Inputs: `confirmer.promptText(msg, {title?, confirmLabel?, initial?, placeholder?}) → Promise<string|null>`; `confirmer.ask(msg, {...}) → Promise<bool>`. (`ui/src/lib/confirm.svelte.ts`)
- Clipboard: `navigator.clipboard.writeText(text)` (as used in Terminal.svelte); wrap in try/catch + `toasts`.
- Git exec: `LocalGit::run(&[args])` (local), `run_remote(&[args], token)` (remote, AskPass token). Handlers get the token via `optional_token(&s, &user, &repo)` (same as push/pull). (`crates/otto-git/src/local.rs`, `http.rs`)
- GraphView: LEFT refs tree rows (local/remote/tags) + MIDDLE commit `.graph-row` buttons. Add `oncontextmenu` to each. `refKnowledge` derived already gives `{remoteNames, localNames, currentBranch}`. `checkout(branch, create)` exists.

## Backend — new `LocalGit` methods (`crates/otto-git/src/local.rs`)
All return `Result<()>` unless noted; surface git stderr on failure.
- `cherry_pick(sha)` → `git cherry-pick <sha>` (conflicts → Err with stderr).
- `create_branch(name, start_point: Option<&str>, checkout: bool)`:
  - checkout=true → `git checkout -b <name> [<start_point>]`
  - else → `git branch <name> [<start_point>]`
- `delete_branch(name, force)` → `git branch -D <name>` (force) / `-d`.
- `delete_remote_branch(name, token)` → `run_remote(["push","origin","--delete",name], token)` → String.
- `rename_branch(from, to)` → `git branch -m <from> <to>`.
- `create_tag(name, sha, message: Option<&str>)`:
  - annotated (message present) → `git tag -a <name> -m <msg> <sha>`
  - else lightweight → `git tag <name> <sha>`
- `push_tag(name, token)` → `run_remote(["push","origin",format!("refs/tags/{name}")], token)` → String.
- `delete_tag(name)` → `git tag -d <name>`.
- `delete_remote_tag(name, token)` → `run_remote(["push","origin","--delete",format!("refs/tags/{name}")], token)`.
- `revert(sha)` → `git revert --no-edit <sha>`.

## Backend — new routes (`crates/otto-git/src/http.rs`)
All `WorkspaceRole::Editor`, return **`RepoStatusResp`** (via `git.status()`), so the
UI refreshes ahead/behind + graph after every op (mirrors fetch/pull/push). Remote
ops fetch `optional_token` first.
- `POST /repos/{id}/cherry-pick`  `{ sha }`
- `POST /repos/{id}/branch`        `{ name, start_point?, checkout? }`  (create)
- `POST /repos/{id}/branch/rename` `{ from, to }`
- `POST /repos/{id}/branch/delete` `{ name, remote? }`  (remote→also `push origin --delete`)
- `POST /repos/{id}/tag`           `{ name, sha, message?, push? }`  (annotated if message; push via token)
- `POST /repos/{id}/tag/delete`    `{ name, remote? }`
- `POST /repos/{id}/revert`        `{ sha }`
Register in the router list; document each in `docs/contracts/api.md`. Req structs
live in `otto-core` api.rs (or inline `#[derive(Deserialize)]` in http.rs, matching
neighbours like `StagePathsReq`). Mirror any new TS req types only if the UI needs
them (it sends inline literals → likely none).

## Frontend — `ui/src/modules/git/GraphView.svelte`
Add a small `clip(text, label)` helper (clipboard + toast) and `refreshAll()` (re-run
the log+refs+status load — reuse the existing reload path / bump the parent via the
`onstatus` it already calls; after a mutation, re-fetch refs+log+status).

### Commit context menu (right-click a `.graph-row`)
Sections (separators between):
1. **Check out commit (detached)** → checkout(sha,false) · **Cherry-pick commit** → /cherry-pick
2. **Create branch here…** → promptText → /branch {name, start_point:sha, checkout:true} · **Create tag here…** → promptText(name) → /tag {name, sha} · **Create annotated tag here…** → promptText(name)+promptText(message) → /tag {name, sha, message} (then ask "push tag to origin?" → push:true)
3. **Revert commit** → /revert {sha} (ask confirm)
4. **Copy commit SHA** · **Copy short SHA** · **Copy commit message** → clipboard

### Branch context menu (right-click a LOCAL or REMOTE ref row)
1. **Checkout** → local: checkout(name,false); remote: checkout(localName,true) (existing checkoutRemote)
2. **Create branch from here…** → promptText → /branch {name, start_point:<ref>, checkout:true} · **Rename…** (local only) → promptText(initial=name) → /branch/rename {from,to}
3. **Delete <name>** (local, confirm, danger) → /branch/delete {name} · **Delete origin/<name>** (when a matching remote exists, or on a remote row) → /branch/delete {name, remote:true} · (local with tracking) **Delete local + remote** → two calls / {name, remote:true}
4. **Copy branch name** → clipboard (the short name) · **Copy upstream name** (if upstream)
Guard: never offer Delete on the **current** branch (git refuses); disable/omit.

### Tag context menu (right-click a TAG row)
1. **Check out tag (detached)** → checkout(tag,false)
2. **Push tag to origin** → /tag {name:<tag>, sha:<tagged sha or "">, push:true} … simpler: add `POST /repos/{id}/tag` accepts existing tag push? To avoid ambiguity, push-existing-tag uses `/tag/push {name}` OR reuse create with the same name+sha. DECISION: add `push` handling to `/tag` only for create; for an EXISTING tag, the create section already pushed. A standalone tag row "Push" is a nice-to-have → use `/tag/delete`-style `/tag/push {name}` (add it). KEEP minimal: tag row offers **Delete tag** (local) + **Delete tag on origin** (remote) + **Copy tag name**.

(Tag push for already-created tags is covered by the commit-menu "create … then push?" flow; a dedicated existing-tag push can be a follow-up.)

## Safety / guardrails
- Destructive ops (delete branch/tag, revert, reset-like) → `confirmer.ask` with `danger`.
- Never delete the checked-out branch (omit the item when `name === currentBranch`).
- Cherry-pick / revert can conflict → surface the stderr via `toasts.error`; the
  existing merge-conflict UI is separate (don't auto-open it; just report).
- All remote ops require the repo's bound account token (already via `optional_token`);
  SSH remotes use agent/keys (token may be None — fine).
- After every mutation: refresh refs+log+status so the graph reflects the change
  (mirrors the pull/push fix — return RepoStatusResp + FE re-query).

## Verification
- `cargo build/clippy -p otto-git -p otto-server -- -D warnings`; `cargo test -p otto-server` (route_inventory).
- `cd ui && npm run check`.
- Manual: right-click a commit (cherry-pick/checkout/branch/tag/copy/revert) and a
  branch (checkout/rename/delete local+remote/copy) on a real repo.
- Merge to main + rebuild/sign/reinstall + `kickstart -k` to load the new daemon.
