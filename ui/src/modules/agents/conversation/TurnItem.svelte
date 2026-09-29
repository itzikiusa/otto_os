<script lang="ts">
  // One render item of the chat — two speakers, told apart at a glance:
  //   • You: a right-aligned BLUE bubble (accent tint) with the time, Copy and
  //     "Edit & resend" under it.
  //   • The agent: a GREEN identity — its mark in a green disc + name + model +
  //     time — and the response on a faint green surface hanging off a green
  //     rail. A settled response keeps its FIRST and FINAL message visible and
  //     folds the work between them (tool groups, narration, plan snapshots)
  //     into "Worked for 2m 7s · 7 steps ›" (t3code); a live one stays flat.
  //     Under it: the changed-files card and a footer with duration, model and
  //     where the tokens went (in · thinking · out · cache).
  // Thinking is its own violet, italic marker — never mistaken for the answer.
  // Per-turn system notes stay a chip that expands in place.
  import { getContext } from 'svelte';
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import Markdown from './Markdown.svelte';
  import WorkSteps from './WorkSteps.svelte';
  import TasksBlock from './TasksBlock.svelte';
  import ImageBlock from './ImageBlock.svelte';
  import ThinkingMarker from './ThinkingMarker.svelte';
  import UsageLine from './UsageLine.svelte';
  import ChangedFiles from './ChangedFiles.svelte';
  import { transcript } from '../../../lib/stores/transcript.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { changedFiles, fmtClock, fmtDuration, foldResponse, foldStats, providerName, segment, type RenderItem, type Segment } from './format';
  import type { Artifact, SystemNote } from '../../../lib/api/types';
  import { CONV_CTX, type ConvContext } from './context';

  interface Props {
    item: RenderItem;
    /** True for the newest response while the agent works on it (flat, step groups start open). */
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
  /** Your message's own words (Edit & resend puts them back in the composer). */
  const promptText = $derived(
    item.role === 'user' ? item.blocks.flatMap((b) => (b.kind === 'text' ? [b.md] : [])).join('\n\n').trim() : '',
  );
  // The composer sends attached images as `[Image: <path>]` lines after the
  // text; the bubble shows them as attachment chips, not as raw markers.
  const IMAGE_LINE = /^\[Image: (.+)\]\s*$/gm;
  function splitAttachments(md: string): { md: string; images: string[] } {
    const images: string[] = [];
    const rest = md.replace(IMAGE_LINE, (_m, p: string) => {
      images.push(p.trim());
      return '';
    });
    return { md: images.length ? rest.trim() : md, images };
  }
  async function copyPath(path: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(path);
      toasts.info('Path copied', path);
    } catch (e) {
      toasts.error('Copy failed', e instanceof Error ? e.message : String(e));
    }
  }
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
  // Live / waiting responses stay flat: the reader follows the work as it happens.
  const fold = $derived(nested || live || waiting ? null : foldResponse(visibleBlocks));
  let foldOpen = $state(false);
  const foldShown = $derived(!!fold && (foldOpen || !!ctx.expandAll || current || ctx.revealId === item.id));
  const stats = $derived(fold ? foldStats(fold.body) : null);
  const firstStepsIdx = $derived(visibleBlocks.findIndex((s) => s.kind === 'steps'));
  // Only the response's LAST plan snapshot starts open; earlier ones fold.
  const lastTasks = $derived(visibleBlocks.findLast((s) => s.kind === 'block' && s.block.kind === 'tasks') ?? null);
  // The first thinking marker carries the response's thinking-token count.
  const firstThink = $derived(visibleBlocks.find((s) => s.kind === 'steps' && s.steps.every((b) => b.kind === 'thinking')) ?? null);
  const changed = $derived(item.role === 'assistant' && !nested && !live ? changedFiles(item.blocks) : []);
  const clock = $derived(fmtClock(item.ts));
  const fullTime = $derived(item.ts ? new Date(item.ts).toLocaleString() : '');
  const workedFor = $derived(item.duration_ms != null ? `Worked for ${fmtDuration(item.duration_ms)}` : 'Worked');

  function artifactIcon(a: Artifact): IconName {
    return a.kind === 'pr' ? 'pr' : a.kind === 'image' ? 'image' : a.kind === 'url' ? 'link' : a.kind === 'report' ? 'note' : 'file';
  }
  function openArtifact(a: Artifact): void {
    if (a.url) ctx?.openUrl?.(a.url);
    else if (a.path) ctx?.openPreview?.({ kind: 'file', path: a.path });
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

{#snippet seg(s: Segment, i: number)}
  {#if s.kind === 'steps'}
    {#if s.steps.every((b) => b.kind === 'thinking')}
      <ThinkingMarker count={s.steps.reduce((n, b) => n + (b.kind === 'thinking' ? b.count : 0), 0)} tokens={s === firstThink ? (item.usage?.thinking_tokens ?? null) : null} {agentName} />
    {:else}
      <WorkSteps steps={s.steps} durationMs={!fold && i === firstStepsIdx ? item.duration_ms : null} {live} {active} {waiting} />
    {/if}
  {:else if s.block.kind === 'text'}
    <Markdown md={s.block.md} />
  {:else if s.block.kind === 'tasks'}
    <TasksBlock tasks={s.block.tasks} folded={s !== lastTasks} />
  {:else if s.block.kind === 'image'}
    <ImageBlock id={s.block.id} alt={s.block.alt} mediaType={s.block.media_type} />
  {:else if s.block.kind === 'queued'}
    {@render pendingChip(s.block.text)}
  {:else if s.block.kind === 'artifact'}
    {@const a = s.block.artifact}
    <button class="chip artifact" onclick={() => openArtifact(a)} title={a.url ?? a.path ?? a.label}>
      <Icon name={artifactIcon(a)} size={11} /> {a.label}
      {#if a.path && !a.url}<Icon name="eye" size={11} />{/if}
    </button>
  {:else if s.block.kind === 'notice'}
    <div class="notice" title={s.block.note.kind}>
      <Icon name="info" size={11} /> <strong>{s.block.note.title}</strong>
      {#if s.block.note.body}<span class="dim"> — {s.block.note.body.slice(0, 400)}</span>{/if}
    </div>
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
          {@const parts = splitAttachments(b.md)}
          {#if parts.md}<Markdown md={parts.md} />{/if}
          {#if parts.images.length}
            <div class="attach" data-attachments={parts.images.length}>
              {#each parts.images as p (p)}
                <button class="attach-chip" onclick={() => void copyPath(p)} title="Attached image — {p} (click to copy the path)">
                  <Icon name="image" size={12} /> <span class="attach-name">{p.split('/').pop()}</span>
                </button>
              {/each}
            </div>
          {/if}
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
      {@render sysChip()}
      {#if ctx.reusePrompt && promptText}
        <button class="act-btn" onclick={() => ctx.reusePrompt?.(promptText)} title="Edit & resend — put this message back in the composer" aria-label="Edit and resend"><Icon name="edit" size={12} /></button>
      {/if}
      {#if copyText}
        <button class="act-btn copy-btn" onclick={() => void copyTurn()} title="Copy message" aria-label="Copy message"><Icon name="copy" size={12} /></button>
      {/if}
      {#if clock}<time class="ts" datetime={item.ts ?? undefined} title={fullTime}>{clock}</time>{/if}
    </div>
  {:else}
    {#if !nested}
      <header class="agent-head">
        <span class="avatar" aria-hidden="true"><ProviderIcon provider={ctx.provider} size={13} /></span>
        <span class="agent-name">{agentName}</span>
        {#if item.model}<span class="agent-model mono" title="Model">{item.model}</span>{/if}
        {#if clock}<time class="ts" datetime={item.ts ?? undefined} title={fullTime}>{clock}</time>{/if}
      </header>
    {/if}
    <div class="resp">
      {#if isCodex && item.reasoning_steps > 0}
        <ThinkingMarker codex count={item.reasoning_steps} tokens={item.usage?.thinking_tokens ?? null} {agentName} />
      {/if}
      {#if fold}
        {#each fold.head as s, i (i)}{@render seg(s, i)}{/each}
        <div class="fold" class:open={foldShown}>
          <button class="fold-row" onclick={() => (foldOpen = !foldShown)} aria-expanded={foldShown} data-fold-toggle title={foldShown ? 'Hide the work' : 'Show the work: tool calls, narration and plan updates'}>
            <span class="fold-label">{workedFor}</span>
            <Icon name={foldShown ? 'chevronDown' : 'chevronRight'} size={12} />
            {#if stats}
              <span class="fold-meta">{`${stats.steps} ${stats.steps === 1 ? 'step' : 'steps'}${stats.thinking ? ` · thought ${stats.thinking}×` : ''}`}</span>
              {#if stats.failed}<span class="fold-fail">{stats.failed} failed</span>{/if}
            {/if}
          </button>
          {#if foldShown}
            <div class="fold-body">
              {#each fold.body as s, i (i)}{@render seg(s, i)}{/each}
            </div>
          {/if}
        </div>
        {#each fold.tail as s, i (i)}{@render seg(s, i)}{/each}
      {:else}
        {#each visibleBlocks as s, i (i)}{@render seg(s, i)}{/each}
      {/if}
      {#if !visibleBlocks.length && !(isCodex && item.reasoning_steps > 0)}
        <div class="dim empty">(no visible content{sysNotes.length ? ' — system notes only' : ''})</div>
      {/if}
      {#if changed.length}
        <ChangedFiles files={changed} />
      {/if}
    </div>
    <footer class="meta">
      <UsageLine usage={item.usage ?? null} durationMs={fold ? null : item.duration_ms} model={nested ? item.model : null} />
      <span class="meta-grow"></span>
      {@render sysChip()}
      {#if nested && clock}<time class="ts" datetime={item.ts ?? undefined} title={fullTime}>{clock}</time>{/if}
      {#if copyText}
        <button class="act-btn copy-btn" onclick={() => void copyTurn()} title="Copy response" aria-label="Copy response"><Icon name="copy" size={12} /></button>
      {/if}
    </footer>
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
    gap: 5px;
    padding: 10px 0;
    min-width: 0;
    scroll-margin-block: 16px;
  }
  .turn.nested {
    padding: 4px 6px;
  }
  .turn.user {
    align-items: flex-end;
    padding-block: 14px 2px;
  }
  /* You: a blue bubble on the trailing side. */
  .bubble {
    max-width: min(80%, 78ch);
    background: color-mix(in srgb, var(--you) 16%, var(--surface));
    border: 1px solid color-mix(in srgb, var(--you) 34%, var(--border));
    border-radius: 18px 18px 5px 18px;
    padding: 9px 15px;
    min-width: 0;
    box-shadow: 0 1px 0 color-mix(in srgb, var(--you) 10%, transparent);
  }
  :global([dir='rtl']) .bubble {
    border-radius: 18px 18px 18px 5px;
  }
  .bubble :global(.md) {
    --prose-measure: none;
  }
  .attach {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-block: 6px 2px;
  }
  .attach-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    max-width: 100%;
    padding: 3px 9px;
    border-radius: var(--radius-m);
    border: 1px solid color-mix(in srgb, var(--you) 30%, var(--border));
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .attach-chip:hover {
    border-color: var(--border-strong);
  }
  .attach-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The agent: green disc + name + model + time, then the response. */
  .agent-head {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .avatar {
    display: inline-grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    flex-shrink: 0;
    background: color-mix(in srgb, var(--agent) 18%, var(--surface));
    border: 1px solid color-mix(in srgb, var(--agent) 45%, var(--border));
    color: var(--text);
  }
  .agent-name {
    font-weight: 650;
  }
  .agent-model {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .ts {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  /* The response: a faint green surface on a green rail, the avatar's column. */
  .resp {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    margin-inline-start: 11px;
    padding-block: 10px 12px;
    padding-inline: 18px 16px;
    border-inline-start: 2px solid color-mix(in srgb, var(--agent) 60%, transparent);
    background: color-mix(in srgb, var(--agent) 5%, transparent);
    border-start-end-radius: var(--radius-l);
    border-end-end-radius: var(--radius-l);
  }
  .turn.nested .resp {
    border: 0;
    padding: 0;
    margin: 0;
    background: none;
  }
  /* "Worked for 2m 7s ›": the settled response's work, folded (t3code). */
  .fold {
    min-width: 0;
    margin: 2px 0;
  }
  .fold-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 4px 6px 6px;
    border: 0;
    border-bottom: 1px solid color-mix(in srgb, var(--agent) 22%, var(--border));
    border-radius: 0;
    background: none;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
    text-align: start;
    font-variant-numeric: tabular-nums;
  }
  .fold-row:hover {
    color: var(--text);
  }
  .fold-row:hover .fold-label {
    text-decoration: underline;
    text-decoration-color: var(--border-strong);
  }
  .fold-row:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
    border-radius: var(--radius-s);
  }
  .fold-label {
    color: var(--text);
    font-weight: 500;
  }
  .fold-meta {
    font-size: var(--fs-xs);
    margin-inline-start: 4px;
  }
  .fold-fail {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--danger);
    background: var(--danger-soft);
    border-radius: 99px;
    padding: 1px 7px;
  }
  .fold-body {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-block: 8px 6px;
    padding-inline: 12px 0;
    border-inline-start: 1px dashed color-mix(in srgb, var(--agent) 35%, var(--border));
    margin-block: 4px 2px;
    margin-inline: 2px 0;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 22px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex-wrap: wrap;
    padding-inline-start: 31px;
    min-width: 0;
  }
  .meta-grow {
    flex: 1;
  }
  .turn.nested .meta {
    padding-inline-start: 0;
  }
  .turn.user .meta {
    justify-content: flex-end;
    padding-inline-start: 0;
  }
  /* Quiet until wanted: the action buttons fade in on hover / keyboard focus
     (always shown on touch); facts (tokens, time) stay. */
  .act-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 22px;
    background: none;
    border: 0;
    border-radius: var(--radius-s);
    padding: 0;
    color: var(--text-dim);
    cursor: pointer;
    opacity: 0;
    transition: opacity 120ms;
  }
  .turn:hover .act-btn,
  .turn:focus-within .act-btn {
    opacity: 1;
  }
  @media (hover: none) {
    .act-btn {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .act-btn {
      transition: none;
    }
  }
  .act-btn:hover {
    color: var(--text);
    background: var(--hover);
  }
  .act-btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .turn.hit .bubble,
  .turn.hit .resp {
    box-shadow: 0 0 0 2px var(--warning-soft);
  }
  .turn.current .bubble,
  .turn.current .resp {
    box-shadow: 0 0 0 2px var(--status-warn);
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
    max-width: min(80%, 78ch);
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
    color: var(--text);
    cursor: pointer;
    font: inherit;
    font-size: var(--fs-xs);
  }
  .chip.artifact:hover {
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
  /* A narrow pane: the response hugs the edge, the rail stays. */
  @container (max-width: 480px) {
    .resp {
      margin-inline-start: 4px;
      padding-inline: 11px 8px;
    }
    .meta {
      padding-inline-start: 16px;
    }
    .bubble {
      max-width: 92%;
    }
  }
</style>
