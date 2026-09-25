<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import SectionIntro from './SectionIntro.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Runtime custom-plugins management (root). Install from a local path or git
  // URL, enable/disable (spawns/stops the sidecar), remove. Access for non-root
  // users is granted per-plugin in Settings → Users.
  import { onMount } from 'svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { plugins, type PluginRecord } from '../../lib/stores/plugins.svelte';
  import Icon, { asIcon } from '../../lib/components/Icon.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { loadErrorText } from '../../lib/loadError';

  let list = $state<PluginRecord[]>([]);
  let loading = $state(true);
  let loadError = $state('');
  let source = $state('');
  // Which action is in flight: 'install', or a plugin slug (toggle/remove).
  let busy = $state<string | null>(null);
  // Install failures stay inline under the field (with the daemon's reason),
  // since the fix is usually editing the source.
  let installError = $state('');
  // Local-folder picker for the plugin source (daemon-host filesystem).
  let pickerOpen = $state(false);
  let sourceEl = $state<HTMLInputElement | null>(null);

  async function load(attempt = 0): Promise<void> {
    loading = true;
    try {
      list = await api.get<PluginRecord[]>('/plugin-admin');
      loadError = '';
    } catch (e) {
      // The daemon can be briefly unreachable right after an app update (it
      // restarts), which surfaces as a fetch "Load failed". Retry once on a fresh
      // connection before showing an error.
      if (attempt < 1) {
        await new Promise((r) => setTimeout(r, 600));
        return load(attempt + 1);
      }
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  onMount(() => void load());

  function errText(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

  async function install(): Promise<void> {
    const src = source.trim();
    if (!src || busy) return;
    busy = 'install';
    installError = '';
    try {
      await api.post('/plugin-admin/install', { source: src });
      source = '';
      toasts.success('Plugin installed', 'Enable it to start its sidecar.');
      await load();
      await plugins.load();
    } catch (e) {
      installError = `Couldn’t install from “${src}”. ${errText(e)}`;
    } finally {
      busy = null;
    }
  }

  async function toggle(p: PluginRecord): Promise<void> {
    busy = p.slug;
    try {
      await api.post(`/plugin-admin/${p.slug}/${p.enabled ? 'disable' : 'enable'}`);
      await load();
      await plugins.load();
    } catch (e) {
      toasts.error(`Couldn’t ${p.enabled ? 'disable' : 'enable'} ${p.name}`, errText(e));
    } finally {
      busy = null;
    }
  }

  async function remove(p: PluginRecord): Promise<void> {
    if (
      !(await confirmer.ask(
        `Remove the plugin “${p.name}”? Its sidecar stops and it leaves every user's sidebar. Its files under ~/otto-plugins are kept, so you can install it again.`,
        { title: 'Remove plugin', confirmLabel: 'Remove' },
      ))
    )
      return;
    busy = p.slug;
    try {
      await api.del(`/plugin-admin/${p.slug}`);
      toasts.success(`Removed ${p.name}`, `Its files are kept — install it again any time.`);
      await load();
      await plugins.load();
    } catch (e) {
      toasts.error(`Couldn’t remove ${p.name}`, errText(e));
    } finally {
      busy = null;
    }
  }

  function installedLabel(ts: string): string {
    const d = new Date(ts);
    return Number.isNaN(d.getTime()) ? '' : `Installed ${d.toLocaleDateString()}`;
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('plugins')} subtitle="Sidecar processes installed at runtime, no rebuild" />
  <PageBody width="readable">
  <SectionIntro>
    Install from a local folder or a git URL, then enable it to run. A plugin runs as its own process on this Mac —
    install only code you trust. Grant non-root users access to a plugin in <strong>Settings → Users</strong>.
  </SectionIntro>

  <div class="install">
    <label class="sr-only" for="plugin-source">Plugin source</label>
    <input
      id="plugin-source"
      dir="ltr"
      class="input grow mono-in"
      bind:this={sourceEl}
      placeholder="~/otto-plugins/dora-metrics or https://github.com/org/plugin.git"
      bind:value={source}
      spellcheck="false"
      autocomplete="off"
      aria-invalid={installError ? 'true' : undefined}
      aria-describedby={installError ? 'plugin-install-err' : undefined}
      oninput={() => (installError = '')}
      onkeydown={(e) => e.key === 'Enter' && void install()}
    />
    <button class="btn" onclick={() => (pickerOpen = true)} title="Browse for a local plugin folder on this Mac">
      <Icon name="folder" size={13} /> Browse…
    </button>
    <button
      class="btn primary"
      onclick={() => void install()}
      disabled={busy !== null || !source.trim()}
      title={source.trim() ? 'Install this plugin' : 'Enter a local path or a git URL first'}
    >
      {busy === 'install' ? 'Installing…' : 'Install'}
    </button>
  </div>
  {#if installError}<p class="field-err" id="plugin-install-err" role="alert">{installError}</p>{/if}

  <LoadState what="plugins" {loading} error={loadError} empty={list.length === 0} rows={3} onretry={() => void load()}>
    {#snippet emptyView()}
      <EmptyState
        icon="box"
        title="No plugins installed"
        body="Plugins add pages to Otto without a rebuild — dashboards, internal tools, integrations. Paste a folder path or git URL above, or Browse…, to install one."
      />
    {/snippet}
    <div class="plist">
      {#each list as p (p.slug)}
        <div class="prow" class:off={!p.enabled}>
          <span class="picon"><Icon name={asIcon(p.icon, 'box')} size={16} /></span>
          <div class="pmain">
            <div class="pname">
              <span class="name-text" title={p.name}>{p.name}</span>
              {#if p.version}<span class="chip version" dir="ltr" title="v{p.version}">v{p.version}</span>{/if}
              <StatusBadge variant="text" tone={p.enabled ? 'success' : 'neutral'} label={p.enabled ? 'Enabled' : 'Disabled'} />
            </div>
            {#if p.description}<div class="pdesc" title={p.description}>{p.description}</div>{/if}
            <div class="psrc mono" dir="ltr" title={p.source}>{p.slug} · {p.source}{#if installedLabel(p.installed_at)}{' · '}{installedLabel(p.installed_at)}{/if}</div>
          </div>
          <div class="pactions">
            <button class="btn small" onclick={() => void toggle(p)} disabled={busy !== null}>
              {busy === p.slug ? 'Working…' : p.enabled ? 'Disable' : 'Enable'}
            </button>
            <button
              class="icon-btn danger-icon"
              onclick={() => void remove(p)}
              disabled={busy !== null}
              aria-label={`Remove ${p.name}`}
              title={`Remove ${p.name}`}
            >
              <Icon name="trash" size={14} />
            </button>
          </div>
        </div>
      {/each}
    </div>
  </LoadState>
  </PageBody>
</div>

{#if pickerOpen}
  <FolderPicker
    title="Choose a local plugin folder"
    start="~/otto-plugins"
    onpick={(p) => {
      source = p;
      installError = '';
      pickerOpen = false;
      queueMicrotask(() => sourceEl?.focus());
    }}
    onclose={() => (pickerOpen = false)}
  />
{/if}

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .install {
    display: flex;
    gap: 8px;
    max-width: var(--settings-col);
    margin-bottom: 6px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .mono-in {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  .field-err {
    margin: 0 0 8px;
    max-width: var(--settings-col);
    font-size: var(--fs-s);
    color: var(--danger);
    overflow-wrap: anywhere;
  }
  .plist {
    margin-top: 14px;
    max-width: var(--settings-col);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    overflow: hidden;
  }
  .prow {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    min-width: 0;
  }
  .prow + .prow {
    border-top: 1px solid var(--border);
  }
  .picon {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .prow.off .picon {
    opacity: 0.6;
  }
  .pmain {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .pname {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    min-width: 0;
    font-weight: 600;
  }
  .name-text {
    flex-basis: 100%;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .version {
    min-width: 0;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pdesc,
  .psrc {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .pactions {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .danger-icon:hover:not(:disabled) {
    color: var(--danger);
  }
  @media (max-width: 640px) {
    .install {
      flex-wrap: wrap;
    }
    .install .input {
      flex-basis: 100%;
    }
  }
</style>
