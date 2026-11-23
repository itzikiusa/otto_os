<script lang="ts">
  // Live iOS Simulator / Android Emulator log stream viewer (`WS /ws/simulators/{id}/logs`).
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import { copyWithToast } from '../../../lib/clipboard';
  import { simulatorsApi } from '../../../lib/api/simulators';
  import type { SimDevice, SimLogLevel, SimLogServerMsg } from '../../../lib/api/types';
  import { ideAgent } from '../store/ideAgentController.svelte';

  interface Props {
    device: SimDevice;
  }

  let { device }: Props = $props();

  interface LogRow {
    seq: number;
    ts: string;
    level: SimLogLevel;
    tag: string;
    pid: number | null;
    message: string;
  }

  const MAX_ROWS = 1200;
  const LEVEL_RANK: Record<SimLogLevel, number> = {
    verbose: 0,
    unknown: 0,
    debug: 1,
    info: 2,
    warn: 3,
    error: 4,
    fatal: 5,
  };

  let rows = $state<LogRow[]>([]);
  let minLevel = $state<'all' | 'info' | 'warn' | 'error'>('all');
  let filter = $state('');
  let paused = $state(false);
  let connState = $state<'connecting' | 'live' | 'error' | 'closed'>('connecting');
  let connError = $state<string | null>(null);
  let seq = 0;
  let scrollEl = $state<HTMLDivElement | undefined>(undefined);
  let sock: WebSocket | null = null;
  let pendingRows: LogRow[] = [];
  let flushRaf: number | null = null;

  // Derive primitive identity + booted state so rotate/appearance/refresh updates
  // to the parent SimDevice object never wipe logs or reconnect the socket.
  const deviceId = $derived(device.id);
  const deviceBooted = $derived(device.state === 'booted');

  function flushPending(): void {
    flushRaf = null;
    if (pendingRows.length === 0) return;
    const batch = pendingRows;
    pendingRows = [];
    const combined = rows.length + batch.length;
    rows =
      combined > MAX_ROWS
        ? [...rows.slice(Math.max(0, combined - MAX_ROWS)), ...batch].slice(-MAX_ROWS)
        : [...rows, ...batch];
    if (scrollEl && scrollEl.scrollHeight - scrollEl.scrollTop - scrollEl.clientHeight < 90) {
      scrollEl.scrollTop = scrollEl.scrollHeight;
    }
  }

  function cancelFlush(): void {
    if (flushRaf !== null) {
      cancelAnimationFrame(flushRaf);
      flushRaf = null;
    }
    pendingRows = [];
  }

  function disconnect(): void {
    cancelFlush();
    if (sock) {
      sock.onopen = null;
      sock.onmessage = null;
      sock.onerror = null;
      sock.onclose = null;
      try {
        sock.close();
      } catch {
        /* ignore */
      }
      sock = null;
    }
  }

  $effect(() => {
    const id = deviceId;
    const booted = deviceBooted;
    disconnect();
    rows = [];
    connError = null;
    if (!booted) {
      connState = 'closed';
      return;
    }
    connState = 'connecting';
    const ws = simulatorsApi.openLogsSocket(id);
    sock = ws;
    ws.onopen = () => {
      connState = 'live';
      connError = null;
    };
    ws.onmessage = (ev) => {
      if (typeof ev.data !== 'string') return;
      let msg: SimLogServerMsg;
      try {
        msg = JSON.parse(ev.data) as SimLogServerMsg;
      } catch {
        return;
      }
      if (msg.type === 'log') {
        if (paused) return;
        pendingRows.push({
          seq: ++seq,
          ts: msg.ts,
          level: msg.level,
          tag: msg.tag ?? '',
          pid: msg.pid ?? null,
          message: msg.message,
        });
        if (flushRaf === null) {
          flushRaf = requestAnimationFrame(flushPending);
        }
      } else if (msg.type === 'error') {
        connState = 'error';
        connError = msg.message;
      }
    };
    ws.onerror = () => {
      connState = 'error';
      connError = 'Log stream disconnected';
    };
    ws.onclose = () => {
      if (connState !== 'error') connState = 'closed';
    };
    return () => disconnect();
  });

  const filteredRows = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    const minRank =
      minLevel === 'error' ? 4 : minLevel === 'warn' ? 3 : minLevel === 'info' ? 2 : 0;
    return rows.filter((r) => {
      if (LEVEL_RANK[r.level] < minRank) return false;
      if (!q) return true;
      return r.message.toLowerCase().includes(q) || r.tag.toLowerCase().includes(q);
    });
  });

  function copyVisibleLogs(): void {
    const text = filteredRows
      .map((r) => `${r.ts} [${r.level.toUpperCase()}]${r.tag ? ` ${r.tag}:` : ''} ${r.message}`)
      .join('\n');
    void copyWithToast(text, 'Device logs');
  }

  function askAgentAboutRow(row: LogRow): void {
    const idx = rows.findIndex((r) => r.seq === row.seq);
    const contextLines = rows
      .slice(Math.max(0, idx - 6), Math.min(rows.length, idx + 8))
      .map((r) => `${r.ts} [${r.level.toUpperCase()}] ${r.tag ? `${r.tag}: ` : ''}${r.message}`)
      .join('\n');
    const key = `simlog:${device.id}:${row.seq}`;
    ideAgent.contexts = [
      ...ideAgent.contexts.filter((c) => c.key !== key),
      {
        id: key,
        key,
        kind: 'run_output',
        label: `${device.name} log (${row.level})`,
        sublabel: `${device.platform} ${device.os_version}`,
        snippet: `Device: ${device.name} (${device.platform} ${device.os_version})\n\n${contextLines}`,
      },
    ];
    ideAgent.togglePanel(true);
  }
</script>

<div class="sim-logs-root" data-testid="ide-sim-logs">
  <div class="sim-logs-bar">
    <Badge
      tone={connState === 'live' ? 'ok' : connState === 'error' ? 'bad' : 'neutral'}
      dot={connState === 'live'}
      live={connState === 'live' && !paused}
      label={paused ? 'Paused' : connState === 'live' ? 'Live' : connState === 'connecting' ? 'Connecting…' : 'Idle'}
    />
    <div class="segmented sim-lvl-seg" role="group" aria-label="Minimum log level">
      <button type="button" class:active={minLevel === 'all'} aria-pressed={minLevel === 'all'} onclick={() => (minLevel = 'all')}>All</button>
      <button type="button" class:active={minLevel === 'info'} aria-pressed={minLevel === 'info'} onclick={() => (minLevel = 'info')}>Info</button>
      <button type="button" class:active={minLevel === 'warn'} aria-pressed={minLevel === 'warn'} onclick={() => (minLevel = 'warn')}>Warn</button>
      <button type="button" class:active={minLevel === 'error'} aria-pressed={minLevel === 'error'} onclick={() => (minLevel = 'error')}>Err</button>
    </div>
    <input
      type="search"
      class="input sim-log-filter"
      dir="ltr"
      aria-label="Filter device logs"
      placeholder="Filter tag or text…"
      bind:value={filter}
    />
    <button
      type="button"
      class="icon-btn"
      aria-label={paused ? 'Resume log stream' : 'Pause log stream'}
      title={paused ? 'Resume log stream' : 'Pause log stream'}
      onclick={() => (paused = !paused)}
    >
      <Icon name={paused ? 'play' : 'pause'} size={13} />
    </button>
    <button
      type="button"
      class="icon-btn"
      aria-label="Copy visible logs"
      title="Copy visible logs"
      disabled={filteredRows.length === 0}
      onclick={copyVisibleLogs}
    >
      <Icon name="copy" size={13} />
    </button>
    <button
      type="button"
      class="icon-btn"
      aria-label="Clear logs"
      title="Clear logs"
      disabled={rows.length === 0}
      onclick={() => {
        cancelFlush();
        rows = [];
      }}
    >
      <Icon name="trash" size={13} />
    </button>
  </div>

  {#if connError}
    <div class="sim-log-err" use:announceShown={{ text: connError, assertive: true }}>{connError}</div>
  {/if}

  <div class="sim-logs-scroll mono" dir="ltr" bind:this={scrollEl}>
    {#if device.state !== 'booted'}
      <div class="sim-logs-empty">Boot {device.name} to stream live device logs.</div>
    {:else if filteredRows.length === 0}
      <div class="sim-logs-empty">
        {rows.length === 0 ? 'Waiting for log entries…' : 'No log lines match the current filter.'}
      </div>
    {:else}
      {#each filteredRows as row (row.seq)}
        <div class="sim-log-row" data-level={row.level}>
          <span class="sim-log-lvl">{row.level.slice(0, 1).toUpperCase()}</span>
          {#if row.tag}<span class="sim-log-tag">{row.tag}</span>{/if}
          <span class="sim-log-msg">{row.message}</span>
          {#if row.level === 'error' || row.level === 'fatal'}
            <button
              type="button"
              class="btn small ghost sim-log-ask"
              title="Send error context to AI Agent"
              onclick={() => askAgentAboutRow(row)}
            >
              <Icon name="assistant" size={12} />
              Ask
            </button>
          {/if}
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .sim-logs-root { display: flex; flex-direction: column; flex: 1; min-block-size: 0; min-inline-size: 0; }
  .sim-logs-bar {
    display: flex; align-items: center; gap: 6px; padding-block: 4px; padding-inline: 8px;
    border-block-end: 1px solid var(--border); flex-shrink: 0; overflow-x: auto; min-inline-size: 0;
  }
  .sim-lvl-seg { flex-shrink: 0; }
  .sim-log-filter { flex: 1 1 120px; min-inline-size: 100px; max-inline-size: 240px; block-size: 24px; font-size: var(--fs-xs); }
  .sim-log-err {
    padding: 6px 10px; font-size: var(--fs-xs); color: var(--danger); background: var(--danger-soft);
    border-block-end: 1px solid var(--border); flex-shrink: 0;
  }
  .sim-logs-scroll {
    flex: 1; min-block-size: 0; overflow-y: auto; padding: 6px 8px; font-size: var(--fs-xs);
    line-height: 1.45; display: flex; flex-direction: column; gap: 2px;
  }
  .sim-logs-empty { color: var(--text-dim); padding: 16px 8px; text-align: center; }
  .sim-log-row {
    display: flex; align-items: baseline; gap: 6px; padding-block: 1px; padding-inline: 4px;
    border-radius: var(--radius-s); min-inline-size: 0;
  }
  .sim-log-row:hover { background: var(--hover); }
  .sim-log-lvl {
    flex-shrink: 0; inline-size: 16px; text-align: center; font-weight: 600;
    border-radius: var(--radius-s); background: var(--surface-2); color: var(--text-dim);
  }
  .sim-log-row[data-level='info'] .sim-log-lvl { color: var(--info); background: var(--info-soft); }
  .sim-log-row[data-level='warn'] .sim-log-lvl { color: var(--warning); background: var(--warning-soft); }
  .sim-log-row[data-level='error'] .sim-log-lvl,
  .sim-log-row[data-level='fatal'] .sim-log-lvl { color: var(--danger); background: var(--danger-soft); }
  .sim-log-row[data-level='error'] .sim-log-msg,
  .sim-log-row[data-level='fatal'] .sim-log-msg { color: var(--danger); }
  .sim-log-tag { flex-shrink: 0; max-inline-size: 140px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-dim); font-weight: 500; }
  .sim-log-msg { flex: 1; min-inline-size: 0; white-space: pre-wrap; overflow-wrap: anywhere; color: var(--text); }
  .sim-log-ask { flex-shrink: 0; block-size: 20px; padding-inline: 6px; font-size: var(--fs-xs); }
  @media (pointer: coarse) {
    .sim-logs-bar :global(.icon-btn) { min-inline-size: 36px; min-block-size: 36px; }
  }
</style>
