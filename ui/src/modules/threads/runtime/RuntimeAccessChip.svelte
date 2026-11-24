<script lang="ts">
  // The composer's access chip: how freely the agent may act from the next
  // turn on. Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
  // apps/web/src/components/chat/runtimeModeConfig.ts (labels, icons,
  // descriptions, accessModes.ts). Full access asks once (Otto's confirmer)
  // and stays in the warning colour while picked.
  import Icon from '../../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import { ACCESS, accessMenu } from './accessModes';
  import type { RuntimeAccessMode } from '../../../lib/api/types';

  interface Props {
    mode: RuntimeAccessMode;
    /** Who acts and where, for the full-access confirm. */
    agent: string;
    cwd: string;
    disabled?: boolean;
    /** A read-only thread (council seats and their judges, `meta.read_only`):
     *  a static "Read-only" chip — no menu, the mode cannot change what it does. */
    readOnly?: boolean;
    onpick: (mode: RuntimeAccessMode) => void;
  }
  let { mode, agent, cwd, disabled = false, readOnly = false, onpick }: Props = $props();

  const cur = $derived(ACCESS[mode]);

  function openMenu(e: MouseEvent): void {
    ctxMenu.showAt(e.currentTarget as HTMLElement, accessMenu(mode, agent, cwd, onpick));
  }
</script>

{#if readOnly}
  <span
    class="ac static"
    role="note"
    aria-label="Read-only thread"
    title="Compared threads only read. Start a new thread to change files."
    data-testid="runtime-access"
    data-readonly="true"
  >
    <Icon name="lock" size={13} />
    <span class="ellipsis">Read-only</span>
  </span>
{:else}
<button
  type="button"
  class="ac"
  class:full={mode === 'full-access'}
  aria-haspopup="menu"
  aria-label="Access for the next turn: {cur.label}"
  title={cur.description}
  data-testid="runtime-access"
  {disabled}
  onclick={openMenu}
>
  <Icon name={cur.icon} size={13} />
  <span class="ellipsis">{cur.label}</span>
</button>
{/if}

<style>
  .ac {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    block-size: 28px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .ac.static {
    cursor: default;
  }
  .ac:not(.static):hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .ac.full,
  .ac.full:hover:not(:disabled) {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .ac:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  @media (pointer: coarse) {
    .ac {
      block-size: var(--hit-min);
    }
  }
</style>
