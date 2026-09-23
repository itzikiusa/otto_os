<script lang="ts">
  // Brand Kit (Phase 0 scaffold): the workspace's brand-kit artifacts — each a
  // versioned `otto-brand` token document that opens in the artifact view
  // (swatches + contrast + source). Token editing with an impact preview and
  // "used in N artifacts" propagation land with the Brand Kit studio (Phase 1).
  import { untrack } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { router } from '../../lib/router.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { designBus } from '../../lib/events.svelte';
  import ArtifactCard from './ArtifactCard.svelte';
  import { createDesign } from './create';
  import { library } from './library.svelte';

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

  const kits = $derived(
    library.hits
      .filter((h) => h.artifact.studio === 'brand' && h.artifact.status !== 'archived')
      .sort((a, b) => b.artifact.updated_at.localeCompare(a.artifact.updated_at)),
  );
  const canEdit = $derived(auth.can('design', 'edit'));
  let creating = $state(false);

  async function create(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.warn('Pick a workspace first', 'A brand kit is filed under a workspace.');
      return;
    }
    const name = await confirmer.promptText('Name the brand kit.', {
      title: 'Create brand kit',
      confirmLabel: 'Create',
      placeholder: 'Acme brand',
    });
    if (!name) return;
    creating = true;
    try {
      const id = await createDesign({ workspaceId: wsId, studio: 'brand', format: 'otto-brand', title: name });
      router.go(`design/a/${encodeURIComponent(id)}`);
    } catch (e) {
      toasts.error('Couldn’t create the brand kit', e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }
</script>

<PageHeader title="Brand Kit" subtitle="Colours, type and voice every studio uses" crumbs={[{ label: 'Design Hall', onclick: () => router.go('design') }]}>
  {#snippet actions()}
    {#if kits.length && canEdit}
      <button class="btn small primary" onclick={create} disabled={creating}><Icon name="plus" size={12} /> New brand kit</button>
    {/if}
  {/snippet}
</PageHeader>

<PageBody>
  {#if library.loading && !library.loaded}
    <Skeleton rows={2} height={200} />
  {:else if library.error && !library.loaded}
    <div class="err" role="alert">
      <Icon name="warning" size={14} /> Couldn’t load brand kits. <span class="dim">{library.error}</span>
      <button class="btn small" onclick={() => void library.load()}>Retry</button>
    </div>
  {:else if kits.length === 0}
    <EmptyState
      variant="page"
      icon="palette"
      title="No brand kit yet"
      body="A brand kit holds your colour, type and spacing tokens as one versioned document. Studios will read tokens from it by name; the token editor with an impact preview arrives in Phase 1."
      actionLabel={canEdit ? 'Create brand kit' : undefined}
      actionIcon="plus"
      onaction={canEdit ? create : undefined}
    />
  {:else}
    <p class="note"><Icon name="info" size={14} /> Open a kit to see its swatches with contrast and edit its tokens as source. Token editing with an impact preview (“changes 14 designs”) lands in Phase 1.</p>
    <div class="cards" data-testid="design-brand-kits">
      {#each kits as h (h.artifact.id)}
        <ArtifactCard artifact={h.artifact} referenceCount={h.reference_count} />
      {/each}
    </div>
  {/if}
</PageBody>

<style>
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 12px;
  }
  .note {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 0 0 16px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .note > :global(svg) {
    color: var(--info);
    flex: none;
    margin-block-start: 1px;
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
  }
</style>
