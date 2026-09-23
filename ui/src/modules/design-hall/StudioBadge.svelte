<script lang="ts">
  // A studio's identity: a small filled tile (studio colour + glyph) and,
  // optionally, its name. The colour is decoration — the name or the tile's
  // `title` always says which studio it is.
  import Icon, { asIcon } from '../../lib/components/Icon.svelte';
  import { studioColorVar, studioInfo } from './model';

  interface Props {
    studio: string;
    /** Tile edge: 16 (inline rows), 20 (cards), 28 (studio tiles). */
    size?: 16 | 20 | 28;
    showName?: boolean;
  }
  let { studio, size = 16, showName = false }: Props = $props();
  const info = $derived(studioInfo(studio));
  const glyph = $derived(size >= 28 ? 16 : 12);
</script>

<span class="studio-badge" title={showName ? undefined : info.name}>
  <span class="tile" style:--tile={`${size}px`} style:background={studioColorVar(studio)} aria-hidden="true">
    <Icon name={asIcon(info.icon, 'box')} size={glyph} />
  </span>
  {#if showName}<span class="name">{info.name}</span>{:else}<span class="sr-only">{info.name}</span>{/if}
</span>

<style>
  .studio-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .tile {
    --tile: 16px;
    width: var(--tile);
    height: var(--tile);
    flex: none;
    display: inline-grid;
    place-items: center;
    border-radius: var(--radius-s);
    color: var(--studio-glyph);
  }
  .name {
    font-size: var(--fs-s);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
