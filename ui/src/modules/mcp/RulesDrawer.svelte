<script lang="ts">
  // Workspace allowlists and policy-as-code stay close to the server registry,
  // but behind a side drawer so the common server/tool flow remains compact.
  // The slide-over itself (scrim, Esc/outside close, focus trap, modal
  // registration) is the shared shell/Drawer.
  import Icon from '../../lib/components/Icon.svelte';
  import Drawer from '../../shell/Drawer.svelte';
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

  // The parent mounts this only while it is wanted; the Drawer closes itself
  // (✕, Esc, scrim) by flipping `open`, which unmounts us via onClose.
  let open = $state(true);
  $effect(() => {
    if (!open) onClose();
  });
</script>

<Drawer bind:open side="right" title="Rules" width="min(720px, 100%)">
  <div class="rules" data-testid="mcp-rules-drawer">
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
</Drawer>

<style>
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
    font-size: var(--fs-m);
    font-weight: 600;
    text-align: start;
    cursor: pointer;
  }
  .section-head:hover {
    background: var(--hover);
  }
  .section-body {
    min-width: 0;
  }
</style>
