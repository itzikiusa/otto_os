<script lang="ts">
  // What the agent is doing RIGHT NOW, at the foot of the chat:
  //   • working → one quiet line: spinner · "Claude is working" · the current
  //     step ("Running cargo test …") · elapsed since your last message;
  //   • waiting → a card: the agent stopped with a tool call that never got a
  //     result — in Claude Code / Codex that is a permission prompt (or an
  //     AskUserQuestion) on the terminal screen. Otto can't answer those for
  //     you from here (the transcript doesn't carry the prompt's options), so
  //     the card says what is being asked and opens the terminal to answer.
  import Icon from '../../../lib/components/Icon.svelte';
  import { now } from '../../../lib/stores/now.svelte';
  import { askQuestions, fmtDuration, toolLine, type ToolCallBlock } from './format';

  interface Props {
    mode: 'working' | 'waiting';
    agentName: string;
    /** The newest call without a result (the current step / what is asked). */
    pending: ToolCallBlock | null;
    /** The in-progress response text is streaming (shown above this line). */
    writing?: boolean;
    /** Timestamp of your last message — the turn's elapsed time. */
    since?: string | null;
    /** Switch this pane to the terminal (null in read-only views). */
    onterminal?: (() => void) | null;
    /** The events socket is down: "working" is only the last known state, so
     *  the line says "Reconnecting…" — no spinner, no elapsed clock. */
    stale?: boolean;
    /** Tooltip for the stale line (lib/status `sessionState(...).hint`). */
    staleHint?: string;
  }
  let { mode, agentName, pending, writing = false, since = null, onterminal = null, stale = false, staleHint }: Props = $props();

  const line = $derived(pending ? toolLine(pending) : null);
  const doing = $derived(
    line ? `${line.present}${line.target ? ` ${line.target}` : ''}` : writing ? 'Writing the response' : 'Thinking',
  );
  const elapsed = $derived.by(() => {
    if (!since) return '';
    const t = Date.parse(since);
    if (!Number.isFinite(t)) return '';
    const ms = now() - t;
    // Past a few hours the "last message" is not this turn's start (a resumed
    // session, a clock skew) — say nothing rather than something wrong.
    return ms > 0 && ms < 3 * 3600_000 ? fmtDuration(Math.floor(ms / 1000) * 1000) : '';
  });
  const questions = $derived(pending?.tool === 'ask' ? askQuestions(pending.input) : []);
</script>

{#if mode === 'working' && stale}
  <div class="live-line stale" data-live-status="stale" role="status" title={staleHint}>
    <Icon name="refresh" size={12} />
    <span class="who">Reconnecting…</span>
    <span class="doing"><span class="sep" aria-hidden="true">·</span> {agentName} was working when live updates paused</span>
  </div>
{:else if mode === 'working'}
  <div class="live-line" data-live-status="working" role="status">
    <span class="spinner" style:--spinner-size="11px" aria-hidden="true"></span>
    <span class="who">{agentName} is working</span>
    <span class="doing" title={line?.hint || doing}><span class="sep" aria-hidden="true">·</span> <span class:mono={!!line?.mono}>{doing}</span></span>
    {#if elapsed}<span class="elapsed" aria-hidden="true">{elapsed}</span>{/if}
  </div>
{:else}
  <div class="waiting" data-live-status="waiting" role="status">
    <div class="w-head">
      <Icon name="warning" size={14} />
      <span class="w-title">{agentName} is waiting for you</span>
    </div>
    {#if questions.length}
      {#each questions as q, i (i)}
        <p class="w-q">{q.question}</p>
        {#if q.options.length}
          <ul class="w-opts" aria-label="Options">
            {#each q.options as o, j (j)}<li>{o}</li>{/each}
          </ul>
        {/if}
      {/each}
      <p class="w-body">Pick an answer in the terminal — the chat can’t send a choice for you.</p>
    {:else if line}
      <p class="w-body">
        {agentName} wants to {line.base}
        <code class="mono w-target">{line.target || pending?.name}</code>. That is usually a permission prompt — allow or
        deny it in the terminal.
      </p>
    {/if}
    {#if onterminal}
      <div class="w-actions">
        <button class="btn small" onclick={onterminal}><Icon name="terminal" size={12} /> Open terminal</button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .live-line {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-block: 6px;
    padding-inline: 8px;
    margin-inline-start: 10px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    min-width: 0;
  }
  .who {
    color: var(--text);
    font-weight: 500;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .doing {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .doing .mono {
    font-size: var(--fs-xs);
  }
  .sep {
    opacity: 0.6;
  }
  .elapsed {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  
  /* "Needs you" is the one amber state (patterns §1). */
  .waiting {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-inline-start: 10px;
    padding: 10px 12px;
    border-radius: var(--radius-m);
    background: var(--warning-soft);
    border: 1px solid color-mix(in srgb, var(--warning) 45%, var(--border));
    font-size: var(--fs-s);
    min-width: 0;
  }
  .w-head {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--warning);
  }
  .w-title {
    font-weight: 600;
    color: var(--text);
  }
  .w-body,
  .w-q {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .w-q {
    font-weight: 500;
  }
  .w-body {
    color: var(--text-dim);
  }
  .w-target {
    font-size: var(--fs-xs);
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    padding: 0 4px;
    border-radius: var(--radius-s);
    color: var(--text);
    direction: ltr;
    unicode-bidi: isolate;
  }
  .w-opts {
    margin: 0;
    padding-inline-start: 1.4em;
    color: var(--text);
  }
  .w-actions {
    display: flex;
    gap: 6px;
  }
</style>
