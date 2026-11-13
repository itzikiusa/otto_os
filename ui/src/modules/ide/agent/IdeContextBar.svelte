<script lang="ts" module>
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { harnessLabel } from '../../../lib/providers';
  import { ide, ideBasename } from '../ideStore.svelte';
  import { ideAgent } from '../store/ideAgentController.svelte';

  export function showIdeSessionMenu(anchor: HTMLElement): void {
    void ideAgent.refreshWorkspaceSessions();
    const liveList = ideAgent.threads.filter((t) => !t.closed);
    const closedList = ideAgent.threads.filter((t) => t.closed);
    const rows: MenuItem[] = [
      { label: 'New thread', icon: 'plus', pinned: true, action: () => ideAgent.startNewChat() },
      { label: 'Open in Threads', icon: 'external', pinned: true, action: () => ideAgent.openActiveInThreads() },
    ];
    if (liveList.length > 0) {
      rows.push({ separator: true });
      for (const t of liveList) {
        rows.push({
          label: t.title,
          checked: t.id === ideAgent.sessionId,
          hint: harnessLabel(t.provider),
          action: () => void ideAgent.selectSession(t.id),
        });
      }
    }
    if (closedList.length > 0) {
      rows.push({ separator: true });
      for (const t of closedList.slice(0, 20)) {
        rows.push({
          label: t.title,
          checked: t.id === ideAgent.sessionId,
          hint: `${harnessLabel(t.provider)} · Closed`,
          action: () => void ideAgent.selectSession(t.id),
        });
      }
    }
    ctxMenu.showAt(anchor, rows, {
      filter: ideAgent.threads.length > 8,
      filterPlaceholder: 'Filter threads…',
    });
  }

  export function showIdeContextMenu(anchor: HTMLElement | null, onFocusComposer?: () => void): void {
    const failedTest = ide.activeRun?.tests.find((t) => t.status === 'failed') ?? null;
    const wrap = (fn: () => void) => () => {
      fn();
      onFocusComposer?.();
    };
    const menu: MenuItem[] = [
      {
        label: 'Mark current editor selection',
        icon: 'cursor',
        hint: '⌘L',
        pinned: true,
        disabled: !ide.activeTab,
        title: ide.activeTab ? undefined : 'Open a file in the editor first',
        action: wrap(() => {
          ideAgent.markSelectionFromEditor({ openPanel: true });
        }),
      },
      {
        label: ide.activeTab ? `Attach active file (${ide.activeTab.name})` : 'Attach active file',
        icon: 'file',
        pinned: true,
        disabled: !ide.activePath,
        action: wrap(() => {
          if (ide.activePath) void ideAgent.attachFile(ide.activePath, { openPanel: true });
        }),
      },
    ];
    if (ide.problems.length > 0) {
      const top = ide.problems[0];
      menu.push({
        label: `Top problem (${ideBasename(top.path)}:${top.line})`,
        icon: 'warning',
        pinned: true,
        action: wrap(() => ideAgent.attachDiagnostic(top, { openPanel: true })),
      });
    }
    if (failedTest) {
      menu.push({
        label: `Latest failed test (${failedTest.name})`,
        icon: 'flask',
        pinned: true,
        action: wrap(() => ideAgent.attachTestFailure(failedTest, ide.activeRun?.output ?? [], { openPanel: true })),
      });
    }
    if (ide.activeDebug?.state === 'paused') {
      const dbg = ide.activeDebug;
      menu.push({
        label: 'Paused debug frame & locals',
        icon: 'bug',
        pinned: true,
        action: wrap(() => ideAgent.attachDebugFrame(dbg, ide.expandedVariables, { openPanel: true })),
      });
    }
    const otherTabs = ide.tabs.filter((t) => t.path !== ide.activePath);
    if (otherTabs.length > 0) {
      menu.push({ separator: true });
      for (const tab of otherTabs) {
        menu.push({
          label: tab.name,
          icon: 'file',
          hint: 'Open tab',
          action: wrap(() => void ideAgent.attachFile(tab.path, { openPanel: true })),
        });
      }
    }
    ctxMenu.showAt(anchor, menu, {
      filter: otherTabs.length > 6,
      filterPlaceholder: 'Filter files & context…',
    });
  }
</script>

<script lang="ts">
  // Smart toggleable IDE context pills + attached context chips tray above the
  // composer in `IdeAgentPanel.svelte`.
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import type { IdeContextKind } from '../store/ideAgentController.svelte';

  interface Props {
    onfocuscomposer?: () => void;
  }
  let { onfocuscomposer }: Props = $props();

  const CHIP_ICONS: Record<IdeContextKind, IconName> = {
    selection: 'cursor',
    file: 'file',
    symbol: 'zap',
    diagnostic: 'warning',
    test_failure: 'flask',
    debug_frame: 'bug',
    run_output: 'terminal',
  };

  const failedTest = $derived(ide.activeRun?.tests.find((t) => t.status === 'failed') ?? null);
  const hasFailedRun = $derived(
    Boolean(failedTest || (ide.activeRun && (ide.activeRun.status === 'failed' || (ide.activeRun.exit_code ?? 0) !== 0))),
  );
  const activeFileAttached = $derived(Boolean(ide.activePath && ideAgent.hasContextKind('file', ide.activePath)));
  const selectionAttached = $derived(ideAgent.hasContextKind('selection'));
  const problemsAttached = $derived(ideAgent.hasContextKind('diagnostic'));
  const debugAttached = $derived(ideAgent.hasContextKind('debug_frame'));
  const failedRunAttached = $derived(ideAgent.hasContextKind('test_failure') || ideAgent.hasContextKind('run_output'));

  function jumpTo(path?: string, line?: number): void {
    if (path) void ide.openFile(path, { line: line ?? 1, preview: false });
  }
</script>

{#if ide.activeTab || ide.problems.length > 0 || ide.activeDebug?.state === 'paused' || hasFailedRun}
  <div class="ia-pills" role="group" aria-label="Toggle IDE context">
    {#if ide.activeTab}
      <button
        type="button"
        class="ia-pill"
        class:on={activeFileAttached}
        aria-pressed={activeFileAttached}
        title="Include active file ({ide.activeTab.name})"
        onclick={() => void ideAgent.toggleActiveFilePill()}
      >
        <Icon name="file" size={12} />
        <span class="ellipsis">{ide.activeTab.name}</span>
      </button>
      <button
        type="button"
        class="ia-pill"
        class:on={selectionAttached}
        aria-pressed={selectionAttached}
        title="Include current editor selection"
        onclick={() => ideAgent.toggleSelectionPill()}
      >
        <Icon name="cursor" size={12} />
        <span>Selection</span>
      </button>
    {/if}
    {#if ide.problems.length > 0}
      <button
        type="button"
        class="ia-pill"
        class:on={problemsAttached}
        aria-pressed={problemsAttached}
        title="Include workspace diagnostics ({ide.problems.length})"
        onclick={() => ideAgent.toggleProblemsPill()}
      >
        <Icon name="warning" size={12} />
        <span>Problems ({ide.problems.length})</span>
      </button>
    {/if}
    {#if ide.activeDebug?.state === 'paused'}
      <button
        type="button"
        class="ia-pill"
        class:on={debugAttached}
        aria-pressed={debugAttached}
        title="Include paused debugger stack & locals"
        onclick={() => ideAgent.toggleDebugPill()}
      >
        <Icon name="bug" size={12} />
        <span>Debug</span>
      </button>
    {/if}
    {#if hasFailedRun}
      <button
        type="button"
        class="ia-pill"
        class:on={failedRunAttached}
        aria-pressed={failedRunAttached}
        title="Include latest failed test or run output"
        onclick={() => ideAgent.toggleFailedRunPill()}
      >
        <Icon name="flask" size={12} />
        <span>Failed run</span>
      </button>
    {/if}
  </div>
{/if}

{#if ideAgent.contexts.length > 0}
  <div class="ia-tray" data-testid="ide-agent-context-tray">
    {#each ideAgent.contexts as chip (chip.id)}
      <div class="ia-ctx-item" data-testid="ide-context-chip">
        <button
          type="button"
          class="ia-ctx-jump"
          dir="ltr"
          disabled={!chip.path}
          title={chip.sublabel ? `${chip.label} — ${chip.sublabel}` : chip.label}
          onclick={() => jumpTo(chip.path, chip.startLine)}
        >
          <Icon name={CHIP_ICONS[chip.kind] ?? 'file'} size={12} />
          <span class="ellipsis">{chip.label}</span>
        </button>
        <button
          type="button"
          class="ia-ctx-rm"
          aria-label="Remove {chip.label} from context"
          title="Remove from context"
          onclick={() => {
            ideAgent.removeContext(chip.id);
            onfocuscomposer?.();
          }}
        >
          <Icon name="x" size={12} />
        </button>
      </div>
    {/each}
    {#if ideAgent.contexts.length > 1}
      <button
        type="button"
        class="ia-clear-btn"
        onclick={() => {
          ideAgent.clearContexts();
          onfocuscomposer?.();
        }}
      >
        Clear
      </button>
    {/if}
  </div>
{/if}

<style>
  .ia-pills {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
  }
  .ia-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-inline-size: 160px;
    block-size: 22px;
    padding-inline: 6px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .ia-pill:hover {
    background: var(--hover);
    color: var(--text);
  }
  .ia-pill.on {
    border-color: var(--accent-line);
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .ia-tray {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
  }
  .ia-ctx-item {
    display: inline-flex;
    align-items: center;
    max-inline-size: 100%;
    block-size: 22px;
    border-radius: var(--radius-s);
    border: 1px solid var(--accent-line);
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: var(--fs-xs);
    overflow: hidden;
  }
  .ia-ctx-jump {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-inline-size: 0;
    max-inline-size: 200px;
    block-size: 100%;
    padding-inline: 6px 4px;
    border: none;
    background: transparent;
    color: inherit;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .ia-ctx-jump:hover:not(:disabled) {
    background: var(--accent-soft-strong);
  }
  .ia-ctx-jump:disabled {
    cursor: default;
  }
  .ia-ctx-rm {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    inline-size: 20px;
    block-size: 100%;
    border: none;
    background: transparent;
    color: inherit;
    cursor: pointer;
    flex-shrink: 0;
  }
  .ia-ctx-rm:hover {
    background: var(--accent-soft-strong);
  }
  .ia-clear-btn {
    padding-block: 2px;
    padding-inline: 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .ia-clear-btn:hover {
    color: var(--text);
    background: var(--hover);
  }
  @media (pointer: coarse) {
    .ia-pill,
    .ia-ctx-item,
    .ia-clear-btn {
      min-block-size: var(--hit-min);
    }
    .ia-ctx-rm {
      min-inline-size: var(--hit-min);
    }
  }
</style>
