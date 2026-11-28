<script lang="ts">
  // Structured Test Runner & Streaming Output panel: the tests are a list pane
  // inside the tool window (a standalone ListPane: suites are tree nodes, a
  // failed test keeps Debug and Fix with Agent and its message), beside
  // `IdeConsoleView` (structured JSON logs, file:line links, wrap, filter). A
  // phone pushes: the tests fill the tool, a tapped test shows the output with
  // "‹ Tests".
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import PhoneBack from '../../lib/components/side/PhoneBack.svelte';
  import SideRow from '../../lib/components/side/SideRow.svelte';
  import { LIST_PANE, loadPaneW, paneKey, paneStorage } from '../../lib/components/side/paneKeys';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { router } from '../../lib/router.svelte';
  import { formatSeconds } from '../../lib/metric-format';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import type { BadgeTone } from '../../lib/status';
  import {
    ide,
    ideBasename,
    type IdeRunState,
    type IdeTestResult,
  } from './ideStore.svelte';
  import { ideAgent } from './store/ideAgentController.svelte';
  import IdeConsoleView, { type ConsoleLineInput } from './debug/IdeConsoleView.svelte';
  import { stripAnsiCodes } from './debug/ideLogParser';
  import {
    browserPresetForDevice,
    buildBrowserPreviewHash,
    buildDevBrowserRoute,
    detectDevServerUrls,
  } from './devices/ideDevServerDetect';
  import { ideDevices } from './devices/ideDevicesState.svelte';
  import { planFailedTestDebug, type FailedTestDebugPlan } from './debug/ideFailedTestDebug';
  import { runOutputSeqBase } from './store/ideDebugSnapshotSync';

  let selectedRunId = $state<string | null>(null);
  let filterMode = $state<'all' | 'failed'>('all');
  let query = $state('');
  let collapsedSuites = $state<Record<string, boolean>>({});
  let selectedTestKey = $state<string | null>(null);
  let debuggingTestKey = $state<string | null>(null);
  let showTestsPane = $state(true);
  // The shared list range and key; the old key is read once.
  let treeW = $state(loadPaneW(paneStorage(), 'ide', 'runTree', LIST_PANE, { w: 'otto.ide.runTreeW' }, 320));
  const phone = $derived(viewport.isPhone);
  let phoneOpen = $state(false);

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

  const detectedServers = $derived(detectDevServerUrls(shownRun?.output ?? []));
  const httpServers = $derived(
    detectedServers.filter((s) => s.scheme === 'http' || s.scheme === 'https'),
  );

  function openServerOnSimulator(url: string): void {
    const dev = ideDevices.bootedDevices[0] ?? ideDevices.selectedDevice;
    if (dev && dev.state === 'booted') {
      void ideDevices.openUrl(dev, url);
    } else {
      ide.openBottomTab('devices');
    }
  }

  function openServerInBrowser(url: string): void {
    const dev = ideDevices.selectedDevice;
    const target =
      dev && dev.state === 'booted'
        ? (`simulator:${dev.id}` as const)
        : browserPresetForDevice(dev);
    window.location.hash = buildBrowserPreviewHash({ url, device: target });
  }

  function openDevUrlInBrowser(e: MouseEvent): void {
    if (httpServers.length === 0) return;
    if (httpServers.length === 1) {
      const route = buildDevBrowserRoute(httpServers[0].url);
      if (route) router.go(route);
      return;
    }
    ctxMenu.show(
      e,
      httpServers.map((s) => ({
        label: s.url,
        icon: 'globe',
        action: () => {
          const route = buildDevBrowserRoute(s.url);
          if (route) router.go(route);
        },
      })),
    );
  }
  // Line `i` is sequence `base + i` (the daemon counts every line ever written),
  // so Clear and row keys survive the 2,000-line window sliding.
  const consoleLines = $derived.by((): readonly ConsoleLineInput[] => {
    if (!shownRun) return [];
    const base = runOutputSeqBase(shownRun);
    return shownRun.output.map((text, i) => ({ text, seq: base + i }));
  });

  function testKey(t: IdeTestResult, idx: number): string {
    return `${t.suite ?? ''}::${t.name}::${idx}`;
  }

  function onSelectTest(t: IdeTestResult, key: string): void {
    if (phone) {
      selectedTestKey = key;
      phoneOpen = true;
      return;
    }
    selectedTestKey = selectedTestKey === key ? null : key;
    if (t.file) void ide.openFile(t.file, { line: t.line ?? 1 });
  }

  function toggleSuite(suite: string): void {
    collapsedSuites = { ...collapsedSuites, [suite]: !collapsedSuites[suite] };
  }

  async function debugFailedTest(plan: FailedTestDebugPlan, key: string): Promise<void> {
    if (debuggingTestKey !== null || ide.debugLoading) return;
    debuggingTestKey = key;
    try {
      if (plan.kind === 'gutter') {
        ide.runFromGutter(plan.item, 'debug');
      } else {
        await ide.startDebug({ language: plan.language, launch: plan.launch });
      }
    } finally {
      debuggingTestKey = null;
    }
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
      {/if}

      <button
        type="button"
        class="btn small run-compact-btn"
        disabled={!ide.hasTarget || isRunning}
        title={!ide.hasTarget ? 'Select a workspace target to run' : isRunning ? 'A run is currently in progress' : 'Re-run'}
        onclick={() => void ide.rerunRun(shownRun.id)}
        data-testid="ide-run-rerun"
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

    {#if detectedServers.length > 0}
      {@const primarySrv = detectedServers[0]}
      <div class="run-devsrv-bar" role="region" aria-label="Detected local dev server" data-testid="ide-run-devserver-bar">
        <Icon name="globe" size={12} />
        <span class="run-devsrv-url mono ellipsis" dir="ltr">{primarySrv.url}</span>
        {#if httpServers.length > 0}
          <button
            type="button"
            class="btn small ghost run-compact-btn"
            aria-haspopup={httpServers.length > 1 ? 'menu' : undefined}
            title={httpServers.length === 1
              ? `Open ${httpServers[0].url} in Otto Browser`
              : `Open a detected local server (${httpServers.length}) in Otto Browser`}
            onclick={openDevUrlInBrowser}
            data-testid="ide-run-open-browser"
          >
            <Icon name="globe" size={12} />
            <span>Open in Browser</span>
          </button>
        {/if}
        <button
          type="button"
          class="btn small ghost run-compact-btn"
          onclick={() => openServerOnSimulator(primarySrv.url)}
          title={ideDevices.bootedDevices[0]
            ? `Open ${primarySrv.url} on ${ideDevices.bootedDevices[0].name}`
            : 'Open Mobile Devices tool window'}
          data-testid="ide-run-open-sim"
        >
          <Icon name="phone" size={12} />
          <span>{ideDevices.bootedDevices[0] ? `Open on ${ideDevices.bootedDevices[0].name}` : 'Open on Simulator'}</span>
        </button>
        <button
          type="button"
          class="btn small ghost run-compact-btn"
          onclick={() => openServerInBrowser(primarySrv.url)}
          title={`Preview ${primarySrv.url} in Otto Browser mobile emulator`}
          data-testid="ide-run-open-browser-emu"
        >
          <Icon name="compass" size={12} />
          <span>Browser Emulator</span>
        </button>
      </div>
    {/if}

    <div class="run-flex">
      {#if tests.length > 0 && showTestsPane}
        <div class="run-tree" class:phone style:inline-size={phone ? undefined : `${treeW}px`} hidden={phone && phoneOpen}>
          <ListPane name="Tests" count={tests.length}>
            {#snippet searchRow()}
              <div class="input-group">
                <Icon name="search" size={12} />
                <input type="search" dir="ltr" aria-label="Filter tests by name" placeholder="Filter tests…" bind:value={query} />
              </div>
            {/snippet}
            {#snippet filterRow()}
              <div class="segmented" role="group" aria-label="Filter tests by status">
                <button type="button" aria-pressed={filterMode === 'all'} onclick={() => (filterMode = 'all')}>All</button>
                <button type="button" aria-pressed={filterMode === 'failed'} onclick={() => (filterMode = 'failed')}>Failed ({failedCount})</button>
              </div>
            {/snippet}
            {#if filteredTests.length === 0}
              <div class="run-filter-empty">
                <p class="dim">No tests match.</p>
                <button
                  type="button"
                  class="btn small ghost"
                  onclick={() => {
                    filterMode = 'all';
                    query = '';
                  }}
                >Clear filter</button>
              </div>
            {:else}
              <div role="tree" aria-label="Structured test results">
                {#each suiteGroups as group (group.suite)}
                  {@const collapsed = Boolean(collapsedSuites[group.suite])}
                  <SideRow
                    tree
                    title={group.suite}
                    tip={group.suite}
                    expanded={!collapsed}
                    ontoggle={() => toggleSuite(group.suite)}
                    onclick={() => toggleSuite(group.suite)}
                  >
                    {#snippet trailing()}{group.failed > 0 ? `${group.failed} failed` : `${group.passed}/${group.items.length}`}{/snippet}
                  </SideRow>
                  {#if !collapsed}
                    {#each group.items as t, idx (testKey(t, idx))}
                      {@const k = testKey(t, idx)}
                      {@const showMsg = Boolean(t.message) && (t.status === 'failed' || selectedTestKey === k)}
                      <div class="side-row" class:two-line={!!t.file} class:selected={selectedTestKey === k} style:--side-depth={1}>
                        <button
                          type="button"
                          class="side-row-main"
                          data-side-row
                          role="treeitem"
                          aria-level={2}
                          aria-selected={selectedTestKey === k}
                          title={t.file ? `${t.name} — ${t.file}:${t.line ?? 1}` : t.name}
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
                          <span class="side-row-text">
                            <span class="side-row-title mono" dir="ltr">{t.name}</span>
                            {#if t.file}<span class="side-row-meta mono" dir="ltr">{ideBasename(t.file)}{t.line ? `:${t.line}` : ''}</span>{/if}
                          </span>
                          <span class="side-row-trailing">
                            <span class="run-test-status" class:is-ok={t.status === 'passed'} class:is-bad={t.status === 'failed'}>{t.status}</span>
                            {#if t.duration_ms != null}<span>{formatSeconds(t.duration_ms / 1000)}</span>{/if}
                          </span>
                        </button>
                        {#if t.status === 'failed'}
                          {@const debugPlan = ide.hasTarget && !ide.collabIsGuest
                            ? planFailedTestDebug({
                                test: t,
                                runner: shownRun.runner,
                                targetRoot: ide.target.root,
                                activePath: ide.activePath,
                                activeLanguage: ide.activeTab?.language,
                                gutterByPath: ide.gutterByPath,
                                resolvePath: (p) => ide.resolvePath(p),
                              })
                            : null}
                          {#if debugPlan}
                            <button
                              type="button"
                              class="icon-btn"
                              aria-label="Debug {t.name}"
                              title={ide.debugLoading || debuggingTestKey !== null ? 'Starting debug session…' : `Debug ${t.name}`}
                              disabled={ide.debugLoading || debuggingTestKey !== null}
                              onclick={() => void debugFailedTest(debugPlan, k)}
                              data-testid="ide-test-debug-btn"
                            >
                              {#if debuggingTestKey === k}<span class="spinner" style="--spinner-size: 12px" aria-hidden="true"></span>{:else}<Icon name="bug" size={14} />{/if}
                            </button>
                          {/if}
                          <button
                            type="button"
                            class="icon-btn run-fix-btn"
                            aria-label="Fix {t.name} with Agent"
                            title="Fix with Agent"
                            onclick={() =>
                              ideAgent.attachTestFailure(
                                t,
                                (shownRun?.output ?? []).map((line) => stripAnsiCodes(line)),
                                { openPanel: true },
                              )}
                            data-testid="ide-test-fix-agent"
                          >
                            <Icon name="assistant" size={14} />
                          </button>
                        {/if}
                      </div>
                      {#if showMsg && t.message}
                        <pre class="run-test-msg mono" dir="ltr">{stripAnsiCodes(t.message)}</pre>
                      {/if}
                    {/each}
                  {/if}
                {/each}
              </div>
            {/if}
          </ListPane>
        </div>

        {#if !phone}
          <PaneDivider bind:width={treeW} storageKey={paneKey('ide', 'runTree', 'w')} min={LIST_PANE.min} max={LIST_PANE.max} defaultWidth={320} label="Resize tests" />
        {/if}
      {/if}

      <section class="run-log-pane" aria-label="Console output" hidden={phone && tests.length > 0 && showTestsPane && !phoneOpen}>
        {#if phone && phoneOpen}
          <header class="run-log-head">
            <PhoneBack label="Tests" onclick={() => (phoneOpen = false)} testid="ide-run-tests-back" />
          </header>
        {/if}
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
  .run-root { display: flex; flex-direction: column; block-size: 100%; min-block-size: 0; min-inline-size: 0; background: var(--surface); }
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
  .run-compact-btn { block-size: 22px; padding-block: 0; flex-shrink: 0; }
  .run-devsrv-bar {
    display: flex; align-items: center; gap: 6px; padding-block: 4px; padding-inline: 8px;
    border-block-end: 1px solid var(--border); background: var(--surface-2); flex-shrink: 0;
    overflow-x: auto; font-size: var(--fs-xs); min-inline-size: 0;
  }
  .run-devsrv-url { color: var(--accent-text); font-weight: 500; flex-shrink: 1; min-inline-size: 0; max-inline-size: 240px; }
  .run-flex { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; overflow: hidden; }
  .run-tree { flex-shrink: 0; display: flex; flex-direction: column; min-block-size: 0; min-inline-size: 0; }
  .run-tree.phone { flex: 1; }
  .run-tree[hidden], .run-log-pane[hidden] { display: none; }
  .run-filter-empty { display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 20px 12px; font-size: var(--fs-s); }
  .run-fix-btn { color: var(--accent-text); }
  .run-icon { display: inline-flex; align-items: center; flex-shrink: 0; }
  .run-icon.is-ok, .run-test-status.is-ok { color: var(--success); }
  .run-icon.is-bad, .run-test-status.is-bad { color: var(--danger); }
  .run-test-msg { margin: 0 0 2px; padding-block: 6px; padding-inline: 40px 12px; border-radius: var(--radius-s); background: var(--danger-soft); color: var(--danger); font-size: var(--fs-xs); white-space: pre-wrap; overflow-wrap: anywhere; }
  .run-log-pane { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; flex-direction: column; }
  .run-log-head { display: flex; align-items: center; flex-shrink: 0; padding-inline: 4px; border-block-end: 1px solid var(--border); }
</style>
