<script lang="ts">
  // Build Tools, Debugger, Shell Environment, Remote SSH Targets, and
  // Plugins & Language Servers sections for the Unified IDE Settings modal.
  import { api } from '../../../lib/api/client';
  import type { Connection } from '../../../lib/api/types';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Switch from '../../../lib/components/Switch.svelte';
  import Tabs, { type TabItem } from '../../../lib/components/Tabs.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { ide } from '../ideStore.svelte';
  import IdeShellEnvCard from '../shell/IdeShellEnvCard.svelte';
  import { ideSettings } from './ideSettingsStore.svelte';

  interface Props {
    section: string;
  }

  let { section }: Props = $props();

  const LSP_SERVERS: readonly { name: string; eco: string; lang: string }[] = [
    { name: 'gopls', eco: 'go', lang: 'Go' },
    { name: 'jdtls', eco: 'java', lang: 'Java' },
    { name: 'typescript-language-server', eco: 'node', lang: 'TypeScript / JS' },
    { name: 'pyright', eco: 'python', lang: 'Python' },
    { name: 'rust-analyzer', eco: 'rust', lang: 'Rust' },
    { name: 'bash-language-server', eco: 'shell', lang: 'Shell (zsh / bash)' },
  ];

  let sshConns = $state<Connection[]>([]);
  let sshLoading = $state(false);
  let sshRemotePath = $state('~');
  let plTab = $state<'installed' | 'catalog' | 'source'>('installed');
  let plSource = $state('');
  let plBusy = $state(false);

  $effect(() => {
    if (section === 'build.ssh') void fetchSshConnections();
    if (section === 'plugins' && ide.plugins.length === 0 && !ide.pluginsLoading) {
      void ide.loadPlugins();
    }
  });

  async function fetchSshConnections(): Promise<void> {
    sshLoading = true;
    try {
      const scopes = Array.from(
        new Set(
          [ide.target.workspace_id, ws.currentId, ...ws.workspaces.map((w) => w.id), SCRATCH_WORKSPACE_ID].filter(
            (id): id is string => Boolean(id),
          ),
        ),
      );
      const settled = await Promise.allSettled(
        scopes.map((id) => api.get<Connection[]>(`/workspaces/${encodeURIComponent(id)}/connections`)),
      );
      const seen = new Map<string, Connection>();
      for (const r of settled) {
        if (r.status === 'fulfilled') {
          for (const c of r.value ?? []) {
            if (c.kind === 'ssh') seen.set(c.id, c);
          }
        }
      }
      sshConns = Array.from(seen.values());
    } finally {
      sshLoading = false;
    }
  }

  function lspInstalled(ecoId: string): boolean {
    if (ecoId === 'shell') return true;
    const eco = ide.toolchains.find((t) => t.ecosystem === ecoId);
    return Boolean(eco?.lsp_status?.available ?? eco?.lsp?.installed);
  }

  function addEnvOverride(): void {
    ideSettings.patchEnv({
      env_vars: [...ideSettings.envDraft.env_vars, { key: '', value: '', secret: false }],
    });
  }

  function updateEnvVar(idx: number, field: 'key' | 'value', val: string): void {
    const next = ideSettings.envDraft.env_vars.map((v, i) => (i === idx ? { ...v, [field]: val } : v));
    ideSettings.patchEnv({ env_vars: next });
  }

  function removeEnvVar(idx: number): void {
    ideSettings.patchEnv({ env_vars: ideSettings.envDraft.env_vars.filter((_, i) => i !== idx) });
  }

  function mergePastedVars(items: { key: string; value: string }[]): void {
    const map = new Map(ideSettings.envDraft.env_vars.map((v) => [v.key, v]));
    for (const it of items) map.set(it.key, { key: it.key, value: it.value, secret: false });
    ideSettings.patchEnv({ env_vars: Array.from(map.values()) });
  }

  const pluginTabs = $derived<readonly TabItem<'installed' | 'catalog' | 'source'>[]>([
    { id: 'installed', label: 'Installed', count: ide.plugins.length || undefined },
    { id: 'catalog', label: 'Marketplace / Catalog', count: ide.pluginCatalog.length || undefined },
    { id: 'source', label: 'Install from Path / Git' },
  ]);

  async function installFromSource(): Promise<void> {
    const src = plSource.trim();
    if (!src) return;
    plBusy = true;
    try {
      const ok = await ide.installPlugin(src);
      if (ok) {
        plSource = '';
        plTab = 'installed';
      }
    } finally {
      plBusy = false;
    }
  }

  async function confirmRemovePlugin(slug: string, name: string): Promise<void> {
    const ok = await confirmer.ask(`Remove IDE extension "${name}"?`, {
      title: 'Remove IDE extension?',
      confirmLabel: 'Remove',
      danger: true,
    });
    if (ok) await ide.removePlugin(slug);
  }
</script>

{#if section === 'build.tools'}
  <div class="bt-sec" data-testid="ide-settings-build-tools">
    <div class="bt-card">
      <span class="bt-title">Java Build Tool (Maven / Gradle)</span>
      <select class="input" aria-label="Default Java build tool" bind:value={ideSettings.draft.defaultBuildToolJava}>
        <option value="auto">Auto-detect (pom.xml → Maven, build.gradle → Gradle)</option>
        <option value="maven">Maven (mvn)</option>
        <option value="gradle">Gradle (./gradlew / gradle)</option>
      </select>
    </div>

    <div class="bt-card">
      <div class="bt-sw-row">
        <Switch checked={ideSettings.draft.goModulesEnabled} label="Enable Go Modules" onchange={(v) => (ideSettings.draft.goModulesEnabled = v)} />
        <span class="bt-title">Go Modules (<code>go.mod</code> / multi-repo <code>replace</code> resolution)</span>
      </div>
      <div class="bt-sw-row">
        <Switch checked={ideSettings.draft.cargoCheckOnSave} label="Enable Cargo check on save" onchange={(v) => (ideSettings.draft.cargoCheckOnSave = v)} />
        <span class="bt-title">Cargo — run <code>cargo check</code> diagnostics on save</span>
      </div>
      <div class="bt-field">
        <label class="bt-lbl" for="bt-npm-mgr">Node / TypeScript package manager:</label>
        <select id="bt-npm-mgr" class="input" bind:value={ideSettings.draft.npmPackageManager}>
          <option value="npm">npm</option>
          <option value="pnpm">pnpm</option>
          <option value="yarn">yarn</option>
          <option value="bun">bun</option>
        </select>
      </div>
      <div>
        <button
          type="button"
          class="btn small"
          onclick={() => { ide.settingsOpen = false; ide.openEnvironmentsModal('configs'); }}
        >
          <Icon name="play" size={12} />
          Edit Run / Debug Configurations…
        </button>
      </div>
    </div>
  </div>

{:else if section === 'build.debugger'}
  <div class="bt-sec" data-testid="ide-settings-debugger">
    <div class="bt-card">
      <div class="bt-sw-row">
        <Switch
          checked={ideSettings.draft.breakOnExceptions}
          label="Break on unhandled exceptions and panics"
          onchange={(v) => (ideSettings.draft.breakOnExceptions = v)}
        />
        <span class="bt-title">Break automatically on unhandled exceptions / Go panics</span>
      </div>
      <div class="bt-sw-row">
        <Switch
          checked={ideSettings.draft.prettyJsonLogs}
          label="Pretty-print JSON log lines in console"
          onchange={(v) => (ideSettings.draft.prettyJsonLogs = v)}
        />
        <span class="bt-title">Console JSON log pretty-printing (collapsible structured log cards)</span>
      </div>
      <div class="bt-sw-row">
        <Switch
          checked={ideSettings.draft.consoleLineWrap}
          label="Soft-wrap lines in debugger and run console"
          onchange={(v) => (ideSettings.draft.consoleLineWrap = v)}
        />
        <span class="bt-title">Soft-wrap long output lines in Run &amp; Debug consoles</span>
      </div>
      <div class="bt-field">
        <span class="bt-lbl">Debugger View Layout:</span>
        <div class="segmented" role="group" aria-label="Debugger view layout">
          <button
            type="button"
            aria-pressed={ideSettings.draft.debuggerLayout === 'tabbed'}
            onclick={() => (ideSettings.draft.debuggerLayout = 'tabbed')}
          >
            Tabbed (Variables | Console)
          </button>
          <button
            type="button"
            aria-pressed={ideSettings.draft.debuggerLayout === 'split'}
            onclick={() => (ideSettings.draft.debuggerLayout = 'split')}
          >
            Split View
          </button>
        </div>
      </div>
    </div>
  </div>

{:else if section === 'build.env'}
  <div class="bt-sec" data-testid="ide-settings-shell-env">
    <IdeShellEnvCard
      inheritShellEnv={ideSettings.envDraft.inherit_shell_env}
      onchangeInherit={(next) => ideSettings.patchEnv({ inherit_shell_env: next })}
      onpasteVars={mergePastedVars}
    />

    <div class="bt-card">
      <div class="bt-field">
        <label class="bt-lbl" for="bt-dotenv-path">Optional <code>.env</code> file (relative to project root):</label>
        <input
          id="bt-dotenv-path"
          class="input mono"
          type="text"
          dir="ltr"
          placeholder=".env"
          value={ideSettings.envDraft.dotenv_path}
          oninput={(e) => ideSettings.patchEnv({ dotenv_path: (e.currentTarget as HTMLInputElement).value })}
        />
      </div>

      <div class="bt-toolbar">
        <span class="bt-title">Custom Environment Overrides ({ideSettings.envDraft.env_vars.length})</span>
        <button type="button" class="btn small ghost" onclick={addEnvOverride}>
          <Icon name="plus" size={12} />
          Add Variable
        </button>
      </div>

      {#each ideSettings.envDraft.env_vars as row, idx (idx)}
        <div class="env-row">
          <input
            class="input mono env-in"
            type="text"
            dir="ltr"
            aria-label="Environment variable name"
            placeholder="KEY (e.g. GO_DEBUGGING_PORT)"
            value={row.key}
            oninput={(e) => updateEnvVar(idx, 'key', (e.currentTarget as HTMLInputElement).value)}
          />
          <input
            class="input mono env-in"
            type="text"
            dir="ltr"
            aria-label="Environment variable value"
            placeholder="value"
            value={row.value}
            oninput={(e) => updateEnvVar(idx, 'value', (e.currentTarget as HTMLInputElement).value)}
          />
          <button
            type="button"
            class="icon-btn"
            aria-label="Remove environment variable"
            title="Remove variable"
            onclick={() => removeEnvVar(idx)}
          >
            <Icon name="trash" size={13} />
          </button>
        </div>
      {/each}
    </div>
  </div>

{:else if section === 'build.ssh'}
  <div class="bt-sec" data-testid="ide-settings-ssh">
    <div class="bt-card">
      <div class="bt-toolbar">
        <span class="bt-title">Connect Remote SSH Target</span>
        <Badge tone={ide.target.kind === 'ssh' ? 'ok' : 'neutral'} variant="outline" label={ide.target.kind === 'ssh' ? `Connected: ${ide.target.connection_name || 'SSH'}` : 'Local Workspace'} />
      </div>
      <div class="bt-field">
        <label class="bt-lbl" for="bt-ssh-path">Remote project directory:</label>
        <input id="bt-ssh-path" class="input mono" type="text" dir="ltr" bind:value={sshRemotePath} />
      </div>

      <LoadState what="saved SSH connections" loading={sshLoading} empty={sshConns.length === 0} variant="compact">
        {#snippet emptyView()}
          <p class="bt-sub">No saved SSH connections found in Otto Connections yet.</p>
        {/snippet}
        {#each sshConns as conn (conn.id)}
          <div class="srv-row">
            <div class="srv-copy">
              <span class="srv-name" dir="auto">{conn.name}</span>
              <span class="srv-sub" dir="ltr">{String(conn.params?.host ?? conn.params?.hostname ?? 'remote')}</span>
            </div>
            <button
              type="button"
              class="btn small primary"
              onclick={() => {
                void ide.setSshTarget(conn.id, sshRemotePath || '~', conn.name);
                toasts.success(`Switched IDE target to ${conn.name}`);
                ide.settingsOpen = false;
              }}
            >
              Connect
            </button>
          </div>
        {/each}
      </LoadState>

      <div>
        <button
          type="button"
          class="btn small ghost"
          onclick={() => { ide.settingsOpen = false; ide.sshOpen = true; }}
        >
          <Icon name="server" size={12} />
          Browse Remote Directories…
        </button>
      </div>
    </div>
  </div>

{:else}
  <div class="bt-sec" data-testid="ide-settings-plugins">
    <div class="bt-card">
      <span class="bt-title">Language Servers (LSP)</span>
      <div class="lsp-grid">
        {#each LSP_SERVERS as srv (srv.name)}
          {@const ok = lspInstalled(srv.eco)}
          <div class="srv-row">
            <div class="srv-copy">
              <span class="srv-name" dir="ltr">{srv.name}</span>
              <span class="srv-sub">{srv.lang}</span>
            </div>
            <Badge tone={ok ? 'ok' : 'warn'} variant="outline" label={ok ? 'Ready' : 'Not installed'} />
            {#if !ok && srv.eco !== 'shell'}
              <button type="button" class="btn small ghost" onclick={() => void ide.installToolchain(srv.eco, 'lsp')}>
                Install
              </button>
            {/if}
          </div>
        {/each}
      </div>
    </div>

    <div class="bt-card">
      <Tabs label="IDE plugins" tabs={pluginTabs} value={plTab} onchange={(id) => (plTab = id)} size="s" />

      {#if plTab === 'installed'}
        {#if ide.plugins.length === 0}
          <p class="bt-sub">No runtime IDE plugins installed yet — browse the Marketplace tab or install from a directory / Git URL.</p>
        {:else}
          {#each ide.plugins as pl (pl.slug)}
            <div class="srv-row pl-item">
              <Switch checked={pl.enabled} label={`Enable plugin ${pl.name}`} onchange={(v) => void ide.togglePlugin(pl.slug, v)} />
              <div class="srv-copy">
                <span class="srv-name">{pl.name} <Badge tone="neutral" variant="outline" label={`v${pl.version}`} /></span>
                <span class="srv-sub">{pl.description}</span>
              </div>
              <button type="button" class="btn small ghost" onclick={() => void confirmRemovePlugin(pl.slug, pl.name)}>Remove</button>
            </div>
          {/each}
        {/if}
      {:else if plTab === 'catalog'}
        {#each ide.pluginCatalog as cat (cat.slug)}
          {@const already = ide.plugins.some((p) => p.slug === cat.slug)}
          <div class="srv-row pl-item">
            <div class="srv-copy">
              <span class="srv-name">{cat.name}</span>
              <span class="srv-sub">{cat.description}</span>
            </div>
            <button
              type="button"
              class="btn small"
              disabled={already || plBusy}
              onclick={() => void ide.installPlugin(`catalog:${cat.slug}`)}
            >
              {already ? 'Installed' : 'Install'}
            </button>
          </div>
        {/each}
      {:else}
        <div class="bt-field">
          <label class="bt-lbl" for="pl-source-input">Plugin directory path, <code>catalog:&lt;slug&gt;</code>, or <code>https://</code> Git URL:</label>
          <div class="env-row">
            <input id="pl-source-input" class="input mono env-in" type="text" dir="ltr" placeholder="catalog:go-extras or https://…" bind:value={plSource} />
            <button type="button" class="btn small primary" disabled={plBusy || !plSource.trim()} onclick={() => void installFromSource()}>
              {plBusy ? 'Installing…' : 'Install'}
            </button>
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .bt-sec { display: flex; flex-direction: column; gap: 14px; }
  .bt-card { border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface); padding: 12px; display: flex; flex-direction: column; gap: 10px; }
  .bt-toolbar { display: flex; align-items: center; justify-content: space-between; gap: 8px; flex-wrap: wrap; }
  .bt-title { font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .bt-lbl { font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); }
  .bt-sub { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); }
  .bt-field { display: flex; flex-direction: column; gap: 4px; }
  .bt-sw-row { display: flex; align-items: center; gap: 10px; }
  .mono { font-family: var(--font-mono); font-size: var(--fs-xs); }
  .env-row { display: flex; align-items: center; gap: 6px; }
  .env-in { flex: 1; min-inline-size: 0; }
  .lsp-grid { display: flex; flex-direction: column; gap: 6px; }
  .srv-row { display: flex; align-items: center; gap: 10px; padding: 6px 8px; border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface-2); }
  .srv-copy { flex: 1; min-inline-size: 0; display: flex; flex-direction: column; gap: 2px; }
  .srv-name { font-size: var(--fs-xs); font-weight: 600; color: var(--text); display: flex; align-items: center; gap: 6px; }
  .srv-sub { font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
