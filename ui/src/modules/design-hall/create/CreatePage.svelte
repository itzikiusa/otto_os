<script lang="ts">
  // Create (`#/design/create…`): the plain-words front door for making
  // characters, things, voices and sounds — the first tab of Product. Smart
  // mode is the default: one box, "Make it", and the app plans and makes the
  // whole set. "Choose what to make myself" is the one-thing path. Experts
  // keep the 3D Studio and Voice Studio ("More options", one quiet link on the first screen).
  import { untrack } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import PageBody from '../../../lib/components/PageBody.svelte';
  import PageHeader from '../../../lib/components/PageHeader.svelte';
  import ProductModeTabs from '../../../lib/components/ProductModeTabs.svelte';
  import { router } from '../../../lib/router.svelte';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { createStore } from './createStore.svelte';
  import { createHref, parseCreateRoute } from './plan';
  import SmartHome from './SmartHome.svelte';
  import ProductionView from './ProductionView.svelte';
  import KindPicker from './KindPicker.svelte';
  import DescribeStep from './DescribeStep.svelte';
  import JobView from './JobView.svelte';
  import ResultView from './ResultView.svelte';
  import ChangeView from './ChangeView.svelte';
  import CompareView from './CompareView.svelte';
  import Gallery from './Gallery.svelte';
  import VoiceCreate from './VoiceCreate.svelte';
  import SoundCreate from './SoundCreate.svelte';

  const route = $derived(parseCreateRoute(router.parts.slice(2)));
  const wsId = $derived(ws.currentId);
  const canMake = $derived(auth.can('design', 'edit'));

  $effect(() => {
    const id = wsId;
    untrack(() => {
      void createStore.loadEngine();
      if (id) void createStore.resume(id);
    });
  });

  const back = (): void => router.back();
</script>

<div class="create-page">
  <!-- Create sits in the normal app frame like every Product tab: the shared
       header with the mode tabs, and the Create screen as the page body. -->
  <PageHeader title="Product" tabsPlacement="start">
    {#snippet tabs()}
      <ProductModeTabs active="create" />
    {/snippet}
    {#snippet actions()}
      {#if route.view !== 'mine'}
        <button type="button" class="btn small" data-keep onclick={() => router.go(createHref('mine'))} data-testid="create-mine">
          <Icon name="folder" size={12} /> My creations
        </button>
      {/if}
    {/snippet}
  </PageHeader>

  <PageBody>
    <div class="create-body">
      {#if route.view !== 'home'}
        <!-- In the body, not the header: the mode tabs never move. -->
        <button type="button" class="cr-back" onclick={back} data-testid="create-back"><Icon name="chevronLeft" size={16} /> Back</button>
      {/if}
      {#if !wsId}
        <EmptyState variant="page" icon="folder" title="Pick a workspace first" body="Things you make are kept in a workspace. Choose one at the top of the sidebar." />
      {:else if !canMake}
        <EmptyState variant="page" icon="lock" title="You can look, but not make things here" body="Ask a workspace admin to let you edit designs." />
      {:else if route.view === 'home'}
        {#key route.project}<SmartHome project={route.project ?? null} />{/key}
      {:else if route.view === 'production'}
        {#key route.id}<ProductionView id={route.id} peek={!!route.scene} />{/key}
      {:else if route.view === 'kinds'}
        <KindPicker />
      {:else if route.view === 'describe'}
        <DescribeStep kind={route.kind} />
      {:else if route.view === 'job'}
        {#key route.id}<JobView id={route.id} />{/key}
      {:else if route.view === 'result'}
        {#key route.id}<ResultView id={route.id} />{/key}
      {:else if route.view === 'change'}
        {#key route.id}<ChangeView id={route.id} />{/key}
      {:else if route.view === 'compare'}
        <CompareView id={route.id} before={route.before} />
      {:else if route.view === 'voice'}
        <VoiceCreate who={route.who} />
      {:else if route.view === 'sound'}
        <SoundCreate />
      {:else}
        <Gallery />
      {/if}
    </div>
  </PageBody>
</div>

<style>
  .create-page {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
  }
  .create-body {
    /* Every control in Create is at least a finger wide, on every pointer. */
    --cr-hit: 44px;
    max-inline-size: 960px;
    inline-size: 100%;
    margin-inline: auto;
    padding-block-end: var(--sp-8);
  }
  .cr-back {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    min-block-size: var(--cr-hit);
    padding-inline: var(--sp-2);
    margin-inline-start: calc(-1 * var(--sp-2));
    border: 0;
    border-radius: var(--radius-m);
    background: none;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-m);
    cursor: pointer;
  }
  .cr-back:focus-visible {
    outline: 3px solid var(--accent-text);
  }
  /* Shared Create vocabulary: big type, big targets, calm cards. */
  .create-body :global(.btn) {
    min-block-size: var(--cr-hit);
  }
  .create-body :global(.cr-h1) {
    font-size: var(--fs-2xl);
    line-height: 1.2;
    margin: var(--sp-4) 0 var(--sp-2);
  }
  .create-body :global(.cr-h2) {
    font-size: var(--fs-l);
    margin: var(--sp-6) 0 var(--sp-3);
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .create-body :global(.cr-sub) {
    color: var(--text-dim);
    font-size: var(--fs-m);
    margin: 0 0 var(--sp-4);
  }
  .create-body :global(.cr-card) {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow-card);
    padding: var(--sp-4);
  }
  .create-body :global(.cr-field) {
    inline-size: 100%;
    min-block-size: 140px;
    border: 1.5px solid var(--control-border);
    border-radius: var(--radius-m);
    padding: var(--sp-3);
    font: inherit;
    font-size: var(--fs-l);
    line-height: 1.4;
    background: var(--bg);
    color: var(--text);
    resize: vertical;
  }
  .create-body :global(.cr-field:focus-visible) {
    outline: 3px solid var(--accent-soft-strong);
    border-color: var(--accent-line-strong);
  }
  .create-body :global(.cr-primary) {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    inline-size: 100%;
    min-block-size: 56px;
    border: 0;
    border-radius: var(--radius-l);
    background: var(--accent-solid);
    color: var(--accent-contrast);
    font: inherit;
    font-size: var(--fs-l);
    font-weight: 600;
    cursor: pointer;
  }
  /* "Not yet": plainly quiet, not a washed-out blue that looks broken. */
  .create-body :global(.cr-primary:disabled) {
    background: var(--surface-3);
    color: var(--text-dim);
    cursor: not-allowed;
  }
  .create-body :global(.cr-secondary) {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    min-block-size: 52px;
    padding-inline: var(--sp-4);
    border: 1.5px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    cursor: pointer;
  }
  .create-body :global(.cr-primary:focus-visible),
  .create-body :global(.cr-secondary:focus-visible),
  .create-body :global(.cr-quiet:focus-visible) {
    outline: 3px solid var(--accent-text);
    outline-offset: 2px;
  }
  /* The main button sits after the content (never on top of it). */
  .create-body :global(.cr-sticky) {
    margin-block-start: var(--sp-4);
  }
  .create-body :global(.cr-quiet) {
    display: block;
    margin: var(--sp-3) auto 0;
    min-block-size: var(--cr-hit);
    padding-inline: var(--sp-3);
    border: 0;
    background: none;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-m);
    text-decoration: underline;
    cursor: pointer;
  }
  .create-body :global(.cr-chips) {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
    margin-block-start: var(--sp-3);
  }
  .create-body :global(.cr-chip) {
    min-block-size: var(--cr-hit);
    padding: var(--sp-2) var(--sp-3);
    border: 1.5px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .create-body :global(.cr-chip:hover),
  .create-body :global(.cr-chip[aria-pressed='true']) {
    border-color: var(--accent-line-strong);
    background: var(--accent-soft);
  }
  .create-body :global(.cr-tile) {
    flex: none;
    display: grid;
    place-items: center;
    inline-size: 56px;
    block-size: 56px;
    border: 1.5px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text-dim);
    overflow: hidden;
  }
  .create-body :global(.cr-tile img) {
    inline-size: 100%;
    block-size: 100%;
    object-fit: cover;
  }
  .create-body :global(.cr-row) {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    flex-wrap: wrap;
  }
  .create-body :global(.cr-two) {
    display: grid;
    grid-template-columns: minmax(0, 1.25fr) minmax(0, 1fr);
    gap: var(--sp-4);
    align-items: start;
  }
  .create-body :global(.cr-stage) {
    aspect-ratio: 1;
    inline-size: 100%;
  }
  @media (max-width: 1024px) {
    .create-body :global(.cr-two) {
      grid-template-columns: minmax(0, 1fr);
    }
    .create-body :global(.cr-stage) {
      aspect-ratio: 4 / 3;
    }
  }
  /* A phone on its side: the thing on the left, the words on the right, both
     on screen at once. */
  @media (max-height: 500px) and (orientation: landscape) {
    .create-body :global(.cr-two) {
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    }
    .create-body :global(.cr-stage) {
      aspect-ratio: auto;
      block-size: max(200px, calc(100dvh - 160px));
    }
    .create-body :global(.cr-h1) {
      margin-block-start: 0;
    }
  }
  .create-body :global(.cr-note) {
    display: flex;
    gap: var(--sp-2);
    align-items: flex-start;
    background: var(--warning-soft);
    color: var(--text);
    border-radius: var(--radius-m);
    padding: var(--sp-3);
    font-size: var(--fs-m);
  }
</style>
