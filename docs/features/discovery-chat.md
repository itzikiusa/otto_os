# Discovery Chat

Discovery Chat is a conversational agent on the **Product / Story** page that you
can talk to **before you have written anything** — from a brand-new Untitled
draft — to help with early **discovery and research**. Tell it a rough idea or a
problem; it researches, asks the right questions, surfaces edge cases, and shapes
a story. It can see your draft, mockups/attachments, and discovery notes, and it
proposes concrete next steps as **action cards** you apply explicitly.

This guide documents what `crates/otto-server/src/product_chat.rs`,
`crates/otto-state/src/product_chat.rs`, and `ui/src/modules/product/ChatTab.svelte`
(+ `DiscoveryChat.svelte`, `ActionCard.svelte`) do.

> Distinct from two existing features: **Discovery (swarm)** — a heavyweight
> multi-agent investigation that produces a report — and **Refine** — a chat that
> edits an *existing* story version. Discovery Chat is the lightweight, immediate,
> "I have nothing yet" assistant.

---

## 1. Summary

| | |
|---|---|
| **What it is** | An interactive agent chat on a story/draft for early discovery. |
| **Works from** | An empty/Untitled draft (no published story needed). |
| **What it sees** | The latest relevant version, mockups/attachments (text inlined), the most recent discovery report, open questions, and notes — a relevance-bounded bundle. |
| **Proposes** | Action cards: fill/replace the draft, add questions, add notes, or **open a diagram in Canvas Studio**. Nothing is auto-applied. |
| **Engine** | One `orchestrator.run_agent` turn per message (request/response), history replayed. |
| **Where it lives** | The **Chat** tab on a Product story (after Overview). RBAC feature key `product`. |
| **Daemon** | `ottod` on `127.0.0.1:7700`; routes under `/api/v1` (contract #109–#114). |

---

## 2. Where everything lives

| Layer | Path |
|---|---|
| Handlers + context assembly | `crates/otto-server/src/product_chat.rs` |
| Persistence | `crates/otto-state/src/product_chat.rs` (`DiscoveryChatRepo`); migrations `0073_product_discovery_chats.sql` |
| Server wiring | `crates/otto-server/src/modules.rs` (routes under the `/product/` policy prefix) |
| UI | `ui/src/modules/product/{ChatTab,DiscoveryChat,ActionCard}.svelte`; store methods in `ui/src/lib/stores/product.svelte.ts` |
| Types | `ui/src/modules/product/types.ts` (`DiscoveryChat`, `DiscoveryChatMessage`, `DiscoveryAction`, …) |
| API contract | `docs/contracts/api.md` #109–#114 |

---

## 3. How a turn works

1. Resolve the chat → story → workspace (Editor required).
2. **Assemble context** (bounded ≤ ~24 KB, least-relevant trimmed first):
   - the latest relevant version (suggested > draft > source > title),
   - attachments/mockups — text ones (mermaid/html/markdown) inlined, raster
     images listed by absolute path so an agent with file tools can open them,
   - the most recent discovery run's report,
   - open questions and notes.
   The bundle is stored on the user message's `meta_json` for audit/repro.
3. Replay the chat history + the new message into one `run_agent` turn.
4. Split the reply into prose (markdown) + an optional `actions` JSON array.
5. Persist both messages; the agent message carries `actions_json`.

---

## 4. Action cards

The agent may propose actions; each renders as a card with an explicit button:

- **`apply_draft`** — fill/replace the draft. Shows a **diff preview** before
  applying; if the draft is non-empty, Apply is confirmed (it overwrites). After
  applying, an inline **Undo** restores the prior body.
- **`add_questions` / `add_notes`** — a checkbox per item (default all checked);
  applies only the checked ones. Undo deletes what was just created.
- **`create_canvas`** — shows a Mermaid thumbnail and an **Open in Canvas**
  button that creates a scene (linked to the story) and jumps to Canvas Studio.

Apply calls `POST /api/v1/product/discovery-chats/{cid}/apply` (#114).

---

## 5. Using it

1. Open a Product story (or create a new draft) → the **Chat** tab.
2. Click **New chat**. The empty state offers starter prompts — click one to
   prefill the composer (e.g. "What questions should I answer before building
   this?"), edit, and send.
3. Read the reply; apply any action cards you like (preview first).
4. Iterate. When the draft is ready, publish it as a Jira Story / Confluence RFC
   from the Overview tab as usual.

---

## 6. Capabilities & limits

- The turn is **request/response** (not token-streamed); a "thinking…" indicator
  covers the latency.
- The agent runs in the story's working directory when set (so it can research
  code), else a scratch dir.
- Fully usable on a phone (the chips become a horizontal scroll row; diffs force
  line mode).
- Actions are never auto-applied — every change is explicit and undoable.
