///<reference types="svelte" />
;// Otto Assistant (`#/assistant[/<threadId> | /tasks | /memory | /permissions]`).
// List/detail: threads on the left (Spaces 01–04 + Recent), the selected
// thread's conversation on the right — opening on the remembered or latest
// thread, never on an empty pane. Tasks · Memory · Permissions are the other
// tabs. On wide windows a rail shows what needs you, what's running and how
// much of each subscription this week has used. Phone: push navigation.

import PageHeader from '../../lib/components/PageHeader.svelte';
import { onTabKey } from '../../lib/tabKeys';
import PageBody from '../../lib/components/PageBody.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
import Icon from '../../lib/components/Icon.svelte';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import { router } from '../../lib/router.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { registry } from '../../lib/commands.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { recallSelection, rememberSelection } from '../../lib/lastSelection';
import { assistant, describeError } from '../../lib/stores/assistant.svelte';
import { latestThreadId, providerLabel, spaceLabel } from '../../lib/assistant/model';
import ThreadList from './ThreadList.svelte';
import ChatView from './ChatView.svelte';
import TasksTab from './TasksTab.svelte';
import MemoryTab from './MemoryTab.svelte';
import PermissionsTab from './PermissionsTab.svelte';
import NeedsYouRail from './NeedsYouRail.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  type Tab = 'chat' | 'tasks' | 'memory' | 'permissions';
  const TABS: { id: Tab; label: string }[] = [
    { id: 'chat', label: 'Chat' },
    { id: 'tasks', label: 'Tasks' },
    { id: 'memory', label: 'Memory' },
    { id: 'permissions', label: 'Permissions' },
  ];
  const TAB_IDS = new Set<string>(['tasks', 'memory', 'permissions']);

  const seg = $derived(router.parts[1] ?? '');
  const tab = $derived<Tab>(TAB_IDS.has(seg) ? (seg as Tab) : 'chat');
  const routeThread = $derived(tab === 'chat' && seg ? seg : null);

  $effect(() => {
    void assistant.loadThreads();
  });

  const threads = $derived(assistant.threads.data);
  const listState = $derived(assistant.threads.state);
  // The open thread: the route's, else the remembered / latest one (desktop).
  // On phone the list IS the page until a thread is opened.
  const selectedId = $derived.by(() => {
    if (routeThread) return routeThread;
    if (viewport.isPhone && tab === 'chat') return null;
    return recallSelection('assistant') ?? latestThreadId(threads, null);
  });
  const thread = $derived(assistant.thread(selectedId));
  $effect(() => {
    if (thread) rememberSelection('assistant', thread.id);
  });
  $effect(() => {
    if (selectedId) void assistant.loadSelectedThread(selectedId);
  });
  const selectedError = $derived(!!selectedId && assistant.selectedThreadId === selectedId && assistant.selectedThread.state === 'error' ? assistant.selectedThread.error : '');

  function open(id: string): void {
    router.go(`assistant/${id}`);
  }
  function go(t: Tab): void {
    router.go(t === 'chat' ? (selectedId ? `assistant/${selectedId}` : 'assistant') : `assistant/${t}`);
  }

  let creating = $state(false);
  async function newThread(slot: 1 | 2 | 3 | 4 | null = null, title?: string): Promise<void> {
    creating = true;
    try {
      const t = await assistant.createThread({ space_slot: slot, title });
      open(t.id);
    } catch (e) {
      toasts.error('Couldn’t start a thread', describeError(e));
    } finally {
      creating = false;
    }
  }
  async function newSpace(slot: 1 | 2 | 3 | 4): Promise<void> {
    const name = await confirmer.promptText(`Name space ${spaceLabel(slot)} — a thread you come back to, like Personal or Work.`, {
      title: `New space ${spaceLabel(slot)}`,
      confirmLabel: 'Create space',
      initial: '',
    });
    if (name) await newThread(slot, name);
  }

  function showWork(): void {
    if (thread?.session_id) router.go(`agents/${thread.session_id}`);
  }

  // ⌘K verbs.
  $effect(() => {
    return registry.register('assistant', [
      { id: 'assistant.new', title: 'New assistant thread', group: 'Assistant', keywords: 'otto chat ask conversation', run: () => void newThread() },
      { id: 'assistant.tasks', title: 'Go to Assistant tasks', group: 'Assistant', keywords: 'reminders running needs you approvals', run: () => go('tasks') },
      { id: 'assistant.memory', title: 'Go to Assistant memory', group: 'Assistant', keywords: 'profile remember forget', run: () => go('memory') },
      { id: 'assistant.routing', title: 'Assistant routing settings', group: 'Assistant', keywords: 'claude codex model limit failover subscription', run: () => router.go('settings/assistant') },
      ...(thread?.session_id
        ? [{ id: 'assistant.work', title: 'Show work (terminal)', group: 'Assistant', keywords: 'terminal session pty', run: showWork }]
        : []),
    ]);
  });

  const title = $derived(tab === 'chat' && thread ? thread.title : 'Assistant');
  const subtitle = $derived(
    tab === 'chat' && thread
      ? [
          thread.space_slot ? `Space ${spaceLabel(thread.space_slot)}` : 'Thread',
          thread.route_pinned ? `pinned to ${providerLabel(thread.provider, thread.model)}` : 'routed by your rules',
          thread.incognito ? 'incognito — nothing is remembered' : '',
        ]
          .filter(Boolean)
          .join(' · ')
      : 'Chats, remembers, reminds — and asks before anything leaves your Mac',
  );
  const needsCount = $derived(assistant.needsYouCount);
  // Threads belong to Chat: Tasks / Memory / Permissions get the full width.
  const showList = $derived(tab === 'chat' && (!viewport.isPhone || !selectedId));
  let listW = $state(loadPaneWidth('assistant.listW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  const showMain = $derived(!viewport.isPhone || tab !== 'chat' || !!selectedId);
;
async () => {

 { svelteHTML.createElement("div", { "class":`as-page`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: { title,subtitle,leading:() => { async ()/*Ωignore_positionΩ*/ => {
      if(viewport.isPhone && tab === 'chat' && selectedId){
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => router.go('assistant'),"aria-label":`Back to threads`,"title":`Back to threads`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});}
         }
      }
    };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
      if(tab === 'chat' && thread){
         { svelteHTML.createElement("span", {   "class":`prov`,"title":thread.route_pinned ? 'Pinned for this thread' : 'Current provider (routing rules)',});
           { const $$_nocIredivorP1C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP1C({ target: __sveltets_2_any(), props: {    "provider":thread.provider,"size":12,}});}providerLabel(thread.provider, thread.model);
         }
      }
    };return __sveltets_2_any(0)},tabs:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("div", {         "class":`segmented`,"role":`tablist`,"aria-label":`Assistant view`,"tabindex":-1,"onkeydown":onTabKey,});
           for(let t of __sveltets_2_ensureArray(TABS)){t.id;
           { svelteHTML.createElement("button", {            "role":`tab`,"aria-selected":tab === t.id,"tabindex":tab === t.id ? 0 : -1,"onclick":() => go(t.id),...__sveltets_2_empty({"data-testid":`assistant-tab-${t.id}`}),});tab === t.id;
            t.label;if(t.id === 'tasks' && needsCount > 0){ { svelteHTML.createElement("span", {   "class":`tab-count`,"title":`${needsCount} waiting on you`,});needsCount; }}
           }
        }
       }
    };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(tab === 'chat' && thread){
         { svelteHTML.createElement("button", {         "class":`btn small`,...__sveltets_2_empty({"data-icon":`terminal`}),"onclick":showWork,"disabled":!thread.session_id,"title":thread.session_id ? 'Open the CLI session behind this thread' : 'No CLI session yet — it starts with your first message',});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":12,}});}  
         }
      }
       { svelteHTML.createElement("button", {             "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-1`}),...__sveltets_2_empty({"data-icon":`gear`}),...__sveltets_2_empty({"data-label":`Routing settings`}),"onclick":() => router.go('settings/assistant'),"aria-label":`Routing settings`,"title":`Routing settings`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":14,}});}
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, badge, tabs, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
    
    
    
   PageHeader}

   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
    if(listState === 'unsupported'){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {        "variant":`page`,"icon":`assistant`,"title":`The assistant isn’t available yet`,"body":`This Otto daemon doesn’t include the assistant. Update Otto, then come back here.`,}});}
    } else if (listState === 'error' && !threads.length){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "variant":`page`,"what":`your threads`,"error":assistant.threads.error || 'Something went wrong.',"empty":true,"onretry":() => void assistant.loadThreads(),}});}
    } else if (listState === 'ready' && !threads.length && tab === 'chat'){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`assistant`,"title":`Meet Otto, your assistant`,"body":`One place to ask, plan and get things done. It runs on your Claude and Codex subscriptions, remembers what matters, and asks before anything leaves your Mac.`,"actionLabel":creating ? 'Starting…' : 'Start a conversation',"actionIcon":`plus`,"onaction":() => void newThread(1, 'Personal'),}});}
    }else{
       { svelteHTML.createElement("div", { "class":`split-wrap`,}); { svelteHTML.createElement("div", {  "class":`split`,});viewport.isPhone;
        if(showList){
           { svelteHTML.createElement("div", {   "class":`list-pane`,"style":`--list-pane-w:${listW}px`,});
            if(listState === 'loading' && !threads.length){
               { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS6C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS6C({ target: __sveltets_2_any(), props: {      "rows":6,"height":28,"label":`threads`,}});} }
            }else{
               { const $$_tsiLdaerhT5C = __sveltets_2_ensureComponent(ThreadList); new $$_tsiLdaerhT5C({ target: __sveltets_2_any(), props: {          threads,"selected":selectedId,"onopen":open,"onnew":() => void newThread(),"onnewspace":(s) => void newSpace(s),creating,}});}
            }
           }
          if(!viewport.isPhone){
             { const $$_rediviDenaP4C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP4 = new $$_rediviDenaP4C({ target: __sveltets_2_any(), props: {       width:listW,"storageKey":`assistant.listW`,"label":`Resize the threads list`,}});/*Ωignore_startΩ*/() => listW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP4.$$bindings = 'width';}
          }
        }
        if(showMain){
           { svelteHTML.createElement("section", {   "class":`main`,"aria-label":tab === 'chat' ? 'Conversation' : TABS.find((t) => t.id === tab)?.label,});
            if(tab === 'chat'){
              if(selectedError && !thread){
                 { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {        "variant":`page`,"what":`this thread`,"error":selectedError,"empty":true,"onretry":() => { if (selectedId) void assistant.loadSelectedThread(selectedId); },}});}
                 { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => { const latest = latestThreadId(threads, null); if (latest) open(latest); },});    }
              } else if (thread){
                thread.id; {
                   { const $$_weiVtahC5C = __sveltets_2_ensureComponent(ChatView); new $$_weiVtahC5C({ target: __sveltets_2_any(), props: {     thread,"onopentasks":() => go('tasks'),"onopenmemory":() => go('memory'),}});}
                }
              }else{
                 { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS6C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS6C({ target: __sveltets_2_any(), props: {      "rows":4,"height":44,"label":`the conversation`,}});} }
              }
            }else{
               { svelteHTML.createElement("div", { "class":`tab-scroll`,});
                if(tab === 'tasks'){
                   { const $$_baTsksaT6C = __sveltets_2_ensureComponent(TasksTab); new $$_baTsksaT6C({ target: __sveltets_2_any(), props: {  "onopenthread":open,}});}
                } else if (tab === 'memory'){
                   { const $$_baTyromeM6C = __sveltets_2_ensureComponent(MemoryTab); new $$_baTyromeM6C({ target: __sveltets_2_any(), props: {  "onopenthread":open,}});}
                }else{
                   { const $$_baTsnoissimreP6C = __sveltets_2_ensureComponent(PermissionsTab); new $$_baTsnoissimreP6C({ target: __sveltets_2_any(), props: {}});}
                }
               }
            }
           }
          if(!viewport.isPhone){
             { svelteHTML.createElement("div", { "class":`rail-pane`,});
               { const $$_liaRuoYsdeeN5C = __sveltets_2_ensureComponent(NeedsYouRail); new $$_liaRuoYsdeeN5C({ target: __sveltets_2_any(), props: {    "onopenthread":open,"onopentasks":() => go('tasks'),}});}
             }
          }
        }
       } }
    }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AssistantPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AssistantPage__SvelteComponent_ = ReturnType<typeof AssistantPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default AssistantPage__SvelteComponent_;