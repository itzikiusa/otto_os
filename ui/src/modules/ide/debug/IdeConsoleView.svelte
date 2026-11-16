<script lang="ts">
  // Shared crisp Console + Structured JSON Log Formatter + stacktrace file:line
  // linkifier used across both `IdeDebuggerPanel` and `IdeRunPanel`.
  import type { Snippet } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { api } from '../../../lib/api/client';
  import { copyWithToast } from '../../../lib/clipboard';
  import { scrollBehavior } from '../../../lib/motion';
  import {
    ide,
    ideBasename,
    type IdeReplEntry,
    type IdeSearchItem,
  } from '../ideStore.svelte';
  import { pushIdeNavLocation } from '../editor/ideEditorStoreBridge.svelte';
  import { ideDockPrefs } from './ideDockPrefs.svelte';
  import {
    isPlainErrorLine,
    parsePlainTextSegments,
    parseStructuredJsonLog,
    stripAnsiCodes,
    type ConsoleFileLink,
    type ParsedJsonLog,
  } from './ideLogParser';

  export interface ConsoleLineInput {
    text: string;
    isError?: boolean;
  }

  interface Props {
    sessionKey?: string;
    lines: readonly ConsoleLineInput[];
    replEntries?: readonly IdeReplEntry[];
    emptyText?: string;
    title?: string;
    clearLabel?: string;
    onclear?: () => void;
    footer?: Snippet;
  }

  let {
    sessionKey = '',
    lines,
    replEntries = [],
    emptyText = 'Program output appears here.',
    title,
    clearLabel = 'Clear console output',
    onclear,
    footer,
  }: Props = $props();

  const LOG_CAP = 500;

  let filterQuery = $state('');
  let showAll = $state(false);
  let clearedLineIdx = $state(0);
  let expandedJsonRows = $state<Record<number, boolean>>({});
  let scrollEl = $state<HTMLDivElement>();
  let stickBottom = $state(true);

  let lastSessionKey = '';
  $effect(() => {
    const k = sessionKey;
    if (k !== lastSessionKey) {
      lastSessionKey = k;
      clearedLineIdx = 0;
      expandedJsonRows = {};
      showAll = false;
      stickBottom = true;
    } else if (lines.length < clearedLineIdx) {
      clearedLineIdx = 0;
    }
  });

  interface PreparedRow {
    index: number;
    clean: string;
    isError: boolean;
    json: ParsedJsonLog | null;
  }

  const activeSlice = $derived(clearedLineIdx > 0 ? lines.slice(clearedLineIdx) : lines);

  const filteredRows = $derived.by((): PreparedRow[] => {
    const q = filterQuery.trim().toLowerCase();
    const pretty = ideDockPrefs.consolePrettyJson;
    const out: PreparedRow[] = [];
    for (let i = 0; i < activeSlice.length; i++) {
      const src = activeSlice[i];
      const clean = stripAnsiCodes(src.text).replace(/\r?\n$/, '');
      if (q && !clean.toLowerCase().includes(q)) continue;
      const json = pretty ? parseStructuredJsonLog(clean) : null;
      out.push({
        index: clearedLineIdx + i,
        clean,
        isError: Boolean(src.isError) || (json ? json.tone === 'error' : isPlainErrorLine(clean)),
        json,
      });
    }
    return out;
  });

  const filteredRepl = $derived.by((): readonly IdeReplEntry[] => {
    const q = filterQuery.trim().toLowerCase();
    if (!q) return replEntries;
    return replEntries.filter(
      (r) => r.expression.toLowerCase().includes(q) || r.result.toLowerCase().includes(q),
    );
  });

  const hiddenCount = $derived(!showAll && filteredRows.length > LOG_CAP ? filteredRows.length - LOG_CAP : 0);
  const visibleRows = $derived(hiddenCount > 0 ? filteredRows.slice(-LOG_CAP) : filteredRows);
  const hasAnyContent = $derived(activeSlice.length > 0 || replEntries.length > 0);

  $effect(() => {
    void activeSlice.length;
    void replEntries.length;
    const el = scrollEl;
    if (!el || !stickBottom) return;
    queueMicrotask(() => {
      el.scrollTop = el.scrollHeight;
    });
  });

  function onScroll(): void {
    const el = scrollEl;
    if (!el) return;
    stickBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 28;
  }

  function jumpToBottom(): void {
    const el = scrollEl;
    if (!el) return;
    stickBottom = true;
    el.scrollTo({ top: el.scrollHeight, behavior: scrollBehavior() });
  }

  function toggleRowExpand(index: number): void {
    expandedJsonRows = { ...expandedJsonRows, [index]: !expandedJsonRows[index] };
  }

  function clearConsole(): void {
    clearedLineIdx = lines.length;
    expandedJsonRows = {};
    onclear?.();
  }

  function copyAll(): void {
    const outLines = filteredRows.map((r) => r.clean.trimEnd());
    const replLines = filteredRepl.map((r) => `> ${r.expression}\n${r.result}`);
    const text = [...outLines, ...replLines].filter(Boolean).join('\n');
    if (text) void copyWithToast(text, 'console output');
  }

  async function openConsoleLink(link: ConsoleFileLink): Promise<void> {
    const raw = link.file.trim();
    if (!raw) return;
    if (ide.activePath) pushIdeNavLocation(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
    if (raw.startsWith('/')) {
      await ide.openFile(raw, { line: link.line, col: link.col, preview: false });
      return;
    }
    const cleanRel = raw.replace(/^\.\//, '').replace(/^\/+/, '');
    const suffix = `/${cleanRel}`;
    const base = ideBasename(cleanRel);

    const openMatch = ide.tabs.find((t) => t.path.endsWith(suffix));
    if (openMatch) {
      await ide.openFile(openMatch.path, { line: link.line, col: link.col, preview: false });
      return;
    }
    for (const entries of Object.values(ide.childrenByDir)) {
      const hit = entries.find((e) => e.kind === 'file' && e.path.endsWith(suffix));
      if (hit) {
        await ide.openFile(hit.path, { line: link.line, col: link.col, preview: false });
        return;
      }
    }
    if (ide.target.root) {
      try {
        const res = await api.post<{ items: IdeSearchItem[] }>('/ide/search', {
          target: ide.targetPayload,
          query: base,
          mode: 'files',
          limit: 25,
        });
        const items = res.items ?? [];
        const exact = items.find((it) => it.path.endsWith(suffix)) ?? items.find((it) => ideBasename(it.path) === base);
        if (exact) {
          await ide.openFile(exact.path, { line: link.line, col: link.col, preview: false });
          return;
        }
      } catch {
        /* fall back to workspace root join */
      }
    }
    await ide.openFile(ide.resolvePath(cleanRel), { line: link.line, col: link.col, preview: false });
  }
</script>

<div class="con-root" data-testid="ide-console-view">
  <header class="con-bar">
    {#if title}<span class="con-title">{title}</span>{/if}
    <button
      type="button"
      class="con-toggle"
      class:is-on={ideDockPrefs.consolePrettyJson}
      aria-pressed={ideDockPrefs.consolePrettyJson}
      title="Format structured JSON log lines"
      onclick={() => ideDockPrefs.setConsolePrettyJson(!ideDockPrefs.consolePrettyJson)}
      data-testid="ide-console-pretty-json"
    ><Icon name="format" size={12} /><span>Pretty JSON</span></button>
    <button
      type="button"
      class="con-toggle"
      class:is-on={ideDockPrefs.consoleWrapLines}
      aria-pressed={ideDockPrefs.consoleWrapLines}
      title="Soft-wrap long console lines"
      onclick={() => ideDockPrefs.setConsoleWrapLines(!ideDockPrefs.consoleWrapLines)}
      data-testid="ide-console-wrap"
    ><Icon name="wrap" size={12} /><span>Wrap</span></button>

    <div class="input-group con-filter">
      <Icon name="search" size={12} />
      <input type="search" dir="ltr" aria-label="Filter console output" placeholder="Filter logs…" bind:value={filterQuery} data-testid="ide-console-filter" />
    </div>

    <span class="grow"></span>
    {#if filterQuery.trim()}<span class="con-count dim">{filteredRows.length}/{activeSlice.length}</span>{/if}

    <button type="button" class="icon-btn" aria-label="Copy console output" title="Copy console output" disabled={!hasAnyContent} onclick={copyAll}>
      <Icon name="copy" size={12} />
    </button>
    <button type="button" class="icon-btn" aria-label={clearLabel} title={clearLabel} disabled={!hasAnyContent} onclick={clearConsole} data-testid="ide-console-clear">
      <Icon name="trash" size={12} />
    </button>
  </header>

  <div class="con-body-wrap">
    <div class="con-scroll" class:is-wrap={ideDockPrefs.consoleWrapLines} dir="ltr" bind:this={scrollEl} onscroll={onScroll}>
      {#if !hasAnyContent}
        <p class="con-empty dim">{emptyText}</p>
      {:else if filteredRows.length === 0 && filteredRepl.length === 0}
        <div class="con-empty-filter">
          <span class="dim">No console lines match “{filterQuery.trim()}”.</span>
          <button type="button" class="btn small ghost" onclick={() => (filterQuery = '')}>Clear filter</button>
        </div>
      {:else}
        {#if hiddenCount > 0}
          <div class="con-cap-bar">
            <button type="button" class="btn small ghost" onclick={() => (showAll = true)}>
              Show all {filteredRows.length} lines ({hiddenCount} earlier hidden)
            </button>
          </div>
        {/if}

        {#each visibleRows as row (row.index)}
          {#if row.json}
            {@const j = row.json}
            {@const isOpen = Boolean(expandedJsonRows[row.index])}
            <div class="con-json-row" class:is-open={isOpen} data-testid="ide-console-json-line">
              <div class="con-json-main">
                {#if j.timestamp}<span class="con-ts mono">{j.timestamp}</span>{/if}
                <span class={`con-lvl mono is-${j.tone}`}>{j.levelLabel}</span>
                {#if j.caller}
                  {@const callerLink = j.caller}
                  <button
                    type="button"
                    class="con-file-link mono"
                    title={`Open ${callerLink.display} in editor`}
                    onclick={() => void openConsoleLink(callerLink)}
                    data-testid="ide-console-caller-link"
                  >{callerLink.display}</button>
                {/if}
                <span class="con-msg mono" class:is-err={j.tone === 'error'}>{j.message}</span>
                {#if j.extraEntries.length > 0}
                  <button
                    type="button"
                    class="con-fields-btn mono"
                    aria-expanded={isOpen}
                    aria-label={isOpen ? 'Collapse JSON log fields' : `Expand ${j.extraEntries.length} JSON log fields`}
                    title={isOpen ? 'Collapse JSON log fields' : 'Inspect raw JSON fields'}
                    onclick={() => toggleRowExpand(row.index)}
                  >
                    <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
                    <span>{j.extraEntries.length === 1 ? '1 field' : `${j.extraEntries.length} fields`}</span>
                  </button>
                {/if}
              </div>
              {#if isOpen}
                <div class="con-json-detail">
                  <div class="con-kv-list">
                    {#each j.extraEntries as [k, v] (k)}
                      <div class="con-kv-row mono"><span class="con-kv-key">{k}:</span><span class="con-kv-val">{v}</span></div>
                    {/each}
                  </div>
                  <div class="con-json-actions">
                    <button type="button" class="btn small ghost" onclick={() => void copyWithToast(j.rawPretty, 'JSON log entry')}>
                      <Icon name="copy" size={12} /><span>Copy JSON</span>
                    </button>
                  </div>
                </div>
              {/if}
            </div>
          {:else}
            {@const segs = parsePlainTextSegments(row.clean)}
            <div class="con-plain-line mono" class:is-err={row.isError}>
              {#if row.clean.length === 0}
                {'\u00a0'}
              {:else}
                {#each segs as seg, sIdx (sIdx)}
                  {#if seg.kind === 'link'}
                    <button type="button" class="con-file-link mono" title={`Open ${seg.link.display} in editor`} onclick={() => void openConsoleLink(seg.link)}>{seg.link.display}</button>
                  {:else}
                    {seg.text}
                  {/if}
                {/each}
              {/if}
            </div>
          {/if}
        {/each}

        {#each filteredRepl as entry (entry.id)}
          <div class="dbg-repl-item reveal-host">
            <div class="dbg-repl-expr mono">&gt; {entry.expression}</div>
            <div class="dbg-repl-res mono" class:is-err={entry.isError}>
              <span class="dbg-repl-val">{entry.result}</span>
              {#if entry.var_type}<span class="con-repl-type">{entry.var_type}</span>{/if}
              <button type="button" class="icon-btn reveal-on-hover" aria-label={`Copy result for ${entry.expression}`} title={`Copy result for ${entry.expression}`} onclick={() => void copyWithToast(entry.result, entry.expression)}>
                <Icon name="copy" size={12} />
              </button>
            </div>
          </div>
        {/each}
      {/if}
    </div>

    {#if !stickBottom && hasAnyContent}
      <div class="con-tail-cta">
        <button type="button" class="btn small" onclick={jumpToBottom}><Icon name="arrowDown" size={12} /><span>Follow output</span></button>
      </div>
    {/if}
  </div>

  {#if footer}{@render footer()}{/if}
</div>

<style>
  .con-root { display: flex; flex-direction: column; flex: 1; block-size: 100%; min-block-size: 0; min-inline-size: 0; background: var(--term-bg); }
  .con-bar {
    display: flex; align-items: center; gap: 6px; block-size: 28px; min-block-size: 28px; max-block-size: 28px;
    padding-inline: 8px; border-block-end: 1px solid var(--border); background: var(--surface-2);
    flex-shrink: 0; overflow-x: auto; overflow-y: hidden;
  }
  .con-title { font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); text-transform: uppercase; letter-spacing: 0.06em; margin-inline-end: 2px; white-space: nowrap; }
  .con-toggle {
    display: inline-flex; align-items: center; gap: 4px; block-size: 22px; padding-inline: 6px;
    border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface);
    color: var(--text-dim); font-size: var(--fs-xs); white-space: nowrap; cursor: pointer; flex-shrink: 0;
  }
  .con-toggle:hover { background: var(--hover); color: var(--text); }
  .con-toggle.is-on { border-color: var(--accent-line); background: var(--accent-soft); color: var(--accent-text); }
  .con-filter { block-size: 22px; inline-size: 164px; min-inline-size: 96px; font-size: var(--fs-xs); }
  .con-count { font-size: var(--fs-xs); font-variant-numeric: tabular-nums; white-space: nowrap; }
  .con-body-wrap { position: relative; flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; flex-direction: column; }
  .con-scroll { flex: 1; min-block-size: 0; min-inline-size: 0; overflow: auto; padding: 6px 10px; background: var(--term-bg); color: var(--text); font-size: var(--fs-xs); }
  .con-empty { margin: 0; padding-block: 8px; font-size: var(--fs-xs); }
  .con-empty-filter { display: flex; align-items: center; gap: 8px; padding-block: 8px; font-size: var(--fs-xs); }
  .con-cap-bar { padding-block-end: 6px; }
  .con-json-row { padding-block: 2px; border-block-end: 1px solid var(--separator); }
  .con-json-main { display: flex; align-items: baseline; gap: 6px; line-height: 1.45; white-space: nowrap; }
  .con-scroll.is-wrap .con-json-main { flex-wrap: wrap; white-space: normal; }
  .con-ts { color: var(--text-dim); font-size: var(--fs-xs); font-variant-numeric: tabular-nums; flex-shrink: 0; }
  .con-lvl { display: inline-flex; align-items: center; justify-content: center; min-inline-size: 40px; padding-inline: 4px; border-radius: var(--radius-s); font-size: var(--fs-xs); font-weight: 600; flex-shrink: 0; }
  .con-lvl.is-info { background: var(--info-soft); color: var(--info); }
  .con-lvl.is-warn { background: var(--warning-soft); color: var(--warning); }
  .con-lvl.is-error { background: var(--danger-soft); color: var(--danger); }
  .con-lvl.is-debug { background: var(--surface-2); color: var(--text-dim); }
  .con-file-link { display: inline; padding: 0; border: 0; background: transparent; color: var(--accent-text); font-size: var(--fs-xs); text-decoration: underline; text-underline-offset: 2px; cursor: pointer; flex-shrink: 0; }
  .con-file-link:hover { color: var(--info); }
  .con-msg { color: var(--text); font-size: var(--fs-xs); min-inline-size: 0; }
  .con-scroll.is-wrap .con-msg { white-space: pre-wrap; overflow-wrap: break-word; }
  .con-msg.is-err, .con-plain-line.is-err, .dbg-repl-res.is-err { color: var(--danger); }
  .con-fields-btn { display: inline-flex; align-items: center; gap: 2px; padding: 0 6px; border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface-2); color: var(--text-dim); font-size: var(--fs-xs); cursor: pointer; flex-shrink: 0; }
  .con-fields-btn:hover { background: var(--hover); color: var(--text); }
  .con-json-detail { margin-block: 4px; margin-inline-start: 16px; padding: 8px 10px; border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface-2); }
  .con-kv-list { display: flex; flex-direction: column; gap: 4px; }
  .con-kv-row { display: flex; align-items: baseline; gap: 6px; font-size: var(--fs-xs); line-height: 1.4; }
  .con-kv-key { color: var(--accent-text); font-weight: 500; flex-shrink: 0; }
  .con-kv-val { color: var(--text); white-space: pre-wrap; overflow-wrap: anywhere; }
  .con-json-actions { display: flex; justify-content: flex-end; margin-block-start: 6px; }
  .con-plain-line { font-size: var(--fs-xs); line-height: 1.45; white-space: pre; }
  .con-scroll.is-wrap .con-plain-line { white-space: pre-wrap; overflow-wrap: break-word; }
  .dbg-repl-item { padding-block: 4px; border-block-end: 1px solid var(--separator); }
  .dbg-repl-expr { color: var(--text-dim); font-size: var(--fs-xs); overflow-wrap: anywhere; }
  .dbg-repl-res { display: flex; align-items: center; gap: 6px; color: var(--text); font-size: var(--fs-xs); overflow-wrap: anywhere; }
  .dbg-repl-val { flex: 1; min-inline-size: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  .con-repl-type { color: var(--text-dim); font-size: var(--fs-xs); max-inline-size: 140px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .con-tail-cta { position: absolute; inset-block-end: 8px; inset-inline-end: 12px; z-index: var(--z-sticky); }
</style>
