<script lang="ts">
  // Structured query-error panel: replaces the raw mono error block. Parses the
  // engine's error string into a code chip and (when the engine reports a line)
  // a focused statement excerpt with a caret, and offers "Ask AI to fix" — which
  // opens the DB Assistant in investigate mode seeded with the statement + error.
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    error: string;
    /** Active engine (mysql/postgres/clickhouse/mongodb/redis) — used for the
     *  label; parsing is pattern-based so it works for all of them. */
    engine: string | null;
    statement: string;
    /** Omitted where there is no DB Assistant to open (e.g. the Athena view). */
    onAskAi?: () => void;
  }
  let { error, engine, statement, onAskAi }: Props = $props();

  // Parse a code chip + (when derivable) the 1-based line the error points at.
  //   MySQL:      ERROR 1064 (42000): … at line 2
  //   ClickHouse: Code: 62. DB::Exception: …
  //   Postgres:   ERROR: syntax error …  \n  LINE 3: …
  const parsed = $derived.by(() => {
    const my = error.match(/ERROR\s+(\d+)\s*\(([0-9A-Za-z]+)\)/);
    const ch = error.match(/\bCode:\s*(\d+)/);
    const pgLine = error.match(/\bLINE\s+(\d+)\s*:/);
    const myLine = error.match(/\bat line\s+(\d+)/i);
    let code: string | null = null;
    if (my) code = `Error ${my[1]}${my[2] ? ` · ${my[2]}` : ''}`;
    else if (ch) code = `Code ${ch[1]}`;
    const lineNo = pgLine ? Number(pgLine[1]) : myLine ? Number(myLine[1]) : null;
    return { code, lineNo };
  });

  // Policy refusals from the daemon read `forbidden: <code>: <why>`. They are
  // not query mistakes: say so, show the code as the chip, and don't offer an
  // AI "fix" for something only an administrator can change.
  const policy = $derived.by(() => {
    const m = error.match(/^\s*forbidden:\s*(?:([a-z0-9_]+):\s*)?([\s\S]*)$/i);
    if (!m) return null;
    const why = m[2].trim();
    return { code: m[1] ?? null, message: why ? why.charAt(0).toUpperCase() + why.slice(1) : error };
  });

  // The offending statement line + a caret under its first non-space char.
  const excerpt = $derived.by(() => {
    const n = parsed.lineNo;
    if (!n || !statement.trim()) return null;
    const lines = statement.split('\n');
    if (n < 1 || n > lines.length) return null;
    const line = lines[n - 1];
    const indent = line.length - line.trimStart().length;
    return { n, text: `${line}\n${' '.repeat(Math.max(0, indent))}^` };
  });

  const label = $derived(engine ? engine.toUpperCase() : 'SQL');
</script>

<div class="err-panel">
  <div class="err-head" role="alert">
    <span class="err-icon"><Icon name="warning" size={14} /></span>
    <span class="err-title">{policy ? 'Not allowed on this connection' : 'Query failed'}</span>
    {#if policy?.code}<span class="err-code mono">{policy.code}</span>{:else if parsed.code}<span class="err-code mono">{parsed.code}</span>{/if}
    <span class="err-engine mono">{label}</span>
    <span class="err-grow"></span>
    {#if onAskAi && !policy}
      <button class="btn small" onclick={onAskAi} title="Open the DB Assistant to investigate and fix this error">
        <Icon name="zap" size={12} /> Ask AI to fix
      </button>
    {/if}
  </div>
  {#if policy}<p class="err-policy">{policy.message}</p>{:else}<pre class="err-msg mono">{error}</pre>{/if}
  {#if policy}
    <p class="err-hint">An administrator decides what this connection may run: in the Connections list, open its ⋯ menu and choose Access.</p>
  {/if}
  {#if excerpt}
    <div class="err-excerpt">
      <div class="err-excerpt-label mono">line {excerpt.n}</div>
      <pre class="err-excerpt-code mono">{excerpt.text}</pre>
    </div>
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
  }
  .err-code {
    font-size: var(--fs-xs);
    padding: 1px 7px;
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
    letter-spacing: 0.04em;
    color: var(--text-dim);
    background: var(--surface-2);
    padding: 1px 7px;
    border-radius: 999px;
  }
  .err-grow {
    flex: 1;
  }
  .err-msg {
    margin: 0;
    font-size: var(--fs-s);
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--text);
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
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .err-excerpt {
    border-top: 1px solid var(--border);
    padding-top: 6px;
  }
  .err-excerpt-label {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
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
</style>
