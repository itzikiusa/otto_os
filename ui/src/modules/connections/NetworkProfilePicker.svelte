<script lang="ts">
  import { api } from '../../lib/api/client';
  import type { NetworkProfile } from '../../lib/api/types';
  import NetworkProfiles from './NetworkProfiles.svelte';
  let { workspaceId, value = '', onchange, editable = false, disabled = false }: {
    workspaceId: string; value?: string; onchange: (id: string) => void; editable?: boolean; disabled?: boolean;
  } = $props();
  let profiles = $state<NetworkProfile[]>([]);
  let loading = $state(false);
  let error = $state('');
  let managing = $state(false);
  let reload = $state(0);
  $effect(() => {
    const scope = workspaceId; void reload;
    let alive = true;
    profiles = []; loading = true; error = ''; managing = false;
    void api.get<NetworkProfile[]>(`/workspaces/${scope}/network-profiles`).then((rows) => { if (alive) profiles = rows; })
      .catch((e) => { if (alive) error = e instanceof Error ? e.message : String(e); }).finally(() => { if (alive) loading = false; });
    return () => { alive = false; };
  });
  function saved(profile: NetworkProfile) {
    profiles = [...profiles.filter((p) => p.id !== profile.id), profile];
    if (!profile.archived) onchange(profile.id);
    else if (value === profile.id) onchange('');
  }
</script>
<div class="network-picker">
  <label>Network profile<select aria-label="Network profile" value={value} disabled={disabled || loading} onchange={(e) => onchange(e.currentTarget.value)}>
    <option value="">None — direct connection</option>
    {#each profiles.filter((p) => !p.archived) as profile (profile.id)}<option value={profile.id}>{profile.name}</option>{/each}
    {#if value && !profiles.some((p) => p.id === value && !p.archived)}<option value={value} disabled>Selected profile unavailable</option>{/if}
  </select></label>
  {#if editable}<button type="button" aria-expanded={managing} disabled={disabled || loading} onclick={() => managing = !managing}>Manage network profiles</button>{/if}
  {#if error}<p role="alert">{error} <button type="button" onclick={() => reload++}>Retry loading profiles</button></p>{/if}
  {#if managing}{#key workspaceId}<NetworkProfiles {workspaceId} {profiles} onsaved={saved} />{/key}{/if}
</div>
<style>
  .network-picker { display: grid; gap: 7px; min-width: 0; margin-block: 10px; }
  label { display: grid; gap: 5px; font-size: 12px; }
  select, button { min-width: 0; max-width: 100%; padding: 6px; color: var(--text); background: var(--surface-2); border: 1px solid var(--border); border-radius: 4px; }
  p { font-size: 12px; color: var(--danger); overflow-wrap: anywhere; }
</style>
