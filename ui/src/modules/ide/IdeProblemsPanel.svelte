<script lang="ts">
  // Workspace & current-file Diagnostics panel: grouped by file, filterable by
  // Error / Warning / Info and search text, click-to-jump to line:col, and
  // Quick Fix… (⌥⏎) anchored menu powered by LSP / code-actions.
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import {
    ide,
    ideBasename,
    type IdeDiagnostic,
  } from './ideStore.svelte';

  type SeverityFilter = 'all' | 'error' | 'warning' | 'info';

  let severityFilter = $state<SeverityFilter>('all');
  let currentFileOnly = $state(false);
  let query = $state('');
  let collapsedFiles = $state<Record<string, boolean>>({});
  let fixingKey = $state<string | null>(null);

  const scopedProblems = $derived(
    currentFileOnly && ide.activePath
      ? ide.problems.filter((d) => d.path === ide.activePath)
      : ide.problems,
  );

  const errorCount = $derived(scopedProblems.filter((d) => d.severity === 'error').length);
  const warnCount = $derived(scopedProblems.filter((d) => d.severity === 'warning').length);
  const infoCount = $derived(
    scopedProblems.filter((d) => d.severity === 'info' || d.severity === 'hint').length,
  );

  const filteredProblems = $derived.by((): IdeDiagnostic[] => {
    const q = query.trim().toLowerCase();
    return scopedProblems.filter((d) => {
      if (severityFilter === 'error' && d.severity !== 'error') return false;
      if (severityFilter === 'warning' && d.severity !== 'warning') return false;
      if (severityFilter === 'info' && d.severity !== 'info' && d.severity !== 'hint') {
        return false;
      }
      if (!q) return true;
      return (
        d.message.toLowerCase().includes(q) ||
        d.path.toLowerCase().includes(q) ||
        (d.source ?? '').toLowerCase().includes(q) ||
        (d.code ?? '').toLowerCase().includes(q)
      );
    });
  });

  interface ProblemGroup {
    path: string;
    relPath: string;
    items: IdeDiagnostic[];
    errors: number;
    warnings: number;
  }

  const fileGroups = $derived.by((): ProblemGroup[] => {
    const map = new Map<string, IdeDiagnostic[]>();
    for (const d of filteredProblems) {
      const list = map.get(d.path);
      if (list) list.push(d);
      else map.set(d.path, [d]);
    }
    const groups: ProblemGroup[] = [];
    for (const [path, items] of map.entries()) {
      const sorted = [...items].sort((a, b) => a.line - b.line || (a.column ?? 1) - (b.column ?? 1));
      groups.push({
        path,
        relPath: ide.relativePath(path) || ideBasename(path),
        items: sorted,
        errors: sorted.filter((x) => x.severity === 'error').length,
        warnings: sorted.filter((x) => x.severity === 'warning').length,
      });
    }
    return groups;
  });

  function diagKey(d: IdeDiagnostic, idx: number): string {
    return `${d.path}:${d.line}:${d.column ?? 1}:${d.severity}:${idx}`;
  }

  function toggleFile(path: string): void {
    collapsedFiles = { ...collapsedFiles, [path]: !collapsedFiles[path] };
  }

  function jumpToProblem(d: IdeDiagnostic): void {
    void ide.openFile(d.path, { line: d.line, col: d.column ?? 1 });
  }

  function resetFilters(): void {
    severityFilter = 'all';
    currentFileOnly = false;
    query = '';
  }

  async function openQuickFixes(e: MouseEvent, d: IdeDiagnostic, key: string): Promise<void> {
    const anchor = e.currentTarget as HTMLElement | null;
    fixingKey = key;
    try {
      const actions = await ide.fetchCodeActions(d.path, d.line, d.column ?? 1);
      const items: MenuItem[] =
        actions.length > 0
          ? actions.map((act) => ({
              label: act.title,
              icon: 'bulb',
              hint: act.kind,
              action: () => {
                void ide.applyCodeAction(d.path, act);
              },
            }))
          : [
              {
                label: 'No quick fixes available here',
                disabled: true,
              },
            ];
      items.push(
        { separator: true },
        {
          label: `Jump to ${ideBasename(d.path)}:${d.line}`,
          icon: 'file',
          action: () => jumpToProblem(d),
        },
        {
          label: 'Copy diagnostic message',
          icon: 'copy',
          action: () => {
            void copyWithToast(d.message, 'diagnostic');
          },
        },
      );
      ctxMenu.showAt(anchor, items, { align: 'end' });
    } finally {
      fixingKey = null;
    }
  }
</script>

<div class="prob-root" data-testid="ide-problems-panel">
  <header class="prob-bar">
    <div class="segmented" role="group" aria-label="Filter diagnostics by severity">
      <button
        type="button"
        aria-pressed={severityFilter === 'all'}
        onclick={() => (severityFilter = 'all')}
      >
        All ({scopedProblems.length})
      </button>
      <button
        type="button"
        aria-pressed={severityFilter === 'error'}
        onclick={() => (severityFilter = 'error')}
      >
        Errors ({errorCount})
      </button>
      <button
        type="button"
        aria-pressed={severityFilter === 'warning'}
        onclick={() => (severityFilter = 'warning')}
      >
        Warnings ({warnCount})
      </button>
      <button
        type="button"
        aria-pressed={severityFilter === 'info'}
        onclick={() => (severityFilter = 'info')}
      >
        Info ({infoCount})
      </button>
    </div>

    {#if ide.activePath}
      <button
        type="button"
        class="chip"
        aria-pressed={currentFileOnly}
        onclick={() => (currentFileOnly = !currentFileOnly)}
      >
        Current file
      </button>
    {/if}

    <span class="grow"></span>

    <div class="input-group prob-search">
      <Icon name="search" size={12} />
      <input
        type="search"
        dir="ltr"
        aria-label="Filter problems"
        placeholder="Filter problems…"
        bind:value={query}
      />
    </div>

    <button
      type="button"
      class="icon-btn"
      aria-label="Clear diagnostics"
      title="Clear diagnostics"
      disabled={ide.problems.length === 0}
      onclick={() => ide.clearProblems()}
    >
      <Icon name="trash" size={13} />
    </button>
  </header>

  <div class="prob-body">
    {#if ide.problems.length === 0}
      <EmptyState
        icon="check"
        title="No problems detected"
        body="Compiler, linter, and language-server diagnostics for your workspace appear here."
        headingLevel={2}
      />
    {:else if fileGroups.length === 0}
      <EmptyState
        icon="filter"
        title="No matching problems"
        body="No diagnostics match the active filter."
        actionLabel="Clear filters"
        actionKind="secondary"
        onaction={resetFilters}
        headingLevel={2}
      />
    {:else}
      {#each fileGroups as group (group.path)}
        {@const collapsed = !!collapsedFiles[group.path]}
        <section class="prob-group" aria-label={`Diagnostics in ${group.relPath}`}>
          <button
            type="button"
            class="prob-file-head"
            aria-expanded={!collapsed}
            onclick={() => toggleFile(group.path)}
          >
            <Icon name={collapsed ? 'chevronRight' : 'chevronDown'} size={12} />
            <Icon name="file" size={12} />
            <span class="prob-file-path mono ellipsis" dir="ltr" title={group.path}>
              {group.relPath}
            </span>
            <Badge
              tone={group.errors > 0 ? 'bad' : group.warnings > 0 ? 'warn' : 'info'}
              variant="outline"
              label={String(group.items.length)}
            />
          </button>

          {#if !collapsed}
            {#each group.items as d, idx (diagKey(d, idx))}
              {@const k = diagKey(d, idx)}
              <div class="prob-row reveal-host">
                <button type="button" class="prob-jump" onclick={() => jumpToProblem(d)}>
                  {#if d.severity === 'error'}
                    <span class="prob-sev is-err" aria-hidden="true"><Icon name="x" size={12} /></span>
                    <span class="sr-only">Error:</span>
                  {:else if d.severity === 'warning'}
                    <span class="prob-sev is-warn" aria-hidden="true"><Icon name="warning" size={12} /></span>
                    <span class="sr-only">Warning:</span>
                  {:else}
                    <span class="prob-sev is-info" aria-hidden="true"><Icon name="info" size={12} /></span>
                    <span class="sr-only">Info:</span>
                  {/if}
                  <span class="prob-msg" dir="auto">{d.message}</span>
                  {#if d.source || d.code}
                    <span class="prob-meta mono dim" dir="ltr">
                      {[d.source, d.code].filter(Boolean).join(' · ')}
                    </span>
                  {/if}
                  <span class="prob-loc mono dim" dir="ltr">
                    :{d.line}:{d.column ?? 1}
                  </span>
                </button>

                <button
                  type="button"
                  class="icon-btn reveal-on-hover"
                  aria-label="Copy diagnostic message"
                  title="Copy diagnostic message"
                  onclick={() => void copyWithToast(d.message, 'diagnostic')}
                >
                  <Icon name="copy" size={12} />
                </button>
                <button
                  type="button"
                  class="btn small ghost prob-fix"
                  aria-keyshortcuts="Alt+Enter"
                  disabled={fixingKey === k}
                  onclick={(e) => void openQuickFixes(e, d, k)}
                >
                  <Icon name="bulb" size={12} />
                  <span>{fixingKey === k ? 'Checking…' : 'Quick fix…'}</span>
                </button>
              </div>
            {/each}
          {/if}
        </section>
      {/each}
    {/if}
  </div>
</div>

<style>
  .prob-root {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
    min-inline-size: 0;
    background: var(--surface);
  }
  .prob-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding-block: 4px;
    padding-inline: 10px;
    border-block-end: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
  }
  .prob-search {
    block-size: 24px;
    inline-size: 190px;
    font-size: var(--fs-xs);
  }
  .prob-body {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
  }
  .prob-group {
    border-block-end: 1px solid var(--separator);
  }
  .prob-file-head {
    display: flex;
    align-items: center;
    gap: 6px;
    inline-size: 100%;
    padding-block: 6px;
    padding-inline: 10px;
    border: 0;
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-xs);
    font-weight: 600;
    text-align: start;
    cursor: pointer;
  }
  .prob-file-head:hover {
    background: var(--hover);
  }
  .prob-file-path {
    flex: 1;
    font-size: var(--fs-xs);
  }
  .prob-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-inline-end: 8px;
    border-block-start: 1px solid var(--separator);
  }
  .prob-row:hover {
    background: var(--hover);
  }
  .prob-jump {
    flex: 1;
    min-inline-size: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding-block: 6px;
    padding-inline: 22px 8px;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
  }
  .prob-sev {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
  }
  .prob-sev.is-err {
    color: var(--danger);
  }
  .prob-sev.is-warn {
    color: var(--warning);
  }
  .prob-sev.is-info {
    color: var(--info);
  }
  .prob-msg {
    flex: 1;
    min-inline-size: 0;
    overflow-wrap: anywhere;
  }
  .prob-meta {
    flex-shrink: 0;
    font-size: var(--fs-xs);
  }
  .prob-loc {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .prob-fix {
    flex-shrink: 0;
  }
  @media (max-width: 640px) {
    .prob-search {
      inline-size: 100%;
    }
    .prob-jump {
      flex-wrap: wrap;
      padding-inline-start: 12px;
    }
  }
</style>
