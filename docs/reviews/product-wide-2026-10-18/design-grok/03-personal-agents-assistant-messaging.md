# Design review 03 — Personal agents, assistant, messaging

Reviewer: grok (design). Date: 2026-10-18. Read-only code review against `docs/design/guidelines/`. No browser, no light/dark screenshots, no Cargo/npm. Terminal/PTY drawing was not reviewed. Room games (kart/shooter canvases) are out of the scores; canvas hex and fullscreen `100vh` there are game paint, not chrome.

Inspected: personal-agents list, agent page (all eight tabs), edit sheet, autonomy, activity, memory, documents, agent channels; assistant page, thread list, chat, composer, tasks, memory, permissions, needs-you cards and rail; rooms lobby, room page, chat, participants, invite, recap banner, start-room; agents empty/session open, share, handover, message composer.

Tokens in these modules are almost entirely `--fs-*` / semantic colours. Loading, empty, error and retry exist on the main lists. Confirms use `confirmer`. Phone push navigation exists for assistant threads and agent channels. Assistant replies and channel posts are attributed. That is why the scores are high. They are not 9.8: the defects below are user-visible.

## Scores

| Area | Score | Defects |
|---|---|---|
| Personal agents (list + agent page) | 8.6 | 4 |
| Agent channels | 8.7 | 2 |
| Assistant | 9.4 | 1 |
| Session rooms (lobby, room, chat, people, recap) | 8.2 | 4 |
| Agents list, share, handover, composer | 8.9 | 3 |
| **Overall** | **8.8** | **14** |

Overall is the unweighted mean of the five areas, not a target.

## Defects

### Personal agents — 8.6

1. **Eight tabs scroll off the phone with no affordance.** `ui/src/modules/personal-agents/AgentPage.svelte:397` (styles `:687`). Overview through Context is one horizontal strip (`overflow-x: auto`, `white-space: nowrap`). On a phone, Chat, Memory and Context sit past the fold with no fade, scroll buttons or menu. **Fix:** on `max-width: 640px`, replace the strip with a labelled `<select>` (or the shared tab control plus an explicit “more sections” menu). Keep arrow-key roving on desktop.

2. **Chat does not fill the page, so the page and the session both scroll.** `AgentPage.svelte:644` and `:742` (`.chatwrap { height: min(64vh, 620px) }`). The tab is a capped island inside a readable `PageBody`; below it is empty, and a short window scrolls the page to reach a second scrollbar. `64vh` is also a WKWebView height trap. **Fix:** for the chat tab, use `PageBody fill` and `flex: 1; min-height: 0; height: auto` on `.chatwrap` so the session is the only scroller under the header. Do not set a `vh` cap.

3. **Turning on Slack, Telegram, email or a webhook is a generic Save.** `ui/src/modules/personal-agents/AgentEditSheet.svelte:236` and footer `:286`. The note at `:262` says every run’s report is posted and who sees it, but the button stays “Save” and there is no confirm. A persona edit and “start emailing reports” are the same click. **Fix:** if the destination type or target changed away from none, `confirmer.ask` with where, what (the run report) and who sees it, and label the button “Save and deliver reports”.

4. **“Waiting for you” cannot be decided here, and the row’s title is a tool id.** `ui/src/modules/personal-agents/AgentActivity.svelte:133` and `:145`. The row is `otto.{tool}` plus an optional detail, then a link to MCP activity. No who / what / why, no Approve or Deny. A person who opened the agent to answer is sent away. `:122` also prints raw `session_status`. **Fix:** reuse the assistant approval card (where, what, who, Approve / Deny) or deep-link each row to that approval. Show a `runStatus` / `sessionState` label, not the enum.

### Agent channels — 8.7

5. **Reading earlier messages jumps to the bottom whenever a new post arrives.** `ui/src/modules/personal-agents/RoomsView.svelte:104`. The effect sets `scrollTop = scrollHeight` on every new tail id, with no “already at the bottom” check. `showEarlier` (`:64`) preserves position, then the next agent post undoes it. **Fix:** copy `ChatView.svelte:112`: track `atBottom` (within ~48px) and only pin when already at the bottom; otherwise show “Jump to latest”.

6. **Removing a member is a 16px control with no confirm.** `RoomsView.svelte:284` and `.unlink` at `:399` (`padding: 2px`, 12px icon). One tap drops the agent from the channel. On a phone the hit target is far under `--hit-min` (36px), so it is easy to hit while scrolling the member row. **Fix:** use `.icon-btn` with `min-width/min-height: var(--hit-min)` at `max-width: 640px`, and `confirmer.ask` (“Remove {name} from {channel}?”, danger, confirmLabel “Remove”).

### Assistant — 9.4

States, attribution, draft-until-applied memory, outward approval cards (where / who / what), phone thread push, and stick-to-bottom are in place. Tasks still carry Needs you when the rail hides under 1060px.

7. **A disabled microphone sits in the composer and does nothing.** `ui/src/modules/assistant/AssistantComposer.svelte:149`. Title says voice is a later phase. On a phone (`:302`) it is a 36px control beside Send. Touch has no hover title, so it reads as a broken button. **Fix:** do not render the mic control until dictation exists.

### Session rooms — 8.2

8. **People & chat does not keep the transcript or the composer on screen.** `ui/src/modules/rooms/RoomChat.svelte:33` (`.messages { max-height: 45vh }`) and `RoomPage.svelte:133`. The transcript never scrolls to new messages (no `scrollTop` / “jump to latest”). `max-height: 45vh` caps the log while `RoomParticipants` sits above it. On a phone or tablet the conversation tab is one column, so the composer is below the participant block and a short log. `45vh` is the same WebKit height trap. **Fix:** make the conversation column `flex` with `min-height: 0`, let `.messages` be `flex: 1` (no `vh` cap), pin the composer, and follow the tail the same way as assistant chat. On `max-width: 1024px`, put People behind a disclosure so the composer is visible without scrolling.

9. **End room sits next to the primary Invite.** `RoomPage.svelte:112`. “End room…” is `.danger` with `data-overflow="-2"`; “Invite someone…” is `.primary`. They share the toolbar whenever the row is wide enough not to overflow. Ending the room for everyone is adjacent to the action a host uses most. **Fix:** keep Invite as the only toolbar primary; put End room only in the overflow menu (or a footer in the participants section), still with the existing confirm.

10. **Invite failures and room failures are one string.** `RoomPage.svelte:24`, written by `makeInvite` at `:101`, shown in the room banner `:131` and inside the invite sheet `:144`. A connection error already on screen appears in “Invite someone” as if creating the link failed. Dismissing the banner clears the sheet error and the reverse. **Fix:** `inviteError` used only inside the modal; leave `error` for the room connection.

11. **Withdraw consent and the access menu act immediately, and “Can control” overclaims.** `ui/src/modules/rooms/RoomRecap.svelte:49` calls `withdraw()` with no confirm, while the copy says withdrawal pauses capture for everyone. `ui/src/modules/rooms/RoomParticipants.svelte:29` changes role on `<select>` change; the option label is “Can control”, but control is a separate “Give control…” confirm (`:7`). A mis-tap upgrades a guest and reads as if they can type now. **Fix:** confirm withdraw (“Pause capture for everyone?”). Label the option “May be granted control” and confirm before sending `role: editor`.

### Agents list, share, handover, composer — 8.9

Empty sessions, auto-open of the latest session, and the composer’s clamped slash list (`--pop-max` from `window.innerHeight`) match the guidelines. PTY drawing was not reviewed.

12. **Revoke one share link does not ask; Revoke all does.** `ui/src/modules/agents/ShareModal.svelte:187` versus `:217`. The row button at `:457` disconnects whoever holds that link on one click. The row also prints the raw role enum at `:447` (`viewer` / `editor`). **Fix:** `confirmer.ask` per link, same copy as revoke-all, confirmLabel “Revoke link”. Show “View only” / “Can type”.

13. **“Archive source after confirming receipt” is not on the button that sends.** `ui/src/modules/agents/Handover.svelte:264` and primary label `:311` (“Hand over”). Archiving the source session is one of four checkboxes. The primary does not say the source will close, and there is no second confirm. **Fix:** when `archiveSource` is set, label the button “Hand over and archive” and `confirmer.ask` naming the source session before `send`.

14. **Composer send and attachment-remove are under the 36px touch minimum, and remove uses `top`.** `ui/src/modules/agents/conversation/Composer.svelte:552` (`.send` is 28×28) and `:653` (`.thumb-x` is 18×18, `top: 3px`). No phone rule raises them to `var(--hit-min)`. Removing a pasted image is a small corner target. **Fix:** at `max-width: 640px` and `(pointer: coarse)`, set send and thumb-x to at least `var(--hit-min)`; position the remove control with `inset-block-start`.

## Not scored as defects

- Agent-channel vs session-room naming is deliberate (`RoomsView.svelte:3`, layout table).
- Personal-agent destructive actions (delete, reset-by-name, stop run, delete schedule) already confirm.
- Assistant outward cards, memory keep/reject, and task stop already confirm.
- Room join and “give control” / “remove participant” already confirm.
- Schedule `LoadState` with `empty` and no `emptyView` renders nothing, then the custom empty state; not a double empty.
- No hex in personal-agents or assistant module CSS. `outline: none` on the assistant composer and memory search is paired with a `:focus-within` ring.

## Residual gap

Scores are from source, not from rendered Native light/dark, Warm dark, RTL, or phone screenshots. Contrast of tinted chips and the room recap banner was not measured in a browser.
