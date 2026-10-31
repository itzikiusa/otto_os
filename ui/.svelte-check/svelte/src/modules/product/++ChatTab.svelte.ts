///<reference types="svelte" />
;
import { rel } from '../../lib/stores/now.svelte';
// ChatTab — Discovery Chat for the current story.
// Left: "New chat" + a list of this story's DiscoveryChats (newest first, with
// an archive action), in a shared ListPane. Right: the active DiscoveryChat
// pane. Layout mirrors RefineTab (220px sidebar + chat pane); the
// conversation works from an EMPTY/Untitled draft to help with early
// discovery & research.
import { untrack } from 'svelte';
import { product } from '../../lib/stores/product.svelte';
import { toastError } from '../../lib/toastError';
import DiscoveryChat from './DiscoveryChat.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import ListPane from './ui/ListPane.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { confirmer } from '../../lib/confirm.svelte';
import type { DiscoveryChat as DiscoveryChatT } from './types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ── State ──────────────────────────────────────────────────────────────────
  let chats = $state<DiscoveryChatT[]>([]);
  let activeCid = $state<string | null>(null);
  let loading = $state(false);
  let loadError = $state<string | null>(null);

  let creating = $state(false);

  // ── Load on mount / story change ──────────────────────────────────────────
  $effect(() => {
    // Re-run whenever the selected story changes.
    product.selectedId;
    untrack(() => {
      chats = [];
      activeCid = null;
      creating = false;
    });
    void loadChats();
  });

  async function loadChats(): Promise<void> {
    const current = product.captureSelection();
    // No story selected → nothing to load (avoid a "No story selected" error from
    // the store's story-scoped call).
    if (!product.selectedId) {
      chats = [];
      activeCid = null;
      return;
    }
    loading = true;
    loadError = null;
    // Drop any selection carried over from a previously-open story; it belongs to
    // a different story and would make DiscoveryChat fetch the wrong chat.
    activeCid = null;
    try {
      const list = await product.listDiscoveryChats();
      if (!current()) return;
      // Newest first.
      chats = [...list].sort((a, b) => b.created_at.localeCompare(a.created_at));
      // Auto-select the first active chat if any.
      if (chats.length > 0) {
        const first = chats.find((c) => c.status === 'active') ?? chats[0];
        activeCid = first.id;
      }
    } catch (e) {
      if (!current()) return;
      loadError = loadErrorText(e);
    } finally {
      if (current()) loading = false;
    }
  }

  // ── Create new chat ─────────────────────────────────────────────────────────

  async function createChat(): Promise<void> {
    const current = product.captureSelection();
    if (creating) return;
    creating = true;
    try {
      const newChat = await product.createDiscoveryChat({});
      if (!current()) return;
      chats = [newChat, ...chats];
      activeCid = newChat.id;
    } catch (e) {
      if (!current()) return;
      toastError('Couldn’t create chat', e);
    } finally {
      if (current()) creating = false;
    }
  }

  // ── Archive ──────────────────────────────────────────────────────────────────

  async function archiveChat(cid: string): Promise<void> {
    const current = product.captureSelection();
    const target = chats.find((c) => c.id === cid);
    const label = target?.title || 'Untitled';
    const ok = await confirmer.ask(
      `Archive discovery chat “${label}”? It will be marked archived in this story’s chat list.`,
      { title: 'Archive chat', confirmLabel: 'Archive', danger: false },
    );
    if (!ok || !current()) return;
    try {
      const updated = await product.archiveDiscoveryChat(cid);
      if (!current()) return;
      chats = chats.map((c) => (c.id === cid ? updated : c));
      if (activeCid === cid) {
        // Switch away from an archived chat.
        const next = chats.find((c) => c.id !== cid && c.status === 'active');
        activeCid = next?.id ?? null;
      }
    } catch (e) {
      if (!current()) return;
      toastError('Couldn’t archive chat', e);
    }
  }

  // ── Helpers ──────────────────────────────────────────────────────────────────

;
async () => {

 { svelteHTML.createElement("div", { "class":`chat-tab`,});
  
   { const $$_enaPtsiL1C = __sveltets_2_ensureComponent(ListPane); const $$_enaPtsiL1 = new $$_enaPtsiL1C({ target: __sveltets_2_any(), props: {   "title":`Chats`,"isEmpty":!loading && !loadError && chats.length === 0,actions:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {         "class":`btn small primary`,"onclick":createChat,"disabled":creating,"title":`Start a new discovery chat`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} creating ? 'Creating…' : 'New chat';
       }
    };return __sveltets_2_any(0)},children:() => { async ()/*Ωignore_positionΩ*/ => {
      if((loading || loadError) && chats.length === 0){
         { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {          "what":`chats`,"variant":`compact`,"loading":loading,"error":loadError,"empty":true,"onretry":() => void loadChats(),}});}
      }else{
        if(loadError){
           { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {          "what":`chats`,"variant":`compact`,"loading":loading,"error":loadError,"onretry":() => void loadChats(),}});}
        }
         { svelteHTML.createElement("div", { "class":`chat-list`,});
             for(let c of __sveltets_2_ensureArray(chats)){c.id;
             { svelteHTML.createElement("div", {     "class":`chat-item`,});activeCid === c.id;c.status === 'archived';
               { svelteHTML.createElement("button", {       "class":`chat-btn`,"aria-current":activeCid === c.id ? 'true' : undefined,"onclick":() => (activeCid = c.id),});
                 { svelteHTML.createElement("span", { "class":`chat-title`,});c.title; }
                 { svelteHTML.createElement("span", { "class":`chat-meta`,});
                   { svelteHTML.createElement("span", {  "class":`chat-status`,});c.status === 'archived';
                    c.status === 'archived' ? 'Archived' : 'Active';
                   }
                   { svelteHTML.createElement("span", {   "class":`chat-date`,"title":new Date(c.updated_at).toLocaleString(),});rel(c.updated_at); }
                 }
               }
              if(c.status === 'active'){
                 { svelteHTML.createElement("button", {       "class":`archive-btn reveal-on-hover`,"onclick":() => archiveChat(c.id),"title":`Archive this chat`,});
                  
                 }
              }
             }
          }
         }
      }
    };return __sveltets_2_any(0)},empty:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("p", { "class":`empty-rail-hint`,});           }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions, children, empty} = $$_enaPtsiL1.$$prop_def;/*Ωignore_endΩ*/
    
    
    
   ListPane}

  
   { svelteHTML.createElement("div", { "class":`chat-pane`,});
    if(activeCid){
       { const $$_tahCyrevocsiD2C = __sveltets_2_ensureComponent(DiscoveryChat); new $$_tahCyrevocsiD2C({ target: __sveltets_2_any(), props: {  "cid":activeCid,}});}
    }else{
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {       "icon":`comment`,"title":chats.length === 0 ? 'Figure out what to build' : 'No chat open',"body":chats.length === 0
          ? 'Start a New chat and describe the rough idea — the agent researches, asks the right questions and turns it into a story.'
          : 'Open a chat on the left, or start a New chat.',}});}
    }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ChatTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ChatTab__SvelteComponent_ = ReturnType<typeof ChatTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default ChatTab__SvelteComponent_;