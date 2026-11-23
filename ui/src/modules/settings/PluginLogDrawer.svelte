<script lang="ts">
  // Collapsible live log tail drawer for a managed runtime plugin helper (E-20).
  // Reads GET /api/v1/plugins/{slug}/logs?limit=200 (root only).
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { api } from '../../lib/api/client';
  import { copyWithToast } from '../../lib/clipboard';
  import { loadErrorText } from '../../lib/loadError';

  export interface PluginLogLine {
    ts: string;
    stream: 'stdout' | 'stderr';
    line: string;
  }
  interface PluginLogsResp {
    slug: string;
    running: boolean;
    pid: number | null;
    lines: PluginLogLine[];
  }

  interface Props {
    slug: string;
    name: string;
  }
  let { slug, name }: Props = $props();

  let open = $state(false);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let lines = $state<PluginLogLine[]>([]);
  let streamFilter = $state<'all' | 'stderr' | 'stdout'>('all');
  let seq = 0;

  async function fetchLogs(): Promise<void> {
    const mySeq = ++seq;
    const s = slug;
    loading = true;
    error = null;
    try {
      const resp = await api.get<PluginLogsResp>(`/plugins/${encodeURIComponent(s)}/logs?limit=200`);
      if (mySeq !== seq || s !== slug) return;
      lines = resp.lines ?? [];
    } catch (e) {
      if (mySeq !== seq || s !== slug) return;
      error = loadErrorText(e);
    } finally {
      if (mySeq === seq && s === slug) loading = false;
    }
  }

  function toggle(): void {
    open = !open;
    if (open) void fetchLogs();
  }

  const shown = $derived(
    streamFilter === 'all' ? lines : lines.filter((l) => l.stream === streamFilter),
  );
  function copyLogs(): void {
    const text = shown.map((l) => `[${l.ts}] [${l.stream}] ${l.line}`).join('\n');
    void copyWithToast(text, `${name} logs`);
  }
</script>

<div class="plog-wrap" data-testid="plugin-log-drawer">
  <button type="button" class="plog-toggle" aria-expanded={open} onclick={toggle}>
    <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
    <span>Helper logs</span>
  </button>
  {#if open}
    <div class="plog-box">
      <div class="plog-bar">
        <div class="segmented" role="group" aria-label="Log stream filter">
          <button type="button" class:active={streamFilter === 'all'} aria-pressed={streamFilter === 'all'} onclick={() => (streamFilter = 'all')}>All</button>
          <button type="button" class:active={streamFilter === 'stderr'} aria-pressed={streamFilter === 'stderr'} onclick={() => (streamFilter = 'stderr')}>stderr</button>
          <button type="button" class:active={streamFilter === 'stdout'} aria-pressed={streamFilter === 'stdout'} onclick={() => (streamFilter = 'stdout')}>stdout</button>
        </div>
        <span class="grow"></span>
        <button type="button" class="btn small ghost" disabled={shown.length === 0} onclick={copyLogs}>
          <Icon name="copy" size={12} /> Copy
        </button>
        <button type="button" class="btn small" disabled={loading} onclick={() => void fetchLogs()}>
          <Icon name="refresh" size={12} /> {loading ? 'Refreshing…' : 'Refresh'}
        </button>
      </div>
      <LoadState what="helper logs" variant="compact" {loading} {error} empty={shown.length === 0} rows={3} onretry={() => void fetchLogs()}>
        {#snippet emptyView()}
          <p class="plog-empty">No {streamFilter === 'all' ? '' : `${streamFilter} `}log lines captured yet.</p>
        {/snippet}
        <pre class="plog-pre" dir="ltr">{#each shown as entry, i (`${entry.ts}:${i}`)}<span class="plog-line" class:stderr={entry.stream === 'stderr'}><span class="plog-ts">{entry.ts.slice(11, 19)}</span> <span class="plog-st">[{entry.stream}]</span> {entry.line}
</span>{/each}</pre>
      </LoadState>
    </div>
  {/if}
</div>

<style>
  .plog-wrap { flex-basis: 100%; min-width: 0; }
  .plog-toggle {
    display: inline-flex; align-items: center; gap: 4px; padding: 4px 0;
    border: 0; background: transparent; color: var(--text-dim); font: inherit; font-size: var(--fs-s); cursor: pointer;
  }
  .plog-toggle:hover { color: var(--text); }
  .plog-box {
    margin-block-start: 6px; border: 1px solid var(--border); border-radius: var(--radius-s);
    background: var(--bg); padding: 8px; display: flex; flex-direction: column; gap: 8px;
  }
  .plog-bar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .grow { flex: 1; }
  .plog-empty { margin: 4px 0; font-size: var(--fs-xs); color: var(--text-dim); }
  .plog-pre {
    margin: 0; max-height: 240px; overflow: auto; font-family: var(--font-mono); font-size: var(--fs-xs);
    line-height: 1.45; white-space: pre-wrap; word-break: break-word; color: var(--text);
  }
  .plog-line.stderr { color: var(--danger); }
  .plog-ts, .plog-st { color: var(--text-dim); }
</style>
