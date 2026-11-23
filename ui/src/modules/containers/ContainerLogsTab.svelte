<script lang="ts">
  // Container Logs tab: tail selector, timestamps toggle, stream filter,
  // in-buffer search, soft-wrap toggle, clipboard copy, and refresh.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { containersApi } from '../../lib/api/containers';
  import { loadErrorText } from '../../lib/loadError';
  import { copyWithToast } from '../../lib/clipboard';
  import { plural } from '../../lib/plural';
  import type { ContainerLogLine } from '../../lib/api/types/containers';
  import { filterLogLines, formatLogsForCopy, type LogStreamFilter } from './containers-util';

  interface Props {
    containerId: string;
    containerName: string;
  }
  let { containerId, containerName }: Props = $props();

  const TAILS = [100, 200, 500, 1000, 5000] as const;

  let tail = $state<number>(200);
  let timestamps = $state(true);
  let softWrap = $state(false);
  let streamFilter = $state<LogStreamFilter>('all');
  let findQuery = $state('');
  let lines = $state.raw<ContainerLogLine[]>([]);
  let truncated = $state(false);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let loadSeq = 0;
  let activeAbort: AbortController | null = null;

  const visibleLines = $derived(filterLogLines(lines, findQuery, streamFilter));

  async function loadLogs(): Promise<void> {
    activeAbort?.abort();
    const ac = new AbortController();
    activeAbort = ac;
    const seq = ++loadSeq;
    loading = true;
    error = null;
    try {
      const resp = await containersApi.logs(containerId, { tail, timestamps }, ac.signal);
      if (seq !== loadSeq || ac.signal.aborted) return;
      lines = resp.lines;
      truncated = resp.truncated;
    } catch (e) {
      if (seq !== loadSeq || ac.signal.aborted) return;
      error = loadErrorText(e);
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  $effect(() => {
    void containerId;
    void tail;
    void timestamps;
    untrack(() => void loadLogs());
    return () => {
      activeAbort?.abort();
      activeAbort = null;
    };
  });

  function copyVisible(): void {
    const text = formatLogsForCopy(visibleLines, timestamps);
    void copyWithToast(text, `${containerName} logs`);
  }
</script>

<div class="ct-logs">
  <div class="ct-logs-bar">
    <div class="ct-logs-find">
      <Icon name="search" size={12} />
      <input
        class="ct-logs-find-input"
        type="search"
        dir="ltr"
        aria-label="Find in logs"
        placeholder="Find in logs…"
        bind:value={findQuery}
      />
      {#if findQuery}
        <button
          type="button"
          class="icon-btn"
          aria-label="Clear log search"
          title="Clear log search"
          onclick={() => (findQuery = '')}
        >
          <Icon name="x" size={12} />
        </button>
      {/if}
    </div>

    <div class="segmented ct-stream-seg" role="group" aria-label="Filter log stream">
      <button
        type="button"
        class:active={streamFilter === 'all'}
        aria-pressed={streamFilter === 'all'}
        onclick={() => (streamFilter = 'all')}
      >
        All
      </button>
      <button
        type="button"
        class:active={streamFilter === 'stdout'}
        aria-pressed={streamFilter === 'stdout'}
        onclick={() => (streamFilter = 'stdout')}
      >
        stdout
      </button>
      <button
        type="button"
        class:active={streamFilter === 'stderr'}
        aria-pressed={streamFilter === 'stderr'}
        onclick={() => (streamFilter = 'stderr')}
      >
        stderr
      </button>
    </div>

    <select class="input ct-tail-select" aria-label="Tail line limit" bind:value={tail}>
      {#each TAILS as n (n)}
        <option value={n}>Last {n}</option>
      {/each}
    </select>

    <button
      type="button"
      class="icon-btn"
      class:ct-toggle-on={timestamps}
      aria-pressed={timestamps}
      aria-label={timestamps ? 'Hide timestamps' : 'Show timestamps'}
      title={timestamps ? 'Hide timestamps' : 'Show timestamps'}
      onclick={() => (timestamps = !timestamps)}
    >
      <Icon name="clock" size={13} />
    </button>

    <button
      type="button"
      class="icon-btn"
      class:ct-toggle-on={softWrap}
      aria-pressed={softWrap}
      aria-label={softWrap ? 'Disable soft wrap' : 'Enable soft wrap'}
      title={softWrap ? 'Disable soft wrap' : 'Enable soft wrap'}
      onclick={() => (softWrap = !softWrap)}
    >
      <Icon name="wrap" size={13} />
    </button>

    <button
      type="button"
      class="icon-btn"
      disabled={visibleLines.length === 0}
      aria-label="Copy visible logs"
      title="Copy visible logs"
      onclick={copyVisible}
    >
      <Icon name="copy" size={13} />
    </button>

    <button
      type="button"
      class="icon-btn"
      disabled={loading}
      aria-label="Refresh logs"
      title="Refresh logs"
      onclick={() => void loadLogs()}
    >
      <Icon name="refresh" size={13} />
    </button>
  </div>

  <div class="ct-logs-body">
    <LoadState
      what="container logs"
      {loading}
      {error}
      empty={lines.length === 0}
      onretry={() => void loadLogs()}
    >
      {#snippet emptyView()}
        <EmptyState
          icon="terminal"
          title="No log output yet"
          body="This container hasn’t written any log lines for the selected tail window."
          actionLabel="Refresh logs"
          actionIcon="refresh"
          actionKind="secondary"
          onaction={() => void loadLogs()}
        />
      {/snippet}

      {#if visibleLines.length === 0}
        <EmptyState
          icon="search"
          title="No matching log lines"
          body="Adjust the search text or stream filter to see log output."
          actionLabel="Clear filters"
          actionKind="secondary"
          onaction={() => {
            findQuery = '';
            streamFilter = 'all';
          }}
        />
      {:else}
        <div class="ct-logs-scroller" class:wrap={softWrap} dir="ltr">
          {#each visibleLines as line, idx (idx)}
            <div class="ct-log-line" class:stderr={line.stream === 'stderr'}>
              {#if timestamps && line.timestamp}
                <span class="ct-log-ts">{line.timestamp}</span>
              {/if}
              <span class="ct-log-text">{line.text}</span>
            </div>
          {/each}
        </div>
      {/if}
    </LoadState>
  </div>

  <div class="ct-logs-foot">
    <span>{plural(visibleLines.length, 'line')}</span>
    {#if visibleLines.length !== lines.length}
      <span>of {lines.length}</span>
    {/if}
    {#if truncated}
      <span class="ct-logs-trunc">Output capped to response limit</span>
    {/if}
  </div>
</div>

<style>
  .ct-logs {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-block-size: 0;
  }
  .ct-logs-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .ct-logs-find {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1 1 160px;
    min-inline-size: 140px;
    block-size: 26px;
    padding-inline: 8px 4px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .ct-logs-find:focus-within {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .ct-logs-find-input {
    flex: 1;
    min-inline-size: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    font-family: var(--font-mono);
    outline: none;
  }
  .ct-logs-find-input::placeholder {
    color: var(--text-dim);
    font-family: var(--font-ui);
  }
  .ct-stream-seg {
    flex-shrink: 0;
  }
  .ct-tail-select {
    block-size: 26px;
    font-size: var(--fs-s);
    inline-size: auto;
  }
  .ct-toggle-on {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .ct-logs-body {
    flex: 1;
    min-block-size: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .ct-logs-scroller {
    flex: 1;
    min-block-size: 0;
    overflow: auto;
    padding: 10px 12px;
    background: var(--term-bg);
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    line-height: 1.5;
  }
  .ct-log-line {
    display: flex;
    gap: 10px;
    white-space: pre;
    color: var(--text);
  }
  .ct-logs-scroller.wrap .ct-log-line {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .ct-log-line.stderr .ct-log-text {
    color: var(--warning);
  }
  .ct-log-ts {
    color: var(--text-dim);
    flex-shrink: 0;
    user-select: none;
  }
  .ct-log-text {
    min-inline-size: 0;
  }
  .ct-logs-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-block-start: 1px solid var(--border);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  .ct-logs-trunc {
    color: var(--warning);
  }
</style>
