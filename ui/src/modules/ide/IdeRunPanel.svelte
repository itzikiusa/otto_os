<script lang="ts">
  // Structured Test Runner & Streaming Output panel: resizable left test tree
  // with suite grouping + Fix with Agent, and right `IdeConsoleView` with
  // structured JSON log formatting, clickable file:line links, wrap, and filter.
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import { formatSeconds } from '../../lib/metric-format';
  import { loadPaneWidth } from '../../lib/paneResizer';
  import type { BadgeTone } from '../../lib/status';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import {
    ide,
    ideBasename,
    type IdeRunState,
    type IdeTestResult,
  } from './ideStore.svelte';
  import { ideAgent } from './store/ideAgentController.svelte';
  import IdeConsoleView, { type ConsoleLineInput } from './debug/IdeConsoleView.svelte';
  import { stripAnsiCodes } from './debug/ideLogParser';

  const LS_RUN_TREE_W = 'otto.ide.runTreeW';

  let selectedRunId = $state<string | null>(null);
  let filterMode = $state<'all' | 'failed'>('all');
  let query = $state('');
  let collapsedSuites = $state<Record<string, boolean>>({});
  let selectedTestKey = $state<string | null>(null);
  let showTestsPane = $state(true);
  let treeW = $state(loadPaneWidth(LS_RUN_TREE_W, 320, 200, 620));

  let lastActiveId = '';
  $effect(() => {
    const id = ide.activeRun?.id ?? '';
    if (id && id !== lastActiveId) {
      lastActiveId = id;
      selectedRunId = id;
    }
  });

  const allRuns = $derived.by((): IdeRunState[] => {
    const cur = ide.activeRun;
    const hist = ide.runsHistory;
    if (!cur) return hist;
    return [cur, ...hist.filter((r) => r.id !== cur.id)];
  });

  const shownRun = $derived(
    allRuns.find((r) => r.id === selectedRunId) ?? ide.activeRun ?? allRuns[0] ?? null,
  );
  const isRunning = $derived(shownRun?.status === 'running');

  const runBadge = $derived.by((): { label: string; tone: BadgeTone; live: boolean; dot: boolean } => {
    if (!shownRun) return { label: 'Idle', tone: 'neutral', live: false, dot: false };
    if (shownRun.status === 'running') return { label: 'Running', tone: 'info', live: true, dot: true };
    if (shownRun.status === 'passed') return { label: 'Passed', tone: 'ok', live: false, dot: true };
    if (shownRun.status === 'failed') {
      const code = shownRun.exit_code != null ? ` (exit ${shownRun.exit_code})` : '';
      return { label: `Failed${code}`, tone: 'bad', live: false, dot: true };
    }
    return { label: 'Stopped', tone: 'neutral', live: false, dot: false };
  });

  const tests = $derived(shownRun?.tests ?? []);
  const passedCount = $derived(tests.filter((t) => t.status === 'passed').length);
  const failedCount = $derived(tests.filter((t) => t.status === 'failed').length);
  const skippedCount = $derived(tests.filter((t) => t.status === 'skipped').length);

  const filteredTests = $derived.by((): IdeTestResult[] => {
    const q = query.trim().toLowerCase();
    return tests.filter((t) => {
      if (filterMode === 'failed' && t.status !== 'failed') return false;
      if (!q) return true;
      return (
        t.name.toLowerCase().includes(q) ||
        (t.suite ?? '').toLowerCase().includes(q) ||
        (t.file ?? '').toLowerCase().includes(q)
      );
    });
  });

  interface SuiteGroup {
    suite: string;
    items: IdeTestResult[];
    failed: number;
    passed: number;
  }

  const suiteGroups = $derived.by((): SuiteGroup[] => {
    const map = new Map<string, IdeTestResult[]>();
    for (let i = 0; i < filteredTests.length; i++) {
      const t = filteredTests[i];
      const key = t.suite?.trim() || 'Test suite';
      const arr = map.get(key);
      if (arr) arr.push(t);
      else map.set(key, [t]);
    }
    const out: SuiteGroup[] = [];
    for (const [suite, items] of map.entries()) {
      out.push({
        suite,
        items,
        failed: items.filter((i) => i.status === 'failed').length,
        passed: items.filter((i) => i.status === 'passed').length,
      });
    }
    return out;
  });

  const cleanLines = $derived((shownRun?.output ?? []).map((line) => stripAnsiCodes(line)));
  const consoleLines = $derived<readonly ConsoleLineInput[]>(
    (shownRun?.output ?? []).map((text) => ({ text })),
  );

  function testKey(t: IdeTestResult, idx: number): string {
    return `${t.suite ?? ''}::${t.name}::${idx}`;
  }

  function onSelectTest(t: IdeTestResult, key: string): void {
    selectedTestKey = selectedTestKey === key ? null : key;
    if (t.file) void ide.openFile(t.file, { line: t.line ?? 1 });
  }

  function toggleSuite(suite: string): void {
    collapsedSuites = { ...collapsedSuites, [suite]: !collapsedSuites[suite] };
  }
</script>

<div class="run-root" data-testid="ide-run-panel">
  {#if !shownRun}
    <div class="run-empty">
      <EmptyState
        icon="play"
        title="No runs yet"
        body="Run a configuration from the toolbar or click the run icon in the gutter next to any main function or test."
        actionLabel={ide.hasTarget ? 'Run' : undefined}
        actionIcon="play"
        actionKind="secondary"
        onaction={ide.hasTarget ? () => void ide.startRun() : undefined}
        headingLevel={2}
      />
    </div>
  {:else}
    <header class="run-bar">
      {#if allRuns.length > 1}
        <select
          class="input run-picker"
          dir="ltr"
          aria-label="Select run"
          value={shownRun.id}
          onchange={(e) => (selectedRunId = (e.currentTarget as HTMLSelectElement).value)}
        >
          {#each allRuns as r (r.id)}
            <option value={r.id}>{r.title} ({r.status})</option>
          {/each}
        </select>
      {:else}
        <span class="run-title ellipsis" title={shownRun.title}>{shownRun.title}</span>
      {/if}

      <Badge tone={runBadge.tone} label={runBadge.label} dot={runBadge.dot} live={runBadge.live} status={shownRun.status} />
      <Badge tone="neutral" variant="outline" label={shownRun.runner} />
      {#if shownRun.coverage_summary || shownRun.coverage_pct != null}
        <Badge
          tone="info"
          variant="outline"
          label={shownRun.coverage_summary || `Coverage ${shownRun.coverage_pct?.toFixed(1)}%`}
        />
      {/if}

      {#if tests.length > 0}
        <span class="run-counts">
          <span class="run-cnt-ok">{passedCount} passed</span>
          {#if failedCount > 0}<span class="run-cnt-bad">· {failedCount} failed</span>{/if}
          {#if skippedCount > 0}<span class="dim">· {skippedCount} skipped</span>{/if}
        </span>
      {/if}

      {#if shownRun.duration_ms != null}
        <span class="run-dur dim">{formatSeconds(shownRun.duration_ms / 1000)}</span>
      {/if}

      <span class="grow"></span>

      {#if tests.length > 0}
        <button
          type="button"
          class="run-pane-toggle"
          class:is-on={showTestsPane}
          aria-pressed={showTestsPane}
          title={showTestsPane ? 'Hide test tree column' : 'Show test tree column'}
          onclick={() => (showTestsPane = !showTestsPane)}
        >Tests ({tests.length})</button>
        {#if showTestsPane}
          <div class="segmented run-seg" role="group" aria-label="Filter tests by status">
            <button type="button" aria-pressed={filterMode === 'all'} onclick={() => (filterMode = 'all')}>All</button>
            <button type="button" aria-pressed={filterMode === 'failed'} onclick={() => (filterMode = 'failed')}>Failed ({failedCount})</button>
          </div>
          <div class="input-group run-filter">
            <Icon name="search" size={12} />
            <input type="search" dir="ltr" aria-label="Filter tests by name" placeholder="Filter tests…" bind:value={query} />
          </div>
        {/if}
      {/if}

      <button
        type="button"
        class="btn small run-compact-btn"
        disabled={!ide.hasTarget || isRunning}
        title={!ide.hasTarget ? 'Select a workspace target to run' : isRunning ? 'A run is currently in progress' : 'Re-run'}
        onclick={() => void ide.startRun()}
      >
        <Icon name="refresh" size={12} />
        <span>Re-run</span>
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="Stop run"
        title="Stop run"
        disabled={!isRunning}
        onclick={() => void ide.stopRun(shownRun.id)}
      >
        <Icon name="stop" size={13} />
      </button>
    </header>

    <div class="run-flex">
      {#if tests.length > 0 && showTestsPane}
        <section
          class="run-tree"
          style:inline-size={!viewport.isPhone ? `${treeW}px` : undefined}
          aria-label="Structured test results"
        >
          {#if filteredTests.length === 0}
            <div class="run-filter-empty">
              <p class="dim">No matching tests.</p>
              <button
                type="button"
                class="btn small ghost"
                onclick={() => {
                  filterMode = 'all';
                  query = '';
                }}
              >Show all tests</button>
            </div>
          {:else}
            {#each suiteGroups as group (group.suite)}
              {@const collapsed = Boolean(collapsedSuites[group.suite])}
              <div class="run-suite">
                <button
                  type="button"
                  class="run-suite-head"
                  aria-expanded={!collapsed}
                  onclick={() => toggleSuite(group.suite)}
                >
                  <Icon name={collapsed ? 'chevronRight' : 'chevronDown'} size={12} />
                  <span class="run-suite-name ellipsis" dir="ltr" title={group.suite}>{group.suite}</span>
                  <Badge
                    tone={group.failed > 0 ? 'bad' : 'ok'}
                    variant="outline"
                    label={group.failed > 0 ? `${group.failed} failed` : `${group.passed}/${group.items.length}`}
                  />
                </button>
                {#if !collapsed}
                  {#each group.items as t, idx (testKey(t, idx))}
                    {@const k = testKey(t, idx)}
                    {@const showMsg = Boolean(t.message) && (t.status === 'failed' || selectedTestKey === k)}
                    <div class="run-test-wrap">
                      <div class="run-test-head" class:is-selected={selectedTestKey === k}>
                        <button
                          type="button"
                          class="run-test-row"
                          aria-pressed={selectedTestKey === k}
                          onclick={() => onSelectTest(t, k)}
                        >
                          {#if t.status === 'passed'}
                            <span class="run-icon is-ok" aria-hidden="true"><Icon name="check" size={12} /></span>
                          {:else if t.status === 'failed'}
                            <span class="run-icon is-bad" aria-hidden="true"><Icon name="x" size={12} /></span>
                          {:else if t.status === 'running'}
                            <span class="spinner" style="--spinner-size: 12px" aria-hidden="true"></span>
                          {:else}
                            <span class="run-icon dim" aria-hidden="true"><Icon name="dot" size={12} /></span>
                          {/if}
                          <span class="run-test-name mono ellipsis" dir="ltr" title={t.name}>{t.name}</span>
                          <span class="run-test-status" class:is-ok={t.status === 'passed'} class:is-bad={t.status === 'failed'}>{t.status}</span>
                          {#if t.file}
                            <span class="run-test-loc mono dim" dir="ltr" title={`${t.file}:${t.line ?? 1}`}>
                              {ideBasename(t.file)}{t.line ? `:${t.line}` : ''}
                            </span>
                          {/if}
                          {#if t.duration_ms != null}
                            <span class="run-test-dur dim">{formatSeconds(t.duration_ms / 1000)}</span>
                          {/if}
                        </button>
                        {#if t.status === 'failed'}
                          <button
                            type="button"
                            class="btn small ghost run-fix-btn"
                            aria-label="Fix with Agent"
                            title="Fix with Agent"
                            onclick={() => ideAgent.attachTestFailure(t, cleanLines, { openPanel: true })}
                            data-testid="ide-test-fix-agent"
                          >
                            <Icon name="assistant" size={12} />
                            <span>Fix</span>
                          </button>
                        {/if}
                      </div>
                      {#if showMsg && t.message}
                        <pre class="run-test-msg mono" dir="ltr">{stripAnsiCodes(t.message)}</pre>
                      {/if}
                    </div>
                  {/each}
                {/if}
              </div>
            {/each}
          {/if}
        </section>

        {#if !viewport.isPhone}
          <PaneDivider bind:width={treeW} storageKey={LS_RUN_TREE_W} min={200} max={620} defaultWidth={320} label="Resize test results tree" />
        {/if}
      {/if}

      <section class="run-log-pane" aria-label="Console output">
        <IdeConsoleView
          sessionKey={shownRun.id}
          lines={consoleLines}
          emptyText={isRunning ? 'Waiting for process output…' : 'Process finished with no console output.'}
        />
      </section>
    </div>
  {/if}
</div>

<style>
  .run-root { container-type: inline-size; display: flex; flex-direction: column; block-size: 100%; min-block-size: 0; min-inline-size: 0; background: var(--surface); }
  .run-empty { flex: 1; min-block-size: 0; overflow-y: auto; display: flex; align-items: center; justify-content: center; }
  .run-bar {
    display: flex; align-items: center; gap: 6px;
    block-size: 32px; min-block-size: 32px; max-block-size: 32px;
    padding-inline: 8px; border-block-end: 1px solid var(--border);
    background: var(--surface); flex-shrink: 0; overflow-x: auto; overflow-y: hidden;
  }
  .run-picker { block-size: 22px; font-size: var(--fs-xs); max-inline-size: 220px; flex-shrink: 0; }
  .run-title { font-size: var(--fs-s); font-weight: 600; max-inline-size: 220px; flex-shrink: 0; }
  .run-counts { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); font-variant-numeric: tabular-nums; white-space: nowrap; flex-shrink: 0; }
  .run-cnt-ok { color: var(--success); font-weight: 500; }
  .run-cnt-bad { color: var(--danger); font-weight: 500; }
  .run-dur { font-size: var(--fs-xs); font-variant-numeric: tabular-nums; white-space: nowrap; flex-shrink: 0; }
  .run-pane-toggle {
    display: inline-flex; align-items: center; block-size: 22px; padding-inline: 6px;
    border: 1px solid var(--border); border-radius: var(--radius-s);
    background: var(--surface-2); color: var(--text-dim); font-size: var(--fs-xs); white-space: nowrap; cursor: pointer; flex-shrink: 0;
  }
  .run-pane-toggle:hover { background: var(--hover); color: var(--text); }
  .run-pane-toggle.is-on { border-color: var(--accent-line); background: var(--accent-soft); color: var(--accent-text); }
  .run-seg { block-size: 22px; flex-shrink: 0; }
  .run-seg button { padding-block: 0; padding-inline: 8px; font-size: var(--fs-xs); }
  .run-filter { block-size: 22px; inline-size: 148px; font-size: var(--fs-xs); flex-shrink: 0; }
  .run-compact-btn { block-size: 22px; padding-block: 0; flex-shrink: 0; }
  .run-flex { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; overflow: hidden; }
  .run-tree { flex-shrink: 0; min-block-size: 0; min-inline-size: 200px; overflow-y: auto; border-inline-end: 1px solid var(--border); }
  .run-filter-empty { display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 20px 12px; font-size: var(--fs-xs); }
  .run-suite { border-block-end: 1px solid var(--separator); }
  .run-suite-head { display: flex; align-items: center; gap: 6px; inline-size: 100%; padding-block: 6px; padding-inline: 10px; border: 0; background: var(--surface-2); color: var(--text); font-size: var(--fs-xs); font-weight: 600; text-align: start; cursor: pointer; }
  .run-suite-head:hover { background: var(--hover); }
  .run-suite-name { flex: 1; }
  .run-test-wrap { border-block-start: 1px solid var(--separator); }
  .run-test-head { display: flex; align-items: center; gap: 4px; padding-inline-end: 8px; }
  .run-test-head:hover { background: var(--hover); }
  .run-test-head.is-selected { background: var(--accent-soft); }
  .run-test-row { display: flex; align-items: center; gap: 6px; flex: 1; min-inline-size: 0; padding-block: 4px; padding-inline: 12px 6px; border: 0; background: transparent; color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer; }
  .run-fix-btn { flex-shrink: 0; color: var(--accent-text); }
  .run-icon { display: inline-flex; align-items: center; flex-shrink: 0; }
  .run-icon.is-ok, .run-test-status.is-ok { color: var(--success); }
  .run-icon.is-bad, .run-test-status.is-bad { color: var(--danger); }
  .run-test-name { flex: 1; font-size: var(--fs-xs); }
  .run-test-status { font-size: var(--fs-xs); color: var(--text-dim); flex-shrink: 0; }
  .run-test-loc, .run-test-dur { font-size: var(--fs-xs); flex-shrink: 0; font-variant-numeric: tabular-nums; }
  .run-test-msg { margin: 0; padding-block: 6px; padding-inline: 28px 12px; background: var(--danger-soft); color: var(--danger); font-size: var(--fs-xs); white-space: pre-wrap; overflow-wrap: anywhere; }
  .run-log-pane { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; flex-direction: column; }
  @container (max-width: 720px) {
    .run-flex { flex-direction: column; overflow-y: auto; }
    .run-tree { inline-size: 100%; border-inline-end: 0; border-block-end: 1px solid var(--border); max-block-size: 200px; }
    .run-flex :global(.pane-divider) { display: none; }
  }
</style>
