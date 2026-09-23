<script lang="ts">
  // Design Hall lobby (Grid view): the prompt hero, all seven studios, the
  // Continue strip, projects, the Linked-to-Product epic tree, and a rail with
  // what Otto captured for learning + agent activity. Dashboard-style: every
  // region owns its loading / empty / error state; the library snapshot is
  // shared (library.svelte.ts) and refreshed by live design events.
  import { untrack } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon, { asIcon } from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import { router } from '../../lib/router.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { designBus } from '../../lib/events.svelte';
  import { search as searchApi } from '../../lib/api/design';
  import { isAbortError } from '../../lib/api/client';
  import type { DesignSearchHit, DesignStudio } from '../../lib/api/types';
  import ArtifactCard from './ArtifactCard.svelte';
  import ProjectCard from './ProjectCard.svelte';
  import LinkedToProduct from './LinkedToProduct.svelte';
  import LearnedCard from './LearnedCard.svelte';
  import StudioBadge from './StudioBadge.svelte';
  import SpatialView from './SpatialView.svelte';
  import { STUDIOS, filterProjects, studioInfo, titleFromPrompt, type ProjectFilter } from './model';
  import { createDesign } from './create';
  import { generateFromBrief, referenceOf, suggestReferences } from './assist/handoff';
  import { library } from './library.svelte';

  interface Props {
    view: 'grid' | 'spatial';
    onnew: (e: MouseEvent) => void;
    onimport: () => void;
    onnewproject: () => void;
  }
  let { view, onnew, onimport, onnewproject }: Props = $props();

  // ── Library + live refresh ────────────────────────────────────────────────
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
      if (evs.some((e) => e.type !== 'design_learning_update')) library.refreshSoon();
    });
  });

  const live = $derived(library.hits.filter((h) => h.artifact.status !== 'archived'));
  const recent = $derived(
    [...live].sort((a, b) => b.artifact.updated_at.localeCompare(a.artifact.updated_at)).slice(0, 8),
  );
  const refCount = (id: string) => library.hitOf(id)?.reference_count ?? 0;

  // ── Projects ──────────────────────────────────────────────────────────────
  let pfilter = $state<ProjectFilter>('all');
  const allProjects = $derived(library.projects.filter((p) => !p.archived));
  const projects = $derived(filterProjects(library.projects, library.artifacts, pfilter, auth.me?.id));
  const PFILTERS: { id: ProjectFilter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'mine', label: 'Mine' },
    { id: 'epic', label: 'By epic' },
    { id: 'shipped', label: 'Shipped' },
  ];
  function epicLabel(id: string | null): string | null {
    if (!id) return null;
    const s = library.stories[id];
    return s ? `${s.source_key} · ${s.title}` : 'Linked epic';
  }

  // ── Agent activity: designs agents drafted, with their live session ──────
  const agentWork = $derived(
    live
      .filter((h) => h.artifact.created_by_kind === 'agent')
      .sort((a, b) => b.artifact.updated_at.localeCompare(a.artifact.updated_at))
      .slice(0, 4)
      .map((h) => ({
        a: h.artifact,
        session: h.artifact.created_session_id
          ? (ws.sessions.find((s) => s.id === h.artifact.created_session_id) ?? null)
          : null,
      })),
  );

  // ── Prompt hero: Generate = create the draft + hand the brief to Otto (one
  // generate turn, or a variants run), then open it on the Otto tab. "Draft
  // only" keeps the brief on an empty draft without an agent. ─────────────
  let prompt = $state('');
  let heroStudio = $state<DesignStudio>('frames');
  let heroProject = $state('');
  let creating = $state(false);
  const heroStudios = STUDIOS.filter((s) => s.formats.length > 0);
  const TRY = ['Launch page with a product hero', 'Social tile in portrait', 'Checkout flow diagram', 'Product shot on a plinth'];
  const TRY_STUDIO: DesignStudio[] = ['frames', 'graphics', 'whiteboard', '3d'];
  let variants = $state(1);
  let useRefs = $state(true);
  /** The team's best matches for the brief — offered to Otto first as [R1..R3]. */
  let refHits = $state<DesignSearchHit[]>([]);
  let refCtl: AbortController | null = null;
  let refTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    const p = prompt.trim();
    const on = useRefs;
    const wsId = ws.currentId;
    if (refTimer) clearTimeout(refTimer);
    if (!on || p.length < 4 || !wsId) {
      refCtl?.abort();
      refHits = [];
      return;
    }
    refTimer = setTimeout(() => void loadRefs(p, wsId), 450);
  });
  async function loadRefs(p: string, wsId: string): Promise<void> {
    refCtl?.abort();
    const mine = new AbortController();
    refCtl = mine;
    try {
      const hits = await suggestReferences(p, wsId, mine.signal);
      if (refCtl === mine) refHits = hits;
    } catch {
      /* no preview; Otto still searches the library itself */
    }
  }

  async function generate(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.warn('Pick a workspace first', 'New designs are filed under a workspace.');
      return;
    }
    creating = true;
    try {
      const r = await generateFromBrief({
        workspaceId: wsId,
        studio: heroStudio,
        format: studioInfo(heroStudio).formats[0],
        title: titleFromPrompt(prompt),
        projectId: heroProject || null,
        brief: prompt,
        variants,
        references: useRefs ? refHits.map(referenceOf) : [],
      });
      prompt = '';
      if (r.assistError) toasts.warn('Draft created, but Otto couldn’t start', r.assistError);
      router.go(`design/a/${encodeURIComponent(r.artifactId)}/otto`);
    } catch (e) {
      toasts.error('Couldn’t create the draft', e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }

  async function createFromPrompt(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.warn('Pick a workspace first', 'New designs are filed under a workspace.');
      return;
    }
    creating = true;
    try {
      const id = await createDesign({
        workspaceId: wsId,
        studio: heroStudio,
        format: studioInfo(heroStudio).formats[0],
        title: titleFromPrompt(prompt),
        projectId: heroProject || null,
        brief: prompt,
      });
      prompt = '';
      router.go(`design/a/${encodeURIComponent(id)}`);
    } catch (e) {
      toasts.error('Couldn’t create the draft', e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }

  function openStudio(id: DesignStudio): void {
    if (id === 'brand') router.go('design/brand');
    else if (id === 'spatial') router.go('design/spatial');
    else router.go(`design/studio/${id}`);
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

  async function runSearch(term: string): Promise<void> {
    ctl?.abort();
    const mine = new AbortController();
    ctl = mine;
    try {
      const hits = await searchApi(term, { limit: 60 }, mine.signal);
      if (ctl !== mine) return;
      results = hits;
      searchError = null;
    } catch (e) {
      if (isAbortError(e) || ctl !== mine) return;
      searchError = e instanceof Error ? e.message : String(e);
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

  function goView(v: 'grid' | 'spatial'): void {
    router.go(v === 'grid' ? 'design' : 'design/spatial');
  }
</script>

<PageHeader title="Design Hall" class="dh-header">
  {#snippet tabs()}
    <div class="segmented" role="tablist" aria-label="Lobby view">
      <button role="tab" aria-selected={view === 'grid'} class:active={view === 'grid'} onclick={() => goView('grid')}>
        <Icon name="grid" size={12} /> Grid
      </button>
      <button role="tab" aria-selected={view === 'spatial'} class:active={view === 'spatial'} onclick={() => goView('spatial')}
        data-testid="design-spatial-tab">
        <Icon name="gallery" size={12} /> Spatial <span class="beta">Beta</span>
      </button>
    </div>
  {/snippet}
  {#snippet actions()}
    <label class="search" data-keep>
      <Icon name="search" size={14} />
      <input
        type="search"
        placeholder="Search designs, stories, references…"
        aria-label="Search all designs"
        bind:value={q}
        onkeydown={onSearchKey}
        data-testid="design-search"
      />
    </label>
    <button class="btn small" data-overflow="-1" data-icon="download" onclick={onimport}>
      <Icon name="download" size={12} /> Import
    </button>
    <button class="btn small" onclick={onnew} aria-haspopup="menu" data-testid="design-new" data-keep>
      <Icon name="plus" size={12} /> New <Icon name="chevronDown" size={12} />
    </button>
  {/snippet}
</PageHeader>

{#if view === 'spatial'}
  <SpatialView />
{:else}
<PageBody>
  <div class="lobby-wrap">
  <div class="lobby" data-testid="design-lobby">
    <div class="main">
      {#if searchActive}
        <section aria-labelledby="dh-results-h">
          <div class="sec-head">
            <h2 id="dh-results-h">Results</h2>
            <span class="dim">{searching ? 'Searching…' : `${results.length} for “${q.trim()}” · shipped first`}</span>
            <span class="grow"></span>
            <button class="btn small ghost" onclick={() => (q = '')}>Clear search</button>
          </div>
          {#if searchError}
            <div class="inline-err" role="alert">
              <Icon name="warning" size={14} />
              <span>Search failed. {searchError}</span>
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
      {:else}
        <!-- Prompt hero -->
        <section class="hero card" aria-labelledby="dh-hero-h">
          <h2 id="dh-hero-h">What do you want to make?</h2>
          <p class="dim">Otto drafts it in the studio you pick, from your team’s past designs. Every result is a version you can compare, keep or undo.</p>
          <div class="composer">
            <textarea
              class="input"
              rows="2"
              placeholder="Describe it… e.g. “a launch page for the loyalty programme with a 3D card hero”"
              aria-label="Describe the design"
              bind:value={prompt}
              onkeydown={(e) => {
                if (e.key === 'Enter' && (e.metaKey || e.ctrlKey) && prompt.trim() && !creating) {
                  e.preventDefault();
                  void generate();
                }
              }}
              data-testid="design-prompt"
            ></textarea>
            <div class="composer-bar">
              <label class="sel">
                <span>Studio</span>
                <select class="input" bind:value={heroStudio} aria-label="Studio for the draft">
                  {#each heroStudios as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
                </select>
              </label>
              <label class="sel">
                <span>Project</span>
                <select class="input" bind:value={heroProject} aria-label="Project for the draft">
                  <option value="">None</option>
                  {#each allProjects as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
                </select>
              </label>
              <label class="sel">
                <span>Variants</span>
                <select class="input" bind:value={variants} aria-label="How many directions Otto drafts" data-testid="design-prompt-variants">
                  {#each [1, 2, 3, 4] as n (n)}<option value={n}>{n}</option>{/each}
                </select>
              </label>
              <button class="pill-toggle" class:on={useRefs} aria-pressed={useRefs} onclick={() => (useRefs = !useRefs)}
                title="Offer your team’s closest past designs to Otto first, as references it can cite" data-testid="design-prompt-refs">
                {#if useRefs}<Icon name="check" size={12} />{/if} Use references
              </button>
              <span class="grow"></span>
              <span class="go">
              <button class="btn ghost" disabled={!prompt.trim() || creating} onclick={createFromPrompt} data-testid="design-prompt-create"
                title="Save the brief on an empty draft, without asking Otto">
                Draft only
              </button>
              <button class="btn primary" disabled={!prompt.trim() || creating} onclick={generate} data-testid="design-prompt-generate"
                title="Create the draft and ask Otto to design it (⌘Enter)">
                <Icon name="sparkle" size={13} /> {creating ? 'Starting…' : 'Generate'}
              </button>
              </span>
            </div>
            {#if useRefs && refHits.length}
              <div class="refs" aria-label="References Otto gets first" data-testid="design-prompt-ref-chips">
                <span class="dim">References</span>
                {#each refHits as h, i (h.artifact.id)}
                  <a class="chip" href={`#/design/a/${encodeURIComponent(h.artifact.id)}`} title={`${h.artifact.title} (${h.artifact.status}) — offered as R${i + 1}`}>
                    R{i + 1} · {h.artifact.title}
                  </a>
                {/each}
              </div>
            {/if}
          </div>
          <div class="try">
            <span class="dim">Try</span>
            {#each TRY as t, i (t)}
              <button class="chip as-btn" onclick={() => { prompt = t; heroStudio = TRY_STUDIO[i]; }}>{t}</button>
            {/each}
          </div>
          <p class="phase"><Icon name="info" size={12} /> Nothing is applied for you: Otto’s drafts are versions, and variants wait until you pick one.</p>
        </section>

        <!-- Studios -->
        <section aria-labelledby="dh-studios-h">
          <div class="sec-head">
            <h2 id="dh-studios-h">Studios</h2>
            <span class="dim">7 studios · one shared library</span>
          </div>
          <div class="studios" data-testid="design-studios">
            {#each STUDIOS as s (s.id)}
              <button class="studio" onclick={() => openStudio(s.id)} data-testid={`design-studio-${s.id}`}>
                <StudioBadge studio={s.id} size={28} />
                <span class="s-name">{s.name}</span>
                <span class="s-blurb">{s.blurb}</span>
                {#if s.phase === 'planned'}
                  <span class="s-tag" title={`Planned for ${s.roadmap}`}>{s.roadmap}</span>
                {/if}
              </button>
            {/each}
          </div>
        </section>

        <!-- Continue -->
        <section aria-labelledby="dh-continue-h">
          <div class="sec-head">
            <h2 id="dh-continue-h">Continue</h2>
            <span class="dim">Recently edited</span>
          </div>
          {#if library.loading && !library.loaded}
            <Skeleton rows={1} height={220} />
          {:else if library.error && !library.loaded}
            <div class="inline-err" role="alert">
              <Icon name="warning" size={14} />
              <span>Couldn’t load the design library.</span>
              <span class="dim err-detail">{library.error}</span>
              <button class="btn small" onclick={() => void library.load()}>Retry</button>
            </div>
          {:else if recent.length === 0}
            <p class="dim">Nothing here yet. Designs you and your agents make — and the ones already in Product and Canvas — show up here.</p>
          {:else}
            <div class="strip" data-testid="design-continue">
              {#each recent as h (h.artifact.id)}
                <ArtifactCard artifact={h.artifact} stories={library.storyKeys(h.artifact.id)} referenceCount={refCount(h.artifact.id)} live />
              {/each}
            </div>
          {/if}
        </section>

        <!-- Projects -->
        <section aria-labelledby="dh-projects-h">
          <div class="sec-head">
            <h2 id="dh-projects-h">Projects</h2>
            <span class="dim">{projects.length} of {allProjects.length}</span>
            <span class="grow"></span>
            {#if allProjects.length}
              <div class="segmented" role="group" aria-label="Filter projects">
                {#each PFILTERS as f (f.id)}
                  <button aria-pressed={pfilter === f.id} class:active={pfilter === f.id} onclick={() => (pfilter = f.id)}>{f.label}</button>
                {/each}
              </div>
            {/if}
          </div>
          {#if library.loading && !library.loaded}
            <Skeleton rows={2} height={120} />
          {:else if allProjects.length === 0}
            <div class="none">
              <p class="dim">No projects yet. A project groups the designs for one launch or epic.</p>
              <button class="btn small" onclick={onnewproject}><Icon name="plus" size={12} /> New project…</button>
            </div>
          {:else if projects.length === 0}
            <p class="dim">No projects match this filter. <button class="linkbtn" onclick={() => (pfilter = 'all')}>Show all</button></p>
          {:else}
            <div class="projects">
              {#each projects as p, i (p.id)}
                <ProjectCard project={p} artifacts={library.artifacts} epicLabel={epicLabel(p.epic_story_id)} live={i < 6} />
              {/each}
            </div>
          {/if}
        </section>

        <!-- Linked to Product -->
        <section aria-labelledby="dh-linked-h">
          <div class="sec-head">
            <h2 id="dh-linked-h">Linked to Product</h2>
            <span class="dim">Epics and the designs that serve them</span>
          </div>
          {#if library.loading && !library.loaded}
            <Skeleton rows={3} height={32} />
          {:else}
            <LinkedToProduct />
          {/if}
        </section>
      {/if}
    </div>

    <aside class="rail" aria-label="Learning and activity">
      <LearnedCard />
      {#if agentWork.length}
        <section class="card activity" aria-labelledby="dh-activity-h">
          <header>
            <span class="ico"><Icon name="user" size={14} /></span>
            <h2 id="dh-activity-h">Agent activity</h2>
          </header>
          <ul>
            {#each agentWork as w (w.a.id)}
              <li>
                <div class="act-line">
                  {#if w.session}<StatusDot status={w.session.status} />{/if}
                  <span><span class="who">{w.session?.title ?? w.a.created_session_title ?? 'An agent'}</span> drafted
                    <a href={`#/design/a/${encodeURIComponent(w.a.id)}`}>{w.a.title}</a></span>
                </div>
                <div class="act-meta">
                  <span title={new Date(w.a.updated_at).toLocaleString()}>{rel(w.a.updated_at)}</span>
                  {#if w.session}
                    <button class="linkbtn" onclick={() => ws.navigateToSession(w.session!.id)}>Open session</button>
                  {/if}
                </div>
              </li>
            {/each}
          </ul>
        </section>
      {/if}
      <nav class="card quick" aria-label="Design Hall pages">
        <a href="#/design/brand"><Icon name="palette" size={14} /> Brand Kit</a>
        <a href="#/design/learned"><Icon name="bulb" size={14} /> What Otto learned</a>
        <a href="#/canvas"><Icon name={asIcon('shapes')} size={14} /> Open Canvas boards</a>
      </nav>
    </aside>
  </div>
  </div>
</PageBody>
{/if}

<style>
  /* The rail stacks under the main column when the PAGE BODY is narrow (a
     container query on the wrapper — tablets, split windows, phones). */
  .lobby-wrap {
    container-type: inline-size;
  }
  .lobby {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 300px;
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
    gap: 28px;
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
  .beta {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--info);
    background: var(--info-soft);
    border-radius: 999px;
    padding: 0 6px;
    margin-inline-start: 2px;
  }
  .segmented > button {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .search {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 27px;
    width: min(320px, 32vw);
    padding: 0 8px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
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
  /* Hero */
  .hero {
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .hero h2 {
    margin: 0;
    font-size: var(--fs-xl);
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .hero > p {
    margin: 0;
    font-size: var(--fs-s);
  }
  .composer {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .composer textarea {
    height: auto;
    min-height: 52px;
    resize: vertical;
    border: 0;
    background: transparent;
    padding: 4px 2px;
    box-shadow: none;
    font-size: var(--fs-m);
  }
  .composer textarea:focus {
    box-shadow: none;
  }
  .composer:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .composer-bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }
  .sel {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .sel select {
    height: 24px;
    padding-block: 0;
    width: auto;
    max-width: 200px;
    background: var(--surface);
  }
  .try {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .as-btn {
    cursor: pointer;
    font-family: inherit;
  }
  .as-btn:hover {
    color: var(--text);
    border-color: var(--border-strong);
  }
  .go {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-inline-start: auto;
  }
  .refs {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .refs .chip {
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    text-decoration: none;
  }
  .refs .chip:hover {
    color: var(--text);
    border-color: var(--border-strong);
  }
  .phase {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  /* Studios */
  /* All seven studios in one row when the column allows it (they are one
     product); narrower columns wrap to an even grid. */
  .main {
    container-type: inline-size;
  }
  .studios {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(132px, 1fr));
    gap: 8px;
  }
  @container (min-width: 800px) {
    .studios {
      grid-template-columns: repeat(7, minmax(0, 1fr));
    }
  }
  .studio {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    min-height: 118px;
    padding: 12px 10px 10px;
    text-align: start;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    color: var(--text);
    font: inherit;
    cursor: pointer;
    transition: border-color 130ms ease-out;
  }
  .studio:hover {
    border-color: var(--border-strong);
  }
  .s-name {
    margin-block-start: 6px;
    font-weight: 600;
  }
  .s-blurb {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    line-height: 1.4;
  }
  .s-tag {
    position: absolute;
    inset-block-start: 10px;
    inset-inline-end: 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 0 6px;
  }
  /* Continue */
  .strip {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: minmax(250px, 1fr);
    gap: 12px;
    overflow-x: auto;
    padding-block-end: 4px;
    scroll-snap-type: x proximity;
  }
  .strip > :global(*) {
    scroll-snap-align: start;
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 12px;
  }
  /* Projects */
  .projects {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
    gap: 12px;
  }
  @media (max-width: 640px) {
    .projects {
      grid-template-columns: 1fr;
    }
    .search {
      width: 44vw;
    }
  }
  .none {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .none p {
    margin: 0;
  }
  .dim {
    color: var(--text-dim);
  }
  p.dim {
    margin: 0;
    font-size: var(--fs-s);
  }
  .linkbtn {
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .linkbtn:hover {
    text-decoration: underline;
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
  .err-detail {
    font-size: var(--fs-xs);
  }
  /* Rail */
  .rail {
    display: flex;
    flex-direction: column;
    gap: 16px;
    min-width: 0;
  }
  .activity {
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .activity header {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .activity h2 {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .ico {
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .activity ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .activity li {
    padding: 8px 0;
    border-block-start: 1px solid var(--border);
    font-size: var(--fs-s);
  }
  .act-line {
    display: flex;
    align-items: baseline;
    gap: 6px;
  }
  .act-line a {
    color: var(--text);
    font-weight: 500;
  }
  .who {
    font-weight: 600;
  }
  .act-meta {
    display: flex;
    justify-content: space-between;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-block-start: 2px;
  }
  .quick {
    padding: 6px;
    display: flex;
    flex-direction: column;
  }
  .quick a {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    color: var(--text);
    text-decoration: none;
    font-size: var(--fs-s);
  }
  .quick a:hover {
    background: var(--hover);
  }
  .quick a :global(svg) {
    color: var(--text-dim);
  }
</style>
