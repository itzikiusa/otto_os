<script lang="ts">
  // Help (route `#/walkthroughs`, kept for old links, the native Help menu and
  // ⌘K). A list/detail page of section guides — one README per sidebar module
  // plus the shell-wide Basics — with the single tour film on top of the
  // default view.
  //
  //   #/walkthroughs            film + Getting started (never an empty pane)
  //   #/walkthroughs/<id>       that guide (unknown id → inline "not found")
  //
  // Guides are ./sections/*.md, loaded at build time (./sections.ts) — the
  // rail lists whatever files exist, grouped like the sidebar. The rail is
  // searchable (title, summary, body and shortcut keys, ranked in ./guide.ts)
  // and arrow-key navigable. A guide shows "Watch this part" when a film
  // chapter maps to it; a chapter shows "Read the guide" when its guide exists.
  // The ⌘K "Guide: …" commands are registered app-wide in App.svelte.
  //
  // Phone: push navigation — the film + list is the page; opening a guide
  // replaces it and the header gets a back button.
  import { tick } from 'svelte';
  import { router } from '../../lib/router.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { SIDEBAR_MODULES } from '../../lib/sidebar';
  import { GUIDE_GROUPS, searchSections, splitChord, timeLabel, type GuideSection } from './guide';
  import { DEFAULT_GUIDE_ID, FILM, GUIDES, guideById } from './sections';
  import { renderGuideHtml } from './render';
  import TourFilm from './TourFilm.svelte';

  const guideIds = new Set(GUIDES.map((g) => g.id));

  // ---- selection (from the route) ----
  const param = $derived(router.module === 'walkthroughs' ? router.parts[1] : undefined);
  const isDefault = $derived(!param);
  const selectedId = $derived(param ?? DEFAULT_GUIDE_ID);
  const selected = $derived(guideById(selectedId));

  /** The film shows on the default view, and on a guide after "Watch this part"
   *  / "Watch the tour" (reset whenever the guide changes). */
  let filmRequested = $state(false);
  const showFilm = $derived(isDefault || filmRequested);
  let film: ReturnType<typeof TourFilm> | undefined = $state();
  let mainEl: HTMLElement | undefined = $state();

  let lastId = '';
  $effect(() => {
    const id = selectedId;
    if (id === lastId) return;
    lastId = id;
    filmRequested = false;
    mainEl?.scrollTo({ top: 0 });
  });

  function open(id: string, replace = false): void {
    const path = `walkthroughs/${id}`;
    if (replace) router.replace(path);
    else router.go(path);
  }

  // ---- rendered body (memoised per guide) ----
  const htmlCache = new Map<string, string>();
  const html = $derived.by(() => {
    if (!selected) return '';
    let h = htmlCache.get(selected.id);
    if (h === undefined) {
      h = renderGuideHtml(selected.body);
      htmlCache.set(selected.id, h);
    }
    return h;
  });

  // ---- film ↔ guide ----
  const chapterFor = $derived(FILM?.chapters.find((c) => c.section === selectedId));

  async function watchPart(start: number): Promise<void> {
    filmRequested = true;
    await tick();
    mainEl?.scrollTo({ top: 0, behavior: 'smooth' });
    film?.playAt(start);
  }

  async function watchTour(): Promise<void> {
    filmRequested = true;
    await tick();
    mainEl?.scrollTo({ top: 0, behavior: 'smooth' });
  }

  // ---- "Open <module>" (respects the sidebar's RBAC gate) ----
  function routeAllowed(route: string): boolean {
    const mod = route.split('/')[0];
    if (mod === 'plugin') return auth.canPlugin(route.split('/')[1] ?? '', 'view');
    const def = SIDEBAR_MODULES.find((m) => m.id === mod);
    if (!def) return true;
    if (def.featureAny) return def.featureAny.some((f) => auth.can(f, 'view'));
    return def.feature == null || auth.can(def.feature, 'view');
  }
  const openAllowed = $derived(selected?.route ? routeAllowed(selected.route) : false);

  // ---- search ----
  let query = $state('');
  let searchEl: HTMLInputElement | undefined = $state();
  const hits = $derived(searchSections(GUIDES, query));
  const searching = $derived(query.trim().length > 0);
  const grouped = $derived(
    GUIDE_GROUPS.map((group) => ({ group, items: GUIDES.filter((g) => g.group === group) })).filter(
      (s) => s.items.length > 0,
    ),
  );
  /** The rail's rows in on-screen order (arrow-key navigation walks this). */
  const railOrder = $derived(searching ? hits.map((h) => h.section.id) : GUIDES.map((g) => g.id));

  // ---- keyboard: ↑/↓/Home/End move through the rail; Enter in search opens ----
  let railEl: HTMLElement | undefined = $state();

  function focusRow(id: string): void {
    railEl?.querySelector<HTMLElement>(`[data-guide="${CSS.escape(id)}"]`)?.focus();
  }

  function step(from: string | undefined, delta: number | 'first' | 'last'): string | undefined {
    const list = railOrder;
    if (!list.length) return undefined;
    if (delta === 'first') return list[0];
    if (delta === 'last') return list[list.length - 1];
    const i = from ? list.indexOf(from) : -1;
    if (i < 0) return delta > 0 ? list[0] : list[list.length - 1];
    return list[Math.min(Math.max(i + delta, 0), list.length - 1)];
  }

  function onRailKey(e: KeyboardEvent): void {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const inSearch = e.target === searchEl;
    // The phone rail also contains the film. Keep its native controls and
    // chapter buttons out of the guide list's keyboard navigation.
    if (!inSearch && !(e.target instanceof HTMLElement && e.target.closest('[data-guide]'))) return;
    const moves: Record<string, number | 'first' | 'last'> = { ArrowDown: 1, ArrowUp: -1 };
    if (!inSearch) Object.assign(moves, { Home: 'first', End: 'last' });
    if (inSearch && e.key === 'Enter') {
      const top = railOrder[0];
      if (top) {
        e.preventDefault();
        open(top);
      }
      return;
    }
    if (inSearch && e.key === 'Escape' && query) {
      e.preventDefault();
      query = '';
      return;
    }
    const mv = moves[e.key];
    if (mv === undefined) return;
    e.preventDefault();
    const current = (document.activeElement as HTMLElement | null)?.dataset?.guide ?? (inSearch ? undefined : selectedId);
    const next = inSearch && mv === 1 && !current ? step(undefined, 'first') : step(current, mv);
    if (!next) return;
    // Arrow browsing replaces the history entry (Back leaves the page, not
    // every guide you arrowed past); on a phone it only moves focus.
    if (!viewport.isPhone) open(next, true);
    void tick().then(() => focusRow(next));
  }

  const pageSubtitle = 'Guides to every part of Otto, with shortcuts and limits';
  const showList = $derived(!viewport.isPhone || isDefault);
  const showArticle = $derived(!viewport.isPhone || !isDefault);
</script>

{#snippet row(g: GuideSection, match?: { kind: string; text: string })}
  <button
    class="rail-row"
    class:active={!searching && g.id === selectedId && !viewport.isPhone}
    class:hit={searching && g.id === railOrder[0]}
    aria-current={g.id === selectedId && !viewport.isPhone ? 'page' : undefined}
    data-guide={g.id}
    data-testid="guide-row"
    onclick={() => open(g.id)}
  >
    <span class="row-title">{g.title}</span>
    {#if searching}
      <span class="row-meta">
        <span class="row-group">{g.group}</span>
        {#if match?.kind === 'shortcut'}
          <span class="keys">{#each splitChord(match.text) as k, i (i)}<kbd>{k}</kbd>{/each}</span>
        {:else if match?.text}
          <span class="row-snippet">{match.text}</span>
        {/if}
      </span>
    {:else if g.summary && viewport.isPhone}
      <span class="row-snippet">{g.summary}</span>
    {/if}
  </button>
{/snippet}

<div class="help-page">
  <PageHeader title="Help" subtitle={pageSubtitle}>
    {#snippet leading()}
      {#if viewport.isPhone && !isDefault}
        <button class="icon-btn help-back" onclick={() => router.go('walkthroughs')} aria-label="Back to all guides" title="Back to all guides">
          <Icon name="chevronLeft" size={16} />
        </button>
      {/if}
    {/snippet}
    {#snippet actions()}
      {#if FILM && !showFilm && showArticle && !viewport.isPhone}
        <button class="btn ghost" onclick={watchTour} data-label="Watch the tour" data-icon="play">
          <Icon name="play" size={12} /> Watch the tour
        </button>
      {/if}
      {#if selected?.route && showArticle}
        <button
          class="btn primary"
          disabled={!openAllowed}
          title={openAllowed ? `Go to ${selected.title}` : `You don't have access to ${selected.title}. Ask an admin for access.`}
          onclick={() => selected?.route && router.go(selected.route)}
          data-testid="guide-open-module"
        >
          Open {selected.title}
        </button>
      {/if}
    {/snippet}
  </PageHeader>

  {#if GUIDES.length === 0}
    <PageBody>
      <EmptyState
        variant="page"
        icon="book"
        title="No guides in this build"
        body="The Help guides ship with the app. Reinstall or update Otto to get them back."
      />
    </PageBody>
  {:else}
    <PageBody fill padded={false}>
      <div class="help-layout" class:phone={viewport.isPhone}>
        {#if showList}
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <nav class="rail" aria-label="Guides" bind:this={railEl} onkeydown={onRailKey}>
            <div class="rail-search">
              <Icon name="search" size={13} />
              <input
                bind:this={searchEl}
                bind:value={query}
                type="search"
                placeholder="Search guides and shortcuts"
                aria-label="Search guides"
                autocomplete="off"
                spellcheck="false"
                data-testid="guide-search"
              />
            </div>

            {#if viewport.isPhone && FILM && !searching}
              <div class="rail-film"><TourFilm bind:this={film} film={FILM} {guideIds} onopenguide={(id) => open(id)} /></div>
            {/if}

            <div class="rail-list">
              {#if searching}
                {#if hits.length === 0}
                  <div class="rail-empty">
                    <p>No guides match “{query.trim()}”.</p>
                    <button class="btn small" onclick={() => { query = ''; searchEl?.focus(); }}>Clear search</button>
                  </div>
                {:else}
                  <div class="rail-count" role="status">{hits.length} {hits.length === 1 ? 'guide' : 'guides'}</div>
                  {#each hits as h (h.section.id)}
                    {@render row(h.section, h.match)}
                  {/each}
                {/if}
              {:else}
                {#each grouped as s (s.group)}
                  <div class="rail-group" role="group" aria-label={s.group}>
                    <div class="rail-group-label" aria-hidden="true">{s.group}</div>
                    {#each s.items as g (g.id)}
                      {@render row(g)}
                    {/each}
                  </div>
                {/each}
              {/if}
            </div>
          </nav>
        {/if}

        {#if showArticle}
          <main class="main" bind:this={mainEl} data-testid="guide-main">
            <div class="column">
              {#if showFilm}
                <TourFilm bind:this={film} film={FILM} {guideIds} onopenguide={(id) => open(id)} />
              {/if}

              {#if selected}
                <article class="guide" data-testid="guide-article" data-guide-id={selected.id}>
                  <header class="guide-head">
                    <div class="eyebrow">{selected.group}</div>
                    <h2>{selected.title}</h2>
                    {#if selected.summary}<p class="summary">{selected.summary}</p>{/if}
                    {#if chapterFor}
                      <button class="btn small" onclick={() => chapterFor && watchPart(chapterFor.start)} data-testid="guide-watch-part">
                        <Icon name="play" size={11} /> Watch this part · {timeLabel(chapterFor.start)}
                      </button>
                    {/if}
                  </header>
                  <div class="md-body guide-body">{@html html}</div>
                </article>
              {:else}
                <EmptyState
                  icon="book"
                  title="There's no guide called “{param}”"
                  body="It may have been renamed. Pick one from the list, or start at the beginning."
                  actionLabel="Open Getting started"
                  onaction={() => open(DEFAULT_GUIDE_ID)}
                />
              {/if}
            </div>
          </main>
        {/if}
      </div>
    </PageBody>
  {/if}
</div>

<style>
  :global([dir='rtl']) .help-back :global(svg) {
    transform: scaleX(-1);
  }
  .help-page {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }
  .help-layout {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 272px minmax(0, 1fr);
  }
  .help-layout.phone {
    grid-template-columns: minmax(0, 1fr);
  }

  /* ---- rail ---- */
  .rail {
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-inline-end: 1px solid var(--separator);
    background: var(--bg);
  }
  .phone .rail {
    border-inline-end: 0;
    overflow-y: auto;
  }
  .rail-search {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 12px 12px 8px;
    padding: 0 8px;
    height: 28px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text-dim);
  }
  .rail-search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }
  .rail-search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-m);
  }
  .rail-film {
    padding: 4px 12px 16px;
    margin-bottom: 4px;
    border-bottom: 1px solid var(--separator);
  }
  .rail-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 8px 16px;
  }
  .phone .rail-list {
    overflow: visible;
  }
  .rail-group + .rail-group {
    margin-top: 10px;
  }
  .rail-group-label {
    padding: 6px 8px 3px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .rail-count {
    padding: 2px 8px 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .rail-row {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 2px;
    width: 100%;
    min-height: 28px;
    padding: 5px 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    text-align: start;
    cursor: pointer;
  }
  .phone .rail-row {
    min-height: 44px;
    padding: 8px 10px;
  }
  .rail-row:hover {
    background: var(--hover);
  }
  .rail-row.active {
    background: var(--accent-soft);
  }
  .rail-row.active .row-title {
    color: var(--accent-text);
    font-weight: 600;
  }
  .rail-row.hit {
    box-shadow: inset 0 0 0 1px var(--border-strong);
  }
  .row-title {
    font-size: var(--fs-m);
    line-height: 1.35;
  }
  .row-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .row-group {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .row-snippet {
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rail-empty {
    padding: 16px 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .rail-empty p {
    margin: 0 0 8px;
  }

  /* ---- main ---- */
  .main {
    min-height: 0;
    min-width: 0;
    overflow-y: auto;
    padding: 20px 28px max(48px, var(--fb-clearance, 0px));
  }
  .phone .main {
    padding: 14px 16px max(40px, var(--fb-clearance, 0px));
  }
  .column {
    max-width: 880px;
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
  .guide-head {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--separator);
  }
  .eyebrow {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .guide-head h2 {
    margin: 0;
    font-size: var(--fs-2xl);
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .summary {
    margin: 0 0 6px;
    font-size: var(--fs-l);
    color: var(--text-dim);
    line-height: 1.45;
  }

  /* Guide prose on top of the shared .md-body. */
  .guide-body {
    line-height: 1.6;
  }
  .guide-body :global(h2) {
    font-size: var(--fs-l);
    margin: 26px 0 8px;
  }
  .guide-body :global(h3) {
    font-size: var(--fs-m);
    margin: 18px 0 6px;
  }
  .guide-body :global(li) {
    margin: 3px 0;
  }
  .guide-body :global(ul),
  .guide-body :global(ol) {
    padding-inline-start: 22px;
  }
  .guide-body :global(a) {
    color: var(--accent-text);
    text-decoration: none;
  }
  .guide-body :global(a:hover) {
    text-decoration: underline;
  }
  .guide-body :global(.table-wrap) {
    overflow-x: auto;
    margin: 8px 0 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
  .guide-body :global(table) {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
  }
  .guide-body :global(th) {
    text-align: start;
    font-weight: 600;
    color: var(--text-dim);
    background: var(--surface-2);
    padding: 6px 10px;
  }
  .guide-body :global(td) {
    padding: 6px 10px;
    border-top: 1px solid var(--border);
    vertical-align: top;
  }
  .guide-body :global(.keys-table td:first-child) {
    white-space: nowrap;
    width: 1%;
  }

  /* Key chips (guide tables, prose and the search results). */
  .keys,
  .guide-body :global(.keys) {
    display: inline-flex;
    gap: 2px;
    vertical-align: baseline;
  }
  kbd,
  .guide-body :global(kbd) {
    display: inline-block;
    min-width: 18px;
    padding: 0 5px;
    border: 1px solid var(--border-strong);
    border-bottom-width: 2px;
    border-radius: 4px;
    background: var(--surface);
    color: var(--text);
    font-family: var(--font-ui);
    font-size: var(--fs-xs);
    line-height: 17px;
    text-align: center;
  }

  @media (max-width: 1024px) {
    .help-layout:not(.phone) {
      grid-template-columns: 232px minmax(0, 1fr);
    }
    .main {
      padding-inline: 20px;
    }
  }
</style>
