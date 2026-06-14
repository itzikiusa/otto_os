<script lang="ts">
  // Two-pane: LEFT = refs tree (local/remote/tags), MIDDLE = commit graph, RIGHT = commit detail/diff.
  import { api } from '../../lib/api/client';
  import type { CommitInfo, RefsResp, RefBranch, RepoStatusResp, DiffResp, FileDiff, DiffLine } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    repoId: string;
    status: RepoStatusResp;
    onstatus: (s: RepoStatusResp) => void;
    /** Drag-to-merge: dropping branch `source` onto a different local branch
     *  `target` asks the parent to open the merge approval modal. Nothing merges
     *  here — the parent owns the modal + conflict-resolver routing. */
    onmergerequest?: (source: string, target: string) => void;
  }
  let { repoId, status, onstatus, onmergerequest }: Props = $props();

  // ── Drag-to-merge state ─────────────────────────────────────────────────────
  // The branch currently being dragged ({ name, remote }), and the local branch
  // the pointer is hovering as a candidate drop target. A valid drop = a local
  // branch whose name differs from the drag source's name.
  let dragSource = $state<{ name: string; remote: boolean } | null>(null);
  let dragOverTarget = $state<string | null>(null);

  function dragSourceName(): string | null {
    if (!dragSource) return null;
    // A remote ref like "origin/feature" merges as its local short name.
    return dragSource.remote ? dragSource.name.replace(/^[^/]+\//, '') : dragSource.name;
  }

  function isValidDropTarget(localName: string): boolean {
    const src = dragSourceName();
    return src !== null && src !== localName;
  }

  function onRefDragStart(e: DragEvent, name: string, remote: boolean): void {
    dragSource = { name, remote };
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      // Stash a payload too so the drag is "real" in all browsers/webviews.
      e.dataTransfer.setData('text/plain', name);
    }
  }

  function onRefDragEnd(): void {
    dragSource = null;
    dragOverTarget = null;
  }

  function onTargetDragOver(e: DragEvent, localName: string): void {
    if (!isValidDropTarget(localName)) return;
    e.preventDefault(); // allow drop
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragOverTarget = localName;
  }

  function onTargetDragLeave(localName: string): void {
    if (dragOverTarget === localName) dragOverTarget = null;
  }

  function onTargetDrop(e: DragEvent, localName: string): void {
    if (!isValidDropTarget(localName)) return;
    e.preventDefault();
    const source = dragSourceName();
    dragOverTarget = null;
    dragSource = null;
    if (source && onmergerequest) onmergerequest(source, localName);
  }

  // ── Refs ──────────────────────────────────────────────────────────────────
  let refs: RefsResp | null = $state(null);
  let refsLoading = $state(true);

  // Section collapse state
  let localOpen = $state(true);
  let remoteOpen = $state(true);
  let tagsOpen = $state(false);

  let checkoutBusy = $state('');

  // ── Commits / graph ───────────────────────────────────────────────────────
  let commits: CommitInfo[] = $state([]);
  let commitsLoading = $state(true);

  // ── Commit detail / diff ─────────────────────────────────────────────────
  let selectedSha = $state<string | null>(null);
  let selectedCommit = $state<CommitInfo | null>(null);
  let diffResp = $state<DiffResp | null>(null);
  let diffLoading = $state(false);
  // Track which files are collapsed (path → true = collapsed)
  let fileCollapsed = $state<Record<string, boolean>>({});

  $effect(() => {
    const id = repoId;
    refsLoading = true;
    commitsLoading = true;
    refs = null;
    commits = [];

    void api
      .get<RefsResp>(`/repos/${id}/refs`)
      .then((r) => (refs = r))
      .catch(() => (refs = { local: [], remote: [], tags: [] }))
      .finally(() => (refsLoading = false));

    void api
      .get<CommitInfo[]>(`/repos/${id}/log?all=true&limit=200`)
      .then((c) => (commits = c))
      .catch(() => (commits = []))
      .finally(() => (commitsLoading = false));
  });

  async function checkout(branch: string, create: boolean): Promise<void> {
    checkoutBusy = branch;
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/checkout`, { branch, create });
      onstatus(s);
      toasts.success(create ? 'Branch created' : 'Switched branch', branch);
      // Refresh refs after checkout
      refs = await api.get<RefsResp>(`/repos/${repoId}/refs`);
    } catch (e) {
      toasts.error('Checkout failed', e instanceof Error ? e.message : String(e));
    } finally {
      checkoutBusy = '';
    }
  }

  function checkoutRemote(b: RefBranch): void {
    // strip "origin/" prefix to get local branch name
    const localName = b.name.replace(/^[^/]+\//, '');
    void checkout(localName, true);
  }

  async function selectCommit(commit: CommitInfo): Promise<void> {
    if (selectedSha === commit.sha) {
      // clicking again deselects
      clearSelection();
      return;
    }
    selectedSha = commit.sha;
    selectedCommit = commit;
    diffResp = null;
    diffLoading = true;
    fileCollapsed = {};
    try {
      const resp = await api.get<DiffResp>(
        `/repos/${repoId}/diff?target=${encodeURIComponent('commit:' + commit.sha)}`
      );
      diffResp = resp;
      // Auto-collapse files with >400 changed lines
      const next: Record<string, boolean> = {};
      for (const f of resp.files) {
        const { add, del } = changedLinesCount(f);
        next[f.path] = add + del > 400;
      }
      fileCollapsed = next;
    } catch (e) {
      toasts.error('Failed to load diff', e instanceof Error ? e.message : String(e));
      diffResp = { files: [] };
    } finally {
      diffLoading = false;
    }
  }

  function clearSelection(): void {
    selectedSha = null;
    selectedCommit = null;
    diffResp = null;
    diffLoading = false;
    fileCollapsed = {};
  }

  // ── Lane / graph algorithm ────────────────────────────────────────────────
  // palette of 8 colors (CSS vars so they adapt to theme)
  const PALETTE = [
    '#5B8BF5', '#E06C75', '#56B6C2', '#E5C07B',
    '#98C379', '#C678DD', '#61AFEF', '#D19A66',
  ];

  interface LaneRow {
    commit: CommitInfo;
    col: number;       // which column this commit's node lands on
    lines: LaneLine[]; // segments to draw in the SVG gutter
    color: string;     // node color
  }

  interface LaneLine {
    fromCol: number;
    toCol: number;
    color: string;
    kind: 'vert' | 'merge-in' | 'branch-out';
  }

  const LANE_W = 14; // pixels per lane column
  const NODE_R = 4;  // node radius

  const laneRows = $derived.by((): LaneRow[] => {
    if (commits.length === 0) return [];

    // lanes[i] = sha of the commit expected next in lane i (null = free)
    const lanes: (string | null)[] = [];
    // laneColor[i] = color index for lane i
    const laneColors: number[] = [];

    let colorIdx = 0;

    function allocateLane(sha: string | null): number {
      // prefer reusing a free slot
      const free = lanes.indexOf(null);
      if (free !== -1) {
        lanes[free] = sha;
        if (laneColors[free] === undefined) laneColors[free] = colorIdx++ % PALETTE.length;
        return free;
      }
      lanes.push(sha);
      laneColors.push(colorIdx++ % PALETTE.length);
      return lanes.length - 1;
    }

    const rows: LaneRow[] = [];

    for (const commit of commits) {
      // Find this commit's column (which lane is "expecting" this sha)
      let col = lanes.indexOf(commit.sha);
      if (col === -1) {
        col = allocateLane(commit.sha);
      }
      const color = PALETTE[laneColors[col] % PALETTE.length];

      // Build line segments for BEFORE this node (connecting to previous)
      const lines: LaneLine[] = [];

      // Vertical continuations for all active lanes (before node)
      for (let i = 0; i < lanes.length; i++) {
        if (i === col) continue;
        if (lanes[i] !== null) {
          lines.push({ fromCol: i, toCol: i, color: PALETTE[laneColors[i] % PALETTE.length], kind: 'vert' });
        }
      }

      // Now update lanes: replace this lane with first parent
      const [firstParent, ...extraParents] = commit.parents;
      if (firstParent) {
        lanes[col] = firstParent;
      } else {
        lanes[col] = null; // root commit or end of history
      }

      // For additional parents (merge commits): find existing lane or create one
      for (const parent of extraParents) {
        const existingLane = lanes.indexOf(parent);
        if (existingLane !== -1) {
          // already tracked — draw merge-in line
          lines.push({ fromCol: col, toCol: existingLane, color, kind: 'merge-in' });
        } else {
          const newLane = allocateLane(parent);
          lines.push({ fromCol: col, toCol: newLane, color: PALETTE[laneColors[newLane] % PALETTE.length], kind: 'merge-in' });
        }
      }

      // If current lane had a different sha expected before, that's a branch-out
      // (handled by allocateLane above; no extra drawing needed for simple cases)

      rows.push({ commit, col, lines, color });
    }

    return rows;
  });

  // Number of lanes actually in use across the whole graph. All rows share one
  // gutter width so the lane dots line up vertically; we size it to the real
  // lane count (plus a half-lane of breathing room for the node radius) instead
  // of a large fixed area, and cap it so a wide fan-out can't blow out the layout.
  const MAX_GUTTER_W = 200;
  const gutterWidth = $derived.by(() => {
    if (laneRows.length === 0) return LANE_W;
    const laneCount = Math.max(...laneRows.map((r) => r.col + 1));
    return Math.min(laneCount * LANE_W + LANE_W / 2, MAX_GUTTER_W);
  });

  // ── Helpers ───────────────────────────────────────────────────────────────
  function fmtDate(iso: string): string {
    const d = new Date(iso);
    const now = Date.now();
    const diff = Math.floor((now - d.getTime()) / 1000);
    if (diff < 60) return 'just now';
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    if (diff < 86400 * 30) return `${Math.floor(diff / 86400)}d ago`;
    return d.toLocaleDateString([], { month: 'short', day: 'numeric', year: '2-digit' });
  }

  function isTagRef(ref: string): boolean {
    return ref.startsWith('tag: ') || ref.startsWith('tag:');
  }

  function refLabel(ref: string): string {
    return ref.replace(/^tag:\s*/, '');
  }

  // ── Inline diff helpers ───────────────────────────────────────────────────
  function changedLinesCount(f: FileDiff): { add: number; del: number } {
    let add = 0;
    let del = 0;
    for (const h of f.hunks) {
      for (const l of h.lines) {
        if (l.origin === 'add') add++;
        else if (l.origin === 'del') del++;
      }
    }
    return { add, del };
  }

  function toggleFileCollapse(path: string): void {
    fileCollapsed = { ...fileCollapsed, [path]: !fileCollapsed[path] };
  }

  function lineClass(origin: DiffLine['origin']): string {
    if (origin === 'add') return 'dl-add';
    if (origin === 'del') return 'dl-del';
    return 'dl-ctx';
  }

  function lineSign(origin: DiffLine['origin']): string {
    if (origin === 'add') return '+';
    if (origin === 'del') return '−';
    return ' ';
  }

  const detailTotals = $derived.by(() => {
    if (!diffResp) return { add: 0, del: 0 };
    let add = 0;
    let del = 0;
    for (const f of diffResp.files) {
      const c = changedLinesCount(f);
      add += c.add;
      del += c.del;
    }
    return { add, del };
  });
</script>

<div class="graphview">
  <!-- ── LEFT: refs tree ───────────────────────────────────────────────────── -->
  <aside class="refs-panel">
    {#if refsLoading}
      <div style="padding: 10px"><Skeleton rows={6} height={22} /></div>
    {:else if refs}
      <!-- LOCAL -->
      <div class="ref-section">
        <button class="ref-header" onclick={() => (localOpen = !localOpen)}>
          <Icon name={localOpen ? 'chevronDown' : 'chevronRight'} size={11} />
          <Icon name="branch" size={12} />
          <span>LOCAL</span>
          <span class="ref-count">{refs.local.length}</span>
        </button>
        {#if localOpen}
          {#each refs.local as b (b.name)}
            <button
              class="ref-row"
              class:current={b.is_current}
              class:drag-target={dragOverTarget === b.name}
              class:dragging={dragSource?.name === b.name && !dragSource.remote}
              draggable={checkoutBusy === ''}
              ondragstart={(e) => onRefDragStart(e, b.name, false)}
              ondragend={onRefDragEnd}
              ondragover={(e) => onTargetDragOver(e, b.name)}
              ondragleave={() => onTargetDragLeave(b.name)}
              ondrop={(e) => onTargetDrop(e, b.name)}
              disabled={checkoutBusy !== ''}
              onclick={() => !b.is_current && checkout(b.name, false)}
              title={dragSource && isValidDropTarget(b.name)
                ? `Merge ${dragSourceName()} → ${b.name}`
                : b.name}
            >
              <Icon name="dot" size={10} />
              <span class="mono ref-name">{b.name}</span>
              {#if b.is_current}<span class="current-badge">✓</span>{/if}
              {#if checkoutBusy === b.name}<span class="dim">…</span>{/if}
            </button>
          {:else}
            <div class="dim ref-empty">No local branches</div>
          {/each}
        {/if}
      </div>

      <!-- REMOTE -->
      <div class="ref-section">
        <button class="ref-header" onclick={() => (remoteOpen = !remoteOpen)}>
          <Icon name={remoteOpen ? 'chevronDown' : 'chevronRight'} size={11} />
          <Icon name="globe" size={12} />
          <span>REMOTE</span>
          <span class="ref-count">{refs.remote.length}</span>
        </button>
        {#if remoteOpen}
          {#each refs.remote as b (b.name)}
            <button
              class="ref-row remote"
              class:dragging={dragSource?.name === b.name && dragSource.remote}
              draggable={checkoutBusy === ''}
              ondragstart={(e) => onRefDragStart(e, b.name, true)}
              ondragend={onRefDragEnd}
              disabled={checkoutBusy !== ''}
              onclick={() => checkoutRemote(b)}
              title="Drag onto a local branch to merge, or click to checkout as a local tracking branch"
            >
              <Icon name="dot" size={10} />
              <span class="mono ref-name">{b.name}</span>
              {#if checkoutBusy === b.name.replace(/^[^/]+\//, '')}<span class="dim">…</span>{/if}
            </button>
          {:else}
            <div class="dim ref-empty">No remote branches</div>
          {/each}
        {/if}
      </div>

      <!-- TAGS -->
      <div class="ref-section">
        <button class="ref-header" onclick={() => (tagsOpen = !tagsOpen)}>
          <Icon name={tagsOpen ? 'chevronDown' : 'chevronRight'} size={11} />
          <Icon name="tag" size={12} />
          <span>TAGS</span>
          <span class="ref-count">{refs.tags.length}</span>
        </button>
        {#if tagsOpen}
          {#each refs.tags as t (t.name)}
            <div class="ref-row tag" title={t.name}>
              <Icon name="tag" size={10} />
              <span class="mono ref-name">{t.name}</span>
            </div>
          {:else}
            <div class="dim ref-empty">No tags</div>
          {/each}
        {/if}
      </div>
    {/if}
  </aside>

  <!-- ── MIDDLE: commit graph ─────────────────────────────────────────────── -->
  <div class="graph-panel" class:panel-shrunk={selectedSha !== null}>
    {#if commitsLoading}
      <div style="padding: 10px"><Skeleton rows={12} height={28} /></div>
    {:else if commits.length === 0}
      <div class="dim" style="padding: 18px; font-size: 12px">No commits found.</div>
    {:else}
      <div class="graph-list">
        {#each laneRows as row (row.commit.sha)}
          {@const svgW = gutterWidth}
          {@const cx = row.col * LANE_W + LANE_W / 2}
          {@const cy = 14}
          {@const totalH = 28}
          {@const isSelected = selectedSha === row.commit.sha}
          <button
            class="graph-row"
            class:graph-row-selected={isSelected}
            onclick={() => selectCommit(row.commit)}
            title={row.commit.subject}
            aria-pressed={isSelected}
          >
            <!-- SVG gutter -->
            <svg
              class="gutter"
              width={svgW}
              height={totalH}
              style="flex-shrink: 0; width: {svgW}px;"
            >
              <!-- draw lane lines (background, behind node) -->
              {#each row.lines as line}
                {@const x1 = line.fromCol * LANE_W + LANE_W / 2}
                {@const x2 = line.toCol * LANE_W + LANE_W / 2}
                {#if line.kind === 'vert'}
                  <line x1={x1} y1={0} x2={x1} y2={totalH} stroke={line.color} stroke-width="1.5" />
                {:else}
                  <!-- merge-in: curved path from node to target column -->
                  <path
                    d="M{cx},{cy} Q{cx},{cy + 10} {x2},{totalH}"
                    stroke={line.color}
                    stroke-width="1.5"
                    fill="none"
                  />
                {/if}
              {/each}
              <!-- vertical continuation for current lane -->
              <line x1={cx} y1={cy + NODE_R} x2={cx} y2={totalH} stroke={row.color} stroke-width="1.5" />
              <!-- node circle -->
              <circle cx={cx} cy={cy} r={NODE_R} fill={row.color} />
            </svg>

            <!-- commit info -->
            <div class="commit-info">
              <div class="ci-top">
                <span class="ci-subject">{row.commit.subject}</span>
                {#each row.commit.refs as ref}
                  <span class="ref-chip" class:tag-chip={isTagRef(ref)}>{refLabel(ref)}</span>
                {/each}
              </div>
              <div class="ci-meta">
                <span class="mono ci-sha">{row.commit.short_sha}</span>
                <span class="dim ci-author">{row.commit.author}</span>
                <span class="grow"></span>
                <span class="dim ci-date">{fmtDate(row.commit.date)}</span>
              </div>
            </div>
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <!-- ── RIGHT: commit detail + diff ─────────────────────────────────────── -->
  <div class="detail-panel" class:detail-visible={selectedSha !== null}>
    {#if selectedSha === null}
      <div class="detail-empty">
        <span class="detail-empty-label">COMMIT</span>
        <span class="dim detail-empty-hint">select a commit</span>
      </div>
    {:else if selectedCommit !== null}
      <!-- Header -->
      <div class="detail-header">
        <div class="detail-header-main">
          <div class="detail-title-row">
            <span class="mono detail-sha">{selectedCommit.short_sha}</span>
            {#each selectedCommit.refs as ref}
              <span class="ref-chip" class:tag-chip={isTagRef(ref)}>{refLabel(ref)}</span>
            {/each}
            <span class="grow"></span>
            <button class="detail-close" onclick={clearSelection} title="Close" aria-label="Close commit detail">
              ✕
            </button>
          </div>
          <div class="detail-subject">{selectedCommit.subject}</div>
          <div class="detail-meta">
            <span class="detail-author">{selectedCommit.author}</span>
            <span class="dim detail-dot">·</span>
            <span class="dim detail-date">{fmtDate(selectedCommit.date)}</span>
          </div>
        </div>
      </div>

      <!-- Diff area -->
      <div class="detail-diff">
        {#if diffLoading}
          <div class="detail-diff-loading">
            <Skeleton rows={8} height={20} />
          </div>
        {:else if diffResp !== null}
          {#if diffResp.files.length === 0}
            <div class="dim" style="padding: 18px; font-size: 12px; text-align: center">No file changes.</div>
          {:else}
            <!-- Diff summary bar -->
            <div class="diff-summary-bar">
              <span class="dim" style="font-size: 11px">{diffResp.files.length} file{diffResp.files.length === 1 ? '' : 's'}</span>
              <span class="ds-add">+{detailTotals.add}</span>
              <span class="ds-del">−{detailTotals.del}</span>
            </div>

            <!-- Per-file diff -->
            {#each diffResp.files as file (file.path)}
              {@const stats = changedLinesCount(file)}
              {@const isCollapsed = fileCollapsed[file.path] ?? false}
              <div class="df-block">
                <!-- File header -->
                <button
                  class="df-head"
                  onclick={() => toggleFileCollapse(file.path)}
                  title={file.path}
                >
                  <span class="df-chevron dim">
                    {isCollapsed ? '▶' : '▼'}
                  </span>
                  <span class="mono df-path">
                    {#if file.old_path}{file.old_path} → {/if}{file.path}
                  </span>
                  <span class="grow"></span>
                  <span class="ds-add">+{stats.add}</span>
                  <span class="ds-del">−{stats.del}</span>
                </button>

                {#if !isCollapsed}
                  {#if file.is_binary}
                    <div class="df-binary dim">Binary file — no text diff.</div>
                  {:else}
                    <div class="df-hunks">
                      {#each file.hunks as hunk, hi (hi)}
                        <div class="hunk-header mono">{hunk.header}</div>
                        <table class="dl-table">
                          <tbody>
                            {#each hunk.lines as line, li (li)}
                              <tr class="dl-row {lineClass(line.origin)}">
                                <td class="dl-gut dl-old">{line.old_line ?? ''}</td>
                                <td class="dl-gut dl-new">{line.new_line ?? ''}</td>
                                <td class="dl-sign">{lineSign(line.origin)}</td>
                                <td class="dl-code mono">{line.content}</td>
                              </tr>
                            {/each}
                          </tbody>
                        </table>
                      {/each}
                    </div>
                  {/if}
                {/if}
              </div>
            {/each}
          {/if}
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .graphview {
    display: flex;
    height: 100%;
    min-height: 0;
  }

  /* ── refs panel ── */
  .refs-panel {
    width: 220px;
    flex-shrink: 0;
    overflow-y: auto;
    border-right: 1px solid var(--border);
    padding: 6px 0;
  }
  .ref-section {
    margin-bottom: 2px;
  }
  .ref-header {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 5px 10px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.04em;
    cursor: pointer;
    text-transform: uppercase;
    text-align: left;
    transition: color 110ms ease-out;
  }
  .ref-header:hover {
    color: var(--text);
  }
  .ref-count {
    margin-left: auto;
    background: var(--surface-2);
    border-radius: 999px;
    font-size: 9.5px;
    padding: 1px 5px;
    font-weight: 600;
    letter-spacing: 0;
  }
  .ref-row {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    height: 24px;
    padding: 0 10px 0 22px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    cursor: pointer;
    text-align: left;
    overflow: hidden;
    transition: background 100ms ease-out, color 100ms ease-out;
  }
  .ref-row:hover:not(:disabled) {
    background: var(--surface-2);
    color: var(--text);
  }
  .ref-row:disabled {
    cursor: default;
  }
  .ref-row.current {
    color: var(--accent);
    font-weight: 600;
  }
  .ref-row[draggable='true'] {
    cursor: grab;
  }
  .ref-row.dragging {
    opacity: 0.45;
  }
  /* Accent outline while a valid merge source hovers this local branch. */
  .ref-row.drag-target {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--text);
    outline: 1.5px solid var(--accent);
    outline-offset: -1.5px;
    border-radius: var(--radius-s);
  }
  .ref-row.tag {
    cursor: default;
    color: var(--text-dim);
  }
  .ref-row.remote {
    color: var(--text-dim);
  }
  .ref-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11.5px;
  }
  .current-badge {
    font-size: 10px;
    color: var(--accent);
    font-weight: 700;
  }
  .ref-empty {
    padding: 4px 22px 6px;
    font-size: 11px;
  }

  /* ── graph panel ── */
  .graph-panel {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    overflow-x: auto;
    transition: flex 180ms ease-out;
  }
  /* When detail is open, the commit list becomes a fixed-width column and the
     detail panel flexes to fill the rest of the page (see .detail-visible). */
  .graph-panel.panel-shrunk {
    flex: 0 0 380px;
    width: auto;
    min-width: 280px;
    border-right: 1px solid var(--border);
  }
  .graph-list {
    display: flex;
    flex-direction: column;
    min-width: min-content;
  }
  .graph-row {
    display: flex;
    align-items: center;
    width: 100%;
    height: 28px;
    padding-right: 12px;
    border: none;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 50%, transparent);
    background: transparent;
    cursor: pointer;
    text-align: left;
    transition: background 100ms ease-out;
  }
  .graph-row:hover {
    background: var(--surface-2);
  }
  .graph-row-selected,
  .graph-row-selected:hover {
    /* Clear selected state: accent wash + inset accent bar (inset avoids a
       layout shift that a left border would cause on the selected row only). */
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    box-shadow: inset 2px 0 0 0 var(--accent);
  }
  .graph-row-selected .ci-subject {
    color: var(--accent);
    font-weight: 600;
  }
  .gutter {
    display: block;
    flex-shrink: 0;
    overflow: visible;
  }
  .commit-info {
    flex: 1;
    min-width: 0;
    padding: 0 8px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 1px;
  }
  .ci-top {
    display: flex;
    align-items: center;
    gap: 5px;
    overflow: hidden;
  }
  .ci-subject {
    font-size: 12px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .ref-chip {
    flex-shrink: 0;
    font-size: 9.5px;
    font-weight: 600;
    padding: 1px 5px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
    white-space: nowrap;
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ref-chip.tag-chip {
    background: color-mix(in srgb, #E5C07B 22%, transparent);
    color: #b8860b;
  }
  .ci-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 10px;
  }
  .ci-sha {
    color: var(--accent);
    font-size: 10px;
  }
  .ci-author {
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ci-date {
    font-size: 10px;
    white-space: nowrap;
  }

  /* ── detail panel ── */
  .detail-panel {
    width: 0;
    min-width: 0;
    flex-shrink: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    transition: width 180ms ease-out;
    border-left: 1px solid var(--border);
    background: var(--surface);
  }
  .detail-panel.detail-visible {
    /* Fill all remaining width after refs + commit-list so the page isn't
       half-empty when a commit is open. */
    flex: 1 1 0;
    width: auto;
    min-width: 360px;
    overflow: hidden;
  }
  .detail-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    gap: 6px;
  }
  .detail-empty-label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--text-dim);
    text-transform: uppercase;
  }
  .detail-empty-hint {
    font-size: 12px;
  }

  /* Detail header */
  .detail-header {
    flex-shrink: 0;
    padding: 10px 12px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
  }
  .detail-header-main {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .detail-title-row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .detail-sha {
    font-size: 11px;
    color: var(--accent);
    font-weight: 700;
    letter-spacing: 0.04em;
  }
  .detail-subject {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    line-height: 1.35;
    word-break: break-word;
  }
  .detail-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
  }
  .detail-author {
    color: var(--text-dim);
    font-size: 11px;
  }
  .detail-dot {
    font-size: 10px;
  }
  .detail-date {
    font-size: 11px;
  }
  .detail-close {
    flex-shrink: 0;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: 13px;
    line-height: 1;
    padding: 2px 4px;
    border-radius: var(--radius-s, 3px);
    margin-left: auto;
    transition: color 100ms, background 100ms;
  }
  .detail-close:hover {
    color: var(--text);
    background: var(--surface-2);
  }

  /* Diff area */
  .detail-diff {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .detail-diff-loading {
    padding: 12px;
  }
  .diff-summary-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    font-size: 11px;
    position: sticky;
    top: 0;
    z-index: 1;
  }
  .ds-add {
    color: var(--status-working);
    font-weight: 600;
    font-size: 11px;
  }
  .ds-del {
    color: var(--status-exited);
    font-weight: 600;
    font-size: 11px;
  }

  /* Per-file diff block */
  .df-block {
    border-bottom: 1px solid var(--border);
  }
  .df-head {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    padding: 5px 10px;
    border: none;
    background: var(--surface-2);
    cursor: pointer;
    font-size: 11px;
    color: var(--text);
    text-align: left;
    transition: background 80ms;
  }
  .df-head:hover {
    background: color-mix(in srgb, var(--accent) 7%, var(--surface-2));
  }
  .df-chevron {
    font-size: 8px;
    flex-shrink: 0;
  }
  .df-path {
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
    color: var(--text);
  }
  .df-binary {
    padding: 10px 12px;
    font-size: 11.5px;
  }
  .df-hunks {
    overflow-x: auto;
  }

  /* Hunk header */
  .hunk-header {
    padding: 2px 10px;
    font-size: 10px;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 7%, var(--surface));
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
  }

  /* Diff line table */
  .dl-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 11px;
    line-height: 1.5;
  }
  .dl-gut {
    width: 34px;
    min-width: 34px;
    text-align: right;
    padding: 0 5px 0 3px;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: 9.5px;
    user-select: none;
    vertical-align: top;
    border-right: 1px solid var(--border);
  }
  .dl-sign {
    width: 14px;
    text-align: center;
    color: var(--text-dim);
    user-select: none;
    font-family: var(--font-mono);
    font-size: 11px;
    vertical-align: top;
    padding: 0 1px;
  }
  .dl-code {
    padding: 0 8px 0 3px;
    white-space: pre;
    word-break: normal;
    user-select: text;
    font-size: 11px;
    font-family: var(--font-mono);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  tr.dl-row.dl-add {
    background: color-mix(in srgb, var(--status-working) 11%, transparent);
  }
  tr.dl-row.dl-add .dl-sign {
    color: var(--status-working);
  }
  tr.dl-row.dl-del {
    background: color-mix(in srgb, var(--status-exited) 10%, transparent);
  }
  tr.dl-row.dl-del .dl-sign {
    color: var(--status-exited);
  }
  tr.dl-row.dl-ctx {
    /* context lines: slightly dimmed */
    color: var(--text-dim);
  }

  /* Shared utilities */
  .grow {
    flex: 1;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
