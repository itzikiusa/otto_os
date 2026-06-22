# Git & Pull Requests

Otto's Git feature is a full, GitKraken-style git client built into the desktop
app: browse and clone repos, stage/commit/discard, read diffs, walk the commit
graph, resolve merge conflicts, and **open pull requests** on GitHub, Bitbucket
Cloud, or GitLab — with an **agent-drafted title and description** generated from
your branch diff and the branch pushed for you automatically.

This is the definitive end-user + operator guide. It documents what the code in
`crates/otto-git/` and `ui/src/modules/git/` actually does — including the exact,
step-by-step way to set up a git token so PR creation works.

> Related docs: **[AI code review](./code-review.md)** (the Review tab and
> multi-agent PR review) and **[SSH & SFTP connections](./connections-ssh-sftp.md)**
> (SFTP file browsing over SSH — a *separate* feature, not part of this module).

---

## 1. Summary

| | |
|---|---|
| **What it is** | An embedded git client + hosted-provider PR client. |
| **Forges supported** | GitHub, **Bitbucket Cloud**, GitLab (incl. GitHub Enterprise and self-hosted GitLab via host heuristics). |
| **How PRs authenticate** | A per-user **git account** (a PAT/app password) stored in the macOS Keychain. |
| **Branch push on PR create** | Automatic (`git push --set-upstream` for a fresh branch) before the PR is opened. |
| **Agent drafting** | The "Draft message with agent" button runs your default agent CLI over the branch diff to produce title + Markdown body. |
| **Where it lives** | The **Git** section of the app (top-level nav); accounts under **Settings → Git Accounts**. |
| **Daemon** | `ottod` on `127.0.0.1:7700`; routes under `/api/v1` (contract endpoints #31–#56 plus the extras table). |

---

## 2. Overview & where it lives

The Git page is **workspace-independent**: it shows GitKraken-style top-level
**repo tabs** (one per open repo) above the active repo's view. With no tab open
it shows a landing page — a repo list plus the **Add Repository** flow. Open
repos and the active tab **persist across app restarts** (stored in the UI's git
store via `localStorage`).

Each repo tab has its own sub-tabs: **Graph**, **Changes**, **History**,
**Pull Requests**, and **Review**. A conflicted merge surfaces a temporary
**Conflict** tab.

### Where everything lives

| Thing | Location |
|---|---|
| Git page (repo tabs + landing) | `ui/src/modules/git/GitPage.svelte`, `GitTabs.svelte` |
| One repo (toolbar + Graph/Changes/History/PRs/Review tabs) | `ui/src/modules/git/RepoView.svelte` |
| Stage/commit/discard panel | `ui/src/modules/git/ChangesView.svelte` |
| Commit graph + branch chips + context menus | `ui/src/modules/git/GraphView.svelte` |
| Diff rendering | `ui/src/modules/git/DiffViewer.svelte` |
| Create-PR sheet | `ui/src/modules/git/CreatePr.svelte` |
| PR list / PR detail | `ui/src/modules/git/PrList.svelte`, `PrDetail.svelte` |
| Merge-conflict resolver | `ui/src/modules/git/ConflictResolverView.svelte`, `ConflictFilePane.svelte`, `ConflictHunk.svelte` |
| **Git accounts settings (token setup)** | `ui/src/modules/git/GitAccounts.svelte` (mounted in `ui/src/modules/settings/Settings.svelte` as the **Git Accounts** page) |
| Daemon git router | `crates/otto-git/src/http.rs` |
| Local git (shells out to system `git`) | `crates/otto-git/src/local.rs` |
| Hosted-provider clients | `crates/otto-git/src/providers/{github,bitbucket,gitlab}.rs` |
| Remote-URL → provider detection | `crates/otto-git/src/providers/detect.rs` |
| Agent PR/commit drafting | `crates/otto-server/src/modules.rs` (`draft_pr`, `draft_commit_message`) |
| Secret (token) storage | `crates/otto-keychain/src/lib.rs` |
| API contract | `docs/contracts/api.md` (#31–#56 and the "Git — repos & PR extras" table) |

A repo only needs a bound git account for **remote** operations (push, pull over
HTTPS, listing/creating PRs). Pure local operations (status, diff, stage,
commit, branch, local merge) work with no account at all.

---

## 3. Setting up a git token for PRs

PR creation, PR listing, and HTTPS push all authenticate through a **git
account** — a label + username + token tied to one provider. The token is stored
in the macOS Keychain; the database holds only an opaque reference to it.

### 3.1 Where it goes in Otto

1. Open **Settings** (gear / settings nav).
2. Choose the **Git Accounts** page.
3. Click **Add Account**.
4. Pick the **Provider** (GitHub / Bitbucket / GitLab — segmented control). The
   provider **cannot be changed after creation**; delete and re-add to switch.
5. Fill in:
   - **Label** — any name you like (e.g. `work github`).
   - **Username** — your provider login (see per-provider notes below).
   - **Token** — paste the PAT / app password. This field is **write-only**: it
     is stored and never shown again. On **Edit**, leaving it blank keeps the
     existing token; entering a new one rotates it (the old Keychain entry is
     deleted).
   - **Organisation / Workspace / Group** (optional, the "namespace") — set this
     to enable the **Browse remote** repo picker (search + clone repos under that
     org/workspace/group).
   - **API base URL** (GitLab only, optional) — for self-hosted GitLab, e.g.
     `https://gitlab.example.com/api/v4`.
   - **Token expiry** (optional) — GitHub/GitLab auto-detect expiry server-side;
     set a date here to get a reminder (and for Bitbucket, which exposes no
     expiry, to get one at all).
6. Click **Add Account**.

When you add or clone a repo, Otto auto-binds it to your first account matching
the repo's detected provider; you can also pick the account explicitly in the
Add-Repository sheet. PR/push operations on that repo then use that account's
token.

### 3.2 GitHub — Personal Access Token

GitHub uses `Authorization: Bearer <token>`. Either token type works:

**Classic PAT** (simplest):
1. Go to **github.com → Settings → Developer settings → Personal access tokens →
   Tokens (classic)** (`https://github.com/settings/tokens`).
2. **Generate new token (classic)**.
3. Select the **`repo`** scope (this covers reading/creating pull requests and
   pushing). For the optional org/repo **Browse remote** picker, `repo` is
   sufficient for private repos; public-only listing needs no extra scope.
4. Generate, copy the token (starts with `ghp_`), and paste it into Otto's
   **Token** field. **Username** = your GitHub login.

**Fine-grained PAT** (more scoped):
1. **github.com → Settings → Developer settings → Fine-grained tokens**
   (`https://github.com/settings/tokens?type=beta`).
2. Select the **resource owner** (you or your org) and the repositories to grant.
3. Under **Repository permissions**, grant at least:
   - **Contents: Read and write** (clone/push),
   - **Pull requests: Read and write** (list/create/comment/merge PRs).
4. Generate, copy (starts with `github_pat_`), paste into Otto.

> Otto reads GitHub's `github-authentication-token-expiration` response header on
> a cheap `GET /user` call to auto-fill the expiry for fine-grained / app tokens.

### 3.3 Bitbucket Cloud — App Password

Bitbucket uses **HTTP Basic auth** (`username:app-password`).

1. Go to **bitbucket.org → (avatar) → Personal settings → App passwords**
   (`https://bitbucket.org/account/settings/app-passwords/`).
2. **Create app password**.
3. Grant at least these scopes:
   - **Pull requests: Read** and **Write** (list / create / comment / merge PRs),
   - **Repositories: Read** and **Write** (clone / push).
4. Create, copy the app password, and paste it into Otto's **Token** field.
   **Username** = your Bitbucket **username** (not your email; this is sent as the
   Basic-auth user). The namespace field, if set, is your **Workspace** id.

> Bitbucket Cloud is the supported variant (`api.bitbucket.org/2.0`). Bitbucket
> **Server / Data Center** is *not* an auto-detected host. Bitbucket exposes no
> token-expiry endpoint, so set **Token expiry** manually if you want a reminder.

### 3.4 GitLab — Personal Access Token

GitLab uses a token sent on every API call.

1. Go to **gitlab.com → (avatar) → Edit profile → Access tokens**
   (`https://gitlab.com/-/user_settings/personal_access_tokens`).
2. **Add new token**.
3. Grant the **`api`** scope (full API access — needed to list/create/merge MRs
   and read repos). For pushing over HTTPS, `write_repository` is also relevant.
4. Create, copy, paste into Otto. **Username** = your GitLab username.
5. **Self-hosted GitLab:** set **API base URL** to your instance's API root, e.g.
   `https://gitlab.example.com/api/v4`. Otto also auto-detects any host whose name
   contains `gitlab` as a GitLab remote.

### 3.5 How the token is used to push

For HTTPS remotes, the daemon hands `git` a temporary `GIT_ASKPASS` helper script
and passes the token via the `OTTO_GIT_TOKEN` environment variable — the token is
**never written to disk** and never embedded in the remote URL. The askpass
script answers "Username" prompts with `x-token-auth` (or your account username)
and everything else with the token. This works uniformly for GitHub (any
username + PAT), Bitbucket (app password), and GitLab (PAT). SSH remotes don't
use the token at all — they go through your SSH agent, so an SSH-remote repo can
push/pull with no git account bound.

---

## 4. Repo tabs & browsing

### Adding a repository

From the Git landing page, **Add Repository** offers three modes:

- **Register a local path** — point at an existing working tree already on disk.
  Otto reads its `origin` remote, detects the provider, and auto-binds a matching
  account. (The daemon refuses a path that isn't a git repo.)
- **Clone a URL** — clone a remote into a chosen parent directory. The
  destination directory is **remembered** across sessions (`localStorage` key
  `otto_git_clone_dir`) so you never re-pick it; a **Browse…** folder picker is
  provided, and `~` is expanded to your home. The clone runs **asynchronously**:
  the repo row appears immediately and progress/done is reported via Notice
  toasts. (A clone into an already-existing destination is rejected with 409.)
- **Browse remote** — for an account with a namespace set, search repositories
  under that org/workspace/group and clone the one you pick.

### Repo tabs

- Tabs are **workspace-independent**: the list of open repos is global, not tied
  to the active workspace, and survives app restarts.
- The repo list itself comes from `GET /git/repos`, which returns every repo
  across all workspaces the caller may view (root sees all; a non-root user sees
  repos in workspaces they're a member of).
- A `#/git/:id` or `#/git/:id/:tab` deep link opens that repo as a tab.

### The Graph view & branch chips

The **Graph** tab renders the commit graph with **branch chips drawn on the
graph** at each ref's tip. Right-clicking a commit, branch chip, or tag opens a
context menu (see §[Context menus](#context-menus)). Removing a repo
**unregisters** it only — `DELETE /repos/{id}` never touches the files on disk.

---

## 5. Stage / commit / discard / diff

The **Changes** tab is the working-tree view, backed by `git status`:

- **Stage / unstage** selected paths (`POST /repos/{id}/stage`,
  `/unstage`). Each returns the fresh `RepoStatusResp` so the UI updates.
- **Discard** un-staged changes to selected paths (`POST /repos/{id}/discard`) —
  this reverts/removes working-tree changes, so confirm before using it.
- **Commit** the staged changes (`POST /repos/{id}/commit`, returns the new
  `{sha}`); supports **amend**.
- **Draft a commit message with an agent** — `POST /repos/{id}/draft-commit-message`
  produces a Conventional-Commits-style message from the **staged** diff (falling
  back to the full working diff when nothing is staged). If the bundled
  **`commit-message`** skill is installed (Settings → Skills), its full method is
  prepended to the draft prompt, and the branch's Jira key (e.g. `GS-1234`) is
  injected into the subject. PR drafting (`/repos/{id}/pr/draft`) does the same
  with the **`pull-request`** skill — Jira key as the title prefix only (never in
  the body). Both skills honor the repo's existing commit convention and add **no
  AI attribution** (no `Co-Authored-By` / "Generated with" footer). When a skill
  isn't installed, drafting behaves exactly as before.
- **Push / pull / fetch** (`/push`, `/pull`, `/fetch`) — push auto-sets upstream
  for a branch that has none; each returns fresh status so the ahead/behind chip
  updates.

### Diffs

`GET /repos/{id}/diff?target=…` renders unified diffs for several targets:

| `target` | Shows |
|---|---|
| `worktree` (default) | Unstaged working-tree changes |
| `staged` | The staged index |
| `commit:<sha>` | A single commit |
| `range:<a>..<b>` | A commit range |

The diff is parsed into a structured `DiffResp` and rendered by `DiffViewer`.

---

## 6. Merge-conflict resolution

Otto resolves conflicts entirely in-app, without dropping to a terminal.

1. **Start a local merge** — `POST /repos/{id}/merge` with a source/target branch
   and optional `auto_stash` (stash → merge → pop when the tree is dirty). A clean
   merge completes immediately; conflicts open the resolver.
   - **Preview first (no mutation):** `POST /repos/{id}/merge/preview` does a
     dry-run via `git merge-tree` and reports whether the merge would conflict —
     no tree changes.
2. **Resolver view** (`ConflictResolverView.svelte`): left pane lists conflicted
   files with a resolved/unresolved indicator; right pane shows the selected
   file's hunks (`ConflictFilePane` / `ConflictHunk`).
   - `GET /repos/{id}/conflict?path=…` returns one file's conflict content;
   - `POST /repos/{id}/conflict/resolve` writes your chosen resolution for a path
     and re-stages it.
3. **Complete or abort:**
   - **Complete merge** (enabled once every file is resolved) →
     `POST /repos/{id}/merge/commit` creates the merge commit.
   - **Abort merge** → `POST /repos/{id}/merge/abort` restores the pre-merge
     state.
   - `GET /repos/{id}/merge/status` reports in-progress merge state (used to
     restore the resolver after a reload).

> All mutating merge/conflict operations on a given repo are **serialized** by a
> per-repo lock in the daemon, so concurrent requests can't corrupt an in-progress
> merge.

---

## 7. Creating a pull request

Open the **New Pull Request** sheet from a repo's Pull Requests tab (or from a
branch/commit context menu, which pre-fills the source branch).

**The flow:**

1. **Pick Source → Target.** The sheet loads the repo's branches; Source defaults
   to the current branch (or the one you opened it from), and Target auto-picks
   `develop`/`main`/`master` if present, else a different branch.
2. **(Optional) Draft message with agent.** Click **Draft message with agent**.
   Otto:
   - computes the diff of the current branch against the chosen target
     (`POST /repos/{id}/pr/draft` → server `draft_pr`),
   - caps the diff at ~40 KB and feeds it to your **default agent CLI** with a
     prompt asking for an imperative title (~72 chars) and a Markdown body
     (summary + "What changed" bullets + "Testing" notes),
   - fills the **Title** and **Description** fields from the agent's JSON reply.
     You review and edit before creating. (If the branch has no changes vs the
     target, drafting returns an error.)
3. **Create.** On **Create Pull Request**, Otto:
   - **pushes the source branch first** (`POST /repos/{id}/push`,
     `--set-upstream` for a fresh branch) so the provider can see it. A real push
     failure (e.g. auth) stops the flow before the PR call.
   - **opens the PR** (`POST /repos/{id}/prs` with `CreatePrReq` =
     `{title, description, source_branch, target_branch}`) on the bound provider.

**After creation**, the PR appears in the **Pull Requests** list. From PR detail
you can read the diff, comments (inline + general, threaded), reviewers/approvals,
CI status, and mergeability; comment; approve; request changes; merge; or decline.

### Merge strategies

`POST /repos/{id}/prs/{number}/merge` takes `MergePrReq.strategy` —
`merge` (default), `squash`, or `rebase`:

| Otto strategy | GitHub | Bitbucket | GitLab |
|---|---|---|---|
| `merge` | `merge` | `merge_commit` | merge (no squash) |
| `squash` | `squash` | `squash` | `squash: true` |
| `rebase` | `rebase` | `fast_forward` (Bitbucket has no rebase-merge) | merge (squash off) |

### What about draft PRs?

There is **no draft-PR toggle on creation**. `CreatePrReq` carries only
title/description/source/target; Otto always opens a ready PR. (GitLab's
`draft` flag is only *read* from a `Draft:`/`WIP:` title prefix, and Bitbucket's
draft flag is not populated.) The "Draft message with agent" button refers to
drafting the **text**, not a GitHub draft PR.

---

## 8. API / contract reference

`docs/contracts/api.md` is authoritative. All paths are under `/api/v1`.

### Git accounts (#31–#33 + extras)

| Method & path | Auth | Notes |
|---|---|---|
| `GET /git/accounts` | member | Own accounts only; token never returned |
| `POST /git/accounts` | member | `CreateGitAccountReq` → token stored in Keychain |
| `PATCH /git/accounts/{id}` | member (owner) | Update; non-empty token rotates the secret |
| `DELETE /git/accounts/{id}` | member (owner) | Also deletes the Keychain entry |
| `GET /git/accounts/{id}/remote-repos?q=…` | member (owner) | Browse repos under the account namespace |

### Repos (#34–#36 + extras)

| Method & path | Auth | Notes |
|---|---|---|
| `GET /git/repos` | Git:View | All visible repos (workspace-independent) |
| `GET /workspaces/{id}/repos` | ws viewer | Repos in one workspace |
| `POST /workspaces/{id}/repos` | ws editor | `AddRepoReq` (`path` or `clone_url`; clone is async) |
| `POST /workspaces/{id}/repos/detect` | ws editor | Resolve + register the repo containing a path |
| `DELETE /repos/{id}` | ws editor | Unregister only — never deletes files |

### Local operations (#37–#47 + graph/merge extras)

| Method & path | Auth | Notes |
|---|---|---|
| `GET /repos/{id}/status` | ws viewer | `RepoStatusResp` |
| `GET /repos/{id}/branches` · `/refs` · `/log` | ws viewer | Branches, refs, commit log |
| `GET /repos/{id}/diff?target=worktree\|staged\|commit:<sha>\|range:<a>..<b>` | ws viewer | `DiffResp` |
| `POST /repos/{id}/stage` · `/unstage` · `/discard` | ws editor | `StagePathsReq` → `RepoStatusResp` |
| `POST /repos/{id}/commit` | ws editor | `CommitReq` (`amend`) → `{sha}` |
| `POST /repos/{id}/push` · `/pull` · `/fetch` | ws editor | Returns fresh `RepoStatusResp` |
| `POST /repos/{id}/checkout` | ws editor | `CheckoutReq` (`create`) |
| `POST /repos/{id}/cherry-pick` · `/revert` | ws editor | `{sha}`; conflict → 502 with git stderr |
| `POST /repos/{id}/branch` · `/branch/rename` · `/branch/delete` | ws editor | Create / rename / delete branch |
| `POST /repos/{id}/tag` · `/tag/push` · `/tag/delete` | ws editor | Create / push / delete tags |
| `POST /repos/{id}/stash` | ws editor | `{op: save\|pop}` |
| `POST /repos/{id}/merge` · `/merge/preview` · `/merge/abort` · `/merge/commit` | ws editor/viewer | Local merge + conflict lifecycle |
| `GET /repos/{id}/merge/status` · `/conflict` | ws viewer | Merge state / one file's conflict |
| `POST /repos/{id}/conflict/resolve` | ws editor | `ResolveConflictReq` |

### Pull requests (#48–#56 + extras)

| Method & path | Auth | Notes |
|---|---|---|
| `GET /repos/{id}/prs?state=open\|merged\|declined\|all` | ws viewer | `PrSummary[]` |
| `POST /repos/{id}/prs` | ws editor | `CreatePrReq` → `PrSummary` |
| `GET /repos/{id}/prs/{number}` | ws viewer | `PrDetail` (comments, reviewers, CI, mergeable) |
| `GET /repos/{id}/prs/{number}/diff` | ws viewer | `DiffResp` |
| `PATCH /repos/{id}/prs/{number}` | ws editor | `UpdatePrReq` (title/description) |
| `POST /repos/{id}/prs/{number}/comments` | ws editor | `NewPrCommentReq` (general / inline / reply) |
| `POST /repos/{id}/prs/{number}/approve` · `/request-changes` · `/merge` · `/decline` | ws editor | Review + merge actions |
| `GET /repos/{id}/prs/{number}/commits` | ws viewer | PR commits |
| `POST /repos/{id}/pr/draft` | ws editor | `DraftPrReq{base}` → `DraftPrResp{title,description,source_branch,target_branch}` |
| `POST /repos/{id}/draft-commit-message` | ws editor | Drafts a commit message from the staged diff |

PR-review-agent endpoints (the Review tab, multi-agent code review) are covered
in **[code-review.md](./code-review.md)**.

---

## 9. Capabilities & limitations

**You can:**

- Use GitHub, Bitbucket Cloud, and GitLab — including **GitHub Enterprise** and
  **self-hosted GitLab** (auto-detected by host name; set GitLab's API base URL).
- Browse, clone (async, with a remembered destination), register local repos,
  and keep many repos open as persistent tabs.
- Stage/unstage/discard, commit (and amend), diff, walk the graph, branch, tag,
  cherry-pick, revert, and stash — all in-app.
- Run a **local merge** and resolve conflicts visually, with a dry-run preview.
- **Open a PR** with the branch pushed for you and the title/description drafted
  by an agent from the branch diff.
- List/read/comment/approve/request-changes/merge/decline PRs across all three
  forges, with CI status and mergeability shown.
- Push/pull over HTTPS using a Keychain-stored token, or over SSH via your agent.

**You cannot (by design / current behavior):**

- Create a provider **draft PR** — PRs are always opened ready (see §7).
- Use providers other than GitHub / Bitbucket Cloud / GitLab. Unrecognized hosts
  have no PR client; **Bitbucket Server / Data Center is not auto-detected.**
- Push to a **protected branch** directly. In this repo `main` is PR-protected —
  **default to opening a PR over `main`**, not pushing to it.
- Use another user's bound git credential. A repo binds exactly one account, and
  its token may be **used only by the account owner or root** (the "S4" guard),
  even for other members of the same workspace.
- See a token after saving it — the Token field is write-only; rotate by entering
  a new value.

---

## 10. Security

- **Tokens never live in the repo or in plain files.** Each git-account token is
  stored as a macOS Keychain generic-password item under service
  `com.otto.daemon`, keyed `gitacct-<id>`. The SQLite DB stores only the opaque
  `token_ref`, never the secret. (`OTTO_SECRETS=file` swaps in a file-backed
  store for non-macOS dev only.)
- **Write-only entry.** The UI never reads tokens back; `GET /git/accounts` omits
  them. Editing without a new token keeps the old one; supplying one rotates it
  and deletes the prior Keychain entry. Deleting an account deletes its secret.
- **Token never on disk or in URLs during push.** HTTPS auth goes through a
  temporary `GIT_ASKPASS` script + the `OTTO_GIT_TOKEN` env var; any
  `user:pass@` embedded in a URL is stripped from logs/notices.
- **Credential-use ownership (S4).** A repo's bound token is usable only by its
  owner or root — workspace membership alone does not grant push/PR rights through
  someone else's credential.
- **Loopback only.** `ottod` listens on `127.0.0.1:7700` unless you explicitly
  enable a network listener.

---

## 11. Troubleshooting

| Symptom | Likely cause & fix |
|---|---|
| **403 / "Bad credentials" creating or listing a PR** | Token missing the right scope. GitHub: `repo` (classic) or Contents+Pull-requests read/write (fine-grained). Bitbucket: Pull-requests read/write. GitLab: `api`. Re-add/edit the account with a fresh token. |
| **"repo has no git account" (400) on PR routes** | The repo isn't bound to an account, or its provider doesn't match the account's. Bind a matching account (Add-Repository sheet or re-detect). |
| **403 even though the token is fine** | You're not the account **owner**. A repo's credential is usable only by the owner or root (S4). Have the owner act, or bind your own account. |
| **Push rejected / auth failed** | HTTPS token lacks write/Contents scope, or the token expired (check the expiry chip on the account). For SSH remotes, fix your SSH agent — no token is used. |
| **"failed to push … protected branch"** | The target branch is protected (e.g. `main`). Push a feature branch and **open a PR** instead. |
| **Draft button errors with "no changes between …"** | The current branch has no diff vs the chosen target. Pick a different target or commit something first. |
| **Clone "destination already exists" (409)** | The chosen folder already contains `<name>`. Pick a different clone directory or remove the existing one. |
| **PRs/clone work but "Browse remote" is empty** | Set the account's **Organisation / Workspace / Group** namespace; the picker needs it. |
| **Expiry shows "expired" but I rotated the token** | GitHub/GitLab auto-detect expiry; if a header isn't present (e.g. classic PAT without expiry) set/clear the date manually on the account. |
| **Self-hosted GitLab not recognized** | Ensure the host name contains `gitlab`, or set the account's **API base URL** to the instance API root. |

---

## 12. Related docs

- **[AI code review](./code-review.md)** — the **Review** tab, multi-agent PR and
  local-working-tree review, findings lifecycle, and merge-readiness.
- **[SSH & SFTP connections](./connections-ssh-sftp.md)** — browsing remote files
  over SSH. SFTP is a **separate feature** from this Git module.
