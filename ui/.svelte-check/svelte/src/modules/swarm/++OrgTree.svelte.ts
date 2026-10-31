///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
import { plural } from '../../lib/plural';
// Recursive org tree (CEO → … → devs) by `reports_to`. Each node shows the
// agent, a status dot, task/run counts, and its open sessions (click → open).
// Supports drag-and-drop to reparent agents within the hierarchy.
import Icon from '../../lib/components/Icon.svelte';
import { toastError } from '../../lib/toastError';
import { sessionState } from '../../lib/status';
import StatusDot from '../../lib/components/StatusDot.svelte';
import { events } from '../../lib/events.svelte';
import { swarm } from '../../lib/stores/swarm.svelte';
import { onMount, tick } from 'svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import type { SwarmAgent } from './types';
;

  interface Props {
    onedit: (a: SwarmAgent) => void;
    onruntask: (a: SwarmAgent) => void;
    /** Called when + button is clicked to add a new direct report to `parent`. */
    onadd?: (parent: SwarmAgent | null) => void;
    /** Duplicate `a` as a new agent (e.g. same role on a different model). */
    onduplicate?: (a: SwarmAgent) => void;
  };function $$render() {
 const node/*Ωignore_positionΩ*/ = (a: SwarmAgent, depth: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const kids = childrenOf(a.id);
  const sessions = agentSessions(a.id);
  const isOpen = open[a.id] ?? true;
  const running = runCount(a.id);
   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                                      "class":`row org-row`,"style":`padding-inline-start:${depth * 14 + 6}px`,"draggable":`true`,"ondragstart":(e) => onAgentDragStart(e, a),"ondragend":onAgentDragEnd,"ondragover":(e) => onNodeDragOver(e, a.id),"ondragleave":() => onNodeDragLeave(a.id),"ondrop":(e) => onNodeDrop(e, a.id),"oncontextmenu":(e) => menu(e, a),"role":`treeitem`,...__sveltets_2_empty({"data-tid":a.id}),"aria-label":a.title ? `${a.name}, ${a.title}` : a.name,"aria-level":depth + 1,"aria-expanded":kids.length > 0 || sessions.length > 0 ? isOpen : undefined,"aria-selected":`false`,"tabindex":tabStop === a.id ? 0 : -1,"onfocus":(e) => e.target === e.currentTarget && (focusId = a.id),});dropTargetId === a.id;draggingAgentId === a.id;
    if(kids.length > 0 || sessions.length > 0){
      
       { svelteHTML.createElement("button", {             "class":`twist`,"tabindex":-1,"aria-hidden":`true`,"aria-expanded":isOpen,"onclick":() => toggle(a.id),"aria-label":isOpen ? `Collapse ${a.name}` : `Expand ${a.name}`,"title":isOpen ? `Collapse ${a.name}` : `Expand ${a.name}`,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":isOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
       }
    }else{
       { svelteHTML.createElement("span", { "class":`twist-spacer`,}); }
    }
    
     { svelteHTML.createElement("span", { "class":`avatar-wrap`,});
       { svelteHTML.createElement("span", {   "class":`avatar`,"aria-hidden":`true`,});a.avatar || a.name.slice(0, 1); }
      if(a.status === 'paused'){
         { svelteHTML.createElement("span", {       "class":`presence paused`,"role":`img`,"aria-label":`Paused`,"title":`Paused — picks up no new work`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`pause`,"size":12,}});} }
      }else{
         { svelteHTML.createElement("span", { "class":`presence`,}); { const $$_toDsutatS3C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS3C({ target: __sveltets_2_any(), props: {    "state":agentDot(running),"size":8,}});} }
      }
     }
    
     { svelteHTML.createElement("button", {       "class":`who`,"tabindex":-1,"onclick":() => onedit(a),"title":a.title ? `${a.name} — ${a.title} · Edit agent` : `${a.name} · Edit agent`,});
       { svelteHTML.createElement("span", { "class":`name`,});a.name; }
      if(a.title){ { svelteHTML.createElement("span", { "class":`title dim`,});a.title; }}
     }
    if(a.schedule?.enabled){
       { svelteHTML.createElement("span", {       "class":`sched-icon`,"role":`img`,"aria-label":`Runs on a schedule`,"title":`Runs on a schedule`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});} }
    }
    if(running > 0){
       { svelteHTML.createElement("span", {   "class":`chip runs-chip`,"title":plural(running, 'active run'),}); { const $$_toDsutatS2C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS2C({ target: __sveltets_2_any(), props: {    "state":agentDot(running),"size":6,}});}running; }
    }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { svelteHTML.createElement("span", { "class":`row-tools`,});
       { svelteHTML.createElement("button", {         "class":`icon-btn small`,"tabindex":-1,"onclick":() => onadd?.(a),"aria-label":`Add a direct report to ${a.name}`,"title":`Add a direct report to ${a.name}`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}
       }
       { svelteHTML.createElement("button", {         "class":`icon-btn small`,"tabindex":-1,"onclick":(e) => menu(e, a),"aria-label":`Actions for ${a.name}`,"title":`Actions for ${a.name}`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
       }
     }
   }}
  if(isOpen){
       for(let s of __sveltets_2_ensureArray(sessions)){s.id;
       { svelteHTML.createElement("button", {                            "type":`button`,"class":`session-row`,"style":`padding-inline-start:${depth * 14 + 30}px`,"role":`treeitem`,...__sveltets_2_empty({"data-tid":s.id}),...__sveltets_2_empty({"data-owner":a.id}),"aria-level":depth + 2,"aria-selected":swarm.selectedSessionId === s.id,"tabindex":tabStop === s.id ? 0 : -1,"onclick":() => (swarm.selectedSessionId = s.id),"onkeydown":(e) => { if (e.key === ' ') { e.preventDefault(); swarm.selectedSessionId = s.id; } },"onfocus":() => (focusId = s.id),"title":`Open session: ${s.title || s.provider}`,});swarm.selectedSessionId === s.id;
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":12,}});}
         { svelteHTML.createElement("span", { "class":`grow mono ellipsis`,});s.title || s.provider; }
         { svelteHTML.createElement("span", { "class":`sess-status`,});sessionDot(s.id, s.status).label; }
         { const $$_toDsutatS1C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS1C({ target: __sveltets_2_any(), props: {    "state":sessionDot(s.id, s.status),"size":8,}});}
       }
    }
       for(let k of __sveltets_2_ensureArray(kids)){k.id;
      ;__sveltets_2_ensureSnippet(node(k, depth + 1));
    }
  }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { onedit, onruntask, onadd, onduplicate }: Props = $props();

  const agents = $derived(swarm.detail?.agents ?? []);
  // Orphans (reports_to names an agent that no longer exists — the daemon does
  // not cascade a manager's delete) are roots too; otherwise they vanished from
  // the tree entirely while still showing in the Graph view.
  const roots = $derived.by(() => {
    const ids = new Set(agents.map((a) => a.id));
    return agents.filter((a) => !a.reports_to || !ids.has(a.reports_to));
  });
  const childrenOf = (id: string) => agents.filter((a) => a.reports_to === id);

  // Hide finished (exited) task sessions by default — show only active work.
  let showCompleted = $state(false);

  // Sessions this agent holds (tagged at spawn with meta.swarm_id + meta.agent_id).
  // Swarm role sessions are background (`meta.source = 'swarm'`): the main
  // session list carries them only while a swarm view asks for them.
  onMount(() => ws.includeSources('swarm'));

  function agentSessions(agentId: string) {
    const sid = swarm.detail?.id;
    return ws.sessions.filter((s) => {
      const m = (s.meta ?? {}) as Record<string, unknown>;
      if (s.archived || m.swarm_id !== sid || m.agent_id !== agentId) return false;
      const st = ws.statusMap[s.id] ?? s.status;
      return showCompleted || st !== 'exited';
    });
  }

  // One dot vocabulary (lib/status.ts): the events socket being down turns a
  // "working" claim into the stale ring instead of a lie.
  const stale = $derived(events.state !== 'connected');
  const sessionDot = (id: string, raw: string) =>
    sessionState(null, ws.statusMap[id] ?? raw, ws.needsYou[id] === true, { stale });
  /** An agent is "working" while it holds a live run, otherwise idle. */
  const agentDot = (live: number) => sessionState(null, live > 0 ? 'working' : 'idle', false, { stale });

  function runCount(agentId: string): number {
    return swarm.agentRunStats(agentId).live;
  }

  let open = $state<Record<string, boolean>>({});
  function toggle(id: string) {
    open[id] = !(open[id] ?? true);
    open = { ...open };
  }

  // Reparent from the menu or a drop; a failed PATCH must not vanish silently
  // (the row would just snap back with no explanation).
  async function reparent(aid: string, reportsTo: string | null) {
    try {
      await swarm.updateAgent(aid, { reports_to: reportsTo });
    } catch (e) {
      toastError('Couldn’t move the agent', e);
    }
  }

  // Deleting an agent is irreversible (its config, soul and schedule go with
  // it), so it confirms like every other swarm delete does.
  async function deleteAgent(a: SwarmAgent) {
    const reports = childrenOf(a.id).length;
    const extra =
      reports === 0
        ? ''
        : reports === 1
          ? ' Its direct report moves to the top level.'
          : ` Its ${reports} direct reports move to the top level.`;
    const ok = await confirmer.ask(`Delete agent “${a.name}”?${extra} This cannot be undone.`, {
      title: 'Delete agent',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    try {
      await swarm.deleteAgent(a.id);
    } catch (e) {
      toastError('Couldn’t delete the agent', e);
    }
  }

  function menu(e: MouseEvent | KeyboardEvent, a: SwarmAgent) {
    ctxMenu.show(e, [
      { label: 'Edit agent', icon: 'edit', action: () => onedit(a) },
      { label: 'Duplicate agent', icon: 'split', action: () => onduplicate?.(a) },
      { label: 'Run a task…', icon: 'play', action: () => onruntask(a) },
      { label: 'Add direct report', icon: 'plus', action: () => onadd?.(a) },
      { separator: true },
      { label: 'Move to top level', icon: 'user', action: () => reparent(a.id, null) },
      { separator: true },
      { label: 'Delete agent…', icon: 'trash', danger: true, action: () => deleteAgent(a) },
    ]);
  }

  // --- Tree keyboard (WAI-ARIA tree view, roving tabindex) -------------------
  // Agents and their sessions are treeitems; one is in the Tab order (the last
  // focused, else the first root). ↑/↓/Home/End move along the visible rows,
  // → expands an agent or steps into it, ← collapses or steps out to the
  // manager (mirrored under RTL). Enter opens: an agent's editor, a session's
  // terminal. ContextMenu / ⇧F10 opens the agent menu (it holds every row-tool
  // action, so the hover buttons stay pointer shortcuts).
  let treeEl = $state<HTMLElement | null>(null);
  let focusId = $state<string | null>(null);
  const tabStop = $derived(
    focusId && (agents.some((a) => a.id === focusId) || ws.sessions.some((s) => s.id === focusId)) ? focusId : (roots[0]?.id ?? null),
  );

  function items(): HTMLElement[] {
    return treeEl ? [...treeEl.querySelectorAll<HTMLElement>('[role="treeitem"]')] : [];
  }
  function focusItem(id: string | null | undefined): void {
    if (!id) return;
    focusId = id;
    void tick().then(() => treeEl?.querySelector<HTMLElement>(`[data-tid="${CSS.escape(id)}"]`)?.focus());
  }
  function onTreeKey(e: KeyboardEvent): void {
    const el = (e.target as HTMLElement).closest<HTMLElement>('[role="treeitem"]');
    if (!el || el !== e.target) return;
    const id = el.dataset.tid ?? '';
    const agent = agents.find((a) => a.id === id);
    const list = items();
    const i = list.indexOf(el);
    const rtl = getComputedStyle(el).direction === 'rtl';
    const key = rtl && e.key === 'ArrowRight' ? 'ArrowLeft' : rtl && e.key === 'ArrowLeft' ? 'ArrowRight' : e.key;
    const expanded = el.getAttribute('aria-expanded');
    if (key === 'ArrowDown' || key === 'ArrowUp' || key === 'Home' || key === 'End') {
      e.preventDefault();
      const next = key === 'Home' ? 0 : key === 'End' ? list.length - 1 : i + (key === 'ArrowDown' ? 1 : -1);
      focusItem(list[Math.max(0, Math.min(list.length - 1, next))]?.dataset.tid);
    } else if (key === 'ArrowRight') {
      e.preventDefault();
      if (agent && expanded === 'false') toggle(agent.id);
      else if (agent && expanded === 'true') focusItem(list[i + 1]?.dataset.tid);
    } else if (key === 'ArrowLeft') {
      e.preventDefault();
      if (agent && expanded === 'true') toggle(agent.id);
      else focusItem(agent ? agent.reports_to : el.dataset.owner);
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      if (agent) onedit(agent);
      else swarm.selectedSessionId = id;
    } else if (agent && (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey))) {
      e.preventDefault();
      menu(e, agent);
    }
  }

  // --- Drag-and-drop reparenting -------------------------------------------
  let draggingAgentId = $state<string | null>(null);
  let dropTargetId = $state<string | null>(null); // null = root drop zone

  function onAgentDragStart(e: DragEvent, a: SwarmAgent) {
    draggingAgentId = a.id;
    e.dataTransfer?.setData('text/plain', a.id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }

  function onAgentDragEnd() {
    draggingAgentId = null;
    dropTargetId = null;
  }

  function onNodeDragOver(e: DragEvent, targetId: string | null) {
    // Prevent dropping on self or a descendant.
    if (draggingAgentId === targetId) return;
    if (targetId !== null && isDescendant(draggingAgentId, targetId)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dropTargetId = targetId;
  }

  function onNodeDragLeave(targetId: string | null) {
    if (dropTargetId === targetId) dropTargetId = null;
  }

  async function onNodeDrop(e: DragEvent, newParentId: string | null) {
    e.preventDefault();
    const aid = e.dataTransfer?.getData('text/plain') ?? draggingAgentId;
    draggingAgentId = null;
    dropTargetId = null;
    if (!aid) return;
    const a = agents.find((x) => x.id === aid);
    if (!a) return;
    if (a.reports_to === newParentId) return; // no change
    if (newParentId !== null && isDescendant(aid, newParentId)) return; // cycle guard
    await reparent(aid, newParentId);
  }

  /** Returns true if `potentialChild` is a descendant of `ancestorId`. */
  function isDescendant(potentialChildId: string | null, ancestorId: string): boolean {
    if (!potentialChildId) return false;
    let cur: string | null = ancestorId;
    let safety = 0;
    while (cur && safety++ < 50) {
      const a = agents.find((x) => x.id === cur);
      if (!a) break;
      if (a.reports_to === potentialChildId) return true;
      cur = a.reports_to ?? null;
    }
    return false;
  }
;
async () => {


  { svelteHTML.createElement("div", {            "class":`tree`,"role":`group`,"aria-label":`Agent org tree`,"ondragover":(e) => onNodeDragOver(e, null),"ondragleave":() => onNodeDragLeave(null),"ondrop":(e) => onNodeDrop(e, null),});
   { svelteHTML.createElement("div", { "class":`tree-bar`,});
     { svelteHTML.createElement("label", { "class":`show-done`,});
       { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":showCompleted,});/*Ωignore_startΩ*/() => showCompleted = __sveltets_2_any(null);/*Ωignore_endΩ*/}   
     }
   }
  if(roots.length === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {             "icon":`user`,"title":`No agents yet`,"body":`Recruit agents in the header, or add one by hand. They report to each other in an org tree; drag a row onto another to change who it reports to.`,"actionLabel":onadd ? 'Add agent' : undefined,"actionIcon":`plus`,"onaction":() => onadd?.(null),}});}
  } else if (draggingAgentId){
     { svelteHTML.createElement("div", {  "class":`drop-zone`,});dropTargetId === null;
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`user`,"size":13,}});}     
     }
  }
  if(roots.length > 0){
     { const $$_div1 = svelteHTML.createElement("div", {        "role":`tree`,"tabindex":-1,"aria-label":`Agents`,"onkeydown":onTreeKey,});treeEl = $$_div1;
         for(let r of __sveltets_2_ensureArray(roots)){r.id;
        ;__sveltets_2_ensureSnippet(node(r, 0));
      }
     }
  }
  if(!draggingAgentId && roots.length > 0){
     { svelteHTML.createElement("button", {     "class":`add-top-btn dim`,"onclick":() => onadd?.(null),"title":`Add an agent at the top level`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}  
     }
  }
 }




};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const OrgTree__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type OrgTree__SvelteComponent_ = ReturnType<typeof OrgTree__SvelteComponent_>;
/*Ωignore_endΩ*/export default OrgTree__SvelteComponent_;