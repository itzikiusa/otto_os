<script lang="ts">
  // Drawer "Logs" tab: container selector, tail / since / previous, follow
  // (streams `kubectl logs -f` over a fetch ReadableStream, aborted on any
  // option change or unmount), timestamps, in-buffer search with highlights,
  // download. Lines are windowed with `VirtualList` and capped so a chatty
  // pod can't grow the DOM or memory without bound.
  import { untrack, tick } from 'svelte';
  import VirtualList from '../../lib/components/VirtualList.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { downloadText } from '../../lib/components/exporters';
  import { followLogs } from '../../lib/api/k8s';
  import type { K8sContainer } from '../../lib/api/types';
  import { safeName } from './k8s-util';

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
  }
  let { clusterId, ns, pod = '', selector = '', title = '', containers, onopenpod }: Props = $props();

  const multi = $derived(!pod && !!selector);
  /** `[pod/<pod>/<container>] rest` → parts (workload mode only). */
  const PREFIX = /^\[pod\/([^/\]]+)\/([^\]]+)\] ?(.*)$/;
  interface Parsed { pod: string; ctr: string; text: string }
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
  let lines: string[] = $state([]);
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

  function flush(): void {
    flushRaf = null;
    if (!pending.length) return;
    const add = pending;
    pending = [];
    const next = lines.length + add.length > MAX_LINES ? [...lines, ...add].slice(-MAX_LINES) : [...lines, ...add];
    lines = next;
  }

  function ingest(text: string): void {
    const parts = (carry + text).split('\n');
    carry = parts.pop() ?? '';
    if (parts.length) pending.push(...parts);
    if (flushRaf === null) flushRaf = requestAnimationFrame(flush);
  }

  async function start(): Promise<void> {
    abort?.abort();
    const ac = new AbortController();
    abort = ac;
    lines = [];
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
      if (carry) {
        pending.push(carry);
        carry = '';
      }
      flush();
    } catch (e) {
      if (!ac.signal.aborted) error = e instanceof Error ? e.message : String(e);
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
    };
  });

  const q = $derived(search.trim().toLowerCase());
  /** Pods seen in the buffer (workload mode) — the pod filter's options. */
  const pods = $derived.by(() => {
    if (!multi) return [] as string[];
    const set = new Set<string>();
    for (const l of lines) {
      const m = PREFIX.exec(l);
      if (m) set.add(m[1]);
    }
    return [...set].sort();
  });
  const shown = $derived.by(() => {
    let out = lines;
    if (multi && podFilter) out = out.filter((l) => l.startsWith(`[pod/${podFilter}/`));
    if (q) out = out.filter((l) => l.toLowerCase().includes(q));
    return out;
  });
  const matchCount = $derived(q ? shown.length : 0);

  // Stick to the bottom while following (unless the user scrolled up).
  $effect(() => {
    void shown.length;
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

  export function focusSearch(): void {
    searchEl?.focus();
  }
</script>

<div class="logs">
  <div class="logs-bar">
    {#if multi}
      <select class="input sm" bind:value={podFilter} aria-label="Pod" title="Show one pod's lines">
        <option value="">all pods{pods.length ? ` (${pods.length})` : ''}</option>
        {#each pods as p (p)}<option value={p}>{p}</option>{/each}
      </select>
      {#if podFilter && onopenpod}
        <button class="btn small" onclick={() => onopenpod?.(podFilter)} title="Open this pod's details"><Icon name="chevronRight" size={11} /> Open pod</button>
      {/if}
    {/if}
    {#if containers.length > 1 || (multi && containers.length)}
      <select class="input sm" bind:value={container} aria-label="Container">
        {#if multi}<option value="">all containers</option>{/if}
        {#each containers as c (c.name)}<option value={c.name}>{c.init ? `init: ${c.name}` : c.name}</option>{/each}
      </select>
    {/if}
    <select class="input sm" bind:value={tail} aria-label="Tail lines" title="Tail">
      {#each TAILS as t (t)}<option value={t}>tail {t}</option>{/each}
    </select>
    <select class="input sm" bind:value={since} aria-label="Since" title="Since">
      {#each SINCES as s (s.v)}<option value={s.v}>since {s.l}</option>{/each}
    </select>
    <button class="pill-toggle" class:on={follow} onclick={() => (follow = !follow)} aria-pressed={follow} title="Stream new lines (kubectl logs -f)">
      <Icon name="play" size={11} /> Follow
    </button>
    <button class="pill-toggle" class:on={timestamps} onclick={() => (timestamps = !timestamps)} aria-pressed={timestamps}>
      <Icon name="clock" size={11} /> Timestamps
    </button>
    <button class="pill-toggle" class:on={previous} onclick={() => (previous = !previous)} aria-pressed={previous} title="Logs of the previous (crashed) container instance">
      Previous
    </button>
    <span class="spacer"></span>
    <div class="search">
      <Icon name="search" size={12} />
      <input bind:this={searchEl} class="search-in" placeholder="Search…" bind:value={search} aria-label="Search logs" />
      {#if q}<span class="count mono">{matchCount}</span>{/if}
    </div>
    <button class="icon-btn" onclick={() => void start()} title="Reload" aria-label="Reload logs"><Icon name="refresh" size={13} /></button>
    <button class="icon-btn" onclick={download} title="Download" aria-label="Download logs" disabled={!lines.length}><Icon name="arrowDown" size={13} /></button>
  </div>

  {#if error}
    <div class="err">{error}</div>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="logs-body" bind:this={wrapEl} onscrollcapture={onScroll}>
    {#if !lines.length && !error}
      <div class="dim pad">{streaming ? 'Waiting for output…' : 'No log lines.'}</div>
    {:else}
      <VirtualList items={shown} estimateHeight={LINE_H} class="logs-vlist">
        {#snippet row(line, i)}
          {@const pl = parse(line)}
          <div class="ln" style="height:{LINE_H}px" data-i={i}>{#if pl.pod}<button class="podtag" style="--h:{hue(pl.pod)}" title="{pl.pod} · {pl.ctr}\nClick: only this pod · ⌥-click: open pod" onclick={(e) => { if (e.altKey) onopenpod?.(pl.pod); else podFilter = podFilter === pl.pod ? '' : pl.pod; }}>{pl.pod.length > 22 ? '…' + pl.pod.slice(-21) : pl.pod}{#if !container}<span class="ctr">/{pl.ctr}</span>{/if}</button>{/if}{#each segments(pl.text) as s, k (k)}{#if s.m}<mark>{s.t}</mark>{:else}{s.t}{/if}{/each}</div>
        {/snippet}
      </VirtualList>
    {/if}
  </div>

  <div class="logs-foot">
    <span class="dim">{lines.length}{lines.length >= MAX_LINES ? '+' : ''} lines{multi && pods.length ? ` · ${pods.length} pods` : ''}{podFilter ? ` · ${shown.length} from ${podFilter}` : ''}{q ? ` · ${matchCount} match` : ''}</span>
    {#if streaming && follow}<span class="live"><span class="live-dot"></span> live</span>{/if}
    {#if !autoScroll && follow}
      <button class="btn small" onclick={() => { autoScroll = true; }}>Jump to bottom</button>
    {/if}
  </div>
</div>

<style>
  .logs {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .logs-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .input.sm {
    height: 24px;
    font-size: 11.5px;
    padding: 0 6px;
  }
  .spacer {
    flex: 1;
  }
  .search {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .search-in {
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 12px;
    width: 140px;
    outline: none;
  }
  .count {
    font-size: 11px;
  }
  .logs-body {
    flex: 1;
    min-height: 0;
    background: color-mix(in srgb, var(--surface-2) 60%, black 8%);
  }
  .logs-body :global(.logs-vlist) {
    height: 100%;
  }
  .ln {
    font-family: var(--font-mono);
    font-size: 11.5px;
    line-height: 18px;
    white-space: pre;
    padding: 0 10px;
    color: var(--text);
  }
  .podtag {
    display: inline-block;
    margin-right: 8px;
    padding: 0 6px;
    border: none;
    border-radius: 3px;
    background: hsl(var(--h) 50% 50% / 0.22);
    color: hsl(var(--h) 70% 72%);
    font: inherit;
    font-size: 10.5px;
    line-height: 16px;
    cursor: pointer;
    vertical-align: middle;
  }
  .podtag:hover {
    background: hsl(var(--h) 50% 50% / 0.38);
  }
  .podtag .ctr {
    opacity: 0.7;
  }
  .ln mark {
    background: color-mix(in srgb, var(--accent) 45%, transparent);
    color: inherit;
    border-radius: 2px;
  }
  .logs-foot {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 3px 10px;
    border-top: 1px solid var(--border);
    font-size: 11px;
  }
  .live {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--status-working);
  }
  .live-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--status-working);
    animation: blink 1s ease-in-out infinite;
  }
  @keyframes blink {
    50% {
      opacity: 0.3;
    }
  }
  .err {
    padding: 8px 10px;
    color: var(--status-exited);
    font-size: 12px;
    white-space: pre-wrap;
  }
  .dim {
    color: var(--text-dim);
  }
  .pad {
    padding: 14px;
    font-size: 12.5px;
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
