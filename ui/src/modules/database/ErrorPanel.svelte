<script lang="ts">
  // Structured query-error panel. `normalizeDbError` (error-normalize.ts) turns
  // the engine's error into a one-line headline (what failed), the likely cause
  // and a fix hint, clickable "did you mean" chips, and a statement position
  // for the caret excerpt. The full raw error stays one click away under "Show
  // full error" (with Copy), and "Ask AI to fix" — which opens the DB Assistant
  // seeded with the statement + the RAW error — is unchanged.
  import Icon from '../../lib/components/Icon.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { copyText } from '../../lib/clipboard';
  import { applySuggestion, normalizeDbError } from './error-normalize';

  interface Props {
    error: string;
    /** Active engine (mysql/postgres/clickhouse/mongodb/redis), or null outside
     *  the explorer (the Athena view). Picks the normaliser's rule table, and —
     *  being set only for an explorer tab — gates the chips that edit the
     *  explorer's statement. */
    engine: string | null;
    statement: string;
    /** Omitted where there is no DB Assistant to open (e.g. the Athena view). */
    onAskAi?: () => void;
  }
  let { error, engine, statement, onAskAi }: Props = $props();

  const n = $derived(normalizeDbError(engine, error, statement));

  // Policy refusals from the daemon read `forbidden: <code>: <why>`. They are
  // not query mistakes: say so, show the code as the chip, and don't offer an
  // AI "fix" for something only an administrator can change.
  const policy = $derived.by(() => {
    const m = error.match(/^\s*forbidden:\s*(?:([a-z0-9_]+):\s*)?([\s\S]*)$/i);
    if (!m) return null;
    const why = m[2].trim();
    return { code: m[1] ?? null, message: why ? why.charAt(0).toUpperCase() + why.slice(1) : error };
  });

  // The offending statement line + a caret under the reported column (or the
  // line's first non-space char when only a line is known). A long line is
  // windowed around the column so the caret stays on screen.
  const WINDOW = 60;
  const excerpt = $derived.by(() => {
    const pos = n.position;
    if (!pos || !statement.trim()) return null;
    const lines = statement.split('\n');
    if (pos.line < 1 || pos.line > lines.length) return null;
    const line = lines[pos.line - 1].replace(/\t/g, ' ');
    const chars = Array.from(line);
    let caretAt = pos.col ? Math.min(pos.col - 1, chars.length) : chars.length - line.trimStart().length;
    let shown = line;
    if (chars.length > WINDOW * 2) {
      const start = Math.max(0, caretAt - WINDOW);
      const end = Math.min(chars.length, caretAt + WINDOW);
      shown = `${start > 0 ? '…' : ''}${chars.slice(start, end).join('')}${end < chars.length ? '…' : ''}`;
      caretAt = caretAt - start + (start > 0 ? 1 : 0);
    }
    return {
      label: pos.col ? `line ${pos.line}, col ${pos.col}` : `line ${pos.line}`,
      text: `${shown}\n${' '.repeat(Math.max(0, caretAt))}^`,
    };
  });

  const label = $derived(engine ? engine.toUpperCase() : 'SQL');
  // The chips edit the explorer's live statement — only inside the explorer.
  const canApply = $derived(!!engine && !!database.tab);

  function applyFix(suggestion: string) {
    const tab = database.tab;
    if (!tab || !n.token) return;
    const next = applySuggestion(tab.statement, n.token, suggestion);
    if (next === null) {
      toasts.info('Nothing to replace', `\`${n.token}\` isn’t in the editor any more.`);
      return;
    }
    // One statement change — the editor applies it as an undoable edit (⌘Z).
    database.setStatement(next);
    toasts.success(`Replaced \`${n.token}\` with \`${suggestion}\``, 'Run the query again to check. ⌘Z undoes it.');
  }

  async function copyRaw() {
    if (await copyText(error)) toasts.success('Error copied');
    else toasts.error('Couldn’t copy');
  }
</script>

<div class="err-panel">
  <div class="err-head" role="alert">
    <span class="err-icon"><Icon name="warning" size={14} /></span>
    <span class="err-title">{policy ? 'Not allowed on this connection' : n.title}</span>
    {#if policy?.code}<span class="err-code mono">{policy.code}</span>{:else if !policy && n.code}<span class="err-code mono">{n.code}</span>{/if}
    <span class="err-engine mono">{label}</span>
    <span class="err-grow"></span>
    {#if onAskAi && !policy}
      <button class="btn small" onclick={onAskAi} title="Open the DB Assistant to investigate and fix this error">
        <Icon name="zap" size={12} /> Ask Otto to fix
      </button>
    {/if}
  </div>
  {#if policy}
    <p class="err-policy">{policy.message}</p>
    <p class="err-hint">An administrator decides what this connection may run: in the Connections list, open its ⋯ menu and choose Access.</p>
  {:else}
    {#if n.cause}<p class="err-cause">{n.cause}</p>{/if}
    {#if n.hint}
      <p class="err-hint"><span class="err-hint-icon"><Icon name="bulb" size={12} /></span>{n.hint}</p>
    {/if}
    {#if n.suggestions.length > 0}
      <div class="err-suggest">
        <span class="err-suggest-label">Did you mean</span>
        {#each n.suggestions as s (s)}
          {#if canApply}
            <button
              class="err-chip mono"
              onclick={() => applyFix(s)}
              title={`Replace \`${n.token}\` with \`${s}\` in the editor`}
              aria-label={`Replace ${n.token} with ${s} in the editor`}
            >{s}</button>
          {:else}
            <span class="err-chip static mono">{s}</span>
          {/if}
        {/each}
      </div>
    {/if}
    {#if excerpt}
      <div class="err-excerpt">
        <div class="err-excerpt-label mono">{excerpt.label}</div>
        <pre class="err-excerpt-code mono">{excerpt.text}</pre>
      </div>
    {/if}
    <details class="err-raw">
      <summary>Show full error</summary>
      <div class="err-raw-body">
        <pre class="err-msg mono">{error}</pre>
        <button class="btn small err-copy" onclick={copyRaw} title="Copy the full error text" aria-label="Copy the full error text">
          <Icon name="copy" size={12} /> Copy
        </button>
      </div>
    </details>
  {/if}
</div>

<style>
  .err-panel {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 14px;
    overflow: auto;
    min-height: 0;
  }
  .err-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    min-width: 0;
    color: var(--text);
  }
  .err-icon {
    display: inline-flex;
    color: var(--danger);
    flex-shrink: 0;
  }
  .err-title {
    font-size: var(--fs-m);
    font-weight: 600;
    min-width: 0;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .err-code {
    font-size: var(--fs-xs);
    padding: 1px 6px;
    border-radius: 999px;
    color: var(--danger);
    background: var(--danger-soft);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .err-engine {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    background: var(--surface-2);
    padding: 1px 6px;
    border-radius: 999px;
  }
  .err-grow {
    flex: 1;
  }
  .err-cause {
    margin: 0;
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text);
    overflow-wrap: anywhere;
    user-select: text;
  }
  .err-policy {
    margin: 0;
    font-size: var(--fs-m);
    line-height: 1.5;
    color: var(--text);
    user-select: text;
  }
  .err-hint {
    margin: 0;
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text-dim);
    user-select: text;
  }
  .err-hint-icon {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--warning);
    transform: translateY(1px);
  }
  .err-suggest {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .err-suggest-label {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .err-chip {
    font-size: var(--fs-s);
    padding: 1px 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--accent-soft);
    color: var(--accent-text);
    cursor: pointer;
  }
  .err-chip:hover {
    border-color: var(--accent-solid);
  }
  .err-chip:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .err-chip.static {
    cursor: default;
  }
  .err-msg {
    margin: 0;
    font-size: var(--fs-s);
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--text);
    user-select: text;
    max-height: 320px;
    overflow: auto;
  }
  .err-excerpt {
    border-top: 1px solid var(--border);
    padding-top: 6px;
  }
  .err-excerpt-label {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    margin-bottom: 2px;
  }
  .err-excerpt-code {
    margin: 0;
    font-size: var(--fs-s);
    line-height: 1.4;
    color: var(--text);
    white-space: pre;
    overflow-x: auto;
  }
  .err-raw {
    border-top: 1px solid var(--border);
    padding-top: 6px;
  }
  .err-raw summary {
    font-size: var(--fs-s);
    color: var(--text-dim);
    cursor: pointer;
    width: fit-content;
  }
  .err-raw summary:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  .err-raw-body {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    margin-block-start: 6px;
  }
</style>
