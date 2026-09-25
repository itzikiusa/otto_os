<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Daemon settings (root): network listener toggle + port, log path display.
  import { api } from '../../lib/api/client';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';

  interface NetworkListener {
    enabled: boolean;
    port: number;
  }

  interface ProcessSandbox {
    enabled: boolean;
    network: 'full' | 'loopback' | 'none';
  }

  let loading = $state(true);
  // A failed load shows inline with Retry — never the form with its defaults
  // (listener off, sandbox off) dressed up as the real settings, one Save away
  // from writing them back.
  let loadError = $state('');
  let saving = $state(false);
  let enabled = $state(false);
  let port = $state(7700);
  let sandboxEnabled = $state(false);
  let sandboxNetwork = $state<'full' | 'loopback' | 'none'>('full');
  // Last-saved values: the ONE Save (in the header) sends only the groups
  // that differ from these, and is disabled while nothing does.
  let savedListener = $state<NetworkListener>({ enabled: false, port: 7700 });
  let savedSandbox = $state<ProcessSandbox>({ enabled: false, network: 'full' });
  const listenerDirty = $derived(enabled !== savedListener.enabled || port !== savedListener.port);
  const sandboxDirty = $derived(
    sandboxEnabled !== savedSandbox.enabled || sandboxNetwork !== savedSandbox.network,
  );
  const dirty = $derived(listenerDirty || sandboxDirty);
  // The inputs' min/max are advisory only; an out-of-range or empty port would
  // be saved as-is (and an empty one silently falls back to the loopback port).
  const portValid = $derived(Number.isInteger(port) && port >= 1024 && port <= 65535);
  const portError = $derived(enabled && !portValid ? 'Enter a whole number from 1024 to 65535.' : '');
  // Latest full settings object (the PUT response). Saves send ONLY the keys
  // they change: PUT /settings upserts exactly the keys in the body, so
  // spreading this page-load snapshot reverted keys written since (auto-update
  // last-run, MCP/PR-review settings, another window) and wrote false
  // skip-permissions / network-listener audit entries on every save.
  let allSettings: Record<string, unknown> = $state({});

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      allSettings = await api.get<Record<string, unknown>>('/settings');
      const nl = allSettings['network_listener'] as NetworkListener | undefined;
      if (nl) {
        enabled = nl.enabled;
        port = nl.port;
      }
      savedListener = { enabled, port };
      const sb = allSettings['process_sandbox'] as ProcessSandbox | undefined;
      if (sb) {
        sandboxEnabled = sb.enabled;
        sandboxNetwork = sb.network ?? 'full';
      }
      savedSandbox = { enabled: sandboxEnabled, network: sandboxNetwork };
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  async function save(): Promise<void> {
    if (!dirty || portError) return;
    // Only the changed groups: an unchanged network_listener in the body would
    // still write a network-listener audit entry.
    const body: Record<string, unknown> = {};
    if (listenerDirty) body.network_listener = { enabled, port };
    if (sandboxDirty) body.process_sandbox = { enabled: sandboxEnabled, network: sandboxNetwork };
    const saveListener = listenerDirty;
    const saveSandbox = sandboxDirty;
    saving = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', body);
      const notes: string[] = [];
      if (saveListener) {
        savedListener = { enabled, port };
        if (auth.meta) auth.meta.network_listener = enabled;
        // The listener is bound once at daemon start — say so rather than
        // claim a socket that isn't open yet.
        notes.push(
          enabled
            ? `https://0.0.0.0:${port} after the daemon restarts`
            : 'Loopback only after the daemon restarts',
        );
      }
      if (saveSandbox) {
        savedSandbox = { enabled: sandboxEnabled, network: sandboxNetwork };
        notes.push(
          sandboxEnabled
            ? `New sessions confined (network: ${sandboxNetwork})`
            : 'Sandbox off for new sessions',
        );
      }
      toasts.success('Daemon settings saved', notes.join(' · '));
    } catch (e) {
      toasts.error('Couldn’t save daemon settings', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('daemon')} subtitle={`ottod ${auth.meta?.version ?? ''} · API v${auth.meta?.api_version ?? 1}`}>
    {#snippet actions()}
      {#if !loading && !loadError}
        <button
          class="btn small primary"
          disabled={!dirty || saving || !!portError}
          title={portError || (dirty ? 'Save network and sandbox settings' : 'No changes to save')}
          onclick={() => void save()}
        >
          {saving ? 'Saving…' : 'Save'}
        </button>
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody width="readable">

  {#if loading}
    <Skeleton rows={3} height={40} />
  {:else if loadError}
    <LoadState what="daemon settings" error={loadError} empty onretry={() => void load()} />
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
      <p class="hint-line">
        Served over HTTPS with a self-signed certificate. Takes effect the next time the daemon
        starts (quit and reopen Otto).
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
          aria-invalid={!!portError}
        />
        {#if portError}<span class="hint port-error">{portError}</span>{/if}
      </div>
    </div>

    <div class="section-title">Process sandbox</div>
    <div class="card pad">
      <label class="checkbox-row">
        <input type="checkbox" bind:checked={sandboxEnabled} data-testid="sandbox-enabled" />
        Confine agent sessions with the OS sandbox (macOS Seatbelt)
      </label>
      <p class="hint-line">
        When on, spawned agent CLIs (claude / codex / agy / shell) can only write to
        the workspace, its git dir, the CLIs' own caches and temp — never the rest of
        your disk. Reads are unaffected. macOS only; ignored on other systems. Applies to
        sessions started from now on — running ones keep the confinement they started with.
        Not yet confined: custom providers, connection terminals, and background agent runs
        (workflow steps, scheduled tasks, swarms).
      </p>
      <div class="field" style="max-width: 320px">
        <label for="dm-sandbox-net">Network</label>
        <select
          id="dm-sandbox-net"
          class="input"
          bind:value={sandboxNetwork}
          disabled={!sandboxEnabled}
          data-testid="sandbox-network"
        >
          <option value="full">Full (agents reach their model API)</option>
          <option value="loopback">Loopback only</option>
          <option value="none">No network</option>
        </select>
      </div>
      <p class="warn-note" class:visible={sandboxEnabled && sandboxNetwork !== 'full'}>
        Non-`full` network blocks agent CLIs from reaching their model API — use only
        for offline shells.
      </p>
    </div>

    <div class="section-title">Logs</div>
    <div class="card pad">
      <div class="row">
        <span class="dim">Log file</span>
        <span class="mono">~/Library/Logs/Otto/ottod.log.YYYY-MM-DD</span>
      </div>
      <p class="hint-line">A new file each day; older files are kept.</p>
      <button class="btn" onclick={() => router.go('settings/logs')}>Open log viewer</button>
    </div>
  {/if}
  </PageBody>
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .card.pad {
    padding: 14px 16px;
    max-width: 520px;
    margin-bottom: 8px;
  }
  .warn-note {
    font-size: 11.5px;
    color: var(--warning);
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
  .port-error {
    color: var(--danger);
  }
</style>
