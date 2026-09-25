---
id: browser
title: Browser
group: Infrastructure
route: browser
summary: Read web pages as clean text or open them live, mark elements, and hand a page and its marks to an agent.
---
## What it's for

The Browser page is a small, workspace-scoped web browser built for working with agents. Each tab runs in one of two modes:

- **Reader** — Otto's daemon fetches the page and shows it as clean, readable text. Good for docs, articles and anything you want to summarize or quote.
- **Live** — a real browser. In the desktop app it can be this Mac's web view; anywhere else (a remote session, your phone) it is a Chromium that Otto runs and streams to you.

You can **mark** elements on a page, add a note to each mark, and ask an agent about the page with those marks attached. Tabs and marks are saved per workspace.

## Getting started

1. Open **Browser** in the sidebar (Infrastructure group).
2. Type a URL in the address bar and press ↵. A missing `https://` is added for you. The page opens as a reader tab.
3. To mark something, choose **Mark passage** above the page, click a passage, write a note and choose **Save mark**. The mark appears in the **Marks** rail.
4. To ask an agent, use the dock under the page: **New agent** starts an agent session, or **Attach…** binds one you already have. Type your question in the ask bar and press ↵.
5. To see the real page, switch the tab to **Live** with the globe button next to the address bar.

## Everything it can do

**Tabs and navigation**
- Tabs ride in the page header. The **+** button starts a new tab; the × on a tab closes it.
- Tabs are saved per workspace, so they come back when you return.
- The toggle next to the address bar switches the current tab between **Reader** (document icon) and **Live** (globe icon).

**Reader mode**
- The daemon fetches the page, strips it to its readable content and renders it as markdown.
- JavaScript-rendered pages use the Lightpanda headless browser when it is installed on the Mac running Otto. Without it, Otto falls back to a plain fetch and shows **Degraded fetch — no JavaScript ran**, meaning some content may be missing.
- A site that fails with Lightpanda 3 times in a row goes straight to the plain fetch until it next succeeds.
- Every fetch is anonymous: no cookies or sign-ins are carried over, so pages behind a login show their logged-out view.

**Live mode**
- **In the desktop app**, the chevron next to the mode toggle picks the live engine for this device:
  - **This Mac's web view** — fastest, desktop app only, and agents can't drive it.
  - **Otto's Chromium** — streams to any device, uses Otto's network guard, and supports agent control.
- **Everywhere else** live tabs always use Otto's Chromium.
- With Otto's Chromium you get **Back**, **Forward** and **Reload page** buttons, a frames-per-second and latency meter, and a **Show keyboard** button for phones.
- Click the page (or Tab onto it) to give it the keyboard. **Esc** hands the keyboard back to Otto; **⇧Esc** sends Esc to the page. Otto's own shortcuts such as ⌘K and ⌘T keep working.
- Page dialogs (alert, confirm, prompt, leave page) appear as cards you answer in Otto.
- A page that opens a new window shows a notice with **Open in new tab**.
- Requests to this Mac, your local network or cloud metadata addresses are blocked, with a notice naming the host.
- Files a page downloads are kept in a quarantine folder and never opened, or blocked, depending on Settings → Browser.
- If the session ends you can **Reconnect** or **Switch to Reader**.

**Enabling Otto's Chromium**
- The first live tab that needs it shows **Enable live browsing**. An admin downloads **Chrome for Testing** (about 180 MB) or, with **Use the lighter engine instead**, `chrome-headless-shell` (about 98 MB). The download is checked against a pinned checksum and stays on the Mac running Otto.
- Progress shows while it downloads, verifies and unpacks. You can keep working meanwhile.
- Non-admins see who can enable it, with **Switch to Reader** as the way forward.

**When an agent drives a live page**
- A bar says **An agent is driving this page**. **Take over** pauses the agent and gives you control; **Hand back** returns it.
- Before an agent submits a form or makes a similar outward request, the page holds and an approval card shows what would be sent. Choose **Approve**, **Take over** (deny and drive yourself) or **Deny…**. A screenshot from just before the request is saved with the approval.
- If you only have view access you can watch but not drive.

**Marks**
- Reader: **Mark passage**, then click an element and add a note.
- Live (this Mac's web view): the target button **Pick an element to mark**, then click in the page.
- Marks are tied to the page URL, so they reappear on any tab that opens the same page and are highlighted on the page.
- In the **Marks** rail you can edit a note, delete a mark, or send one mark into a live agent session.

**Asking an agent**
- The agent dock under the page shows the agent's real terminal; you can type to it directly. Drag its top edge to resize it, or collapse it with the chevron. **Detach** leaves the session running in Agents.
- **New agent** starts an ordinary agent session (it also appears in Agents) with Otto's browser tools enabled. With more than one provider you can pick which agent to start.
- The ask bar sends the page URL, your question and, by default, the page's newest 20 marks (untick the marks chip to leave them out). Marking an element focuses the ask bar.
- The same Browser is available inside an agent session's right panel (**Browser** tab, **v2**), where the ask bar targets that session.

**Page tools**
- **Summarize** — one short agent turn condenses the page into a few sentences.
- **Save to vault** — writes the page summary and its marks as a note in a Vault. With several vaults you pick one.
- **Autofill saved credentials** (key button) — appears on a live page in this Mac's web view when a saved site credential matches the domain and the page has a password field. It fills the form after you confirm; nothing is submitted for you.

**Agents' browser tools**
- Every agent session gets these Otto tools:
  - `browser_navigate` opens a reader tab in this workspace's Browser.
  - `browser_page` returns a page as markdown, with the engine used and whether it was degraded.
  - `browser_query` returns the elements that match a CSS selector.
  - `browser_summarize` summarizes a page.
  - `browser_marks` lists your marks and notes, so "the element I marked" makes sense to the agent.
  - `browser_login` signs in with a stored site credential, but only one you marked **allow agent use**. The password never appears in the tool call or its result.
- Page content sent to an agent is fenced as untrusted, so a page can't pose as instructions from you.

**Settings → Browser**
- **Live tabs on this device** (desktop app): this Mac's web view or Otto's Chromium.
- **Browser engine** (admin): Chrome for Testing or the lighter engine, each with a **Download** button until it is installed.
- **Show the window on this Mac** (admin): also opens the live browser as a visible Chrome window. Needs Chrome for Testing.
- **Files pages download** (admin): **Keep in quarantine** or **Block**.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| ↵ | Open the URL in the address bar |
| ⌘L | Focus the address bar (while a live page in Otto's Chromium has the keyboard) |
| ⌘R | Reload the live page (while a live page in Otto's Chromium has the keyboard) |
| ⌘V | Paste into the live page |
| Esc | Give the keyboard back to Otto from a live page |
| ⇧Esc | Send Esc to the live page |
| ↵ | Ask bar: send the question |
| ⇧↵ | Ask bar: new line |

See [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts) for the global ones.

## Tips and limits

- **Access:** the page needs the Browser feature. Reading needs view access; opening pages, marking and driving live pages need edit access. Downloading the Chromium engine and changing engine settings need Browser admin.
- Loopback, private-network and cloud metadata addresses are refused, in both reader and live mode. This is intentional.
- Reader fetches are capped at 2 MB and 30 seconds.
- For JavaScript-heavy pages in reader mode, install Lightpanda on the Mac running Otto (`brew install lightpanda-io/tap/lightpanda`). It is beta software, so some pages still come back degraded.
- Otto's Chromium runs on Apple silicon Macs only for now. A daemon without the live engine shows reader view with **Open in new tab**.
- A live session in Otto's Chromium is private to you (and workspace admins). Switching a tab back to Reader frees its Chromium session.
- Element picking in live mode works only in this Mac's web view, not in Otto's Chromium.
- The page has no screen for managing site credentials yet, so autofill and `browser_login` only work with credentials that already exist. `browser_login` always starts at `https://<domain>/`, needs Lightpanda, and allows 3 attempts a minute per domain.
- Otto's `browser_*` agent tools work on fetched pages. They don't click or type in a live tab.

## Related

- [Vault](#/walkthroughs/vault)
- [Agents](#/walkthroughs/agents)
- [MCP Control Plane](#/walkthroughs/mcp)
- [Settings](#/walkthroughs/settings)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
