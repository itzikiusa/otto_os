<script lang="ts">
  // Snapshots (R1) + per-snapshot report export (R9). Lists a pack's immutable,
  // content-hashed snapshots (seq / status / done score / sha256 tamper key /
  // note / time), a "Snapshot" button to freeze the current state, and a
  // .md / .html download for each frozen report.
  import Icon from './Icon.svelte';
  import ProofStatusChip from './ProofStatusChip.svelte';
  import { createSnapshot, getSnapshot } from '../api/proof';
  import { downloadText } from './exporters';
  import { toasts } from '../toast.svelte';
  import type { ProofSnapshotMeta } from '../api/types';

  interface Props {
    packId: string;
    snapshots: ProofSnapshotMeta[];
    onchange: () => void | Promise<void>;
  }
  let { packId, snapshots, onchange }: Props = $props();

  let busy = $state(false);
  let note = $state('');

  async function snap(): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      await createSnapshot(packId, { note: note.trim() || undefined });
      note = '';
      await onchange();
      toasts.success('Snapshot created', 'An immutable, hashed copy was frozen.');
    } catch (e) {
      toasts.error('Snapshot failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  async function download(s: ProofSnapshotMeta, format: 'md' | 'html'): Promise<void> {
    try {
      const full = await getSnapshot(s.id);
      const text = format === 'md' ? full.report_md : full.report_html;
      downloadText(
        text,
        `proof-snapshot-${s.seq}.${format}`,
        format === 'md' ? 'text/markdown' : 'text/html',
      );
    } catch (e) {
      toasts.error('Download failed', e instanceof Error ? e.message : String(e));
    }
  }

  function shortSha(sha: string): string {
    return sha ? sha.slice(0, 12) : '—';
  }

  function fmtTime(iso: string): string {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
  }
</script>

<section class="snaps">
  <div class="snaps-head">
    <h3 class="group-title">Snapshots <span class="dim">· {snapshots.length}</span></h3>
    <div class="snap-new">
      <input
        class="input note"
        bind:value={note}
        placeholder="Note (optional)"
        aria-label="Snapshot note"
      />
      <button class="btn small" onclick={snap} disabled={busy}>
        <Icon name="archive" size={12} /> {busy ? 'Snapshotting…' : 'Snapshot'}
      </button>
    </div>
  </div>

  {#if snapshots.length === 0}
    <p class="dim empty">No snapshots yet — freeze a tamper-evident copy of the current evidence.</p>
  {:else}
    <div class="snap-list">
      {#each snapshots as s (s.id)}
        <div class="snap-row">
          <span class="seq">#{s.seq}</span>
          <ProofStatusChip status={s.status} risk={s.risk_score} />
          <span class="score" title="Done score">{s.done_score}/100</span>
          <span class="sha" title={s.sha256}>sha:{shortSha(s.sha256)}</span>
          {#if s.note}<span class="note-text ellipsis" title={s.note}>{s.note}</span>{/if}
          <span class="grow"></span>
          <span class="time">{fmtTime(s.created_at)}</span>
          <button class="link-btn" onclick={() => download(s, 'md')}>.md</button>
          <button class="link-btn" onclick={() => download(s, 'html')}>.html</button>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .snaps {
    margin-bottom: 16px;
  }
  .snaps-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-wrap: wrap;
    margin-bottom: 8px;
  }
  .group-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    margin: 0;
  }
  .snap-new {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .note {
    height: 26px;
    font-size: 12px;
    width: 200px;
    max-width: 48vw;
  }
  .dim {
    color: var(--text-dim);
    font-size: 11px;
  }
  .empty {
    margin: 0;
    font-size: 12px;
  }
  .snap-list {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .snap-row {
    display: flex;
    align-items: center;
    gap: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 10px;
    background: var(--surface);
    font-size: 12px;
  }
  .seq {
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    font-weight: 600;
  }
  .score {
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
  }
  .sha {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 10.5px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    border-radius: var(--radius-s);
    padding: 1px 6px;
    white-space: nowrap;
  }
  .note-text {
    color: var(--text);
    min-width: 0;
    max-width: 200px;
  }
  .time {
    color: var(--text-dim);
    font-size: 11px;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .link-btn {
    border: none;
    background: transparent;
    color: var(--accent);
    cursor: pointer;
    font-size: 11px;
    padding: 0 2px;
    white-space: nowrap;
  }
  .link-btn:hover {
    text-decoration: underline;
  }
</style>
