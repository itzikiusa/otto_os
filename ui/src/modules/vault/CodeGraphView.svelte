<script lang="ts">
  // Graph tab — the headline Obsidian-style unified graph (knowledge + code).
  // Left: a collapsible tree of vault content by group → bucket → node.
  // Centre: a dependency-free force-directed SVG canvas (drag / zoom / pan /
  // hover / click-to-focus). Right: Obsidian-like control panels (Filters,
  // Display, Forces, Groups). Data: GET /vault/fullgraph (optionally per-repo).
  import { onMount, onDestroy } from 'svelte';
  import { vault } from './vault.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ForceSim, DEFAULT_PARAMS } from './force';
  import type { FullGraphNode, FullGraphEdge } from '../../lib/api/types';

  // ── simulation + render plumbing ────────────────────────────────────────
  const sim = new ForceSim();
  let svgEl = $state<SVGSVGElement | undefined>(undefined);
  let frame = $state(0); // bumped each tick to drive position re-reads
  let raf = 0;

  // view transform (pan/zoom). 1 unit = 1px; no viewBox.
  let view = $state({ x: 0, y: 0, k: 1 });

  // interaction
  let dragId: string | null = null;
  let panning = false;
  let last = { x: 0, y: 0 };
  let moved = false;

  // hover + focus highlight
  let hoverNode = $state<FullGraphNode | null>(null);
  let hoverXY = $state({ x: 0, y: 0 });
  let hoverEdge = $state<{ rel: string; detail?: string; x: number; y: number } | null>(null);
  let highlightId = $state<string | null>(null);

  // ── panel collapse state ────────────────────────────────────────────────
  let showTree = $state(true);
  let showFilters = $state(true);
  let showDisplay = $state(true);
  let showForces = $state(true);
  let showGroups = $state(true);
  let treeOpen = $state<Record<string, boolean>>({});

  // ── Filters ─────────────────────────────────────────────────────────────
  let textFilter = $state('');
  let showKnowledge = $state(true);
  let showCode = $state(true);
  let hideOrphans = $state(false);
  let kindOff = $state<Record<string, boolean>>({});

  // ── Display ─────────────────────────────────────────────────────────────
  let showLabels = $state(true);
  let sizeByDegree = $state(true);
  let linkThickness = $state(1);
  let showArrows = $state(true);

  // ── Forces (feed the simulation) ─────────────────────────────────────────
  let centerStrength = $state(DEFAULT_PARAMS.centerStrength);
  let repel = $state(DEFAULT_PARAMS.repel);
  let linkDistance = $state(DEFAULT_PARAMS.linkDistance);

  // ── Groups (colour a query match) ────────────────────────────────────────
  let groupQuery = $state('');
  let groupColor = $state('#ffd43b');
  let groupApplied = $state('');

  // ── data ────────────────────────────────────────────────────────────────
  const nodes = $derived(vault.fullGraph?.nodes ?? []);
  const edges = $derived(vault.fullGraph?.edges ?? []);

  /** Distinct kinds present (for the by-kind toggles), with their group. */
  const kinds = $derived.by(() => {
    const seen = new Map<string, string>();
    for (const n of nodes) if (!seen.has(n.kind)) seen.set(n.kind, n.group);
    return [...seen.entries()].map(([kind, group]) => ({ kind, group })).sort((a, b) => a.kind.localeCompare(b.kind));
  });

  /** Degree across ALL edges (for orphan detection — independent of filters). */
  const fullDegree = $derived.by(() => {
    const m = new Map<string, number>();
    for (const e of edges) {
      m.set(e.src, (m.get(e.src) ?? 0) + 1);
      m.set(e.dst, (m.get(e.dst) ?? 0) + 1);
    }
    return m;
  });

  const filteredNodes = $derived.by(() => {
    const q = textFilter.trim().toLowerCase();
    return nodes.filter((n) => {
      if (n.group === 'knowledge' && !showKnowledge) return false;
      if (n.group === 'code' && !showCode) return false;
      if (kindOff[n.kind]) return false;
      if (hideOrphans && (fullDegree.get(n.id) ?? 0) === 0) return false;
      if (q) {
        const hay = `${n.label} ${n.kind} ${n.file ?? ''}`.toLowerCase();
        if (!hay.includes(q)) return false;
      }
      return true;
    });
  });

  const visibleIds = $derived(new Set(filteredNodes.map((n) => n.id)));

  const filteredEdges = $derived.by(() => {
    return edges.filter((e) => visibleIds.has(e.src) && visibleIds.has(e.dst));
  });

  /** Edge degree by node id, from the *filtered* edge set (drives node size). */
  const degreeById = $derived.by(() => {
    const m = new Map<string, number>();
    for (const e of filteredEdges) {
      m.set(e.src, (m.get(e.src) ?? 0) + 1);
      m.set(e.dst, (m.get(e.dst) ?? 0) + 1);
    }
    return m;
  });

  /** Group-match set: text/kind/file match OR a semantic hit (knowledge nodes). */
  const groupMatched = $derived.by(() => {
    const set = new Set<string>();
    const q = groupApplied.trim().toLowerCase();
    if (!q) return set;
    for (const n of filteredNodes) {
      const hay = `${n.label} ${n.kind} ${n.file ?? ''}`.toLowerCase();
      if (hay.includes(q) || vault.hitsById.has(n.id)) set.add(n.id);
    }
    return set;
  });

  // frame-driven position map (sim mutates plain objects; `frame` forces re-read)
  const positions = $derived.by(() => {
    frame;
    const m = new Map<string, { x: number; y: number }>();
    for (const nd of sim.nodes) m.set(nd.id, { x: nd.x, y: nd.y });
    return m;
  });

  // ── left tree ────────────────────────────────────────────────────────────
  interface TreeBucket { key: string; label: string; leaves: FullGraphNode[] }
  interface TreeSection { group: string; label: string; count: number; buckets: TreeBucket[] }

  const tree = $derived.by((): TreeSection[] => {
    const code = new Map<string, FullGraphNode[]>();
    const know = new Map<string, FullGraphNode[]>();
    for (const n of filteredNodes) {
      if (n.group === 'code') {
        const key = n.file ? `f:${n.file}` : `k:${n.kind}`;
        (code.get(key) ?? code.set(key, []).get(key)!).push(n);
      } else {
        const key = `k:${n.kind}`;
        (know.get(key) ?? know.set(key, []).get(key)!).push(n);
      }
    }
    const toBuckets = (m: Map<string, FullGraphNode[]>): TreeBucket[] =>
      [...m.entries()]
        .map(([key, leaves]) => ({
          key,
          label: key.startsWith('f:') ? basename(key.slice(2)) : key.slice(2),
          leaves: leaves.slice().sort((a, b) => a.label.localeCompare(b.label)),
        }))
        .sort((a, b) => a.label.localeCompare(b.label));
    const sections: TreeSection[] = [];
    if (showCode && code.size) {
      const buckets = toBuckets(code);
      sections.push({ group: 'code', label: 'Code', count: buckets.reduce((s, b) => s + b.leaves.length, 0), buckets });
    }
    if (showKnowledge && know.size) {
      const buckets = toBuckets(know);
      sections.push({ group: 'knowledge', label: 'Knowledge', count: buckets.reduce((s, b) => s + b.leaves.length, 0), buckets });
    }
    return sections;
  });

  function basename(p: string): string {
    const parts = p.split('/');
    return parts[parts.length - 1] || p;
  }

  // ── colours / styles ──────────────────────────────────────────────────────
  function knowledgeColor(kind: string): string {
    switch (kind) {
      case 'entity': return '#6ea8fe';
      case 'decision': return '#63e6be';
      case 'constraint':
      case 'requirement': return '#ffa94d';
      case 'qa': return '#da77f2';
      case 'chunk': return '#adb5bd';
      default: return '#74c0fc';
    }
  }
  function nodeColor(n: FullGraphNode): string {
    if (n.group === 'knowledge') return knowledgeColor(n.kind);
    switch (n.kind) {
      case 'file': return '#6ea8fe';
      case 'symbol': return '#a5d8ff';
      case 'service': return '#63e6be';
      case 'db_table': return '#ffa94d';
      case 'endpoint': return '#ffd43b';
      case 'doc': return '#da77f2';
      case 'external': return '#adb5bd';
      default: return '#74c0fc';
    }
  }
  interface EStyle { stroke: string; dash?: string; marker: string }
  function edgeStyle(rel: string): EStyle {
    switch (rel) {
      case 'http_call': return { stroke: '#ffa94d', dash: '5 4', marker: 'orange' };
      case 'db_call': return { stroke: '#38d9a9', dash: '5 4', marker: 'teal' };
      case 'documents': return { stroke: '#da77f2', dash: '1 4', marker: 'pink' };
      case 'imports': return { stroke: '#6ea8fe', marker: 'blue' };
      case 'test_of': return { stroke: '#69db7c', dash: '4 3', marker: 'green' };
      case 'depends_on': return { stroke: '#9775fa', dash: '6 3', marker: 'purple' };
      case 'defined_in': return { stroke: '#868e96', marker: 'grey' };
      case 'calls':
      default: return { stroke: '#868e96', marker: 'grey' };
    }
  }
  const REL_LEGEND: Array<{ rel: string; label: string }> = [
    { rel: 'calls', label: 'calls' },
    { rel: 'imports', label: 'imports' },
    { rel: 'http_call', label: 'http' },
    { rel: 'db_call', label: 'db' },
    { rel: 'test_of', label: 'test' },
    { rel: 'documents', label: 'docs' },
    { rel: 'depends_on', label: 'depends' },
  ];

  function radiusOf(id: string): number {
    if (!sizeByDegree) return 6;
    return 5 + Math.min(degreeById.get(id) ?? 0, 14) * 0.9;
  }

  // ── geometry helpers (read `positions`, so they recompute each frame) ──────
  function nodePos(id: string): { x: number; y: number } | undefined {
    return positions.get(id);
  }
  function edgeGeom(e: FullGraphEdge) {
    const a = positions.get(e.src);
    const b = positions.get(e.dst);
    if (!a || !b) return null;
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    const d = Math.hypot(dx, dy) || 1;
    const back = radiusOf(e.dst) + (showArrows ? 6 : 1);
    return {
      x1: a.x,
      y1: a.y,
      x2: b.x - (dx / d) * back,
      y2: b.y - (dy / d) * back,
      mx: (a.x + b.x) / 2,
      my: (a.y + b.y) / 2,
    };
  }

  // ── animation loop ────────────────────────────────────────────────────────
  // The SVG re-renders on every `frame` bump, so we THROTTLE the visual update
  // (~25fps) and HARD-CAP the run (~8s) — the sim still steps each rAF, but the
  // expensive DOM diff of hundreds of nodes can't peg the main thread, and the
  // loop is guaranteed to terminate (fixes the "stuck after revisiting" freeze).
  let destroyed = false;
  let lastRender = 0;
  let runStart = 0;
  const MAX_RUN_MS = 8000;
  function loop() {
    if (destroyed) {
      raf = 0;
      return;
    }
    const now = performance.now();
    const overtime = now - runStart > MAX_RUN_MS;
    const alive = sim.step() && !overtime;
    if (now - lastRender > 40 || !alive || dragId || panning) {
      frame++;
      lastRender = now;
    }
    if (alive || dragId || panning) raf = requestAnimationFrame(loop);
    else raf = 0;
  }
  function kick() {
    if (destroyed) return;
    sim.reheat();
    runStart = performance.now();
    if (!raf) raf = requestAnimationFrame(loop);
  }

  // Compute the layout SYNCHRONOUSLY (bounded) and render exactly ONCE. This is
  // the key fix for the "graph freezes on revisit": the old continuous rAF loop
  // re-rendered hundreds of SVG nodes every frame, pegging the main thread. The
  // rAF loop now runs ONLY during an active drag/pan (short, user-driven).
  function settle() {
    if (destroyed) return;
    sim.reheat();
    let i = 0;
    while (sim.step() && i++ < 600) {
      // advance physics on plain objects; no DOM work in the loop
    }
    // Render ONCE, but bump `frame` on the NEXT frame — NOT synchronously inside
    // the calling $effect (that retriggers the effect → effect_update_depth_exceeded).
    if (!raf) raf = requestAnimationFrame(() => {
      raf = 0;
      if (!destroyed) frame++;
    });
  }

  // rebuild + lay out the sim whenever the visible graph changes
  $effect(() => {
    const fn = filteredNodes;
    const fe = filteredEdges.map((e) => ({ source: e.src, target: e.dst }));
    sim.setGraph(fn, fe);
    settle();
  });

  // push force params into the sim (re-layout once)
  $effect(() => {
    sim.params = {
      centerStrength,
      repel,
      linkDistance,
      linkStrength: DEFAULT_PARAMS.linkStrength,
    };
    settle();
  });

  // ── coordinate conversion ─────────────────────────────────────────────────
  function screenToGraph(clientX: number, clientY: number): { x: number; y: number } {
    const rect = svgEl?.getBoundingClientRect();
    if (!rect) return { x: 0, y: 0 };
    return {
      x: (clientX - rect.left - view.x) / view.k,
      y: (clientY - rect.top - view.y) / view.k,
    };
  }

  // ── pointer handlers ──────────────────────────────────────────────────────
  function onNodeDown(e: PointerEvent, id: string) {
    e.stopPropagation();
    dragId = id;
    moved = false;
    const nd = sim.byId.get(id);
    if (nd) {
      nd.fx = nd.x;
      nd.fy = nd.y;
    }
    svgEl?.setPointerCapture(e.pointerId);
    kick();
  }
  function onBgDown(e: PointerEvent) {
    panning = true;
    moved = false;
    last = { x: e.clientX, y: e.clientY };
    svgEl?.setPointerCapture(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    if (dragId) {
      const g = screenToGraph(e.clientX, e.clientY);
      const nd = sim.byId.get(dragId);
      if (nd) {
        nd.fx = g.x;
        nd.fy = g.y;
      }
      moved = true;
      kick();
    } else if (panning) {
      view.x += e.clientX - last.x;
      view.y += e.clientY - last.y;
      last = { x: e.clientX, y: e.clientY };
      moved = true;
    }
  }
  function onUp(e: PointerEvent) {
    if (dragId) {
      const nd = sim.byId.get(dragId);
      if (nd) {
        nd.fx = null;
        nd.fy = null;
      }
      dragId = null;
      kick();
    }
    panning = false;
    svgEl?.releasePointerCapture?.(e.pointerId);
  }
  function onWheel(e: WheelEvent) {
    e.preventDefault();
    const rect = svgEl?.getBoundingClientRect();
    if (!rect) return;
    const factor = e.deltaY < 0 ? 1.12 : 1 / 1.12;
    const newK = Math.min(4, Math.max(0.2, view.k * factor));
    const px = e.clientX - rect.left;
    const py = e.clientY - rect.top;
    view.x = px - ((px - view.x) / view.k) * newK;
    view.y = py - ((py - view.y) / view.k) * newK;
    view.k = newK;
  }

  function onNodeClick(e: MouseEvent, n: FullGraphNode) {
    e.stopPropagation();
    if (moved) return; // a drag, not a click
    focusNode(n.id);
  }

  function focusNode(id: string) {
    const nd = sim.byId.get(id);
    const rect = svgEl?.getBoundingClientRect();
    if (!nd || !rect) return;
    view.x = rect.width / 2 - nd.x * view.k;
    view.y = rect.height / 2 - nd.y * view.k;
    highlightId = id;
    setTimeout(() => {
      if (highlightId === id) highlightId = null;
    }, 2200);
  }

  function resetView() {
    view = { x: 0, y: 0, k: 1 };
    settle();
  }

  // ── hover ─────────────────────────────────────────────────────────────────
  function onNodeEnter(n: FullGraphNode) {
    const p = positions.get(n.id);
    if (p) hoverXY = { x: p.x * view.k + view.x, y: p.y * view.k + view.y };
    hoverNode = n;
  }
  function onEdgeEnter(e: FullGraphEdge) {
    const g = edgeGeom(e);
    if (g) hoverEdge = { rel: e.rel, detail: e.detail, x: g.mx * view.k + view.x, y: g.my * view.k + view.y };
  }

  // ── groups ──────────────────────────────────────────────────────────────
  function applyGroup() {
    groupApplied = groupQuery;
    // Pull semantic reasons in too: run a memory search so knowledge nodes that
    // match by meaning (not just text) join the group and gain "why" tooltips.
    if (groupQuery.trim()) {
      vault.query = groupQuery;
      void vault.search();
    }
  }
  function clearGroup() {
    groupApplied = '';
    groupQuery = '';
  }

  function toggleKind(k: string) {
    kindOff = { ...kindOff, [k]: !kindOff[k] };
  }
  function toggleTree(key: string) {
    treeOpen = { ...treeOpen, [key]: !treeOpen[key] };
  }

  // node fill: group colour, overridden by the group-match highlight colour
  function fillFor(n: FullGraphNode): string {
    return groupMatched.has(n.id) ? groupColor : nodeColor(n);
  }

  // ── lifecycle ─────────────────────────────────────────────────────────────
  let ro: ResizeObserver | undefined;
  // Load the graph + repos once the workspace is available. (A one-shot onMount
  // raced workspace selection — if ws.currentId wasn't set yet the graph loaded
  // nothing and never retried.) The loads run in a microtask so they don't mutate
  // store state synchronously inside the effect (which Svelte forbids and which
  // broke the component's reactivity).
  $effect(() => {
    const id = ws.currentId;
    if (!id) return;
    queueMicrotask(() => {
      if (!vault.repos.length) void vault.loadRepos();
      if (!vault.fullGraph && !vault.fullGraphLoading) void vault.loadFullGraph();
    });
  });
  onMount(() => {
    const measure = () => {
      const r = svgEl?.getBoundingClientRect();
      if (r && r.width > 0) sim.resize(r.width, r.height);
    };
    measure();
    if (svgEl && 'ResizeObserver' in window) {
      ro = new ResizeObserver(measure);
      ro.observe(svgEl);
    }
    // Layout is driven by the setGraph $effect (synchronous settle); no loop here.
  });
  onDestroy(() => {
    destroyed = true;
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
    ro?.disconnect();
  });

  // hover info for the open tooltip's reason chips (knowledge hits only)
  const hoverHit = $derived(hoverNode ? vault.hitsById.get(hoverNode.id) : undefined);
</script>

<div class="cg" class:no-tree={!showTree}>
  <!-- ── Left: content tree ─────────────────────────────────────────── -->
  {#if showTree}
    <aside class="cg-tree">
      <div class="cg-tree-head">
        <span>Content</span>
        <button class="icon-btn" title="Hide tree" onclick={() => (showTree = false)} aria-label="Hide tree">
          <Icon name="chevronLeft" size={13} />
        </button>
      </div>
      <div class="cg-tree-body">
        {#each tree as sec (sec.group)}
          <div class="tree-section">
            <button class="tree-sec-head" onclick={() => toggleTree(sec.group)}>
              <Icon name={treeOpen[sec.group] === false ? 'chevronRight' : 'chevronDown'} size={12} />
              <span class="dot" style:background={sec.group === 'code' ? '#6ea8fe' : '#63e6be'}></span>
              {sec.label}
              <span class="muted">{sec.count}</span>
            </button>
            {#if treeOpen[sec.group] !== false}
              {#each sec.buckets as b (b.key)}
                <div class="tree-bucket">
                  <button class="tree-bucket-head" onclick={() => toggleTree(b.key)} title={b.label}>
                    <Icon name={treeOpen[b.key] ? 'chevronDown' : 'chevronRight'} size={11} />
                    <Icon name={b.key.startsWith('f:') ? 'file' : 'box'} size={11} />
                    <span class="bucket-label">{b.label}</span>
                    <span class="muted">{b.leaves.length}</span>
                  </button>
                  {#if treeOpen[b.key]}
                    <ul class="tree-leaves">
                      {#each b.leaves as leaf (leaf.id)}
                        <li>
                          <button
                            class="tree-leaf"
                            class:hl={highlightId === leaf.id}
                            onclick={() => focusNode(leaf.id)}
                            title={leaf.label}
                          >
                            <span class="leaf-dot" style:background={nodeColor(leaf)}></span>
                            <span class="leaf-label">{leaf.label}</span>
                          </button>
                        </li>
                      {/each}
                    </ul>
                  {/if}
                </div>
              {/each}
            {/if}
          </div>
        {:else}
          <p class="tree-empty">No nodes — index a repo or adjust filters.</p>
        {/each}
      </div>
    </aside>
  {/if}

  <!-- ── Centre: graph canvas ────────────────────────────────────────── -->
  <div class="cg-canvas">
    <!-- top toolbar -->
    <div class="cg-toolbar">
      {#if !showTree}
        <button class="icon-btn" title="Show tree" onclick={() => (showTree = true)} aria-label="Show tree">
          <Icon name="sidebar" size={14} />
        </button>
      {/if}
      <select
        class="repo-select"
        bind:value={vault.graphRepoId}
        onchange={() => vault.loadFullGraph()}
        title="Scope the graph to a repository"
        aria-label="Repository scope"
      >
        <option value="">All repos</option>
        {#each vault.repos as r (r.id)}
          <option value={r.id}>{r.name}</option>
        {/each}
      </select>
      <button class="tb-btn" onclick={() => vault.loadFullGraph()} title="Reload graph">
        <Icon name="refresh" size={13} /> Reload
      </button>
      <button class="tb-btn" onclick={resetView} title="Reset zoom/pan">
        <Icon name="maximize" size={13} /> Fit
      </button>
      <span class="cg-stats">
        {filteredNodes.length}/{nodes.length} nodes · {filteredEdges.length}/{edges.length} links
      </span>
      {#if !showFilters || !showDisplay || !showForces || !showGroups}
        <button class="tb-btn ml" onclick={() => { showFilters = showDisplay = showForces = showGroups = true; }}>
          <Icon name="panel" size={13} /> Panels
        </button>
      {/if}
    </div>

    {#if vault.fullGraphLoading}
      <div class="cg-overlay">Loading graph…</div>
    {:else if nodes.length === 0}
      <div class="cg-overlay">
        <Icon name="branch" size={20} />
        <p>No graph yet.</p>
        <p class="muted">Index a repo in the <b>Repos</b> tab to populate the code graph.</p>
      </div>
    {/if}

    <svg
      bind:this={svgEl}
      class="cg-svg"
      width="100%"
      height="100%"
      role="application"
      aria-label="Vault graph"
      onpointerdown={onBgDown}
      onpointermove={onMove}
      onpointerup={onUp}
      onpointerleave={onUp}
      onwheel={onWheel}
    >
      <defs>
        {#each [['grey', '#868e96'], ['blue', '#6ea8fe'], ['orange', '#ffa94d'], ['teal', '#38d9a9'], ['pink', '#da77f2'], ['green', '#69db7c'], ['purple', '#9775fa']] as [id, col] (id)}
          <marker id={`vg-arrow-${id}`} viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
            <path d="M0 0 L8 4 L0 8 z" fill={col} />
          </marker>
        {/each}
      </defs>

      <g transform={`translate(${view.x} ${view.y}) scale(${view.k})`}>
        <!-- edges -->
        <g class="edges">
          {#each filteredEdges as e (e.src + e.dst + e.rel)}
            {@const g = edgeGeom(e)}
            {@const st = edgeStyle(e.rel)}
            {#if g}
              <line
                x1={g.x1}
                y1={g.y1}
                x2={g.x2}
                y2={g.y2}
                stroke={st.stroke}
                stroke-opacity="0.5"
                stroke-width={linkThickness}
                stroke-dasharray={st.dash}
                marker-end={showArrows ? `url(#vg-arrow-${st.marker})` : undefined}
              />
              <!-- wide invisible hit line for hover (decorative — info is in the panels/tree) -->
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <line
                class="edge-hit"
                aria-hidden="true"
                x1={g.x1}
                y1={g.y1}
                x2={g.x2}
                y2={g.y2}
                onpointerenter={() => onEdgeEnter(e)}
                onpointerleave={() => (hoverEdge = null)}
              />
            {/if}
          {/each}
        </g>

        <!-- nodes -->
        <g class="nodes">
          {#each filteredNodes as n (n.id)}
            {@const p = nodePos(n.id)}
            {#if p}
              {@const r = radiusOf(n.id)}
              <g
                class="cg-node"
                class:hl={highlightId === n.id}
                class:matched={groupMatched.has(n.id)}
                transform={`translate(${p.x} ${p.y})`}
                onpointerdown={(e) => onNodeDown(e, n.id)}
                onclick={(e) => onNodeClick(e, n)}
                onkeydown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    focusNode(n.id);
                  }
                }}
                onpointerenter={() => onNodeEnter(n)}
                onpointerleave={() => (hoverNode = null)}
                role="button"
                tabindex="-1"
                aria-label={n.label}
              >
                {#if highlightId === n.id}
                  <circle class="ring" r={r + 5} fill="none" />
                {/if}
                <circle r={r} fill={fillFor(n)} stroke="rgba(0,0,0,0.35)" stroke-width="0.6" />
                {#if showLabels}
                  <text x={r + 3} y="3.5" class="node-label">{n.label}</text>
                {/if}
              </g>
            {/if}
          {/each}
        </g>
      </g>
    </svg>

    <!-- hover tooltips -->
    {#if hoverNode}
      <div class="cg-tip" style:left={`${hoverXY.x + 12}px`} style:top={`${hoverXY.y + 12}px`}>
        <div class="tip-title">{hoverNode.label}</div>
        <div class="tip-meta">
          <span class="tip-kind" style:background={nodeColor(hoverNode)}>{hoverNode.kind}</span>
          <span class="muted">{hoverNode.group}</span>
        </div>
        {#if hoverNode.file}
          <div class="tip-file">{hoverNode.file}{#if hoverNode.line}:{hoverNode.line}{/if}</div>
        {/if}
        {#if hoverHit?.reasons?.length}
          <div class="tip-reasons">
            {#each hoverHit.reasons as rr, i (i)}
              <span class="tip-reason"><b>{rr.kind}</b> {rr.detail || rr.score.toFixed(2)}</span>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
    {#if hoverEdge}
      <div class="cg-edge-tip" style:left={`${hoverEdge.x + 8}px`} style:top={`${hoverEdge.y + 8}px`}>
        {hoverEdge.rel}{#if hoverEdge.detail}<span class="muted"> · {hoverEdge.detail}</span>{/if}
      </div>
    {/if}

    <!-- relationship legend -->
    <div class="cg-legend">
      {#each REL_LEGEND as l (l.rel)}
        {@const st = edgeStyle(l.rel)}
        <span class="leg">
          <svg width="22" height="8" aria-hidden="true">
            <line x1="1" y1="4" x2="21" y2="4" stroke={st.stroke} stroke-width="1.6" stroke-dasharray={st.dash} />
          </svg>
          {l.label}
        </span>
      {/each}
    </div>
  </div>

  <!-- ── Right: control panels ───────────────────────────────────────── -->
  <aside class="cg-panels">
    <section class="panel">
      <button class="panel-head" onclick={() => (showFilters = !showFilters)}>
        <Icon name={showFilters ? 'chevronDown' : 'chevronRight'} size={12} /> Filters
      </button>
      {#if showFilters}
        <div class="panel-body">
          <input class="p-input" type="text" placeholder="Filter by name…" bind:value={textFilter} />
          <label class="p-check"><input type="checkbox" bind:checked={showKnowledge} /> Knowledge</label>
          <label class="p-check"><input type="checkbox" bind:checked={showCode} /> Code</label>
          <label class="p-check"><input type="checkbox" bind:checked={hideOrphans} /> Hide orphans</label>
          {#if kinds.length}
            <div class="p-sub">By kind</div>
            <div class="kind-grid">
              {#each kinds as k (k.kind)}
                <button class="kind-tog" class:off={kindOff[k.kind]} onclick={() => toggleKind(k.kind)} title={`${k.group} · ${k.kind}`}>
                  <span class="kind-dot" style:background={k.group === 'knowledge' ? knowledgeColor(k.kind) : nodeColor({ id: '', label: '', kind: k.kind, group: k.group, file: null, line: null })}></span>
                  {k.kind}
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    </section>

    <section class="panel">
      <button class="panel-head" onclick={() => (showDisplay = !showDisplay)}>
        <Icon name={showDisplay ? 'chevronDown' : 'chevronRight'} size={12} /> Display
      </button>
      {#if showDisplay}
        <div class="panel-body">
          <label class="p-check"><input type="checkbox" bind:checked={showLabels} /> Show labels</label>
          <label class="p-check"><input type="checkbox" bind:checked={sizeByDegree} /> Size by degree</label>
          <label class="p-check"><input type="checkbox" bind:checked={showArrows} /> Arrows</label>
          <label class="p-range">
            Link thickness <span class="muted">{linkThickness.toFixed(1)}</span>
            <input type="range" min="0.5" max="3" step="0.1" bind:value={linkThickness} />
          </label>
        </div>
      {/if}
    </section>

    <section class="panel">
      <button class="panel-head" onclick={() => (showForces = !showForces)}>
        <Icon name={showForces ? 'chevronDown' : 'chevronRight'} size={12} /> Forces
      </button>
      {#if showForces}
        <div class="panel-body">
          <label class="p-range">
            Center strength <span class="muted">{centerStrength.toFixed(3)}</span>
            <input type="range" min="0" max="0.2" step="0.005" bind:value={centerStrength} />
          </label>
          <label class="p-range">
            Repel <span class="muted">{repel.toFixed(0)}</span>
            <input type="range" min="100" max="4000" step="50" bind:value={repel} />
          </label>
          <label class="p-range">
            Link distance <span class="muted">{linkDistance.toFixed(0)}</span>
            <input type="range" min="20" max="240" step="5" bind:value={linkDistance} />
          </label>
          <button class="p-btn" onclick={resetView}>Reheat & fit</button>
        </div>
      {/if}
    </section>

    <section class="panel">
      <button class="panel-head" onclick={() => (showGroups = !showGroups)}>
        <Icon name={showGroups ? 'chevronDown' : 'chevronRight'} size={12} /> Groups
      </button>
      {#if showGroups}
        <div class="panel-body">
          <p class="p-hint">Colour nodes matching a query (name, kind, file — plus semantic knowledge hits).</p>
          <div class="group-row">
            <input class="p-input" type="text" placeholder="e.g. limits" bind:value={groupQuery} onkeydown={(e) => e.key === 'Enter' && applyGroup()} />
            <input class="color-input" type="color" bind:value={groupColor} title="Group colour" aria-label="Group colour" />
          </div>
          <div class="group-actions">
            <button class="p-btn primary" onclick={applyGroup} disabled={!groupQuery.trim()}>Apply</button>
            <button class="p-btn" onclick={clearGroup} disabled={!groupApplied}>Clear</button>
          </div>
          {#if groupApplied}
            <p class="p-matched">
              <span class="swatch" style:background={groupColor}></span>
              {groupMatched.size} matched “{groupApplied}”
            </p>
          {/if}
        </div>
      {/if}
    </section>
  </aside>
</div>

<style>
  .cg {
    display: grid;
    grid-template-columns: 230px 1fr 230px;
    height: 100%;
    min-height: 0;
    overflow: hidden;
  }
  .cg.no-tree {
    grid-template-columns: 1fr 230px;
  }

  /* ── tree ─────────────────────────────────────────────── */
  .cg-tree {
    display: flex;
    flex-direction: column;
    border-inline-end: 1px solid var(--border);
    min-height: 0;
    background: var(--bg-sidebar, var(--surface));
  }
  .cg-tree-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 10px;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }
  .cg-tree-body {
    overflow-y: auto;
    flex: 1;
    padding: 4px 0 12px;
  }
  .tree-section { margin-bottom: 2px; }
  .tree-sec-head {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    background: transparent;
    border: none;
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
    padding: 5px 8px;
    cursor: pointer;
  }
  .tree-sec-head .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .tree-bucket { margin-inline-start: 8px; }
  .tree-bucket-head {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    background: transparent;
    border: none;
    color: var(--text-dim);
    font-size: 11.5px;
    padding: 3px 8px;
    cursor: pointer;
    text-align: start;
  }
  .tree-bucket-head:hover { color: var(--text); }
  .bucket-label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tree-leaves {
    list-style: none;
    margin: 0;
    padding: 0 0 2px 18px;
  }
  .tree-leaf {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    background: transparent;
    border: none;
    color: var(--text-dim);
    font-size: 11.5px;
    padding: 3px 8px;
    cursor: pointer;
    text-align: start;
    border-radius: 5px;
  }
  .tree-leaf:hover { background: var(--surface-2); color: var(--text); }
  .tree-leaf.hl { background: #7ee787; color: #0b0b0b; }
  .leaf-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
  }
  .leaf-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tree-empty {
    padding: 14px;
    font-size: 12px;
    opacity: 0.6;
  }

  /* ── canvas ───────────────────────────────────────────── */
  .cg-canvas {
    position: relative;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    background:
      radial-gradient(circle at 1px 1px, color-mix(in srgb, var(--text-dim) 18%, transparent) 1px, transparent 0);
    background-size: 26px 26px;
  }
  .cg-svg {
    display: block;
    width: 100%;
    height: 100%;
    touch-action: none;
    cursor: grab;
  }
  .cg-toolbar {
    position: absolute;
    top: 8px;
    left: 8px;
    right: 8px;
    z-index: 3;
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .repo-select {
    font-size: 11.5px;
    padding: 4px 6px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    max-width: 160px;
  }
  .tb-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11.5px;
    padding: 4px 9px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    cursor: pointer;
  }
  .tb-btn:hover { color: var(--text); }
  .tb-btn.ml { margin-inline-start: auto; }
  .icon-btn {
    display: inline-flex;
    padding: 3px;
    border-radius: 5px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    cursor: pointer;
  }
  .icon-btn:hover { color: var(--text); }
  .cg-stats {
    font-size: 11px;
    color: var(--text-dim);
    padding: 2px 6px;
    background: color-mix(in srgb, var(--surface) 80%, transparent);
    border-radius: 5px;
  }

  .cg-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    z-index: 2;
    color: var(--text-dim);
    pointer-events: none;
    text-align: center;
  }
  .cg-overlay p { margin: 0; font-size: 13px; }

  .node-label {
    font-size: 10px;
    fill: var(--text);
    paint-order: stroke;
    stroke: var(--bg);
    stroke-width: 2.5px;
    stroke-linejoin: round;
    pointer-events: none;
  }
  .cg-node { cursor: pointer; }
  .cg-node:hover circle { stroke: #fff; stroke-width: 1.2; }
  .cg-node .ring {
    stroke: #7ee787;
    stroke-width: 2;
  }
  .edge-hit {
    stroke: transparent;
    stroke-width: 8;
    cursor: help;
  }

  .cg-tip {
    position: absolute;
    z-index: 5;
    max-width: 280px;
    pointer-events: none;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 7px 9px;
    box-shadow: 0 6px 22px rgba(0, 0, 0, 0.4);
    font-size: 11.5px;
  }
  .tip-title { font-weight: 700; overflow-wrap: anywhere; }
  .tip-meta { display: flex; align-items: center; gap: 6px; margin-top: 3px; }
  .tip-kind {
    font-size: 9px;
    padding: 1px 5px;
    border-radius: 4px;
    color: #000;
  }
  .tip-file {
    font-size: 10.5px;
    color: var(--text-dim);
    margin-top: 3px;
    overflow-wrap: anywhere;
  }
  .tip-reasons { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 5px; }
  .tip-reason {
    font-size: 9.5px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  .tip-reason b { color: var(--text); }
  .cg-edge-tip {
    position: absolute;
    z-index: 5;
    pointer-events: none;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 2px 7px;
    font-size: 10.5px;
    color: var(--text);
  }

  .cg-legend {
    position: absolute;
    bottom: 8px;
    left: 8px;
    z-index: 3;
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding: 5px 8px;
    background: color-mix(in srgb, var(--surface) 85%, transparent);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-size: 10px;
    color: var(--text-dim);
  }
  .leg { display: inline-flex; align-items: center; gap: 4px; }

  /* ── panels ───────────────────────────────────────────── */
  .cg-panels {
    border-inline-start: 1px solid var(--border);
    overflow-y: auto;
    min-height: 0;
    background: var(--bg-sidebar, var(--surface));
  }
  .panel { border-bottom: 1px solid var(--border); }
  .panel-head {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    background: transparent;
    border: none;
    color: var(--text);
    font-size: 12px;
    font-weight: 700;
    padding: 8px 10px;
    cursor: pointer;
  }
  .panel-body {
    display: flex;
    flex-direction: column;
    gap: 7px;
    padding: 4px 10px 12px;
  }
  .p-input {
    width: 100%;
    font-size: 12px;
    padding: 5px 7px;
    border-radius: 5px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
  }
  .p-check {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    color: var(--text-dim);
    cursor: pointer;
  }
  .p-range {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .p-range input[type='range'] { width: 100%; accent-color: #7ee787; }
  .p-check input { accent-color: #7ee787; }
  .p-sub {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    opacity: 0.8;
    margin-top: 2px;
  }
  .kind-grid { display: flex; flex-wrap: wrap; gap: 4px; }
  .kind-tog {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 10.5px;
    padding: 2px 7px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
  }
  .kind-tog.off { opacity: 0.4; text-decoration: line-through; }
  .kind-dot { width: 8px; height: 8px; border-radius: 50%; }
  .p-btn {
    font-size: 11.5px;
    padding: 5px 10px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    cursor: pointer;
  }
  .p-btn:hover:not(:disabled) { color: var(--text); }
  .p-btn:disabled { opacity: 0.4; cursor: default; }
  .p-btn.primary {
    background: #7ee787;
    color: #0b0b0b;
    border-color: #7ee787;
    font-weight: 600;
  }
  .p-hint, .p-matched { font-size: 11px; color: var(--text-dim); margin: 0; }
  .group-row { display: flex; gap: 6px; align-items: center; }
  .color-input {
    width: 32px;
    height: 30px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--surface);
    cursor: pointer;
    flex: none;
  }
  .group-actions { display: flex; gap: 6px; }
  .p-matched { display: flex; align-items: center; gap: 6px; margin-top: 2px; }
  .swatch { width: 12px; height: 12px; border-radius: 3px; flex: none; }
  .muted { opacity: 0.55; }

  /* ── responsive ───────────────────────────────────────── */
  @media (max-width: 1024px) {
    .cg { grid-template-columns: 1fr 200px; }
    .cg-tree { display: none; }
  }
  @media (max-width: 640px) {
    .cg, .cg.no-tree { grid-template-columns: 1fr; }
    .cg-panels { display: none; }
  }
</style>
