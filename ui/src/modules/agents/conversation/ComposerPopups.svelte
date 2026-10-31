<script lang="ts">
  // Clamped completion popups anchored above the composer box: `@file`
  // workspace path typeahead and `/` slash-command completion.
  import type { SlashCommand } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import type { MentionEntry } from './fileMentions';

  interface Props {
    popId: string;
    anchorEl: HTMLElement | null;
    paneEl?: HTMLElement | null;
    paneH: number;
    attachmentCount: number;
    mentionOpen: boolean;
    fileHits: MentionEntry[];
    fileIdx: number;
    onfilehover: (idx: number) => void;
    onacceptmention: (entry: MentionEntry) => void;
    cmdOpen: boolean;
    suggestions: SlashCommand[];
    cmdIdx: number;
    oncmdhover: (idx: number) => void;
    onacceptcmd: (cmd: SlashCommand) => void;
  }

  let {
    popId,
    anchorEl,
    paneEl = null,
    paneH,
    attachmentCount,
    mentionOpen,
    fileHits,
    fileIdx,
    onfilehover,
    onacceptmention,
    cmdOpen,
    suggestions,
    cmdIdx,
    oncmdhover,
    onacceptcmd,
  }: Props = $props();

  let listEl = $state<HTMLDivElement | null>(null);
  let fileListEl = $state<HTMLDivElement | null>(null);
  let popMax = $state(320);

  function measurePopMax(): void {
    if ((!cmdOpen && !mentionOpen) || !anchorEl) return;
    const paneTop = Math.max(0, paneEl?.getBoundingClientRect().top ?? 0);
    const room = anchorEl.getBoundingClientRect().top - paneTop - 12;
    popMax = Math.floor(Math.max(0, Math.min(320, window.innerHeight * 0.5, room)));
  }

  $effect(() => {
    void suggestions.length;
    void fileHits.length;
    void attachmentCount;
    void paneH;
    if ((!cmdOpen && !mentionOpen) || !anchorEl) return;
    measurePopMax();
    window.addEventListener('resize', measurePopMax);
    return () => window.removeEventListener('resize', measurePopMax);
  });

  $effect(() => {
    const el = listEl?.children[cmdIdx] as HTMLElement | undefined;
    el?.scrollIntoView({ block: 'nearest' });
  });

  $effect(() => {
    const el = fileListEl?.children[fileIdx] as HTMLElement | undefined;
    el?.scrollIntoView({ block: 'nearest' });
  });
</script>

{#if mentionOpen}
  <div
    class="cmd-pop file-pop"
    id="{popId}-files"
    bind:this={fileListEl}
    style="--pop-max:{popMax}px"
    role="listbox"
    aria-label="Workspace files"
    data-file-pop
  >
    {#each fileHits as f, i (f.relPath)}
      <div
        id="{popId}-file-{i}"
        class="cmd-row"
        class:active={i === fileIdx}
        role="option"
        tabindex="-1"
        aria-selected={i === fileIdx}
        onmousedown={(e) => { e.preventDefault(); onacceptmention(f); }}
        onmouseenter={() => onfilehover(i)}
      >
        <span class="file-icon" aria-hidden="true"><Icon name={f.isDir ? 'folder' : 'file'} size={12} /></span>
        <span class="file-path mono" dir="ltr">{f.relPath}{f.isDir ? '/' : ''}</span>
        <span class="cmd-src dim">{f.isDir ? 'dir' : ''}</span>
      </div>
    {/each}
    <div class="cmd-hint dim">↑↓ choose · Tab/⏎ insert · Esc dismiss</div>
  </div>
{:else if cmdOpen}
  <div
    class="cmd-pop"
    id="{popId}-list"
    bind:this={listEl}
    style="--pop-max:{popMax}px"
    role="listbox"
    aria-label="Slash commands"
    data-slash-pop
  >
    {#each suggestions as c, i (c.name)}
      <div
        id="{popId}-opt-{i}"
        class="cmd-row"
        class:active={i === cmdIdx}
        role="option"
        tabindex="-1"
        aria-selected={i === cmdIdx}
        onmousedown={(e) => { e.preventDefault(); onacceptcmd(c); }}
        onmouseenter={() => oncmdhover(i)}
      >
        <span class="cmd-name mono" dir="ltr">/{c.name}</span>
        <span class="cmd-desc dim">{c.description}</span>
        <span class="cmd-src dim">{c.source === 'builtin' ? '' : c.source}</span>
      </div>
    {/each}
    <div class="cmd-hint dim">↑↓ choose · Tab/⏎ complete · Esc dismiss</div>
  </div>
{/if}

<style>
  .cmd-pop {
    position: absolute;
    bottom: calc(100% + 6px);
    inset-inline-start: 0;
    width: min(100%, 560px);
    max-width: 100%;
    max-height: var(--pop-max);
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-l);
    box-shadow: var(--glass-shadow);
    padding: 4px;
    z-index: var(--z-popover);
  }
  .file-pop {
    max-height: min(260px, 45vh, var(--pop-max));
  }
  .cmd-row {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .cmd-row.active {
    background: var(--accent-soft);
  }
  .cmd-name {
    color: var(--accent-text);
    white-space: nowrap;
    unicode-bidi: isolate;
  }
  .file-icon {
    display: inline-flex;
    color: var(--text-dim);
  }
  .file-path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text);
    unicode-bidi: isolate;
    text-align: start;
  }
  .cmd-desc {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    font-size: var(--fs-xs);
  }
  .cmd-src {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
  }
  .cmd-hint {
    font-size: var(--fs-xs);
    padding: 4px 8px 2px;
    border-top: 1px solid var(--border);
    margin-block-start: 2px;
  }
</style>
