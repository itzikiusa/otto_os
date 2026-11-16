<script lang="ts">
  // SDKs, Toolchains & Debuggers Manager modal for the Otto Full IDE.
  // Detects target machine OS + CPU architecture (Local or Remote SSH), browses official
  // release versions (`include_catalog=true`), supports 1-click Download & Install with
  // live progress for SDKs, LSP servers (`gopls`, `jdtls`, `rust-analyzer`, …), and DAP
  // debuggers (`dlv`, `debugpy`, `codelldb`, …), plus Add Local SDK from Disk.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import { confirmer } from '../../lib/confirm.svelte';
  import { runStatus } from '../../lib/status';
  import { toasts } from '../../lib/toast.svelte';
  import {
    ide,
    type IdeInstalledSdk,
    type IdeInstallJob,
    type IdeToolchainEcosystem,
  } from './ideStore.svelte';

  interface Props {
    open?: boolean;
    onclose?: () => void;
  }

  let { open, onclose = () => (ide.toolchainsOpen = false) }: Props = $props();

  const BASE_ECOS: readonly { id: string; label: string }[] = [
    { id: 'go', label: 'Go' },
    { id: 'java', label: 'Java' },
    { id: 'rust', label: 'Rust' },
    { id: 'node', label: 'JavaScript / TypeScript (Node)' },
    { id: 'python', label: 'Python' },
  ];

  const isOpen = $derived(open === undefined ? ide.toolchainsOpen : open);

  let activeEco = $state<string>('go');
  let selectedVersions = $state<Record<string, string>>({});
  let expandedLogs = $state<Record<string, boolean>>({});
  let busyKey = $state<string | null>(null);
  let localSdkPickerOpen = $state(false);
  let initializedForOpen = false;

  function close(): void {
    ide.toolchainsOpen = false;
    onclose();
  }

  function defaultEcoFromWorkspace(): string {
    const lang = ide.activeTab?.language;
    if (lang === 'java') return 'java';
    if (lang === 'rust') return 'rust';
    if (lang === 'python') return 'python';
    if (lang === 'typescript' || lang === 'javascript') return 'node';
    if (lang === 'go') return 'go';
    const names = new Set(ide.treeRootEntries.map((e) => e.name));
    if (names.has('go.mod')) return 'go';
    if (names.has('pom.xml') || names.has('build.gradle') || names.has('build.gradle.kts')) return 'java';
    if (names.has('Cargo.toml')) return 'rust';
    if (names.has('pyproject.toml') || names.has('requirements.txt')) return 'python';
    if (names.has('package.json')) return 'node';
    return 'go';
  }

  $effect(() => {
    if (!isOpen) {
      initializedForOpen = false;
      return;
    }
    const hasTarget = ide.hasTarget;
    untrack(() => {
      if (!initializedForOpen) {
        initializedForOpen = true;
        activeEco = defaultEcoFromWorkspace();
      }
      if (!hasTarget) return;
      void ide.loadToolchains({ includeCatalog: true });
      if (!ide.projectEnv && !ide.envLoading) {
        void ide.loadEnvironments();
      }
    });
  });

  function formatOsLabel(raw: string): string {
    const v = (raw || '').toLowerCase();
    if (v === 'macos' || v === 'darwin') return 'macOS (darwin)';
    if (v === 'linux') return 'Linux';
    if (v === 'windows' || v === 'win32') return 'Windows';
    return raw || 'macOS (darwin)';
  }

  function formatArchLabel(raw: string): string {
    const v = (raw || '').toLowerCase();
    if (v === 'aarch64' || v === 'arm64') return 'arm64 (aarch64)';
    if (v === 'x86_64' || v === 'amd64' || v === 'x64') return 'x86_64 (amd64)';
    return raw || 'arm64 (aarch64)';
  }

  const targetSummary = $derived(
    ide.target.kind === 'ssh'
      ? `SSH remote · ${ide.target.connection_name || 'Remote host'}`
      : 'Local workspace',
  );
  const platformBadgeText = $derived(`${formatOsLabel(ide.targetOs)} · ${formatArchLabel(ide.targetArch)}`);

  function countInstalled(id: string): number | undefined {
    const n = ide.toolchains.find((e) => e.ecosystem === id)?.installed.length ?? 0;
    return n > 0 ? n : undefined;
  }

  const ecoTabs = $derived.by((): readonly TabItem<string>[] => {
    const baseIds = new Set(BASE_ECOS.map((b) => b.id));
    const tabs: TabItem<string>[] = BASE_ECOS.map((b) => ({
      id: b.id,
      label: b.label,
      count: countInstalled(b.id),
    }));
    for (let i = 0; i < ide.toolchains.length; i++) {
      const t = ide.toolchains[i];
      if (!baseIds.has(t.ecosystem)) {
        tabs.push({ id: t.ecosystem, label: t.display_name || t.ecosystem, count: countInstalled(t.ecosystem) });
      }
    }
    return tabs;
  });

  const currentEco = $derived<IdeToolchainEcosystem | null>(
    ide.toolchains.find((e) => e.ecosystem === activeEco) ?? ide.toolchains[0] ?? null,
  );

  const selectedDownloadVersion = $derived.by(() => {
    const eco = currentEco;
    if (!eco || eco.downloadable.length === 0) return '';
    const saved = selectedVersions[eco.ecosystem];
    if (saved && eco.downloadable.some((d) => d.version === saved)) return saved;
    const preferred = eco.downloadable.find((d) => d.lts) ?? eco.downloadable[0];
    return preferred?.version ?? '';
  });

  const selectedDownloadItem = $derived(
    currentEco?.downloadable.find((d) => d.version === selectedDownloadVersion) ?? null,
  );

  function isSdkActiveInProject(ecosystem: string, sdk: IdeInstalledSdk): boolean {
    if (sdk.active) return true;
    const env = ide.projectEnv;
    if (!env) return false;
    if (ecosystem === 'java') return env.java_home === sdk.path;
    if (ecosystem === 'go') return env.go_root === sdk.path;
    if (ecosystem === 'rust') return env.rust_toolchain === sdk.version || env.rust_toolchain === sdk.path;
    if (ecosystem === 'node') return env.node_home === sdk.path;
    if (ecosystem === 'python') return env.python_interpreter === sdk.path;
    return false;
  }

  async function applySdkPathInProject(ecosystem: string, pathOrVersion: string): Promise<void> {
    const patch =
      ecosystem === 'java'
        ? { java_home: pathOrVersion }
        : ecosystem === 'go'
          ? { go_root: pathOrVersion }
          : ecosystem === 'rust'
            ? { rust_toolchain: pathOrVersion }
            : ecosystem === 'node'
              ? { node_home: pathOrVersion }
              : ecosystem === 'python'
                ? { python_interpreter: pathOrVersion }
                : null;
    if (patch) await ide.saveEnvironment(patch);
  }

  async function useSdkInProject(ecosystem: string, sdk: IdeInstalledSdk): Promise<void> {
    busyKey = `use:${sdk.id}`;
    try {
      await applySdkPathInProject(ecosystem, ecosystem === 'rust' ? sdk.version : sdk.path);
      await ide.loadToolchains({ includeCatalog: true });
    } finally {
      busyKey = null;
    }
  }

  async function handleAddLocalSdk(): Promise<void> {
    const eco = currentEco?.ecosystem ?? activeEco;
    if (ide.target.kind === 'ssh') {
      const remotePath = await confirmer.promptText(`Path to ${currentEco?.display_name ?? eco} SDK root on remote host`, {
        title: 'Add SDK from remote path',
        confirmLabel: 'Register SDK',
        placeholder: '/usr/local/go',
      });
      if (remotePath) await registerPickedSdk(eco, remotePath.trim());
      return;
    }
    localSdkPickerOpen = true;
  }

  async function registerPickedSdk(ecosystem: string, sdkPath: string): Promise<void> {
    if (!sdkPath) return;
    busyKey = `local:${ecosystem}`;
    try {
      const registered = await ide.addLocalToolchain(ecosystem, sdkPath);
      if (registered) {
        await applySdkPathInProject(ecosystem, ecosystem === 'rust' ? registered.version : registered.path);
        toasts.success(`Added ${ecosystem.toUpperCase()} SDK`, registered.version || registered.path);
      } else {
        await applySdkPathInProject(ecosystem, sdkPath);
      }
      await ide.loadToolchains({ includeCatalog: true });
    } finally {
      busyKey = null;
    }
  }

  async function triggerInstall(
    ecosystem: string,
    component: 'sdk' | 'lsp' | 'dap' | 'all',
    version?: string,
  ): Promise<void> {
    busyKey = `install:${ecosystem}:${component}`;
    try {
      const job = await ide.installToolchain(ecosystem, component, version || undefined);
      if (job) expandedLogs = { ...expandedLogs, [job.id]: true };
    } finally {
      busyKey = null;
    }
  }

  function toggleJobLog(jobId: string): void {
    expandedLogs = { ...expandedLogs, [jobId]: !expandedLogs[jobId] };
  }

  function jobStatusKey(job: IdeInstallJob): string {
    if (job.status === 'done') return 'succeeded';
    if (job.status === 'error') return 'failed';
    return 'running';
  }
</script>

{#if isOpen}
  <Modal title="SDKs, toolchains & debuggers" width={760} dismissable={!busyKey} onclose={close}>
    <div class="tc-shell">
      <div class="tc-arch-banner">
        <div class="tc-arch-main">
          <span class="tc-arch-icon" aria-hidden="true">
            <Icon name={ide.target.kind === 'ssh' ? 'server' : 'ide'} size={16} />
          </span>
          <div class="tc-arch-copy">
            <div class="tc-arch-headline">
              <span class="tc-arch-target">{targetSummary}</span>
              <Badge tone="accent" label={platformBadgeText} />
            </div>
            <p class="tc-arch-sub">
              Official SDK archives, language servers, and debug adapters matched to this machine’s OS and CPU architecture.
            </p>
          </div>
        </div>
        <button
          type="button"
          class="btn small ghost"
          disabled={ide.toolchainsLoading || !ide.hasTarget}
          onclick={() => ide.loadToolchains({ includeCatalog: true })}
        >
          <Icon name="refresh" size={12} />
          Refresh
        </button>
      </div>

      {#if !ide.hasTarget}
        <EmptyState
          icon="folder"
          title="Open a workspace first"
          body="Choose a local project directory or connect an SSH host to manage SDKs, LSP servers, and debuggers."
          actionLabel="Choose target…"
          actionKind="secondary"
          onaction={() => { close(); ide.sshOpen = true; }}
        />
      {:else}
        <LoadState
          what="toolchains"
          loading={ide.toolchainsLoading && ide.toolchains.length === 0}
          error={ide.toolchainsError}
          empty={ide.toolchains.length === 0}
          onretry={() => ide.loadToolchains({ includeCatalog: true })}
        >
          {#snippet emptyView()}
            <EmptyState
              icon="box"
              title="No toolchain catalog loaded yet"
              body="Probe this target to inspect installed SDKs and architecture-matched downloads."
              actionLabel="Probe target"
              actionKind="secondary"
              onaction={() => ide.loadToolchains({ includeCatalog: true })}
            />
          {/snippet}

          <Tabs label="Toolchain ecosystems" tabs={ecoTabs} value={activeEco} onchange={(id) => (activeEco = id)} idBase="ide-tc" />

          {#if currentEco}
            <div class="tc-panel" role="tabpanel" id="ide-tc-panel-{activeEco}" aria-labelledby="ide-tc-tab-{activeEco}">
              <section class="tc-section" aria-labelledby="tc-sdk-heading">
                <div class="tc-sec-head">
                  <div class="tc-sec-title-group">
                    <h3 id="tc-sdk-heading" class="tc-sec-title">Installed {currentEco.display_name} SDKs</h3>
                    <span class="tc-sec-meta">{currentEco.installed.length} detected</span>
                  </div>
                  <button
                    type="button"
                    class="btn small"
                    disabled={busyKey === `local:${currentEco.ecosystem}`}
                    onclick={() => void handleAddLocalSdk()}
                    data-testid="ide-add-local-sdk-btn"
                  >
                    <Icon name="folder" size={12} />
                    Add local SDK…
                  </button>
                </div>

                {#if currentEco.installed.length === 0}
                  <div class="tc-empty-note">
                    No {currentEco.display_name} SDK detected yet. Download an official release below or add an existing SDK from disk.
                  </div>
                {:else}
                  <div class="tc-sdk-list">
                    {#each currentEco.installed as sdk (sdk.id)}
                      {@const active = isSdkActiveInProject(currentEco.ecosystem, sdk)}
                      <div class="tc-sdk-item">
                        <div class="tc-sdk-info">
                          <div class="tc-sdk-top">
                            <span class="tc-sdk-ver">{sdk.version}</span>
                            <Badge
                              tone={sdk.source === 'otto' ? 'accent' : 'neutral'}
                              label={sdk.source === 'otto' ? 'Otto managed' : sdk.source === 'local' || sdk.source === 'custom' ? 'Local disk' : 'System'}
                            />
                            {#if active}<Badge tone="ok" dot label="Active" />{/if}
                          </div>
                          <div class="tc-sdk-path-row">
                            <code class="tc-mono-path" dir="ltr">{sdk.path}</code>
                            <button
                              type="button"
                              class="icon-btn"
                              aria-label="Copy SDK path"
                              title="Copy SDK path"
                              onclick={() => copyWithToast(sdk.path, 'SDK path')}
                            >
                              <Icon name="copy" size={12} />
                            </button>
                          </div>
                        </div>
                        <button
                          type="button"
                          class="btn small"
                          disabled={active || busyKey === `use:${sdk.id}`}
                          onclick={() => useSdkInProject(currentEco.ecosystem, sdk)}
                        >
                          {busyKey === `use:${sdk.id}` ? 'Applying…' : active ? 'In use' : 'Use in project'}
                        </button>
                      </div>
                    {/each}
                  </div>
                {/if}
              </section>

              <div class="tc-tools-grid">
                <section class="tc-tool-box" aria-labelledby="tc-lsp-heading">
                  <div class="tc-tool-head">
                    <div>
                      <h3 id="tc-lsp-heading" class="tc-sec-title">Language server (LSP)</h3>
                      <span class="tc-tool-name" dir="ltr">{currentEco.lsp_status.name}</span>
                    </div>
                    <StatusBadge tone={currentEco.lsp_status.available && !currentEco.lsp_status.outdated ? 'success' : 'warning'} label={currentEco.lsp_status.outdated ? 'Outdated' : currentEco.lsp_status.available ? 'Installed' : 'Missing'} />
                  </div>
                  {#if currentEco.lsp_status.outdated}
                    <p class="tc-tool-hint tc-outdated" data-testid="ide-tc-lsp-outdated">{currentEco.lsp_status.name} {currentEco.lsp_status.outdated}: update it to match.</p>
                  {/if}
                  {#if currentEco.lsp_status.path}
                    <code class="tc-mono-path" dir="ltr">{currentEco.lsp_status.path}</code>
                  {:else}
                    <p class="tc-tool-hint">Powers completions, hover types, diagnostics, rename, and find usages.</p>
                  {/if}
                  <div class="tc-tool-actions">
                    <button
                      type="button"
                      class="btn small"
                      class:ghost={currentEco.lsp_status.available && !currentEco.lsp_status.outdated}
                      disabled={busyKey === `install:${currentEco.ecosystem}:lsp`}
                      onclick={() => triggerInstall(currentEco.ecosystem, 'lsp')}
                    >
                      <Icon name="download" size={12} />
                      {busyKey === `install:${currentEco.ecosystem}:lsp` ? 'Installing…' : currentEco.lsp_status.outdated ? 'Update LSP' : currentEco.lsp_status.available ? 'Reinstall LSP' : 'Install LSP'}
                    </button>
                  </div>
                </section>

                <section class="tc-tool-box" aria-labelledby="tc-dap-heading">
                  <div class="tc-tool-head">
                    <div>
                      <h3 id="tc-dap-heading" class="tc-sec-title">Debug adapter (DAP)</h3>
                      <span class="tc-tool-name" dir="ltr">{currentEco.dap_status.name}</span>
                    </div>
                    <StatusBadge tone={currentEco.dap_status.available && !currentEco.dap_status.outdated ? 'success' : 'warning'} label={currentEco.dap_status.outdated ? 'Outdated' : currentEco.dap_status.available ? 'Installed' : 'Missing'} />
                  </div>
                  {#if currentEco.dap_status.outdated}
                    <p class="tc-tool-hint tc-outdated" data-testid="ide-tc-dap-outdated">{currentEco.dap_status.name} {currentEco.dap_status.outdated}: update it to match.</p>
                  {/if}
                  {#if currentEco.dap_status.path}
                    <code class="tc-mono-path" dir="ltr">{currentEco.dap_status.path}</code>
                  {:else}
                    <p class="tc-tool-hint">Powers breakpoints, stepping, call stacks, variable inspection, and live REPL.</p>
                  {/if}
                  <div class="tc-tool-actions">
                    <button
                      type="button"
                      class="btn small"
                      class:ghost={currentEco.dap_status.available && !currentEco.dap_status.outdated}
                      disabled={busyKey === `install:${currentEco.ecosystem}:dap`}
                      onclick={() => triggerInstall(currentEco.ecosystem, 'dap')}
                    >
                      <Icon name="bug" size={12} />
                      {busyKey === `install:${currentEco.ecosystem}:dap` ? 'Installing…' : currentEco.dap_status.outdated ? 'Update debugger' : currentEco.dap_status.available ? 'Reinstall debugger' : 'Install debugger'}
                    </button>
                  </div>
                </section>
              </div>

              <section class="tc-section" aria-labelledby="tc-dl-heading">
                <div class="tc-sec-head">
                  <h3 id="tc-dl-heading" class="tc-sec-title">Download official SDK version</h3>
                  <Badge tone="info" variant="outline" label={`${ide.targetOs || 'macos'}/${ide.targetArch || 'arm64'}`} />
                </div>

                {#if currentEco.downloadable.length === 0}
                  <div class="tc-empty-note">
                    {ide.toolchainsLoading ? 'Loading official release catalog…' : `No prebuilt SDK archives listed for ${ide.targetOs}/${ide.targetArch}.`}
                  </div>
                {:else}
                  <div class="tc-dl-bar">
                    <div class="tc-dl-picker">
                      <label class="tc-sr-label" for="tc-dl-select">Select {currentEco.display_name} version</label>
                      <select
                        id="tc-dl-select"
                        class="input"
                        value={selectedDownloadVersion}
                        onchange={(e) => { selectedVersions = { ...selectedVersions, [currentEco.ecosystem]: e.currentTarget.value }; }}
                        data-testid="ide-sdk-version-select"
                      >
                        {#each currentEco.downloadable as dl (dl.version)}
                          <option value={dl.version}>
                            {dl.label}{dl.lts ? ' (LTS)' : ''}{dl.size_hint ? ` · ${dl.size_hint}` : ''} — {dl.os}/{dl.arch}
                          </option>
                        {/each}
                      </select>
                    </div>
                    <div class="tc-dl-btns">
                      <button
                        type="button"
                        class="btn"
                        disabled={!selectedDownloadVersion || busyKey === `install:${currentEco.ecosystem}:all`}
                        onclick={() => triggerInstall(currentEco.ecosystem, 'all', selectedDownloadVersion)}
                      >
                        {busyKey === `install:${currentEco.ecosystem}:all` ? 'Installing…' : 'Full setup (SDK + LSP + DAP)'}
                      </button>
                      <button
                        type="button"
                        class="btn primary"
                        disabled={!selectedDownloadVersion || busyKey === `install:${currentEco.ecosystem}:sdk`}
                        onclick={() => triggerInstall(currentEco.ecosystem, 'sdk', selectedDownloadVersion)}
                        data-testid="ide-sdk-download-btn"
                      >
                        <Icon name="download" size={13} />
                        {busyKey === `install:${currentEco.ecosystem}:sdk` ? 'Downloading…' : 'Download & install'}
                      </button>
                    </div>
                  </div>

                  {#if selectedDownloadItem}
                    <div class="tc-dl-meta">
                      <span>{selectedDownloadItem.label}</span>
                      {#if selectedDownloadItem.lts}<Badge tone="accent" label="LTS" />{/if}
                      <Badge tone="neutral" variant="outline" label={`${selectedDownloadItem.os} · ${selectedDownloadItem.arch}`} />
                      {#if selectedDownloadItem.sha256}<Badge tone="ok" variant="outline" label="SHA-256 verified" />{/if}
                      {#if selectedDownloadItem.size_hint}<span class="tc-dim">{selectedDownloadItem.size_hint}</span>{/if}
                      <code class="tc-mono-url" dir="ltr">{selectedDownloadItem.url}</code>
                    </div>
                  {/if}
                {/if}
              </section>
            </div>
          {/if}
        </LoadState>
      {/if}

      {#if ide.installJobs.length > 0}
        <section class="tc-jobs" aria-labelledby="tc-jobs-heading">
          <h3 id="tc-jobs-heading" class="tc-sec-title">Installation jobs</h3>
          <div class="tc-job-list">
            {#each ide.installJobs as job (job.id)}
              <div class="tc-job-item">
                <div class="tc-job-head">
                  <div class="tc-job-title">
                    <span class="tc-sdk-ver">
                      {job.ecosystem.toUpperCase()} · {job.component.toUpperCase()}
                      {#if job.version}({job.version}){/if}
                    </span>
                    <Badge tone="neutral" variant="outline" label={`${job.target_os}/${job.target_arch}`} />
                    <StatusBadge status={runStatus(jobStatusKey(job))} />
                  </div>
                  <div class="tc-job-right">
                    <span class="tc-job-pct">{job.progress_pct}%</span>
                    {#if job.log_lines.length > 0}
                      <button type="button" class="btn small ghost" onclick={() => toggleJobLog(job.id)}>
                        {expandedLogs[job.id] ? 'Hide log' : `Install log (${job.log_lines.length})`}
                      </button>
                    {/if}
                  </div>
                </div>

                <div
                  class="tc-prog-track"
                  role="progressbar"
                  aria-label="Installation progress for {job.ecosystem} {job.component}"
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={job.progress_pct}
                >
                  <div
                    class="tc-prog-fill"
                    class:is-error={job.status === 'error'}
                    class:is-done={job.status === 'done'}
                    style:inline-size="{Math.min(100, Math.max(0, job.progress_pct))}%"
                  ></div>
                </div>

                {#if job.installed_path}
                  <div class="tc-job-path">
                    <span class="tc-dim">Installed to:</span>
                    <code class="tc-mono-path" dir="ltr">{job.installed_path}</code>
                  </div>
                {/if}
                {#if job.error}
                  <div class="tc-job-err" use:announceShown={{ text: job.error, assertive: true }}>{job.error}</div>
                {/if}
                {#if expandedLogs[job.id] && job.log_lines.length > 0}
                  <pre class="tc-job-log" dir="ltr">{job.log_lines.join('\n')}</pre>
                {/if}
              </div>
            {/each}
          </div>
        </section>
      {/if}
    </div>

    {#snippet footer()}
      <button type="button" class="btn ghost" onclick={close}>Done</button>
    {/snippet}
  </Modal>

  {#if localSdkPickerOpen}
    <FolderPicker
      title="Select local {currentEco?.display_name ?? activeEco} SDK directory"
      start="/opt/homebrew"
      onpick={(picked) => {
        localSdkPickerOpen = false;
        void registerPickedSdk(currentEco?.ecosystem ?? activeEco, picked);
      }}
      onclose={() => (localSdkPickerOpen = false)}
    />
  {/if}
{/if}

<style>
  .tc-shell { display: flex; flex-direction: column; gap: 14px; min-inline-size: 0; }
  .tc-arch-banner {
    display: flex; align-items: center; justify-content: space-between; gap: 12px;
    padding: 10px 12px; border-radius: var(--radius-m); background: var(--surface-2); border: 1px solid var(--border);
  }
  .tc-arch-main { display: flex; align-items: flex-start; gap: 10px; min-inline-size: 0; }
  .tc-arch-icon {
    display: inline-flex; align-items: center; justify-content: center;
    inline-size: 32px; block-size: 32px; border-radius: var(--radius-s);
    background: var(--accent-soft); color: var(--accent-text); flex-shrink: 0;
  }
  .tc-arch-copy { display: flex; flex-direction: column; gap: 2px; min-inline-size: 0; }
  .tc-arch-headline { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .tc-arch-target { font-size: var(--fs-m); font-weight: 600; color: var(--text); }
  .tc-arch-sub { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); }
  .tc-panel { display: flex; flex-direction: column; gap: 12px; padding-block-start: 4px; min-inline-size: 0; }

  .tc-section, .tc-tool-box {
    display: flex; flex-direction: column; gap: 8px; padding: 12px;
    border-radius: var(--radius-m); background: var(--surface-2); border: 1px solid var(--border); min-inline-size: 0;
  }
  .tc-sec-head, .tc-tool-head { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px; }
  .tc-sec-title-group { display: flex; align-items: center; gap: 8px; }
  .tc-sec-title { margin: 0; font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .tc-sec-meta, .tc-dim { font-size: var(--fs-xs); color: var(--text-dim); }
  .tc-empty-note { font-size: var(--fs-s); color: var(--text-dim); padding-block: 4px; }
  .tc-sdk-list { display: flex; flex-direction: column; gap: 6px; }
  .tc-sdk-item {
    display: flex; align-items: center; justify-content: space-between; gap: 10px;
    padding: 8px 10px; border-radius: var(--radius-s); background: var(--surface); border: 1px solid var(--border); min-inline-size: 0;
  }
  .tc-sdk-info { display: flex; flex-direction: column; gap: 2px; min-inline-size: 0; flex: 1; }
  .tc-sdk-top { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .tc-sdk-ver { font-size: var(--fs-m); font-weight: 600; color: var(--text); }
  .tc-sdk-path-row { display: flex; align-items: center; gap: 4px; min-inline-size: 0; }
  .tc-mono-path {
    font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-inline-size: 0;
  }

  .tc-tools-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
  .tc-tool-name { display: block; font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); margin-block-start: 2px; }
  .tc-tool-hint { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); }
  .tc-tool-hint.tc-outdated { color: var(--warning); }
  .tc-tool-actions { display: flex; align-items: center; gap: 6px; margin-block-start: auto; padding-block-start: 4px; }

  .tc-dl-bar { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .tc-dl-picker { flex: 1; min-inline-size: 220px; }
  .tc-dl-picker select { inline-size: 100%; }
  .tc-dl-btns { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .tc-dl-meta { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; font-size: var(--fs-xs); color: var(--text-dim); min-inline-size: 0; }
  .tc-mono-url {
    font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-inline-size: 100%;
  }

  .tc-jobs { display: flex; flex-direction: column; gap: 8px; padding-block-start: 4px; border-block-start: 1px solid var(--border); }
  .tc-job-list { display: flex; flex-direction: column; gap: 8px; }
  .tc-job-item {
    display: flex; flex-direction: column; gap: 6px; padding: 10px 12px;
    border-radius: var(--radius-m); background: var(--surface-2); border: 1px solid var(--border);
  }
  .tc-job-head { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px; }
  .tc-job-title, .tc-job-right { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .tc-job-pct { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); }

  .tc-prog-track { inline-size: 100%; block-size: 6px; border-radius: 999px; background: var(--surface-3); overflow: hidden; }
  .tc-prog-fill { block-size: 100%; border-radius: 999px; background: var(--accent-solid); }
  .tc-prog-fill.is-done { background: var(--success); }
  .tc-prog-fill.is-error { background: var(--danger-solid); }
  .tc-job-path { display: flex; align-items: center; gap: 6px; min-inline-size: 0; }
  .tc-job-err { font-size: var(--fs-xs); color: var(--danger); }
  .tc-job-log {
    margin: 0; padding: 8px 10px; max-block-size: 160px; overflow: auto;
    border-radius: var(--radius-s); background: var(--term-bg); border: 1px solid var(--border);
    font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text);
    white-space: pre-wrap; word-break: break-word;
  }
  .tc-sr-label { position: absolute; inline-size: 1px; block-size: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }

  @media (max-width: 640px) {
    .tc-arch-banner, .tc-sdk-item { flex-direction: column; align-items: stretch; }
    .tc-tools-grid { grid-template-columns: 1fr; }
  }
</style>
