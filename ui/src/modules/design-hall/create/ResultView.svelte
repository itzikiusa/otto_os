<script lang="ts">
  // The thing itself, large — turn it with a finger, a person's moves and
  // skeleton under it (PieceLook) — and a few big actions: Give it a voice, Change something, Save to my game, Start
  // over were trimmed: Back already starts over.
  import { untrack } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import { getArtifact } from '../../../lib/api/design';
  import type { DesignArtifact } from '../../../lib/api/types';
  import type { RigInfo } from '../../../lib/design/scene3d';
  import { loadErrorText } from '../../../lib/loadError';
  import { router } from '../../../lib/router.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import PieceLook from './PieceLook.svelte';
  import type { CreationMeta } from './createStore.svelte';
  import { createHref, savePath } from './plan';
  import { saveModel, saveProblem } from './saveToGame';

  interface Props {
    id: string;
  }
  let { id }: Props = $props();

  let artifact = $state<DesignArtifact | null>(null);
  let error = $state<string | null>(null);
  let rig = $state<RigInfo | null>(null);
  let saving = $state(false);
  const meta = $derived((artifact?.meta as { create?: CreationMeta } | undefined)?.create ?? null);
  const kind = $derived(meta?.kind ?? (rig?.hasSkeleton ? 'character' : 'object'));

  $effect(() => {
    const aid = id;
    untrack(() => void load(aid));
  });

  async function load(aid: string): Promise<void> {
    error = null;
    try {
      artifact = (await getArtifact(aid)).artifact;
    } catch (e) {
      error = loadErrorText(e);
    }
  }

  async function save(): Promise<void> {
    if (!artifact || saving) return;
    saving = true;
    try {
      await saveModel(artifact.id, (n) => savePath(kind, artifact!.title, 'glb', n));
      toasts.success('Saved', 'It’s in your game’s folder.');
    } catch (e) {
      toasts.warn('I couldn’t save it', saveProblem(e));
    } finally {
      saving = false;
    }
  }

  const ACTIONS = $derived<{ id: string; icon: IconName; label: string; hint: string; run: () => void }[]>([
    { id: 'change', icon: 'edit', label: 'Change something', hint: 'Tell me what to change', run: () => router.go(createHref('a', id, 'change')) },
    ...(kind === 'character'
      ? [{ id: 'voice', icon: 'mic' as IconName, label: 'Give it a voice', hint: 'Make it talk', run: () => router.go(createHref('voice', id)) }]
      : []),
  ]);
</script>

{#if error}
  <EmptyState variant="page" tone="error" icon="warning" title="I couldn’t open this" body={error} actionLabel="Try again" actionIcon="refresh" onaction={() => void load(id)} />
{:else if artifact}
  <h1 class="cr-h1">{artifact.title}</h1>
  <p class="cr-sub">Drag to turn it around.</p>
  <div class="cr-two">
    <div>
      <PieceLook artifactId={artifact.id} label={artifact.title} thumbnail={!artifact.thumb_blob} person={meta?.kind === 'character'} onrig={(i) => (rig = i)} />
    </div>
    <div>
      <button type="button" class="cr-primary" disabled={saving} onclick={() => void save()} data-testid="create-save">
        <Icon name="download" size={16} /> {saving ? 'Saving…' : 'Save'}
      </button>
      <div class="rv-acts">
        {#each ACTIONS as a (a.id)}
          <button type="button" class="rv-act" onclick={a.run} data-testid={`create-act-${a.id}`}>
            <span class="rv-ic" aria-hidden="true"><Icon name={a.icon} size={16} /></span>
            <b>{a.label}</b>
            <small>{a.hint}</small>
          </button>
        {/each}
      </div>
    </div>
  </div>
{:else}
  <div class="rv-calm"><span class="spinner" role="img" aria-label="Loading"></span></div>
{/if}

<style>
  .rv-calm {
    display: grid;
    justify-items: center;
    gap: var(--sp-2);
    text-align: center;
    padding-block: var(--sp-7);
    margin-block-start: var(--sp-5);
  }
  .rv-acts {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--sp-3);
    margin-block-start: var(--sp-3);
  }
  .rv-act {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--sp-1);
    min-block-size: 96px;
    padding: var(--sp-3);
    border: 1.5px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .rv-act:hover,
  .rv-act:focus-visible {
    border-color: var(--accent-line-strong);
    outline: none;
  }
  .rv-act small {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rv-ic {
    display: grid;
    place-items: center;
    inline-size: 36px;
    block-size: 36px;
    border-radius: var(--radius-m);
    background: var(--accent-soft);
    color: var(--accent-text);
  }
</style>
