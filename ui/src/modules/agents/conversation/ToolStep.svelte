<script lang="ts">
  // One tool call inside a response's step group, as ONE scannable line —
  // kind icon · verb · target (command / file / pattern) · dim detail — with a
  // status mark at the end (spinner while running, ✓, ✕, or "no result") and,
  // for commands, the output's last line under it (the tail you'd look for
  // first: "test result: FAILED …"). Expanding shows the detail: the full
  // command + output (scrolled to its end; windowed past 400 lines), an edit's
  // inline diff, a written file's content, a read's text, result images.
  // Elided live-push previews fetch the stored result on first expand.
  import { getContext, tick } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import VirtualList from '../../../lib/components/VirtualList.svelte';
  import ImageBlock from './ImageBlock.svelte';
  import InlineDiff from './InlineDiff.svelte';
  import CodeView from './CodeView.svelte';
  import Markdown from './Markdown.svelte';
  import { openFile } from '../../../lib/stores/openfile.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { linkifyOutput } from './chatMarkdown';
  import { openExternal } from '../../../lib/external';
  import { TOOL_CHROME, editInputPatch, relPath, fmtBytes, lastLine, patchToDiff, toolLine, toolStatus } from './format';
  import { autoLang, ensureHljs, highlightBlock, highlightLine, langFromPath } from '../../../lib/hl';
  import type { Block } from '../../../lib/api/types';
  import { CONV_CTX, type ConvContext } from './context';
  import { effectiveResult, fetchToolResult, isElided } from './toolDetail';
  import type { ToolResult } from '../../../lib/api/types';

  interface Props {
    block: Extract<Block, { kind: 'tool_call' }>;
    /** The response is still being worked on — a call without a result is
     *  running, not abandoned. */
    live?: boolean;
    /** The agent is stopped on this call waiting for you (permission prompt /
     *  question on its terminal) — shown as waiting, not spinning. */
    waiting?: boolean;
  }
  let { block, live = false, waiting = false }: Props = $props();
  const ctx = getContext<ConvContext>(CONV_CTX);

  let open = $state(false);
  // An over-cap live delta ships this result as a short preview
  // (`result.elided`); the full stored output is fetched the first time the
  // step is expanded (SA-04 — the view used to re-fetch the whole page).
  let full = $state<ToolResult | null>(null);
  let fullFor = $state<string | null>(null);
  let fullState = $state<'idle' | 'loading' | 'error'>('idle');
  const loaded = $derived(fullFor === block.id);
  const result = $derived(effectiveResult(block, loaded ? full : null));
  const previewOnly = $derived(isElided(block) && !loaded);
  function loadFull(): void {
    const sid = ctx.sessionId;
    if (!sid || fullState === 'loading') return;
    const id = block.id;
    fullState = 'loading';
    fetchToolResult(sid, id).then(
      (r) => {
        if (block.id !== id) return;
        fullFor = id;
        full = r;
        fullState = 'idle';
      },
      () => {
        if (block.id === id) fullState = 'error';
      },
    );
  }
  $effect(() => {
    if (open && previewOnly && fullState === 'idle') loadFull();
  });
  const chrome = $derived(TOOL_CHROME[block.tool] ?? TOOL_CHROME.other);
  const line = $derived(toolLine(block));
  const cwd = $derived(ctx.cwd ?? null);
  /** The folder / scope, relative to where the agent ran. */
  const detail = $derived(line.detail && cwd && line.detail === cwd.replace(/\/$/, '') ? '' : relPath(line.detail, cwd));
  const status = $derived(toolStatus(block, live));
  const isShell = $derived(block.tool === 'shell');
  const STATUS_LABEL = { ok: 'Succeeded', err: 'Failed', running: 'Running…', none: 'No result recorded' } as const;
  const blocked = $derived(waiting && status === 'running');

  // Edit calls carry `structuredPatch` on the result; older records (and a
  // failed edit) do not — synthesize a −old/+new hunk from the input so the
  // change is still shown as a diff, never as two blobs of text.
  const diff = $derived.by(() => {
    if (result?.patch) return patchToDiff(result.patch, result.file_path);
    const synth = editInputPatch(block);
    return synth ? patchToDiff(synth, result?.file_path ?? null) : null;
  });
  const hasDiff = $derived(!!diff && diff.files.some((f) => f.hunks.length));
  const diffStats = $derived.by(() => {
    if (!diff) return null;
    let add = 0;
    let del = 0;
    for (const f of diff.files) {
      add += f.added ?? 0;
      del += f.deleted ?? 0;
    }
    return add || del ? { add, del } : null;
  });
  /** A fresh Write has no patch — its content (from the input) is the change. */
  const writtenContent = $derived.by(() => {
    if (block.tool !== 'write' || hasDiff) return '';
    const c = (block.input as { content?: unknown } | null)?.content;
    return typeof c === 'string' ? c : '';
  });

  const filePath = $derived(
    result?.file_path ??
      (block.tool === 'read' || block.tool === 'edit' || block.tool === 'write'
        ? (line.detail ? `${line.detail}/${line.target}` : line.target) || null
        : null),
  );
  const text = $derived(result?.text ?? '');
  // The collapsed row's second line: a command's last output line (a preview
  // cut from the head would lie about the tail, so not for elided results).
  const tail = $derived(isShell && !previewOnly && status !== 'running' ? lastLine(text) : '');
  const command = $derived.by(() => {
    if (!isShell || block.input == null || typeof block.input !== 'object') return '';
    const c = (block.input as { command?: unknown; cmd?: unknown }).command ?? (block.input as { cmd?: unknown }).cmd;
    return typeof c === 'string' ? c : Array.isArray(c) ? c.map(String).join(' ') : '';
  });

  // Syntax colors: hljs loads lazily (stays out of the main bundle). Command
  // output is never auto-detected — a log coloured as code reads as wrong.
  let hlReady = $state(false);
  $effect(() => {
    if (!open) return;
    void ensureHljs().then(() => (hlReady = true));
  });
  const lang = $derived.by(() => {
    if (!hlReady || isShell) return null;
    const byPath = filePath ? langFromPath(filePath) : null;
    if (byPath) return byPath;
    return autoLang(text);
  });
  const lines = $derived(text ? text.split('\n') : []);
  // Long outputs render through the windowed list (uniform mono rows); short
  // ones as a plain <pre> so selection/copy stays natural.
  const windowed = $derived(lines.length > 400);
  const inputJson = $derived.by(() => {
    if (block.input == null) return '';
    try {
      return typeof block.input === 'string' ? block.input : JSON.stringify(block.input, null, 2);
    } catch {
      return String(block.input);
    }
  });
  let showInput = $state(false);
  // Plain output (commands, unhighlighted text) gets its paths and URLs linked.
  const outHtml = $derived(open && !windowed && text ? (lang ? highlightBlock(text, lang) : linkifyOutput(highlightBlock(text, null))) : '');
  function onOutClick(e: MouseEvent): void {
    const a = e.target instanceof Element ? e.target.closest('a') : null;
    if (!a) return;
    e.preventDefault();
    if (a.classList.contains('file-ref') && a.dataset.path) {
      const line = a.dataset.anchor ? Number(a.dataset.anchor.split(':')[0]) : null;
      ctx?.openPreview?.({ kind: 'file', path: a.dataset.path, line });
    } else if (a.getAttribute('href')) {
      const href = a.getAttribute('href') ?? '';
      if (ctx.openUrl) ctx.openUrl(href, e.altKey);
      else void openExternal(href);
    }
  }
  const inputHtml = $derived(open && showInput && inputJson ? highlightBlock(inputJson, hlReady ? 'json' : null) : '');
  // A Read's result is `cat -n` text ("    80\tpub fn …"): shown as source with
  // the file's own line numbers, like an editor, instead of a numbered blob.
  const readView = $derived.by(() => {
    if (!open || block.tool !== 'read' || !text) return null;
    const m = /^\s*(\d+)\t/.exec(text);
    if (!m) return null;
    const body: string[] = [];
    for (const l of lines) {
      const hit = /^\s*\d+\t/.exec(l);
      if (!hit) {
        if (l.trim() === '') continue;
        return null;
      }
      body.push(l.slice(hit[0].length));
    }
    return { start: Number(m[1]), text: body.join('\n') };
  });

  // A command's output opens at its END — the failure summary / final status
  // is what you look for first.
  let bodyEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    if (!open || !isShell || !text) return;
    void tick().then(() => {
      const el = bodyEl?.querySelector<HTMLElement>('.out, .out-vlist');
      if (el) el.scrollTop = el.scrollHeight;
    });
  });

  const canFiles = $derived(!!ctx.sessionId && !ctx.readonly);
  function openInFiles(): void {
    if (filePath) openFile.open(filePath);
  }
  /** The side panel: a write's content as written, an edit's file with this diff. */
  function openPreview(): void {
    if (!filePath) return;
    ctx?.openPreview?.({ kind: 'file', path: filePath, content: writtenContent || null, diff: hasDiff ? diff : null });
  }
  function openDiff(): void {
    if (!diff) return;
    ctx?.openPreview?.({ kind: 'diff', title: filePath ? (filePath.split('/').pop() ?? filePath) : 'Diff', diff, focus: null });
  }
  async function copyCommand(): Promise<void> {
    try {
      await navigator.clipboard.writeText(command);
      toasts.info('Copied', 'The command');
    } catch (e) {
      toasts.error('Copy failed', e instanceof Error ? e.message : String(e));
    }
  }
</script>

<div class="step" class:open data-tool={block.tool} data-status={status === 'running' ? 'pending' : status}>
  <button class="step-row" onclick={() => (open = !open)} aria-expanded={open} title={line.hint || block.title || block.name}>
    <span class="step-icon" aria-hidden="true"><Icon name={chrome.icon} size={13} /></span>
    <span class="step-text">
      <span class="step-main">
        <span class="step-verb">{blocked ? `Wants to ${line.base}` : status === 'running' ? line.present : line.verb}</span>
        {#if line.target}<span class="step-target" class:mono={line.mono}>{line.target}</span>{/if}
        {#if detail}<span class="step-detail mono">{detail}</span>{/if}
      </span>
      {#if tail}
        <span class="step-tail mono" class:err={status === 'err'}>{tail}</span>
      {:else if status === 'none'}
        <span class="step-tail">No result was recorded — the turn was interrupted</span>
      {/if}
    </span>
    {#if diffStats}
      <span class="step-stats mono" title="Lines added / removed"><span class="add">+{diffStats.add}</span> <span class="del">−{diffStats.del}</span></span>
    {/if}
    {#if result?.truncated || previewOnly}
      <span class="chip step-trunc" title={previewOnly ? 'Preview — expand to load the full output' : 'Output capped at 64 KB'}>{fmtBytes(result?.bytes ?? 0)}</span>
    {/if}
    <span class="step-status {status}" class:blocked role="img" aria-label={blocked ? 'Waiting for you' : STATUS_LABEL[status]} title={blocked ? 'Waiting for you' : STATUS_LABEL[status]}>
      {#if blocked}<Icon name="warning" size={12} />
      {:else if status === 'running'}<span class="spin"></span>
      {:else if status === 'ok'}<Icon name="check" size={12} />
      {:else if status === 'err'}<Icon name="x" size={12} />
      {:else}<Icon name="minus" size={12} />{/if}
    </span>
    <span class="step-caret" aria-hidden="true"><Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} /></span>
  </button>
  {#if open}
    <div class="step-body" bind:this={bodyEl}>
      {#if isShell && command}
        <div class="cmd">
          <span class="cmd-prompt mono" aria-hidden="true">$</span>
          <code class="cmd-text mono" dir="ltr">{command}</code>
          <button class="icon-btn cmd-copy" onclick={() => void copyCommand()} aria-label="Copy command" title="Copy command"><Icon name="copy" size={12} /></button>
        </div>
        {#if line.hint}<div class="cmd-hint">{line.hint}</div>{/if}
      {:else if filePath}
        <div class="file-line">
          <button class="file-chip mono" onclick={openPreview} title="Preview — {filePath}">
            <Icon name="eye" size={12} /> <span class="file-chip-path">{relPath(filePath, cwd)}</span>
          </button>
          {#if hasDiff}
            <button class="link-btn" onclick={openDiff} title="Open this change in the side panel">Open diff</button>
          {/if}
          {#if canFiles}
            <button class="icon-btn file-open" onclick={openInFiles} aria-label="Open in Files" title="Open in Files"><Icon name="folder" size={12} /></button>
          {/if}
        </div>
      {/if}
      {#if hasDiff && diff}
        <InlineDiff {diff} />
      {:else if writtenContent}
        <div class="code-frame"><CodeView text={writtenContent} lang={hlReady && filePath ? langFromPath(filePath) : null} /></div>
      {:else if result == null}
        <div class="pending">{status === 'running' ? 'Waiting for the result…' : 'No result was recorded for this call.'}</div>
      {:else if block.tool === 'web' || block.tool === 'ask'}
        <Markdown md={text} small />
      {:else if readView && !windowed}
        <div class="code-frame"><CodeView text={readView.text} start={readView.start} lang={lang} /></div>
      {:else if windowed}
        <VirtualList items={lines} estimateHeight={18} class="out-vlist" findText={(l: string) => l}>
          {#snippet row(l)}<div class="out-line mono hljs" dir="ltr">{@html highlightLine(l || ' ', lang)}</div>{/snippet}
        </VirtualList>
      {:else if text}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
        <pre class="out mono hljs" class:err={status === 'err'} class:shell={isShell} dir="ltr" data-lang={lang} onclick={onOutClick}>{@html outHtml}</pre>
      {:else if !result.image_ids.length && !previewOnly}
        <div class="pending">(no output)</div>
      {/if}
      {#if previewOnly}
        {#if fullState === 'error'}
          <div class="trunc-note load-err" role="alert">
            Couldn't load the full output.
            <button class="link-btn" onclick={loadFull}>Retry</button>
          </div>
        {:else if fullState === 'loading' || !ctx.sessionId}
          <div class="trunc-note" aria-live="polite">{ctx.sessionId ? 'Loading the full output…' : `Preview of ${fmtBytes(result?.bytes ?? 0)}.`}</div>
        {/if}
      {:else if result?.truncated}
        <div class="trunc-note">Output truncated to 64 KB ({fmtBytes(result.bytes)} total).</div>
      {/if}
      {#if result?.image_ids.length}
        <div class="imgs">
          {#each result.image_ids as id (id)}
            <ImageBlock {id} alt="Tool result image" small />
          {/each}
        </div>
      {/if}
      {#if inputJson && !isShell}
        <button class="link-btn" onclick={() => (showInput = !showInput)} aria-expanded={showInput}>{showInput ? 'Hide raw input' : 'Show raw input'}</button>
        {#if showInput}
          <pre class="out mono hljs" dir="ltr">{@html inputHtml}</pre>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .step {
    min-width: 0;
  }
  .step-row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    width: 100%;
    padding: 4px 8px;
    background: none;
    border: 0;
    border-radius: var(--radius-s);
    color: var(--text);
    cursor: pointer;
    text-align: start;
    font: inherit;
    min-width: 0;
  }
  .step-row:hover {
    background: var(--hover);
  }
  .step-row:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .step-icon {
    color: var(--text-dim);
    display: inline-flex;
    flex-shrink: 0;
    height: 18px;
    align-items: center;
  }
  .step-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .step-main {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: var(--fs-s);
    line-height: 18px;
    overflow: hidden;
    white-space: nowrap;
    min-width: 0;
  }
  .step-verb {
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .step-target {
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex-shrink: 1;
  }
  .step-target.mono {
    font-size: var(--fs-xs);
    direction: ltr;
    unicode-bidi: isolate;
  }
  .step-detail {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex-shrink: 4;
    direction: ltr;
    unicode-bidi: isolate;
  }
  .step-tail {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .step-tail.mono {
    direction: ltr;
    unicode-bidi: isolate;
    text-align: start;
  }
  .step-tail.err {
    color: var(--danger);
  }
  .step-trunc {
    height: 16px;
    font-size: var(--fs-xs);
    flex-shrink: 0;
  }
  .step-stats {
    font-size: var(--fs-xs);
    flex-shrink: 0;
    white-space: nowrap;
    line-height: 18px;
  }
  .step-stats .add {
    color: var(--success);
    font-weight: 600;
  }
  .step-stats .del {
    color: var(--danger);
    font-weight: 600;
  }
  .step-status {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 18px;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .step-status.ok {
    color: var(--success);
  }
  .step-status.err {
    color: var(--danger);
  }
  .step-status.blocked {
    color: var(--warning);
  }
  .spin {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 2px solid color-mix(in srgb, var(--accent) 25%, transparent);
    border-top-color: var(--accent);
  }
  @media (prefers-reduced-motion: no-preference) {
    .spin {
      animation: otto-spin 0.8s linear infinite;
    }
  }
  
  .step-caret {
    display: inline-flex;
    align-items: center;
    height: 18px;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .step-body {
    padding-block: 2px 10px; padding-inline: 29px 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .cmd {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding-block: 5px; padding-inline: 10px 4px;
    min-width: 0;
    direction: ltr;
  }
  .cmd-prompt {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    line-height: 18px;
    user-select: none;
  }
  .cmd-text {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    line-height: 18px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    text-align: start;
  }
  .cmd-copy {
    flex-shrink: 0;
    width: 20px;
    height: 20px;
  }
  .cmd-hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .code-frame {
    max-height: 360px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--code-bg, var(--surface-2));
  }
  .out {
    margin: 0;
    max-height: 320px;
    overflow: auto;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 8px 10px;
    font-size: var(--fs-xs);
    line-height: 18px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    text-align: start;
  }
  .out.shell {
    white-space: pre;
    overflow-wrap: normal;
  }
  .out.err {
    border-color: color-mix(in srgb, var(--danger) 45%, var(--border));
  }
  :global(.out-vlist) {
    max-height: 320px;
    overflow: auto;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 4px 0;
    direction: ltr;
  }
  .out-line {
    height: 18px;
    line-height: 18px;
    padding: 0 10px;
    font-size: var(--fs-xs);
    white-space: pre;
    /* Long lines widen the row so the windowed list scrolls horizontally
       (AGENTS.md: wide content scrolls inside its own container). */
    width: max-content;
    min-width: 100%;
    box-sizing: border-box;
  }
  /* hljs paints tokens only; the block keeps the chat's surface. */
  .out.hljs,
  .out-line.hljs {
    color: var(--text);
  }
  .file-line {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .file-line .link-btn {
    align-self: center;
    flex-shrink: 0;
  }
  .file-open {
    flex-shrink: 0;
    width: 22px;
    height: 22px;
  }
  .file-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    max-width: 100%;
    font-size: var(--fs-xs);
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    cursor: pointer;
    overflow: hidden;
    white-space: nowrap;
    direction: ltr;
  }
  /* The ellipsis lives on the TEXT span — on the inline-flex chip itself it
     never applied and long paths were cut mid-character. */
  .out :global(a.file-ref),
  .out :global(a.out-link) {
    color: var(--accent-text);
    text-decoration: underline dotted;
    text-underline-offset: 2px;
    cursor: pointer;
  }
  .file-chip-path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .file-chip:hover {
    color: var(--text);
    border-color: var(--border-strong);
  }
  .link-btn {
    align-self: flex-start;
    background: none;
    border: 0;
    padding: 0;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
    text-decoration: underline dotted;
  }
  .link-btn:hover {
    color: var(--text);
  }
  .pending {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .trunc-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .load-err {
    color: var(--danger);
    display: flex;
    gap: 8px;
    align-items: baseline;
  }
  .imgs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
</style>
