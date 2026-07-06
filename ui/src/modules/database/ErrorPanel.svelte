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
    onAskAi: () => void;
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
  <div class="err-head">
    <Icon name="x" size={13} />
    <span class="err-title">Query failed</span>
    {#if parsed.code}<span class="err-code mono">{parsed.code}</span>{/if}
    <span class="err-engine mono">{label}</span>
    <span class="err-grow"></span>
    <button class="err-ai" onclick={onAskAi} title="Open the DB Assistant to investigate and fix this error">
      <Icon name="zap" size={12} /> Ask AI to fix
    </button>
  </div>
  <pre class="err-msg mono">{error}</pre>
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
    color: var(--status-exited);
  }
  .err-title {
    font-size: 12.5px;
    font-weight: 600;
  }
  .err-code {
    font-size: 11px;
    padding: 1px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--status-exited) 18%, transparent);
    border: 1px solid color-mix(in srgb, var(--status-exited) 40%, transparent);
  }
  .err-engine {
    font-size: 10px;
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
  .err-ai {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: 1px solid color-mix(in srgb, var(--accent) 45%, var(--border));
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
    border-radius: var(--radius-s);
    font-size: 11.5px;
    padding: 3px 9px;
    cursor: pointer;
    flex-shrink: 0;
  }
  .err-ai:hover {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .err-msg {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--status-exited);
    user-select: text;
  }
  .err-excerpt {
    border-top: 1px solid color-mix(in srgb, var(--status-exited) 25%, transparent);
    padding-top: 6px;
  }
  .err-excerpt-label {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    margin-bottom: 2px;
  }
  .err-excerpt-code {
    margin: 0;
    font-size: 12px;
    line-height: 1.4;
    color: var(--text);
    white-space: pre;
    overflow-x: auto;
  }
</style>
