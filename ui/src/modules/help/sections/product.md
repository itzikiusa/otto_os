---
id: product
title: Product
group: Build
route: product
summary: Turn a Jira issue, a Confluence page or a blank draft into an analysed, planned and tested story, then hand it to agents.
---

## What it's for

Product is the product owner's workspace. Import a Jira issue or Confluence page, or start a blank draft. Agents then analyse it, raise questions, suggest a rewrite, write test cases and plan the work. You can post the results back to Jira and Confluence, send the plan to a swarm, or open a coding agent with the full context.

## Getting started

1. Add your Atlassian account in **Settings → Jira** (a label, your Atlassian site URL, your email and an API token).
2. Open **Product** in the sidebar and choose **Import**. Pick the account, choose **Jira issue** or **Confluence page**, then search by project or space, or paste a key or URL.
3. Optionally set a **Repo path** so agents can map the story to your code, and tick **Watch this story for changes**.
4. Open the story and go to **Discover → Analysis**. Choose lenses and providers, then **Run analysis**.
5. Answer the open questions in **Questions**, then use **Deliver → Plan** to break the work into tasks.
6. Choose **Send to Swarm**, or open **Inject** to start a coding agent with everything it needs.

## Everything it can do

**Stories and the list**
- Import Jira issues and Confluence pages by searching a project or space, or by pasting a key, a `/browse/` URL or a page ID.
- **New ▾** creates a **Draft** (a blank story) or an **Epic**. Inside an epic, add a **Story** or a **Doc** child and file it in a folder such as Design, PO or QA.
- Right-click a story to move it to an epic, set its folder, detach it, mark or unmark it as an epic, make it a doc or a full story, or delete it. Deleting an epic moves its children to the top level.
- Filter the list by tag. Switch between **Stories** and **Learnings** at the top.
- Stories and learnings are one shared library across all workspaces.

**Story tabs**
- **Story → Overview**: the story body, with live Jira details for Jira stories. For a draft it becomes an editor with a place to add transcripts. An epic shows a board of its children.
- **Story → Rewrite**: generate a suggested rewrite (Jira stories use the story writer, others the RFC writer), compare it word by word with the source, and publish it back.
- **Story → Design**: the design arena for screens, boards, diagrams and 3D scenes, with the designs from Design Hall that implement the story.
- **Discover → Chat**: a discovery chat for early ideas. It can see your draft, designs, open questions and notes, and proposes action cards (fill the draft, add questions, add notes, open a diagram) that you apply yourself.
- **Discover → Analysis**: run the PO Overview, Architecture and Clarifying Questions lenses on one or more providers. Each lens is an openable session you can stop or retry. A summarizer merges the results into related repos, functionalities, integration points, risks, open questions and suggested learnings.
- **Discover → Questions**: filter by status and category, then edit, answer, discard or delete. Tick questions to post them as one comment on the Jira issue or Confluence page.
- **Discover → Notes**: private notes on the story.
- **Discover → Discovery**: run a multi-agent discovery investigation and read its report, task summaries and board messages.
- **Discover → Refine**: chat threads that refine the story, including threads started from a discovery run.
- **Deliver → Plan**: one or more planning agents write a task-by-task plan. They don't ask questions unless you untick **Don't ask me questions**. Tick items to track progress. **Send to Swarm** creates or reuses a swarm project with one task per plan task.
- **Deliver → Test Cases**: generate test cases grouped as happy path, validation, error and edge cases. Approve, request changes, edit, reorder and bulk-approve them, then publish the approved ones to a Confluence page.
- **Deliver → Inject**: build a context bundle (story, analysis summary, answered questions, approved test cases, learnings and plan) and open it in a new agent session.
- **Log → History**: every event on the story, filterable by section.
- A doc child shows only the lighter tabs.

**Publishing**
- Publish a rewrite over the live Jira issue or Confluence page. Otto asks first, because it replaces the existing text.
- Publish a draft as a Jira story or a Confluence RFC. The draft then links to what it created.
- Post questions as one combined comment. Publish approved test cases to a page titled "Test Cases — <story title>".

**Learnings**
- A knowledge base of patterns to follow and cases to avoid, with tags.
- Analysis suggests learnings; they stay pending until you choose **Accept**. Turn any learning on or off.
- Active learnings are included in every agent run.

**Watching and design**
- A watched story is checked every few minutes (at least 5). New comments are recorded and matched to open questions, and you get a notification.
- The Design tab creates screens (HTML in device frames), Excalidraw boards, Mermaid diagrams and 3D scenes, each with an assistant that edits it. Export PNG, SVG, the source file, GLB or a Blender script. With Blender installed, **Render + export GLB** attaches a rendered image and a GLB of a 3D scene.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `Enter` | Send a message in Chat or Refine |
| `⇧Enter` | New line in Chat or Refine |
| `W` / `E` / `R` | Move, rotate or scale in the 3D viewport |
| `F` | Frame the selection in the 3D viewport |
| `⌘D` | Duplicate the selected 3D object |
| `Delete` or `⌫` | Delete the selected 3D object |
| `Esc` | Deselect in the 3D viewport |
| `F2` | Rename the selected item in the 3D hierarchy |

## Tips and limits

- Access needs the **Product** feature. Importing, searching and posting back also need the **Issues** feature. Reading needs the workspace Viewer role; changes need Editor.
- Only Atlassian (Jira and Confluence cloud) is supported. Confluence uses the same account and token as Jira.
- You can only publish through your own Atlassian account. Tokens are stored in the macOS Keychain.
- The Architecture lens needs a **Repo path**; without one it has no code to look at.
- Publishing test cases from a Jira story needs a Confluence space key.
- Epics nest one level deep. Drag and drop in the tree isn't available; use the right-click menu.
- Ticking plan items updates the latest plan in place; it doesn't create a new version.
- Questions are always posted as Markdown.

## Related

- [Swarm](#/walkthroughs/swarm)
- [Design Hall](#/walkthroughs/design)
- [Agents](#/walkthroughs/agents)
- [Git](#/walkthroughs/git)
- [Vault](#/walkthroughs/vault)
- [Run with Otto](#/walkthroughs/run-with-otto)
