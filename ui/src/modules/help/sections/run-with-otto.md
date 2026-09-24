---
id: run-with-otto
title: Run with Otto
group: Work
route: run-with-otto
summary: One button that turns a Jira story, GitHub issue or PR, Confluence page, finding, failing test or plain request into a reviewed, evidence-backed PR draft.
---
## What it's for

Run with Otto chains the pieces Otto already has into one fixed pipeline. You bring a source — a ticket, a link, an id or a sentence — and Otto fetches it, builds the task, works on an isolated branch, assembles a proof pack, runs an AI review, and then stops for your approval before it drafts the PR.

Use it for "turn this into a reviewed PR draft". For a custom multi-step process with your own gates, build a [Workflow](#/walkthroughs/workflows) instead.

## Getting started

1. Make sure the workspace has at least one git repository registered (Git page, or **Browse…** in the launcher).
2. Open **Run with Otto** from the sidebar.
3. Paste a source into the big field — for example `PROJ-123`, a GitHub pull request or issue URL, a Confluence page URL — or just describe what you want. Otto shows what it detected as you type.
4. Choose **Single agent** or **Goal loop**, and optionally a repository, provider and model.
5. Choose **Run with Otto**, or press `⌘Enter`.
6. Follow the run in the list and its detail panel. When it reaches **Awaiting approval**, choose **Approve** or **Reject**.
7. After approval Otto writes the PR draft. Choose **Open PR** to open the real pull request.

## Everything it can do

**Sources** (click a chip to insert its prefix)
- **Jira:** a bare key (`PROJ-123`) or `jira:PROJ-123` — summary, description and top comments.
- **Confluence:** a page URL or `confluence:<page id>`.
- **GitHub PR:** a `…/pull/42` URL — description and discussion.
- **GitHub issue:** a `…/issues/9` URL.
- **Story:** `story:<id>` from the Product module.
- **Finding:** `finding:<id>` from a code review — title, evidence and suggested fix.
- **Failing test:** `test:<id>`, a Product test-case run and its failing cases.
- **Report:** `report:<id>`, a Scheduled Tasks report.
- **Anything else** becomes a free-text run whose goal is your text.

**Pipeline** (the stage rail shows where each run is)
- **Source:** fetch and normalize the item.
- **Context:** assemble the task prompt.
- **Branch:** create an isolated `otto-run/<id>` branch and worktree. Your own checkout is never touched.
- **Execute:** one agent makes the change (Single agent), or a Plan → Execute → Evaluate loop iterates until the goal is met (Goal loop).
- **Proof:** a proof pack built from the change, with a status and risk score.
- **Review:** an AI review of the branch; findings and blocking findings are counted.
- **Approval:** the one human gate. Nothing ships without you.
- **PR draft:** a title and description, with the branch pushed when a git account is set up.

**Launch options**
- **Repo:** "Auto" uses the repo the source implies (a GitHub remote, a finding's repo), else the workspace's first repo. **Browse…** registers any folder inside a git repo and selects it.
- **Provider** and **model:** any registered agent provider except the plain shell. Claude runs headless; other providers run as a real session you can open. The model defaults to the provider's default.

**Runs list and detail**
- Each run shows its source, title, status, stage rail, proof status, findings and blocking findings, and the provider and model.
- The detail panel shows the goal, a link to the source, the branch, a stage timeline with messages, the approval gate, the PR draft, and the result.
- **Reject** asks for an optional reason and ends the run. The branch and its commits are kept.
- **Cancel run** is available until a run finishes (Otto asks first).
- Each run also appears in Mission Control.

**Other ways to start a run**
- **Slack or Telegram:** send `/run <source>` in a connected channel. Otto posts status, the approval prompt and the result in the thread; reply `approve` or `reject` there.
- **Webhook and REST:** start runs from other tools with the workspace's webhook key, and get the result posted back to a callback URL.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `⌘Enter` | Launch the run from the source field |

## Tips and limits

- Needs the **Run with Otto** feature: view access to see runs, edit access to launch, approve, reject, cancel or open a PR. Your workspace role applies too.
- A run always works inside a registered git repository. With none, the run fails with "no git repo registered in this workspace".
- **Open PR** works only after approval and when the proof pack passed or was waived.
- The **Auto-open PR** checkbox is saved with the run, but runs still stop at the PR draft. Use **Open PR** to open it.
- If the PR draft says the branch wasn't pushed, connect a git account for that repository. The title and description are still written.
- GitHub is the only issue tracker read for issues. GitLab and Bitbucket pull requests are read; their issues aren't.
- A Slack or Telegram run uses only the message that started it, not the whole thread history.
- Runs caught mid-work by a daemon restart are marked failed. The branch keeps its commits; launch again.

## Related

- [Mission Control](#/walkthroughs/mission-control)
- [Proof](#/walkthroughs/proof)
- [Goal Loops](#/walkthroughs/loops)
- [Workflows](#/walkthroughs/workflows)
- [Git](#/walkthroughs/git)
- [Product](#/walkthroughs/product)
