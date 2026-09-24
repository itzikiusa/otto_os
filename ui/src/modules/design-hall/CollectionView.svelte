<script module lang="ts">
  import type { DesignStudio as DS } from '../../lib/api/types';
  export type Scope = { kind: 'project'; id: string } | { kind: 'studio'; id: DS } | { kind: 'story'; id: string };
</script>

<script lang="ts">
  // A filtered slice of the library as a page: one project, one studio, or the
  // designs that implement one product story. Artifacts are grouped by studio;
  // the header's primary action starts a new design already filed here.
  import { untrack } from 'svelte';
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
  import { updateProject } from '../../lib/api/design';
  import type { DesignArtifact, DesignStudio } from '../../lib/api/types';
  import ArtifactCard from './ArtifactCard.svelte';
  import StudioBadge from './StudioBadge.svelte';
  import { STUDIOS, studioInfo } from './model';
  import { library } from './library.svelte';
  import { openStoryInProduct } from './nav';

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
      if (evs.some((e) => e.type !== 'design_learning_update')) library.refreshSoon();
    });
  });

  const project = $derived(scope.kind === 'project' ? library.projectOf(scope.id) : undefined);
  const story = $derived(scope.kind === 'story' ? library.stories[scope.id] : undefined);
  const studio = $derived(scope.kind === 'studio' ? studioInfo(scope.id) : null);

  const items = $derived.by((): DesignArtifact[] => {
    const live = library.hits.filter((h) => h.artifact.status !== 'archived');
    const pick =
      scope.kind === 'project'
        ? live.filter((h) => h.artifact.project_id === scope.id)
        : scope.kind === 'studio'
          ? live.filter((h) => h.artifact.studio === scope.id)
          : live.filter((h) => h.story_ids.includes(scope.id));
    return pick.map((h) => h.artifact).sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  });
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
  const notFound = $derived(library.loaded && scope.kind === 'project' && !project);

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
      toasts.error('Couldn’t rename the project', e instanceof Error ? e.message : String(e));
    }
  }

  async function archiveProject(): Promise<void> {
    if (!project) return;
    const ok = await confirmer.ask(
      `Archive the project “${project.name}”? Its designs stay in the library (unchanged) and can be filed elsewhere.`,
      { title: 'Archive project', confirmLabel: 'Archive' },
    );
    if (!ok) return;
    try {
      await updateProject(project.id, { archived: true });
      toasts.success('Project archived');
      router.go('design');
    } catch (e) {
      toasts.error('Couldn’t archive the project', e instanceof Error ? e.message : String(e));
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
  {#if library.loading && !library.loaded}
    <Skeleton rows={3} height={180} />
  {:else if library.error && !library.loaded}
    <div class="err" role="alert">
      <Icon name="warning" size={14} /> Couldn’t load the design library. <span class="dim">{library.error}</span>
      <button class="btn small" onclick={() => void library.load()}>Retry</button>
    </div>
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
        <ArtifactCard artifact={a} stories={library.storyKeys(a.id)} referenceCount={library.hitOf(a.id)?.reference_count ?? 0} live={i < 16} />
      {/each}
    </div>
  {/if}
</PageBody>

<style>
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
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: var(--fs-s);
  }
  .err > :global(svg) {
    color: var(--danger);
  }
  .dim {
    color: var(--text-dim);
    font-weight: 400;
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
