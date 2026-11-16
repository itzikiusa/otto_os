<script lang="ts">
  // Live DB Watch & Component-Test Watcher panel for the IDE Bottom Dock.
  // Auto-detects tables, collections, and Redis key prefixes in DAO/source files,
  // connects to saved Otto DB connections or live Testcontainers/local processes,
  // and computes before -> after diffs automatically on debugger pauses and test runs.
  import { untrack } from 'svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Switch from '../../../lib/components/Switch.svelte';
  import { pollWhileVisible } from '../../../lib/poll';
  import { router } from '../../../lib/router.svelte';
  import type { IdeDbEngine, IdeDbFieldChange } from '../../../lib/api/types/ide-db-watch';
  import { ide, ideBasename } from '../ideStore.svelte';
  import {
    formatDbCellValue,
    IDE_DB_ENGINES,
    ideDbWatch,
    targetKeyOf,
  } from './ideDbWatchState.svelte';

  function refreshTargets(): void {
    if (!ide.target.root) return;
    void ideDbWatch.detect({
      root: ide.target.root,
      activeFile: ide.activePath,
      connectionId: ide.target.connection_id,
      additionalRoots: ide.additionalRoots,
    });
  }

  // Re-detect accessed DB objects and ephemeral Testcontainers when project or active file changes.
  $effect(() => {
    const root = ide.target.root;
    const activeFile = ide.activePath;
    const connId = ide.target.connection_id;
    const extraCount = ide.additionalRoots.length;
    void activeFile;
    void connId;
    void extraCount;
    if (!root) return;
    untrack(() => refreshTargets());
  });

  // Auto-refresh snapshot diff when debugger pauses on a new location/frame or a test run finishes.
  let lastPauseSig = '';
  let lastRunSig = '';

  $effect(() => {
    const dbg = ide.activeDebug;
    const pauseSig =
      dbg && dbg.state === 'paused'
        ? `${dbg.id}:${dbg.paused_file ?? ''}:${dbg.paused_line ?? 0}:${dbg.active_frame_id ?? 0}`
        : '';
    if (pauseSig && pauseSig !== lastPauseSig) {
      lastPauseSig = pauseSig;
      if (untrack(() => ideDbWatch.autoRefreshOnExec && ideDbWatch.canCapture && Boolean(ide.target.root))) {
        untrack(() => void ideDbWatch.captureSnapshot(ide.target.root, 'diff'));
      }
    } else if (!pauseSig) {
      lastPauseSig = '';
    }
  });

  $effect(() => {
    const run = ide.activeRun;
    const runSig = run ? `${run.id}:${run.status}` : '';
    const wasRunning = lastRunSig.endsWith(':running');
    lastRunSig = runSig;
    if (wasRunning && run && run.status !== 'running') {
      if (untrack(() => ideDbWatch.autoRefreshOnExec && ideDbWatch.canCapture && Boolean(ide.target.root))) {
        untrack(() => void ideDbWatch.captureSnapshot(ide.target.root, 'diff'));
      }
    }
  });

  // Optional 3s live watch poller — pauses automatically when dock or window is hidden.
  $effect(() => {
    if (!ideDbWatch.liveWatch || !ide.bottomDockOpen || ide.activeBottomTab !== 'dbwatch') return;
    const root = ide.target.root;
    if (!root) return;
    const poller = pollWhileVisible(
      async () => {
        if (!untrack(() => ideDbWatch.canCapture) || untrack(() => ideDbWatch.snapshotLoading)) return;
        await ideDbWatch.captureSnapshot(root, 'diff');
      },
      { ms: 3000, floorMs: 2000, immediate: false },
    );
    return () => poller.stop();
  });

  const resp = $derived(ideDbWatch.latestResp);
  const diff = $derived(resp?.diff ?? null);
  const snapshotCols = $derived(resp?.snapshot.columns ?? []);
  const snapshotRows = $derived(resp?.snapshot.rows ?? []);
  const selectedTarget = $derived(ideDbWatch.selectedTarget);

  function changeForColumn(changes: readonly IdeDbFieldChange[], colName: string): IdeDbFieldChange | undefined {
    return changes.find((c) => c.column === colName);
  }
</script>

<div class="dbw-root" data-testid="ide-dbwatch-panel">
  <header class="dbw-toolbar">
    <div class="dbw-target-group">
      <label class="dbw-field-label" for="ide-dbw-target">Target</label>
      <select
        id="ide-dbw-target"
        class="input dbw-select"
        aria-label="Database target"
        value={ideDbWatch.selectedTargetKey}
        onchange={(e) => ideDbWatch.selectTargetByKey((e.currentTarget as HTMLSelectElement).value)}
        data-testid="ide-dbw-target-select"
      >
        {#if ideDbWatch.allTargets.length === 0}
          <option value="">No targets detected</option>
        {:else}
          {#if ideDbWatch.ephemeralTargets.length > 0}
            <optgroup label="Live Testcontainers & Local Processes">
              {#each ideDbWatch.ephemeralTargets as ep (ep.ephemeral_id)}
                <option value={targetKeyOf(ep)}>
                  {ep.name} ({ep.engine} · {ep.host}:{ep.port})
                </option>
              {/each}
            </optgroup>
          {/if}
          {#if ideDbWatch.savedConnections.length > 0}
            <optgroup label="Saved DB Connections">
              {#each ideDbWatch.savedConnections as sv (sv.connection_id)}
                <option value={targetKeyOf(sv)}>
                  {sv.name} ({sv.engine}{sv.database ? `/${sv.database}` : ''})
                </option>
              {/each}
            </optgroup>
          {/if}
        {/if}
      </select>

      <button
        type="button"
        class="icon-btn"
        aria-label="Refresh containers and targets"
        title="Refresh live Testcontainers and detected DB objects"
        disabled={ideDbWatch.detectLoading || !ide.target.root}
        onclick={refreshTargets}
        data-testid="ide-dbw-refresh-targets"
      >
        <Icon name="refresh" size={13} />
      </button>

      <select
        class="input dbw-engine-select"
        aria-label="Database engine"
        value={ideDbWatch.selectedEngine}
        onchange={(e) => (ideDbWatch.selectedEngine = (e.currentTarget as HTMLSelectElement).value as IdeDbEngine)}
        data-testid="ide-dbw-engine-select"
      >
        {#each IDE_DB_ENGINES as eng (eng.id)}
          <option value={eng.id}>{eng.label}</option>
        {/each}
      </select>

      <input
        class="input dbw-input mono"
        type="text"
        dir="ltr"
        aria-label="Table, collection, or key prefix"
        placeholder={ideDbWatch.selectedEngine === 'redis' ? 'Key prefix (e.g. session:*)' : ideDbWatch.selectedEngine === 'mongodb' ? 'Collection name' : 'Table name'}
        bind:value={ideDbWatch.selectedObjectName}
        onkeydown={(e) => {
          if (e.key === 'Enter' && ideDbWatch.canCapture) {
            e.preventDefault();
            void ideDbWatch.captureSnapshot(ide.target.root, ideDbWatch.baselineSnapshot ? 'diff' : 'baseline');
          }
        }}
        data-testid="ide-dbw-object-input"
      />

      <input
        class="input dbw-query-input mono"
        type="text"
        dir="ltr"
        aria-label="Optional filter or read-only query"
        placeholder="Optional WHERE / custom read-only query…"
        bind:value={ideDbWatch.customQuery}
        onkeydown={(e) => {
          if (e.key === 'Enter' && ideDbWatch.canCapture) {
            e.preventDefault();
            void ideDbWatch.captureSnapshot(ide.target.root, ideDbWatch.baselineSnapshot ? 'diff' : 'baseline');
          }
        }}
        data-testid="ide-dbw-query-input"
      />
    </div>

    <div class="dbw-actions">
      <button
        type="button"
        class="btn small ghost"
        disabled={!ideDbWatch.canCapture || ideDbWatch.snapshotLoading}
        title="Capture current rows as the comparison baseline"
        onclick={() => void ideDbWatch.captureSnapshot(ide.target.root, 'baseline')}
        data-testid="ide-dbw-capture-baseline"
      >
        <Icon name="target" size={12} />
        <span>Capture baseline</span>
      </button>

      <button
        type="button"
        class="btn small primary"
        disabled={!ideDbWatch.canCapture || ideDbWatch.snapshotLoading}
        title="Fetch latest rows and compare against baseline"
        onclick={() => void ideDbWatch.captureSnapshot(ide.target.root, ideDbWatch.baselineSnapshot ? 'diff' : 'baseline')}
        data-testid="ide-dbw-refresh-diff"
      >
        <Icon name="play" size={12} />
        <span>{ideDbWatch.snapshotLoading ? 'Querying…' : ideDbWatch.baselineSnapshot ? 'Refresh & diff' : 'Query'}</span>
      </button>

      <div class="dbw-toggle" title="Automatically re-query and diff when the debugger pauses or a test run finishes">
        <Switch
          checked={ideDbWatch.autoRefreshOnExec}
          onchange={(next) => (ideDbWatch.autoRefreshOnExec = next)}
          label="Auto-refresh on Debug step & Run finish"
        />
        <span class="dbw-toggle-text">Auto-diff</span>
      </div>

      <div class="dbw-toggle" title="Inject TESTCONTAINERS_RYUK_DISABLED=true on Run & Debug so Docker containers remain inspectable after tests exit">
        <Switch
          checked={ide.keepTestcontainersAlive}
          onchange={(next) => (ide.keepTestcontainersAlive = next)}
          label="Keep Testcontainers on exit"
        />
        <span class="dbw-toggle-text">Keep containers</span>
      </div>

      <div class="dbw-toggle" title="Poll every 3 seconds while this panel is visible">
        <Switch
          checked={ideDbWatch.liveWatch}
          onchange={(next) => (ideDbWatch.liveWatch = next)}
          label="Live watch every 3 seconds"
        />
        <span class="dbw-toggle-text">Live (3s)</span>
      </div>

      {#if selectedTarget?.target_kind === 'saved'}
        <button
          type="button"
          class="btn small ghost"
          title="Open saved connection in Database Explorer"
          onclick={() => router.go('database')}
          data-testid="ide-dbw-open-explorer"
        >
          <Icon name="external" size={12} />
          <span>Open in DB Explorer</span>
        </button>
      {/if}
    </div>
  </header>

  {#if ideDbWatch.detectedObjects.length > 0}
    <div class="dbw-chips-bar" role="region" aria-label="Detected database objects in code" data-testid="ide-dbw-detected-bar">
      <span class="dbw-chips-label">Detected in code:</span>
      {#each ideDbWatch.detectedObjects as obj (`${obj.engine}:${obj.object_name}:${obj.source_file}:${obj.line}`)}
        {@const active = ideDbWatch.selectedEngine === obj.engine && ideDbWatch.selectedObjectName === obj.object_name}
        <div class="dbw-chip" class:is-active={active}>
          <button
            type="button"
            class="dbw-chip-select"
            aria-pressed={active}
            title={`${obj.engine} ${obj.kind} · ${obj.operations.join(', ') || 'read'}`}
            onclick={() => ideDbWatch.selectDetectedObject(obj)}
          >
            <Badge tone={active ? 'accent' : 'neutral'} variant="outline" label={obj.engine} />
            <span class="mono">{obj.object_name}</span>
          </button>
          <button
            type="button"
            class="dbw-chip-jump mono"
            aria-label="Jump to {ideBasename(obj.source_file)} line {obj.line}"
            title="Open {obj.source_file}:{obj.line} in editor"
            onclick={() => void ide.openFile(obj.source_file, { line: obj.line, preview: false })}
          >
            {ideBasename(obj.source_file)}:{obj.line}
          </button>
        </div>
      {/each}
    </div>
  {/if}

  {#if resp}
    <div class="dbw-subbar">
      <div class="segmented dbw-seg" role="group" aria-label="Snapshot view mode">
        <button
          type="button"
          class:active={ideDbWatch.viewMode === 'changes'}
          aria-pressed={ideDbWatch.viewMode === 'changes'}
          onclick={() => (ideDbWatch.viewMode = 'changes')}
          data-testid="ide-dbw-mode-changes"
        >
          Changes (Before → After)
        </button>
        <button
          type="button"
          class:active={ideDbWatch.viewMode === 'rows'}
          aria-pressed={ideDbWatch.viewMode === 'rows'}
          onclick={() => (ideDbWatch.viewMode = 'rows')}
          data-testid="ide-dbw-mode-rows"
        >
          All rows ({snapshotRows.length})
        </button>
      </div>

      {#if diff}
        <div class="dbw-diff-badges" data-testid="ide-dbw-diff-summary">
          <Badge tone="ok" label={`+${diff.added_count} added`} />
          <Badge tone="warn" label={`~${diff.modified_count} modified`} />
          <Badge tone="bad" label={`-${diff.removed_count} removed`} />
          <Badge tone="neutral" variant="outline" label={`${diff.unchanged_count} unchanged`} />
          {#if diff.added_count + diff.modified_count + diff.removed_count > 0}
            <button
              type="button"
              class="btn small ghost"
              title="Accept current snapshot as the new baseline"
              onclick={() => ideDbWatch.promoteLatestToBaseline()}
            >
              Accept as baseline
            </button>
          {/if}
        </div>
      {:else}
        <span class="dbw-meta-note">
          Baseline active ({snapshotRows.length} rows) · {resp.duration_ms}ms on {resp.target_label}
        </span>
      {/if}

      <span class="dbw-query-echo mono ellipsis" dir="ltr" title={resp.executed_query}>
        {resp.executed_query}
      </span>
    </div>
  {/if}

  <div class="dbw-body">
    <LoadState
      what="database snapshot"
      variant="panel"
      loading={(ideDbWatch.detectLoading && !resp) || (ideDbWatch.snapshotLoading && !resp)}
      error={ideDbWatch.snapshotError ?? ideDbWatch.detectError}
      empty={!resp}
      onretry={() => {
        if (ideDbWatch.snapshotError && ideDbWatch.canCapture) {
          void ideDbWatch.captureSnapshot(ide.target.root, ideDbWatch.baselineSnapshot ? 'diff' : 'baseline');
        } else {
          refreshTargets();
        }
      }}
    >
      {#snippet emptyView()}
        <EmptyState
          headingLevel={2}
          icon="db"
          title={ideDbWatch.allTargets.length === 0 ? 'No database targets connected yet' : 'Ready to watch database state'}
          body={ideDbWatch.allTargets.length === 0
            ? 'Start a component test using Testcontainers (enable Keep containers to inspect after exit) or add a saved connection in Connections.'
            : 'Pick a detected table, collection, or key prefix above and click Capture baseline before stepping through your code or running a test.'}
          actionLabel={ideDbWatch.canCapture ? 'Capture baseline' : 'Refresh containers'}
          actionIcon={ideDbWatch.canCapture ? 'target' : 'refresh'}
          actionKind="secondary"
          onaction={() => {
            if (ideDbWatch.canCapture) {
              void ideDbWatch.captureSnapshot(ide.target.root, 'baseline');
            } else {
              refreshTargets();
            }
          }}
        />
      {/snippet}

      {#if resp}
        {#if ideDbWatch.viewMode === 'changes' && diff}
          {#if diff.diffs.length === 0}
            <div class="dbw-calm-banner" data-testid="ide-dbw-no-changes">
              <Icon name="check" size={14} />
              <span>No row changes since baseline ({diff.unchanged_count} rows identical).</span>
            </div>
          {:else}
            <div class="dbw-table-scroll">
              <table class="dbw-table mono" dir="ltr" data-testid="ide-dbw-diff-table">
                <thead>
                  <tr>
                    <th class="dbw-status-col">State</th>
                    {#each snapshotCols as col (col)}
                      <th>{col}</th>
                    {/each}
                  </tr>
                </thead>
                <tbody>
                  {#each diff.diffs as rowDiff (`${rowDiff.kind}:${rowDiff.key}`)}
                    {@const displayCells = rowDiff.kind === 'removed' ? (rowDiff.previous_row ?? rowDiff.row) : rowDiff.row}
                    <tr class="dbw-row-{rowDiff.kind}" data-testid="ide-dbw-diff-row">
                      <td class="dbw-status-col">
                        <Badge
                          tone={rowDiff.kind === 'added' ? 'ok' : rowDiff.kind === 'removed' ? 'bad' : 'warn'}
                          label={rowDiff.kind === 'added' ? '+ Added' : rowDiff.kind === 'removed' ? '− Removed' : '~ Modified'}
                        />
                      </td>
                      {#each snapshotCols as col, colIdx (col)}
                        {@const fieldChange = rowDiff.kind === 'modified' ? changeForColumn(rowDiff.changes, col) : undefined}
                        <td class:dbw-cell-changed={Boolean(fieldChange)}>
                          {#if fieldChange}
                            <span class="dbw-val-before">{formatDbCellValue(fieldChange.before)}</span>
                            <span class="dbw-val-arrow" aria-hidden="true">→</span>
                            <span class="dbw-val-after">{formatDbCellValue(fieldChange.after)}</span>
                          {:else}
                            {formatDbCellValue(displayCells[colIdx])}
                          {/if}
                        </td>
                      {/each}
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {/if}
        {:else}
          {#if ideDbWatch.viewMode === 'changes' && !diff}
            <div class="dbw-calm-banner" data-testid="ide-dbw-baseline-ready">
              <Icon name="info" size={14} />
              <span>Baseline captured ({snapshotRows.length} rows). Step in the debugger, finish a test run, or click Refresh &amp; diff to inspect mutations.</span>
            </div>
          {/if}
          {#if snapshotRows.length === 0}
            <div class="dbw-calm-banner">
              <Icon name="db" size={14} />
              <span>Query returned 0 rows on {resp.target_label}.</span>
            </div>
          {:else}
            <div class="dbw-table-scroll">
              <table class="dbw-table mono" dir="ltr" data-testid="ide-dbw-rows-table">
                <thead>
                  <tr>
                    {#each snapshotCols as col (col)}
                      <th>{col}</th>
                    {/each}
                  </tr>
                </thead>
                <tbody>
                  {#each snapshotRows as row, rIdx (rIdx)}
                    <tr>
                      {#each snapshotCols as col, cIdx (col)}
                        <td>{formatDbCellValue(row[cIdx])}</td>
                      {/each}
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {/if}
        {/if}
      {/if}
    </LoadState>
  </div>
</div>

<style>
  .dbw-root {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
    min-inline-size: 0;
    background: var(--surface);
  }
  .dbw-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-4);
    padding: var(--sp-2) var(--sp-4);
    border-block-end: 1px solid var(--border);
    background: var(--surface);
    flex-wrap: wrap;
    flex-shrink: 0;
  }
  .dbw-target-group,
  .dbw-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    flex-wrap: wrap;
    min-inline-size: 0;
  }
  .dbw-field-label {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-weight: 500;
  }
  .dbw-select {
    min-inline-size: 180px;
    max-inline-size: 260px;
    block-size: 26px;
    padding-block: 0;
    font-size: var(--fs-xs);
  }
  .dbw-engine-select {
    inline-size: 112px;
    block-size: 26px;
    padding-block: 0;
    font-size: var(--fs-xs);
  }
  .dbw-input {
    inline-size: 150px;
    block-size: 26px;
    font-size: var(--fs-xs);
  }
  .dbw-query-input {
    inline-size: 210px;
    max-inline-size: 100%;
    block-size: 26px;
    font-size: var(--fs-xs);
  }
  .dbw-toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .dbw-toggle-text {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .dbw-chips-bar {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-4);
    border-block-end: 1px solid var(--separator);
    background: var(--surface-2);
    overflow-x: auto;
    flex-shrink: 0;
  }
  .dbw-chips-label {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .dbw-chip {
    display: inline-flex;
    align-items: center;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    flex-shrink: 0;
  }
  .dbw-chip.is-active {
    border-color: var(--accent-line);
    background: var(--accent-faint);
  }
  .dbw-chip-select {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-1) var(--sp-3);
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .dbw-chip-select:hover {
    color: var(--accent-text);
  }
  .dbw-chip-jump {
    padding: var(--sp-1) var(--sp-3);
    border: 0;
    border-inline-start: 1px solid var(--separator);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .dbw-chip-jump:hover {
    color: var(--accent-text);
    background: var(--hover);
  }
  .dbw-subbar {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding: var(--sp-2) var(--sp-4);
    border-block-end: 1px solid var(--border);
    background: var(--surface);
    flex-wrap: wrap;
    flex-shrink: 0;
  }
  .dbw-seg button {
    font-size: var(--fs-xs);
    padding-inline: var(--sp-4);
  }
  .dbw-diff-badges {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }
  .dbw-meta-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .dbw-query-echo {
    margin-inline-start: auto;
    max-inline-size: 360px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .dbw-body {
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .dbw-calm-banner {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-4) var(--sp-5);
    border-block-end: 1px solid var(--separator);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-s);
    flex-shrink: 0;
  }
  .dbw-table-scroll {
    flex: 1;
    min-block-size: 0;
    overflow: auto;
  }
  .dbw-table {
    inline-size: 100%;
    border-collapse: collapse;
    font-size: var(--fs-xs);
    text-align: start;
  }
  .dbw-table th {
    position: sticky;
    inset-block-start: 0;
    z-index: var(--z-sticky);
    padding: var(--sp-2) var(--sp-4);
    background: var(--surface-2);
    border-block-end: 1px solid var(--border);
    color: var(--text-dim);
    font-weight: 600;
    white-space: nowrap;
  }
  .dbw-table td {
    padding: var(--sp-2) var(--sp-4);
    border-block-end: 1px solid var(--separator);
    color: var(--text);
    white-space: nowrap;
    max-inline-size: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dbw-status-col {
    inline-size: 1%;
    white-space: nowrap;
  }
  .dbw-row-added td {
    background: var(--success-soft);
  }
  .dbw-row-removed td {
    background: var(--danger-soft);
    color: var(--text-dim);
  }
  .dbw-cell-changed {
    background: var(--warning-soft);
  }
  .dbw-val-before {
    color: var(--danger);
    text-decoration: line-through;
  }
  .dbw-val-arrow {
    padding-inline: var(--sp-2);
    color: var(--text-dim);
  }
  .dbw-val-after {
    color: var(--success);
    font-weight: 600;
  }
</style>
