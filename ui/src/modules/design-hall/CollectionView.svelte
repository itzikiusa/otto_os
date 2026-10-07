<script module lang="ts">
  import type { DesignStudio as DS } from '../../lib/api/types';
  import { toastError } from '../../lib/toastError';
  export type Scope = { kind: 'project'; id: string } | { kind: 'studio'; id: DS } | { kind: 'story'; id: string };
</script>

<script lang="ts">
  // A filtered slice of the library as a page: one project, one studio, or the
  // designs that implement one product story. Artifacts are grouped by studio;
  // the header's primary action starts a new design already filed here.
  import { onDestroy, untrack } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { router } from '../../lib/router.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { designBus } from '../../lib/events.svelte';
  import { getArtifact, search, updateProject, type DesignListFilter } from '../../lib/api/design';
  import type { DesignArtifact, DesignSearchHit, DesignStudio } from '../../lib/api/types';
  import ArtifactCard from './ArtifactCard.svelte';
  import StudioBadge from './StudioBadge.svelte';
  import { STUDIOS, studioInfo } from './model';
  import { library } from './library.svelte';
  import { openStoryInProduct } from './nav';
  import LoadState from '../../lib/components/LoadState.svelte';

  interface Props {
    scope: Scope;
    onnew: (init: { studio?: DesignStudio; projectId?: string | null; storyId?: string | null }) => void;
  }
  let { scope, onnew }: Props = $props();

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

  const project = $derived(scope.kind === 'project' ? library.projectOf(scope.id) : undefined);
  const story = $derived(scope.kind === 'story' ? library.stories[scope.id] : undefined);
  const studio = $derived(scope.kind === 'studio' ? studioInfo(scope.id) : null);

  // DH1: the slice is loaded from the SERVER, scoped + paged — filtering the
  // shared library (the 500 newest overall) silently dropped older designs.
  // The library still supplies projects/stories metadata for the header.
  const PAGE = 120;
  let hits = $state.raw<DesignSearchHit[]>([]);
  let sLoading = $state(false);
  let sLoaded = $state(false);
  let sError = $state<string | null>(null);
  let hasMore = $state(false);
  let sSeq = 0;
  let reloadTimer: ReturnType<typeof setTimeout> | undefined;
  const filter = (): DesignListFilter =>
    scope.kind === 'project'
      ? { project_id: scope.id }
      : scope.kind === 'studio'
        ? { studio: scope.id }
        : { story_id: scope.id };

  /** `append` fetches the next page by KEYSET cursor (the last row's
   *  `updated_at|id`, newest-first like the server) — OFFSET paging re-read
   *  every skipped row. Otherwise reload everything shown so far (at least
   *  one page) so a resync never collapses "Load more". */
  async function loadScoped(append = false): Promise<void> {
    const my = ++sSeq;
    sLoading = true;
    const last = append ? hits[hits.length - 1]?.artifact : undefined;
    const limit = append ? PAGE : Math.max(PAGE, hits.length);
    try {
      const page = await search('', { ...filter(), limit, ...(last ? { cursor: `${last.updated_at}|${last.id}` } : {}) });
      if (my !== sSeq) return;
      if (append) {
        const have = new Set(hits.map((h) => h.artifact.id));
        hits = [...hits, ...page.filter((h) => !have.has(h.artifact.id))];
      } else {
        hits = page;
      }
      hasMore = page.length === limit;
      sError = null;
      sLoaded = true;
    } catch (e) {
      if (my !== sSeq) return;
      sError = e instanceof Error ? e.message : String(e);
    } finally {
      if (my === sSeq) sLoading = false;
    }
  }
  $effect(() => {
    void scope.kind;
    void scope.id;
    untrack(() => {
      clearTimeout(patchTimer);
      patchTimer = undefined;
      patchIds.clear();
      hits = [];
      sLoaded = false;
      void loadScoped();
    });
  });
  // A resync (WS reconnect: events may have been missed) reloads the slice.
  $effect(() => {
    void designBus.resyncTick;
    untrack(() => {
      if (sLoaded) scheduleReload();
    });
    return () => clearTimeout(reloadTimer);
  });
  function scheduleReload(): void {
    clearTimeout(reloadTimer);
    reloadTimer = setTimeout(() => void loadScoped(), 600); // coalesce bursts
  }
  // Live events PATCH the cards they name (one detail GET each) instead of
  // re-reading the whole slice; only creates/deletes/link changes — which
  // can change membership — fall back to a debounced reload.
  let seenScoped = designBus.seq;
  const patchIds = new Set<string>();
  let patchTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const now = designBus.seq;
    untrack(() => {
      const evs = designBus.since(seenScoped);
      seenScoped = now;
      if (!sLoaded || !evs.length) return;
      const ids = new Set(hits.map((h) => h.artifact.id));
      // A project lives in one workspace: a create / delete / link change in
      // ANOTHER workspace (every design event is broadcast) can't change this
      // slice, so it must not re-read it. Cards it shows still patch.
      const home = scope.kind === 'project' ? project?.workspace_id : undefined;
      let full = false;
      for (const ev of evs) {
        if (ev.type === 'design_learning_update') continue;
        if (home && ev.workspace_id !== home && !ids.has(ev.artifact_id)) continue;
        if (ev.type === 'design_artifact_updated' && ev.change !== 'created' && ev.change !== 'deleted') {
          if (ids.has(ev.artifact_id)) patchIds.add(ev.artifact_id);
          // An update to a card this slice doesn't show can't add it here
          // (studio/project/story membership changes arrive as their own events).
          else if (scope.kind === 'project' && ev.change === 'meta') full = true;
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
  // Effect cleanup also runs before every new event batch. Cancelling there
  // stranded patchIds behind a truthy timer handle and froze live cards.
  onDestroy(() => {
    clearTimeout(patchTimer);
    ++sSeq; // an in-flight detail request no longer owns this collection
  });
  async function patchCards(): Promise<void> {
    const ids = [...patchIds];
    patchIds.clear();
    const my = sSeq;
    const fresh = await Promise.all(ids.map((id) => getArtifact(id).then((d) => d.artifact, () => null)));
    if (my !== sSeq) return; // a reload landed meanwhile
    const ok = fresh.filter((a): a is DesignArtifact => !!a);
    if (ok.length < ids.length) {
      scheduleReload(); // gone or unreadable → the server decides
      return;
    }
    const byId = new Map<string, DesignArtifact>(ok.map((a) => [a.id, a]));
    hits = hits
      .map((h) => {
        const a = byId.get(h.artifact.id);
        return a ? { ...h, artifact: a } : h;
      })
      .filter((h) => scope.kind !== 'project' || h.artifact.project_id === scope.id);
  }
  const hitById = $derived(new Map(hits.map((h) => [h.artifact.id, h])));
  const storyKeysOf = (id: string): string[] =>
    (hitById.get(id)?.story_ids ?? []).map((sid) => library.stories[sid]?.source_key).filter((k): k is string => !!k);

  const items = $derived.by((): DesignArtifact[] =>
    hits
      .filter((h) => h.artifact.status !== 'archived')
      .map((h) => h.artifact)
      .sort((a, b) => b.updated_at.localeCompare(a.updated_at)),
  );
  const groups = $derived(
    STUDIOS.map((s) => ({ s, list: items.filter((a) => a.studio === s.id) })).filter((g) => g.list.length),
  );
  /** Studio filter chips (only when the slice spans several studios). */
  let only = $state<'' | DesignStudio>('');
  const shown = $derived(only ? items.filter((a) => a.studio === only) : items);

  const title = $derived(
    scope.kind === 'project'
      ? (project?.name ?? 'Project')
      : scope.kind === 'studio'
        ? (studio?.name ?? 'Studio')
        : story
          ? `${story.source_key} · ${story.title}`
          : 'Story designs',
  );
  const subtitle = $derived(
    scope.kind === 'project'
      ? project?.description || (project?.epic_story_id && library.stories[project.epic_story_id] ? `Epic ${library.stories[project.epic_story_id].source_key}` : '')
      : scope.kind === 'studio'
        ? studio?.blurb
        : 'Designs that implement this story',
  );
  const canNew = $derived(scope.kind !== 'studio' || (studio?.formats.length ?? 0) > 0);
  const notFound = $derived(library.loaded && sLoaded && scope.kind === 'project' && !project && items.length === 0);

  function newHere(): void {
    if (scope.kind === 'project') onnew({ projectId: scope.id, storyId: project?.epic_story_id ?? null });
    else if (scope.kind === 'studio') onnew({ studio: scope.id });
    else onnew({ storyId: scope.id });
  }

  async function renameProject(): Promise<void> {
    if (!project) return;
    const name = await confirmer.promptText('Project name', { title: 'Rename project', confirmLabel: 'Rename', initial: project.name });
    if (!name || name === project.name) return;
    try {
      await updateProject(project.id, { name });
      void library.load();
    } catch (e) {
      toastError('Couldn’t rename the project', e);
    }
  }

  async function archiveProject(): Promise<void> {
    if (!project) return;
    const ok = await confirmer.ask(
      `Archive the project “${project.name}”? Its designs stay in the library (unchanged) and can be filed elsewhere.`,
      { title: 'Archive project', confirmLabel: 'Archive', danger: false },
    );
    if (!ok) return;
    try {
      await updateProject(project.id, { archived: true });
      toasts.success('Project archived');
      router.go('design');
    } catch (e) {
      toastError('Couldn’t archive the project', e);
    }
  }

  const crumbs = [{ label: 'Design Hall', onclick: () => router.go('design') }];
  const canEdit = $derived(auth.can('design', 'edit'));
</script>

<PageHeader {title} {subtitle} crumbs={viewport.isPhone ? [] : crumbs}>
    {#snippet leading()}
      {#if viewport.isPhone}
        <button class="icon-btn" onclick={() => router.go('design')} aria-label="Back" title="Back">
          <Icon name="chevronLeft" size={16} />
        </button>
      {/if}
    {/snippet}
  {#snippet badge()}
    {#if studio}
      <span class="badge-row">
        <StudioBadge studio={studio.id} size={20} />
        {#if studio.phase === 'planned'}<span class="chip">Planned · {studio.roadmap}</span>{:else if studio.phase === 'classic'}<span class="chip">Classic editor</span>{/if}
      </span>
    {/if}
  {/snippet}
  {#snippet actions()}
    {#if scope.kind === 'project' && project && canEdit}
      <button class="btn small" data-overflow="-2" data-icon="archive" onclick={archiveProject}>Archive…</button>
      <button class="btn small" data-overflow="-1" data-icon="edit" onclick={renameProject}>Rename…</button>
    {/if}
    {#if scope.kind === 'story'}
      <button class="btn small" data-icon="external" onclick={() => openStoryInProduct(scope.id)}><Icon name="external" size={12} /> Open in Product</button>
    {/if}
    {#if scope.kind === 'studio' && scope.id === 'whiteboard'}
      <button class="btn small" data-icon="shapes" onclick={() => router.go('canvas')}><Icon name="shapes" size={12} /> Open Canvas boards</button>
    {/if}
    {#if canNew && canEdit && items.length}
      <button class="btn small primary" onclick={newHere} data-testid="design-collection-new"><Icon name="plus" size={12} /> New design</button>
    {/if}
  {/snippet}
</PageHeader>

<PageBody>
  {#if studio}
    <p class="note" data-testid="design-studio-note"><Icon name="info" size={14} /> {studio.note}</p>
  {/if}
  {#if !sLoaded && !sError}
    <Skeleton rows={3} height={180} />
  {:else if sError && !sLoaded}
    <LoadState variant="compact" what="these designs" error={sError} empty onretry={() => { void library.load(); void loadScoped(); }} />
  {:else if notFound}
    <EmptyState variant="page" icon="designHall" title="This project isn’t available" body="It was archived or deleted, or it belongs to a workspace you can’t view."
      actionLabel="Back to Design Hall" onaction={() => router.go('design')} />
  {:else if items.length === 0}
    <EmptyState
      variant="page"
      icon={scope.kind === 'studio' ? 'frame' : 'designHall'}
      title={scope.kind === 'studio' && !canNew ? `${studio?.name} is planned` : 'No designs here yet'}
      body={scope.kind === 'studio' && !canNew
        ? `Nothing to open yet — ${studio?.name} arrives in ${studio?.roadmap}.`
        : scope.kind === 'story'
          ? 'No design implements this story yet. Start one here, or link an existing design from its Links panel.'
          : 'Start a draft — it keeps every version and can link to stories and other designs.'}
      actionLabel={canNew && canEdit ? 'New design' : undefined}
      actionIcon="plus"
      onaction={canNew && canEdit ? newHere : undefined}
    />
  {:else}
    {#if groups.length > 1}
      <div class="studio-filter" role="group" aria-label="Filter by studio">
        <button class="pill-toggle" class:on={only === ''} aria-pressed={only === ''} onclick={() => (only = '')}>All {items.length}</button>
        {#each groups as g (g.s.id)}
          <button class="pill-toggle" class:on={only === g.s.id} aria-pressed={only === g.s.id} onclick={() => (only = g.s.id)}>
            <StudioBadge studio={g.s.id} /> {g.s.name} {g.list.length}
          </button>
        {/each}
      </div>
    {/if}
    <div class="cards">
      {#each shown as a, i (a.id)}
        <ArtifactCard artifact={a} stories={storyKeysOf(a.id)} referenceCount={hitById.get(a.id)?.reference_count ?? 0} live={i < 16} />
      {/each}
    </div>
    {#if hasMore}
      <div class="more">
        <button class="btn small" disabled={sLoading} onclick={() => void loadScoped(true)} data-testid="design-collection-more">
          {sLoading ? 'Loading more…' : 'Load more'}
        </button>
      </div>
    {/if}
  {/if}
</PageBody>

<style>
  .more {
    display: flex;
    justify-content: center;
    margin-block: 16px 4px;
  }
  .badge-row {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .note {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 0 0 18px;
    padding: 10px 12px;
    max-width: 880px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .note > :global(svg) {
    color: var(--info);
    margin-block-start: 1px;
    flex: none;
  }
  .studio-filter {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-block-end: 14px;
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 12px;
  }
</style>
