---
id: mission-control
title: Mission Control
group: Work
route: mission-control
summary: One live work graph of everything agents are doing in this workspace — status, risk, cost, approvals, links and evidence.
---
## What it's for

Mission Control shows every piece of agent work in the current workspace as one traceable item: sessions, swarm projects, goal loops, workflow runs, code reviews, product stories, pull requests, channel-triggered sessions and Run with Otto runs. Use it to see what is running, what is waiting on approval, what it has cost so far, and how the pieces relate.

The graph builds itself from the other modules. You add approvals, notes and risk levels; you don't create items by hand.

## Getting started

1. Open **Mission Control** from the sidebar.
2. If the page is empty, choose **Refresh** to build the graph from existing work in every module.
3. Check the tiles at the top: work items, active, needs approval and total cost.
4. Filter by kind, status or risk, or search by title.
5. Click an item to open its detail panel. Approve or reject pending gates there.
6. Switch to **Graph** to see items as nodes with their links.

## Everything it can do

**Summary tiles**
- Work items, Active, Needs approval (highlighted when above zero) and Total cost in USD.

**Filters and views**
- **Kind:** Session, Swarm Project, Goal Loop, Workflow Run, PR Review, Product Story, Pull Request, External Trigger.
- **Status:** Pending, Running, Waiting, Blocked, Succeeded, Failed, Cancelled, Done. Each module's own states are mapped onto these 8.
- **Risk:** Low, Medium, High, Critical.
- **Search** by title. **Clear** resets every filter.
- The list and the graph always show the same filtered set (up to 300 items).
- **List:** kind, title, repo and owner, a Needs approval badge, status, risk, cost and last activity.
- **Graph:** one column per kind. Node colour shows status, node size grows with cost, and a dashed ring marks items waiting on approval. Links are labelled with their relation (spawned, depends on, fixes, reviews, verifies, blocks, belongs to). Click a node, or focus it and press `Enter`, to open it.
- The page updates live as work changes in this workspace.
- **Refresh** rebuilds the graph from every source and refreshes session costs.

**Detail panel**
- Status, risk and a Needs approval count. **Open session** jumps to the live terminal for sessions and channel-triggered sessions.
- Owner (user, agent, system or integration), cost so far, repository, branch, created and updated times.
- **Goal & context:** **Edit** to change the goal, the result summary and the risk level, then **Save**.
- **Approvals:** every gate with its reason and who asked. **Approve** or **Reject** a pending gate, or **Request approval** with an optional reason.
- **Relations:** linked items in both directions. Click one to open it.
- **Evidence:** diffs, commits, pull requests, test runs, reports, files, links, findings and sessions attached to the item.
- **Timeline:** the full audit trail, newest first, with who did what.
- Drag the panel's edge to resize it; double-click the edge to reset its width. The width is remembered.

**Elsewhere in Otto**
- A Mission Control widget on [Home](#/walkthroughs/home) shows the same summary and the most recently updated items.
- Work items awaiting approval also appear under **Needs you** on Home.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `Enter` / `Space` | Open the focused node in the Graph view |

## Tips and limits

- Needs the **Mission Control** feature: view access and the workspace Viewer role to see items; edit access and the Editor role to edit, approve, reject, request approval or refresh.
- The graph covers the current workspace only. Switch workspaces to see others.
- Risk is set automatically when an item is created — titles mentioning things like security, payment, auth, secrets, production, deploy or migration start at High — and you can change it afterwards.
- A state Otto doesn't recognise counts as Running, so an item never drops out of the active view.
- Run with Otto runs appear in the list and graph, but the Kind filter doesn't offer them yet.
- On narrow windows the detail panel opens full screen.

## Related

- [Run with Otto](#/walkthroughs/run-with-otto)
- [Agents](#/walkthroughs/agents)
- [Swarm](#/walkthroughs/swarm)
- [Goal Loops](#/walkthroughs/loops)
- [Workflows](#/walkthroughs/workflows)
- [Proof](#/walkthroughs/proof)
- [Home](#/walkthroughs/home)
