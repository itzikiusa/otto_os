<script lang="ts">
  // Skills Lab → Skills. Every skill your agents can load, grouped by NAME —
  // one row per skill with a badge per copy (Library · Claude · Codex ·
  // Antigravity · Bundled) and a sync dot (in sync / drifted / one copy). The
  // list filters by category and source and searches name + description; the
  // detail pane (SkillDetail) opens on the last-viewed skill, else the first.
  //
  // Drift: library bodies arrive with the list; provider SKILL.md bodies are
  // fetched in the background (4 at a time) only for skills that exist in more
  // than one editable place, so the dots settle a moment after the list shows.
  import type { BundledSkillView, LibrarySkill, ProviderSkillInfo } from '../../lib/api/types';
  import { skillLabApi } from '../../lib/api/skillLab';
  import { toasts } from '../../lib/toast.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { recallSelection, rememberSelection } from '../../lib/lastSelection';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import SkillDetail, { type DetailTab } from './SkillDetail.svelte';
  import NewSkillModal from './NewSkillModal.svelte';
  import {
    bodyKey,
    categoryCounts,
    filterGroups,
    groupSkills,
    namesNeedingBodies,
    sourceCounts,
    sourceLabel,
    type SkillGroup,
    type SourceFilter,
    type VariantSource,
  } from './skillGroups';

  interface Props {
    onreview?: (name: string, source: string) => void;
    onevaluate?: (name: string, source: string) => void;
    /** Tells the page whether the library is empty (it then drops its header
     *  primary: the empty state owns "New skill"). */
    onempty?: (empty: boolean) => void;
    /** Phone push-navigation: the page shows a back button in its header. */
    onphonedetail?: (open: boolean) => void;
  }
  let { onreview, onevaluate, onempty, onphonedetail }: Props = $props();

  // ---- Data -----------------------------------------------------------------
  let library = $state<LibrarySkill[]>([]);
  let bundled = $state<BundledSkillView[]>([]);
  let providerSkills = $state<ProviderSkillInfo[]>([]);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let bodies = $state<Record<string, string>>({});

  async function loadAll(): Promise<void> {
    loadError = null;
    const [lib, bun, prov] = await Promise.allSettled([
      skillLabApi.listLibrary(),
      skillLabApi.listBundled(),
      skillLabApi.listProvider(),
    ]);
    if (lib.status === 'rejected' && bun.status === 'rejected' && prov.status === 'rejected') {
      loadError = lib.reason instanceof Error ? lib.reason.message : String(lib.reason);
    }
    library = lib.status === 'fulfilled' ? lib.value : [];
    bundled = bun.status === 'fulfilled' ? bun.value : [];
    providerSkills = prov.status === 'fulfilled' ? prov.value : [];
    loading = false;
  }
  let started = false;
  $effect(() => {
    if (started) return;
    started = true;
    void loadAll();
  });

  const groups = $derived(groupSkills(library, bundled, providerSkills, bodies));
  $effect(() => {
    if (!loading) onempty?.(groups.length === 0);
  });

  // Background drift check: fetch the provider bodies that have a copy to
  // compare against. Four in flight; each result lands in `bodies`.
  let inflight = new Set<string>();
  $effect(() => {
    if (loading) return;
    const need = namesNeedingBodies(groups, bodies).filter((n) => !inflight.has(bodyKey(n.source, n.name)));
    if (need.length === 0) return;
    const queue = [...need];
    for (const n of queue) inflight.add(bodyKey(n.source, n.name));
    const worker = async () => {
      for (let n = queue.shift(); n; n = queue.shift()) {
        try {
          const p = await skillLabApi.getProvider(n.source, n.name);
          bodies = { ...bodies, [bodyKey(n.source, n.name)]: p.body };
        } catch {
          // Unreadable copy: record an empty marker so the check settles.
          bodies = { ...bodies, [bodyKey(n.source, n.name)]: '' };
        }
      }
    };
    void Promise.all([worker(), worker(), worker(), worker()]);
  });

  // ---- Filters ----------------------------------------------------------------
  let query = $state('');
  let category = $state<string | null>(null);
  let source = $state<SourceFilter>('all');
  let driftedOnly = $state(false);
  const cats = $derived(categoryCounts(groups));
  // The four biggest categories as chips; the rest in a (clamped) ctxMenu so
  // the filter block stays two or three rows however many categories exist.
  const TOP_CATS = 4;
  const topCats = $derived(cats.slice(0, TOP_CATS));
  const moreCats = $derived(cats.slice(TOP_CATS));
  function moreCategories(e: MouseEvent): void {
    ctxMenu.show(
      e,
      moreCats.map(([c, n]) => ({
        label: `${c} · ${n}`,
        icon: category === c ? ('check' as const) : undefined,
        action: () => (category = category === c ? null : c),
      })),
      { filter: moreCats.length > 8 },
    );
  }
  const sources = $derived(sourceCounts(groups));
  const driftedCount = $derived(groups.filter((g) => g.sync === 'drifted').length);
  const shown = $derived(filterGroups(groups, { query, category, source, driftedOnly }));
  const shownByCat = $derived.by(() => {
    const m = new Map<string, SkillGroup[]>();
    for (const g of shown) {
      const arr = m.get(g.category) ?? [];
      arr.push(g);
      m.set(g.category, arr);
    }
    return [...m.entries()];
  });
  const filtering = $derived(!!query.trim() || !!category || source !== 'all' || driftedOnly);
  function clearFilters(): void {
    query = '';
    category = null;
    source = 'all';
    driftedOnly = false;
  }

  // ---- Selection -----------------------------------------------------------
  let selName = $state<string | null>(null);
  let selSource = $state<VariantSource>('library');
  let tab = $state<DetailTab>('overview');
  let phoneDetail = $state(false);
  const selected = $derived(groups.find((g) => g.name === selName) ?? null);
  $effect(() => onphonedetail?.(viewport.isPhone && phoneDetail));
  /** Phone: back from the detail to the list (the page header's back button). */
  export function back(): void {
    phoneDetail = false;
  }

  function defaultSource(g: SkillGroup): VariantSource {
    return g.variants.find((v) => v.source === 'library')?.source ?? g.variants.find((v) => v.source !== 'bundled')?.source ?? g.variants[0].source;
  }
  function select(g: SkillGroup, src?: VariantSource): void {
    selName = g.name;
    selSource = src && g.variants.some((v) => v.source === src) ? src : defaultSource(g);
    if (viewport.isPhone) phoneDetail = true;
  }
  // Open on the last-viewed skill, else the first one listed.
  $effect(() => {
    if (loading || groups.length === 0) return;
    if (selName && groups.some((g) => g.name === selName)) return;
    const last = recallSelection('skills-lab');
    const g = (last && groups.find((x) => x.name === last)) || shown[0] || groups[0];
    select(g);
    phoneDetail = false;
  });
  $effect(() => {
    if (selName) rememberSelection('skills-lab', selName);
  });
  // A copy that disappeared (deleted) falls back to the default copy.
  $effect(() => {
    if (selected && !selected.variants.some((v) => v.source === selSource)) selSource = defaultSource(selected);
  });

  // ---- New / import (called from the page header) --------------------------
  let newOpen = $state(false);
  let newTemplate = $state<'blank' | 'bundled' | 'import'>('blank');
  export function openNew(): void {
    newTemplate = 'blank';
    newOpen = true;
  }
  export function openImport(): void {
    newTemplate = 'import';
    newOpen = true;
  }
  async function created(s: LibrarySkill): Promise<void> {
    newOpen = false;
    await loadAll();
    const g = groups.find((x) => x.name === s.name);
    if (g) select(g, 'library');
    tab = 'edit';
    toasts.success('Skill created', `${s.name} is in your library`);
  }

  async function changed(next?: { name: string; source: VariantSource }): Promise<void> {
    await loadAll();
    if (next) {
      selName = next.name;
      selSource = next.source;
    }
  }
  async function deleted(): Promise<void> {
    selName = null;
    phoneDetail = false;
    await loadAll();
  }

  // ---- ⌘K ---------------------------------------------------------------------
  $effect(() => {
    const g = selected;
    return registry.register('skills-lab', [
      { id: 'skills.new', title: 'New skill…', group: 'Skills Lab', keywords: 'create skill template', run: openNew },
      { id: 'skills.import', title: 'Import a skill package…', group: 'Skills Lab', keywords: 'zip upload skill', run: openImport },
      ...(g
        ? [
            { id: 'skills.review', title: `Review skill ${g.name}`, group: 'Skills Lab', keywords: 'multi-agent review', run: () => onreview?.(g.name, selSource) },
            { id: 'skills.evaluate', title: `Evaluate skill ${g.name}`, group: 'Skills Lab', keywords: 'eval score', run: () => onevaluate?.(g.name, selSource) },
          ]
        : []),
    ]);
  });

  // ---- List keyboard: ↑/↓ move, Enter opens --------------------------------
  let listEl = $state<HTMLElement | null>(null);
  function onListKey(e: KeyboardEvent): void {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const i = shown.findIndex((g) => g.name === selName);
    const n = e.key === 'ArrowDown' ? Math.min(shown.length - 1, i + 1) : Math.max(0, i - 1);
    if (n === i || !shown[n]) return;
    e.preventDefault();
    select(shown[n]);
    queueMicrotask(() => (listEl?.querySelector(`[data-name="${CSS.escape(shown[n].name)}"]`) as HTMLElement | null)?.focus());
  }

  function syncTitle(g: SkillGroup): string {
    switch (g.sync) {
      case 'in_sync':
        return `In sync across ${g.variants.length} copies`;
      case 'drifted':
        return g.drift.join('\n');
      case 'single':
        return 'Only in one place';
      default:
        return 'Comparing copies…';
    }
  }
</script>

<div class="browser" data-testid="skills-browser">
  {#if loading}
    <div class="split">
      <aside class="list-pane" aria-busy="true"><div class="pad"><Skeleton rows={10} height={40} /></div></aside>
      <section class="detail-pane"><p class="dim pad" role="status">Loading skills…</p></section>
    </div>
  {:else if loadError}
    <div class="load-error" role="alert">
      <Icon name="warning" size={16} />
      <div>
        <strong>Couldn't load skills.</strong>
        <p class="dim">Otto couldn't read the library or the bundled catalog. Retry, or check Settings → Logs.</p>
        <p class="dim mono small">{loadError}</p>
      </div>
      <button class="btn small" onclick={loadAll}>Retry</button>
    </div>
  {:else if groups.length === 0}
    <EmptyState
      variant="page"
      icon="zap"
      title="No skills yet"
      body="Skills are instruction packs that teach an agent a method — how to review a diff, write release notes, run a report. Create one, or import a package."
      actionLabel="New skill"
      actionIcon="plus"
      onaction={openNew}
    >
      <button class="btn ghost" onclick={openImport}>Import a .zip</button>
    </EmptyState>
  {:else}
    <div class="split">
      <aside class="list-pane" class:hide-phone={viewport.isPhone && phoneDetail} aria-label="Skills">
        <div class="list-tools">
          <label class="search">
            <Icon name="search" size={14} />
            <input type="search" placeholder="Search skills" bind:value={query} aria-label="Search skills" />
          </label>
          <div class="chips" role="group" aria-label="Filter by source">
            <button class="fchip" class:active={source === 'all'} aria-pressed={source === 'all'} onclick={() => (source = 'all')}>All <span class="n">{groups.length}</span></button>
            {#each sources as [s, n] (s)}
              <button class="fchip" class:active={source === s} aria-pressed={source === s} onclick={() => (source = source === s ? 'all' : s)} title="Skills with a {sourceLabel(s)} copy">
                {#if s === 'library'}<Icon name="book" size={12} />{:else if s === 'bundled'}<Icon name="box" size={12} />{:else}<ProviderIcon provider={s} size={12} />{/if}
                {sourceLabel(s)} <span class="n">{n}</span>
              </button>
            {/each}
            {#if driftedCount > 0}
              <button class="fchip warn" class:active={driftedOnly} aria-pressed={driftedOnly} onclick={() => (driftedOnly = !driftedOnly)} title="Skills whose copies differ">
                <span class="sdot drifted"></span>Drifted <span class="n">{driftedCount}</span>
              </button>
            {/if}
          </div>
          <div class="chips" role="group" aria-label="Filter by category">
            {#each topCats as [c, n] (c)}
              <button class="fchip" class:active={category === c} aria-pressed={category === c} onclick={() => (category = category === c ? null : c)}>{c} <span class="n">{n}</span></button>
            {/each}
            {#if moreCats.length > 0}
              <button class="fchip" class:active={!!category && !topCats.some(([c]) => c === category)} aria-haspopup="menu" onclick={moreCategories} title="More categories">
                {category && !topCats.some(([c]) => c === category) ? category : `${moreCats.length} more`}
                <Icon name="chevronDown" size={12} />
              </button>
            {/if}
          </div>
        </div>
        <div class="list" bind:this={listEl} data-testid="skill-list" role="listbox" aria-label="Skills" tabindex="-1" onkeydown={onListKey}>
          {#each shownByCat as [cat, items] (cat)}
            <div class="cat" role="presentation">{cat} <span class="n">{items.length}</span></div>
            {#each items as g (g.name)}
              <button
                class="row"
                class:active={g.name === selName}
                role="option"
                aria-selected={g.name === selName}
                tabindex={g.name === selName ? 0 : -1}
                data-name={g.name}
                data-testid="skill-row"
                onclick={() => select(g)}
              >
                <span class="sdot {g.sync}" title={syncTitle(g)}></span>
                <span class="row-main">
                  <span class="row-name">{g.name}</span>
                  {#if g.description}<span class="row-desc">{g.description}</span>{/if}
                </span>
                <span class="badges" aria-label="Copies: {g.variants.map((v) => sourceLabel(v.source)).join(', ')}">
                  {#each g.variants as v (v.source)}
                    <span class="badge" class:drift={g.driftedSources.includes(v.source)} title="{sourceLabel(v.source)}{g.driftedSources.includes(v.source) ? ' — differs' : ''}">
                      {#if v.source === 'library'}<Icon name="book" size={12} />{:else if v.source === 'bundled'}<Icon name="box" size={12} />{:else}<ProviderIcon provider={v.source} size={12} />{/if}
                    </span>
                  {/each}
                </span>
              </button>
            {/each}
          {:else}
            <div class="no-match">
              <p class="dim">No skills match{query.trim() ? ` "${query.trim()}"` : ''}.</p>
              {#if filtering}<button class="btn small ghost" onclick={clearFilters}>Clear filters</button>{/if}
            </div>
          {/each}
        </div>
        <div class="legend dim" aria-hidden="true">
          <span><span class="sdot in_sync"></span>In sync</span>
          <span><span class="sdot drifted"></span>Drifted</span>
          <span><span class="sdot single"></span>One copy</span>
        </div>
      </aside>

      <section class="detail-pane" class:hide-phone={viewport.isPhone && !phoneDetail}>
        {#if selected}
          <SkillDetail
            group={selected}
            source={selSource}
            {tab}
            wsId={ws.currentId ?? ''}
            libraryBody={library.find((l) => l.name === selected.name)?.body}
            bodyOf={(s) => bodies[bodyKey(s, selected.name)]}
            onsource={(s) => (selSource = s)}
            ontab={(t) => (tab = t)}
            onchanged={changed}
            ondeleted={deleted}
            onbody={(s, b) => (bodies = { ...bodies, [bodyKey(s, selected.name)]: b })}
            onreview={() => onreview?.(selected.name, selSource)}
            onevaluate={() => onevaluate?.(selected.name, selSource)}
          />
        {:else}
          <EmptyState title="No skill selected" body="Pick a skill on the left to see its method, files and history." icon="zap" />
        {/if}
      </section>
    </div>
  {/if}
</div>

{#if newOpen}
  <NewSkillModal
    {bundled}
    categories={cats.map(([c]) => c).filter((c) => c !== 'uncategorized')}
    taken={new Set(library.map((l) => l.name))}
    initial={newTemplate}
    onclose={() => (newOpen = false)}
    oncreated={created}
  />
{/if}

<style>
  .browser {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .split {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .pad {
    padding: 12px;
  }
  .list-pane {
    width: 320px;
    flex: none;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-inline-end: 1px solid var(--border);
    background: var(--surface);
  }
  .detail-pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .list-tools {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  /* The focus ring is drawn on the .search wrapper (:focus-within above). */
  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    outline: none;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .fchip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 500;
    cursor: pointer;
  }
  .fchip:hover {
    background: var(--hover);
    color: var(--text);
  }
  .fchip.active {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
    color: var(--text);
  }
  .fchip.warn {
    color: var(--warning);
  }
  .n {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 6px 8px;
  }
  .cat {
    display: flex;
    align-items: baseline;
    gap: 6px;
    padding: 10px 8px 4px;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .cat .n {
    letter-spacing: 0;
    font-weight: 500;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 40px;
    padding: 5px 8px;
    border: 1px solid transparent;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    text-align: start;
    cursor: pointer;
    font: inherit;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.active {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 28%, transparent);
  }
  .row-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .row-name,
  .row-desc {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row-name {
    font-weight: 500;
  }
  .row-desc {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .badges {
    display: inline-flex;
    gap: 2px;
    flex: none;
  }
  .badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .badge.drift {
    box-shadow: inset 0 0 0 1.5px var(--status-warn);
  }
  .sdot {
    display: inline-block;
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--status-idle);
  }
  .sdot.in_sync {
    background: var(--status-working);
  }
  .sdot.drifted {
    background: var(--status-warn);
  }
  .sdot.single {
    background: transparent;
    border: 1.5px solid var(--text-dim);
  }
  .legend {
    display: flex;
    gap: 12px;
    padding: 6px 12px;
    border-top: 1px solid var(--border);
    font-size: var(--fs-xs);
  }
  .legend > span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .no-match {
    padding: 16px 10px;
  }
  .no-match p {
    margin: 0 0 6px;
  }
  .load-error {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    margin: 20px;
    padding: 14px 16px;
    max-width: 720px;
    border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .load-error > :global(svg) {
    color: var(--danger);
    margin-top: 2px;
  }
  .load-error > div {
    flex: 1;
  }
  .load-error p {
    margin: 4px 0 0;
  }
  .small {
    font-size: var(--fs-xs);
  }
  @media (max-width: 1024px) {
    .list-pane {
      width: 280px;
    }
  }
  @media (max-width: 640px) {
    .list-pane {
      width: 100%;
      border-inline-end: none;
    }
    .hide-phone {
      display: none;
    }
    .row {
      min-height: 44px;
    }
  }
</style>
