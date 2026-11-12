<script lang="ts">
  // Design Hall lobby: one prompt line (studio ▾ · brief · ⋯ · Generate — the
  // page's one primary action), then "All designs" — a server-paged grid you
  // group by Recent · Story · Project and filter by studio, status, Mine and
  // Archived (the one place archived designs can be found and restored) —
  // and a rail with only what needs a person: Needs review and Agent
  // activity. A workspace with no designs yet gets a calm start: a heading,
  // the prompt and "Import a file". Brand Kit and Team design rules live in
  // the header ⋯; the studios are the prompt's picker and the Studio chip.
  import { untrack } from 'svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProductModeTabs from '../../lib/components/ProductModeTabs.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { router } from '../../lib/router.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { designBus } from '../../lib/events.svelte';
  import { search as searchApi } from '../../lib/api/design';
  import { isAbortError } from '../../lib/api/client';
  import type { DesignSearchHit } from '../../lib/api/types';
  import { loadErrorText } from '../../lib/loadError';
  import ArtifactCard from './ArtifactCard.svelte';
  import DesignGrid from './DesignGrid.svelte';
  import GridFilterBar from './GridFilterBar.svelte';
  import LobbyPrompt from './LobbyPrompt.svelte';
  import LobbyRail from './LobbyRail.svelte';
  import { library } from './library.svelte';
  import { NO_FILTERS, type GridFilters, type GroupBy } from './designGrid';

  interface Props {
    onnew: (e: MouseEvent) => void;
    onimport: () => void;
    onnewproject: () => void;
  }
  let { onnew, onimport, onnewproject }: Props = $props();
  const canEdit = $derived(auth.can('design', 'edit'));

  // ── Library (rail, project + story names) + live refresh ─────────────────
  $effect(() => {
    void designBus.resyncTick;
    untrack(() => void library.load());
  });
  let seen = designBus.seq;
  $effect(() => {
    const now = designBus.seq;
    untrack(() => {
      const evs = designBus.since(seen);
      seen = now;
      library.applyEvents(evs); // patch one card, or a debounced full reload
    });
  });

  // ── All designs ──────────────────────────────────────────────────────────
  let filters = $state<GridFilters>({ ...NO_FILTERS });
  let groupBy = $state<GroupBy>('recent');
  const projects = $derived(library.projects.filter((p) => !p.archived));
  /** First run: nothing in the library, and no filter asked for archived ones. */
  const firstRun = $derived(library.loaded && !library.error && library.hits.length === 0 && !filters.archived);

  function showReview(): void {
    filters = { ...NO_FILTERS, status: 'review' };
    groupBy = 'recent';
    document.getElementById('dh-all-h')?.scrollIntoView({ block: 'start' });
  }

  function moreMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Brand Kit', icon: 'palette', action: () => router.go('design/brand') },
      { label: 'Team design rules', icon: 'bulb', action: () => router.go('design/learned') },
      { separator: true },
      {
        label: 'Archived designs',
        checked: filters.archived,
        title: 'Show only archived designs, to find or restore one',
        action: () => (filters = { ...filters, archived: !filters.archived, status: '' }),
      },
    ]);
  }

  // ── Global search (header field) ─────────────────────────────────────────
  let q = $state('');
  let results = $state<DesignSearchHit[]>([]);
  let searching = $state(false);
  let searchError = $state<string | null>(null);
  let ctl: AbortController | null = null;
  let debounce: ReturnType<typeof setTimeout> | null = null;
  const searchActive = $derived(q.trim().length > 0);

  $effect(() => {
    const term = q.trim();
    untrack(() => {
      if (debounce) clearTimeout(debounce);
      if (!term) {
        ctl?.abort();
        results = [];
        searching = false;
        searchError = null;
        return;
      }
      searching = true;
      debounce = setTimeout(() => void runSearch(term), 220);
    });
  });

  async function runSearch(term: string): Promise<void> {
    ctl?.abort();
    const mine = (ctl = new AbortController());
    try {
      const hits = await searchApi(term, { limit: 60 }, mine.signal);
      if (ctl !== mine) return;
      results = hits;
      searchError = null;
    } catch (e) {
      if (isAbortError(e) || ctl !== mine) return;
      searchError = loadErrorText(e);
    } finally {
      if (ctl === mine) searching = false;
    }
  }

  function onSearchKey(e: KeyboardEvent): void {
    if (e.key === 'Escape' && q) {
      e.preventDefault();
      q = '';
    }
  }
</script>

<!-- The search field rides in the header on desktop/tablet; a phone row has
     no room for it (it pushed New off the edge), so it leads the body there. -->
{#snippet searchField()}
  <label class="search" data-keep>
    <Icon name="search" size={14} />
    <input dir="auto"
      type="search"
      placeholder="Search designs, stories, references…"
      aria-label="Search all designs"
      bind:value={q}
      onkeydown={onSearchKey}
      data-testid="design-search"
    />
  </label>
{/snippet}

<PageHeader title="Product" subtitle="Stories, specs and design studios" class="dh-header">
  {#snippet tabs()}
    <ProductModeTabs active="design" />
  {/snippet}
  {#snippet actions()}
    {#if !viewport.isPhone}{@render searchField()}{/if}
    <button class="btn small" data-overflow="-1" data-icon="download" onclick={onimport} disabled={!canEdit}>
      <Icon name="download" size={12} /> Import
    </button>
    <button class="btn small" class:primary={searchActive} onclick={onnew} aria-haspopup="menu" data-testid="design-new" data-keep>
      <Icon name="plus" size={12} /> New <Icon name="chevronDown" size={12} />
    </button>
    <button class="icon-btn" onclick={moreMenu} aria-haspopup="menu" aria-label="More Design Hall pages" title="Brand Kit, team design rules, archived designs"
      data-testid="design-more" data-keep>
      <Icon name="more" size={14} />
    </button>
  {/snippet}
</PageHeader>

<PageBody>
  <div class="lobby-wrap" data-testid="design-lobby">
  {#if viewport.isPhone}<div class="phone-search">{@render searchField()}</div>{/if}
  {#if searchActive}
    <section aria-labelledby="dh-results-h">
      <div class="sec-head">
        <h2 id="dh-results-h">Results</h2>
        <span class="dim">{searching ? 'Searching…' : `${results.length} for “${q.trim()}” · shipped first`}</span>
        <span class="grow"></span>
        <button class="btn small ghost" onclick={() => (q = '')}>Clear search</button>
      </div>
      {#if searchError}
        <div class="inline-err" use:announceShown={{ text: 'Couldn’t search. Check the connection and try again.', assertive: true }}>
          <Icon name="warning" size={14} />
          <span title={searchError}>Couldn’t search. Check the connection and try again.</span>
          <button class="btn small" onclick={() => void runSearch(q.trim())}>Retry</button>
        </div>
      {:else if !searching && results.length === 0}
        <p class="dim">No designs match “{q.trim()}”. Search looks at titles, tags, text inside designs, linked stories and project names.</p>
      {:else}
        <div class="cards">
          {#each results as h (h.artifact.id)}
            <ArtifactCard artifact={h.artifact} stories={library.storyKeys(h.artifact.id)} referenceCount={h.reference_count} />
          {/each}
        </div>
      {/if}
    </section>
  {:else if firstRun}
    <section class="first" aria-labelledby="dh-first-h" data-testid="design-first-run">
      <h2 id="dh-first-h">What do you want to make?</h2>
      <LobbyPrompt {canEdit} />
      <div class="first-import">
        <button class="btn small ghost" onclick={onimport} disabled={!canEdit} data-testid="design-first-import">
          <Icon name="download" size={12} /> Import a file
        </button>
      </div>
    </section>
  {:else}
    <div class="lobby">
      <div class="main">
        <section class="hero" aria-labelledby="dh-hero-h">
          <h2 id="dh-hero-h">What do you want to make?</h2>
          <LobbyPrompt {canEdit} />
        </section>

        <section aria-labelledby="dh-all-h" data-testid="design-all">
          <div class="sec-head">
            <h2 id="dh-all-h">{filters.archived ? 'Archived designs' : 'All designs'}</h2>
          </div>
          <GridFilterBar bind:filters bind:groupBy />
          {#if groupBy === 'project'}
            <div class="projects" role="group" aria-label="Projects">
              {#each projects as p (p.id)}
                <a class="pchip" href={`#/design/p/${encodeURIComponent(p.id)}`}><Icon name="folder" size={12} /> {p.name}</a>
              {/each}
              <button class="pchip" onclick={onnewproject} disabled={!canEdit}><Icon name="plus" size={12} /> New project…</button>
            </div>
          {/if}
          <DesignGrid {filters} {groupBy} page={60} follow onclearfilters={() => (filters = { ...NO_FILTERS })} />
        </section>
      </div>
      <LobbyRail onshowreview={showReview} />
    </div>
  {/if}
  </div>
</PageBody>

<style>
  /* The rail stacks under the main column when the PAGE BODY is narrow (a
     container query on the wrapper — tablets, split windows, phones). */
  .lobby-wrap {
    container-type: inline-size;
  }
  .lobby {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 280px;
    gap: 24px;
    align-items: start;
  }
  @container (max-width: 860px) {
    .lobby {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: 24px;
    min-width: 0;
  }
  .sec-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin-block-end: 10px;
    min-width: 0;
  }
  .sec-head h2 {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .sec-head .dim {
    font-size: var(--fs-s);
  }
  .grow {
    flex: 1;
  }
  .hero h2,
  .first h2 {
    margin: 0 0 10px;
    font-size: var(--fs-l);
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .first {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
    max-width: 720px;
    margin: 8vh auto 0;
  }
  .first h2 {
    margin: 0;
    font-size: var(--fs-xl);
    text-align: center;
  }
  .first-import {
    display: flex;
    justify-content: center;
  }
  .projects {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-block-end: 14px;
  }
  .pchip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 260px;
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    text-decoration: none;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }
  .pchip:hover:not(:disabled) {
    color: var(--text);
    border-color: var(--border-strong);
  }
  .pchip:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .search {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 27px;
    width: min(240px, 26vw);
    padding: 0 8px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  .phone-search {
    margin-block-end: 16px;
  }
  .phone-search .search {
    display: flex;
    width: 100%;
    height: 36px;
  }
  .search:focus-within {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    outline: none;
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(240px, 100%), 1fr));
    gap: 12px;
  }
  .dim {
    color: var(--text-dim);
  }
  p.dim {
    margin: 0;
    font-size: var(--fs-s);
  }
  .inline-err {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: var(--fs-s);
  }
  .inline-err > :global(svg) {
    color: var(--danger);
  }
  @media (pointer: coarse), (max-width: 640px) {
    .search {
      min-block-size: var(--hit-min);
    }
  }
</style>
