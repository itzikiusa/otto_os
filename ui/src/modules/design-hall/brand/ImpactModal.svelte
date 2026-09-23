<script lang="ts">
  // The impact preview (proposal §3.3): before a token change is saved, show
  // what changes, which designs see it and when they move (follow approved →
  // on approve; follow latest → on save; pinned → never until updated), plus
  // contrast warnings for the proposed colours. Nothing is saved from here
  // unless the person presses "Save as vN".
  import type { BrandConsumer, BrandImpactResp, BrandTokenChange } from '../../../lib/api/types';
  import Modal from '../../../lib/components/Modal.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import StudioBadge from '../StudioBadge.svelte';
  import { openArtifact } from '../nav';
  import { studioInfo } from '../model';
  import { normalizeHex } from './tokens';

  interface Props {
    impact: BrandImpactResp | null;
    loading: boolean;
    error: string | null;
    nextSeq: number;
    approvedSeq: number | null;
    canSave: boolean;
    saving: boolean;
    onclose: () => void;
    onsave: () => void;
    onretry: () => void;
  }
  let { impact, loading, error, nextSeq, approvedSeq, canSave, saving, onclose, onsave, onretry }: Props = $props();

  const affected = $derived(impact?.consumers.filter((c) => c.affected.length > 0) ?? []);
  const untouched = $derived(impact ? impact.artifact_count - affected.length : 0);

  function when(c: BrandConsumer): { text: string; tone: string } {
    if (c.policy === 'pinned') return { text: 'pinned · stays until updated', tone: 'warn' };
    if (c.policy === 'follow_latest') return { text: 'follows latest · changes on save', tone: 'info' };
    return { text: `follows approved · changes when you approve v${nextSeq}`, tone: 'accent' };
  }
  function isColor(ch: BrandTokenChange): boolean {
    return ch.token.startsWith('color.');
  }
</script>

<Modal title="Impact of your changes" width={640} {onclose}>
  <div class="imp" data-testid="brand-impact-modal">
    {#if loading && !impact}
      <Skeleton rows={4} height={28} />
    {:else if error && !impact}
      <p class="err" role="alert"><Icon name="warning" size={14} /> Couldn’t preview the impact. <span class="dim">{error}</span> <button class="btn small" onclick={onretry}>Retry</button></p>
    {:else if impact}
      <p class="lead">
        {#if impact.changes.length === 0}
          No token changes compared with {impact.base === 'approved' ? `the approved v${impact.base_seq}` : impact.base === 'head' ? `v${impact.base_seq}` : 'the empty kit'}.
        {:else}
          {impact.changes.length} token {impact.changes.length === 1 ? 'change' : 'changes'} compared with
          {impact.base === 'approved' ? `the approved v${impact.base_seq}` : impact.base === 'head' ? `v${impact.base_seq} (nothing approved yet)` : 'the empty kit'}
          reach <b>{impact.affected_count} {impact.affected_count === 1 ? 'design' : 'designs'}</b> in
          <b>{impact.affected_studio_count} {impact.affected_studio_count === 1 ? 'studio' : 'studios'}</b>.
        {/if}
      </p>

      {#if impact.changes.length}
        <ul class="changes" aria-label="Token changes">
          {#each impact.changes as ch (ch.token)}
            <li>
              <span class="mono tok">{ch.token}</span>
              <span class="kind {ch.change}">{ch.change}</span>
              <span class="vals mono">
                {#if isColor(ch) && ch.before && normalizeHex(ch.before)}<i class="sw" style:background={ch.before}></i>{/if}
                <span class:gone={!ch.before}>{ch.before ?? '—'}</span>
                <Icon name="chevronRight" size={12} />
                {#if isColor(ch) && ch.after && normalizeHex(ch.after)}<i class="sw" style:background={ch.after}></i>{/if}
                <span class:gone={!ch.after}>{ch.after ?? 'removed'}</span>
              </span>
            </li>
          {/each}
        </ul>
      {/if}

      {#if impact.affected_by_studio.length}
        <div class="studios">
          {#each impact.affected_by_studio as s (s.studio)}
            <span class="chip"><StudioBadge studio={s.studio} /> {studioInfo(s.studio).name} · {s.count}</span>
          {/each}
        </div>
      {/if}

      {#if affected.length}
        <ul class="rows" aria-label="Designs that change">
          {#each affected as c (c.artifact.id)}
            {@const w = when(c)}
            <li>
              <StudioBadge studio={c.artifact.studio} />
              <button class="linkish t" onclick={() => openArtifact(c.artifact.id)} title="Open {c.artifact.title}">{c.artifact.title}</button>
              <span class="pill {w.tone}">{w.text}</span>
              <span class="mono dim aff" title={c.affected.join(', ')}>{c.whole_kit ? 'whole kit' : c.affected.join(', ')}</span>
            </li>
          {/each}
        </ul>
      {/if}
      {#if untouched > 0}
        <p class="dim">{untouched} other {untouched === 1 ? 'design uses' : 'designs use'} this kit but none of the changed tokens.</p>
      {/if}
      {#if impact.hidden_count > 0}
        <p class="dim">{impact.hidden_count} more in workspaces you can’t see.</p>
      {/if}

      {#if impact.warnings.length}
        <div class="warns" role="note">
          {#each impact.warnings as w, i (i)}
            <p><Icon name="warning" size={13} /> {w}</p>
          {/each}
        </div>
      {/if}

      <p class="note">
        <Icon name="info" size={13} />
        Saving makes v{nextSeq}. Designs that follow the approved kit keep {approvedSeq ? `v${approvedSeq}` : 'the current tokens'} until you approve v{nextSeq}.
      </p>
    {/if}
  </div>

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Close</button>
    <button class="btn primary" onclick={onsave} disabled={!canSave || saving} data-testid="brand-impact-save">
      {saving ? 'Saving…' : `Save as v${nextSeq}`}
    </button>
  {/snippet}
</Modal>

<style>
  .imp {
    display: flex;
    flex-direction: column;
    gap: 12px;
    font-size: var(--fs-m);
  }
  .lead {
    margin: 0;
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
    margin: 0;
  }
  .changes,
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    max-height: 30vh;
    overflow-y: auto;
  }
  .changes li,
  .rows li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px;
    min-width: 0;
  }
  .changes li + li,
  .rows li + li {
    border-block-start: 1px solid var(--border);
  }
  .tok {
    font-size: var(--fs-s);
    min-width: 120px;
  }
  .kind {
    font-size: var(--fs-xs);
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .kind.changed {
    background: var(--info-soft);
    color: var(--info);
  }
  .kind.added {
    background: var(--success-soft);
    color: var(--success);
  }
  .kind.removed {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .vals {
    margin-inline-start: auto;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .gone {
    color: var(--text-dim);
  }
  .sw {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1px solid var(--border);
    display: inline-block;
  }
  .studios {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .studios .chip {
    color: var(--text);
  }
  .t {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: start;
  }
  .linkish {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    color: var(--text);
    cursor: pointer;
  }
  .linkish:hover {
    color: var(--accent-text);
    text-decoration: underline;
  }
  .pill {
    font-size: var(--fs-xs);
    padding: 1px 8px;
    border-radius: 999px;
    white-space: nowrap;
  }
  .pill.warn {
    color: var(--warning);
    background: var(--warning-soft);
  }
  .pill.info {
    color: var(--info);
    background: var(--info-soft);
  }
  .pill.accent {
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  .aff {
    font-size: var(--fs-xs);
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .warns {
    border-radius: var(--radius-m);
    background: var(--warning-soft);
    padding: 8px 10px;
  }
  .warns p {
    margin: 0;
    display: flex;
    gap: 6px;
    align-items: flex-start;
    color: var(--warning);
    font-size: var(--fs-s);
  }
  .note,
  .err {
    margin: 0;
    display: flex;
    gap: 6px;
    align-items: flex-start;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .note > :global(svg) {
    color: var(--info);
    flex: none;
    margin-block-start: 2px;
  }
  .err {
    color: var(--text);
    align-items: center;
    flex-wrap: wrap;
  }
  .err > :global(svg) {
    color: var(--danger);
  }
  @media (max-width: 600px) {
    .aff {
      display: none;
    }
    .changes li,
    .rows li {
      flex-wrap: wrap;
    }
  }
</style>
