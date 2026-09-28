<script lang="ts">
  // One render item of the chat.
  //   • You: a right-aligned, neutral bubble.
  //   • The agent: an attributed response — provider mark + name + time — then
  //     prose, tool-activity groups, the plan, images and chips, hanging off a
  //     2 px inline-start rule (guidelines patterns §2: agent content is
  //     attributed and distinct, but never louder than yours).
  // Copy, the exact time, duration and model sit in a quiet action row that
  // appears on hover / focus (always on touch). Per-turn system notes stay a
  // chip that expands in place; "Show system" reveals them all.
  import { getContext } from 'svelte';
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import Markdown from './Markdown.svelte';
  import WorkSteps from './WorkSteps.svelte';
  import TasksBlock from './TasksBlock.svelte';
  import ImageBlock from './ImageBlock.svelte';
  import { transcript } from '../../../lib/stores/transcript.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { fmtClock, fmtDuration, providerName, segment, type RenderItem } from './format';
  import type { Artifact, SystemNote } from '../../../lib/api/types';
  import { CONV_CTX, type ConvContext } from './context';

  interface Props {
    item: RenderItem;
    /** True for the newest response while the agent works on it (step groups start open). */
    live?: boolean;
    /** Newest response of a session that is still alive: calls without a
     *  result are in progress / waiting on you, not interrupted. */
    active?: boolean;
    /** Stopped on its open call, waiting for you (permission / question). */
    waiting?: boolean;
    /** Inside a subagent card — no attribution header, tighter chrome. */
    nested?: boolean;
    /** Matches the conversation search (soft highlight) / is the current match. */
    hit?: boolean;
    current?: boolean;
  }
  let { item, live = false, active = live, waiting = false, nested = false, hit = false, current = false }: Props = $props();

  // Copy: the turn's prose as markdown; tool steps as one-line summaries so a
  // pasted response still reads. Images/chips are skipped.
  const copyText = $derived.by(() => {
    const parts: string[] = [];
    for (const b of item.blocks) {
      if (b.kind === 'text') parts.push(b.md.trim());
      else if (b.kind === 'tool_call') parts.push(`[${b.name}] ${b.title}`.trim());
      else if (b.kind === 'queued' && b.op === 'enqueue') parts.push(`Queued: ${b.text}`);
    }
    return parts.filter(Boolean).join('\n\n');
  });
  async function copyTurn(): Promise<void> {
    try {
      await navigator.clipboard.writeText(copyText);
      toasts.info('Copied', item.role === 'user' ? 'Your message' : 'The response');
    } catch (e) {
      toasts.error('Copy failed', e instanceof Error ? e.message : String(e));
    }
  }
  const ctx = getContext<ConvContext>(CONV_CTX);

  const segs = $derived(segment(item.blocks));
  const showSystem = $derived(transcript.showSystem);
  // Codex reasoning is dropped by the parser and counted per turn
  // (`Turn.reasoning_steps`) → one "N reasoning steps (not recorded)" note per response.
  const isCodex = $derived(ctx.provider === 'codex');
  const agentName = $derived(providerName(ctx.provider));
  let sysOpen = $state(false);
  const sysNotes = $derived<SystemNote[]>(item.system);
  const visibleBlocks = $derived(
    segs.filter((s) => {
      if (s.kind !== 'block') return true;
      const b = s.block;
      if (b.kind === 'queued') return b.op === 'enqueue' && ctx.queuedLive.includes(b.text) && (showSystem || !b.injected);
      if (b.kind === 'notice') return showSystem;
      return true;
    }),
  );
  const firstStepsIdx = $derived(visibleBlocks.findIndex((s) => s.kind === 'steps'));
  // Only the response's LAST plan snapshot starts open; earlier ones fold.
  const lastTasksIdx = $derived(visibleBlocks.findLastIndex((s) => s.kind === 'block' && s.block.kind === 'tasks'));
  const clock = $derived(fmtClock(item.ts));
  const fullTime = $derived(item.ts ? new Date(item.ts).toLocaleString() : '');

  function artifactIcon(a: Artifact): IconName {
    return a.kind === 'pr' ? 'pr' : a.kind === 'image' ? 'image' : a.kind === 'url' ? 'link' : a.kind === 'report' ? 'note' : 'file';
  }
</script>

{#snippet pendingChip(text: string)}
  <div class="pending" data-pending title="Sent while the agent was busy — the CLI delivers it when the current turn ends">
    <Icon name="clock" size={12} />
    <span class="pending-label">Queued</span>
    <span class="pending-text">{text}</span>
  </div>
{/snippet}

{#snippet sysChip()}
  {#if sysNotes.length}
    <button class="sys-chip" class:on={sysOpen} onclick={() => (sysOpen = !sysOpen)} aria-expanded={sysOpen || showSystem} title="System notes attached to this turn (reminders, hooks, attachments)">
      <Icon name="info" size={11} /> {sysNotes.length} system
    </button>
  {/if}
{/snippet}

<article
  class="turn {item.role}"
  class:nested
  class:hit
  class:current
  data-turn-id={item.id}
  data-role={item.role}
  aria-label={item.role === 'user' ? `You${clock ? `, ${clock}` : ''}` : `${agentName}${clock ? `, ${clock}` : ''}`}
>
  {#if item.role === 'user'}
    <div class="bubble" dir="auto">
      {#each item.blocks as b, i (i)}
        {#if b.kind === 'text'}
          <Markdown md={b.md} />
        {:else if b.kind === 'image'}
          <ImageBlock id={b.id} alt={b.alt} mediaType={b.media_type} />
        {:else if b.kind === 'queued' && b.op === 'enqueue' && ctx.queuedLive.includes(b.text) && (showSystem || !b.injected)}
          {@render pendingChip(b.text)}
        {:else if b.kind === 'notice' && showSystem}
          <span class="chip note" title={b.note.body ?? ''}>{b.note.title}</span>
        {/if}
      {/each}
    </div>
    <div class="meta">
      {#if clock}<time class="ts" datetime={item.ts ?? undefined} title={fullTime}>{clock}</time>{/if}
      {#if copyText}
        <button class="copy-btn" onclick={() => void copyTurn()} title="Copy message" aria-label="Copy message"><Icon name="copy" size={12} /></button>
      {/if}
      {@render sysChip()}
    </div>
  {:else}
    {#if !nested}
      <header class="agent-head">
        <ProviderIcon provider={ctx.provider} size={14} />
        <span class="agent-name">{agentName}</span>
        {#if clock}<time class="ts" datetime={item.ts ?? undefined} title={fullTime}>{clock}</time>{/if}
      </header>
    {/if}
    <div class="resp">
      {#each visibleBlocks as s, i (i)}
        {#if s.kind === 'steps'}
          <WorkSteps steps={s.steps} durationMs={i === firstStepsIdx ? item.duration_ms : null} {live} {active} {waiting} />
        {:else if s.block.kind === 'text'}
          <Markdown md={s.block.md} />
        {:else if s.block.kind === 'tasks'}
          <TasksBlock tasks={s.block.tasks} folded={i !== lastTasksIdx} />
        {:else if s.block.kind === 'image'}
          <ImageBlock id={s.block.id} alt={s.block.alt} mediaType={s.block.media_type} />
        {:else if s.block.kind === 'queued'}
          {@render pendingChip(s.block.text)}
        {:else if s.block.kind === 'artifact'}
          {@const a = s.block.artifact}
          {#if a.url}
            <a class="chip artifact" href={a.url} target="_blank" rel="noopener noreferrer" title={a.path ?? a.url}>
              <Icon name={artifactIcon(a)} size={11} /> {a.label}
            </a>
          {:else}
            <span class="chip artifact" title={a.path ?? ''}><Icon name={artifactIcon(a)} size={11} /> {a.label}</span>
          {/if}
        {:else if s.block.kind === 'notice'}
          <div class="notice" title={s.block.note.kind}>
            <Icon name="info" size={11} /> <strong>{s.block.note.title}</strong>
            {#if s.block.note.body}<span class="dim"> — {s.block.note.body.slice(0, 400)}</span>{/if}
          </div>
        {/if}
      {/each}
      {#if !visibleBlocks.length}
        <div class="dim empty">(no visible content{sysNotes.length ? ' — system notes only' : ''})</div>
      {/if}
    </div>
    <div class="meta">
      {#if copyText}
        <button class="copy-btn" onclick={() => void copyTurn()} title="Copy response" aria-label="Copy response"><Icon name="copy" size={12} /></button>
      {/if}
      {#if nested && clock}<time class="ts" datetime={item.ts ?? undefined} title={fullTime}>{clock}</time>{/if}
      {#if item.duration_ms != null}<span class="dim" title="How long the agent worked on this response">{fmtDuration(item.duration_ms)}</span>{/if}
      {#if item.model}<span class="dim mono model">{item.model}</span>{/if}
      {#if isCodex && item.reasoning_steps > 0}
        <span class="dim" title="Codex does not persist reasoning text">{item.reasoning_steps} reasoning steps (not recorded)</span>
      {/if}
      {@render sysChip()}
    </div>
  {/if}
  {#if sysNotes.length && (sysOpen || showSystem)}
    <div class="sys-list" data-system-notes={sysNotes.length}>
      {#each sysNotes as n, i (i)}
        <details class="sys-note">
          <summary><span class="sys-kind mono">{n.kind}</span> {n.title}</summary>
          {#if n.body}<pre class="mono" dir="ltr">{n.body.length > 4000 ? n.body.slice(0, 4000) + '\n…' : n.body}</pre>{/if}
        </details>
      {/each}
    </div>
  {/if}
</article>

<style>
  .turn {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 0;
    min-width: 0;
    scroll-margin-block: 16px;
  }
  .turn.nested {
    padding: 4px 6px;
  }
  .turn.user {
    align-items: flex-end;
    padding-block: 12px 4px;
  }
  /* You: a neutral bubble on the trailing side (accent means "selected" in
     Otto, so it is not used for identity). */
  .bubble {
    max-width: min(85%, 640px);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 16px 16px 4px 16px;
    padding: 8px 14px;
    min-width: 0;
  }
  :global([dir='rtl']) .bubble {
    border-radius: 16px 16px 16px 4px;
  }
  /* The agent: attribution line + a response hanging off a hairline rule. */
  .agent-head {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .agent-name {
    font-weight: 600;
  }
  .agent-head .ts {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .resp {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    border-inline-start: 2px solid var(--border-strong);
    padding-inline-start: 12px;
    margin-inline-start: 6px;
  }
  .turn.nested .resp {
    border: 0;
    padding: 0;
    margin: 0;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 20px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex-wrap: wrap;
    padding-inline-start: 20px;
  }
  .turn.user .meta {
    justify-content: flex-end;
    padding-inline-start: 0;
  }
  /* Quiet until wanted: the action row fades in on hover / keyboard focus
     (always shown on touch, and while its system notes are open). */
  .meta > :global(*) {
    opacity: 0;
    transition: opacity 120ms;
  }
  .turn:hover .meta > :global(*),
  .turn:focus-within .meta > :global(*),
  .meta > :global(.sys-chip) {
    opacity: 1;
  }
  @media (hover: none) {
    .meta > :global(*) {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .meta > :global(*) {
      transition: none;
    }
  }
  .copy-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 20px;
    background: none;
    border: 0;
    border-radius: var(--radius-s);
    padding: 0;
    color: var(--text-dim);
    cursor: pointer;
  }
  .copy-btn:hover {
    color: var(--text);
    background: var(--hover);
  }
  .copy-btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .turn.hit .bubble,
  .turn.hit .resp {
    box-shadow: 0 0 0 2px var(--warning-soft);
    border-radius: var(--radius-m);
  }
  .turn.current .bubble,
  .turn.current .resp {
    box-shadow: 0 0 0 2px var(--status-warn);
    border-radius: var(--radius-m);
  }
  .model {
    font-size: var(--fs-xs);
  }
  .sys-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: none;
    border: 1px solid var(--border);
    border-radius: 99px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    padding: 0 7px;
    height: 18px;
    cursor: pointer;
  }
  .sys-chip:hover,
  .sys-chip.on {
    color: var(--text);
    border-color: var(--border-strong);
  }
  .sys-list {
    display: flex;
    flex-direction: column;
    gap: 3px;
    width: 100%;
  }
  .turn.user .sys-list {
    align-self: flex-end;
    max-width: min(85%, 640px);
  }
  .sys-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    padding: 3px 8px;
  }
  .sys-note summary {
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sys-kind {
    font-size: var(--fs-xs);
    opacity: 0.8;
  }
  .sys-note pre {
    margin: 4px 0 2px;
    font-size: var(--fs-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 260px;
    overflow: auto;
    text-align: start;
  }
  .pending {
    display: flex;
    align-items: baseline;
    gap: 8px;
    align-self: stretch;
    padding: 6px 10px;
    border-radius: var(--radius-m);
    background: var(--warning-soft);
    border: 1px dashed color-mix(in srgb, var(--warning) 55%, var(--border));
    color: var(--text);
    font-size: var(--fs-s);
    min-width: 0;
  }
  .pending :global(svg) {
    color: var(--warning);
    align-self: center;
    flex-shrink: 0;
  }
  .pending-label {
    font-weight: 600;
    color: var(--warning);
    white-space: nowrap;
  }
  .pending-text {
    overflow-wrap: anywhere;
    min-width: 0;
  }
  .chip.note {
    align-self: flex-start;
    color: var(--text-dim);
  }
  .chip.artifact {
    align-self: flex-start;
    gap: 5px;
    text-decoration: none;
    color: var(--text);
    cursor: pointer;
  }
  a.chip.artifact:hover {
    border-color: var(--border-strong);
  }
  .notice {
    display: flex;
    align-items: baseline;
    gap: 5px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 4px 8px;
    border-inline-start: 2px solid var(--border);
    overflow-wrap: anywhere;
  }
  .empty {
    font-size: var(--fs-s);
  }
</style>
