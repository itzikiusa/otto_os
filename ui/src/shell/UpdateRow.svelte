<script lang="ts">
  // The quiet sidebar row above Help: "Update to 0.5.4" → "Downloading… 42 %"
  // → "Restart to update" (or "Update failed and was undone · Details").
  // Desktop app + root + release build only (lib/stores/updates.svelte.ts);
  // nothing renders otherwise. `rail`: the collapsed rail's one icon with a dot.
  import Icon from '../lib/components/Icon.svelte';
  import { auth } from '../lib/stores/auth.svelte';
  import { isTauri } from '../lib/desktop';
  import { updates } from '../lib/stores/updates.svelte';

  let { rail = false }: { rail?: boolean } = $props();
  const label = $derived(isTauri && auth.isRoot ? updates.row : null);
  const failed = $derived(!!updates.failed);
</script>

{#if label}
  {#if rail}
    <button class="rail-update" class:failed onclick={() => updates.openDialog()} title={label} aria-label={label} data-testid="update-row">
      <Icon name={failed ? 'warning' : 'download'} />
      <span class="dot" aria-hidden="true"></span>
    </button>
  {:else}
    <button class="nav-update" class:failed onclick={() => updates.openDialog()} title={failed ? 'Show what went wrong' : 'Show the update'} data-testid="update-row">
      <Icon name={failed ? 'warning' : 'download'} size={14} />
      <span class="grow">{label}{#if failed}<span class="details"> · Details</span>{/if}</span>
    </button>
  {/if}
{/if}

<style>
  .nav-update {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 28px;
    padding: 0 8px;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    color: var(--accent-text);
    font-size: var(--fs-m);
    cursor: pointer;
    text-align: start;
    transition: background var(--dur-fast) ease-out;
  }
  .nav-update:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  .nav-update > .grow {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .failed {
    color: var(--warning);
  }
  .details {
    color: var(--text-dim);
  }
  .rail-update {
    position: relative;
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    color: var(--text-dim);
    cursor: pointer;
    flex-shrink: 0;
  }
  .rail-update:hover {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text);
  }
  .dot {
    position: absolute;
    top: 4px;
    inset-inline-end: 4px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent-solid);
  }
  .rail-update.failed .dot {
    background: var(--warning);
  }
  @media (pointer: coarse), (max-width: 640px) {
    .nav-update {
      height: auto;
      min-block-size: 40px;
    }
  }
</style>
