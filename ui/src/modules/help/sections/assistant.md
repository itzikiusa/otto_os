---
id: assistant
title: Assistant
group: Work
route: assistant
summary: Otto's personal assistant — chat, remember, remind and run tasks on your Claude and Codex subscriptions, asking before anything leaves your Mac.
---
## What it's for

The Assistant is one place to ask, plan and get things done. Each thread is a conversation backed by a real Claude or Codex CLI session, so it runs on the subscriptions you already have — no API keys. Otto remembers what matters about you, sets reminders, tracks longer jobs as tasks, and stops to ask before it sends, posts, publishes, buys or touches production.

## Getting started

1. Open **Assistant** from the sidebar and choose **Start a conversation**. Otto creates your first space, "Personal".
2. Type in **Reply to Otto…** and press `Enter`. Use `⇧Enter` for a new line.
3. To send one message to a specific provider, start it with `@claude` or `@codex`. The rest of the thread keeps its model.
4. To keep a thread on one model, choose the model chip in the composer (it says **Auto** by default) and pick **Pin to thread**.
5. Ask for something that takes a while ("remind me at 5", "find a cheaper flight"). It appears as a card in the chat and on the **Tasks** tab.
6. Open **Settings → Assistant** (the gear in the toolbar) to set which provider and model handle which kind of request.

## Everything it can do

**Threads and spaces**
- The thread list has 4 spaces (01–04) — threads you come back to, like Personal or Work — plus **Recent** for everything else, newest first.
- Choose an empty space slot to name and start it, or **+** to start a plain thread.
- The page opens on the thread you last used, or the latest one.
- The toolbar shows the thread's space, its provider and model, and whether it is pinned or "routed by your rules".
- **Show work** opens the CLI session behind the thread on the Agents page.
- Incognito threads show an eye-off icon: nothing is recalled or remembered, and they are deleted 24 hours after the last message.

**Chat**
- Replies render as a conversation (not a terminal), with the provider that wrote each one, tool steps and images.
- Everything the assistant does shows between the messages as a card: tasks, reminders, approvals, delegations to Personal Agents, memory chips, and routing or usage-limit notes.
- **Attach files** (up to 20 MB each); they land in the assistant's inbox for that thread.
- If a message can't be sent, your draft and attachments come back with the reason.
- You can't send while Otto is still answering.
- The microphone button is disabled — voice isn't available yet.

**Needs you**
- **Approval needed:** the card shows where it goes, exactly what is sent and who sees it. The button names the action (Send, Post, Publish, Buy, Delete, Submit, Run on prod). **Deny…** takes an optional reason.
- **Always allow for *destination*** is offered per destination and tool from the card — never for purchases or production.
- **Otto asks:** answer a question by picking an option or typing a reply.
- **Usage limit:** choose whether to continue on the other subscription.
- **Browser chores:** a live thumbnail and step list. **Watch** opens the live tab, **Take over** pauses Otto so you can type a password, 2FA code or CAPTCHA, and **Hand back** lets it continue.
- **Suggested memory:** keep or reject it.
- On wide windows a right-hand rail shows what needs you, what is running, and how this week's load splits across your subscriptions.

**Tasks tab**
- Everything the assistant is doing, in order: Needs you → Running → Queued → Done. The tab shows a count when something waits on you.
- Decide approvals and questions right here. **Stop** a running task; **Cancel reminder** on a scheduled one.
- Open the thread a task came from.

**Memory tab**
- **Profile:** a Markdown file of facts you own. Otto only proposes additions. Save with `⌘S`.
- **Waiting for review:** memories Otto suggested — **Keep** or **Reject**.
- **Memories:** every short fact Otto saved, with the thread it came from. Search, **Forget** one, or **Forget…** everything matching a phrase. Each forget offers **Undo**.
- **Import from Hermes:** when `~/.hermes/memories` exists, Otto reads it (read-only) and queues new entries for your review. Hermes itself is never changed.

**Permissions tab** (read-only for now)
- Lists the rules Otto follows today: sending, posting, publishing and submitting ask first; purchases and production connections always ask; site logins come from Keychain by name and are never pasted into prompts; incognito threads keep no memory.

**Settings → Assistant (routing)**
- **Subscriptions:** each provider's share of this week's load, any limit hit, and which CLI accounts are signed in. Otto uses the CLIs' own sign-ins and never reads your credentials.
- **Rules:** pick the provider and model for 4 kinds of request — conversation and writing, code and data, "think hard" long tasks, and voice. Add extra comma-separated keywords that mean "code" or "think hard". Classification happens on your Mac with keyword rules, no extra model call.
- **When a limit is reached:** ask before switching provider (the default), or switch automatically with a visible note in the thread.
- **Memory:** turn on **Review memories before Otto keeps them** to hold every suggestion for review. Off, Otto saves and shows a chip with Undo.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `Enter` | Send the message |
| `⇧Enter` | New line in the message |
| `⌘S` | Save your profile (Memory tab, while editing it) |
| `←` / `→` | Previous / next tab (Chat, Tasks, Memory, Permissions) when the tab list is focused |
| `Home` / `End` | First / last tab when the tab list is focused |
| `↑` / `↓` | Move between threads in the thread list |

## Tips and limits

- The Assistant needs view access to the **Agents** feature; starting threads and sending needs edit access. Handing work to a Personal Agent also needs edit access to Scheduled Tasks and Editor on that agent's workspace.
- Threads, tasks and memories are personal. Nobody else — including an admin — can open yours.
- A message can be up to 32 KB; an attachment up to 20 MB.
- Only a **leading** `@claude` or `@codex` routes a message. `me@codex.dev` or "ask @codex" mid-sentence doesn't.
- A pinned model or a leading mention always beats the routing rules.
- On an older daemon the page says the assistant isn't available; update Otto.
- On a phone the thread list is its own page. Open a thread, then use the back button to return.
- The Assistant is not the same as the floating command bar's Ask Otto; see [Command bar](#/walkthroughs/command-bar).

## Related

- [Personal Agents](#/walkthroughs/personal-agents)
- [Agents](#/walkthroughs/agents)
- [Home](#/walkthroughs/home)
- [Command bar](#/walkthroughs/command-bar)
- [Browser](#/walkthroughs/browser)
