<script lang="ts">
  // Shared crisp Console + Structured JSON Log Formatter + stacktrace file:line
  // linkifier used across both `IdeDebuggerPanel` and `IdeRunPanel`. Rows are
  // keyed and cleared by sequence number (never by array index, which slides
  // once the buffer is capped), and evaluations sit where they were made.
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
  import IdeVarRows from './IdeVarRows.svelte';
  import { interleaveRepl } from './ideConsoleModel';
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
    /** Monotonic sequence number (default: position + 1). */
    seq?: number;
    /** Last sequence number that wrote to this line (a stitched line). */
    endSeq?: number;
    /** Stable row key (default: `seq`). */
    key?: string;
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
  /** Lines up to this sequence number were cleared. */
  let clearedSeq = $state(0);
  let expandedJsonRows = $state<Record<string, boolean>>({});
  let replOpen = $state<Record<string, boolean>>({});
  let replPaths = $state<Record<string, boolean>>({});
  let scrollEl = $state<HTMLDivElement>();
  let stickBottom = $state(true);

  const seqOf = (l: ConsoleLineInput, i: number): number => l.seq ?? i + 1;
  const lastSeq = $derived(lines.length > 0 ? (lines[lines.length - 1].endSeq ?? seqOf(lines[lines.length - 1], lines.length - 1)) : 0);

  let lastSessionKey = '';
  $effect(() => {
    const k = sessionKey;
    if (k !== lastSessionKey) {
      lastSessionKey = k;
      clearedSeq = 0;
      expandedJsonRows = {};
      replOpen = {};
      replPaths = {};
      showAll = false;
      stickBottom = true;
    } else if (lastSeq < clearedSeq) {
      clearedSeq = 0; // the counter restarted (a restarted session)
    }
  });

  interface PreparedRow {
    key: string;
    seq: number;
    clean: string;
    isError: boolean;
    json: ParsedJsonLog | null;
  }

  const activeSlice = $derived.by((): { line: ConsoleLineInput; seq: number }[] => {
    const out: { line: ConsoleLineInput; seq: number }[] = [];
    for (let i = 0; i < lines.length; i++) {
      const seq = seqOf(lines[i], i);
      if ((lines[i].endSeq ?? seq) > clearedSeq) out.push({ line: lines[i], seq });
    }
    return out;
  });

  const filteredRows = $derived.by((): PreparedRow[] => {
    const q = filterQuery.trim().toLowerCase();
    const pretty = ideDockPrefs.consolePrettyJson;
    const out: PreparedRow[] = [];
    for (const { line: src, seq } of activeSlice) {
      const clean = stripAnsiCodes(src.text).replace(/\r?\n$/, '');
      if (q && !clean.toLowerCase().includes(q)) continue;
      const json = pretty ? parseStructuredJsonLog(clean) : null;
      out.push({
        key: src.key ?? String(seq),
        seq,
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
  // Evaluations hidden with the capped lines stay hidden with them.
  const visibleRepl = $derived(
    hiddenCount > 0 && visibleRows.length > 0
      ? filteredRepl.filter((r) => typeof r.afterSeq !== 'number' || r.afterSeq >= visibleRows[0].seq - 1)
      : filteredRepl,
  );
  const items = $derived(interleaveRepl(visibleRows, visibleRepl));

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

  function toggleRowExpand(key: string): void {
    expandedJsonRows = { ...expandedJsonRows, [key]: !expandedJsonRows[key] };
  }

  async function toggleRepl(entry: IdeReplEntry): Promise<void> {
    const ref = entry.variables_reference ?? 0;
    const next = !replOpen[entry.id];
    replOpen = { ...replOpen, [entry.id]: next };
    if (next && ref > 0 && !ide.expandedVariables[ref]) await ide.expandVariables(ref);
  }

  function clearConsole(): void {
    clearedSeq = lastSeq;
    expandedJsonRows = {};
    replOpen = {};
    onclear?.();
  }

  function copyAll(): void {
    const text = interleaveRepl(filteredRows, filteredRepl)
      .map((it) => (it.kind === 'line' ? it.line.clean.trimEnd() : `> ${it.entry.expression}\n${it.entry.result}`))
      .filter(Boolean)
      .join('\n');
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

        {#each items as item (item.kind === 'line' ? `l:${item.line.key}` : `r:${item.entry.id}`)}
          {#if item.kind === 'repl'}
            {@const entry = item.entry}
            {@const ref = entry.variables_reference ?? 0}
            {@const open = Boolean(replOpen[entry.id])}
            <div class="dbg-repl-item reveal-host" data-testid="ide-console-repl-entry">
              <div class="dbg-repl-expr mono">&gt; {entry.expression}</div>
              <div class="dbg-repl-res mono" class:is-err={entry.isError}>
                {#if ref > 0 && !entry.isError}
                  <button type="button" class="icon-btn con-repl-twisty" aria-expanded={open} aria-label={open ? `Collapse ${entry.expression}` : `Expand ${entry.expression}`} title={open ? `Collapse ${entry.expression}` : `Expand ${entry.expression}`} onclick={() => void toggleRepl(entry)}>
                    <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
                  </button>
                {/if}
                <span class="dbg-repl-val">{entry.result}</span>
                {#if entry.var_type}<span class="con-repl-type">{entry.var_type}</span>{/if}
                <button type="button" class="icon-btn reveal-on-hover" aria-label={`Copy result for ${entry.expression}`} title={`Copy result for ${entry.expression}`} onclick={() => void copyWithToast(entry.result, entry.expression)}>
                  <Icon name="copy" size={12} />
                </button>
              </div>
              {#if open && ref > 0}
                <div class="con-repl-children" role="list" aria-label={`Fields of ${entry.expression}`}>
                  <IdeVarRows
                    vars={ide.expandedVariables[ref] ?? []}
                    root={`repl:${entry.id}`}
                    chain={[{ name: entry.expression, value: entry.result, var_type: entry.var_type, variables_reference: ref }]}
                    openPaths={replPaths}
                    onsetopen={(p, o) => (replPaths = { ...replPaths, [p]: o })}
                  />
                </div>
              {/if}
            </div>
          {:else if item.line.json}
            {@const row = item.line}
            {@const j = item.line.json}
            {@const isOpen = Boolean(expandedJsonRows[row.key])}
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
                    onclick={() => toggleRowExpand(row.key)}
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
            {@const row = item.line}
            {@const segs = parsePlainTextSegments(row.clean)}
            <div class="con-plain-line mono" class:is-err={row.isError} data-testid="ide-console-line">
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
  .con-repl-twisty { inline-size: 18px; block-size: 18px; flex-shrink: 0; }
  .con-repl-children { margin-inline-start: -10px; }
  .con-repl-type { color: var(--text-dim); font-size: var(--fs-xs); max-inline-size: 140px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .con-tail-cta { position: absolute; inset-block-end: 8px; inset-inline-end: 12px; z-index: var(--z-sticky); }
</style>
