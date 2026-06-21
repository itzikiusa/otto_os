<script lang="ts">
  // Create a swarm — from a preset (5 templates) or blank.
  import Modal from '../../lib/components/Modal.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { toasts } from '../../lib/toast.svelte';

  interface Props {
    onclose: () => void;
  }
  let { onclose }: Props = $props();

  let name = $state('');
  let selected = $state<string>(''); // preset slug or '' for blank
  let busy = $state(false);

  $effect(() => {
    swarm.loadPresets();
  });

  async function create() {
    if (!name.trim() || busy) return;
    busy = true;
    try {
      await swarm.createSwarm(name.trim(), selected || undefined);
      toasts.success('Swarm created');
      onclose();
    } catch (e) {
      toasts.error('Could not create swarm', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="New swarm" width={560} {onclose}>
  <div class="field">
    <label for="swarm-name">Name</label>
    <input id="swarm-name" class="input" placeholder="e.g. Platform Team" bind:value={name} />
  </div>

  <p class="section-title">Start from a preset</p>
  <div class="presets">
    <button class="preset" class:sel={selected === ''} onclick={() => (selected = '')}>
      <div class="p-name">Blank</div>
      <div class="p-desc dim">Start empty and recruit your own agents.</div>
    </button>
    {#each swarm.presets as p (p.slug)}
      <button class="preset" class:sel={selected === p.slug} onclick={() => (selected = p.slug)}>
        <div class="p-name">{p.name} <span class="dim">· {p.agents.length} agents</span></div>
        <div class="p-desc dim">{p.description}</div>
      </button>
    {/each}
  </div>

  {#snippet footer()}
    <button class="btn ghost" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={create} disabled={!name.trim() || busy}>Create</button>
  {/snippet}
</Modal>

<style>
  .presets {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-height: 320px;
    overflow-y: auto;
  }
  .preset {
    text-align: start;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 10px 12px;
    cursor: pointer;
  }
  .preset:hover {
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
  }
  .preset.sel {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .p-name {
    font-weight: 600;
    font-size: 13px;
    margin-bottom: 2px;
  }
  .p-desc {
    font-size: 11.5px;
  }
</style>
