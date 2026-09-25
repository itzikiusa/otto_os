<script lang="ts">
  import { api } from '../../lib/api/client';
  import type { Connection, NetworkEndpoint, NetworkProfile, NetworkProfileInput } from '../../lib/api/types';
  let { workspaceId, profiles, onsaved }: { workspaceId: string; profiles: NetworkProfile[]; onsaved: (profile: NetworkProfile) => void } = $props();
  let connections = $state<Connection[]>([]);
  let error = $state('');
  let busy = $state(false);
  let editing = $state<NetworkProfile | null>(null);
  let name = $state('');
  let sshId = $state('');
  let endpoints = $state<NetworkEndpoint[]>([emptyEndpoint()]);
  let archived = $state(false);
  let alive = true;
  $effect(() => () => { alive = false; });
  function emptyEndpoint(): NetworkEndpoint { return { name: '', remote_host: '', remote_port: 5432, host_env: '', port_env: '' }; }
  $effect(() => {
    const scope = workspaceId;
    let alive = true;
    void api.get<Connection[]>(`/workspaces/${scope}/connections`).then((rows) => {
      if (alive) connections = rows.filter((c) => c.kind === 'ssh');
    }).catch((e) => { if (alive) error = e instanceof Error ? e.message : String(e); });
    return () => { alive = false; };
  });
  function edit(profile: NetworkProfile | null) {
    editing = profile; name = profile?.name ?? ''; sshId = profile?.ssh_connection_id ?? '';
    endpoints = profile?.endpoints.map((e) => ({...e})) ?? [emptyEndpoint()];
    archived = profile?.archived ?? false; error = '';
  }
  async function reloadSaved() {
    if (!editing) return;
    busy = true; error = '';
    try { const profile = await api.get<NetworkProfile>(`/network-profiles/${editing.id}`); if (alive) edit(profile); }
    catch (e) { if (alive) error = e instanceof Error ? e.message : String(e); }
    finally { busy = false; }
  }
  async function save() {
    busy = true; error = '';
    const payload: NetworkProfileInput = { name: name.trim(), ssh_connection_id: sshId, archived,
      endpoints: endpoints.map((e) => ({...e, name: e.name.trim(), remote_host: e.remote_host.trim(), remote_port: Number(e.remote_port), host_env: e.host_env?.trim() || null, port_env: e.port_env?.trim() || null})) };
    try {
      const saved = editing
        ? await api.put<NetworkProfile>(`/network-profiles/${editing.id}`, {...payload, version: editing.version})
        : await api.post<NetworkProfile>(`/workspaces/${workspaceId}/network-profiles`, payload);
      if (alive) { edit(saved); onsaved(saved); }
    } catch (e) { error = e instanceof Error ? e.message : String(e); }
    finally { busy = false; }
  }
</script>
<div class="manager">
  <p>Forward explicit TCP endpoints through an existing SSH connection. Programs must use the localhost addresses or mapped environment variables.</p>
  <div class="profiles">
    {#each profiles as profile (profile.id)}<button type="button" disabled={busy} onclick={() => edit(profile)} aria-label={`Edit ${profile.name}`}>{profile.name}{profile.archived ? ' (archived)' : ''}</button>{/each}
    <button type="button" disabled={busy} onclick={() => edit(null)}>New network profile</button>
  </div>
  <label>Network profile name<input aria-label="Network profile name" bind:value={name} disabled={busy} maxlength="120" /></label>
  <label>SSH connection<select aria-label="SSH connection" bind:value={sshId} disabled={busy}>
    <option value="">Choose SSH connection</option>
    {#each connections as connection (connection.id)}<option value={connection.id}>{connection.name}</option>{/each}
  </select></label>
  {#if !connections.length}<p>Create an SSH connection in Connections before saving a network profile.</p>{/if}
  {#each endpoints as endpoint, i (i)}
    <fieldset disabled={busy}><legend>Endpoint {i + 1}</legend>
      <label>Name<input aria-label={`Endpoint ${i + 1} name`} bind:value={endpoint.name} placeholder="DB" /></label>
      <label>Remote host<input aria-label={`Endpoint ${i + 1} remote host`} bind:value={endpoint.remote_host} placeholder="db.internal" /></label>
      <label>Remote port<input aria-label={`Endpoint ${i + 1} remote port`} type="number" min="1" max="65535" bind:value={endpoint.remote_port} /></label>
      <label>Host environment (optional)<input aria-label={`Endpoint ${i + 1} host environment`} bind:value={endpoint.host_env} placeholder="PGHOST or APP_HOST" /></label>
      <label>Port environment (optional)<input aria-label={`Endpoint ${i + 1} port environment`} bind:value={endpoint.port_env} placeholder="PGPORT or APP_PORT" /></label>
      <button type="button" disabled={endpoints.length === 1} title={endpoints.length === 1 ? 'A profile needs at least one endpoint' : undefined} onclick={() => endpoints = endpoints.filter((_, n) => n !== i)}>Remove endpoint {i + 1}</button>
    </fieldset>
  {/each}
  <button type="button" disabled={busy || endpoints.length >= 8} title={endpoints.length >= 8 ? 'Up to 8 endpoints per profile' : undefined} onclick={() => endpoints = [...endpoints, emptyEndpoint()]}>Add endpoint</button>
  <p>Otto also sets OTTO_TUNNEL_&lt;NAME&gt;_HOST and _PORT. TLS server names and topology discovery may need application configuration.</p>
  {#if editing}<label class="archive"><input type="checkbox" bind:checked={archived} disabled={busy} />Archived (unavailable for new launches)</label>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <div class="profiles"><button type="button" disabled={busy || !name.trim() || !sshId || endpoints.some((e) => !e.name.trim() || !e.remote_host.trim() || !e.remote_port)} onclick={save}>{busy ? 'Saving…' : 'Save network profile'}</button>
  {#if editing}<button type="button" disabled={busy} onclick={reloadSaved}>Reload saved profile</button>{/if}
  <button type="button" disabled={busy} onclick={() => edit(null)}>Cancel edits</button></div>
</div>
<style>
  .manager { display: grid; gap: 8px; min-width: 0; max-height: 60vh; overflow-y: auto; padding: 8px; border: 1px solid var(--border); border-radius: 5px; }
  label { display: grid; gap: 4px; font-size: 12px; min-width: 0; } fieldset { display: grid; gap: 6px; min-width: 0; border: 1px solid var(--border); }
  input, select, button { min-width: 0; max-width: 100%; box-sizing: border-box; padding: 6px; color: var(--text); background: var(--surface-2); border: 1px solid var(--border); border-radius: 4px; }
  .profiles { display: flex; flex-wrap: wrap; gap: 5px; } .archive { display: flex; align-items: center; } p { font-size: 12px; color: var(--text-dim); margin: 3px 0; overflow-wrap: anywhere; } .error { color: var(--danger); }
</style>
