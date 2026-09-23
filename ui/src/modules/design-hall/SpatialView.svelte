<script lang="ts">
  // Spatial Hall (beta) — a showcase view, not the way to browse: one gallery
  // wall with a bay per project (framed thumbnails, 3D work on plinths). Pure
  // CSS in Phase 0 (the three.js room is v3). Click-to-open only; the bay chips
  // jump between bays; "Back to grid" is always there.
  import { untrack } from 'svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { router } from '../../lib/router.svelte';
  import { lsGet, lsSet } from '../../lib/storage';
  import type { DesignArtifact } from '../../lib/api/types';
  import ArtifactThumb from './ArtifactThumb.svelte';
  import { library } from './library.svelte';

  $effect(() => {
    untrack(() => {
      if (!library.loaded) void library.load();
    });
  });

  const BANNER_KEY = 'otto.design.spatialBanner';
  let bannerOpen = $state(lsGet(BANNER_KEY) !== 'dismissed');
  function dismissBanner(): void {
    bannerOpen = false;
    lsSet(BANNER_KEY, 'dismissed');
  }

  interface Bay {
    id: string;
    name: string;
    sub: string;
    flat: DesignArtifact[];
    plinth: DesignArtifact | null;
  }
  const bays = $derived.by((): Bay[] => {
    const live = library.artifacts.filter((a) => a.status !== 'archived');
    const out: Bay[] = [];
    for (const p of library.projects.filter((x) => !x.archived)) {
      const mine = live
        .filter((a) => a.project_id === p.id)
        .sort((a, b) => b.updated_at.localeCompare(a.updated_at));
      if (!mine.length) continue;
      const plinth = mine.find((a) => a.studio === '3d') ?? null;
      out.push({
        id: p.id,
        name: p.name,
        sub: `${mine.length} artifact${mine.length === 1 ? '' : 's'}`,
        flat: mine.filter((a) => a !== plinth).slice(0, 4),
        plinth,
      });
    }
    const unfiled = live.filter((a) => !a.project_id).sort((a, b) => b.updated_at.localeCompare(a.updated_at));
    if (unfiled.length) {
      const plinth = unfiled.find((a) => a.studio === '3d') ?? null;
      out.push({ id: '_unfiled', name: 'Unfiled', sub: `${unfiled.length} artifacts`, flat: unfiled.filter((a) => a !== plinth).slice(0, 4), plinth });
    }
    return out;
  });

  function jump(id: string): void {
    const el = document.getElementById(`bay-${id}`);
    const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    el?.scrollIntoView({ behavior: reduce ? 'auto' : 'smooth', inline: 'center', block: 'nearest' });
    el?.focus({ preventScroll: true });
  }

  const open = (a: DesignArtifact) => router.go(`design/a/${encodeURIComponent(a.id)}`);
</script>

<PageBody fill padded={false}>
  <div class="room" data-testid="design-spatial">
    {#if bannerOpen}
      <div class="banner" role="note">
        <Icon name="info" size={14} />
        <span>Spatial Hall is for showcases and reviews. The grid is still the fastest way to find things.</span>
        <button class="icon-btn" onclick={dismissBanner} aria-label="Dismiss" title="Dismiss"><Icon name="x" size={14} /></button>
      </div>
    {/if}
    {#if library.loading && !library.loaded}
      <div class="pad"><Skeleton rows={2} height={180} /></div>
    {:else if library.error && !library.loaded}
      <div class="pad err" role="alert">
        <Icon name="warning" size={14} /> Couldn’t load the design library.
        <button class="btn small" onclick={() => void library.load()}>Retry</button>
      </div>
    {:else if bays.length === 0}
      <EmptyState variant="page" icon="gallery" title="Nothing to exhibit yet"
        body="Projects with designs become bays on this wall. File a design into a project to see it here." />
    {:else}
      <div class="wall" role="list" aria-label="Project bays">
        {#each bays as b (b.id)}
          <div class="bay" id={`bay-${b.id}`} role="listitem" tabindex="-1" aria-label={b.name}>
            <div class="spot" aria-hidden="true"></div>
            <div class="works">
              {#each b.flat as a (a.id)}
                <button class="frame" onclick={() => open(a)} title={`Open ${a.title}`}>
                  <span class="canvas"><ArtifactThumb artifact={a} live /></span>
                  <span class="sr">{a.title}</span>
                </button>
              {/each}
            </div>
            {#if b.plinth}
              <button class="plinth" onclick={() => open(b.plinth!)} title={`Open ${b.plinth.title}`}>
                <span class="obj"><ArtifactThumb artifact={b.plinth} /></span>
                <span class="base" aria-hidden="true"></span>
                <span class="sr">{b.plinth.title}</span>
              </button>
            {/if}
            <div class="plaque">
              <strong>{b.name}</strong>
              <span>{b.sub}</span>
            </div>
          </div>
        {/each}
      </div>
      <div class="floor" aria-hidden="true"></div>
      <nav class="dock" aria-label="Bays">
        {#each bays as b (b.id)}
          <button class="btn ghost small" onclick={() => jump(b.id)}>{b.name}</button>
        {/each}
        <button class="btn small" onclick={() => router.go('design')}><Icon name="grid" size={12} /> Back to grid</button>
      </nav>
    {/if}
  </div>
</PageBody>

<style>
  .room {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background:
      radial-gradient(ellipse at 50% 0%, color-mix(in srgb, var(--text) 5%, transparent), transparent 60%),
      var(--bg);
  }
  .banner {
    position: absolute;
    inset-block-start: 14px;
    inset-inline: 0;
    margin-inline: auto;
    width: fit-content;
    max-width: calc(100% - 32px);
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 6px 6px 12px;
    border-radius: 999px;
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
    font-size: var(--fs-s);
  }
  .banner > :global(svg) {
    color: var(--info);
  }
  .pad {
    padding: 24px;
  }
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .wall {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 72px;
    padding: 80px 96px 40px;
    overflow-x: auto;
    scroll-snap-type: x proximity;
    background: linear-gradient(color-mix(in srgb, var(--text) 3%, var(--surface)) 0 80%, color-mix(in srgb, var(--text) 9%, var(--bg)) 80%);
    border-block-end: 1px solid var(--border);
  }
  .bay {
    position: relative;
    flex: none;
    display: flex;
    align-items: flex-end;
    gap: 18px;
    padding: 28px 24px 48px;
    scroll-snap-align: center;
    border-radius: var(--radius-l);
  }
  .bay:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent);
  }
  .spot {
    position: absolute;
    inset: -40px -40px 20px;
    background: radial-gradient(ellipse at 50% 0%, color-mix(in srgb, var(--warning) 10%, transparent), transparent 70%);
    pointer-events: none;
  }
  .works {
    position: relative;
    display: grid;
    grid-template-columns: repeat(2, 176px);
    gap: 14px;
  }
  .frame {
    position: relative;
    padding: 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: var(--border-strong);
    box-shadow: 0 6px 16px rgba(0, 0, 0, 0.18);
    cursor: pointer;
    transition: transform 140ms ease-out;
  }
  .frame:hover {
    transform: translateY(-2px);
  }
  .canvas {
    display: block;
    width: 164px;
    height: 110px;
    overflow: hidden;
    border-radius: 2px;
  }
  .plinth {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    border: 0;
    background: none;
    padding: 0;
    cursor: pointer;
  }
  .obj {
    display: block;
    width: 96px;
    height: 72px;
    border-radius: var(--radius-s);
    overflow: hidden;
    box-shadow: 0 8px 18px rgba(0, 0, 0, 0.2);
    transform: perspective(300px) rotateY(-14deg);
  }
  .base {
    display: block;
    width: 84px;
    height: 90px;
    margin-block-start: 8px;
    border-radius: 42px / 10px;
    background: linear-gradient(var(--surface-3), var(--surface-2));
    border: 1px solid var(--border);
  }
  .plaque {
    position: absolute;
    inset-block-end: 8px;
    inset-inline-start: 24px;
    display: flex;
    flex-direction: column;
    padding: 4px 10px;
    border-radius: var(--radius-s);
    background: var(--surface);
    border: 1px solid var(--border);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .plaque strong {
    color: var(--text);
    font-size: var(--fs-s);
  }
  .floor {
    height: 72px;
    flex: none;
    background: linear-gradient(var(--surface-2), var(--bg));
  }
  .dock {
    position: absolute;
    inset-block-end: 16px;
    inset-inline: 0;
    margin-inline: auto;
    width: fit-content;
    max-width: calc(100% - 32px);
    overflow-x: auto;
    display: flex;
    gap: 4px;
    padding: 4px;
    border-radius: var(--radius-l);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  @media (prefers-reduced-motion: reduce) {
    .frame {
      transition: none;
    }
    .frame:hover {
      transform: none;
    }
  }
</style>
