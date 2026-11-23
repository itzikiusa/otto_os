<script lang="ts">
  // Viewport-clamped Git gutter hunk popover (`E-10a`): compact diff preview +
  // Copy hunk diff, Revert hunk (undoable via ⌘Z), and Ask Agent to review hunk.
  import { tick } from 'svelte';
  import { placeAnchored, type AnchorPlacement } from '../../../lib/anchorPlace';
  import { copyWithToast } from '../../../lib/clipboard';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { plural } from '../../../lib/plural';
  import { ideBasename } from '../ideStore.svelte';
  import { formatHunkTitle, type IdeGutterHunk } from '../git/ideGutterHunks';

  interface Props {
    path: string;
    hunks: IdeGutterHunk[];
    hunkIndex: number;
    anchorX: number;
    anchorY: number;
    readOnly: boolean;
    onselecthunk: (nextIndex: number) => void;
    onrevert: (hunk: IdeGutterHunk) => void;
    onaskagent: (hunk: IdeGutterHunk) => void;
    onclose: () => void;
  }

  let {
    path,
    hunks,
    hunkIndex,
    anchorX,
    anchorY,
    readOnly,
    onselecthunk,
    onrevert,
    onaskagent,
    onclose,
  }: Props = $props();

  const returnFocusEl =
    typeof document !== 'undefined' && document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;

  let anchorEl = $state<HTMLSpanElement | undefined>(undefined);
  let popEl = $state<HTMLDivElement | undefined>(undefined);
  let copyBtnEl = $state<HTMLButtonElement | undefined>(undefined);
  let width = $state(460);
  let pos = $state<AnchorPlacement>({ left: 12, top: 12, maxH: 380 });

  const clampedIndex = $derived(hunks.length > 0 ? Math.max(0, Math.min(hunkIndex, hunks.length - 1)) : 0);
  const hunk = $derived<IdeGutterHunk | null>(hunks[clampedIndex] ?? null);
  const title = $derived(hunk ? formatHunkTitle(hunk) : 'Git hunk');
  const fileBase = $derived(ideBasename(path));
  const previewRemoved = $derived(hunk ? hunk.removedLines.slice(0, 12) : []);
  const previewAdded = $derived(hunk ? hunk.addedLines.slice(0, 12) : []);
  const hiddenRemoved = $derived(hunk ? Math.max(0, hunk.removedLines.length - previewRemoved.length) : 0);
  const hiddenAdded = $derived(hunk ? Math.max(0, hunk.addedLines.length - previewAdded.length) : 0);

  function closePopover(restoreFocus = true): void {
    onclose();
    if (restoreFocus && returnFocusEl?.isConnected) returnFocusEl.focus({ preventScroll: true });
  }

  function updatePlacement(): void {
    if (typeof window === 'undefined') return;
    width = Math.min(460, Math.max(280, window.innerWidth - 16));
    if (anchorEl && popEl) {
      pos = placeAnchored(anchorEl, popEl, { prefer: 'below', align: 'start', gap: 6 });
    }
  }

  $effect(() => {
    void anchorX;
    void anchorY;
    void clampedIndex;
    void tick().then(() => {
      updatePlacement();
      copyBtnEl?.focus();
    });
  });

  $effect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        closePopover(true);
      }
    };
    const onResize = (): void => updatePlacement();
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', onResize);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('resize', onResize);
    };
  });

  async function handleCopy(): Promise<void> {
    if (!hunk) return;
    await copyWithToast(hunk.unifiedDiff, 'hunk diff');
  }

  async function handleRevert(): Promise<void> {
    if (!hunk || readOnly) return;
    const totalLines = hunk.addedLines.length + hunk.removedLines.length;
    if (totalLines > 1) {
      const ok = await confirmer.ask(
        `Revert ${plural(totalLines, 'changed line')} in ${fileBase} back to Git HEAD? You can still undo with ⌘Z.`,
        { title: 'Revert Git hunk', confirmLabel: 'Revert hunk', danger: true },
      );
      if (!ok) return;
    }
    const target = hunk;
    closePopover(true);
    onrevert(target);
  }

  function handleAskAgent(): void {
    if (!hunk) return;
    const target = hunk;
    closePopover(false);
    onaskagent(target);
  }
</script>

<span bind:this={anchorEl} class="gh-anchor" aria-hidden="true" style:left="{anchorX}px" style:top="{anchorY}px"></span>

{#if hunk}
  <div
    class="gh-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) closePopover(true);
    }}
  >
    <div
      bind:this={popEl}
      class="gh-popover"
      role="dialog"
      aria-modal="true"
      aria-label="Git hunk {title} in {fileBase}"
      style:left="{pos.left}px"
      style:top="{pos.top}px"
      style:inline-size="{width}px"
      style:max-block-size="{pos.maxH}px"
      data-testid="ide-git-hunk-popover"
    >
      <header class="gh-head">
        <div class="gh-head-main" dir="ltr">
          <span class="gh-title">{title}</span>
          <span class="gh-file mono dim ellipsis" title={path}>{fileBase}</span>
        </div>
        <div class="gh-head-controls">
          {#if hunks.length > 1}
            <Badge tone="neutral" variant="outline" label="{clampedIndex + 1}/{hunks.length}" />
            <button
              type="button"
              class="icon-btn"
              aria-label="Previous Git hunk"
              title="Previous Git hunk"
              onclick={() => onselecthunk((clampedIndex - 1 + hunks.length) % hunks.length)}
              data-testid="ide-git-hunk-prev"
            >
              <Icon name="chevronUp" size={12} />
            </button>
            <button
              type="button"
              class="icon-btn"
              aria-label="Next Git hunk"
              title="Next Git hunk"
              onclick={() => onselecthunk((clampedIndex + 1) % hunks.length)}
              data-testid="ide-git-hunk-next"
            >
              <Icon name="chevronDown" size={12} />
            </button>
          {/if}
          <button
            type="button"
            class="icon-btn"
            aria-label="Close Git hunk popover"
            title="Close Git hunk popover"
            onclick={() => closePopover(true)}
          >
            <Icon name="x" size={12} />
          </button>
        </div>
      </header>

      <div class="gh-diff mono" dir="ltr" data-testid="ide-git-hunk-preview">
        {#each previewRemoved as line, idx (`del-${idx}`)}
          <div class="gh-line gh-del"><span class="gh-sign" aria-hidden="true">-</span><span class="gh-code">{line || ' '}</span></div>
        {/each}
        {#if hiddenRemoved > 0}
          <div class="gh-more dim">… {plural(hiddenRemoved, 'more removed line')}</div>
        {/if}
        {#each previewAdded as line, idx (`add-${idx}`)}
          <div class="gh-line gh-add"><span class="gh-sign" aria-hidden="true">+</span><span class="gh-code">{line || ' '}</span></div>
        {/each}
        {#if hiddenAdded > 0}
          <div class="gh-more dim">… {plural(hiddenAdded, 'more added line')}</div>
        {/if}
      </div>

      <footer class="gh-foot">
        <button
          bind:this={copyBtnEl}
          type="button"
          class="btn small ghost"
          onclick={() => void handleCopy()}
          data-testid="ide-git-hunk-copy"
        >
          <Icon name="copy" size={12} />
          <span>Copy hunk diff</span>
        </button>
        <button
          type="button"
          class="btn small ghost gh-revert-btn"
          disabled={readOnly}
          title={readOnly ? 'File is read-only' : 'Revert this hunk to Git HEAD (undoable with ⌘Z)'}
          onclick={() => void handleRevert()}
          data-testid="ide-git-hunk-revert"
        >
          <Icon name="undo" size={12} />
          <span>Revert hunk</span>
        </button>
        <span class="grow"></span>
        <button
          type="button"
          class="btn small primary"
          onclick={handleAskAgent}
          data-testid="ide-git-hunk-ask-agent"
        >
          <Icon name="assistant" size={12} />
          <span>Ask Agent</span>
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .gh-anchor {
    position: fixed;
    inline-size: 0;
    block-size: 0;
    pointer-events: none;
  }
  .gh-backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--z-popover-backdrop);
  }
  .gh-popover {
    position: fixed;
    z-index: var(--z-popover);
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    padding: var(--sp-4);
    background: var(--surface);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-m);
    box-shadow: var(--glass-shadow);
    overflow: hidden;
  }
  .gh-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding-block-end: var(--sp-2);
    border-block-end: 1px solid var(--separator);
  }
  .gh-head-main {
    display: flex;
    align-items: baseline;
    gap: var(--sp-2);
    min-inline-size: 0;
    flex: 1;
  }
  .gh-title {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
  }
  .gh-file {
    font-size: var(--fs-xs);
    min-inline-size: 0;
  }
  .gh-head-controls {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    flex-shrink: 0;
  }
  .gh-diff {
    overflow: auto;
    max-block-size: 200px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    font-size: var(--fs-xs);
    line-height: 1.45;
  }
  .gh-line {
    display: flex;
    gap: var(--sp-2);
    padding-block: 0;
    padding-inline: var(--sp-3);
    white-space: pre;
  }
  .gh-sign {
    flex-shrink: 0;
    user-select: none;
    font-weight: 600;
  }
  .gh-code {
    min-inline-size: 0;
  }
  .gh-del {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .gh-add {
    background: var(--success-soft);
    color: var(--success);
  }
  .gh-more {
    padding-block: var(--sp-1);
    padding-inline: var(--sp-3);
    font-size: var(--fs-xs);
  }
  .gh-foot {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
    padding-block-start: var(--sp-1);
  }
  .gh-revert-btn {
    color: var(--danger);
  }
</style>
