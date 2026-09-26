<script module lang="ts">
  import type { DesignArtifact as DA } from '../../lib/api/types';
  export interface CompareSide {
    artifact: DA;
    versionId: string;
  }
</script>

<script lang="ts">
  import { onTabKey } from './tabKeys';
  // Compare two versions — of this artifact, or this artifact against a
  // reference from the library. Side by side renders both with the same stage
  // the editor uses; Changes is a text diff of the sources (JSON pretty-printed
  // first). Restore puts the left version's content back as a NEW version
  // (the view confirms and saves) — history never rewinds.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import DiffView from '../../lib/components/DiffView.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { fetchContent, listVersions } from '../../lib/api/design';
  import type { DesignArtifact, DesignVersion } from '../../lib/api/types';
  import ArtifactStage from './ArtifactStage.svelte';
  import { isJsonFormat, isTextFormat, versionAuthor } from './model';

  interface Props {
    left: CompareSide;
    right: CompareSide;
    /** The artifact open in the view — only its versions can be restored. */
    currentId: string;
    headId: string | null;
    meId: string | null | undefined;
    onclose: () => void;
    onrestore: (versionId: string, seq: number) => void;
  }
  let { left: initialLeft, right: initialRight, currentId, headId, meId, onclose, onrestore }: Props = $props();

  let left = $state<CompareSide>(untrack(() => initialLeft));
  let right = $state<CompareSide>(untrack(() => initialRight));
  let mode = $state<'side' | 'changes'>('side');
  let innerH = $state(800);

  // Version lists per artifact (for the pickers), fetched once each.
  let versionsOf = $state<Record<string, DesignVersion[]>>({});
  $effect(() => {
    for (const id of [left.artifact.id, right.artifact.id]) {
      if (untrack(() => versionsOf[id])) continue;
      void listVersions(id).then(
        (v) => (versionsOf = { ...versionsOf, [id]: v }),
        () => (versionsOf = { ...versionsOf, [id]: [] }),
      );
    }
  });

  interface Loaded {
    key: string;
    text: string | null;
    blobUrl: string | null;
    error: string | null;
  }
  let loadedL = $state<Loaded | null>(null);
  let loadedR = $state<Loaded | null>(null);

  function loader(side: () => CompareSide, set: (l: Loaded) => void, get: () => Loaded | null) {
    return () => {
      const s = side();
      const key = `${s.artifact.id}:${s.versionId}`;
      if (untrack(get)?.key === key) return;
      const asText = isTextFormat(s.artifact.format);
      let cancelled = false;
      void fetchContent(s.artifact.id, { version: s.versionId, asText }).then(
        (c) => {
          if (cancelled) {
            if (c.blobUrl) URL.revokeObjectURL(c.blobUrl);
            return;
          }
          const prev = untrack(get);
          if (prev?.blobUrl) URL.revokeObjectURL(prev.blobUrl);
          set({ key, text: c.text, blobUrl: c.blobUrl, error: null });
        },
        (e) => {
          if (!cancelled) set({ key, text: null, blobUrl: null, error: e instanceof Error ? e.message : String(e) });
        },
      );
      return () => {
        cancelled = true;
      };
    };
  }
  $effect(loader(() => left, (l) => (loadedL = l), () => loadedL));
  $effect(loader(() => right, (l) => (loadedR = l), () => loadedR));
  $effect(() => () => {
    if (loadedL?.blobUrl) URL.revokeObjectURL(loadedL.blobUrl);
    if (loadedR?.blobUrl) URL.revokeObjectURL(loadedR.blobUrl);
  });

  const textBoth = $derived(isTextFormat(left.artifact.format) && isTextFormat(right.artifact.format));
  function pretty(format: string, t: string | null): string {
    if (t == null) return '';
    if (!isJsonFormat(format)) return t;
    try {
      return JSON.stringify(JSON.parse(t), null, 2);
    } catch {
      return t;
    }
  }

  function seqOf(side: CompareSide): number | null {
    return versionsOf[side.artifact.id]?.find((v) => v.id === side.versionId)?.seq ?? null;
  }
  function labelOf(side: CompareSide, v: DesignVersion): string {
    const cur = side.artifact.id === currentId && v.id === headId ? ', current' : '';
    return `v${v.seq} (${versionAuthor(v, meId)}${cur})`;
  }

  const canRestore = $derived(left.artifact.id === currentId && left.versionId !== headId && seqOf(left) != null);
  const paneH = $derived(Math.max(260, Math.round(innerH * 0.58)));
</script>

<svelte:window bind:innerHeight={innerH} />

<Modal title="Compare versions" width={1180} {onclose}>
  <div class="cmp" data-testid="design-compare-modal">
    <div class="modes">
      <div class="segmented" role="tablist" aria-label="Compare mode">
        <button role="tab" aria-selected={mode === 'side'} tabindex={mode === 'side' ? 0 : -1} onkeydown={onTabKey} class:active={mode === 'side'} onclick={() => (mode = 'side')}>Side by side</button>
        <button role="tab" aria-selected={mode === 'changes'} tabindex={mode === 'changes' ? 0 : -1} onkeydown={onTabKey} class:active={mode === 'changes'} onclick={() => (mode = 'changes')}
          disabled={!textBoth} title={textBoth ? undefined : 'Changes compare text sources; images and models compare side by side'}>Changes</button>
      </div>
    </div>
    <div class="pickers">
      {#each [{ s: left, set: (v: string) => (left = { ...left, versionId: v }), n: 'Left' }, { s: right, set: (v: string) => (right = { ...right, versionId: v }), n: 'Right' }] as p (p.n)}
        <label class="picker">
          <span class="title" title={p.s.artifact.title}>{p.s.artifact.id === currentId ? '' : `${p.s.artifact.title} · `}</span>
          <select class="input" aria-label={`${p.n} version`} value={p.s.versionId} onchange={(e) => p.set((e.currentTarget as HTMLSelectElement).value)}>
            {#each versionsOf[p.s.artifact.id] ?? [] as v (v.id)}<option value={v.id}>{labelOf(p.s, v)}</option>{/each}
            {#if !(versionsOf[p.s.artifact.id] ?? []).length}<option value={p.s.versionId}>Loading…</option>{/if}
          </select>
        </label>
      {/each}
    </div>
    {#if mode === 'side' || !textBoth}
      <div class="panes" style:height={`${paneH}px`}>
        {#each [{ s: left, l: loadedL }, { s: right, l: loadedR }] as p, i (i)}
          <div class="pane">
            {#if !p.l}
              <p class="msg">Loading version…</p>
            {:else if p.l.error}
              <p class="msg err"><Icon name="warning" size={14} /> Couldn’t load this version. {p.l.error}</p>
            {:else}
              {#key p.l.key}
                <ArtifactStage artifact={p.s.artifact} source={p.l.text} blobUrl={p.l.blobUrl} readonly compact />
              {/key}
            {/if}
          </div>
        {/each}
      </div>
    {:else}
      <div class="diff" style:height={`${paneH}px`}>
        {#if loadedL && loadedR && !loadedL.error && !loadedR.error}
          <DiffView before={pretty(left.artifact.format, loadedL.text)} after={pretty(right.artifact.format, loadedR.text)} mode="split" contextLines={4} />
        {:else}
          <p class="msg">Loading versions…</p>
        {/if}
      </div>
    {/if}
  </div>
  {#snippet footer()}
    {#if canRestore}
      <button class="btn" onclick={() => onrestore(left.versionId, seqOf(left)!)} data-testid="design-restore">
        <Icon name="refresh" size={13} /> Restore v{seqOf(left)}
      </button>
    {/if}
    <button class="btn primary" onclick={onclose}>Done</button>
  {/snippet}
</Modal>

<style>
  .cmp {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .modes {
    display: flex;
    justify-content: center;
  }
  .pickers {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .picker {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .picker .title {
    font-size: var(--fs-s);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 50%;
  }
  .picker select {
    width: auto;
    max-width: 100%;
  }
  .panes {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .pane,
  .diff {
    min-width: 0;
    min-height: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: auto;
    background: var(--surface-2);
    container-type: inline-size;
  }
  .msg {
    margin: 16px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .msg.err {
    color: var(--danger);
    display: flex;
    gap: 6px;
  }
  @media (max-width: 640px) {
    .pickers,
    .panes {
      grid-template-columns: 1fr;
    }
  }
</style>
