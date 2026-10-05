<script lang="ts">
  import { plural } from '../../lib/plural';
  import { pollWhileVisible } from '../../lib/poll';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  import { tick } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { DaemonLogs } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { copyText } from '../../lib/clipboard';

  type LogMode = 'all' | 'tail';

  const ALL_FILES = '__all__';
  /** Ring cap on what the page holds and renders. "All log files" was ~80k
   *  lines re-split, re-escaped and re-rendered through `{@html}` on every
   *  Live tick; the newest 5k lines are what anyone reads here (Copy copies
   *  what is shown — the files themselves are in the log folder). */
  const MAX_LINES = 5000;

  let loading = $state(true);
  let refreshing = $state(false);
  let autoRefresh = $state(true);
  let follow = $state(true);
  let mode: LogMode = $state('all');
  let tailLines = $state(2000);
  let selected = $state('');
  // The input binds to `filterInput`; the (whole-buffer) filter pass runs on
  // `filter`, debounced so typing doesn't re-scan per keystroke.
  let filterInput = $state('');
  let filter = $state('');
  let content = $state('');
  let nextOffset = $state(0);
  // The file Live follows with `mode=since`: the selected file, or for "All
  // log files" the newest one (the only one still being written).
  let liveFile = '';
  // Bumped by every full read, so an in-flight Live append that started
  // before a file/mode switch is discarded instead of landing on the new view.
  let readGen = 0;
  // True when older lines were dropped — by the ring cap or the daemon's byte cap.
  let clipped = $state(false);
  let payload: DaemonLogs | null = $state(null);
  let logEl: HTMLDivElement | null = $state(null);
  let loadError = $state('');
  let wrap = $state(true);

  const visibleContent = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return content;
    return content
      .split('\n')
      .filter((line) => line.toLowerCase().includes(q))
      .join('\n');
  });

  const lineCount = $derived(content ? content.split('\n').filter(Boolean).length : 0);
  const shownCount = $derived(visibleContent ? visibleContent.split('\n').filter(Boolean).length : 0);

  const status = $derived.by(() => {
    if (!payload) return '';
    const fileCount = payload.files.length;
    const nf = new Intl.NumberFormat();
    const lines = filter.trim()
      ? `${nf.format(shownCount)} of ${nf.format(lineCount)} lines match`
      : `${nf.format(lineCount)} lines`;
    const clip = clipped ? ` (newest ${nf.format(mode === 'tail' ? Math.min(tailLines, MAX_LINES) : MAX_LINES)} kept)` : '';
    return `${lines}${clip} · ${plural(fileCount, 'file')} in the log folder`;
  });

  $effect(() => {
    const q = filterInput;
    const id = setTimeout(() => (filter = q), 150);
    return () => clearTimeout(id);
  });

  /** Keep at most `max` lines (from the end); cheap — scans back from the end. */
  function capLines(s: string, max: number): { text: string; cut: boolean } {
    let idx = s.length;
    // A trailing newline terminates the last line; it doesn't start a new one.
    if (s.endsWith('\n')) idx -= 1;
    for (let n = 0; n < max; n++) {
      idx = s.lastIndexOf('\n', idx - 1);
      if (idx < 0) return { text: s, cut: false };
    }
    return { text: s.slice(idx + 1), cut: true };
  }

  function lineCap(): number {
    return mode === 'tail' && selected !== ALL_FILES ? Math.min(Math.max(1, tailLines), MAX_LINES) : MAX_LINES;
  }

  /** Compare-before-assign: an unchanged read must not re-derive/re-render. */
  function setContent(next: string, serverTruncated: boolean): void {
    const { text, cut } = capLines(next, lineCap());
    if (text !== content) content = text;
    const c = cut || serverTruncated;
    if (c !== clipped) clipped = c;
  }

  function sameFiles(a: DaemonLogs | null, b: DaemonLogs): boolean {
    if (!a || a.log_dir !== b.log_dir || a.files.length !== b.files.length) return false;
    return a.files.every((f, i) => f.name === b.files[i].name);
  }

  function liveFileOf(data: DaemonLogs): string {
    return data.selected === ALL_FILES ? (data.files.at(-1)?.name ?? '') : data.selected;
  }

  // Severity markers: the level word on each line gets a tone (ERROR/WARN),
  // never the whole line (patterns.md → Logs). Escaped first, so log text can
  // never inject markup.
  function esc(t: string): string {
    return t.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }
  const highlighted = $derived(
    esc(visibleContent).replace(
      /(^|\s)(ERROR|WARN|WARNING)(?=\s)/gm,
      (_m, pre: string, lvl: string) => `${pre}<span class="lvl ${lvl === 'ERROR' ? 'err' : 'warn'}">${lvl}</span>`,
    ),
  );

  // Follow tails the log until the user scrolls up; "Jump to latest" resumes.
  function onLogScroll(): void {
    if (!logEl) return;
    const atBottom = logEl.scrollHeight - logEl.scrollTop - logEl.clientHeight < 24;
    if (follow && !atBottom) follow = false;
    else if (!follow && atBottom) follow = true;
  }
  function jumpToLatest(): void {
    follow = true;
    if (logEl) logEl.scrollTop = logEl.scrollHeight;
  }
  async function copyLog(): Promise<void> {
    const ok = await copyText(visibleContent);
    if (ok) toasts.success(filter.trim() ? 'Matching lines copied' : 'Log copied');
    else toasts.error('Couldn’t copy the log', 'The clipboard write was blocked.');
  }

  $effect(() => {
    void loadInitial();
  });

  // Shared poll chain: a slow read never overlaps the next, and nothing runs
  // while the window is hidden (one catch-up read on return).
  $effect(() => {
    if (!autoRefresh) return;
    const p = pollWhileVisible(() => refreshIncremental(), { ms: 1500, immediate: false });
    return () => p.stop();
  });

  async function loadInitial(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      const data = await fetchLogs(mode);
      payload = data;
      selected = data.selected;
      liveFile = liveFileOf(data);
      setContent(data.content, data.truncated);
      nextOffset = data.next_offset;
      await maybeFollow();
    } catch (e) {
      // Inline with Retry (a toast left an empty toolbar with nothing under it).
      loadError = loadErrorText(e);
      autoRefresh = false;
    } finally {
      loading = false;
    }
  }

  async function refreshFull(): Promise<void> {
    refreshing = true;
    const gen = ++readGen;
    try {
      const data = await fetchLogs(mode);
      if (gen !== readGen) return;
      if (!sameFiles(payload, data) || payload?.selected !== data.selected || payload?.mode !== data.mode) payload = data;
      selected = data.selected;
      liveFile = liveFileOf(data);
      setContent(data.content, data.truncated);
      nextOffset = data.next_offset;
      await maybeFollow();
    } catch (e) {
      failed('Couldn’t refresh the log', e);
    } finally {
      refreshing = false;
    }
  }

  /** Live tick: fetch only the bytes written since the last read of the live
   *  file (every mode — "All log files" and Tail included) and append. Falls
   *  back to a full read on rollover (a new newest file) or truncation. */
  async function refreshIncremental(): Promise<void> {
    if (refreshing || loading || loadError) return;
    if (!liveFile) {
      await refreshFull();
      return;
    }
    refreshing = true;
    let full = false;
    const gen = readGen;
    const file = liveFile;
    const from = nextOffset;
    try {
      const data = await fetchLogs('since', from, file);
      // A full read (file/mode switch) landed meanwhile — drop this append.
      if (gen !== readGen || file !== liveFile || from !== nextOffset) return;
      if (
        (selected === ALL_FILES && data.files.at(-1)?.name !== liveFile) ||
        data.next_offset < nextOffset
      ) {
        full = true; // new day's file, or the file shrank under us
      } else {
        if (!sameFiles(payload, data)) payload = { ...data, selected, mode };
        if (data.truncated) setContent(data.content, true);
        else if (data.content) setContent(content + data.content, clipped);
        nextOffset = data.next_offset;
        if (data.content) await maybeFollow();
      }
    } catch (e) {
      failed('Couldn’t update the log', e);
    } finally {
      refreshing = false;
    }
    if (full) await refreshFull();
  }

  /** A failed read. While Live is on this ran every 1.5 s and stacked a toast
   *  per tick; pause Live instead and say so once. */
  function failed(title: string, e: unknown): void {
    const msg = e instanceof Error ? e.message : String(e);
    if (autoRefresh) {
      autoRefresh = false;
      toasts.error(`${title} — live updates paused`, `${msg} Turn Live back on to retry.`);
    } else {
      toasts.error(title, msg);
    }
  }

  async function fetchLogs(fetchMode: LogMode | 'since', offset?: number, file?: string): Promise<DaemonLogs> {
    const params = new URLSearchParams();
    const f = file ?? selected;
    if (f) params.set('file', f);
    params.set('mode', fetchMode);
    if (fetchMode === 'tail') params.set('lines', String(Math.max(1, tailLines)));
    if (fetchMode === 'since') params.set('offset', String(offset ?? 0));
    return api.get<DaemonLogs>(`/logs/daemon?${params.toString()}`);
  }

  async function onFileChange(): Promise<void> {
    if (selected === ALL_FILES) mode = 'all';
    await refreshFull();
  }

  async function onModeChange(): Promise<void> {
    await refreshFull();
  }

  async function maybeFollow(): Promise<void> {
    if (!follow) return;
    await tick();
    if (logEl) logEl.scrollTop = logEl.scrollHeight;
  }
</script>

<div class="settings-section logs-section">
  <PageHeader title={sectionLabel('logs')} subtitle={payload?.log_dir ?? '~/Library/Logs/Otto'}>
    {#snippet actions()}
      <button class="btn small" data-icon="copy" data-overflow="-1" disabled={loading || !visibleContent} onclick={() => void copyLog()}>
        <Icon name="copy" size={12} /> Copy
      </button>
      <button class="btn small" data-icon="refresh" disabled={refreshing || loading} onclick={() => (loadError ? void loadInitial() : void refreshFull())}>
        <Icon name="refresh" size={12} />
        {refreshing ? 'Refreshing…' : 'Refresh'}
      </button>
    {/snippet}
  </PageHeader>
  <PageBody padded={false} fill>

  {#if loading}
    <div class="pad"><Skeleton rows={8} height={34} /></div>
  {:else if loadError && !payload}
    <div class="pad">
      <LoadState what="the daemon logs" error={loadError} empty onretry={() => void loadInitial()} />
    </div>
  {:else}
    <div class="toolbar">
      <label class="field compact">
        <span>File</span>
        <select class="input mono" bind:value={selected} onchange={onFileChange}>
          <option value={ALL_FILES}>All log files</option>
          {#each payload?.files ?? [] as file (file.name)}
            <option value={file.name}>{file.name}</option>
          {/each}
        </select>
      </label>

      <label class="field mode">
        <span>Read</span>
        <!-- "All log files" always reads every file (newest content first,
             byte-capped by the daemon), so Tail is only offered for a single file. -->
        <select
          class="input"
          bind:value={mode}
          onchange={onModeChange}
          disabled={selected === ALL_FILES}
          title={selected === ALL_FILES ? 'Pick one file to tail it' : undefined}
        >
          <option value="all">Full file</option>
          <option value="tail">Tail</option>
        </select>
      </label>

      {#if mode === 'tail'}
        <label class="field lines">
          <span>Lines</span>
          <input
            class="input mono"
            type="number"
            min="1"
            max="50000"
            bind:value={tailLines}
            title="Up to 50,000 lines — press Enter or Refresh to apply"
            onkeydown={(e) => e.key === 'Enter' && refreshFull()}
          />
        </label>
      {/if}

      <label class="field search-field">
        <span>Filter</span>
        <input class="input" type="search" placeholder="slack, telegram, bridge…" bind:value={filterInput} />
      </label>

      <div class="checks">
        <label class="checkbox-row check-control" title="Poll for new log lines every 1.5 s">
          <input type="checkbox" bind:checked={autoRefresh} />
          Live
        </label>
        <label class="checkbox-row check-control" title="Keep the view scrolled to the newest line">
          <input type="checkbox" bind:checked={follow} onchange={() => follow && jumpToLatest()} />
          Follow
        </label>
        <label class="checkbox-row check-control" title="Wrap long lines">
          <input type="checkbox" bind:checked={wrap} />
          Wrap
        </label>
      </div>
    </div>

    <div class="log-meta">
      <span>{status}</span>
      {#if autoRefresh}<span class="live"><span class="live-dot" aria-hidden="true"></span>Live</span>{:else}<span>Paused</span>{/if}
    </div>

    <div class="log-wrap">
      {#if filter.trim() && !visibleContent}
        <div class="no-match">
          No lines match “{filter.trim()}”.
          <button class="btn small ghost" onclick={() => (filterInput = filter = '')}>Clear filter</button>
        </div>
      {:else if !content.trim()}
        <!-- A fresh daemon (or a just-rotated file) has nothing yet — say so
             rather than show a blank terminal-coloured pane. -->
        <div class="no-match">
          {selected === ALL_FILES ? 'The log folder is empty.' : 'This log file is empty so far.'}
          {autoRefresh ? 'New lines appear here as the daemon writes them.' : 'Turn on Live to watch for new lines.'}
        </div>
      {/if}
      <!-- eslint-disable-next-line svelte/no-at-html-tags -- escaped in `highlighted` -->
      <div class="log-view" class:nowrap={!wrap} bind:this={logEl} onscroll={onLogScroll} role="textbox" aria-readonly="true" aria-multiline="true" tabindex="0" aria-label="Daemon log">{@html highlighted}</div>
      {#if !follow && visibleContent}
        <button class="btn small jump" onclick={jumpToLatest}><Icon name="arrowDown" size={12} /> Jump to latest</button>
      {/if}
    </div>
  {/if}
  </PageBody>
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .pad {
    padding: 18px 20px;
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 8px 12px;
    padding: 12px 20px;
    border-bottom: 1px solid var(--border);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 0;
  }
  .field span {
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text-dim);
  }
  .field.compact {
    width: 210px;
  }
  /* "Full file" / "Tail" — narrow, so Live · Follow · Wrap stay on the
     toolbar's one row at a 1280px window instead of wrapping to a second. */
  .field.mode {
    width: 120px;
  }
  .field.lines {
    width: 96px;
  }
  .search-field {
    flex: 1;
    min-width: 180px;
  }
  .checks {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 27px;
  }
  .check-control {
    font-size: var(--fs-s);
    user-select: none;
    white-space: nowrap;
  }
  .log-meta {
    min-height: 28px;
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    padding: 0 20px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    border-bottom: 1px solid var(--border);
  }
  .live {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .live-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--status-working);
  }
  .log-wrap {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .no-match {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 20px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .log-view {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 14px 18px 32px;
    overflow: auto;
    background: var(--term-bg);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    border: 0;
  }
  .log-view.nowrap {
    white-space: pre;
    overflow-wrap: normal;
  }
  .log-view :global(.lvl) {
    font-weight: 600;
  }
  .log-view :global(.lvl.err) {
    color: var(--danger);
  }
  .log-view :global(.lvl.warn) {
    color: var(--warning);
  }
  .jump {
    position: absolute;
    inset-block-end: 16px;
    inset-inline-end: 24px;
    box-shadow: var(--shadow);
  }
</style>
