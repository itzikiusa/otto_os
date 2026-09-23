<script lang="ts">
  // Brand Kit (`#/design/brand[/<id>]`): the workspace's `otto-brand/1` kits.
  // With an id it is the kit editor (brand/BrandEditor.svelte); without one it
  // opens the most recently edited kit (list/detail pages open on an item),
  // or — when there is none — offers the three starter kits. Studios read a
  // kit's tokens by name (`token:color.primary`); helpers in brand/tokens.ts.
  import { untrack } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { router } from '../../lib/router.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { designBus } from '../../lib/events.svelte';
  import { library } from './library.svelte';
  import BrandEditor from './brand/BrandEditor.svelte';
  import NewKitModal from './brand/NewKitModal.svelte';

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
      if (evs.some((e) => e.type === 'design_artifact_updated' && (e.format === 'otto-brand' || e.change === 'created' || e.change === 'deleted'))) {
        library.refreshSoon();
      }
    });
  });

  // `parseDesignRoute` maps every `design/brand…` to the brand view; the kit
  // id (if any) is the third route part.
  const kitId = $derived(router.parts[1] === 'brand' ? (router.parts[2] ?? null) : null);
  const kits = $derived(
    library.hits
      .filter((h) => h.artifact.format === 'otto-brand' && h.artifact.status !== 'archived')
      .sort((a, b) => b.artifact.updated_at.localeCompare(a.artifact.updated_at)),
  );
  const canEdit = $derived(auth.can('design', 'edit'));

  // No id → open the most recent kit (replace, so Back doesn't bounce here).
  $effect(() => {
    if (kitId || !library.loaded || kits.length === 0) return;
    const first = kits[0].artifact.id;
    untrack(() => router.replace(`design/brand/${encodeURIComponent(first)}`));
  });

  let creating = $state(false);
  function openNew(): void {
    if (!canEdit) {
      toasts.warn('You can view brand kits but not create them', 'Ask a workspace admin for Design Hall edit access.');
      return;
    }
    if (!ws.currentId) {
      toasts.warn('Pick a workspace first', 'A brand kit is filed under a workspace.');
      return;
    }
    creating = true;
  }
  function created(id: string): void {
    creating = false;
    void library.load();
    router.go(`design/brand/${encodeURIComponent(id)}`);
  }
</script>

{#if kitId}
  {#key kitId}
    <BrandEditor id={kitId} {kits} onnew={openNew} />
  {/key}
{:else}
  <PageHeader title="Brand Kit" subtitle="Colours, type and voice every studio uses" crumbs={[{ label: 'Design Hall', onclick: () => router.go('design') }]} />
  <PageBody>
    {#if (library.loading && !library.loaded) || (library.loaded && kits.length > 0)}
      <Skeleton rows={2} height={200} />
    {:else if library.error && !library.loaded}
      <div class="err" role="alert">
        <Icon name="warning" size={14} /> Couldn’t load brand kits. <span class="dim">{library.error}</span>
        <button class="btn small" onclick={() => void library.load()}>Retry</button>
      </div>
    {:else}
      <EmptyState
        variant="page"
        icon="palette"
        title="No brand kit yet"
        body="A brand kit holds your colours, type scale, spacing, logos and voice as one versioned document. Every studio reads its tokens by name, and a change shows which designs it reaches before you save."
        actionLabel={canEdit ? 'Create brand kit' : undefined}
        actionIcon="plus"
        onaction={canEdit ? openNew : undefined}
      />
    {/if}
  </PageBody>
{/if}

{#if creating && ws.currentId}
  <NewKitModal workspaceId={ws.currentId} onclose={() => (creating = false)} oncreated={created} />
{/if}

<style>
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
  }
</style>
