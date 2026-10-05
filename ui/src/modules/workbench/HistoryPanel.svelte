<script lang="ts">
  // History: the doc's full revision timeline (append-only — only "Delete
  // forever" from the trash removes it). Pick a revision to see it, diff it
  // against the current text or another revision, and restore it (restoring
  // adds a new revision; the current text stays in history).
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
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

  const KIND_LABEL: Record<string, string> = {
    create: 'Created',
    auto: 'Autosave',
    checkpoint: 'Saved',
    restore: 'Restored',
  };

  async function load(): Promise<void> {
    loading = true;
    try {
      const list = await listWorkbenchRevisions(ws, docId);
      revs = [...list].sort((a, b) => b.seq - a.seq);
      error = null;
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  // Reload on doc switch and whenever a save moved the head revision.
  $effect(() => {
    void docId;
    void rev;
    void load();
  });

  // Reset the selection on doc switch.
  $effect(() => {
    void docId;
    selected = null;
    compareTo = 'current';
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
    if (seq === 'current') return;
    let gone = false;
    getWorkbenchRevision(ws, id, seq)
      .then((d) => {
        if (!gone) other = d;
      })
      .catch(() => {
        /* surfaced by the empty diff side */
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
    if (selected == null) return;
    const ok = await confirmer.ask(
      `The file’s content becomes revision ${selected}. Your current text is kept in history, so you can come back to it.`,
      { title: `Restore revision ${selected}?`, confirmLabel: 'Restore', danger: false },
    );
    if (!ok) return;
    restoring = true;
    try {
      const doc = await restoreWorkbenchRevision(ws, docId, selected);
      onrestored(doc);
      toasts.success(`Restored revision ${selected}`, 'Saved as a new revision.');
      selected = null;
      await load();
    } catch (e) {
      toasts.error("Couldn’t restore", loadErrorText(e));
    } finally {
      restoring = false;
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
    <button class="icon-btn" onclick={onclose} aria-label="Close history" title="Close history">
      <Icon name="x" size={12} />
    </button>
  </header>
  <p class="wb-hist-note">Every save is kept. Autosaves within a minute fold into one revision.</p>

  <LoadState what="history" variant="compact" {loading} {error} empty={revs.length === 0} onretry={() => void load()}>
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
              {#each revs.filter((r) => r.seq !== selected) as r (r.seq)}
                <option value={r.seq}>Rev {r.seq}</option>
              {/each}
            </select>
          </label>
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
      {#if detailError}
        <p class="wb-err" role="alert">Couldn’t load revision {selected}: {detailError}
          <button class="btn small" onclick={() => { const s = selected; selected = null; queueMicrotask(() => (selected = s)); }}>Retry</button>
        </p>
      {:else if !detail}
        <Skeleton rows={6} height={14} label="the revision" />
      {:else if view === 'text'}
        <pre class="wb-rev-text" data-testid="wb-rev-text">{detail.content}</pre>
      {:else if diffSides}
        <RevisionDiff {ws} {docId} {...diffSides} />
      {:else}
        <Skeleton rows={6} height={14} label="the revision" />
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
    padding: 2px 10px;
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
