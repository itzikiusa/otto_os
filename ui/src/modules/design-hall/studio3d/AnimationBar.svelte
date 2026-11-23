<script lang="ts">
  // Compact Skeleton overlay + Animation Clip player bar shown over the 3D
  // viewport whenever a loaded GLB/glTF asset carries bones or animation clips.
  import Icon from '../../../lib/components/Icon.svelte';
  import { plural } from '../../../lib/plural';
  import { formatAnimDuration, type RigInfo } from '../../../lib/design/scene3d';

  interface Props {
    rig: RigInfo;
    showSkeleton: boolean;
    clipIndex: number;
    playing: boolean;
    onskeleton: (on: boolean) => void;
    onclip: (index: number) => void;
    onplay: (on: boolean) => void;
  }
  let { rig, showSkeleton, clipIndex, playing, onskeleton, onclip, onplay }: Props = $props();
  /** A short name and the length, so the closed select never cuts mid-character (full name in the title). */
  const shortName = (n: string): string => (n.length > 12 ? `${n.slice(0, 11)}…` : n);
</script>

{#if rig.hasSkeleton || rig.clips.length > 0}
  <div class="ab" role="group" aria-label="Rig and animation controls" data-testid="s3d-animation-bar">
    {#if rig.hasSkeleton}
      <button
        type="button"
        class="btn small"
        class:on={showSkeleton}
        aria-pressed={showSkeleton}
        title="Toggle 3D skeleton bone overlay"
        onclick={() => onskeleton(!showSkeleton)}
        data-testid="s3d-skeleton-toggle"
      >
        <Icon name="layers" size={12} /> Skeleton ({plural(rig.boneCount, 'bone')})
      </button>
    {/if}

    {#if rig.clips.length > 0}
      <button
        type="button"
        class="btn small ab-play"
        aria-label={playing ? 'Pause animation' : 'Play animation'}
        title={playing ? 'Pause animation' : 'Play animation'}
        onclick={() => onplay(!playing)}
        data-testid="s3d-anim-play"
      >
        <Icon name={playing ? 'pause' : 'play'} size={16} />
      </button>
      <span class="ab-sel">
      <select
        class="ab-select"
        aria-label="Animation clip"
        value={clipIndex}
        onchange={(e) => onclip(Number((e.currentTarget as HTMLSelectElement).value))}
        data-testid="s3d-anim-clip"
      >
        {#each rig.clips as c (c.index)}
          <option value={c.index} title={c.name}>{shortName(c.name)} · {formatAnimDuration(c.duration)}</option>
        {/each}
      </select>
      <Icon name="chevronDown" size={14} />
      </span>
    {/if}
  </div>
{/if}

<style>
  .ab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border-radius: var(--radius-m);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-card);
    pointer-events: auto;
  }
  .on {
    background: var(--accent-soft);
    border-color: var(--accent-line);
    color: var(--accent-text);
  }
  /* Our own chevron: iOS draws a native select at its own height whatever min-height says. */
  .ab-sel {
    position: relative;
    display: inline-flex;
    align-items: center;
    min-inline-size: 0;
  }
  .ab-sel :global(svg) {
    position: absolute;
    inset-inline-end: 6px;
    pointer-events: none;
    color: var(--text-dim);
  }
  .ab-select {
    appearance: none;
    font: inherit;
    font-size: var(--fs-xs);
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 4px 6px;
    padding-inline-end: 24px;
    max-inline-size: 200px;
    text-overflow: ellipsis;
  }
  @media (pointer: coarse) {
    .ab :global(.btn),
    .ab-select {
      min-block-size: 40px;
    }
    .ab-play {
      min-inline-size: 40px;
    }
    .ab-select {
      block-size: 40px;
      font-size: 16px; /* ui-guards: allow */
    }
  }
</style>
