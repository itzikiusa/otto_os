<script module lang="ts">
  import type { DesignStudio as DS } from '../../lib/api/types';
  import { toastError } from '../../lib/toastError';
  export type Scope = { kind: 'project'; id: string } | { kind: 'studio'; id: DS } | { kind: 'story'; id: string };
</script>

<script lang="ts">
  // A filtered slice of the library as a page: one project, one studio, or the
  // designs that implement one product story (an epic: its stories' too). The
  // cards are a server-paged DesignGrid; the header's primary action starts a
  // new design already filed here.
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { router } from '../../lib/router.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { designBus } from '../../lib/events.svelte';
  import { updateProject } from '../../lib/api/design';
  import type { DesignStudio } from '../../lib/api/types';
  import StudioBadge from './StudioBadge.svelte';
  import DesignGrid from './DesignGrid.svelte';
  import { STUDIOS, studioInfo } from './model';
  import { library } from './library.svelte';
  import { NO_FILTERS, type GridScope } from './designGrid';
  import { openStoryInProduct } from './nav';
  import { untrack } from 'svelte';

  interface Props {
    scope: Scope;
    onnew: (init: { studio?: DesignStudio; projectId?: string | null; storyId?: string | null }) => void;
  }
  let { scope, onnew }: Props = $props();

  // The library supplies the header's project / story names (the cards are
  // paged from the server by DesignGrid). It reloads on a resync; created /
  // deleted / renamed projects reload it where they happen.
  $effect(() => {
    void designBus.resyncTick;
    untrack(() => void library.load());
  });

  const project = $derived(scope.kind === 'project' ? library.projectOf(scope.id) : undefined);
  const story = $derived(scope.kind === 'story' ? library.stories[scope.id] : undefined);
  const studio = $derived(scope.kind === 'studio' ? studioInfo(scope.id) : null);

  const gridScope = $derived.by((): GridScope =>
    scope.kind === 'project'
      ? { project_id: scope.id }
      : scope.kind === 'studio'
        ? { studio: scope.id }
        : { story_id: scope.id, include_children: true },
  );
  /** Studio filter chip (a project or story page spans several studios). */
  let only = $state<'' | DesignStudio>('');
  $effect(() => {
    void scope.id;
    untrack(() => (only = ''));
  });
  const filters = $derived({ ...NO_FILTERS, studio: only });
  let count = $state(0);
  let gridLoaded = $state(false);

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
        : 'Designs that implement this story (an epic: its stories’ too)',
  );
  const notFound = $derived(library.loaded && gridLoaded && scope.kind === 'project' && !project && count === 0 && !only);

  function newHere(): void {
    if (scope.kind === 'project') onnew({ projectId: scope.id, storyId: project?.epic_story_id ?? null });
    else if (scope.kind === 'studio') onnew({ studio: scope.id });
    else onnew({ storyId: scope.id });
  }

  async function renameProject(): Promise<void> {
    if (!project) return;
    const target = project;
    const name = await confirmer.promptText('Project name', { title: 'Rename project', confirmLabel: 'Rename', initial: target.name });
    if (!name || name === target.name) return;
    try {
      await updateProject(target.id, { name });
      void library.load();
    } catch (e) {
      toastError('Couldn’t rename the project', e);
    }
  }

  async function archiveProject(): Promise<void> {
    if (!project) return;
    const target = project;
    const ok = await confirmer.ask(
      `Archive the project “${target.name}”? Its designs stay in the library (unchanged) and can be filed elsewhere.`,
      { title: 'Archive project', confirmLabel: 'Archive', danger: false },
    );
    if (!ok) return;
    try {
      await updateProject(target.id, { archived: true });
      toasts.success('Project archived');
      if (project?.id === target.id) router.go('design');
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
    {#if studio}<StudioBadge studio={studio.id} size={20} />{/if}
  {/snippet}
  {#snippet actions()}
    {#if scope.kind === 'project' && project && canEdit}
      <button class="btn small" data-overflow="-2" data-icon="archive" onclick={archiveProject}>Archive…</button>
      <button class="btn small" data-overflow="-1" data-icon="edit" onclick={renameProject}>Rename…</button>
    {/if}
    {#if scope.kind === 'story'}
      <button class="btn small" data-icon="external" onclick={() => openStoryInProduct(scope.id)}><Icon name="external" size={12} /> Open in Product</button>
    {/if}
    {#if canEdit && count}
      <button class="btn small primary" onclick={newHere} data-testid="design-collection-new"><Icon name="plus" size={12} /> New design</button>
    {/if}
  {/snippet}
</PageHeader>

<PageBody>
  {#if studio}
    <p class="note" data-testid="design-studio-note"><Icon name="info" size={14} /> {studio.note}</p>
  {/if}
  {#if notFound}
    <EmptyState variant="page" icon="designHall" title="This project isn’t available" body="It was archived or deleted, or it belongs to a workspace you can’t view."
      actionLabel="Back to Design Hall" onaction={() => router.go('design')} />
  {:else}
    {#if scope.kind !== 'studio' && (count || only)}
      <div class="studio-filter" role="group" aria-label="Filter by studio">
        <button class="pill-toggle" class:on={only === ''} aria-pressed={only === ''} onclick={() => (only = '')}>All studios</button>
        {#each STUDIOS as s (s.id)}
          <button class="pill-toggle" class:on={only === s.id} aria-pressed={only === s.id} onclick={() => (only = s.id)}>
            <StudioBadge studio={s.id} /> {s.name}
          </button>
        {/each}
      </div>
    {/if}
    <DesignGrid scope={gridScope} {filters} homeWorkspace={project?.workspace_id} bind:count bind:loaded={gridLoaded}
      onclearfilters={() => (only = '')}>
      {#snippet empty()}
        <EmptyState
          variant="page"
          icon={scope.kind === 'studio' ? 'frame' : 'designHall'}
          title="No designs here yet"
          body={scope.kind === 'story'
            ? 'No design implements this story yet. Start one here, or link an existing design from its Links panel.'
            : 'Start a draft — it keeps every version and can link to stories and other designs.'}
          actionLabel={canEdit ? 'New design' : undefined}
          actionIcon="plus"
          onaction={canEdit ? newHere : undefined}
        />
      {/snippet}
    </DesignGrid>
  {/if}
</PageBody>

<style>
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
</style>
