<script lang="ts">
  // One thing, described: words (with ideas to tap) or a picture, then
  // "Make it". A character comes out complete — shape, ready for games, a
  // skeleton and basic moves — in ONE job the daemon runs to the end, so
  // leaving the screen never stops it.
  import Icon from '../../../lib/components/Icon.svelte';
  import { router } from '../../../lib/router.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { createStore, readPicture } from './createStore.svelte';
  import { KIND_EXAMPLES, WHERE, characterPrompt, createHref, estimateSecs, nameFromIdea, timeText } from './plan';
  import KitMissing from './KitMissing.svelte';

  interface Props {
    kind: 'character' | 'object';
  }
  let { kind }: Props = $props();

  let idea = $state('');
  let starting = $state(false);
  let fileEl = $state<HTMLInputElement | null>(null);
  const picture = $derived(createStore.picture);
  const word = $derived(kind === 'character' ? 'person' : 'thing');
  const can = $derived(!!picture || !!idea.trim());

  async function onPicture(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const f = input.files?.[0];
    input.value = '';
    if (!f) return;
    try {
      createStore.picture = await readPicture(f);
    } catch (err) {
      toasts.warn('That picture won’t work', err instanceof Error ? err.message : '');
    }
  }

  async function makeIt(): Promise<void> {
    if (!can || starting) return;
    const where = 'pc' as const;
    const provider = createStore.engine.provider;
    const wsId = ws.currentId;
    if (!provider || !wsId) return;
    starting = true;
    const text = idea.trim();
    const w = WHERE[where];
    try {
      const job = await createStore.start(
        {
          workspace_id: wsId,
          provider_id: provider.id,
          stage: 'generate',
          then_stages: kind === 'character' ? ['rig', 'animate'] : [],
          title: nameFromIdea(text, picture ? `${kind === 'character' ? 'Person' : 'Thing'} from a picture` : kind === 'character' ? 'Person' : 'Thing'),
          prompt: picture ? null : kind === 'character' ? characterPrompt(text) : text,
          image_b64: picture?.b64 ?? null,
          image_mime: picture?.mime ?? null,
          output_format: 'glb',
          poly_budget: w.triangles,
          preset: w.preset,
        },
        { kind, idea: text || (picture?.name ?? ''), where, picture: !!picture },
      );
      createStore.picture = null;
      router.go(createHref('job', job.id));
    } catch {
      toasts.warn('I couldn’t start making it', 'Please try again in a moment.');
    } finally {
      starting = false;
    }
  }
</script>

{#if createStore.engine.checked && !createStore.engine.provider}
  <KitMissing />
{:else}
  <h1 class="cr-h1">Describe your {word}</h1>
  <p class="cr-sub">Say it like you would to a friend — or use a picture.</p>
  <div class="cr-card">
    {#if picture}
      <div class="ds-pic">
        <img src={picture.url} alt="What you chose" />
        <button type="button" class="cr-secondary" onclick={() => (createStore.picture = null)}><Icon name="x" size={16} /> Use words instead</button>
      </div>
      <label class="ds-label" for="create-name">Give it a name (you can skip this)</label>
      <input id="create-name" class="ds-name" dir="auto" maxlength="60" bind:value={idea} placeholder={kind === 'character' ? 'Knight' : 'Treasure chest'} />
    {:else}
      <label class="ds-sr" for="create-idea-one">Describe it</label>
      <textarea id="create-idea-one" class="cr-field" dir="auto" maxlength="3000" placeholder="Describe it…" bind:value={idea} data-testid="create-idea"></textarea>
      <div class="cr-chips" aria-label="Ideas to try">
        {#each KIND_EXAMPLES[kind] as ex (ex)}
          <button type="button" class="cr-chip" onclick={() => (idea = ex)}>{ex}</button>
        {/each}
      </div>
      <button type="button" class="ds-link" onclick={() => fileEl?.click()}><Icon name="image" size={16} /> Use a picture</button>
    {/if}
    <p class="ds-time"><Icon name="clock" size={16} /> This takes {timeText(estimateSecs(kind, !!picture)).toLowerCase()}{kind === 'character' ? ', and it comes out ready to move' : ''}.
    </p>
  </div>
  <div class="cr-sticky">
    <button type="button" class="cr-primary" disabled={!can || starting} onclick={() => void makeIt()} data-testid="create-make">{starting ? 'Starting…' : 'Make it'}</button>
  </div>
{/if}

<input bind:this={fileEl} class="ds-file" type="file" accept="image/png,image/jpeg,image/webp" onchange={onPicture} tabindex="-1" aria-hidden="true" />


<style>
  .ds-label {
    display: block;
    font-weight: 600;
    margin-block: var(--sp-2);
  }
  .ds-sr {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  .ds-name {
    inline-size: 100%;
    min-block-size: 48px;
    padding: var(--sp-2) var(--sp-3);
    border: 1.5px solid var(--control-border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-l);
  }
  .ds-link {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    min-block-size: var(--cr-hit);
    margin-block-start: var(--sp-2);
    padding-inline: var(--sp-2);
    border: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-m);
    cursor: pointer;
  }
  .ds-time {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-2);
    color: var(--text-dim);
    font-size: var(--fs-m);
    margin: var(--sp-3) 0 0;
  }
  .ds-pic {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-3);
  }
  .ds-pic img {
    inline-size: min(100%, 220px);
    border-radius: var(--radius-m);
  }
  .ds-file {
    display: none;
  }
</style>
