<script lang="ts">
  // A server-paged grid of design cards for one scope — the lobby's "All
  // designs", a project / studio / story page, and (next) the Product story
  // Design tab. Pages come from `GET /design/search?q=` by KEYSET cursor
  // (`updated_at|id`, newest first): never a client-side slice of the shared
  // 500-row library, so older designs are one "Load more" away, not cut off.
  // Live events PATCH the cards they name; only membership changes (create,
  // delete, links) reload. With `follow`, a card the shared library already
  // shows is patched FROM the library — the lobby must not GET it twice.
  import { onDestroy, untrack, type Snippet } from 'svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { toastError } from '../../lib/toastError';
  import { toasts } from '../../lib/toast.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { designBus } from '../../lib/events.svelte';
  import { getArtifact, search, updateArtifact } from '../../lib/api/design';
  import type { DesignArtifact, DesignSearchHit } from '../../lib/api/types';
  import ArtifactCard from './ArtifactCard.svelte';
  import { library } from './library.svelte';
  import { storyKeysFor } from './model';
  import {
    NO_FILTERS,
    activeFilterCount,
    cursorAfter,
    gridQuery,
    groupHits,
    matchesGrid,
    sameCard,
    type GridFilters,
    type GridScope,
    type GroupBy,
  } from './designGrid';

  interface Props {
    scope?: GridScope;
    filters?: GridFilters;
    groupBy?: GroupBy;
    /** Rows per page (the server caps a page at 500). */
    page?: number;
    /** Patch cards the shared library also shows from it (no second GET). */
    follow?: boolean;
    /** A project's workspace: events from other workspaces can't change it. */
    homeWorkspace?: string;
    /** Shown when the scope has no designs at all (no chip active). */
    empty?: Snippet;
    onclearfilters?: () => void;
    /** Loaded cards (after the client-side Mine chip) — for the caller's header. */
    count?: number;
    loaded?: boolean;
  }
  let {
    scope = {},
    filters = NO_FILTERS,
    groupBy = 'recent',
    page = 120,
    follow = false,
    homeWorkspace,
    empty,
    onclearfilters,
    count = $bindable(0),
    loaded = $bindable(false),
  }: Props = $props();

  let hits = $state.raw<DesignSearchHit[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let hasMore = $state(false);
  let seq = 0;
  let reloadTimer: ReturnType<typeof setTimeout> | undefined;
  const query = $derived(gridQuery(scope, filters));
  const queryKey = $derived(JSON.stringify(query));
  const meId = $derived(auth.me?.id ?? null);

  /** `append` fetches the next page after the last loaded row; otherwise
   *  reload everything shown so far (at least one page), so a resync never
   *  collapses what "Load more" already opened. */
  async function load(append = false): Promise<void> {
    const my = ++seq;
    const q = query;
    loading = true;
    const cursor = append ? cursorAfter(hits) : undefined;
    const limit = append ? page : Math.min(500, Math.max(page, hits.length));
    try {
      const rows = await search('', { ...q, limit, ...(cursor ? { cursor } : {}) });
      if (my !== seq) return;
      if (append) {
        const have = new Set(hits.map((h) => h.artifact.id));
        hits = [...hits, ...rows.filter((h) => !have.has(h.artifact.id))];
      } else {
        hits = rows;
      }
      hasMore = rows.length === limit;
      error = null;
      loaded = true;
    } catch (e) {
      if (my !== seq) return;
      error = loadErrorText(e);
    } finally {
      if (my === seq) loading = false;
    }
  }

  // A new scope / filter set starts over from the first page.
  $effect(() => {
    void queryKey;
    untrack(() => {
      clearTimeout(patchTimer);
      patchTimer = undefined;
      patchIds.clear();
      hits = [];
      loaded = false;
      error = null;
      void load();
    });
  });
  // A resync (WS reconnect: events may have been missed) reloads.
  $effect(() => {
    void designBus.resyncTick;
    untrack(() => {
      if (loaded) scheduleReload();
    });
    return () => clearTimeout(reloadTimer);
  });
  function scheduleReload(): void {
    clearTimeout(reloadTimer);
    reloadTimer = setTimeout(() => void load(), 600); // coalesce bursts
  }

  let seen = designBus.seq;
  const patchIds = new Set<string>();
  let patchTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const now = designBus.seq;
    untrack(() => {
      const evs = designBus.since(seen);
      seen = now;
      if (!loaded || !evs.length) return;
      const ids = new Set(hits.map((h) => h.artifact.id));
      let full = false;
      for (const ev of evs) {
        if (ev.type === 'design_learning_update') continue;
        if (homeWorkspace && ev.workspace_id !== homeWorkspace && !ids.has(ev.artifact_id)) continue;
        if (ev.type === 'design_artifact_updated' && ev.change !== 'created' && ev.change !== 'deleted') {
          // The library patches its own copy; `followLibrary` picks it up.
          if (ids.has(ev.artifact_id) && !(follow && library.hitOf(ev.artifact_id))) patchIds.add(ev.artifact_id);
          // A card this grid doesn't show can still move INTO it (a project
          // move, an archive / unarchive, a status change under a chip).
          else if (!ids.has(ev.artifact_id) && (ev.change === 'meta' || ev.change === 'archived')) full = true;
          continue;
        }
        full = true;
      }
      if (full || patchIds.size > 20) {
        patchIds.clear();
        scheduleReload();
      } else if (patchIds.size && !patchTimer) {
        patchTimer = setTimeout(() => {
          patchTimer = undefined;
          void patchCards();
        }, 350);
      }
    });
  });
  onDestroy(() => {
    clearTimeout(patchTimer);
    clearTimeout(reloadTimer);
    ++seq; // an in-flight request no longer owns this grid
  });

  function applyFresh(byId: Map<string, DesignArtifact>): void {
    let changed = false;
    // Mine is applied to `shown`, not here: a card never leaves on it.
    const f = { ...filters, mine: false };
    const next = hits
      .map((h) => {
        const a = byId.get(h.artifact.id);
        // Never step back to an older copy (the library can lag a fresh page).
        if (!a || sameCard(a, h.artifact) || a.updated_at < h.artifact.updated_at) return h;
        changed = true;
        return { ...h, artifact: a };
      })
      .filter((h) => matchesGrid(h.artifact, scope, f, meId));
    if (changed || next.length !== hits.length) hits = next;
  }
  async function patchCards(): Promise<void> {
    const ids = [...patchIds];
    patchIds.clear();
    const my = seq;
    const fresh = await Promise.all(ids.map((id) => getArtifact(id).then((d) => d.artifact, () => null)));
    if (my !== seq) return; // a reload landed meanwhile
    const ok = fresh.filter((a): a is DesignArtifact => !!a);
    if (ok.length < ids.length) {
      scheduleReload(); // gone or unreadable → the server decides
      return;
    }
    applyFresh(new Map(ok.map((a) => [a.id, a])));
  }
  // `follow`: adopt the library's patched copy of a card both show.
  $effect(() => {
    if (!follow) return;
    const lib = library.hits;
    untrack(() => {
      if (!loaded || !hits.length) return;
      const byId = new Map<string, DesignArtifact>();
      for (const h of lib) byId.set(h.artifact.id, h.artifact);
      applyFresh(byId);
    });
  });

  // ── Unarchive (status → draft through the ordinary status route) ─────────
  const canEdit = $derived(auth.can('design', 'edit'));
  async function unarchive(a: DesignArtifact): Promise<void> {
    try {
      const next = await updateArtifact(a.id, { status: 'draft' });
      applyFresh(new Map([[next.id, next]]));
      void library.load();
      toasts.success('Moved back to Draft', next.title);
    } catch (e) {
      toastError('Couldn’t unarchive the design', e);
    }
  }

  const shown = $derived(filters.mine ? hits.filter((h) => !!meId && h.artifact.created_by === meId) : hits);
  $effect(() => {
    count = shown.length;
  });
  const groups = $derived(
    groupHits(shown, groupBy, {
      story: (id) => {
        const s = library.stories[id];
        return s ? { key: s.source_key, title: s.title } : undefined;
      },
      project: (id) => library.projectOf(id)?.name,
    }),
  );
  const filtered = $derived(activeFilterCount(filters) > 0);
</script>

{#if !loaded && !error}
  <Skeleton rows={2} height={220} />
{:else if error && !loaded}
  <LoadState variant="compact" what="these designs" {error} empty onretry={() => void load()} />
{:else if hits.length === 0 && !filtered && empty}
  {@render empty()}
{:else if hits.length === 0}
  <div class="none" data-testid="design-grid-none">
    <p class="dim">{filters.archived ? 'No archived designs here.' : filtered ? 'No designs match these filters.' : 'No designs here yet.'}</p>
    {#if filtered && onclearfilters}<button class="btn small" onclick={onclearfilters}>Clear filters</button>{/if}
  </div>
{:else}
  {#if shown.length === 0}
    <p class="dim" data-testid="design-grid-none-mine">None of the {hits.length} newest designs loaded so far are yours{hasMore ? ' — load more to keep looking.' : '.'}</p>
  {/if}
  {#each groups as g (g.key)}
    <section class="group" aria-label={g.label ?? 'Designs'}>
      {#if g.label}
        <h3 class="g-head">
          {#if g.href}<a href={g.href}>{g.label}</a>{:else}<span>{g.label}</span>{/if}
          <span class="dim">{g.hits.length}</span>
        </h3>
      {/if}
      <div class="cards" data-testid="design-grid">
        {#each g.hits as h, i (h.artifact.id)}
          <ArtifactCard
            artifact={h.artifact}
            stories={storyKeysFor(h.story_ids, library.stories)}
            referenceCount={h.reference_count}
            live={i < 16}
            onunarchive={canEdit ? (a) => void unarchive(a) : undefined}
          />
        {/each}
      </div>
    </section>
  {/each}
  {#if error}
    <div class="more-err">
      <LoadState variant="compact" what="more designs" {error} empty onretry={() => void load(true)} />
    </div>
  {:else if hasMore}
    <div class="more">
      <button class="btn small" disabled={loading} onclick={() => void load(true)} data-testid="design-collection-more">
        {loading ? 'Loading more…' : 'Load more'}
      </button>
    </div>
  {/if}
{/if}

<style>
  .group + .group {
    margin-block-start: 20px;
  }
  .g-head {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 0 0 10px;
    font-size: var(--fs-s);
    font-weight: 600;
    min-width: 0;
  }
  .g-head a,
  .g-head > span:first-child {
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .g-head a:hover {
    text-decoration: underline;
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(240px, 100%), 1fr));
    gap: 12px;
  }
  .more {
    display: flex;
    justify-content: center;
    margin-block: 16px 4px;
  }
  .none {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .dim {
    color: var(--text-dim);
  }
  p.dim {
    margin: 0 0 12px;
    font-size: var(--fs-s);
  }
  .more-err {
    margin-block: 16px 4px;
  }
</style>
