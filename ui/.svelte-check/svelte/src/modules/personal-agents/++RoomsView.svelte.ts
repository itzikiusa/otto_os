///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { rowMenu } from '../../lib/rowMenu';
import { plural } from '../../lib/plural';
// Agent channels (stored as "rooms") — the only agent-to-agent transport,
// always user-visible. Named "channels" in the UI so they are never confused
// with the sidebar's Rooms (human collaboration) — S20-07.
// Room list + create on the left; the selected room's membership editor,
// live message feed (WS agent_room_message + `after` paging) and the user
// post box on the right.
import { tick, untrack } from 'svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import AgentChip from '../../lib/components/AgentChip.svelte';
import { personalAgents } from '../../lib/stores/personalAgents.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
import { toasts } from '../../lib/toast.svelte';
import { loadErrorText } from '../../lib/loadError';
import AgentAvatar from './AgentAvatar.svelte';
import { loadErrorOf } from './loadError';
import type { AgentRoomMessage, AgentRoomWithMembers } from '../../lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let selectedId = $state<string | null>(null);
  /** Rooms list width — the shared list-pane default/range, remembered. */
  let listW = $state(loadPaneWidth('personalAgents.rooms.listW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  let showingList = $state(false);
  let newRoomName = $state('');
  // Drafts and send results belong to a room, even while a request is pending.
  let drafts = $state<Record<string, string>>({});
  let sending = $state<Record<string, boolean>>({});
  let sendErrors = $state<Record<string, string>>({});
  const draft = $derived(selectedId ? (drafts[selectedId] ?? '') : '');
  const error = $derived(selectedId ? (sendErrors[selectedId] ?? '') : '');
  let busy = $state(false);
  let createError = $state('');
  let feedEl = $state<HTMLElement | null>(null);
  let createEl = $state<HTMLInputElement | null>(null);
  let roomListEl = $state<HTMLElement | null>(null);
  let roomsLoading = $state(true);
  const roomsError = $derived(loadErrorOf(personalAgents, 'roomsError'));

  const rooms = $derived(personalAgents.rooms);
  const selected = $derived(rooms.find((r) => r.room.id === selectedId) ?? null);
  const messages = $derived(selectedId ? (personalAgents.messagesByRoom[selectedId] ?? []) : []);
  // Render the newest RENDER_STEP messages; "Show earlier" reveals more of
  // what is held, then pages older history from the server (backlog B6 /
  // SA-08 — a busy room rendered up to 5,000 rows).
  const RENDER_STEP = 200;
  let renderCount = $state(RENDER_STEP);
  $effect(() => {
    void selectedId;
    renderCount = RENDER_STEP;
  });
  const shown = $derived(messages.length > renderCount ? messages.slice(messages.length - renderCount) : messages);
  const hiddenHeld = $derived(messages.length - shown.length);
  const hasOlder = $derived(hiddenHeld > 0 || (selectedId ? !!personalAgents.olderByRoom[selectedId] : false));
  async function showEarlier(): Promise<void> {
    const el = feedEl;
    const before = el ? el.scrollHeight - el.scrollTop : 0;
    if (hiddenHeld > 0) renderCount += RENDER_STEP;
    else if (selectedId) {
      await personalAgents.loadOlder(selectedId);
      renderCount += RENDER_STEP;
    }
    await tick();
    // Keep the message the user was reading in place (content grew above it).
    if (el) el.scrollTop = el.scrollHeight - before;
  }
  const nonMembers = $derived(
    personalAgents.agents.filter((a) => !(selected?.members ?? []).includes(a.id)),
  );

  function reloadRooms(): void {
    if (!ws.currentId) return;
    roomsLoading = true;
    void personalAgents.loadRooms(ws.currentId).finally(() => (roomsLoading = false));
  }
  $effect(() => {
    if (ws.currentId) reloadRooms();
  });
  // First room auto-selects; a deleted selection falls back.
  $effect(() => {
    if (!showingList && rooms.length > 0 && !rooms.some((r) => r.room.id === selectedId)) {
      selectedId = rooms[0].room.id;
    }
  });
  // Room events touch message feeds only while this view is mounted; on
  // unmount every feed but the selected room's is evicted.
  $effect(() => personalAgents.watchRooms());
  $effect(() => {
    const id = selectedId;
    untrack(() => personalAgents.setActiveRoom(id));
    if (id) untrack(() => void personalAgents.loadMessages(id));
  });
  // Follow the tail only when the reader is already near the bottom.
  // Paging older history keeps its own scroll position.
  let atBottom = $state(true);
  function onFeedScroll(): void {
    const el = feedEl;
    if (el) atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
  }
  function jumpLatest(): void {
    const el = feedEl;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    atBottom = true;
  }
  $effect(() => {
    void selectedId;
    atBottom = true;
  });
  $effect(() => {
    void messages.at(-1)?.id;
    if (!untrack(() => atBottom)) return;
    untrack(() => { if (feedEl) feedEl.scrollTop = feedEl.scrollHeight; });
  });

  function authorName(m: AgentRoomMessage): string {
    if (m.author_kind === 'user') return m.author_id === auth.me?.id ? 'You' : 'User';
    return personalAgents.agent(m.author_id)?.name ?? 'Agent (removed)';
  }


  async function backToRooms(): Promise<void> {
    const previous = selectedId;
    showingList = true;
    selectedId = null;
    await tick();
    Array.from(roomListEl?.querySelectorAll<HTMLButtonElement>('.room') ?? [])
      .find(button => button.dataset.roomId === previous)?.focus();
  }

  async function createRoom(): Promise<void> {
    const submitted = newRoomName;
    const name = submitted.trim();
    if (!name || !ws.currentId || busy) return;
    busy = true;
    createError = '';
    try {
      selectedId = await personalAgents.createRoom(ws.currentId, name);
      showingList = false;
      if (newRoomName === submitted) newRoomName = '';
    } catch (e) {
      createError = `Couldn’t create the channel. ${loadErrorText(e)}`;
    } finally {
      busy = false;
    }
  }

  function roomMenu(e: MouseEvent | KeyboardEvent, r: AgentRoomWithMembers): void {
    ctxMenu.show(e, [
      {
        label: 'Rename',
        icon: 'edit',
        action: async () => {
          const name = await confirmer.promptText('Channel name', { title: 'Rename channel', confirmLabel: 'Rename', initial: r.room.name });
          // A blank (or whitespace-only) name is not a rename.
          const trimmed = name?.trim();
          if (trimmed && trimmed !== r.room.name) {
            void personalAgents.renameRoom(r.room.id, trimmed).catch((e) => toasts.error('Couldn’t rename the channel', loadErrorText(e)));
          }
        },
      },
      {
        label: 'Delete channel…',
        icon: 'trash',
        danger: true,
        action: async () => {
          if (await confirmer.ask(`Delete channel “${r.room.name}” and its transcript? Member agents stay; only the channel and its messages go.`, { title: 'Delete channel', danger: true, confirmLabel: 'Delete' })) {
            void personalAgents.deleteRoom(r.room.id).catch((e) => toasts.error('Couldn’t delete the channel', loadErrorText(e)));
          }
        },
      },
    ]);
  }

  async function addMember(agentId: string): Promise<void> {
    if (!selectedId || !agentId) return;
    try {
      await personalAgents.addMember(selectedId, agentId);
    } catch (e) {
      toasts.error('Couldn’t add the agent to the channel', loadErrorText(e));
    }
  }

  async function removeMember(agentId: string): Promise<void> {
    if (!selectedId) return;
    const name = personalAgents.agent(agentId)?.name ?? 'this agent';
    const channel = selected?.room.name ?? 'this channel';
    const ok = await confirmer.ask(
      `Remove ${name} from ${channel}? They stop posting here. The channel and its messages stay.`,
      { title: 'Remove from channel', danger: true, confirmLabel: 'Remove' },
    );
    if (!ok) return;
    try {
      await personalAgents.removeMember(selectedId, agentId);
    } catch (e) {
      toasts.error('Couldn’t remove the agent from the channel', loadErrorText(e));
    }
  }

  async function send(): Promise<void> {
    const roomId = selectedId;
    const submitted = draft;
    const text = submitted.trim();
    if (!text || !roomId || sending[roomId]) return;
    sending[roomId] = true;
    sendErrors[roomId] = '';
    try {
      await personalAgents.postMessage(roomId, text);
      // Keep anything typed after Send, including a draft in another room.
      if (drafts[roomId] === submitted) drafts[roomId] = '';
    } catch (e) {
      sendErrors[roomId] = `Couldn’t post the message. ${loadErrorText(e)}`;
    } finally {
      sending[roomId] = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`rooms`,});
  if(!viewport.isPhone || !selectedId){
   { const $$_aside1 = svelteHTML.createElement("aside", {      "class":`list`,"aria-label":`Agent channels`,"style":`--list-pane-w:${listW}px`,});roomListEl = $$_aside1;
     { svelteHTML.createElement("div", { "class":`create`,});
       { const $$_input3 = svelteHTML.createElement("input", {           "dir":`auto`,"bind:value":newRoomName,"placeholder":`New channel name`,"aria-label":`New channel name`,"onkeydown":(e) => { if (e.key === 'Enter') void createRoom(); },});createEl = $$_input3;/*Ωignore_startΩ*/() => newRoomName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy || !newRoomName.trim(),"onclick":createRoom,});  }
     }
    if(createError){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: createError, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`err`,});createError; }}}
     { svelteHTML.createElement("ul", {});
         for(let r of __sveltets_2_ensureArray(rooms)){r.room.id;
         { svelteHTML.createElement("li", {});
           {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {             "class":`room`,...__sveltets_2_empty({"data-room-id":r.room.id}),"aria-current":r.room.id === selectedId ? 'true' : undefined,"onclick":() => { showingList = false; selectedId = r.room.id; },"oncontextmenu":(e) => roomMenu(e, r),});r.room.id === selectedId;
             { svelteHTML.createElement("span", {   "class":`room-name`,"title":r.room.name,});r.room.name; }
             { svelteHTML.createElement("span", { "class":`meta`,});
              plural(r.members.length, 'agent');
               if(r.last_message_at){ { const $$_emiTleR6C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR6C({ target: __sveltets_2_any(), props: {  "iso":r.last_message_at,}});}}else{  }
             }
           }}
         }
      }
     }
   }
  if(!viewport.isPhone){
     { const $$_rediviDenaP1C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP1 = new $$_rediviDenaP1C({ target: __sveltets_2_any(), props: {       width:listW,"storageKey":`personalAgents.rooms.listW`,"label":`Resize the channels list`,}});/*Ωignore_startΩ*/() => listW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP1.$$bindings = 'width';}
  }
  }

  if(!viewport.isPhone || selectedId || rooms.length === 0){
   { svelteHTML.createElement("section", { "class":`detail`,});
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"what":`agent channels`,"loading":roomsLoading,"error":roomsError,"empty":!selected,"onretry":reloadRooms,emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
         { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {             "icon":`comment`,"title":`No agent channels yet`,"body":`Channels are how personal agents talk to each other. Every message is kept and shown here, and you can post into any channel.`,"actionLabel":`Create a channel`,"actionIcon":`plus`,"onaction":() => createEl?.focus(),}});}
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
      
      if(selected){
         { svelteHTML.createElement("header", { "class":`detail-head`,});
          if(viewport.isPhone){
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`Back to channels`,"title":`Back to channels`,"onclick":backToRooms,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});} }
          }
           { svelteHTML.createElement("strong", {   "class":`detail-title`,"title":selected.room.name,});selected.room.name; }
           { svelteHTML.createElement("button", {        "class":`icon-btn`,"aria-label":`Channel actions for ${selected.room.name}`,"title":`Channel actions`,"onclick":(e) => roomMenu(e, selected),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
         }

         { svelteHTML.createElement("div", {     "class":`members`,"role":`group`,"aria-label":`Member agents`,});
             for(let mid of __sveltets_2_ensureArray(selected.members)){mid;
            const a = personalAgents.agent(mid);
             { svelteHTML.createElement("span", { "class":`member`,});
                { const $$_ratavAtnegA5C = __sveltets_2_ensureComponent(AgentAvatar); new $$_ratavAtnegA5C({ target: __sveltets_2_any(), props: {      "avatar":a?.avatar,"name":a?.name ?? '?',"size":18,"round":true,}});}
               { svelteHTML.createElement("span", {   "class":`member-name`,"title":a?.name ?? 'Removed agent',});a?.name ?? 'Removed agent'; }
               { svelteHTML.createElement("button", {        "class":`icon-btn unlink`,"aria-label":`Remove ${a?.name ?? 'this agent'} from ${selected.room.name}`,"title":`Remove from channel`,"onclick":() => void removeMember(mid),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
             }
          }
          if(nonMembers.length > 0){
             { svelteHTML.createElement("select", {        "class":`add-member`,"value":"","aria-label":`Add an agent to this channel`,"onchange":(e) => {
                void addMember((e.currentTarget as HTMLSelectElement).value);
                (e.currentTarget as HTMLSelectElement).value = '';
              },});
               { svelteHTML.createElement("option", {"value":"","disabled":true,});  }
                 for(let a of __sveltets_2_ensureArray(nonMembers)){a.id; { svelteHTML.createElement("option", { "value":a.id,});a.name; }}
             }
          }
          if(selected.members.length === 0){
             { svelteHTML.createElement("span", { "class":`meta`,});           }
          }
         }
        if(selected.members.length > 0){
           { svelteHTML.createElement("p", { "class":`meta how`,});
                           
                  
           }
        }

         { svelteHTML.createElement("div", { "class":`feed-wrap`,});
         { const $$_div4 = svelteHTML.createElement("div", {          "class":`feed`,"onscroll":onFeedScroll,"role":`log`,"aria-label":`Channel messages`,"aria-live":`polite`,});feedEl = $$_div4;
           { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL5 = new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`channel messages`,"loading":personalAgents.messagesLoading[selectedId ?? ''],"error":personalAgents.messagesError[selectedId ?? ''],"empty":messages.length === 0,"onretry":() => selectedId && void personalAgents.loadMessages(selectedId),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
               { svelteHTML.createElement("div", { "class":`meta pad`,});                     }
            };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL5.$$prop_def;/*Ωignore_endΩ*/
            
          if(hasOlder && messages.length > 0){
             { svelteHTML.createElement("button", {     "class":`btn small ghost earlier`,"onclick":() => void showEarlier(),"disabled":!!personalAgents.olderLoading[selectedId ?? ''],});
              personalAgents.olderLoading[selectedId ?? ''] ? 'Loading earlier messages…' : 'Show earlier messages';
             }
          }
             for(let m of __sveltets_2_ensureArray(shown)){m.id;
            const agentMsg = m.author_kind !== 'user';
            const a = agentMsg ? personalAgents.agent(m.author_id) : undefined;
             { svelteHTML.createElement("div", {  "class":`msg`,});agentMsg;
              if(agentMsg){
                  { const $$_ratavAtnegA7C = __sveltets_2_ensureComponent(AgentAvatar); new $$_ratavAtnegA7C({ target: __sveltets_2_any(), props: {      "avatar":a?.avatar,"name":a?.name ?? 'Agent',"size":26,"round":true,}});}
              }else{
                 { svelteHTML.createElement("span", {   "class":`me-avatar`,"aria-hidden":`true`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`user`,"size":14,}});} }
              }
               { svelteHTML.createElement("div", { "class":`msg-body`,});
                 { svelteHTML.createElement("div", { "class":`msg-head`,});
                   { svelteHTML.createElement("strong", {});authorName(m); }
                  if(agentMsg){ { const $$_pihCtnegA9C = __sveltets_2_ensureComponent(AgentChip); new $$_pihCtnegA9C({ target: __sveltets_2_any(), props: {}});}}
                   { svelteHTML.createElement("span", { "class":`meta`,}); { const $$_emiTleR10C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR10C({ target: __sveltets_2_any(), props: {  "iso":m.created_at,}});} }
                 }
                 { svelteHTML.createElement("p", { "class":`msg-text`,});m.text; }
               }
             }
          }
           LoadState}
         }
        if(!atBottom && messages.length > 0){
           { svelteHTML.createElement("button", {     "class":`btn small jump`,"type":`button`,"onclick":jumpLatest,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}    }
        }
         }

        if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: error, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`err`,});error; }}}
         { svelteHTML.createElement("div", { "class":`composer`,});
           { svelteHTML.createElement("textarea", {              "dir":`auto`,"value":draft,"oninput":(e) => { if (selectedId) drafts[selectedId] = e.currentTarget.value; },"rows":2,"aria-label":`Message to the channel`,"placeholder":`Post into the channel (visible to all member agents)…`,"onkeydown":(e) => {
              if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void send();
            },}); }
           { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":sending[selectedId ?? ''] || !draft.trim(),"onclick":send,"title":`Send (⌘↩)`,});  }
         }
      }
     LoadState}
   }
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RoomsView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RoomsView__SvelteComponent_ = ReturnType<typeof RoomsView__SvelteComponent_>;
/*Ωignore_endΩ*/export default RoomsView__SvelteComponent_;