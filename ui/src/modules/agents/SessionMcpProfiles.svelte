<script lang="ts">
  // Session ⋯ → MCP tools: change which Otto MCP profiles this agent session
  // runs under. The profile is captured when the agent starts, so the change
  // applies on the next restart — said here, and offered as "Save & restart".
  import Modal from '../../lib/components/Modal.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { loadErrorText } from '../../lib/loadError';
  import { mcpProfiles } from '../../lib/stores/mcp-profiles.svelte';
  import { sessionProfileIds } from '../../lib/mcpProfiles';
  import McpProfilePicker from './McpProfilePicker.svelte';

  interface Props {
    sessionId: string;
    meta: Record<string, unknown> | null | undefined;
    onclose: () => void;
  }
  let { sessionId, meta, onclose }: Props = $props();

  // svelte-ignore state_referenced_locally
  let ids = $state<string[]>(sessionProfileIds(meta));
  let busy = $state(false);
  // svelte-ignore state_referenced_locally
  const before = sessionProfileIds(meta).join(',');
  const changed = $derived(ids.join(',') !== before);

  async function save(alsoRestart: boolean): Promise<void> {
    if (busy) return;
    if (alsoRestart && !(await ws.confirmRestart(sessionId))) return;
    busy = true;
    try {
      try {
        await ws.updateSessionMeta(sessionId, { mcp_profiles: ids.length ? ids : null });
      } catch (e) {
        toastError('Couldn’t save the MCP tools', e);
        return;
      }
      onclose();
      const label = mcpProfiles.labelFor(ids);
      if (!alsoRestart) {
        toasts.success(`MCP tools: ${label}`, 'Applies the next time the agent restarts.');
        return;
      }
      try {
        await ws.restartSession(sessionId, { quiet: true });
        toasts.success(`MCP tools: ${label}`, 'Agent restarted with the new tool set.');
      } catch (e) {
        toasts.push('error', 'MCP tools saved, but the agent didn’t restart', loadErrorText(e), 12000, {
          action: { label: 'Restart', run: () => ws.requestRestart(sessionId) },
        });
      }
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="MCP tools" width={480} {onclose}>
  <p class="lead">
    Pick the Otto MCP profiles this agent gets. It sees only their tools; anything else is refused with
    the profile’s name. <strong>Applies when the agent restarts.</strong>
  </p>
  <McpProfilePicker value={ids} onchange={(next) => (ids = next)} disabled={busy} idPrefix="sv-mcp-tools" />
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn" disabled={busy || !changed} onclick={() => void save(false)}>{busy ? 'Saving…' : 'Save'}</button>
    <button class="btn primary" disabled={busy || !changed} onclick={() => void save(true)} data-testid="mcp-tools-save-restart">
      {busy ? 'Saving…' : 'Save & restart'}
    </button>
  {/snippet}
</Modal>

<style>
  .lead {
    margin: 0 0 12px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
