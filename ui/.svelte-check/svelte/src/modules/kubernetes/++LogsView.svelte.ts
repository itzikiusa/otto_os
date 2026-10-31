///<reference types="svelte" />
;// Drawer "Logs" tab: container selector, tail / since / previous, follow
// (streams `kubectl logs -f` over a fetch ReadableStream, aborted on any
// option change or unmount), timestamps, in-buffer search with highlights,
// download. Lines are windowed with `VirtualList` and capped so a chatty
// pod can't grow the DOM or memory without bound.

import { untrack, tick } from 'svelte';
import VirtualList from '../../lib/components/VirtualList.svelte';
import { providerErrorText } from '../../lib/loadError';
import { appendCapped, appendFiltered, pushPendingCapped } from './logRing';
import Icon from '../../lib/components/Icon.svelte';
import { downloadText } from '../../lib/components/exporters';
import { followLogs } from '../../lib/api/k8s';
import type { K8sContainer } from '../../lib/api/types';
import { safeName } from './k8s-util';
;

  interface Props {
    clusterId: string;
    ns: string;
    /** One pod — or, for a workload, empty with `selector` set. */
    pod?: string;
    /** Workload mode: stream every pod matching this label selector; lines
     *  arrive `[pod/<pod>/<container>] `-prefixed and get a per-pod filter. */
    selector?: string;
    /** Display name for downloads in workload mode. */
    title?: string;
    containers: K8sContainer[];
    /** Workload mode: jump to one pod's own drawer (Logs tab). */
    onopenpod?: (pod: string) => void;
  };;
  interface Parsed { pod: string; ctr: string; text: string };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { clusterId, ns, pod = '', selector = '', title = '', containers, onopenpod }: Props = $props();

  const multi = $derived(!pod && !!selector);
  /** `[pod/<pod>/<container>] rest` → parts (workload mode only). */
  const PREFIX = /^\[pod\/([^/\]]+)\/([^\]]+)\] ?(.*)$/;

  function parse(line: string): Parsed {
    const m = multi ? PREFIX.exec(line) : null;
    return m ? { pod: m[1], ctr: m[2], text: m[3] } : { pod: '', ctr: '', text: line };
  }
  /** Stable color per pod so interleaved lines are scannable. */
  const HUES = [205, 150, 30, 280, 350, 90, 240, 60];
  const podColors = new Map<string, number>();
  function hue(p: string): number {
    let h = podColors.get(p);
    if (h === undefined) {
      h = HUES[podColors.size % HUES.length];
      podColors.set(p, h);
    }
    return h;
  }
  let podFilter = $state('');

  const MAX_LINES = 20_000;
  const LINE_H = 18;
  const TAILS = [100, 500, 1000, 5000];
  const SINCES: { v: string; l: string }[] = [
    { v: '', l: 'all' },
    { v: '5m', l: '5m' },
    { v: '15m', l: '15m' },
    { v: '1h', l: '1h' },
    { v: '6h', l: '6h' },
    { v: '24h', l: '24h' },
  ];

  let container = $state('');
  let tail = $state(500);
  let since = $state('');
  let previous = $state(false);
  let follow = $state(false);
  let timestamps = $state(false);
  let search = $state('');
  // Raw ring buffers: a deep proxy over 20k lines made every per-frame flush
  // copy the buffer through the proxy. perf K8s R5: flushes append IN PLACE
  // (logRing.ts) and bump `ver` instead of concat-copying 20k lines per
  // frame; a reset / filter change still assigns a new array.
  let lines = $state.raw<string[]>([]);
  let ver = $state(0);
  const lineCount = $derived((void ver, lines.length));
  /** `lines` narrowed by the pod filter + search — maintained incrementally
   *  per flush (only the new lines are tested); recomputed in full only when
   *  the filter itself changes. */
  let shown = $state.raw<string[]>([]);
  /** Pods seen since the stream started (workload mode) — grown per flush. */
  let pods = $state.raw<string[]>([]);
  const podSet = new Set<string>();
  let streaming = $state(false);
  let error = $state('');
  let autoScroll = $state(true);
  let wrapEl = $state<HTMLDivElement | null>(null);
  let searchEl = $state<HTMLInputElement | null>(null);

  let abort: AbortController | null = null;
  let carry = '';
  /** Batch incoming chunks into one state write per animation frame. */
  let pending: string[] = [];
  let flushRaf: number | null = null;

  function matches(l: string, qq: string, pf: string): boolean {
    if (multi && pf && !l.startsWith(`[pod/${pf}/`)) return false;
    return !qq || l.toLowerCase().includes(qq);
  }

  function flush(): void {
    flushRaf = null;
    if (!pending.length) return;
    const add = pending;
    pending = [];
    const filtered = shown !== lines;
    const trimmed = appendCapped(lines, add, MAX_LINES);
    if (multi) {
      let grew = false;
      for (const l of add) {
        const m = PREFIX.exec(l);
        if (m && !podSet.has(m[1])) {
          podSet.add(m[1]);
          grew = true;
        }
      }
      if (grew) pods = [...podSet].sort();
    }
    if (filtered) {
      // Append the new matches; drop as many matches as the ring trimmed.
      const qq = q;
      const pf = podFilter;
      appendFiltered(shown, add, trimmed, (l) => matches(l, qq, pf));
    }
    ver++;
  }

  function ingest(text: string): void {
    const parts = (carry + text).split('\n');
    carry = parts.pop() ?? '';
    if (parts.length) pushPendingCapped(pending, parts, MAX_LINES);
    scheduleFlush();
  }

  /** rAF while visible; a timer while hidden (rAF is paused then, and the
   *  ring should still be current when the window comes back). */
  let flushTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleFlush(): void {
    if (flushRaf !== null || flushTimer !== null) return;
    if (typeof document !== 'undefined' && document.hidden) {
      flushTimer = setTimeout(() => {
        flushTimer = null;
        flush();
      }, 500);
    } else {
      flushRaf = requestAnimationFrame(flush);
    }
  }

  async function start(): Promise<void> {
    abort?.abort();
    const ac = new AbortController();
    abort = ac;
    lines = [];
    // Unfiltered, the view IS the ring (one array); filtered, its own.
    shown = q || (multi && podFilter) ? [] : lines;
    ver++;
    podSet.clear();
    pods = [];
    pending = [];
    carry = '';
    error = '';
    streaming = true;
    try {
      await followLogs(
        clusterId,
        ns,
        multi ? { selector } : { pod },
        { container: container || undefined, tail, since: since || undefined, previous, follow, timestamps },
        ingest,
        ac.signal,
      );
      // Superseded while awaiting: `carry` / `pending` now belong to the new
      // stream — leave them alone.
      if (ac.signal.aborted) return;
      if (carry) {
        pending.push(carry);
        carry = '';
      }
      flush();
    } catch (e) {
      if (!ac.signal.aborted) error = providerErrorText(e);
    } finally {
      if (abort === ac) streaming = false;
    }
  }

  // Default container once the list arrives (first non-init). Workload mode
  // defaults to ALL containers (kubectl --all-containers).
  $effect(() => {
    const first = multi ? '' : (containers.find((c) => !c.init)?.name ?? '');
    untrack(() => {
      if (!container || !containers.some((c) => c.name === container)) container = first;
    });
  });

  // (Re)start whenever an option changes; abort on unmount.
  $effect(() => {
    void clusterId;
    void ns;
    void pod;
    void selector;
    void container;
    void tail;
    void since;
    void previous;
    void follow;
    void timestamps;
    untrack(() => void start());
    return () => {
      abort?.abort();
      abort = null;
      if (flushRaf !== null) cancelAnimationFrame(flushRaf);
      flushRaf = null;
      if (flushTimer !== null) clearTimeout(flushTimer);
      flushTimer = null;
    };
  });

  // The search applies ~100 ms after typing stops on a big buffer (each
  // change re-filters every buffered line).
  let searchQ = $state('');
  $effect(() => {
    const v = search;
    const big = untrack(() => lines.length) > 2000;
    if (!big) {
      searchQ = v;
      return;
    }
    const t = setTimeout(() => (searchQ = v), 100);
    return () => clearTimeout(t);
  });
  const q = $derived(searchQ.trim().toLowerCase());
  // Full recompute only when the filter changes (flush() keeps it current).
  $effect(() => {
    const qq = q;
    const pf = podFilter;
    const m = multi;
    untrack(() => {
      shown = qq || (m && pf) ? lines.filter((l) => matches(l, qq, pf)) : lines;
      ver++;
    });
  });
  const matchCount = $derived((void ver, q ? shown.length : 0));

  // Stick to the bottom while following (unless the user scrolled up).
  $effect(() => {
    void ver;
    void shown;
    if (!autoScroll) return;
    void tick().then(() => {
      const sc = wrapEl?.querySelector<HTMLElement>('.vlist');
      if (sc) sc.scrollTop = sc.scrollHeight;
    });
  });

  function onScroll(e: Event): void {
    const el = e.target as HTMLElement;
    if (!el.classList.contains('vlist')) return;
    autoScroll = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  }

  /** Split a line around case-insensitive matches for <mark> rendering. */
  function segments(line: string): { t: string; m: boolean }[] {
    if (!q) return [{ t: line, m: false }];
    const out: { t: string; m: boolean }[] = [];
    const low = line.toLowerCase();
    let i = 0;
    for (;;) {
      const j = low.indexOf(q, i);
      if (j < 0) break;
      if (j > i) out.push({ t: line.slice(i, j), m: false });
      out.push({ t: line.slice(j, j + q.length), m: true });
      i = j + q.length;
    }
    if (i < line.length) out.push({ t: line.slice(i), m: false });
    return out;
  }

  function download(): void {
    const base = pod || title || 'workload';
    downloadText(shown.join('\n') + '\n', `${safeName(base)}${podFilter ? '-' + safeName(podFilter) : ''}${container ? '-' + safeName(container) : ''}.log`);
  }

   function focusSearch(): void {
    searchEl?.focus();
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`logs`,});
   { svelteHTML.createElement("div", { "class":`logs-bar`,});
    if(multi){
       { svelteHTML.createElement("select", {       "class":`input sm`,"bind:value":podFilter,"aria-label":`Pod`,"title":`Show one pod’s lines`,});/*Ωignore_startΩ*/() => podFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",}); pods.length ? ` (${pods.length})` : ''; }
           for(let p of __sveltets_2_ensureArray(pods)){p; { svelteHTML.createElement("option", { "value":p,});p; }}
       }
      if(podFilter && onopenpod){
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => onopenpod?.(podFilter),"title":`Open this pod’s details`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}   }
      }
    }
    if(containers.length > 1 || (multi && containers.length)){
       { svelteHTML.createElement("select", {     "class":`input sm`,"bind:value":container,"aria-label":`Container`,});/*Ωignore_startΩ*/() => container = __sveltets_2_any(null);/*Ωignore_endΩ*/
        if(multi){ { svelteHTML.createElement("option", {"value":"",});  }}
           for(let c of __sveltets_2_ensureArray(containers)){c.name; { svelteHTML.createElement("option", { "value":c.name,});c.init ? `init: ${c.name}` : c.name; }}
       }
    }
     { svelteHTML.createElement("select", {       "class":`input sm`,"bind:value":tail,"aria-label":`Tail lines`,"title":`Tail`,});/*Ωignore_startΩ*/() => tail = __sveltets_2_any(null);/*Ωignore_endΩ*/
         for(let t of __sveltets_2_ensureArray(TAILS)){t; { svelteHTML.createElement("option", { "value":t,}); t; }}
     }
     { svelteHTML.createElement("select", {       "class":`input sm`,"bind:value":since,"aria-label":`Since`,"title":`Since`,});/*Ωignore_startΩ*/() => since = __sveltets_2_any(null);/*Ωignore_endΩ*/
         for(let s of __sveltets_2_ensureArray(SINCES)){s.v; { svelteHTML.createElement("option", { "value":s.v,}); s.l; }}
     }
     { svelteHTML.createElement("button", {        "class":`pill-toggle`,"onclick":() => (follow = !follow),"aria-pressed":follow,"title":`Stream new lines (kubectl logs -f)`,});follow;
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});} 
     }
     { svelteHTML.createElement("button", {      "class":`pill-toggle`,"onclick":() => (timestamps = !timestamps),"aria-pressed":timestamps,});timestamps;
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});} 
     }
     { svelteHTML.createElement("button", {        "class":`pill-toggle`,"onclick":() => (previous = !previous),"aria-pressed":previous,"title":`Logs of the previous (crashed) container instance`,});previous;
      
     }
     { svelteHTML.createElement("span", { "class":`spacer`,}); }
     { svelteHTML.createElement("div", { "class":`search`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
       { const $$_input3 = svelteHTML.createElement("input", {           "dir":`auto`,"class":`search-in`,"placeholder":`Search…`,"bind:value":search,"aria-label":`Search logs`,});searchEl = $$_input3;/*Ωignore_startΩ*/() => search = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(q){ { svelteHTML.createElement("span", { "class":`count mono`,});matchCount; }}
     }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void start(),"title":`Refresh logs`,"aria-label":`Refresh logs`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});} }
     { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":download,"title":`Download`,"aria-label":`Download logs`,"disabled":!lineCount,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":13,}});} }
   }

  if(error){
     { svelteHTML.createElement("div", { "class":`err`,});error;  { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void start(),});  } }
  }

   { const $$_div1 = svelteHTML.createElement("div", {      "class":`logs-body`,"dir":`ltr`,"onscrollcapture":onScroll,});wrapEl = $$_div1;
    if(!lineCount && !error){
       { svelteHTML.createElement("div", { "class":`dim pad`,});streaming ? 'Waiting for output…' : 'No log lines.'; }
    }else{
       { const $$_tsiLlautriV2C = __sveltets_2_ensureComponent(VirtualList); const $$_tsiLlautriV2 = new $$_tsiLlautriV2C({ target: __sveltets_2_any(), props: {         "items":shown,"version":ver,"estimateHeight":LINE_H,"class":`logs-vlist`,"findText":(line) => parse(line).text,row:(line, i) => { async ()/*Ωignore_positionΩ*/ => {
          const pl = parse(line);
           { svelteHTML.createElement("div", {     "class":`ln`,"style":`height:${LINE_H}px`,...__sveltets_2_empty({"data-i":i}),});if(pl.pod){ { svelteHTML.createElement("button", {       "class":`podtag`,...__sveltets_2_empty({"data-find-skip":true}),"style":`--h:${hue(pl.pod)}`,"title":`${pl.pod} · ${pl.ctr}\nClick: only this pod · ⌥-click: open pod`,"onclick":(e) => { if (e.altKey) onopenpod?.(pl.pod); else podFilter = podFilter === pl.pod ? '' : pl.pod; },});pl.pod.length > 22 ? '…' + pl.pod.slice(-21) : pl.pod;if(!container){ { svelteHTML.createElement("span", { "class":`ctr`,}); pl.ctr; }} }}    for(let s of __sveltets_2_ensureArray(segments(pl.text))){let k = 1;k;if(s.m){ { svelteHTML.createElement("mark", {});s.t; }}else{s.t;}} }
        };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {row} = $$_tsiLlautriV2.$$prop_def;/*Ωignore_endΩ*/
        
       VirtualList}
    }
   }

   { svelteHTML.createElement("div", { "class":`logs-foot`,});
     { svelteHTML.createElement("span", { "class":`dim`,});lineCount;lineCount >= MAX_LINES ? '+' : ''; multi && pods.length ? ` · ${pods.length} pods` : '';podFilter ? ` · ${(void ver, shown.length)} from ${podFilter}` : '';q ? ` · ${matchCount} match` : ''; }
    if(streaming && follow){ { svelteHTML.createElement("span", { "class":`live`,}); { svelteHTML.createElement("span", { "class":`live-dot`,}); }  }}
    if(!autoScroll && follow){
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => { autoScroll = true; },});   }
    }
   }
 }


};
return { props: {} as any as Props, exports: {} as any as { focusSearch: typeof focusSearch }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LogsView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LogsView__SvelteComponent_ = ReturnType<typeof LogsView__SvelteComponent_>;
/*Ωignore_endΩ*/export default LogsView__SvelteComponent_;