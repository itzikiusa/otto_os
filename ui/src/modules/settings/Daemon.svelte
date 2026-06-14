<script lang="ts">
  // Daemon settings (root): network listener toggle + port, log path display.
  import { api } from '../../lib/api/client';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';

  interface NetworkListener {
    enabled: boolean;
    port: number;
  }

  let loading = $state(true);
  let saving = $state(false);
  let enabled = $state(false);
  let port = $state(7700);
  let allSettings: Record<string, unknown> = $state({});

  $effect(() => {
    void (async () => {
      try {
        allSettings = await api.get<Record<string, unknown>>('/settings');
        const nl = allSettings['network_listener'] as NetworkListener | undefined;
        if (nl) {
          enabled = nl.enabled;
          port = nl.port;
        }
      } catch {
        toasts.error('Could not load daemon settings');
      } finally {
        loading = false;
      }
    })();
  });

  async function save(): Promise<void> {
    saving = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        ...allSettings,
        network_listener: { enabled, port },
      });
      toasts.success('Daemon settings saved', enabled ? `Listening on 0.0.0.0:${port}` : 'Loopback only');
      if (auth.meta) auth.meta.network_listener = enabled;
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>Daemon</h1>
      <div class="sub">ottod {auth.meta?.version ?? ''} · API v{auth.meta?.api_version ?? 1}</div>
    </div>
  </div>

  {#if loading}
    <Skeleton rows={3} height={40} />
  {:else}
    <div class="section-title">Network</div>
    <div class="card pad">
      <label class="checkbox-row">
        <input type="checkbox" bind:checked={enabled} />
        Enable network listener (binds <span class="mono">0.0.0.0</span>)
      </label>
      <p class="warn-note" class:visible={enabled}>
        Anyone on your network can reach the login page. Only enable on trusted networks.
      </p>
      <div class="field" style="max-width: 160px">
        <label for="dm-port">Port</label>
        <input
          id="dm-port"
          class="input mono"
          type="number"
          min="1024"
          max="65535"
          bind:value={port}
          disabled={!enabled}
        />
      </div>
      <button class="btn primary" disabled={saving} onclick={save}>
        {saving ? 'Saving…' : 'Save'}
      </button>
    </div>

    <div class="section-title">Logs</div>
    <div class="card pad">
      <div class="row">
        <span class="dim">Log file</span>
        <span class="mono">~/Library/Logs/Otto/ottod.log</span>
      </div>
      <p class="hint-line">Rotated daily by the daemon.</p>
      <button class="btn" onclick={() => router.go('settings/logs')}>Open log viewer</button>
    </div>
  {/if}
</div>

<style>
  .card.pad {
    padding: 14px 16px;
    max-width: 520px;
    margin-bottom: 8px;
  }
  .warn-note {
    font-size: 11.5px;
    color: #b8860b;
    margin: 6px 0 12px;
    opacity: 0;
    transition: opacity 150ms ease-out;
  }
  .warn-note.visible {
    opacity: 1;
  }
  .hint-line {
    font-size: 11.5px;
    color: var(--text-dim);
    margin: 8px 0 0;
  }
</style>
