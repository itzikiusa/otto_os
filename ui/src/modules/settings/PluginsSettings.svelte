<script lang="ts">
  // Runtime custom-plugins management (root). Install from a local path or git
  // URL, enable/disable (spawns/stops the sidecar), remove. Access for non-root
  // users is granted per-plugin in Settings → Users.
  import { onMount } from 'svelte';
  import { api } from '../../lib/api/client';
  import { plugins, type PluginRecord } from '../../lib/stores/plugins.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  let list = $state<PluginRecord[]>([]);
  let source = $state('');
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function load() {
    error = null;
    try {
      list = await api.get<PluginRecord[]>('/plugin-admin');
    } catch (e) {
      error = String(e);
    }
  }
  onMount(load);

  async function install() {
    if (!source.trim()) return;
    busy = true;
    error = null;
    try {
      await api.post('/plugin-admin/install', { source: source.trim() });
      source = '';
      await load();
      await plugins.load();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function toggle(p: PluginRecord) {
    busy = true;
    error = null;
    try {
      await api.post(`/plugin-admin/${p.slug}/${p.enabled ? 'disable' : 'enable'}`);
      await load();
      await plugins.load();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function remove(p: PluginRecord) {
    if (!window.confirm(`Remove plugin "${p.name}"? Its files under ~/otto-plugins are kept.`)) return;
    busy = true;
    error = null;
    try {
      await api.del(`/plugin-admin/${p.slug}`);
      await load();
      await plugins.load();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="page">
  <h1>Plugins</h1>
  <p class="lead">
    Custom plugins are external sidecar processes installed at runtime (no app rebuild).
    Install from a local folder or a git URL, then enable to run it. Grant non-root users
    access to a plugin in <strong>Settings → Users</strong>.
  </p>

  <div class="install">
    <input
      placeholder="Local path (e.g. ~/otto-plugins/dora-metrics) or git URL"
      bind:value={source}
      onkeydown={(e) => e.key === 'Enter' && install()}
    />
    <button class="btn primary" onclick={install} disabled={busy || !source.trim()}>Install</button>
  </div>

  {#if error}<div class="error">{error}</div>{/if}

  {#if list.length === 0}
    <p class="empty">No plugins installed.</p>
  {:else}
    <table>
      <thead>
        <tr><th>Plugin</th><th>Slug</th><th>Version</th><th>Status</th><th></th></tr>
      </thead>
      <tbody>
        {#each list as p (p.slug)}
          <tr>
            <td>
              <div class="name"><Icon name={p.icon} size={14} /> {p.name}</div>
              <div class="src">{p.source}</div>
            </td>
            <td><code>{p.slug}</code></td>
            <td>{p.version || '—'}</td>
            <td>
              <span class="badge" class:on={p.enabled}>{p.enabled ? 'enabled' : 'disabled'}</span>
            </td>
            <td class="actions">
              <button class="btn" onclick={() => toggle(p)} disabled={busy}>
                {p.enabled ? 'Disable' : 'Enable'}
              </button>
              <button class="btn danger" onclick={() => remove(p)} disabled={busy}>Remove</button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .page {
    padding: 20px 24px;
    max-width: 900px;
  }
  h1 {
    font-size: 20px;
    margin: 0 0 6px;
  }
  .lead {
    color: var(--text-dim);
    font-size: 13px;
    margin: 0 0 16px;
  }
  .install {
    display: flex;
    gap: 8px;
    margin-bottom: 14px;
  }
  input {
    flex: 1;
    padding: 7px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-elev, transparent);
    color: var(--text);
    font-size: 13px;
  }
  .btn {
    padding: 7px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-elev, transparent);
    color: var(--text);
    font-size: 13px;
    cursor: pointer;
  }
  .btn.primary {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
  }
  .btn.danger {
    color: #e5484d;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .error {
    color: #e5484d;
    border: 1px solid color-mix(in srgb, #e5484d 40%, transparent);
    border-radius: 6px;
    padding: 8px 12px;
    margin-bottom: 12px;
  }
  .empty {
    color: var(--text-dim);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th,
  td {
    text-align: left;
    padding: 8px;
    border-bottom: 1px solid var(--border);
    vertical-align: top;
  }
  th {
    color: var(--text-dim);
    font-weight: 600;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 500;
  }
  .src {
    color: var(--text-dim);
    font-size: 11px;
    margin-top: 2px;
  }
  .badge {
    font-size: 11px;
    padding: 1px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
  }
  .badge.on {
    background: color-mix(in srgb, #30a46c 24%, transparent);
    color: #4cc38a;
  }
  .actions {
    display: flex;
    gap: 6px;
  }
</style>
