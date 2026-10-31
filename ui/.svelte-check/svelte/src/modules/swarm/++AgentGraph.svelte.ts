///<reference types="svelte" />
;// Agent-centric swarm graph: TEAM MEMBERS are the nodes (not tasks), laid out
// radially around the coordinator by `reports_to`. Click a node for Edit,
// Open session, and Add report. A side rail shows a per-member brief.

import { sentenceCase, sessionState, type SessionStateInfo } from '../../lib/status';
import StatusDot from '../../lib/components/StatusDot.svelte';
import { events } from '../../lib/events.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { plural } from '../../lib/plural';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { swarm } from '../../lib/stores/swarm.svelte';
import { onMount } from 'svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { now } from '../../lib/stores/now.svelte';
import type { SwarmAgent, SwarmRun } from './types';
;

  interface Props {
    onedit?: (agent: SwarmAgent) => void;
    onadd?: (parent: SwarmAgent) => void;
  };;
// vertical spacing per depth

  interface Pos {
    x: number;
    y: number;
    depth: number;
  };;

  type Activity = 'working' | 'waiting' | 'queued' | 'open' | 'idle';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { onedit, onadd }: Props = $props();

  const agents = $derived(swarm.detail?.agents ?? []);
  const allTasks = $derived.by(() => {
    const projectIds = new Set((swarm.detail?.projects ?? []).map((p) => p.id));
    return [...projectIds].flatMap((id) => swarm.tasksByProject[id] ?? []);
  });
  // One pass over the runs, shared by every helper below (was a filter per
  // agent per helper per render).
  const liveTaskIds = $derived(swarm.runStats.liveTaskIds);

  // ── Tidy top-down TREE: root (coordinator) at the top, reports branching down
  //    level by level. x = tidy leaf packing (parents centered over children),
  //    y = depth row. Positions are node CENTERS. ─────────────────────────────
  const NODE_W = 178;
  const NODE_H = 64;
  const GAP_X = 206; // horizontal spacing per leaf
  const LEVEL_H = 142; 
  const positions = $derived.by((): Map<string, Pos> => {
    const ags = agents;
    const pos = new Map<string, Pos>();
    if (ags.length === 0) return pos;
    const childrenOf = (id: string | null) =>
      ags.filter((a) => (a.reports_to ?? null) === id).sort((a, b) => a.order_idx - b.order_idx);
    const seen = new Set<string>();
    let cursor = 0;
    const assign = (id: string, depth: number): number => {
      if (seen.has(id)) return cursor * GAP_X; // reports_to cycle guard
      seen.add(id);
      const kids = childrenOf(id);
      let x: number;
      if (kids.length === 0) {
        x = cursor * GAP_X;
        cursor += 1;
      } else {
        const xs = kids.map((k) => assign(k.id, depth + 1));
        x = (xs[0] + xs[xs.length - 1]) / 2; // centre the parent over its span
      }
      pos.set(id, { x, y: depth * LEVEL_H, depth });
      return x;
    };
    for (const r of childrenOf(null)) assign(r.id, 0);
    // Orphans (reports_to points at a missing agent) → lay out as extra roots.
    for (const a of ags) if (!pos.has(a.id)) assign(a.id, 0);
    return pos;
  });

  // Edges: parent (bottom) → child (top), a vertical S-curve.
  const edges = $derived(
    agents
      .filter((a) => a.reports_to && positions.has(a.reports_to) && positions.has(a.id))
      .map((a) => ({ from: a.reports_to as string, to: a.id })),
  );
  function edgePath(p: Pos, c: Pos): string {
    const y1 = p.y + NODE_H / 2;
    const y2 = c.y - NODE_H / 2;
    const my = (y1 + y2) / 2;
    return `M ${p.x} ${y1} C ${p.x} ${my}, ${c.x} ${my}, ${c.x} ${y2}`;
  }

  // ── Per-agent derivations (sessions / activity / counts) ────────────────────
  // Swarm role sessions are background (`meta.source = 'swarm'`): the main
  // session list carries them only while a swarm view asks for them.
  onMount(() => ws.includeSources('swarm'));

  function agentSessions(agentId: string) {
    const sid = swarm.detail?.id;
    return ws.sessions.filter((s) => {
      const m = (s.meta ?? {}) as Record<string, unknown>;
      if (s.archived || m.swarm_id !== sid || m.agent_id !== agentId) return false;
      return (ws.statusMap[s.id] ?? s.status) !== 'exited';
    });
  }

  function activeRun(agentId: string): SwarmRun | null {
    return swarm.agentRunStats(agentId).active;
  }

  function activity(agentId: string): Activity {
    const run = activeRun(agentId);
    const sess = agentSessions(agentId);
    const liveSess = sess.some((s) => {
      const st = ws.statusMap[s.id] ?? s.status;
      return st === 'running' || st === 'working';
    });
    if (run?.status === 'running' || liveSess) return 'working';
    // Amber is for "needs you" only: a run waiting on a person/approval. A
    // queued run (no slot yet) is just neutral.
    if (run?.status === 'waiting') return 'waiting';
    if (run?.status === 'queued') return 'queued';
    if (sess.length > 0) return 'open';
    return 'idle';
  }

  // One dot + label vocabulary (lib/status.ts `sessionState`): working is
  // green, a run waiting on a person is "Needs you" (amber), everything else
  // is calm. While the events socket is down a "working" claim reads as the
  // stale "Reconnecting…" ring instead of a lie.
  const stale = $derived(events.state !== 'connected');
  function agentState(a: SwarmAgent): SessionStateInfo {
    const opts = { stale };
    switch (activity(a.id)) {
      case 'working':
        return sessionState(null, 'working', false, opts);
      case 'waiting':
        return sessionState(null, 'idle', true, opts);
      case 'queued':
        return { ...sessionState(null, 'idle', false, opts), label: 'Queued', hint: 'Queued — waiting for a free slot' };
      case 'open':
        return { ...sessionState(null, 'running', false, opts), label: 'Session open', hint: 'A session is open for this agent' };
      default:
        return a.status === 'paused'
          ? { ...sessionState(null, 'idle', false, opts), label: 'Paused', hint: 'Paused — picks up no new work' }
          : sessionState(null, 'idle', false, opts);
    }
  }
  const sessionDot = (id: string, raw: string): SessionStateInfo =>
    sessionState(null, ws.statusMap[id] ?? raw, ws.needsYou[id] === true, { stale });

  function elapsed(agentId: string): string | null {
    const run = activeRun(agentId);
    if (!run?.started_at) return null;
    void now(); // reactive 1s tick
    const ms = Date.now() - new Date(run.started_at).getTime();
    if (ms < 0) return null;
    const s = Math.floor(ms / 1000);
    return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s`;
  }

  const completedCount = (agentId: string) => swarm.agentRunStats(agentId).done;

  function toAddress(agentId: string): number {
    return allTasks.filter(
      (t) => t.assignee_agent_id === agentId && liveTaskIds.has(t.id),
    ).length;
  }

  // ── Task rail: currently live tasks + search ───────────────────────────────
  const PRIO: Record<string, number> = { urgent: 3, high: 2, medium: 1, low: 0 };
  const SBIAS: Record<string, number> = {
    in_progress: 5, blocked: 4, todo: 3, in_review: 2, backlog: 1, done: 0, cancelled: 0,
  };
  const liveTasks = $derived(
    [...allTasks]
      .filter((t) => liveTaskIds.has(t.id))
      .sort(
        (a, b) =>
          (PRIO[b.priority] ?? 0) - (PRIO[a.priority] ?? 0) ||
          (SBIAS[b.status] ?? 0) - (SBIAS[a.status] ?? 0) ||
          a.order_idx - b.order_idx,
      ),
  );
  let taskQuery = $state('');
  // 5 most-urgent by default; searching reveals every match.
  const shownTasks = $derived.by(() => {
    const q = taskQuery.trim().toLowerCase();
    if (!q) return liveTasks.slice(0, 5);
    return liveTasks.filter((t) => t.title.toLowerCase().includes(q));
  });

  // ── Click → open the agent's (most recent) session; the node's session
  // button lists them all (ctxMenu: opens on click / Enter / Space, clamped
  // into the viewport and scrollable, unlike the old hover-only tooltip).
  function sessionMenu(e: MouseEvent, a: SwarmAgent): void {
    const sess = agentSessions(a.id);
    ctxMenu.show(
      e,
      sess.map((s) => ({
        label: s.title || s.provider,
        icon: 'terminal' as const,
        hint: sessionDot(s.id, s.status).label,
        action: () => (swarm.selectedSessionId = s.id),
      })),
    );
  }
  function agentMenu(e: MouseEvent, a: SwarmAgent): void {
    const sess = agentSessions(a.id);
    ctxMenu.show(e, [
      { label: 'Edit agent', icon: 'edit', action: () => onedit?.(a) },
      {
        label: 'Open session',
        icon: 'terminal',
        disabled: sess.length === 0,
        title: sess.length === 0
          ? `${a.name} has no open session. Start a task from the Board.`
          : `Open ${a.name}’s session`,
        action: () => {
          if (sess[0]) swarm.selectedSessionId = sess[0].id;
        },
      },
      { label: 'Add report', icon: 'plus', action: () => onadd?.(a) },
    ]);
  }

  // ── Pan + zoom (drag background, wheel to zoom) ──────────────────────────────
  let tx = $state(0);
  let ty = $state(0);
  let scale = $state(1);
  let drag: { sx: number; sy: number; ox: number; oy: number } | null = null;
  let wrapEl = $state<HTMLDivElement | null>(null);
  // While false, the graph auto-fits (initial layout + container resize). Any
  // manual pan/zoom sets it true so we stop fighting the user's gesture.
  let userTouched = $state(false);

  const AUTO_FIT_MIN = 1;
  // Fit ALL nodes into view (with padding); never zoom IN past 1:1, nor out
  // past legibility (AUTO_FIT_MIN) unless `all` (the Fit to view button).
  function fit(all = false): void {
    if (!wrapEl || positions.size === 0) return;
    let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
    for (const p of positions.values()) {
      minX = Math.min(minX, p.x - NODE_W / 2);
      maxX = Math.max(maxX, p.x + NODE_W / 2);
      minY = Math.min(minY, p.y - NODE_H / 2);
      maxY = Math.max(maxY, p.y + NODE_H / 2);
    }
    const bw = Math.max(1, maxX - minX);
    const bh = Math.max(1, maxY - minY);
    const w = wrapEl.clientWidth;
    const h = wrapEl.clientHeight;
    if (w === 0 || h === 0) return;
    const pad = 44;
    const fitS = Math.min((w - pad * 2) / bw, (h - pad * 2) / bh);
    // Scaled below 1:1 the node text drops under 11px (the legibility floor),
    // so the auto-fit stops at 1:1: a wide org then opens top-anchored and
    // centred on its root, readable, and pans for the rest (Fit to view still
    // shows the whole org, small).
    const s = Math.min(1, Math.max(all ? 0.2 : AUTO_FIT_MIN, fitS));
    scale = s;
    const clamped = !all && fitS < AUTO_FIT_MIN;
    // The root(s): the top row of the layout.
    const top = [...positions.values()].filter((p) => p.y - NODE_H / 2 <= minY + 1);
    const rootX = top.reduce((a, p) => a + p.x, 0) / Math.max(1, top.length);
    tx = w / 2 - (clamped ? rootX : (minX + maxX) / 2) * s;
    ty = clamped ? pad - minY * s : h / 2 - ((minY + maxY) / 2) * s;
  }

  // Auto-fit on first layout, when the node set changes, and on container resize
  // — until the user manually pans/zooms.
  $effect(() => {
    if (!wrapEl) return;
    void positions.size; // re-run when the graph changes
    if (!userTouched) fit();
    const ro = new ResizeObserver(() => {
      if (!userTouched) fit();
    });
    ro.observe(wrapEl);
    return () => ro.disconnect();
  });

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    userTouched = true;
    // Pinch (ctrl+wheel on macOS) zooms; a plain two-finger scroll pans.
    if (!e.ctrlKey) {
      tx -= e.deltaX;
      ty -= e.deltaY;
      return;
    }
    scale = Math.min(2, Math.max(0.2, scale * (e.deltaY < 0 ? 1.1 : 0.9)));
  }
  function startPan(e: PointerEvent) {
    const t = e.target as HTMLElement;
    if (t.closest('.node')) return;
    userTouched = true;
    drag = { sx: e.clientX, sy: e.clientY, ox: tx, oy: ty };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    if (!drag) return;
    tx = drag.ox + (e.clientX - drag.sx);
    ty = drag.oy + (e.clientY - drag.sy);
  }
  function endPan() {
    drag = null;
  }
  function zoomBy(f: number) {
    userTouched = true;
    scale = Math.min(2, Math.max(0.2, scale * f));
  }
  /** Fit to view: the whole org, however small (then holds — a resize won't
   *  snap it back to the legible auto-fit). */
  function recenter() {
    userTouched = true;
    fit(true);
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`agent-graph`,});
   { svelteHTML.createElement("div", { "class":`graph-col`,});
     { svelteHTML.createElement("div", { "class":`controls`,});
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => zoomBy(1.15),"aria-label":`Zoom in`,"title":`Zoom in`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});} }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => zoomBy(0.87),"aria-label":`Zoom out`,"title":`Zoom out`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`minimize`,"size":14,}});} }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":recenter,"aria-label":`Fit to view`,"title":`Fit to view`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`maximize`,"size":14,}});} }
     }

    if(agents.length === 0){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {      "icon":`user`,"title":`No team yet`,"body":`Recruit agents and they’ll appear here as a live org graph.`,}});}
    }else{
       { const $$_div2 = svelteHTML.createElement("div", {                "class":`canvas`,"onpointerdown":startPan,"onpointermove":onMove,"onpointerup":endPan,"onwheel":onWheel,"role":`application`,"aria-label":`Agent org graph`,});wrapEl = $$_div2;
         { svelteHTML.createElement("div", {   "class":`viewport`,"style":`transform: translate(${tx}px,${ty}px) scale(${scale});`,});
           { svelteHTML.createElement("svg", {   "class":`edges`,"aria-hidden":`true`,});
               for(let e of __sveltets_2_ensureArray(edges)){e.from + e.to;
              const p = positions.get(e.from);
              const c = positions.get(e.to);
              if(p && c){
                 { svelteHTML.createElement("path", {    "d":edgePath(p, c),"class":`edge`,});}
              }
            }
           }

             for(let a of __sveltets_2_ensureArray(agents)){a.id;
            const pos = positions.get(a.id);
            if(pos){
              const act = activity(a.id);
              const sess = agentSessions(a.id);
              const el = elapsed(a.id);
               { svelteHTML.createElement("div", {     "class":`node act-${act}`,"style":`left:${pos.x - NODE_W / 2}px; top:${pos.y - NODE_H / 2}px; width:${NODE_W}px; min-height:${NODE_H}px`,});
                 { svelteHTML.createElement("button", {         "class":`node-hit`,"onclick":(e) => agentMenu(e, a),"aria-haspopup":`menu`,"aria-label":`Actions for ${a.name}`,"title":`Actions for ${a.name}`,});
                   { svelteHTML.createElement("span", { "class":`avatar`,});a.avatar || a.name.slice(0, 1); }
                   { svelteHTML.createElement("span", { "class":`node-body`,});
                     { svelteHTML.createElement("span", { "class":`node-top`,});
                       { svelteHTML.createElement("span", { "class":`node-name`,});a.name; }
                       { const $$_toDsutatS8C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS8C({ target: __sveltets_2_any(), props: {    "state":agentState(a),"size":7,}});}
                     }
                     { svelteHTML.createElement("span", { "class":`node-status dim`,});agentState(a).label; }
                     { svelteHTML.createElement("span", { "class":`node-foot`,});
                       { svelteHTML.createElement("span", { "class":`role`,});a.specialization || a.title; }
                      if(el){ { svelteHTML.createElement("span", { "class":`el`,}); { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});} el; }}
                     }
                   }
                 }

                if(sess.length > 0){
                   { svelteHTML.createElement("button", {           "class":`sess-btn`,"aria-haspopup":`menu`,"aria-label":`${a.name}: ${plural(sess.length, 'session')} — choose one to open`,"title":`${a.name}: ${plural(sess.length, 'session')} — choose one to open`,"onclick":(e) => sessionMenu(e, a),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":12,}});} sess.length;  { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});} }
                }
               }
            }
          }
         }
       }
    }
   }

  
   { svelteHTML.createElement("aside", { "class":`side`,});
     { svelteHTML.createElement("section", { "class":`brief`,});
       { svelteHTML.createElement("div", { "class":`side-head`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`user`,"size":12,}});}   }
       { svelteHTML.createElement("div", { "class":`brief-list`,});
           for(let a of __sveltets_2_ensureArray(agents)){a.id;
          const addr = toAddress(a.id);
           { svelteHTML.createElement("button", {         "class":`brief-row`,"onclick":(e) => agentMenu(e, a),"aria-haspopup":`menu`,"aria-label":`Actions for ${a.name}`,"title":`Actions for ${a.name}`,});
             { svelteHTML.createElement("span", { "class":`avatar sm`,});a.avatar || a.name.slice(0, 1); }
             { svelteHTML.createElement("span", { "class":`grow ellipsis`,});a.name; }
             { svelteHTML.createElement("span", {   "class":`stat done`,"title":`Completed runs`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} completedCount(a.id); }
             { svelteHTML.createElement("span", {    "class":`stat addr`,"title":`Tasks or runs that need attention`,});addr > 0; { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});} addr; }
           }
        }
       }
     }

     { svelteHTML.createElement("section", { "class":`tasks`,});
       { svelteHTML.createElement("div", { "class":`side-head`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}   }
       { svelteHTML.createElement("div", { "class":`search`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
         { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`search-input`,"aria-label":`Filter live tasks`,"placeholder":`Filter live tasks…`,"bind:value":taskQuery,});/*Ωignore_startΩ*/() => taskQuery = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", {     "class":`task-list`,"role":`list`,"aria-label":`Live tasks`,});
           for(let t of __sveltets_2_ensureArray(shownTasks)){t.id;
          const ag = swarm.agentById(t.assignee_agent_id);
           { svelteHTML.createElement("div", {     "class":`task-card prio-${t.priority}`,"role":`listitem`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}
             { svelteHTML.createElement("span", { "class":`task-main`,});
               { svelteHTML.createElement("span", {   "class":`task-title ellipsis2`,"title":t.title,});t.title; }
               { svelteHTML.createElement("span", { "class":`task-meta dim`,});
                 { svelteHTML.createElement("span", {   "class":`pchip prio-${t.priority}`,"title":`Priority`,});sentenceCase(t.priority); }
                 { svelteHTML.createElement("span", {});sentenceCase(t.status); }
                if(ag){ { svelteHTML.createElement("span", {}); ag.name; }}
               }
             }
           }
        }
        if(shownTasks.length === 0){
           { svelteHTML.createElement("p", { "class":`dim empty`,});taskQuery ? 'No matching live tasks.' : 'No live tasks.'; }
        }
       }
     }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentGraph__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentGraph__SvelteComponent_ = ReturnType<typeof AgentGraph__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentGraph__SvelteComponent_;