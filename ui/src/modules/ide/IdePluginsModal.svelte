<script lang="ts">
  // Runtime IDE Extensions & Plugins modal for the Otto Full IDE.
  // Installs, enables/disables, and removes declarative IDE packs (languages, LSP servers,
  // DAP debuggers, gutter test detectors, keymaps, snippets) at runtime with zero rebuild.
  import Modal from '../../lib/components/Modal.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import Switch from '../../lib/components/Switch.svelte';
  import PathField from '../../lib/components/PathField.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import {
    ide,
    type IdePluginCatalogEntry,
    type IdePluginInfo,
  } from './ideStore.svelte';

  interface Props {
    open?: boolean;
    onclose?: () => void;
  }

  let { open, onclose = () => (ide.pluginsOpen = false) }: Props = $props();

  type PluginTab = 'installed' | 'catalog' | 'source';

  const isOpen = $derived(open === undefined ? ide.pluginsOpen : open);

  let activeTab = $state<PluginTab>('installed');
  let customSource = $state('');
  let sourceError = $state('');
  let installingSlug = $state<string | null>(null);
  let installingSource = $state(false);
  let busyToggleSlug = $state<string | null>(null);

  function close(): void {
    ide.pluginsOpen = false;
    onclose();
  }

  $effect(() => {
    if (!isOpen) return;
    sourceError = '';
    void ide.loadPlugins();
  });

  const installedList = $derived<IdePluginInfo[]>(ide.plugins);
  const catalogList = $derived<IdePluginCatalogEntry[]>(ide.pluginCatalog);

  const pluginTabs = $derived<readonly TabItem<PluginTab>[]>([
    {
      id: 'installed',
      label: 'Installed',
      count: installedList.length > 0 ? installedList.length : undefined,
    },
    {
      id: 'catalog',
      label: 'Catalog',
      count: catalogList.length > 0 ? catalogList.length : undefined,
    },
    { id: 'source', label: 'Install from path / Git' },
  ]);

  function isSlugInstalled(slug: string): boolean {
    return installedList.some((p) => p.slug === slug);
  }

  function languageLabels(plugin: IdePluginInfo): string[] {
    const rich = plugin.contributions?.languages ?? [];
    return rich.map((l) => {
      const exts = (l.extensions ?? []).slice(0, 3).join(', ');
      return exts ? `${l.display_name} (${exts})` : l.display_name;
    });
  }

  function lspLabels(plugin: IdePluginInfo): string[] {
    return (plugin.contributions?.lsp_servers ?? []).map((s) => `${s.language}: ${s.command}`);
  }

  function dapLabels(plugin: IdePluginInfo): string[] {
    return (plugin.contributions?.dap_adapters ?? []).map(
      (d) => `${d.language}: ${d.adapter_name}`,
    );
  }

  function gutterLabels(plugin: IdePluginInfo): string[] {
    return (plugin.contributions?.gutter_detectors ?? []).map(
      (g) => `${g.language} (${g.kind ?? 'test'})`,
    );
  }

  function keymapLabels(plugin: IdePluginInfo): string[] {
    return (plugin.contributions?.keymaps ?? []).map((k) => `${k.key} → ${k.action}`);
  }

  async function installFromCatalog(slug: string): Promise<void> {
    installingSlug = slug;
    try {
      const ok = await ide.installPlugin(`catalog:${slug}`);
      if (ok) {
        activeTab = 'installed';
      }
    } finally {
      installingSlug = null;
    }
  }

  function validateSource(raw: string): string | null {
    const val = raw.trim();
    if (!val) return 'Enter a local directory path, catalog:<slug>, or an https:// Git repository URL.';
    if (val.startsWith('-')) return 'Source cannot start with a hyphen (-).';
    if (/^(http|ssh|git):\/\//i.test(val) || val.startsWith('git@')) {
      return 'Remote Git repositories must use an https:// URL.';
    }
    return null;
  }

  async function handleInstallFromSource(): Promise<void> {
    sourceError = '';
    const err = validateSource(customSource);
    if (err) {
      sourceError = err;
      return;
    }
    installingSource = true;
    try {
      const ok = await ide.installPlugin(customSource.trim());
      if (ok) {
        customSource = '';
        activeTab = 'installed';
      }
    } finally {
      installingSource = false;
    }
  }

  async function handleTogglePlugin(plugin: IdePluginInfo, enabled: boolean): Promise<void> {
    busyToggleSlug = plugin.slug;
    try {
      const ok = await ide.togglePlugin(plugin.slug, enabled);
      if (ok) {
        toasts.success(
          enabled ? `Enabled ${plugin.name}` : `Disabled ${plugin.name}`,
          enabled
            ? 'Language, LSP, debugger, and gutter contributions are active immediately.'
            : 'Contributions deactivated without restarting Otto.',
        );
      }
    } finally {
      busyToggleSlug = null;
    }
  }

  async function confirmRemovePlugin(plugin: IdePluginInfo): Promise<void> {
    const ok = await confirmer.ask(
      `Remove IDE extension "${plugin.name}" (${plugin.slug})? Its contributed languages, LSP servers, debuggers, and gutter detectors will unload immediately.`,
      {
        title: 'Remove IDE extension?',
        confirmLabel: 'Remove extension',
        danger: true,
      },
    );
    if (!ok) return;
    const removed = await ide.removePlugin(plugin.slug);
    if (removed) {
      toasts.success(`Removed ${plugin.name}`);
    }
  }
</script>

{#if isOpen}
  <Modal
    title="IDE extensions & plugins"
    width={720}
    dismissable={!installingSource}
    onclose={close}
  >
    <div class="pl-shell">
      <div class="pl-banner">
        <span class="pl-banner-icon" aria-hidden="true">
          <Icon name="layers" size={16} />
        </span>
        <div class="pl-banner-copy">
          <span class="pl-banner-title">Zero-rebuild runtime extension packs</span>
          <p class="pl-sub">
            Extensions activate languages, LSP servers, DAP debuggers, gutter test runners, and keymaps immediately — no daemon or app rebuild required.
          </p>
        </div>
      </div>

      <Tabs
        label="IDE plugin views"
        tabs={pluginTabs}
        value={activeTab}
        onchange={(id) => {
          sourceError = '';
          activeTab = id;
        }}
        idBase="ide-plugins"
      />

      <LoadState
        what="IDE extensions"
        loading={ide.pluginsLoading}
        error={ide.pluginsError}
        empty={false}
        onretry={() => ide.loadPlugins()}
      >
        {#if activeTab === 'installed'}
          <div
            class="pl-pane"
            role="tabpanel"
            id="ide-plugins-panel-installed"
            aria-labelledby="ide-plugins-tab-installed"
          >
            {#if installedList.length === 0}
              <EmptyState
                icon="layers"
                title="No IDE extensions installed yet"
                body="Install 1-click support for Kotlin, Makefile & Docker, GraphQL, or Protobuf/gRPC from the Catalog, or load your own manifest."
                actionLabel="Browse catalog"
                actionKind="secondary"
                onaction={() => (activeTab = 'catalog')}
              />
            {:else}
              <div class="pl-list">
                {#each installedList as plugin (plugin.slug)}
                  {@const langs = languageLabels(plugin)}
                  {@const lsps = lspLabels(plugin)}
                  {@const daps = dapLabels(plugin)}
                  {@const gutters = gutterLabels(plugin)}
                  {@const keymaps = keymapLabels(plugin)}
                  <article class="pl-item" class:is-disabled={!plugin.enabled}>
                    <div class="pl-item-head">
                      <div class="pl-item-title-row">
                        <h3 class="pl-item-name">{plugin.name}</h3>
                        <Badge tone="neutral" variant="outline" label={`v${plugin.version}`} />
                        <code class="pl-mono" dir="ltr">{plugin.slug}</code>
                        <Badge
                          tone={plugin.enabled ? 'ok' : 'neutral'}
                          dot
                          label={plugin.enabled ? 'Enabled' : 'Disabled'}
                        />
                      </div>

                      <div class="pl-item-actions">
                        <Switch
                          checked={plugin.enabled}
                          disabled={busyToggleSlug === plugin.slug}
                          label="Enable {plugin.name}"
                          onchange={(next) => handleTogglePlugin(plugin, next)}
                        />
                        <button
                          type="button"
                          class="btn small danger"
                          onclick={() => confirmRemovePlugin(plugin)}
                        >
                          <Icon name="trash" size={12} />
                          Remove…
                        </button>
                      </div>
                    </div>

                    {#if plugin.description}
                      <p class="pl-desc">{plugin.description}</p>
                    {/if}

                    <div class="pl-contribs">
                      {#each langs as lang (lang)}
                        <Badge tone="accent" label={`Lang · ${lang}`} />
                      {/each}
                      {#each lsps as lsp (lsp)}
                        <Badge tone="info" label={`LSP · ${lsp}`} />
                      {/each}
                      {#each daps as dap (dap)}
                        <Badge tone="warn" label={`Debugger · ${dap}`} />
                      {/each}
                      {#each gutters as g (g)}
                        <Badge tone="ok" variant="outline" label={`Gutter · ${g}`} />
                      {/each}
                      {#each keymaps as km (km)}
                        <Badge tone="neutral" variant="outline" label={`Keymap · ${km}`} />
                      {/each}
                    </div>

                    {#if plugin.install_path || plugin.source}
                      <div class="pl-meta-line">
                        <span class="pl-sub">Source:</span>
                        <code class="pl-mono-dim" dir="ltr">
                          {plugin.install_path || plugin.source}
                        </code>
                      </div>
                    {/if}
                  </article>
                {/each}
              </div>
            {/if}
          </div>
        {:else if activeTab === 'catalog'}
          <div
            class="pl-pane"
            role="tabpanel"
            id="ide-plugins-panel-catalog"
            aria-labelledby="ide-plugins-tab-catalog"
          >
            {#if catalogList.length === 0}
              <EmptyState
                icon="box"
                title="Catalog unavailable"
                body="Refresh to load Otto’s built-in IDE extension catalog."
                actionLabel="Refresh"
                actionKind="secondary"
                onaction={() => ide.loadPlugins()}
              />
            {:else}
              <div class="pl-catalog-grid">
                {#each catalogList as entry (entry.slug)}
                  {@const installed = Boolean(entry.installed) || isSlugInstalled(entry.slug)}
                  {@const tags = entry.tags ?? []}
                  <article class="pl-item">
                    <div class="pl-item-head">
                      <div class="pl-item-title-row">
                        <h3 class="pl-item-name">{entry.name}</h3>
                        <Badge tone="neutral" variant="outline" label={`v${entry.version}`} />
                      </div>
                      {#if installed}
                        <Badge tone="ok" dot label="Installed" />
                      {:else}
                        <button
                          type="button"
                          class="btn small"
                          disabled={installingSlug === entry.slug}
                          onclick={() => installFromCatalog(entry.slug)}
                        >
                          <Icon name="download" size={12} />
                          {installingSlug === entry.slug ? 'Installing…' : 'Install'}
                        </button>
                      {/if}
                    </div>

                    <code class="pl-mono-dim" dir="ltr">{entry.slug}</code>
                    <p class="pl-desc">{entry.description}</p>

                    {#if tags.length > 0}
                      <div class="pl-contribs">
                        {#each tags as tag (tag)}
                          <Badge tone="neutral" variant="outline" label={tag} />
                        {/each}
                      </div>
                    {/if}
                  </article>
                {/each}
              </div>
            {/if}
          </div>
        {:else}
          <div
            class="pl-pane"
            role="tabpanel"
            id="ide-plugins-panel-source"
            aria-labelledby="ide-plugins-tab-source"
          >
            <section class="pl-source-box" aria-labelledby="pl-src-heading">
              <h3 id="pl-src-heading" class="pl-item-name">
                Install from local folder or Git repository
              </h3>
              <p class="pl-sub">
                Point to a folder containing <code class="pl-mono" dir="ltr">otto-ide-plugin.json</code> (or <code class="pl-mono" dir="ltr">otto-plugin.json</code> with an <code class="pl-mono" dir="ltr">"ide"</code> section), or an <code class="pl-mono" dir="ltr">https://</code> Git repository URL.
              </p>

              <div class="field">
                <label for="pl-source-input">Plugin folder path or HTTPS Git URL</label>
                <PathField bind:value={customSource} start={ide.target.root}>
                  <input
                    id="pl-source-input"
                    class="input pl-mono"
                    type="text"
                    dir="ltr"
                    placeholder="/path/to/my-ide-plugin or https://github.com/org/otto-ide-pack.git"
                    bind:value={customSource}
                    onkeydown={(e) => {
                      if (e.key === 'Enter') {
                        e.preventDefault();
                        void handleInstallFromSource();
                      }
                    }}
                  />
                </PathField>
              </div>

              {#if sourceError}
                <div class="pl-err" use:announceShown={{ text: sourceError, assertive: true }}>
                  {sourceError}
                </div>
              {/if}

              <div class="pl-manifest-hint">
                <span class="pl-sub">
                  Supported contributions: <strong>languages</strong>, <strong>lsp_servers</strong>, <strong>dap_adapters</strong>, <strong>gutter_detectors</strong>, <strong>keymaps</strong>, <strong>snippets</strong>, and <strong>panels</strong>.
                </span>
              </div>
            </section>
          </div>
        {/if}
      </LoadState>
    </div>

    {#snippet footer()}
      {#if activeTab === 'source'}
        <button type="button" class="btn ghost" disabled={installingSource} onclick={close}>
          Cancel
        </button>
        <button
          type="button"
          class="btn primary"
          disabled={installingSource}
          onclick={handleInstallFromSource}
        >
          {installingSource ? 'Installing…' : 'Install'}
        </button>
      {:else}
        <button type="button" class="btn ghost" onclick={close}>Done</button>
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  .pl-shell {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-inline-size: 0;
  }

  .pl-banner {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 12px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    border: 1px solid var(--border);
  }

  .pl-banner-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    inline-size: 30px;
    block-size: 30px;
    border-radius: var(--radius-s);
    background: var(--accent-soft);
    color: var(--accent-text);
    flex-shrink: 0;
  }

  .pl-banner-copy {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-inline-size: 0;
  }

  .pl-banner-title,
  .pl-item-name {
    margin: 0;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }

  .pl-sub {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }

  .pl-pane {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-block-start: 4px;
    min-inline-size: 0;
  }

  .pl-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .pl-catalog-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
  }

  .pl-item,
  .pl-source-box {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    border: 1px solid var(--border);
    min-inline-size: 0;
  }

  .pl-item.is-disabled {
    border-style: dashed;
  }

  .pl-item-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .pl-item-title-row,
  .pl-item-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }

  .pl-desc {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }

  .pl-contribs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .pl-meta-line {
    display: flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
  }

  .pl-mono,
  .pl-mono-dim {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }

  .pl-mono-dim {
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-inline-size: 0;
  }

  .pl-manifest-hint {
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--surface);
    border: 1px solid var(--border);
  }

  .pl-manifest-hint strong {
    color: var(--text);
    font-weight: 600;
  }

  .pl-err {
    font-size: var(--fs-xs);
    color: var(--danger);
  }

  @media (max-width: 640px) {
    .pl-catalog-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
