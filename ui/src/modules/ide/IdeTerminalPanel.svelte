<script module lang="ts">
  interface IdeTermTab {
    id: string;
    label: string;
    kind: 'local' | 'ssh';
    targetKey: string;
    cwd: string;
  }

  let savedTabs = $state<IdeTermTab[]>([]);
  let savedActiveId = $state<string | null>(null);
  let savedSplit = $state(false);
  let handledTerminalSeq = 0;
  const autoAttemptedTargets = new Set<string>();
</script>

<script lang="ts">
  // IDE Terminal panel: spawns or attaches to an Otto shell PTY session in the
  // local workspace `cwd`, or opens a remote SSH PTY session for the active SSH
  // connection, rendering Otto's code-split `LazyTerminal` cleanly.
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { api } from '../../lib/api/client';
  import type { Session } from '../../lib/api/types';
  import { loadErrorText } from '../../lib/loadError';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
  import { toastError } from '../../lib/toastError';
  import { ide, ideBasename } from './ideStore.svelte';

  let spawning = $state(false);
  let spawnError = $state<string | null>(null);
  let primaryRef = $state<ReturnType<typeof LazyTerminal> | undefined>();

  const isSshTarget = $derived(ide.target.kind === 'ssh' && !!ide.target.connection_id);
  const targetKey = $derived(
    isSshTarget
      ? `ssh:${ide.target.connection_id}:${ide.target.root}`
      : `local:${ide.target.root}`,
  );
  const folderLabel = $derived(
    ideBasename(ide.target.root) || ide.target.connection_name || 'workspace',
  );

  // Adopt any existing live `ide_terminal` shell in this project on reload.
  $effect(() => {
    if (!ws.listSettled || spawning || isSshTarget || !ide.target.root) return;
    const known = new Set(savedTabs.map((t) => t.id));
    const roots = ide.projectRoots;
    for (const s of ws.sessions) {
      if (s.archived || known.has(s.id) || s.meta?.ide_terminal !== true) continue;
      const sCwd = s.cwd ?? '';
      const inProject = sCwd === ide.target.root || roots.some((r) => sCwd === r.path || sCwd.startsWith(`${r.path}/`));
      if (inProject) {
        savedTabs = [
          ...savedTabs,
          {
            id: s.id,
            label: s.title || `Shell ${savedTabs.length + 1}`,
            kind: 'local',
            targetKey,
            cwd: sCwd || ide.target.root,
          },
        ];
        known.add(s.id);
      }
    }
  });

  // Prune local shell tabs that were deleted/archived outside the IDE.
  $effect(() => {
    if (!ws.listSettled) return;
    const liveIds = new Set(ws.sessions.filter((s) => !s.archived).map((s) => s.id));
    const next = savedTabs.filter((t) => t.kind === 'ssh' || liveIds.has(t.id));
    if (next.length !== savedTabs.length) {
      savedTabs = next;
    }
  });

  const targetTabs = $derived(savedTabs.filter((t) => t.targetKey === targetKey));
  const activeTab = $derived(
    targetTabs.find((t) => t.id === savedActiveId) ?? targetTabs[0] ?? null,
  );
  const secondTab = $derived(
    savedSplit ? (targetTabs.find((t) => t.id !== activeTab?.id) ?? null) : null,
  );

  // Handle "Open in terminal" requests from the Project tree context menu.
  $effect(() => {
    const req = ide.pendingTerminalCwd;
    if (!req || req.seq === handledTerminalSeq || spawning || !ide.hasTarget) return;
    if (!isSshTarget && !ws.listSettled) return;
    handledTerminalSeq = req.seq;
    ide.pendingTerminalCwd = null;
    autoAttemptedTargets.add(targetKey);
    const existing = targetTabs.find((t) => t.cwd === req.cwd);
    if (existing) {
      savedActiveId = existing.id;
      return;
    }
    void spawnTerminal(req.cwd);
  });

  // Auto-open one shell per target when the user first visits Terminal.
  $effect(() => {
    if (!ide.hasTarget || spawning || ide.pendingTerminalCwd) return;
    if (!isSshTarget && !ws.listSettled) return;
    if (targetTabs.length > 0 || autoAttemptedTargets.has(targetKey)) return;
    autoAttemptedTargets.add(targetKey);
    void spawnTerminal();
  });

  async function spawnTerminal(customCwd?: string): Promise<void> {
    if (spawning || !ide.hasTarget) return;
    spawning = true;
    spawnError = null;
    const launchCwd = customCwd || ide.target.root;
    const dirName = ideBasename(launchCwd) || folderLabel;
    try {
      const idx = targetTabs.length + 1;
      if (isSshTarget && ide.target.connection_id) {
        const hostName = ide.target.connection_name || ide.target.connection_id;
        const s = await api.post<Session>(
          `/connections/${encodeURIComponent(ide.target.connection_id)}/open`,
          { title: `IDE SSH · ${hostName}` },
        );
        ws.statusMap[s.id] = s.status;
        const entry: IdeTermTab = {
          id: s.id,
          label: `SSH ${idx} · ${hostName}`,
          kind: 'ssh',
          targetKey,
          cwd: launchCwd,
        };
        savedTabs = [...savedTabs.filter((t) => t.id !== s.id), entry];
        savedActiveId = s.id;
      } else {
        const s = await ws.createSessionQuiet(
          {
            kind: 'agent',
            provider: 'shell',
            title: `IDE · ${dirName}`,
            cwd: launchCwd || null,
            meta: { surface: 'ide', ide_terminal: true },
          },
          { scratch: ws.current === null && ws.currentId !== SCRATCH_WORKSPACE_ID },
        );
        const sub = launchCwd && launchCwd !== ide.target.root ? ` · ${dirName}` : '';
        const entry: IdeTermTab = {
          id: s.id,
          label: `Terminal ${idx}${sub}`,
          kind: 'local',
          targetKey,
          cwd: launchCwd,
        };
        savedTabs = [...savedTabs.filter((t) => t.id !== s.id), entry];
        savedActiveId = s.id;
      }
    } catch (e) {
      const cause = loadErrorText(e);
      if (targetTabs.length === 0) spawnError = cause;
      else toastError('Couldn’t open terminal', e);
    } finally {
      spawning = false;
    }
  }

  async function killTerminal(id: string): Promise<void> {
    try {
      await ws.killSession(id);
      savedTabs = savedTabs.filter((t) => t.id !== id);
      if (savedActiveId === id) {
        savedActiveId = savedTabs.find((t) => t.targetKey === targetKey)?.id ?? null;
      }
    } catch (e) {
      toastError('Couldn’t close terminal', e);
    }
  }
</script>

<div class="iterm-root" data-testid="ide-terminal-panel">
  <header class="iterm-bar">
    <Badge
      tone={isSshTarget ? 'info' : 'neutral'}
      variant={isSshTarget ? 'soft' : 'outline'}
      label={isSshTarget ? `SSH · ${ide.target.connection_name || 'remote'}` : `Local · ${folderLabel}`}
    />

    {#if targetTabs.length > 0}
      <div class="iterm-tabs" role="group" aria-label="IDE terminals">
        {#each targetTabs as t (t.id)}
          <button
            type="button"
            class="iterm-tab"
            class:is-active={t.id === activeTab?.id}
            class:is-split-peer={t.id === secondTab?.id}
            aria-pressed={t.id === activeTab?.id}
            title={t.id === secondTab?.id ? `${t.cwd} (split pane)` : t.cwd}
            onclick={() => (savedActiveId = t.id)}
          >
            <Icon name="terminal" size={12} />
            <span>{t.label}</span>
          </button>
        {/each}
      </div>
    {/if}

    <span class="grow"></span>

    <button
      type="button"
      class="icon-btn"
      aria-label="Find in terminal"
      title="Find in terminal"
      disabled={!activeTab}
      onclick={() => primaryRef?.openFind()}
    >
      <Icon name="search" size={12} />
    </button>
    <button
      type="button"
      class="icon-btn"
      aria-label="New terminal"
      title="New terminal"
      disabled={!ide.hasTarget || spawning}
      onclick={() => void spawnTerminal()}
    >
      <Icon name="plus" size={13} />
    </button>
    <button
      type="button"
      class="icon-btn"
      class:is-split={savedSplit && targetTabs.length > 1}
      aria-pressed={savedSplit}
      aria-label="Split terminals side by side"
      title="Split terminals side by side"
      disabled={targetTabs.length < 2}
      onclick={() => (savedSplit = !savedSplit)}
    >
      <Icon name="split" size={13} />
    </button>
    <button
      type="button"
      class="icon-btn"
      aria-label="Kill active terminal"
      title="Kill active terminal"
      disabled={!activeTab}
      onclick={() => activeTab && void killTerminal(activeTab.id)}
    >
      <Icon name="trash" size={13} />
    </button>
  </header>

  {#if !ide.hasTarget}
    <div class="iterm-state">
      <EmptyState
        icon="terminal"
        title="No workspace selected"
        body="Choose a local workspace or remote SSH target to open a shell."
        headingLevel={2}
      />
    </div>
  {:else if !activeTab}
    <div class="iterm-state">
      <LoadState
        what="terminal session"
        loading={spawning}
        error={spawnError}
        empty={true}
        onretry={() => void spawnTerminal()}
        variant="panel"
        rows={3}
      >
        {#snippet emptyView()}
          <EmptyState
            icon="terminal"
            title="No open terminal"
            body={isSshTarget
              ? 'Open a remote SSH PTY shell connected to your active target.'
              : 'Open an interactive shell inside your workspace directory.'}
            actionLabel="New terminal"
            actionIcon="plus"
            actionKind="secondary"
            onaction={() => void spawnTerminal()}
            headingLevel={2}
          />
        {/snippet}
      </LoadState>
    </div>
  {:else}
    <div class="iterm-stage otto-force-dark" class:is-split={!!secondTab}>
      {#key activeTab.id}
        <div class="iterm-pane">
          <LazyTerminal bind:this={primaryRef} sessionId={activeTab.id} forceDark autoFocus resumeOnOpen />
        </div>
      {/key}
      {#if secondTab}
        {#key secondTab.id}
          <div class="iterm-pane">
            <LazyTerminal sessionId={secondTab.id} forceDark resumeOnOpen />
          </div>
        {/key}
      {/if}
    </div>
  {/if}
</div>

<style>
  .iterm-root {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
    min-inline-size: 0;
    background: var(--surface);
  }
  .iterm-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    min-block-size: 30px;
    padding-block: 2px;
    padding-inline: 10px;
    border-block-end: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
  }
  .iterm-tabs {
    display: flex;
    align-items: center;
    gap: 2px;
    min-inline-size: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .iterm-tabs::-webkit-scrollbar {
    display: none;
  }
  .iterm-tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    block-size: 24px;
    padding-inline: 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    white-space: nowrap;
    cursor: pointer;
  }
  .iterm-tab:hover {
    background: var(--hover);
    color: var(--text);
  }
  .iterm-tab.is-active {
    background: var(--accent-soft);
    color: var(--accent-text);
    font-weight: 500;
  }
  .iterm-tab.is-split-peer {
    background: var(--surface-2);
    color: var(--text);
    box-shadow: inset 0 0 0 1px var(--border-strong);
  }
  .is-split {
    color: var(--accent-text);
  }
  .iterm-state {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .iterm-stage {
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    display: flex;
    background: var(--term-bg);
  }
  .iterm-pane {
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    display: flex;
  }
  .iterm-stage.is-split .iterm-pane + .iterm-pane {
    border-inline-start: 1px solid var(--border);
  }
  @media (max-width: 640px) {
    .iterm-stage.is-split {
      flex-direction: column;
    }
    .iterm-stage.is-split .iterm-pane + .iterm-pane {
      border-inline-start: 0;
      border-block-start: 1px solid var(--border);
    }
  }
</style>
