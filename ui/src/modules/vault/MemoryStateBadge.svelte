<script lang="ts">
  // State chip for the governance lifecycle of a memory.
  // Renders a compact badge coloured by state; clicking it opens a dropdown.
  import type { MemoryState } from './vault.svelte';
  import type { Memory } from '../../lib/api/types';
  import { vault } from './vault.svelte';

  interface Props {
    memory: Memory;
  }
  let { memory }: Props = $props();

  // The `state` field is new in 0056 — older rows from the API may not have it;
  // cast through `unknown` to satisfy TypeScript (the server always sends it now).
  const curState = $derived((memory as unknown as Record<string, string>).state ?? 'accepted');

  let open = $state(false);

  const STATES: MemoryState[] = ['suggested', 'accepted', 'stale', 'contradicted'];

  function color(s: string): string {
    switch (s) {
      case 'suggested':
        return '#fab005';
      case 'accepted':
        return '#40c057';
      case 'stale':
        return '#868e96';
      case 'contradicted':
        return '#fa5252';
      default:
        return '#74c0fc';
    }
  }

  async function pick(s: MemoryState) {
    open = false;
    await vault.setState(memory, s);
  }
</script>

<div class="state-wrap">
  <button
    class="state-chip"
    style:background={color(curState)}
    onclick={() => (open = !open)}
    title="Lifecycle state — click to change"
  >
    {curState}
  </button>
  {#if open}
    <ul class="state-menu" role="menu">
      {#each STATES as s (s)}
        <li>
          <button
            role="menuitem"
            class="state-opt"
            class:current={s === curState}
            style:--dot-color={color(s)}
            onclick={() => pick(s)}
          >
            <span class="dot"></span>{s}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .state-wrap {
    position: relative;
    display: inline-block;
  }
  .state-chip {
    font-size: 9px;
    font-weight: 600;
    padding: 2px 7px;
    border-radius: 4px;
    color: #000;
    border: none;
    cursor: pointer;
    letter-spacing: 0.03em;
    text-transform: uppercase;
  }
  .state-menu {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 100;
    background: var(--surface, #1a1a1a);
    border: 1px solid var(--border, #333);
    border-radius: 6px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
    list-style: none;
    margin: 0;
    padding: 4px 0;
    min-width: 130px;
  }
  .state-opt {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    padding: 6px 12px;
    font-size: 12px;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text, #ddd);
    text-align: left;
  }
  .state-opt:hover,
  .state-opt.current {
    background: var(--surface-2, #1e2330);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--dot-color);
    flex-shrink: 0;
  }
</style>
