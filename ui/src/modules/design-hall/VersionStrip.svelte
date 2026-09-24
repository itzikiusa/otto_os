<script lang="ts">
  // Bottom version strip: every committed version, oldest → newest, as a chip
  // with its author (agents marked). Clicking chips selects up to two for
  // Compare; versions are the undo model, so nothing here ever deletes.
  import Icon from '../../lib/components/Icon.svelte';
  import type { DesignVersion } from '../../lib/api/types';
  import { stripOrder, versionAuthor } from './model';

  interface Props {
    versions: DesignVersion[];
    headId: string | null;
    approvedId: string | null;
    meId: string | null | undefined;
    selected: string[];
    loading?: boolean;
    error?: string | null;
    onselect: (id: string) => void;
    oncompare: () => void;
    onretry: () => void;
  }
  let { versions, headId, approvedId, meId, selected, loading = false, error = null, onselect, oncompare, onretry }: Props =
    $props();

  const ordered = $derived(stripOrder(versions));
  let scroller = $state<HTMLDivElement | null>(null);

  // Keep the newest version in view when the strip grows.
  $effect(() => {
    void ordered.length;
    const el = scroller;
    if (el) queueMicrotask(() => (el.scrollLeft = el.scrollWidth));
  });

  function tip(v: DesignVersion): string {
    const when = new Date(v.created_at).toLocaleString();
    const msg = v.message ? ` — ${v.message}` : '';
    return `v${v.seq} · ${versionAuthor(v, meId)} · ${when}${msg}`;
  }

  function onKey(e: KeyboardEvent, i: number): void {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    const btns = scroller?.querySelectorAll<HTMLButtonElement>('button.vchip');
    const j = Math.max(0, Math.min((btns?.length ?? 1) - 1, i + (e.key === 'ArrowRight' ? 1 : -1)));
    btns?.[j]?.focus();
  }

  const hint = $derived(
    selected.length === 0
      ? 'Select versions to compare'
      : selected.length === 1
        ? 'Compare with the current version, or pick one more'
        : 'Compare the two selected versions',
  );
</script>

<div class="strip" data-testid="design-version-strip">
  <span class="label"><Icon name="clock" size={14} /> Versions</span>
  <div class="chips" bind:this={scroller} role="group" aria-label="Versions">
    {#if loading && !versions.length}
      <span class="dim">Loading versions…</span>
    {:else if error}
      <span class="err"><Icon name="warning" size={12} /> Couldn’t load versions.</span>
      <button class="btn small ghost" onclick={onretry}>Retry</button>
    {:else}
      {#each ordered as v, i (v.id)}
        {@const author = versionAuthor(v, meId)}
        <button
          class="vchip"
          class:sel={selected.includes(v.id)}
          class:head={v.id === headId}
          aria-pressed={selected.includes(v.id)}
          title={tip(v)}
          onclick={() => onselect(v.id)}
          onkeydown={(e) => onKey(e, i)}
          data-testid="design-version-chip"
          data-seq={v.seq}
        >
          <span class="av" class:agent={v.author_kind === 'agent'} class:system={v.author_kind === 'system'} aria-hidden="true"></span>
          <span class="v">v{v.seq}</span>
          <span class="who">({author}{v.id === headId ? ', current' : ''})</span>
          {#if v.id === approvedId}<span class="appr" title="Approved version"><Icon name="check" size={12} /></span>{/if}
        </button>
      {/each}
    {/if}
  </div>
  <span class="hint">{hint}</span>
  <button class="btn small" onclick={oncompare} disabled={selected.length === 0 || versions.length < 2}
    data-testid="design-compare">
    <Icon name="columns" size={12} /> Compare
  </button>
</div>

<style>
  .strip {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 40px;
    padding: 0 12px;
    border-block-start: 1px solid var(--border);
    background: var(--bg);
    min-width: 0;
  }
  .label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
    flex: none;
  }
  .chips {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    overflow-x: auto;
    scrollbar-width: thin;
    padding-block: 4px;
  }
  .vchip {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 9px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
    transition: background 130ms ease-out, border-color 130ms ease-out;
  }
  .vchip:hover {
    background: var(--hover);
  }
  .vchip.head {
    border-color: var(--border-strong);
  }
  .vchip.sel {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 50%, transparent);
  }
  .av {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-dim);
    flex: none;
  }
  /* Agent-authored versions: a hollow ring (never the accent colour). */
  .av.agent {
    background: transparent;
    border: 2px solid var(--text);
  }
  .av.system {
    background: transparent;
    border: 1px dashed var(--text-dim);
  }
  .v {
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .who {
    color: var(--text-dim);
  }
  .appr {
    color: var(--success);
    display: inline-flex;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex: none;
  }
  .dim {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .err {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-s);
  }
  .err :global(svg) {
    color: var(--danger);
  }
  @container (max-width: 760px) {
    .hint {
      display: none;
    }
  }
</style>
