<script lang="ts">
  // Workspace allowlists and policy-as-code stay close to the server registry,
  // but behind a side drawer so the common server/tool flow remains compact.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import type { McpServerDetail } from '../../lib/api/types';
  import AllowlistsTab from './AllowlistsTab.svelte';
  import PoliciesTab from './PoliciesTab.svelte';

  interface Props {
    wsId: string;
    servers: McpServerDetail[];
    onClose: () => void;
  }
  let { wsId, servers, onClose }: Props = $props();

  let allowlistsOpen = $state(true);
  let policiesOpen = $state(false);

  $effect(() => {
    untrack(() => ui.pushModal());
    return () => untrack(() => ui.popModal());
  });

  function onKeydown(e: KeyboardEvent): void {
    if (e.key !== 'Escape') return;
    if (document.querySelectorAll('[role="dialog"]').length > 1) return;
    e.stopPropagation();
    onClose();
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="backdrop" role="presentation" onclick={onClose}></div>
<div
  class="drawer"
  role="dialog"
  aria-modal="true"
  aria-label="Rules"
  data-testid="mcp-rules-drawer"
>
  <header class="drawer-head">
    <h2>Rules</h2>
    <button class="icon-btn" onclick={onClose} aria-label="Close rules" title="Close (Esc)">
      <Icon name="x" size={14} />
    </button>
  </header>

  <section class="rule-section">
    <button
      class="section-head"
      type="button"
      aria-expanded={allowlistsOpen}
      onclick={() => (allowlistsOpen = !allowlistsOpen)}
    >
      <Icon name={allowlistsOpen ? 'chevronDown' : 'chevronRight'} size={13} />
      <span>Allowlists</span>
    </button>
    {#if allowlistsOpen}
      <div class="section-body"><AllowlistsTab {wsId} {servers} /></div>
    {/if}
  </section>

  <section class="rule-section">
    <button
      class="section-head"
      type="button"
      aria-expanded={policiesOpen}
      onclick={() => (policiesOpen = !policiesOpen)}
    >
      <Icon name={policiesOpen ? 'chevronDown' : 'chevronRight'} size={13} />
      <span>Policies</span>
    </button>
    {#if policiesOpen}
      <div class="section-body"><PoliciesTab {wsId} {servers} /></div>
    {/if}
  </section>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 220;
    background: rgba(0, 0, 0, 0.35);
    animation: fade-in 140ms ease-out;
  }
  .drawer {
    position: fixed;
    inset: 0 0 0 auto;
    z-index: 221;
    width: min(720px, 100vw);
    max-width: 100vw;
    max-height: 100vh;
    overflow-y: auto;
    background: var(--surface);
    border-inline-start: 1px solid var(--border);
    box-shadow: var(--shadow);
    animation: drawer-in 160ms ease-out;
  }
  .drawer-head {
    position: sticky;
    top: 0;
    z-index: 2;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .rule-section {
    border-bottom: 1px solid var(--border);
  }
  .section-head {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 12px 16px;
    border: none;
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
    text-align: start;
    cursor: pointer;
  }
  .section-head:hover {
    background: color-mix(in srgb, var(--text-dim) 9%, transparent);
  }
  .section-body {
    min-width: 0;
  }
  @keyframes fade-in {
    from { opacity: 0; }
  }
  @keyframes drawer-in {
    from { transform: translateX(16px); opacity: 0; }
  }
</style>
