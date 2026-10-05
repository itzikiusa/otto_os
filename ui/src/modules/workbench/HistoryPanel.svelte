<script lang="ts">
  // History: the doc's full revision timeline (append-only — only "Delete
  // forever" from the trash removes it). Pick a revision to see it, diff it
  // against the current text or another revision, and restore it (restoring
  // adds a new revision; the current text stays in history).
  import { onDestroy, untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import {
    getWorkbenchRevision,
    listWorkbenchRevisions,
    restoreWorkbenchRevision,
  } from '../../lib/api/workbench';
  import type { WorkbenchDocFull, WorkbenchRevision, WorkbenchRevisionDetail } from '../../lib/api/types';
  import RevisionDiff from './RevisionDiff.svelte';

  interface Props {
    ws: string;
    docId: string;
    /** The doc's newest revision seq — the timeline refreshes when it moves. */
    rev: number;
    /** The live editor buffer (diff target for "vs current"). */
    current: string;
    onrestored: (doc: WorkbenchDocFull) => void;
    onclose: () => void;
  }
  let { ws, docId, rev, current, onrestored, onclose }: Props = $props();

  let revs: WorkbenchRevision[] = $state([]);
  let loading = $state(true);
  let error: string | null = $state(null);
  let selected: number | null = $state(null);
  let compareTo: number | 'current' = $state('current');
  let detail: WorkbenchRevisionDetail | null = $state(null);
  let other: WorkbenchRevisionDetail | null = $state(null);
  let detailError: string | null = $state(null);
  let view: 'diff' | 'text' = $state('diff');
  let restoring = $state(false);
  const PAGE_SIZE = 100;
  let pageCursor: number | undefined = $state(undefined);
  let pageTrail: (number | undefined)[] = $state([]);
  let compareInput = $state('');
  let compareError: string | null = $state(null);
  let alive = true;
  let loadGeneration = 0;
  let visit = 0;
  let ownerKey = '';

  function syncDocument(): void {
    const key = `${ws}\0${docId}`;
    if (key === ownerKey) return;
    ownerKey = key;
    visit++;
    selected = null;
    compareTo = 'current';
    compareInput = '';
    compareError = null;
    revs = [];
    pageCursor = undefined;
    pageTrail = [];
    restoring = false;
  }

  function invalidate(): void {
    alive = false;
    visit++;
    loadGeneration++;
  }
  onDestroy(invalidate);
  function close(): void {
    invalidate();
    onclose();
  }


  const KIND_LABEL: Record<string, string> = {
    create: 'Created',
    auto: 'Autosave',
    checkpoint: 'Saved',
    restore: 'Restored',
  };

  async function load(before?: number, trail: (number | undefined)[] = []): Promise<void> {
    syncDocument();
    if (!alive) return;
    const generation = ++loadGeneration;
    const workspace = ws, id = docId, originVisit = visit;
    const owns = () => alive && generation === loadGeneration && visit === originVisit && ws === workspace && docId === id;
    pageCursor = before;
    pageTrail = trail;
    loading = true;
    try {
      const list = await listWorkbenchRevisions(workspace, id, { limit: PAGE_SIZE, before_seq: before });
      if (!owns()) return;
      revs = [...list].sort((a, b) => b.seq - a.seq).slice(0, PAGE_SIZE);
      error = null;
    } catch (e) {
      if (owns()) error = loadErrorText(e);
    } finally {
      if (owns()) loading = false;
    }
  }

  async function older(): Promise<void> {
    if (loading || revs.length < PAGE_SIZE) return;
    return load(revs[revs.length - 1].seq, [...pageTrail, pageCursor]);
  }

  async function newer(): Promise<void> {
    if (loading || pageTrail.length === 0) return;
    return load(pageTrail[pageTrail.length - 1], pageTrail.slice(0, -1));
  }

  function compareRevision(): void {
    const seq = Number(compareInput);
    if (!Number.isSafeInteger(seq) || seq < 1) {
      compareError = 'Enter a positive revision number.';
      return;
    }
    compareError = null;
    compareTo = seq;
  }

  // Track document visits before asynchronous completions; a save refreshes
  // the newest page without retaining previously rendered metadata pages.
  $effect.pre(() => {
    void ws;
    void docId;
    untrack(syncDocument);
  });
  $effect(() => {
    void ws;
    void docId;
    void rev;
    untrack(() => void load());
  });

  $effect(() => {
    const seq = selected;
    const id = docId;
    detail = null;
    detailError = null;
    if (seq == null) return;
    let gone = false;
    getWorkbenchRevision(ws, id, seq)
      .then((d) => {
        if (!gone) detail = d;
      })
      .catch((e) => {
        if (!gone) detailError = loadErrorText(e);
      });
    return () => {
      gone = true;
    };
  });

  $effect(() => {
    const seq = compareTo;
    const id = docId;
    other = null;
    compareError = null;
    if (seq === 'current') return;
    let gone = false;
    getWorkbenchRevision(ws, id, seq)
      .then((d) => {
        if (!gone) other = d;
      })
      .catch((e) => {
        if (!gone) compareError = loadErrorText(e);
      });
    return () => {
      gone = true;
    };
  });

  function sizeLabel(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / 1024 / 1024).toFixed(1)} MB`;
  }

  async function restore(): Promise<void> {
    syncDocument();
    if (selected == null || restoring || !alive) return;
    const seq = selected, workspace = ws, id = docId, originVisit = visit;
    const owns = () => alive && visit === originVisit && ws === workspace && docId === id;
    restoring = true;
    try {
      const ok = await confirmer.ask(
        `The file's content becomes revision ${seq}. Your current text is kept in history, so you can come back to it.`,
        { title: `Restore revision ${seq}?`, confirmLabel: 'Restore', danger: false },
      );
      if (!ok || !owns()) return;
      const doc = await restoreWorkbenchRevision(workspace, id, seq);
      if (!owns()) return;
      onrestored(doc);
      toasts.success(`Restored revision ${seq}`, 'Saved as a new revision.');
      selected = null;
      await load();
    } catch (e) {
      if (owns()) toasts.error("Couldn't restore", loadErrorText(e));
    } finally {
      if (owns()) restoring = false;
    }
  }

  // Diff sides: the older revision on the left, newer on the right.
  const diffSides = $derived.by(() => {
    if (!detail) return null;
    if (compareTo === 'current') {
      return { from: detail.seq, to: undefined as number | undefined, before: detail.content, after: current, fromLabel: `Rev ${detail.seq}`, toLabel: 'Current' };
    }
    if (!other) return null;
    const [a, b] = detail.seq < other.seq ? [detail, other] : [other, detail];
    return { from: a.seq, to: b.seq, before: a.content, after: b.content, fromLabel: `Rev ${a.seq}`, toLabel: `Rev ${b.seq}` };
  });
</script>

<aside class="wb-hist" aria-label="History" data-testid="wb-history">
  <header class="wb-side-head">
    <h3>History</h3>
    <button class="icon-btn" onclick={() => void load()} aria-label="Refresh history" title="Refresh history">
      <Icon name="refresh" size={12} />
    </button>
    <button class="icon-btn" onclick={close} aria-label="Close history" title="Close history">
      <Icon name="x" size={12} />
    </button>
  </header>
  <p class="wb-hist-note">Every save is kept. Autosaves within a minute fold into one revision.</p>

  <LoadState what="history" variant="compact" {loading} {error} empty={revs.length === 0} onretry={() => void load(pageCursor, pageTrail)}>
    {#snippet emptyView()}
      <p class="wb-hist-note">No revisions yet — start typing; the first save creates one.</p>
    {/snippet}
    <ol class="wb-revs" aria-label="Revisions">
      {#each revs as r (r.seq)}
        <li>
          <button
            class="wb-rev"
            class:sel={selected === r.seq}
            data-testid="wb-rev-row"
            data-seq={r.seq}
            aria-pressed={selected === r.seq}
            onclick={() => (selected = selected === r.seq ? null : r.seq)}
          >
            <span class="wb-rev-top">
              <span class="wb-rev-seq">#{r.seq}</span>
              <span class="wb-rev-kind k-{r.kind}">{KIND_LABEL[r.kind] ?? r.kind}{r.restored_from ? ` from #${r.restored_from}` : ''}</span>
              <span class="wb-rev-when"><RelTime iso={r.updated_at} /></span>
            </span>
            <span class="wb-rev-meta">
              {sizeLabel(r.size)}{#if r.saves > 1} · {r.saves} saves{/if}
            </span>
          </button>
        </li>
      {/each}
    </ol>
  </LoadState>

  <div class="wb-rev-actions" aria-label="History pages">
    <button class="btn small" disabled={loading || pageTrail.length === 0} onclick={newer}>Newer revisions</button>
    <button class="btn small" disabled={loading || revs.length < PAGE_SIZE} onclick={older}>Older revisions</button>
  </div>

  {#if selected != null}
    <section class="wb-rev-detail" aria-label={`Revision ${selected}`}>
      <div class="wb-rev-actions">
        <div class="seg" role="group" aria-label="Revision view">
          <button class:on={view === 'diff'} aria-pressed={view === 'diff'} onclick={() => (view = 'diff')}>Diff</button>
          <button class:on={view === 'text'} aria-pressed={view === 'text'} onclick={() => (view = 'text')}>Text</button>
        </div>
        {#if view === 'diff'}
          <label class="wb-cmp">
            <span>vs</span>
            <select bind:value={compareTo} aria-label="Compare with">
              <option value="current">Current</option>
              {#if compareTo !== 'current' && !revs.some((r) => r.seq === compareTo && r.seq !== selected)}
                <option value={compareTo}>Rev {compareTo}</option>
              {/if}
              {#each revs.filter((r) => r.seq !== selected) as r (r.seq)}
                <option value={r.seq}>Rev {r.seq}</option>
              {/each}
            </select>
          </label>
          <label class="wb-cmp">
            <span>Revision</span>
            <input type="text" inputmode="numeric" size="6" bind:value={compareInput} aria-label="Compare revision"
              onkeydown={(event) => { if (event.key === 'Enter') compareRevision(); }} />
          </label>
          <button class="btn small" onclick={compareRevision}>Compare</button>
        {/if}
        <button
          class="btn small"
          data-testid="wb-rev-restore"
          disabled={restoring || !detail}
          onclick={() => void restore()}
          title="Make this revision the current content (the current text stays in history)"
        >
          <Icon name="undo" size={12} /> Restore this version
        </button>
      </div>
      {#if compareError}
        <p class="wb-err" role="alert">Couldn't compare revision: {compareError}</p>
      {/if}
      {#if detailError}
        <p class="wb-err" role="alert">Couldn't load revision {selected}: {detailError}
          <button class="btn small" onclick={() => { const s = selected; selected = null; queueMicrotask(() => (selected = s)); }}>Retry</button>
        </p>
      {:else if !detail}
        <p class="wb-hist-note">Loading revision…</p>
      {:else if view === 'text'}
        <pre class="wb-rev-text" data-testid="wb-rev-text">{detail.content}</pre>
      {:else if diffSides}
        <RevisionDiff {ws} {docId} {...diffSides} />
      {:else}
        <p class="wb-hist-note">Loading revision…</p>
      {/if}
    </section>
  {/if}
</aside>

<style>
  .wb-hist {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-height: 0;
    height: 100%;
    overflow-y: auto;
    padding: 8px 10px;
    box-sizing: border-box;
  }
  .wb-side-head {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .wb-side-head h3 {
    flex: 1;
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .wb-hist-note {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .wb-revs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .wb-rev {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: 6px 8px;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    text-align: start;
    cursor: pointer;
  }
  .wb-rev:hover {
    background: var(--hover);
  }
  .wb-rev.sel {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .wb-rev:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .wb-rev-top {
    display: flex;
    gap: 6px;
    align-items: baseline;
    font-size: var(--fs-s);
  }
  .wb-rev-seq {
    font-family: var(--font-mono);
    color: var(--text-dim);
  }
  .wb-rev-kind {
    font-weight: 600;
  }
  .k-restore {
    color: var(--info);
  }
  .k-checkpoint {
    color: var(--success);
  }
  .wb-rev-when {
    margin-inline-start: auto;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .wb-rev-meta {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .wb-rev-detail {
    display: flex;
    flex-direction: column;
    gap: 6px;
    border-block-start: 1px solid var(--border);
    padding-block-start: 8px;
  }
  .wb-rev-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
  .seg {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .seg button {
    border: 0;
    padding: 3px 10px;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .seg button.on {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .wb-cmp {
    display: inline-flex;
    gap: 4px;
    align-items: center;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .wb-cmp select {
    font-size: var(--fs-xs);
  }
  .wb-rev-text {
    margin: 0;
    padding: 8px;
    max-height: 50vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .wb-err {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-s);
  }
</style>
