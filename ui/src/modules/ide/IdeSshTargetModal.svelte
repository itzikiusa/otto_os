<script lang="ts">
  // Workspace Target & Remote SSH Development modal for the Otto Full IDE.
  // Switches seamlessly between Local Workspaces and saved SSH hosts from Connections,
  // with live remote directory browsing via `/ide/fs/tree?kind=ssh&connection_id=...`.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import EnvBadge from '../../lib/components/EnvBadge.svelte';
  import PathField from '../../lib/components/PathField.svelte';
  import { api } from '../../lib/api/client';
  import type { Connection } from '../../lib/api/types';
  import { announceShown } from '../../lib/announce.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ws, SCRATCH_WORKSPACE_ID } from '../../lib/stores/workspace.svelte';
  import { ide, type IdeFsEntry, type IdeFsTreeResp, type IdeNetworkBridgeConfig } from './ideStore.svelte';
  import IdeNetworkBridgeCard from './shell/IdeNetworkBridgeCard.svelte';
  import { confirmProdNetworkBridge } from './shell/networkBridgeGuard.svelte';
  import {
    emptyNetworkBridgeDraft,
    isProdEnvironment,
    serializeNetworkBridgeDraft,
    type IdeNetworkBridgeDraft,
  } from './shell/networkBridgeModel';

  interface Props {
    open?: boolean;
    onclose?: () => void;
  }

  let { open, onclose = () => (ide.sshOpen = false) }: Props = $props();

  const isOpen = $derived(open === undefined ? ide.sshOpen : open);

  let mode = $state<'local' | 'ssh'>('local');
  let localRoot = $state('');
  let localWsId = $state<string | null>(null);

  let sshConns = $state<Connection[]>([]);
  let connsLoading = $state(false);
  let connsError = $state<string | null>(null);
  let selectedConnId = $state<string | null>(null);
  let remotePath = $state('');

  let remoteDirs = $state<IdeFsEntry[]>([]);
  let remoteParent = $state<string | null>(null);
  let remoteOs = $state<string>('');
  let remoteArch = $state<string>('');
  let remoteBrowsed = $state(false);
  let remoteLoading = $state(false);
  let remoteError = $state<string | null>(null);
  let switching = $state(false);
  let validationError = $state('');
  let bridgeDraft = $state<IdeNetworkBridgeDraft>(emptyNetworkBridgeDraft(null));
  let bridgeDirty = $state(false);

  function close(): void {
    ide.sshOpen = false;
    onclose();
  }

  $effect(() => {
    if (!isOpen) return;
    validationError = '';
    bridgeDirty = false;
    mode = ide.target.kind;
    localRoot =
      ide.target.kind === 'local'
        ? ide.target.root
        : (ws.workspaces.find((w) => w.id === ws.currentId)?.root_path ?? '');
    localWsId = ide.target.workspace_id ?? ws.currentId ?? null;
    if (ide.target.kind === 'ssh' && ide.target.connection_id) {
      selectedConnId = ide.target.connection_id;
      remotePath = ide.target.root || '';
    }
    untrack(() => {
      void loadSshConnections();
      if (ide.target.kind === 'local' && ide.target.root && !ide.projectEnv && !ide.envLoading) {
        void ide.loadEnvironments();
      }
    });
  });

  $effect(() => {
    if (!isOpen || bridgeDirty) return;
    bridgeDraft = emptyNetworkBridgeDraft(ide.projectEnv?.network_bridge ?? null);
  });

  const selectedConn = $derived<Connection | null>(
    sshConns.find((c) => c.id === selectedConnId) ?? sshConns[0] ?? null,
  );

  function connEndpointText(conn: Connection): string {
    const p = conn.params ?? {};
    const host = String(p.host ?? p.hostname ?? 'localhost');
    const user = String(p.user ?? p.username ?? '').trim();
    const port = Number(p.port ?? 22);
    const addr = user ? `${user}@${host}` : host;
    return port && port !== 22 ? `${addr}:${port}` : addr;
  }

  function defaultRemoteRoot(conn: Connection): string {
    const p = conn.params ?? {};
    const user = String(p.user ?? p.username ?? '').trim();
    if (!user || user === 'root') return '/root';
    return `/home/${user}`;
  }

  async function loadSshConnections(): Promise<void> {
    connsLoading = true;
    connsError = null;
    try {
      const scopes = Array.from(
        new Set(
          [
            ide.target.workspace_id,
            ws.currentId,
            ...ws.workspaces.map((w) => w.id),
            ws.scratch?.id ?? SCRATCH_WORKSPACE_ID,
          ].filter((id): id is string => Boolean(id)),
        ),
      );
      if (scopes.length === 0) scopes.push(SCRATCH_WORKSPACE_ID);

      const results = await Promise.allSettled(
        scopes.map((id) =>
          api.get<Connection[]>(`/workspaces/${encodeURIComponent(id)}/connections`),
        ),
      );
      const seen = new Map<string, Connection>();
      let firstErr: unknown = null;
      let anyOk = false;
      for (let i = 0; i < results.length; i++) {
        const res = results[i];
        if (res.status === 'fulfilled') {
          anyOk = true;
          for (let j = 0; j < res.value.length; j++) {
            const c = res.value[j];
            if (c.kind === 'ssh' && !seen.has(c.id)) {
              seen.set(c.id, c);
            }
          }
        } else if (!firstErr) {
          firstErr = res.reason;
        }
      }
      if (!anyOk && firstErr) {
        connsError = loadErrorText(firstErr);
      }
      sshConns = Array.from(seen.values());
      if (!selectedConnId && sshConns.length > 0) {
        selectedConnId = sshConns[0].id;
        if (!remotePath) remotePath = defaultRemoteRoot(sshConns[0]);
      }
    } catch (e) {
      connsError = loadErrorText(e);
    } finally {
      connsLoading = false;
    }
  }

  function pickLocalMode(): void {
    validationError = '';
    mode = 'local';
  }

  function pickSshHost(conn: Connection): void {
    validationError = '';
    mode = 'ssh';
    if (selectedConnId !== conn.id) {
      selectedConnId = conn.id;
      remotePath =
        ide.target.kind === 'ssh' && ide.target.connection_id === conn.id
          ? ide.target.root
          : defaultRemoteRoot(conn);
      remoteDirs = [];
      remoteParent = null;
      remoteOs = '';
      remoteArch = '';
      remoteBrowsed = false;
      remoteError = null;
    }
  }

  async function browseRemoteDirectory(targetDir?: string): Promise<void> {
    const conn = selectedConn;
    if (!conn) return;
    const dir = (targetDir ?? remotePath).trim() || defaultRemoteRoot(conn);
    remoteLoading = true;
    remoteError = null;
    remoteBrowsed = true;
    try {
      const qs = new URLSearchParams({
        kind: 'ssh',
        connection_id: conn.id,
        root: dir,
        path: dir,
      });
      const resp = await api.long.get<IdeFsTreeResp>(`/ide/fs/tree?${qs.toString()}`);
      remotePath = resp.path || dir;
      remoteParent = resp.parent ?? null;
      remoteOs = resp.target_os ?? '';
      remoteArch = resp.target_arch ?? '';
      remoteDirs = (resp.entries ?? []).filter((e) => e.kind === 'dir');
    } catch (e) {
      remoteError = loadErrorText(e);
    } finally {
      remoteLoading = false;
    }
  }

  function openConnectionsPage(): void {
    close();
    router.go('connections');
  }

  async function applyTargetSwitch(): Promise<void> {
    validationError = '';
    if (mode === 'local') {
      const root = localRoot.trim();
      if (!root) {
        validationError = 'Select or enter a local folder path.';
        return;
      }
      let pendingBridge: IdeNetworkBridgeConfig | null = null;
      if (bridgeDirty) {
        const ser = serializeNetworkBridgeDraft(bridgeDraft);
        if (ser.error) {
          validationError = ser.error;
          return;
        }
        if (ser.config.enabled) {
          const allowed = await confirmProdNetworkBridge(ser.config, 'save', localWsId);
          if (!allowed) return;
        }
        pendingBridge = ser.config;
      }
      switching = true;
      try {
        await ide.setLocalTarget(root, localWsId);
        if (pendingBridge) {
          await ide.saveEnvironment({
            network_bridge: pendingBridge,
            ...(pendingBridge.enabled ? { confirm_prod_targets: true } : {}),
          });
          bridgeDirty = false;
        }
        toasts.success('Switched to local workspace', root);
        close();
      } finally {
        switching = false;
      }
      return;
    }

    const conn = selectedConn;
    if (!conn) {
      validationError = 'Select an SSH connection or add one in Connections.';
      return;
    }
    const root = remotePath.trim();
    if (!root || root.startsWith('-')) {
      validationError = 'Enter a valid remote directory path on the SSH host.';
      return;
    }

    if (isProdEnvironment(conn.environment)) {
      const ok = await confirmer.ask(
        `Connect IDE to production SSH host "${conn.name}" (${connEndpointText(conn)}) at ${root}? File edits, commands, and debuggers will run directly on that host.`,
        {
          title: 'Connect to production SSH host?',
          confirmLabel: 'Connect to production',
          danger: true,
        },
      );
      if (!ok) return;
    }

    switching = true;
    try {
      await ide.setSshTarget(conn.id, root, conn.name);
      toasts.success(`Connected to ${conn.name}`, root);
      close();
    } finally {
      switching = false;
    }
  }
</script>

{#if isOpen}
  <Modal
    title="Workspace target & remote SSH"
    width={820}
    dismissable={!switching}
    onclose={close}
  >
    <div class="ssh-shell">
      <div class="ssh-current-bar">
        <span class="ssh-cur-lbl">Active target:</span>
        {#if ide.target.kind === 'ssh'}
          <Badge tone="accent" dot label={`SSH · ${ide.target.connection_name || 'Remote'}`} />
        {:else}
          <Badge tone="ok" dot label="Local workspace" />
        {/if}
        <code class="ssh-mono-path" dir="ltr">{ide.target.root || '(none selected)'}</code>
      </div>

      <div class="ssh-grid">
        <aside class="ssh-rail" aria-label="Target machines">
          <button
            type="button"
            class="ssh-host-btn"
            class:is-selected={mode === 'local'}
            aria-pressed={mode === 'local'}
            onclick={pickLocalMode}
          >
            <span class="ssh-host-icon" aria-hidden="true">
              <Icon name="ide" size={16} />
            </span>
            <span class="ssh-host-copy">
              <span class="ssh-host-name">Local workspace</span>
              <span class="ssh-host-sub">This Mac ({ide.targetOs || 'macos'} · {ide.targetArch || 'arm64'})</span>
            </span>
          </button>

          <div class="ssh-rail-sep">
            <span class="ssh-sec-title">Saved SSH hosts</span>
            <button type="button" class="btn small ghost" onclick={openConnectionsPage}>
              <Icon name="plug" size={12} />
              Connections
            </button>
          </div>

          <LoadState
            what="SSH connections"
            variant="compact"
            loading={connsLoading}
            error={connsError}
            empty={sshConns.length === 0}
            onretry={loadSshConnections}
          >
            {#snippet emptyView()}
              <EmptyState
                icon="server"
                title="No SSH hosts yet"
                body="Add an SSH profile in Connections to edit, build, test, and debug remote projects."
                actionLabel="Add SSH host…"
                actionKind="secondary"
                onaction={openConnectionsPage}
              />
            {/snippet}

            <div class="ssh-conn-list">
              {#each sshConns as conn (conn.id)}
                <button
                  type="button"
                  class="ssh-host-btn"
                  class:is-selected={mode === 'ssh' && selectedConn?.id === conn.id}
                  aria-pressed={mode === 'ssh' && selectedConn?.id === conn.id}
                  onclick={() => pickSshHost(conn)}
                >
                  <span class="ssh-host-icon" aria-hidden="true">
                    <Icon name="server" size={16} />
                  </span>
                  <span class="ssh-host-copy">
                    <span class="ssh-host-top">
                      <span class="ssh-host-name">{conn.name}</span>
                      <EnvBadge env={conn.environment} readOnly={conn.read_only} />
                    </span>
                    <span class="ssh-host-sub ssh-mono" dir="ltr">{connEndpointText(conn)}</span>
                  </span>
                </button>
              {/each}
            </div>
          </LoadState>
        </aside>

        <div class="ssh-detail">
          {#if mode === 'local'}
            <section class="ssh-pane" aria-labelledby="ssh-local-heading">
              <h3 id="ssh-local-heading" class="ssh-sec-title">Local repository folder</h3>
              <p class="ssh-hint">
                Open any folder on this machine with native file watching, local SDKs, LSP servers, and DAP debuggers.
              </p>

              <div class="field">
                <label for="ssh-local-input">Folder path</label>
                <PathField bind:value={localRoot} start={localRoot || ide.target.root}>
                  <input
                    id="ssh-local-input"
                    class="input"
                    type="text"
                    dir="ltr"
                    placeholder="/Users/you/project"
                    bind:value={localRoot}
                  />
                </PathField>
              </div>

              {#if ws.workspaces.length > 0}
                <div class="ssh-ws-box">
                  <span class="ssh-sec-title">Registered workspaces</span>
                  <div class="ssh-ws-list">
                    {#each ws.workspaces as w (w.id)}
                      {#if w.root_path}
                        <button
                          type="button"
                          class="ssh-dir-row"
                          class:is-picked={localRoot === w.root_path}
                          aria-pressed={localRoot === w.root_path}
                          onclick={() => {
                            localRoot = w.root_path ?? '';
                            localWsId = w.id;
                          }}
                        >
                          <Icon name="folder" size={14} />
                          <span class="ssh-ws-name">{w.name}</span>
                          <code class="ssh-mono-path" dir="ltr">{w.root_path}</code>
                        </button>
                      {/if}
                    {/each}
                  </div>
                </div>
              {/if}
            </section>

            <IdeNetworkBridgeCard
              bind:draft={bridgeDraft}
              sshConnections={sshConns}
              idPrefix="ide-ssh-nb"
              onDirty={() => (bridgeDirty = true)}
            />
          {:else if selectedConn}
            <section class="ssh-pane" aria-labelledby="ssh-remote-heading">
              <div class="ssh-remote-head">
                <div>
                  <div class="ssh-host-top">
                    <h3 id="ssh-remote-heading" class="ssh-sec-title">{selectedConn.name}</h3>
                    <EnvBadge env={selectedConn.environment} readOnly={selectedConn.read_only} />
                    {#if remoteOs || remoteArch}
                      <Badge tone="accent" label={`${remoteOs} · ${remoteArch}`} />
                    {/if}
                  </div>
                  <code class="ssh-mono-path" dir="ltr">{connEndpointText(selectedConn)}</code>
                </div>
                <button
                  type="button"
                  class="btn small"
                  disabled={remoteLoading}
                  onclick={() => browseRemoteDirectory()}
                >
                  <Icon name="folder" size={12} />
                  {remoteLoading ? 'Browsing…' : 'Browse remote'}
                </button>
              </div>

              <div class="field">
                <label for="ssh-remote-path">Remote repository path</label>
                <div class="ssh-path-bar">
                  <input
                    id="ssh-remote-path"
                    class="input"
                    type="text"
                    dir="ltr"
                    placeholder="/home/deploy/app"
                    bind:value={remotePath}
                    onkeydown={(e) => {
                      if (e.key === 'Enter') {
                        e.preventDefault();
                        void browseRemoteDirectory();
                      }
                    }}
                  />
                  {#if remoteParent}
                    <button
                      type="button"
                      class="btn small ghost"
                      title="Browse parent directory"
                      onclick={() => browseRemoteDirectory(remoteParent ?? '/')}
                    >
                      Up (..)
                    </button>
                  {/if}
                </div>
              </div>

              {#if remoteBrowsed}
                <LoadState
                  what="remote directories"
                  loading={remoteLoading}
                  error={remoteError}
                  empty={remoteDirs.length === 0}
                  onretry={() => browseRemoteDirectory()}
                >
                  {#snippet emptyView()}
                    <div class="ssh-hint">
                      No subdirectories inside <code dir="ltr">{remotePath}</code> — click Connect SSH workspace to open this folder.
                    </div>
                  {/snippet}

                  <div class="ssh-dir-browser" role="region" aria-label="Remote directories">
                    {#each remoteDirs as entry (entry.path)}
                      <button
                        type="button"
                        class="ssh-dir-row"
                        onclick={() => browseRemoteDirectory(entry.path)}
                      >
                        <Icon name="folder" size={14} />
                        <span class="ssh-mono" dir="ltr">{entry.name}</span>
                        <span class="ssh-dir-open">Open <Icon name="chevronRight" size={12} /></span>
                      </button>
                    {/each}
                  </div>
                </LoadState>
              {:else}
                <div class="ssh-hint-card">
                  <Icon name="info" size={14} />
                  <span>
                    Enter the remote project path directly, or click <strong>Browse remote</strong> to explore folders on <code dir="ltr">{selectedConn.name}</code> over SSH.
                  </span>
                </div>
              {/if}
            </section>
          {:else}
            <EmptyState
              icon="server"
              title="Select or add an SSH host"
              body="Configure an SSH connection in Connections to work on a remote Linux or macOS server."
              actionLabel="Open Connections"
              actionKind="secondary"
              onaction={openConnectionsPage}
            />
          {/if}

          {#if validationError}
            <div class="ssh-err" use:announceShown={{ text: validationError, assertive: true }}>
              {validationError}
            </div>
          {/if}
        </div>
      </div>
    </div>

    {#snippet footer()}
      <button type="button" class="btn ghost" disabled={switching} onclick={close}>
        Cancel
      </button>
      <button
        type="button"
        class="btn primary"
        disabled={switching || (mode === 'ssh' && !selectedConn)}
        onclick={applyTargetSwitch}
      >
        {switching
          ? 'Connecting…'
          : mode === 'ssh'
            ? 'Connect SSH workspace'
            : 'Open local workspace'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .ssh-shell { display: flex; flex-direction: column; gap: 12px; min-inline-size: 0; }
  .ssh-current-bar {
    display: flex; flex-wrap: wrap; align-items: center; gap: 8px;
    padding: 8px 12px; border-radius: var(--radius-m); background: var(--surface-2); border: 1px solid var(--border);
  }
  .ssh-cur-lbl { font-size: var(--fs-xs); font-weight: 500; color: var(--text-dim); }
  .ssh-grid { display: grid; grid-template-columns: 260px minmax(0, 1fr); gap: 14px; min-inline-size: 0; }
  .ssh-rail {
    display: flex; flex-direction: column; gap: 8px; padding: 10px;
    border-radius: var(--radius-m); background: var(--surface-2); border: 1px solid var(--border); min-inline-size: 0;
  }
  .ssh-rail-sep {
    display: flex; align-items: center; justify-content: space-between; gap: 6px;
    padding-block-start: 6px; border-block-start: 1px solid var(--border);
  }
  .ssh-conn-list { display: flex; flex-direction: column; gap: 4px; max-block-size: 260px; overflow-y: auto; }
  .ssh-host-btn {
    display: flex; align-items: flex-start; gap: 8px; padding: 8px; border-radius: var(--radius-s);
    border: 1px solid transparent; background: transparent; color: var(--text); text-align: start;
    cursor: pointer; transition: background var(--dur-fast) var(--ease-out);
  }
  .ssh-host-btn:hover { background: var(--hover); }
  .ssh-host-btn.is-selected { background: var(--accent-soft); border-color: var(--accent-line); }
  .ssh-host-icon {
    display: inline-flex; align-items: center; justify-content: center;
    inline-size: 28px; block-size: 28px; border-radius: var(--radius-s);
    background: var(--surface); color: var(--accent-text); flex-shrink: 0;
  }
  .ssh-host-copy { display: flex; flex-direction: column; gap: 2px; min-inline-size: 0; flex: 1; }
  .ssh-host-top { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .ssh-host-name { font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .ssh-host-sub { font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ssh-detail { display: flex; flex-direction: column; gap: 10px; min-inline-size: 0; }
  .ssh-pane {
    display: flex; flex-direction: column; gap: 10px; padding: 12px;
    border-radius: var(--radius-m); background: var(--surface-2); border: 1px solid var(--border); min-inline-size: 0;
  }
  .ssh-remote-head { display: flex; flex-wrap: wrap; align-items: flex-start; justify-content: space-between; gap: 8px; }
  .ssh-sec-title { margin: 0; font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .ssh-hint { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); }
  .ssh-path-bar { display: flex; align-items: center; gap: 6px; }
  .ssh-path-bar input { flex: 1; min-inline-size: 0; }
  .ssh-ws-box { display: flex; flex-direction: column; gap: 6px; }
  .ssh-ws-list, .ssh-dir-browser { display: flex; flex-direction: column; gap: 4px; max-block-size: 200px; overflow-y: auto; }
  .ssh-dir-row {
    display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: var(--radius-s);
    border: 1px solid var(--border); background: var(--surface); color: var(--text); text-align: start;
    cursor: pointer; min-inline-size: 0; transition: background var(--dur-fast) var(--ease-out);
  }
  .ssh-dir-row:hover { background: var(--hover); }
  .ssh-dir-row.is-picked { border-color: var(--accent-line); background: var(--accent-soft); }
  .ssh-ws-name { font-size: var(--fs-s); font-weight: 500; flex-shrink: 0; }
  .ssh-dir-open {
    display: inline-flex; align-items: center; gap: 2px;
    margin-inline-start: auto; font-size: var(--fs-xs); color: var(--accent-text); flex-shrink: 0;
  }
  .ssh-hint-card {
    display: flex; align-items: flex-start; gap: 8px; padding: 10px; border-radius: var(--radius-s);
    background: var(--surface); border: 1px solid var(--border); font-size: var(--fs-xs); color: var(--text-dim);
  }
  .ssh-mono, .ssh-mono-path {
    font-family: var(--font-mono); font-size: var(--fs-xs);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-inline-size: 0;
  }
  .ssh-mono-path { color: var(--text-dim); }
  .ssh-err { font-size: var(--fs-xs); color: var(--danger); }

  @media (max-width: 640px) {
    .ssh-grid { grid-template-columns: 1fr; }
  }
</style>
