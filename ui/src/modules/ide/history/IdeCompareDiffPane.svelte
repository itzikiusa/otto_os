<script lang="ts">
  // Interactive Side-by-Side (and Unified) Comparison Pane with syntax coloring,
  // word-diff marks, `↑`/`↓` hunk navigation, clickable per-hunk `>>` (`Revert ⌃⌘→`)
  // buttons, live editable `Current` right pane, and `ℹ Contents are identical` banner.
  import Icon from '../../../lib/components/Icon.svelte';
  import { ensureHljs, escapeHtml, highlightLines, langFromPath } from '../../../lib/hl';
  import { scrollBehavior } from '../../../lib/motion';
  import { plural } from '../../../lib/plural';
  import DiffViewer from '../../git/DiffViewer.svelte'; // ui-guards: allow — reuse Otto's shared Git DiffViewer
  import { markWords } from '../../git/diff-words'; // ui-guards: allow — reuse Otto's shared Git word diff
  import {
    applyLeftHunkToRight,
    computeSideBySideComparison,
  } from './ideCompareEngine';
  import { buildLocalHistoryDiff } from './ideLocalHistoryDiff';

  interface Props {
    filePath: string;
    leftLabel: string;
    rightLabel: string;
    leftText: string;
    rightText: string;
    rightExists?: boolean;
    canEditRight?: boolean;
    onApplyRight?: (nextText: string, reason: string) => void | Promise<void>;
  }

  let {
    filePath,
    leftLabel,
    rightLabel,
    leftText,
    rightText,
    rightExists = true,
    canEditRight = true,
    onApplyRight,
  }: Props = $props();

  let viewMode = $state<'split' | 'unified'>('split');
  let activeHunkIdx = $state(0);
  let editingRight = $state(false);
  let editDraft = $state('');
  let dismissedIdentical = $state(false);
  let hlReady = $state(false);
  let scrollEl = $state<HTMLDivElement | undefined>(undefined);
  let editSaveTimer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    void ensureHljs().then(() => {
      hlReady = true;
    });
  });

  // Reset identical-banner dismissal when inputs change.
  $effect(() => {
    void leftText;
    void rightText;
    dismissedIdentical = false;
  });

  // Keep inline editor draft synced when external rightText changes while not typing.
  $effect(() => {
    if (!editingRight) editDraft = rightText;
  });

  const effectiveRight = $derived(editingRight ? editDraft : rightText);
  const cmp = $derived(computeSideBySideComparison(leftText, effectiveRight));
  const lang = $derived.by(() => {
    void hlReady;
    return langFromPath(filePath);
  });
  const leftHl = $derived.by(() => {
    void hlReady;
    return highlightLines(leftText.replace(/\r\n/g, '\n'), lang);
  });
  const rightHl = $derived.by(() => {
    void hlReady;
    return highlightLines(effectiveRight.replace(/\r\n/g, '\n'), lang);
  });

  const unifiedDiff = $derived.by(() => {
    if (viewMode !== 'unified') return null;
    return buildLocalHistoryDiff(filePath, effectiveRight, leftText, true);
  });

  $effect(() => {
    if (activeHunkIdx >= cmp.hunks.length) {
      activeHunkIdx = Math.max(0, cmp.hunks.length - 1);
    }
  });

  function scrollToHunk(idx: number): void {
    if (cmp.hunks.length === 0) return;
    const clamped = ((idx % cmp.hunks.length) + cmp.hunks.length) % cmp.hunks.length;
    activeHunkIdx = clamped;
    const el = scrollEl?.querySelector<HTMLElement>(`[data-hunk-start="${clamped}"]`);
    el?.scrollIntoView({ block: 'center', behavior: scrollBehavior() });
  }

  function revertHunk(idx: number): void {
    if (!canEditRight || !onApplyRight) return;
    const next = applyLeftHunkToRight(leftText, effectiveRight, idx);
    if (editingRight) editDraft = next;
    void onApplyRight(next, `Applied hunk ${idx + 1}`);
  }

  function revertAllHunks(): void {
    if (!canEditRight || !onApplyRight) return;
    const next = leftText.replace(/\r\n/g, '\n');
    if (editingRight) editDraft = next;
    void onApplyRight(next, 'Applied all differences');
  }

  function onDraftInput(next: string): void {
    editDraft = next;
    if (!canEditRight || !onApplyRight) return;
    if (editSaveTimer) clearTimeout(editSaveTimer);
    editSaveTimer = setTimeout(() => {
      editSaveTimer = null;
      void onApplyRight(editDraft, 'Edited Current');
    }, 320);
  }

  function toggleEditRight(): void {
    if (editingRight && editSaveTimer && onApplyRight) {
      clearTimeout(editSaveTimer);
      editSaveTimer = null;
      void onApplyRight(editDraft, 'Edited Current');
    } else if (!editingRight) {
      editDraft = rightText;
    }
    editingRight = !editingRight;
  }

  function onPaneKeydown(e: KeyboardEvent): void {
    if (e.ctrlKey && e.metaKey && e.key === 'ArrowRight') {
      e.preventDefault();
      if (cmp.hunks.length > 0) revertHunk(activeHunkIdx);
    } else if (!editingRight && e.key === 'F7') {
      e.preventDefault();
      scrollToHunk(e.shiftKey ? activeHunkIdx - 1 : activeHunkIdx + 1);
    }
  }
</script>

<svelte:window onkeydown={onPaneKeydown} />

<div class="cmp-pane" role="region" aria-label="File comparison" data-testid="ide-compare-diff-pane">
  <header class="cmp-toolbar">
    <button
      type="button"
      class="cmp-icon-btn"
      aria-label="Previous difference"
      title="Previous difference (⇧F7)"
      disabled={cmp.hunks.length === 0}
      onclick={() => scrollToHunk(activeHunkIdx - 1)}
      data-testid="ide-compare-prev-diff"
    >
      <Icon name="arrowUp" size={12} />
    </button>
    <button
      type="button"
      class="cmp-icon-btn"
      aria-label="Next difference"
      title="Next difference (F7)"
      disabled={cmp.hunks.length === 0}
      onclick={() => scrollToHunk(activeHunkIdx + 1)}
      data-testid="ide-compare-next-diff"
    >
      <Icon name="arrowDown" size={12} />
    </button>

    <span class="cmp-diff-count mono" data-testid="ide-compare-diff-count">
      {cmp.hunks.length === 0 ? 'No differences' : plural(cmp.hunks.length, 'difference')}
    </span>

    {#if canEditRight && cmp.hunks.length > 0}
      <button
        type="button"
        class="btn small ghost cmp-bar-btn"
        title="Apply all changes from the left pane into Current"
        onclick={revertAllHunks}
        data-testid="ide-compare-accept-all"
      >
        <Icon name="chevronRight" size={12} noflip />
        <span>Accept all ({cmp.hunks.length})</span>
      </button>
    {/if}

    <span class="grow"></span>

    {#if canEditRight && viewMode === 'split'}
      <button
        type="button"
        class="btn small ghost cmp-bar-btn"
        class:active-pill={editingRight}
        aria-pressed={editingRight}
        title="Edit Current file inline while comparing"
        onclick={toggleEditRight}
        data-testid="ide-compare-edit-toggle"
      >
        <Icon name="edit" size={12} />
        <span>{editingRight ? 'Done editing' : 'Edit Current'}</span>
      </button>
    {/if}

    <div class="segmented cmp-seg" role="group" aria-label="Diff layout">
      <button
        type="button"
        class:active={viewMode === 'split'}
        aria-pressed={viewMode === 'split'}
        onclick={() => (viewMode = 'split')}
        data-testid="ide-compare-mode-split"
      >Side-by-side</button>
      <button
        type="button"
        class:active={viewMode === 'unified'}
        aria-pressed={viewMode === 'unified'}
        onclick={() => (viewMode = 'unified')}
        data-testid="ide-compare-mode-unified"
      >Unified</button>
    </div>
  </header>

  {#if cmp.identical && rightExists && !dismissedIdentical}
    <div class="cmp-identical-banner" data-testid="ide-compare-identical-banner">
      <Icon name="info" size={13} />
      <span>Contents are identical</span>
      <span class="grow"></span>
      <button
        type="button"
        class="cmp-icon-btn"
        aria-label="Dismiss identical notice"
        title="Dismiss"
        onclick={() => (dismissedIdentical = true)}
      >
        <Icon name="x" size={12} />
      </button>
    </div>
  {/if}

  {#if viewMode === 'unified' && unifiedDiff}
    <div class="cmp-unified-wrap">
      <DiffViewer diff={unifiedDiff} stateKey={`cmp-u:${filePath}:${cmp.hunks.length}`} />
    </div>
  {:else}
    <div class="cmp-col-heads" dir="ltr">
      <div class="cmp-col-head ellipsis" title={leftLabel}>
        <Icon name="clock" size={12} />
        <span class="mono ellipsis">{leftLabel}</span>
      </div>
      <div class="cmp-mid-head" aria-hidden="true"></div>
      <div class="cmp-col-head ellipsis" title={rightLabel}>
        <Icon name="file" size={12} />
        <span class="mono ellipsis">{rightLabel}</span>
        {#if !rightExists}
          <span class="cmp-del-tag">Deleted on disk</span>
        {:else if canEditRight}
          <span class="cmp-edit-hint dim">{editingRight ? 'Editing live' : 'Editable'}</span>
        {/if}
      </div>
    </div>

    {#if editingRight}
      <div class="cmp-live-split" dir="ltr">
        <pre class="cmp-live-left mono">{leftText}</pre>
        <div class="cmp-live-mid">
          {#if cmp.hunks.length > 0}
            <button
              type="button"
              class="cmp-chevron-btn"
              aria-label="Revert active hunk into Current"
              title="Revert hunk (⌃⌘→)"
              onclick={() => revertHunk(activeHunkIdx)}
              data-testid="ide-compare-revert-hunk"
            >&gt;&gt;</button>
          {/if}
        </div>
        <textarea
          class="cmp-live-editor mono"
          dir="ltr"
          spellcheck={false}
          aria-label="Editable current file content"
          value={editDraft}
          oninput={(e) => onDraftInput((e.currentTarget as HTMLTextAreaElement).value)}
          data-testid="ide-compare-current-editor"
        ></textarea>
      </div>
    {:else}
      <div class="cmp-scroll" bind:this={scrollEl} dir="ltr">
        {#each cmp.rows as row (row.index)}
          {@const lHtml = row.leftLineNo !== null ? markWords(leftHl[row.leftLineNo - 1] ?? escapeHtml(row.leftText ?? ''), row.leftWords) : ''}
          {@const rHtml = row.rightLineNo !== null ? markWords(rightHl[row.rightLineNo - 1] ?? escapeHtml(row.rightText ?? ''), row.rightWords) : ''}
          <div
            class="cmp-row is-{row.kind}"
            class:is-active-hunk={row.hunkIndex !== null && row.hunkIndex === activeHunkIdx}
            data-hunk-start={row.isHunkStart && row.hunkIndex !== null ? row.hunkIndex : undefined}
          >
            <div class="cmp-cell cmp-left-cell" class:is-empty={row.leftLineNo === null}>
              <span class="cmp-ln mono">{row.leftLineNo ?? ''}</span>
              <span class="cmp-code mono">{@html lHtml}</span>
            </div>

            <div class="cmp-gutter">
              {#if row.isHunkStart && row.hunkIndex !== null && canEditRight}
                {@const hIdx = row.hunkIndex}
                <button
                  type="button"
                  class="cmp-chevron-btn"
                  aria-label="Revert hunk {hIdx + 1} into Current"
                  title="Revert (⌃⌘→)"
                  onclick={() => {
                    activeHunkIdx = hIdx;
                    revertHunk(hIdx);
                  }}
                  data-testid="ide-compare-revert-hunk"
                >&gt;&gt;</button>
              {/if}
            </div>

            <div class="cmp-cell cmp-right-cell" class:is-empty={row.rightLineNo === null}>
              <span class="cmp-ln mono">{row.rightLineNo ?? ''}</span>
              <span class="cmp-code mono">{@html rHtml}</span>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .cmp-pane { display: flex; flex-direction: column; flex: 1; min-block-size: 0; min-inline-size: 0; overflow: hidden; background: var(--surface); }
  .cmp-toolbar {
    display: flex; align-items: center; gap: 6px;
    block-size: 32px; min-block-size: 32px; max-block-size: 32px;
    padding-inline: 10px; border-block-end: 1px solid var(--border);
    background: var(--surface); flex-shrink: 0;
  }
  .cmp-icon-btn {
    display: inline-flex; align-items: center; justify-content: center;
    inline-size: 22px; block-size: 22px; border: 0; border-radius: var(--radius-s);
    background: transparent; color: var(--text-dim); cursor: pointer; flex-shrink: 0;
  }
  .cmp-icon-btn:hover:not(:disabled) { background: var(--hover); color: var(--text); }
  .cmp-icon-btn:disabled { opacity: var(--disabled-opacity); cursor: default; }
  .cmp-diff-count { font-size: var(--fs-xs); font-weight: 600; color: var(--text); padding-inline: 4px; }
  .cmp-bar-btn { block-size: 22px; padding-block: 0; }
  .cmp-bar-btn.active-pill { background: var(--accent-soft); color: var(--accent-text); }
  .cmp-seg button { block-size: 22px; padding-inline: 8px; font-size: var(--fs-xs); }
  .cmp-identical-banner {
    display: flex; align-items: center; gap: 8px;
    block-size: 28px; min-block-size: 28px; padding-inline: 12px;
    background: var(--info-soft); color: var(--info);
    border-block-end: 1px solid var(--border); font-size: var(--fs-xs); font-weight: 500; flex-shrink: 0;
  }
  .cmp-col-heads {
    display: grid; grid-template-columns: minmax(0, 1fr) 28px minmax(0, 1fr);
    block-size: 26px; min-block-size: 26px;
    border-block-end: 1px solid var(--border); background: var(--surface-2); flex-shrink: 0;
  }
  .cmp-col-head { display: flex; align-items: center; gap: 6px; padding-inline: 10px; font-size: var(--fs-xs); font-weight: 600; color: var(--text); min-inline-size: 0; }
  .cmp-mid-head { border-inline: 1px solid var(--border); background: var(--surface-2); }
  .cmp-del-tag { margin-inline-start: auto; font-size: var(--fs-xs); color: var(--danger); font-weight: 500; }
  .cmp-edit-hint { margin-inline-start: auto; font-size: var(--fs-xs); font-weight: 400; }
  .cmp-unified-wrap { flex: 1; min-block-size: 0; overflow: auto; }
  .cmp-scroll { flex: 1; min-block-size: 0; overflow: auto; font-size: var(--fs-xs); line-height: 1.45; }
  .cmp-row { display: grid; grid-template-columns: minmax(0, 1fr) 28px minmax(0, 1fr); min-block-size: 20px; }
  .cmp-cell { display: flex; align-items: flex-start; min-inline-size: 0; overflow-x: auto; scrollbar-width: none; }
  .cmp-cell::-webkit-scrollbar { display: none; }
  .cmp-cell.is-empty { background: var(--surface-2); opacity: 0.55; }
  .cmp-ln { inline-size: 40px; padding-inline: 6px; text-align: end; color: var(--text-dim); user-select: none; flex-shrink: 0; }
  .cmp-code { flex: 1; padding-inline: 6px 10px; white-space: pre; color: var(--text); }
  .cmp-row.is-modify .cmp-left-cell { background: var(--info-soft); }
  .cmp-row.is-modify .cmp-right-cell { background: var(--accent-soft); }
  .cmp-row.is-delete .cmp-left-cell { background: var(--danger-soft); }
  .cmp-row.is-add .cmp-right-cell { background: var(--success-soft); }
  .cmp-code :global(.dw) { background: var(--warning-soft); border-radius: 2px; font-weight: 600; }
  .cmp-gutter {
    display: flex; align-items: center; justify-content: center;
    border-inline: 1px solid var(--border); background: var(--surface-2);
  }
  .cmp-chevron-btn {
    display: inline-flex; align-items: center; justify-content: center;
    inline-size: 24px; block-size: 18px; padding: 0; border: 1px solid var(--border);
    border-radius: var(--radius-s); background: var(--surface); color: var(--accent-text);
    font-family: var(--font-mono); font-size: var(--fs-xs); font-weight: 600; cursor: pointer;
  }
  .cmp-chevron-btn:hover { background: var(--accent-soft); border-color: var(--accent-text); }
  .cmp-live-split { display: grid; grid-template-columns: minmax(0, 1fr) 28px minmax(0, 1fr); flex: 1; min-block-size: 0; overflow: hidden; }
  .cmp-live-left { margin: 0; padding: 8px 10px; overflow: auto; font-size: var(--fs-xs); line-height: 1.45; background: var(--surface-2); color: var(--text); }
  .cmp-live-mid { display: flex; align-items: flex-start; justify-content: center; padding-block-start: 8px; border-inline: 1px solid var(--border); background: var(--surface-2); }
  .cmp-live-editor {
    margin: 0; padding: 8px 10px; border: 0; resize: none;
    overflow: auto; font-size: var(--fs-xs); line-height: 1.45;
    background: var(--surface); color: var(--text);
  }
  .cmp-live-editor:focus-visible { outline: none; box-shadow: inset 0 0 0 1px var(--accent-text); }
</style>
