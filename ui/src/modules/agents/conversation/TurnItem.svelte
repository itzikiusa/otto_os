<script lang="ts">
  // One render item: a right-aligned user bubble, or a full-width assistant
  // response (prose via sanitized markdown, tool steps grouped, images,
  // queued/artifact/notice chips), with the per-turn system chip and the
  // Codex "N reasoning steps (not recorded)" footer.
  import { getContext } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Markdown from './Markdown.svelte';
  import WorkSteps from './WorkSteps.svelte';
  import ImageBlock from './ImageBlock.svelte';
  import { transcript } from '../../../lib/stores/transcript.svelte';
  import { fmtClock, fmtDuration, segment, type RenderItem } from './format';
  import type { Artifact, SystemNote } from '../../../lib/api/types';
  import { CONV_CTX, type ConvContext } from './context';

  interface Props {
    item: RenderItem;
    /** True for the newest response of a live session (step groups start open). */
    live?: boolean;
    /** Inside a subagent card — tighter chrome. */
    nested?: boolean;
  }
  let { item, live = false, nested = false }: Props = $props();
  const ctx = getContext<ConvContext>(CONV_CTX);

  const segs = $derived(segment(item.blocks));
  const showSystem = $derived(transcript.showSystem);
  // Codex reasoning is dropped by the parser and counted per turn
  // (`Turn.reasoning_steps`) → one "N reasoning steps (not recorded)" footer per response.
  const isCodex = $derived(ctx.provider === 'codex');
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

  function artifactHref(a: Artifact): string | null {
    if (a.url) return a.url;
    return null;
  }
  function artifactIcon(a: Artifact): string {
    return a.kind === 'pr' ? 'pr' : a.kind === 'image' ? 'image' : a.kind === 'url' ? 'link' : a.kind === 'report' ? 'note' : 'file';
  }
</script>

<article class="turn {item.role}" class:nested data-turn-id={item.id} data-role={item.role}>
  {#if item.role === 'user'}
    <div class="bubble" dir="auto">
      {#each item.blocks as b, i (i)}
        {#if b.kind === 'text'}
          <Markdown md={b.md} />
        {:else if b.kind === 'image'}
          <ImageBlock id={b.id} alt={b.alt} mediaType={b.media_type} />
        {:else if b.kind === 'queued' && b.op === 'enqueue' && ctx.queuedLive.includes(b.text) && (showSystem || !b.injected)}
          <span class="chip queued" title="Queued while the agent was working">Queued: {b.text}</span>
        {:else if b.kind === 'notice' && showSystem}
          <span class="chip note" title={b.note.body ?? ''}>{b.note.title}</span>
        {/if}
      {/each}
    </div>
    <div class="meta">
      {#if item.ts}<span class="ts">{fmtClock(item.ts)}</span>{/if}
      {#if sysNotes.length}
        <button class="sys-chip" class:on={sysOpen} onclick={() => (sysOpen = !sysOpen)} title="System notes attached to this turn">
          <Icon name="info" size={10} /> {sysNotes.length} system
        </button>
      {/if}
    </div>
  {:else}
    <div class="resp">
      {#each visibleBlocks as s, i (i)}
        {#if s.kind === 'steps'}
          <WorkSteps steps={s.steps} durationMs={i === firstStepsIdx ? item.duration_ms : null} {live} />
        {:else if s.block.kind === 'text'}
          <Markdown md={s.block.md} />
        {:else if s.block.kind === 'image'}
          <ImageBlock id={s.block.id} alt={s.block.alt} mediaType={s.block.media_type} />
        {:else if s.block.kind === 'queued'}
          <span class="chip queued">Queued: {s.block.text}</span>
        {:else if s.block.kind === 'artifact'}
          {@const a = s.block.artifact}
          {#if artifactHref(a)}
            <a class="chip artifact" href={artifactHref(a)} target="_blank" rel="noopener noreferrer" title={a.path ?? a.url ?? ''}>
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
      {#if item.ts}<span class="ts">{fmtClock(item.ts)}</span>{/if}
      {#if item.model}<span class="dim mono model">{item.model}</span>{/if}
      {#if item.duration_ms != null && firstStepsIdx < 0}<span class="dim">{fmtDuration(item.duration_ms)}</span>{/if}
      {#if isCodex && item.reasoning_steps > 0}
        <span class="dim" title="Codex does not persist reasoning text">{item.reasoning_steps} reasoning steps (not recorded)</span>
      {/if}
      {#if sysNotes.length}
        <button class="sys-chip" class:on={sysOpen} onclick={() => (sysOpen = !sysOpen)} title="System notes attached to this turn">
          <Icon name="info" size={10} /> {sysNotes.length} system
        </button>
      {/if}
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
    padding: 6px 16px;
    min-width: 0;
  }
  .turn.nested {
    padding: 4px 6px;
  }
  .turn.user {
    align-items: flex-end;
  }
  .bubble {
    max-width: min(78%, 720px);
    background: color-mix(in srgb, var(--accent) 14%, var(--surface));
    border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--border));
    border-radius: 14px 14px 4px 14px;
    padding: 8px 12px;
    min-width: 0;
  }
  :global([dir='rtl']) .bubble {
    border-radius: 14px 14px 14px 4px;
  }
  .resp {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    max-width: 920px;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 10.5px;
    color: var(--text-dim);
    flex-wrap: wrap;
  }
  .turn.user .meta {
    justify-content: flex-end;
  }
  .model {
    font-size: 10px;
  }
  .sys-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: none;
    border: 1px solid var(--border);
    border-radius: 99px;
    color: var(--text-dim);
    font-size: 10px;
    padding: 0 7px;
    height: 16px;
    cursor: pointer;
  }
  .sys-chip:hover,
  .sys-chip.on {
    color: var(--text);
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .sys-list {
    display: flex;
    flex-direction: column;
    gap: 3px;
    width: 100%;
    max-width: 920px;
  }
  .turn.user .sys-list {
    align-self: flex-end;
    max-width: min(78%, 720px);
  }
  .sys-note {
    font-size: 11.5px;
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
    font-size: 10px;
    opacity: 0.8;
  }
  .sys-note pre {
    margin: 4px 0 2px;
    font-size: 11px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 260px;
    overflow: auto;
    text-align: start;
  }
  .chip.queued {
    align-self: flex-start;
    height: auto;
    padding: 2px 8px;
    white-space: normal;
    color: var(--text-dim);
    border-style: dashed;
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
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .notice {
    display: flex;
    align-items: baseline;
    gap: 5px;
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 4px 8px;
    border-inline-start: 2px solid var(--border);
    overflow-wrap: anywhere;
  }
  .empty {
    font-size: 12px;
  }
</style>
