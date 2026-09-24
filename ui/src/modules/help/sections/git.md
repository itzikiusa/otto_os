---
id: git
title: Git
group: Build
route: git
summary: A full git client with pull requests, AI code review and a merge-conflict resolver, for GitHub, Bitbucket Cloud and GitLab.
---

## What it's for

Git is where you work with repositories without leaving Otto. You can stage and commit, read diffs, walk the commit graph, resolve conflicts, and open, review and merge pull requests. Agents can draft commit messages and PR descriptions, and a team of review agents can check a PR or your uncommitted changes.

## Getting started

1. Open **Git** in the sidebar and choose **Add Repository**. You can register a folder already on disk, clone a URL, or browse the repositories of an account.
2. To work with pull requests or push over HTTPS, add a token in **Settings → Git Accounts** → **Add Account**. Pick GitHub, Bitbucket or GitLab, enter your username and token, and use **Test connection** to check it.
3. Open the repository tab. The **Graph** tab shows the commit graph with a **WIP** row for your uncommitted changes.
4. Select the WIP row, tick files to stage them, write a message (or choose **Draft** to have an agent write it), then choose **Commit**.
5. Choose **Push**, then open the **Pull Requests** tab and choose **New PR**. **Draft message with agent** fills in the title and description from your branch diff.

## Everything it can do

**Repositories and tabs**
- Keep many repositories open as tabs; tabs and the selected sub-tab are restored when you restart Otto.
- Add a repository three ways: a local folder, a clone from a URL (the clone folder is remembered), or **Browse remote** under an account's organisation, workspace or group.
- Switch repositories from the repository name menu. Removing a repository only unregisters it; files on disk are never touched.
- Each repository has the **Graph**, **Pull Requests**, **Review** and **Focus** tabs. A **Resolve conflicts** tab appears while an operation has conflicts.
- Auto-fetch keeps open repositories fresh in the background. It pauses in hidden windows, and you can turn it off from the tab strip. Fetch never checks out or pulls.
- **Remotes**: list, add, change the URL of, and remove remotes.
- **Recovery tools**: browse earlier HEAD positions and create a recovery branch, plan an interactive rebase (pick, squash into previous, edit, reorder), or run a bisect (Works, Broken, Cannot test).

**Commit graph**
- Branch and tag chips on the graph, with ahead/behind counts for every local branch.
- Commit search runs on the daemon over the whole history. Search by message or switch to **Author**. Matches are literal and case-insensitive.
- Right-click a commit, branch, tag or stash for actions: check out, create a branch or tag here, cherry-pick, revert, merge into, rebase onto, pull, delete local or remote, push or delete a tag, apply or drop a stash, copy SHA or message, start a GitFlow feature, release or hotfix branch, update submodules, and create a pull request.
- Set a branch as the cleanup base to mark merged branches as safe to delete.
- Linked worktrees are shown in their own **Worktrees** section. Opening one opens it as its own tab, instead of checking out the branch.
- **History** and **Blame** for any file, from the ⋯ menu on a diff's file header. History follows renames.

**Working tree (WIP panel)**
- Stage, unstage or discard a file, a whole folder, or a whole section.
- Stage, unstage or discard a single hunk. Select lines in the gutter (shift-click for a range) to stage only those lines.
- Every discard asks first. Discarding a hunk records a backup stash.
- Commit with **Amend** and **Sign** options. **Draft** writes the message from your staged changes, and **Watch agent** shows the drafting session live.
- Toolbar: **Fetch**, **Pull** (choose fast-forward only, merge or rebase), **Push** (sets the upstream for a new branch), **Stash**, **Pop stash** and create a branch.
- When a dirty working tree blocks a pull or a branch switch, Otto offers to stash, retry and restore your changes.

**Conflicts**
- Merge, rebase, cherry-pick, revert and stash-pop conflicts all open the same resolver.
- Pick hunks, take a whole side (ours or theirs), keep your edited file, or delete the path. Binary files and delete-versus-modify conflicts are supported.
- **Complete** finishes the operation once every file is resolved. **Abort** cancels it and never falls back to a hard reset.
- Merge preview checks whether a merge would conflict without changing anything.

**Pull requests**
- Filter the list by open, merged, declined or all.
- **New PR** pushes the branch first, then opens the pull request. Options: **Draft message with agent**, **Open as draft** (remembered per repository), and reviewers with typeahead from the provider's members.
- The PR page has **Summary**, **Files**, **Commits** and **Review** tabs. You can comment (inline, general or reply), approve, request changes, decline, edit the title and description, and see CI checks.
- **Merge** first shows a readiness check: CI checks, approvals, mergeability, open blocker findings and unpushed commits. Choose merge, squash or rebase, and whether to delete the source branch.
- **Open as session** opens the pull request in an agent session of your choice. **View on provider** opens it in the browser.
- The agent drafts for commit messages and PRs use the fast draft model set in **Settings → Providers**. When the `commit-message` and `pull-request` skills are installed, drafts follow them.

**AI code review**
- **PR review**: on a pull request's **Review** tab, optionally attach a Jira story and write what the reviewers should focus on, then choose **Send to review agents**.
- **Local review**: on the repository's **Review** tab, pick a base in **Compare to** and choose **Review changes** to review your uncommitted work.
- Each lens runs as a real agent session on each provider you choose (claude, codex, agy). **Open** shows its terminal; **Stop** and **Retry** act on one agent. A summarizer merges and ranks the findings.
- **Configure** edits the review agents: add a lens from your installed review skills or a preset, choose providers and a model, edit the instructions, and save your own presets. The same agents run local reviews.
- PR mode: **Approve** posts a finding as a PR comment; **Decline** discards it. A merge-readiness banner counts the blockers.
- Local mode: select findings and choose **Send to agent** to start a fix session.
- The findings board tracks each finding across runs: **Ask agent to fix**, **Verify resolved**, **Convert to Jira**, **Mark false positive**, **Require human approval**, **Add to repo rule**, **Add regression test**, plus **Accept**, **Waive**, **Approve** and **Reject** in the ⋯ menu.
- Earlier runs stay under **Past reviews**.

**Focus**
- **Focus** shows your pull requests across all repositories (all open, or opened by you) and your Jira work, with a quick view of each issue.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `⌘F` | Search commits (in the graph) |
| `Enter` | Jump to the first commit search result |
| `Esc` | Close the search results, then clear the search |
| `]` or `n` | Next file in a diff |
| `[` or `⇧N` | Previous file in a diff |
| `Esc` | Cancel the new-branch box |

## Tips and limits

- Supported providers are GitHub (including Enterprise), Bitbucket Cloud and GitLab (including self-hosted). Bitbucket Server and Data Center have no pull request support; the tab says so.
- Local work (status, diff, stage, commit, branch, local merge) needs no account. Push, pull over HTTPS and pull requests need an account bound to the repository. SSH remotes use your SSH agent instead of a token.
- Tokens are stored in the macOS Keychain and are never shown again after you save them. Only the account owner (or root) can use a repository's token.
- Access needs the **Git** feature. Reading needs the workspace Viewer role; any change needs Editor.
- Reviewers can't be edited on an existing pull request, and team or CODEOWNERS reviewers aren't supported.
- Review agents are read-only. The summarizer runs on claude, so a signed-in claude CLI is needed for the merged result.
- Review configuration is global and only root can save it.
- A PR diff sent to reviewers is capped at 200 KB.
- Git commands time out after 30 seconds (local) or 180 seconds (remote). If a local write fails, check for a leftover `.git/index.lock`.

## Related

- [Proof](#/walkthroughs/proof)
- [Product](#/walkthroughs/product)
- [Agents](#/walkthroughs/agents)
- [Run with Otto](#/walkthroughs/run-with-otto)
- [Workflows](#/walkthroughs/workflows)
- [Skills Lab](#/walkthroughs/skills-eval)
