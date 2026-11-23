<script module lang="ts">
  import { pruneIdeTerminalTabs, type IdeTerminalTabEntry as IdeTermTab } from './store/ideTerminalTabs';

  let savedTabs = $state<IdeTermTab[]>([]);
  // The selected shell per project (I3-19): coming back to A selects A's shell, not its first.
  let savedActiveByTarget = $state<Record<string, string | null>>({});
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
  import { confirmer } from '../../lib/confirm.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
  import { toastError } from '../../lib/toastError';
  import { ide, ideBasename } from './ideStore.svelte';

  /** The tab label saved on the session (restored after a reload), else its title. */
  function ideTermLabel(s: Session): string {
    const saved = s.meta?.ide_label;
    return typeof saved === 'string' && saved ? saved : s.title || '';
  }

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
            label: ideTermLabel(s) || `Shell ${savedTabs.length + 1}`,
            kind: 'local',
            targetKey,
            cwd: sCwd || ide.target.root,
          },
        ];
        known.add(s.id);
      }
    }
  });

  // Prune local and SSH shell tabs whose session was deleted or archived outside the IDE.
  $effect(() => {
    if (!ws.listSettled) return;
    const next = pruneIdeTerminalTabs(
      savedTabs,
      ws.sessions,
      ws.otherWsSessions,
      ws.archivedSessions,
      ws.statusMap,
    );
    if (next.length !== savedTabs.length) {
      savedTabs = next;
    }
  });

  const targetTabs = $derived(savedTabs.filter((t) => t.targetKey === targetKey));
  /** Next tab number: one past the highest shown, so Kill + New never repeats a label. */
  const nextTabNumber = $derived(
    1 + Math.max(0, ...targetTabs.map((t) => Number(/(\d+)/.exec(t.label)?.[1] ?? 0))),
  );
  const activeTab = $derived(
    targetTabs.find((t) => t.id === savedActiveByTarget[targetKey]) ?? targetTabs[0] ?? null,
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
      savedActiveByTarget[targetKey] = existing.id;
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

  // Clear stale spawn error when switching to a different project/target.
  $effect(() => {
    void targetKey;
    spawnError = null;
  });

  async function spawnTerminal(customCwd?: string): Promise<void> {
    if (spawning || !ide.hasTarget) return;
    spawning = true;
    spawnError = null;
    const launchTargetKey = targetKey;
    const launchRoot = ide.target.root;
    const launchIsSsh = isSshTarget;
    const launchConnId = ide.target.connection_id;
    const launchConnName = ide.target.connection_name;
    const launchCwd = customCwd || launchRoot;
    const dirName = ideBasename(launchCwd) || folderLabel;
    try {
      const idx = nextTabNumber;
      if (launchIsSsh && launchConnId) {
        const hostName = launchConnName || launchConnId;
        // Connections are global profiles: the session needs a workspace to live in.
        const s = await api.post<Session>(
          `/connections/${encodeURIComponent(launchConnId)}/open`,
          {
            title: `IDE SSH · ${hostName}`,
            workspace_id: ws.current ? ws.currentId : SCRATCH_WORKSPACE_ID,
            cwd: launchCwd || null,
          },
        );
        ws.statusMap[s.id] = s.status;
        const entry: IdeTermTab = {
          id: s.id,
          label: `SSH ${idx} · ${hostName}`,
          kind: 'ssh',
          targetKey: launchTargetKey,
          cwd: launchCwd,
        };
        savedTabs = [...savedTabs.filter((t) => t.id !== s.id), entry];
        savedActiveByTarget[launchTargetKey] = s.id;
      } else {
        const sub = launchCwd && launchCwd !== launchRoot ? ` · ${dirName}` : '';
        const label = `Terminal ${idx}${sub}`;
        const s = await ws.createSessionQuiet(
          {
            kind: 'agent',
            provider: 'shell',
            title: `IDE · ${dirName}`,
            cwd: launchCwd || null,
            meta: { surface: 'ide', ide_terminal: true, ide_root: launchRoot, ide_label: label },
          },
          { scratch: ws.current === null && ws.currentId !== SCRATCH_WORKSPACE_ID },
        );
        const entry: IdeTermTab = {
          id: s.id,
          label,
          kind: 'local',
          targetKey: launchTargetKey,
          cwd: launchCwd,
        };
        savedTabs = [...savedTabs.filter((t) => t.id !== s.id), entry];
        savedActiveByTarget[launchTargetKey] = s.id;
      }
    } catch (e) {
      if (targetKey === launchTargetKey) {
        const cause = loadErrorText(e);
        if (targetTabs.length === 0) spawnError = cause;
        else toastError('Couldn’t open terminal', e);
      }
    } finally {
      spawning = false;
    }
  }

  async function killTerminal(tab: IdeTermTab): Promise<void> {
    const status = ws.statusMap[tab.id];
    const isRunning = tab.kind === 'ssh' || status !== 'exited';
    if (isRunning) {
      const ok = await confirmer.ask(
        tab.kind === 'ssh'
          ? `Close SSH session “${tab.label}” and disconnect?`
          : `Close “${tab.label}” and stop any running process?`,
        { title: 'Close terminal', confirmLabel: 'Close terminal', danger: true },
      );
      if (!ok) return;
    }
    try {
      await ws.killSession(tab.id);
      savedTabs = savedTabs.filter((t) => t.id !== tab.id);
      if (savedActiveByTarget[tab.targetKey] === tab.id) {
        savedActiveByTarget[tab.targetKey] = savedTabs.find((t) => t.targetKey === tab.targetKey)?.id ?? null;
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
            onclick={() => (savedActiveByTarget[targetKey] = t.id)}
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
      onclick={() => activeTab && void killTerminal(activeTab)}
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
    <!-- No floating zoom/copy pill over the output: ⌘+ / ⌘− size the text, right-click or ⌘C copies. -->
    <div class="iterm-stage otto-force-dark" class:is-split={!!secondTab}>
      {#key activeTab.id}
        <div class="iterm-pane">
          <LazyTerminal bind:this={primaryRef} sessionId={activeTab.id} forceDark autoFocus resumeOnOpen showToolbar={false} />
        </div>
      {/key}
      {#if secondTab}
        {#key secondTab.id}
          <div class="iterm-pane">
            <LazyTerminal sessionId={secondTab.id} forceDark resumeOnOpen showToolbar={false} />
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
