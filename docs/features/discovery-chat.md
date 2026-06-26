# Discovery Chat

A lightweight, conversational agent on the **Product** page that you can talk to
**before you have written anything** — from a brand-new Untitled story — to help
with early **discovery and research**. Tell it a rough idea or a problem; it
researches, asks the right questions, surfaces edge cases, and shapes a story.
It can see your draft, mockups/attachments, the latest discovery findings, your
open questions and notes, and it proposes concrete next steps as **action cards**
you apply explicitly — fill the draft, add questions, add notes, or open a diagram
in Canvas. When the draft is ready you publish it as a Jira story or a Confluence
RFC from the usual Product flow.

> **Where this lives in the code.** Handlers + context assembly:
> `crates/otto-server/src/product_chat.rs`. Persistence:
> `crates/otto-state/src/product_chat.rs` (`DiscoveryChatRepo`), migrations
> `0073_product_discovery_chats.sql` (chats + messages) and `0074_session_links.sql`
> (the backing session id). UI: `ui/src/modules/product/{ChatTab,DiscoveryChat,ActionCard}.svelte`
> + store methods in `ui/src/lib/stores/product.svelte.ts`. REST contract:
> `docs/contracts/api.md` #109–#114.

---

## 1. Overview

| | |
|---|---|
| **What it is** | An interactive agent chat attached to a product story for early discovery — works from an empty/Untitled draft, before anything is written. |
| **What it sees** | A relevance-bounded context bundle: the latest relevant version, mockups/attachments (text ones inlined, raster ones listed by path), the most recent discovery report, open questions, and notes. |
| **Proposes** | Apply-able **action cards** — `apply_draft`, `add_questions`, `add_notes`, `create_canvas`. **Nothing is auto-applied**; every change is explicit and undoable. |
| **Engine** | One persistent, **resumable** managed Otto session per chat; each message is one `run_session_turn` (request/response). |
| **Where it lives** | The Product story's **Discover → Chat** tab (first sub-tab under *Discover*). RBAC feature key `product`. |
| **Distinct from** | **Discovery (swarm)** — a heavyweight multi-agent investigation that produces a report — and **Refine** — a chat that edits an *existing* story version. Discovery Chat is the "I have nothing yet" assistant. |
| **Daemon** | `ottod` on `127.0.0.1:7700`; routes under `/api/v1` (contract #109–#114). |

### Where each piece lives

| Concern | Source |
|---|---|
| Handlers + context assembly + action dispatch | `crates/otto-server/src/product_chat.rs` |
| One managed agent turn (create/resume) | `crates/otto-server/src/agent_session.rs` (`run_session_turn`) |
| Persistence + repo | `crates/otto-state/src/product_chat.rs` (`DiscoveryChat`, `DiscoveryChatMessage`, `DiscoveryChatRepo`) |
| Draft / questions / notes / canvas writes | `crates/otto-product/` (`update_draft_body`, `create_question`, `create_note`) · `crates/otto-state/src/canvas.rs` (`CanvasRepo::create`) |
| Server wiring (routes under the `/product/` policy prefix) | `crates/otto-server/src/modules.rs` |
| UI — list + pane | `ui/src/modules/product/ChatTab.svelte` |
| UI — conversation | `ui/src/modules/product/DiscoveryChat.svelte` |
| UI — action cards | `ui/src/modules/product/ActionCard.svelte` |
| Store methods | `ui/src/lib/stores/product.svelte.ts` (`listDiscoveryChats`, `createDiscoveryChat`, `getDiscoveryChat`, `sendDiscoveryMessage`, `archiveDiscoveryChat`, `applyDiscoveryAction`) |
| Types | `ui/src/modules/product/types.ts` (`DiscoveryChat`, `DiscoveryChatMessage`, `DiscoveryAction`, `ApplyResult`, …) |
| API contract | `docs/contracts/api.md` #109–#114 |

---

## 2. Where it sits in the Product page

The story page's tabs are grouped; Discovery Chat is the **Chat** sub-tab under the
**Discover** group (alongside *Analysis*, *Questions*, *Notes*, *Discovery* (the
swarm runs), and *Refine*). `ChatTab.svelte` is a 220px chat list + a chat pane,
mirroring the Refine tab's layout. The list shows this story's chats newest-first
with their `active`/`archived` status; it auto-selects the first active chat. A
chat you no longer need is **Archived** (it stays readable, just out of the way).

A chat is scoped to **one story** (`story_id`) and lives in a workspace; switching
the selected story reloads its own chats.

---

## 3. How a turn works

Each message is one round-trip — `POST /api/v1/product/discovery-chats/{cid}/messages`
(workspace **Editor**), handled by `send_message`:

1. **Resolve** the chat → story → workspace (Editor required).
2. **Assemble context** (`assemble_context`) — a bounded bundle, capped at
   **~24 KB** with a clear `…[truncated]` marker:
   - the **latest relevant version** — `suggested` > `draft` > `source`, falling
     back to the story title;
   - **mockups/attachments** — text ones (`text/vnd.mermaid`, `text/html`,
     `text/markdown`, `text/plain`, `.mmd`/`.md`) are **inlined** (each capped at
     ~4 KB); raster images are listed by their absolute path so an agent with file
     tools can open them;
   - the **most recent discovery report** (capped ~4 KB);
   - **open questions** and **notes**.
   The full bundle is stored on the **user message's** `meta_json` for audit/repro.
3. **Persist the user message** (with the bundle in meta), then run **one** agent
   turn via `run_session_turn`, **resuming** the chat's managed session when it
   exists. The chat is a **persistent, resumable Otto session** (visible/resumable
   in Agents, MCP-wired), so the agent **retains the conversation itself** — the
   prompt sends framing + the *refreshed* context + the new message, **not** a
   replayed transcript. The first turn creates the session and stores its id
   (`set_session`).
4. **Split the reply** (`split_actions`) into prose markdown + an optional `actions`
   JSON array.
5. **Persist the agent message**; its `actions_json` carries the proposals.

The prompt (`build_chat_prompt`) carries an `OTTO_TASK: discovery_chat` sentinel
(which routes the deterministic E2E stub) and the **actions contract** the reply is
parsed against. The agent runs as `claude` (the provider is fixed for Discovery
Chat, unlike Canvas where it's per-scene). The working directory is the **story's
cwd** when set (so the agent can research real code) — otherwise a fresh scratch
dir under `<data_dir>/product/discovery-chat/<id>`.

---

## 4. The conversation UI

`DiscoveryChat.svelte` is the chat pane for one chat:

- **Empty state** — when a chat has no messages, an EmptyState ("Let's figure out
  what to build") plus a row of **starter-prompt chips** (e.g. "What questions
  should I answer before building this?", "Research how other products solve this
  problem", "Summarize this into a story I can publish"). Clicking a chip
  **prefills the composer** — it never auto-sends — and focuses it (the caret lands
  at the end for prefixes like "Help me scope a story for: "). A hint reminds you
  it can see your draft, mockups, and discovery notes.
- **Bubbles** — user messages render plain (right-aligned "PO"); agent messages
  render markdown (left-aligned "Agent"). An optimistic user bubble appears on send
  and is reconciled with the server's real messages; a **"thinking…"** indicator
  covers the turn's latency (the turn is request/response, not token-streamed).
- **Action cards** — any actions on an agent message render below it as cards (§5).
- **Composer** — Enter sends, Shift+Enter inserts a newline. On send failure the
  optimistic bubble rolls back and your text is restored.

---

## 5. Action cards

The agent may emit a single fenced ```` ```json ```` block with an `actions` array;
each action renders as a card with an explicit Apply button (`ActionCard.svelte`).
Applying calls `POST /api/v1/product/discovery-chats/{cid}/apply` with the one
action object; the server dispatches by the action's `type` (`apply_action`).
**Nothing is applied until you click, and every apply is reversible.**

| Action `type` | Card | What Apply does (server) | Undo |
|---|---|---|---|
| `apply_draft` | A **diff preview** (collapsed 3-line peek → expandable) of the proposed `body_md` vs the current draft. On a non-empty draft, **Replace draft** is a danger-styled confirm (it overwrites). | `update_draft_body(story, title, body_md)` — the same write the draft PATCH route performs. Keeps the current title when the action omits one. | Re-applies the captured prior body via the draft update. |
| `add_questions` | A **checkbox per item** (default all checked) over the proposed questions (`text`, optional `rationale`, `category`). Applies only the checked ones. | `create_question(...)` per item (defaults `category` to `other`). | Deletes the just-created question ids (`DELETE /product/questions/{qid}`). |
| `add_notes` | A **checkbox per item** (default all checked) over the proposed notes. | `create_note(... section: "discovery")` per item. | Deletes the just-created note ids (`DELETE /product/notes/{nid}`). |
| `create_canvas` | A lazily-rendered **Mermaid thumbnail** (when the action carries `mermaid`) + **Open in Canvas**. | Creates a `CanvasScene` linked to the story (`story_id`), then deep-links: sets `canvas.pendingOpenId` and navigates to the Canvas module, which opens the scene on mount. | — (the scene persists; delete it from Canvas). |

After a successful apply the card collapses to a sticky **"✓ … · Undo"** row (where
an Undo applies). `ApplyResult` returns `{ story_updated, created_question_ids,
created_note_ids, canvas_id }` so the UI can refresh the right panels and wire Undo.

> **Note on `create_canvas` permissions.** That route is gated on the **Product**
> capability (it lives under the `/product/` policy prefix), so a Product-Editor
> creates the scene as a byproduct of discovery — the Canvas axis is intentionally
> *not* additionally required here. The scene lands in the user's workspace;
> `Feature::Canvas` View then gates whether they can open the Canvas module to see
> it.

---

## 6. Using it

1. Open a Product story (or create a new draft) → **Discover → Chat**.
2. Click **+ New chat**. From the empty state, click a starter chip to prefill the
   composer (or just type), edit, and send. Describe the rough idea or problem.
3. Read the reply. Apply any action cards you like — preview the draft diff first;
   un-tick questions/notes you don't want; **Open in Canvas** to turn a proposed
   diagram into a scene.
4. Iterate. Because the chat is one resumable session, the agent remembers the
   thread; each turn also re-reads the freshest draft/mockups/notes.
5. When the draft is ready, **publish** it from the Product flow:
   `POST /product/stories/{sid}/publish-as-rfc` (Confluence RFC) or
   `POST /product/stories/{sid}/publish-as-story` (Jira story).
6. **Archive** the chat when you're done — it stays readable but drops out of the
   active list.

---

## 7. REST surface

`docs/contracts/api.md` is authoritative; `ui/src/modules/product/types.ts` mirrors
the DTOs. All routes are covered by the existing **`/product/`** policy prefix
(read = `Product` View, write = `Product` Edit) and additionally check the caller's
**workspace role** (Viewer to read, Editor to write); item routes resolve the
workspace from the chat → its story. Persistence: `otto_state::product_chat`.

| # | Method & path | Auth | Request | Response |
|---|---|---|---|---|
| 109 | `POST /api/v1/product/stories/{sid}/discovery-chats` | ws editor | `{title?}` | `DiscoveryChat` (title defaults to "Discovery") |
| 110 | `GET /api/v1/product/stories/{sid}/discovery-chats` | ws viewer | — | `DiscoveryChat[]` (newest first) |
| 111 | `GET /api/v1/product/discovery-chats/{cid}` | ws viewer | — | `{ chat, messages }` |
| 112 | `POST /api/v1/product/discovery-chats/{cid}/messages` | ws editor | `{body}` | `{ user_message, agent_message }` (one turn; `agent_message.actions_json` carries proposals) |
| 113 | `POST /api/v1/product/discovery-chats/{cid}/archive` | ws editor | — | `DiscoveryChat` (status → `archived`) |
| 114 | `POST /api/v1/product/discovery-chats/{cid}/apply` | ws editor | `{action}` | `ApplyResult` `{story_updated, created_question_ids, created_note_ids, canvas_id}` |

Related Product routes the chat hands off to:

- **The draft write** behind `apply_draft`: api.md lists it as
  `POST /product/stories/{sid}/draft (PATCH)` — and the **real HTTP method is
  `PATCH`** (`crates/otto-product/src/http.rs` registers it with `patch(update_draft_body)`).
  The contract table writes it in the "POST" column with a `(PATCH)` annotation;
  the code is PATCH. `apply_draft` reaches the same `update_draft_body` service
  method server-side.
- **Publish**: `POST /product/stories/{sid}/publish-as-rfc` and
  `POST /product/stories/{sid}/publish-as-story`.
- **Undo of created items**: `DELETE /product/questions/{qid}` and
  `DELETE /product/notes/{nid}`.

There is **no Discovery-Chat-specific WebSocket event** — the turn is a synchronous
HTTP request/response, and the UI updates from the response (the chat's backing
session does emit the usual session-family events as any agent session would).

---

## 8. Capabilities & limitations

- **Request/response, not streamed.** A turn returns once the agent finishes; the
  "thinking…" indicator covers the latency.
- **Context is bounded and relevance-trimmed** (~24 KB total; ~4 KB per inlined
  mockup and for the discovery report). Very large drafts/mockups are truncated
  with a marker — the agent sees a summary, not the entire repo.
- **Raster mockups are referenced by path, not inlined** — an agent with file tools
  (and a real story cwd) can open them; text mockups (HTML/Mermaid/Markdown/plain)
  are inlined directly.
- **Fixed provider.** Discovery Chat always runs `claude` (resume-aware); the
  provider isn't user-selectable here (unlike Canvas).
- **Nothing is auto-applied.** Every story/question/note/canvas change is an
  explicit, undoable card. `apply_draft` **overwrites** the draft body (with a
  diff preview + a danger confirm on a non-empty draft) — it's not a merge.
- **One chat = one story = one session.** Chats don't span stories; the agent's
  memory is the chat's own resumed session.
- **It shapes a draft; it doesn't publish.** Turning the draft into a Jira
  story / Confluence RFC is the separate Product publish step.
- **Distinct from swarm Discovery and Refine** — use those for a heavyweight
  investigation report or for editing an already-written version, respectively.
- **Fully usable on a phone**: the starter chips become a horizontal scroll row and
  the draft diff forces line mode on narrow screens.

---

## 9. Security & guards

- **RBAC.** Every route is behind the `Product` capability (deny-by-default policy)
  **and** a per-chat workspace-role check — Viewer to read a chat/transcript, Editor
  to create, send, archive, or apply. Item routes resolve the owning workspace from
  the chat's story, so you can't reach a chat in a workspace you lack a role in.
- **Loopback by default.** `ottod` runs on `127.0.0.1`; the agent runs on the
  daemon host, in the story's cwd or a scratch dir under the daemon's data dir.
- **Auditability.** The exact context bundle sent to the agent is stored on the user
  message's `meta_json`, and the agent's proposals on the agent message's
  `actions_json` — so a turn is reproducible and every applied change is traceable.
- **Explicit, reversible writes.** Applying an action is a deliberate user action
  with an Undo path; the agent can *propose* but never *commit* a draft, question,
  note, or canvas.
- **Managed session.** The chat's backing agent session is a normal Otto session,
  subject to the same isolation/RBAC as any session.

> Roles and per-session isolation are documented in
> **[Multi-user RBAC](../MULTI-USER-RBAC.md)**.

---

## 10. Troubleshooting

- **"No story selected."** Discovery Chat is story-scoped; select or create a story
  first. With no story, the list and chat are empty (no error).
- **The agent can't see my mockup's content.** Only **text** mockups
  (HTML/Mermaid/Markdown/plain, `.mmd`/`.md`) are inlined; raster images are passed
  as a path and only readable if the chat runs in a real story cwd with file tools.
- **The draft didn't change after "Replace draft."** That action **overwrites** the
  body — confirm you pressed **Replace** through the danger prompt; use the card's
  **Undo** to restore the prior text.
- **"Add questions/notes" added nothing.** All items were un-ticked — at least one
  checkbox must be checked (the button disables at zero).
- **"Open in Canvas" didn't navigate.** The scene is still created (`canvas_id` in
  the result); the deep-link sets `canvas.pendingOpenId` and routes to Canvas —
  open the Canvas module manually if navigation was blocked, and find the scene
  (linked to the story).
- **The reply has no action cards.** The agent only emits actions when it has a
  concrete proposal; otherwise it just answers in prose. Ask it directly (e.g.
  "draft the story", "list the open questions") to elicit cards.
- **Unknown action error.** Applying an action whose `type` isn't one of
  `apply_draft` / `add_questions` / `add_notes` / `create_canvas` returns a 400
  ("unknown discovery action type").

---

## 11. Related docs

- **[Product](./product.md)** — the full product-owner workflow (analysis, rewrite,
  test cases, the multi-agent Plan, swarm Discovery, Learnings, and publishing
  drafts to Jira/Confluence) that Discovery Chat feeds into.
- **[Canvas](./canvas.md)** — where a `create_canvas` action opens the scene it
  creates.
- **[Agent sessions](./agent-sessions.md)** — the managed-session/PTY machinery the
  chat's resumable session reuses.
- **[Jira / Confluence](./jira-confluence.md)** — the integration the publish step
  targets.
- **[Usage & cost](./usage-and-cost.md)** — token/cost tracking for the turns a
  chat spends.
- **[Multi-user RBAC](../MULTI-USER-RBAC.md)** — roles and per-session isolation.
- **API contract**: `docs/contracts/api.md` #109–#114.
