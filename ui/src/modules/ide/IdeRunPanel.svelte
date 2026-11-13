<script lang="ts">
  // Structured Test Runner & Streaming Output panel: left test tree with suite
  // grouping, status icons, durations, "Failed only" filter, and click-to-jump
  // to file:line; right ANSI-cleaned streaming output with tail follow & copy.
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import { formatSeconds } from '../../lib/metric-format';
  import { scrollBehavior } from '../../lib/motion';
  import type { BadgeTone } from '../../lib/status';
  import {
    ide,
    ideBasename,
    type IdeRunState,
    type IdeTestResult,
  } from './ideStore.svelte';

  const LOG_CAP = 500;

  let selectedRunId = $state<string | null>(null);
  let filterMode = $state<'all' | 'failed'>('all');
  let query = $state('');
  let collapsedSuites = $state<Record<string, boolean>>({});
  let selectedTestKey = $state<string | null>(null);
  let showAllOutput = $state(false);
  let logEl = $state<HTMLDivElement>();
  let stickTail = $state(true);

  // Follow the newest activeRun automatically when it starts.
  let lastActiveId = '';
  $effect(() => {
    const id = ide.activeRun?.id ?? '';
    if (id && id !== lastActiveId) {
      lastActiveId = id;
      selectedRunId = id;
      showAllOutput = false;
      stickTail = true;
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
    if (shownRun.status === 'running') {
      return { label: 'Running', tone: 'info', live: true, dot: true };
    }
    if (shownRun.status === 'passed') {
      return { label: 'Passed', tone: 'ok', live: false, dot: true };
    }
    if (shownRun.status === 'failed') {
      const code = shownRun.exit_code != null ? ` (exit ${shownRun.exit_code})` : '';
      return { label: `Failed${code}`, tone: 'bad', live: false, dot: true };
    }
    return { label: 'Stopped', tone: 'neutral', live: false, dot: false };
  });

  // eslint-disable-next-line no-control-regex
  const ANSI_RE = /\x1b\[[0-9;]*[a-zA-Z]|\x1b\][^\x07]*\x07/g;
  function stripAnsi(raw: string): string {
    return raw.replace(ANSI_RE, '');
  }

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
    for (const t of filteredTests) {
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

  const cleanLines = $derived((shownRun?.output ?? []).map((line) => stripAnsi(line)));
  const hiddenLineCount = $derived(
    !showAllOutput && cleanLines.length > LOG_CAP ? cleanLines.length - LOG_CAP : 0,
  );
  const visibleLines = $derived(
    hiddenLineCount > 0 ? cleanLines.slice(-LOG_CAP) : cleanLines,
  );

  // Auto-follow streaming output while near bottom.
  $effect(() => {
    void cleanLines.length;
    const el = logEl;
    if (!el || !stickTail) return;
    queueMicrotask(() => {
      el.scrollTop = el.scrollHeight;
    });
  });

  function onLogScroll(): void {
    const el = logEl;
    if (!el) return;
    stickTail = el.scrollHeight - el.scrollTop - el.clientHeight < 28;
  }

  function jumpToTail(): void {
    const el = logEl;
    if (!el) return;
    stickTail = true;
    el.scrollTo({ top: el.scrollHeight, behavior: scrollBehavior() });
  }

  function testKey(t: IdeTestResult, idx: number): string {
    return `${t.suite ?? ''}::${t.name}::${idx}`;
  }

  function onSelectTest(t: IdeTestResult, key: string): void {
    selectedTestKey = selectedTestKey === key ? null : key;
    if (t.file) {
      void ide.openFile(t.file, { line: t.line ?? 1 });
    }
  }

  function toggleSuite(suite: string): void {
    collapsedSuites = { ...collapsedSuites, [suite]: !collapsedSuites[suite] };
  }

  function copyOutput(): void {
    const text = cleanLines.join('\n').trim();
    if (text) void copyWithToast(text, 'run output');
  }

  function isErrorLine(line: string): boolean {
    return /^\s*(FAIL|---\s*FAIL:|Error:|panic:|E\s+|error\[)/i.test(line);
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

      <Badge
        tone={runBadge.tone}
        label={runBadge.label}
        dot={runBadge.dot}
        live={runBadge.live}
        status={shownRun.status}
      />
      <Badge tone="neutral" variant="outline" label={shownRun.runner} />

      {#if tests.length > 0}
        <span class="run-counts">
          <span class="run-cnt-ok">{passedCount} passed</span>
          {#if failedCount > 0}
            <span class="run-cnt-bad">· {failedCount} failed</span>
          {/if}
          {#if skippedCount > 0}
            <span class="dim">· {skippedCount} skipped</span>
          {/if}
        </span>
      {/if}

      {#if shownRun.duration_ms != null}
        <span class="run-dur dim">{formatSeconds(shownRun.duration_ms / 1000)}</span>
      {/if}

      <span class="grow"></span>

      {#if tests.length > 0}
        <div class="segmented" role="group" aria-label="Filter tests by status">
          <button
            type="button"
            aria-pressed={filterMode === 'all'}
            onclick={() => (filterMode = 'all')}
          >
            All ({tests.length})
          </button>
          <button
            type="button"
            aria-pressed={filterMode === 'failed'}
            onclick={() => (filterMode = 'failed')}
          >
            Failed ({failedCount})
          </button>
        </div>

        <div class="input-group run-filter">
          <Icon name="search" size={12} />
          <input
            type="search"
            dir="ltr"
            aria-label="Filter tests by name"
            placeholder="Filter tests…"
            bind:value={query}
          />
        </div>
      {/if}

      <button
        type="button"
        class="btn small"
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
      <button
        type="button"
        class="icon-btn"
        aria-label="Copy output"
        title="Copy output"
        disabled={cleanLines.length === 0}
        onclick={copyOutput}
      >
        <Icon name="copy" size={13} />
      </button>
    </header>

    <div class="run-split" class:no-tests={tests.length === 0}>
      {#if tests.length > 0}
        <section class="run-tree" aria-label="Structured test results">
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
              >
                Show all tests
              </button>
            </div>
          {:else}
            {#each suiteGroups as group (group.suite)}
              {@const collapsed = !!collapsedSuites[group.suite]}
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
                    {@const showMsg = !!t.message && (t.status === 'failed' || selectedTestKey === k)}
                    <div class="run-test-wrap">
                      <button
                        type="button"
                        class="run-test-row"
                        class:is-selected={selectedTestKey === k}
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
                        <span class="run-test-status" class:is-ok={t.status === 'passed'} class:is-bad={t.status === 'failed'}>
                          {t.status}
                        </span>
                        {#if t.file}
                          <span class="run-test-loc mono dim" dir="ltr" title={`${t.file}:${t.line ?? 1}`}>
                            {ideBasename(t.file)}{t.line ? `:${t.line}` : ''}
                          </span>
                        {/if}
                        {#if t.duration_ms != null}
                          <span class="run-test-dur dim">{formatSeconds(t.duration_ms / 1000)}</span>
                        {/if}
                      </button>
                      {#if showMsg && t.message}
                        <pre class="run-test-msg mono" dir="ltr">{stripAnsi(t.message)}</pre>
                      {/if}
                    </div>
                  {/each}
                {/if}
              </div>
            {/each}
          {/if}
        </section>
      {/if}

      <section class="run-log-pane" aria-label="Console output">
        <div class="run-log" dir="ltr" bind:this={logEl} onscroll={onLogScroll}>
          {#if cleanLines.length === 0}
            <p class="run-log-placeholder dim">
              {isRunning ? 'Waiting for process output…' : 'Process finished with no console output.'}
            </p>
          {:else}
            {#if hiddenLineCount > 0}
              <div class="run-cap-bar">
                <button type="button" class="btn small ghost" onclick={() => (showAllOutput = true)}>
                  Show all {cleanLines.length} lines ({hiddenLineCount} earlier hidden)
                </button>
              </div>
            {/if}
            {#each visibleLines as line, idx (`ln-${hiddenLineCount + idx}`)}
              <div class="run-line mono" class:is-err={isErrorLine(line)}>{line || '\u00a0'}</div>
            {/each}
          {/if}
        </div>
        {#if !stickTail && cleanLines.length > 0}
          <div class="run-tail-cta">
            <button type="button" class="btn small" onclick={jumpToTail}>
              <Icon name="arrowDown" size={12} />
              <span>Follow output</span>
            </button>
          </div>
        {/if}
      </section>
    </div>
  {/if}
</div>

<style>
  .run-root {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
    min-inline-size: 0;
    background: var(--surface);
  }
  .run-empty {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .run-bar {
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
  .run-picker {
    block-size: 24px;
    font-size: var(--fs-xs);
    max-inline-size: 240px;
  }
  .run-title {
    font-size: var(--fs-s);
    font-weight: 600;
    max-inline-size: 240px;
  }
  .run-counts {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .run-cnt-ok {
    color: var(--success);
    font-weight: 500;
  }
  .run-cnt-bad {
    color: var(--danger);
    font-weight: 500;
  }
  .run-dur {
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .run-filter {
    block-size: 24px;
    inline-size: 160px;
    font-size: var(--fs-xs);
  }
  .run-split {
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    display: grid;
    grid-template-columns: minmax(240px, 0.95fr) minmax(260px, 1.35fr);
  }
  .run-split.no-tests {
    grid-template-columns: 1fr;
  }
  .run-tree {
    min-block-size: 0;
    min-inline-size: 0;
    overflow-y: auto;
    border-inline-end: 1px solid var(--border);
  }
  .run-filter-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 20px 12px;
    font-size: var(--fs-xs);
  }
  .run-suite {
    border-block-end: 1px solid var(--separator);
  }
  .run-suite-head {
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
  .run-suite-head:hover {
    background: var(--hover);
  }
  .run-suite-name {
    flex: 1;
  }
  .run-test-wrap {
    border-block-start: 1px solid var(--separator);
  }
  .run-test-row {
    display: flex;
    align-items: center;
    gap: 6px;
    inline-size: 100%;
    padding-block: 4px;
    padding-inline: 12px;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
  }
  .run-test-row:hover {
    background: var(--hover);
  }
  .run-test-row.is-selected {
    background: var(--accent-soft);
  }
  .run-icon {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
  }
  .run-icon.is-ok,
  .run-test-status.is-ok {
    color: var(--success);
  }
  .run-icon.is-bad,
  .run-test-status.is-bad {
    color: var(--danger);
  }
  .run-test-name {
    flex: 1;
    font-size: var(--fs-xs);
  }
  .run-test-status {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .run-test-loc {
    font-size: var(--fs-xs);
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }
  .run-test-dur {
    font-size: var(--fs-xs);
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }
  .run-test-msg {
    margin: 0;
    padding-block: 6px;
    padding-inline: 28px 12px;
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .run-log-pane {
    position: relative;
    display: flex;
    flex-direction: column;
    min-block-size: 0;
    min-inline-size: 0;
    background: var(--term-bg);
  }
  .run-log {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
    padding: 8px 12px;
    background: var(--term-bg);
  }
  .run-cap-bar {
    padding-block-end: 6px;
  }
  .run-log-placeholder {
    margin: 0;
    font-size: var(--fs-xs);
  }
  .run-line {
    font-size: var(--fs-xs);
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--text);
  }
  .run-line.is-err {
    color: var(--danger);
  }
  .run-tail-cta {
    position: absolute;
    inset-block-end: 10px;
    inset-inline-end: 14px;
    z-index: var(--z-sticky);
  }
  @container (max-width: 720px) {
    .run-split {
      grid-template-columns: 1fr;
      overflow-y: auto;
    }
    .run-tree {
      border-inline-end: 0;
      border-block-end: 1px solid var(--border);
      max-block-size: 200px;
    }
  }
  @media (max-width: 1024px) {
    .run-split {
      grid-template-columns: 1fr;
      overflow-y: auto;
    }
    .run-tree {
      border-inline-end: 0;
      border-block-end: 1px solid var(--border);
      max-block-size: 200px;
    }
    .run-filter {
      inline-size: 130px;
    }
  }
</style>
