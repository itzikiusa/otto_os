<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { SessionNetwork } from '../../lib/api/types';
  import NetworkProfilePicker from './NetworkProfilePicker.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  let { sessionId, workspaceId, selectedProfileId = '', editable = false, manageEditable = false, defaultOpen = false, onchange }: {
    sessionId: string; workspaceId: string; selectedProfileId?: string; editable?: boolean; manageEditable?: boolean;
    /** Start expanded (opened on purpose from the session's ⋯ menu). */
    defaultOpen?: boolean;
    onchange?: (id: string) => Promise<void>;
  } = $props();
  let network = $state<SessionNetwork | null>(null);
  let error = $state('');
  let open = $state(untrack(() => defaultOpen));
  let busy = $state(false);
  let refresh = $state(0);
  // Status enum → words ("disabled" means no profile: a direct connection).
  const STATUS_LABEL: Record<SessionNetwork['status'], string> = {
    connected: 'Connected', error: 'Error', stopped: 'Stopped', disabled: 'Direct',
  };
  $effect(() => {
    const id = sessionId; void selectedProfileId; void refresh;
    let alive = true;
    let pending = false;
    async function poll() {
      if (pending || document.hidden) return;
      pending = true;
      try {
        const next = await api.get<SessionNetwork>(`/sessions/${id}/network`);
        if (alive) { network = next; error = ''; }
      } catch (e) { if (alive) error = e instanceof Error ? e.message : String(e); }
      finally { pending = false; }
    }
    void poll();
    const timer = setInterval(() => { if (open || selectedProfileId || network?.profile_id) void poll(); }, 5000);
    return () => { alive = false; clearInterval(timer); };
  });
  async function choose(id: string) {
    if (!onchange) return;
    busy = true;
    try { await onchange(id); refresh++; }
    catch (e) { error = e instanceof Error ? e.message : String(e); }
    finally { busy = false; }
  }
</script>
<details class="network" bind:open data-testid="network-status">
  <summary aria-expanded={open}>
    <span class="chev" class:open aria-hidden="true"><Icon name="chevronRight" size={12} /></span>
    <span class="sum-text">Network: {network?.profile_name || (selectedProfileId ? 'selected profile' : 'none')} · {network ? (STATUS_LABEL[network.status] ?? network.status) : 'Loading…'}{#if network?.restart_required}{' · '}<span class="warn">Restart required</span>{/if}</span>
  </summary>
  <div class="contents">
    {#if network?.status === 'connected'}<p>SSH forwards ready. Service health is not checked.</p>{/if}
    {#if network?.error}<p class="error" role="alert">{network.error} Restart the session to rebuild its forwards.</p>{/if}
    {#if network?.restart_required}<p>Restart required to apply the selected profile or its latest settings.</p>{/if}
    {#each network?.endpoints ?? [] as endpoint (endpoint.name)}
      <div class="endpoint"><strong>{endpoint.name}</strong><code>{endpoint.host}:{endpoint.port} → {endpoint.remote_host}:{endpoint.remote_port}</code>
        <code>OTTO_TUNNEL_{endpoint.name.toUpperCase()}_HOST={endpoint.host}</code><code>OTTO_TUNNEL_{endpoint.name.toUpperCase()}_PORT={endpoint.port}</code>
        {#if endpoint.host_env}<code>{endpoint.host_env}={endpoint.host}</code>{/if}
        {#if endpoint.port_env}<code>{endpoint.port_env}={endpoint.port}</code>{/if}
      </div>
    {/each}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <button type="button" class="btn small" onclick={() => refresh++}>Refresh network status</button>
    {#if editable && onchange}
      <NetworkProfilePicker {workspaceId} value={selectedProfileId} onchange={(id) => void choose(id)} editable={manageEditable} disabled={busy} />
      <p>Profile changes apply on the next restart.</p>
    {/if}
  </div>
</details>
<style>
  .network { flex-shrink: 0; min-width: 0; border-block-end: 1px solid var(--border); font-size: var(--fs-xs); }
  summary { cursor: pointer; padding: 5px 9px; display: flex; align-items: center; gap: 4px; color: var(--text-dim); list-style: none; }
  summary::-webkit-details-marker { display: none; }
  summary:hover { color: var(--text); }
  .chev { display: inline-flex; flex-shrink: 0; transition: transform 120ms ease-out; }
  .chev.open { transform: rotate(90deg); }
  :global([dir='rtl']) .chev:not(.open) { transform: scaleX(-1); }
  .sum-text { min-width: 0; overflow-wrap: anywhere; }
  .warn { color: var(--warning); }
  @media (prefers-reduced-motion: reduce) { .chev { transition: none; } }
  .contents { max-height: 40vh; overflow: auto; padding: 5px 9px; } .endpoint { display: grid; gap: 3px; margin-block: 8px; }
  code { white-space: normal; overflow-wrap: anywhere; } p { margin: 6px 0; } .error { color: var(--danger); }
</style>
