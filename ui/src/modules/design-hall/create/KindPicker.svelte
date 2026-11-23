<script lang="ts">
  // "Choose what to make myself": four big choices. Character and Object show
  // real pictures Otto Forge made (a knight and a chest); Voice and Sound show
  // plain icons — nothing drawn that could pass for a result.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { createStore } from './createStore.svelte';
  import { router } from '../../../lib/router.svelte';
  import { createHref } from './plan';
  import knight from './art/knight.jpg';
  import chest from './art/chest.jpg';

  const KINDS = [
    { id: 'character', label: 'Person', hint: 'Someone who can move and talk', img: knight, icon: null, to: createHref('new', 'character') },
    { id: 'object', label: 'Thing', hint: 'Like a sword or a chest', img: chest, icon: null, to: createHref('new', 'object') },
    { id: 'voice', label: 'Voice', hint: 'Make someone talk', img: null, icon: 'mic', to: createHref('voice') },
    { id: 'sound', label: 'Sound & music', hint: 'Noises and songs for a place', img: null, icon: 'volume', to: createHref('sound') },
  ] as const;

  $effect(() => {
    untrack(() => {
      if (createStore.soundReady === null) void createStore.loadSoundEngine();
    });
  });
</script>

<h1 class="cr-h1">What do you want to make?</h1>
<p class="cr-sub">Pick one thing.</p>
<div class="kp">
  {#each KINDS as k (k.id)}
    <button type="button" class="kp-card" class:kp-later={k.id === 'sound' && createStore.soundReady === false} onclick={() => router.go(k.to)} data-testid={`create-kind-${k.id}`}>
      <span class="kp-pic" aria-hidden="true">
        {#if k.img}<img src={k.img} alt="" />{:else if k.icon}<Icon name={k.icon} size={24} />{/if}
      </span>
      <b>{k.label}</b>
      <small>{k.id === 'sound' && createStore.soundReady === false ? 'Not ready yet' : k.hint}</small>
    </button>
  {/each}
</div>

<style>
  .kp {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: var(--sp-3);
  }
  .kp-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-3);
    border: 2px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    text-align: center;
    cursor: pointer;
    box-shadow: var(--shadow-card);
  }
  .kp-later {
    opacity: 0.6;
  }
  .kp-card:hover,
  .kp-card:focus-visible {
    border-color: var(--accent-line-strong);
    outline: none;
  }
  .kp-pic {
    display: grid;
    place-items: center;
    inline-size: 100%;
    aspect-ratio: 1;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text-dim);
    overflow: hidden;
  }
  .kp-pic img {
    inline-size: 100%;
    block-size: 100%;
    object-fit: cover;
  }
  .kp-card b {
    font-size: var(--fs-l);
  }
  .kp-card small {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  @media (max-width: 640px) {
    .kp {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
  @media (max-height: 500px) and (orientation: landscape) {
    .kp {
      grid-template-columns: repeat(4, minmax(0, 1fr));
    }
    .kp-pic {
      aspect-ratio: 4 / 3;
    }
  }
</style>
