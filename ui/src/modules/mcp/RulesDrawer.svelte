<script lang="ts">
  // Workspace allowlists and policy-as-code stay close to the server registry:
  // the Servers section's Detail pane (inline beside the list while it keeps
  // 480 px, else a right sheet — resizable, Esc, focus return).
  import Icon from '../../lib/components/Icon.svelte';
  import DetailPane from '../../lib/components/side/DetailPane.svelte';
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
</script>

<DetailPane open module="mcp" pane="rules" title="Rules" onclose={onClose} testid="mcp-rules-drawer">
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
</DetailPane>

<style>
  .rule-section {
    border-bottom: 1px solid var(--border);
  }
  .section-head {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
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
