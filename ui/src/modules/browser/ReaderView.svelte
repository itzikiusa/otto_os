<script lang="ts">
  // Reader-mode page render. The fetched page's markdown is rendered through
  // the SAME sanitizing renderer Vault's reading view uses (`renderNote` from
  // `modules/vault/mdRender.ts`) — never `{@html}` on raw page content. No
  // wikilink/attachment resolution applies here (this isn't a vault note), so
  // `resolve`/`assetUrl` are no-ops.
  //
  // Mark mode: toggling "Mark element" arms a click-to-annotate overlay on the
  // rendered `.page` tree. A click on a rendered element (while armed) builds a
  // short, stable CSS selector scoped to `.page` via `buildSelector` (shared
  // with the live-tab picker overlay — see `./selector.ts`; id/data-attr, else
  // tag + nth-of-type per ancestor step — the DOM here is our own sanitized
  // render, not the original page, so it's almost always the nth-of-type
  // fallback), snapshots its outerHTML (excerpt) + textContent (text), and
  // opens an inline note composer. Saving
  // calls `browser.createAnnotation`, which the store also appends locally for
  // instant feedback (see browser.svelte.ts). Existing marks for this URL are
  // re-highlighted after every render by re-resolving each annotation's
  // selector against the live DOM — a selector that no longer matches (page
  // content changed) is silently skipped rather than erroring.

  import { tick } from 'svelte';
  import { renderNote } from '../vault/mdRender';
  import { browser } from '../../lib/stores/browser.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { buildSelector } from './selector';
  import type { BrowserPage } from '../../lib/api/types';

  let {
    page,
    loading,
    error,
    onretry,
    onopenurl,
  }: {
    page: BrowserPage | null;
    loading: boolean;
    error: string;
    onretry?: () => void;
    /** Empty state's next step: focus the address bar (⌘L). */
    onopenurl?: () => void;
  } = $props();

  const html = $derived(
    page ? renderNote(page.markdown, { resolve: () => null, assetUrl: () => null }) : '',
  );

  let markMode = $state(false);
  let articleEl: HTMLElement | null = $state(null);
  let pending: { selector: string; excerpt: string; text: string } | null = $state(null);
  let noteText = $state('');
  let saving = $state(false);
  let markButton: HTMLButtonElement | undefined = $state();
  let pendingTarget: HTMLElement | null = null;

  function toggleMark(): void {
    markButton?.focus();
    markMode = !markMode;
    pending = null;
  }

  // While marking, the rendered blocks form a roving keyboard selection.
  // Restore their original semantics as soon as marking ends.
  $effect(() => {
    const root = articleEl;
    if (!markMode || !root) return;
    const blocks = Array.from(root.children).filter((el): el is HTMLElement => el instanceof HTMLElement);
    const previous = blocks.map((el) => [el.getAttribute('tabindex'), el.getAttribute('role')]);
    blocks.forEach((el, i) => { el.tabIndex = i === 0 ? 0 : -1; el.setAttribute('role', 'button'); });
    return () => blocks.forEach((el, i) => {
      for (const [j, attr] of ['tabindex', 'role'].entries()) {
        const value = previous[i][j];
        if (value === null) el.removeAttribute(attr); else el.setAttribute(attr, value);
      }
    });
  });

  async function markElement(target: Element): Promise<void> {
    if (!articleEl) return;
    pendingTarget = target.closest<HTMLElement>('[role="button"]');
    const selector = buildSelector(target, articleEl);
    const excerpt = target.outerHTML.slice(0, 2000);
    const text = (target.textContent || '').trim().slice(0, 500);
    pending = { selector, excerpt, text };
    noteText = '';
    await tick();
    document.querySelector<HTMLTextAreaElement>('.mark-composer textarea')?.focus();
  }

  function onArticleClick(e: MouseEvent): void {
    if (!markMode || !articleEl) return;
    const target = e.target as Element | null;
    if (!target || target === articleEl) return;
    e.preventDefault();
    void markElement(target);
  }

  function onArticleKey(e: KeyboardEvent): void {
    if (!markMode || !articleEl) return;
    const blocks = Array.from(articleEl.children) as HTMLElement[];
    const i = blocks.findIndex(el => el === e.target || el.contains(e.target as Node));
    if (i < 0) return;
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      void markElement(blocks[i]);
      return;
    }
    const next = e.key === 'ArrowDown' ? (i + 1) % blocks.length
      : e.key === 'ArrowUp' ? (i + blocks.length - 1) % blocks.length
      : e.key === 'Home' ? 0 : e.key === 'End' ? blocks.length - 1 : -1;
    if (next < 0) return;
    e.preventDefault();
    blocks.forEach((el, j) => { el.tabIndex = j === next ? 0 : -1; });
    blocks[next].focus();
  }

  function cancelMark(): void {
    pending = null;
    noteText = '';
    pendingTarget?.focus();
  }

  async function saveMark(): Promise<void> {
    if (!pending || !page || saving) return;
    saving = true;
    try {
      await browser.createAnnotation({
        url: page.url,
        selector: pending.selector,
        excerpt: pending.excerpt,
        text: pending.text,
        comment: noteText.trim(),
      });
      pending = null;
      noteText = '';
      markMode = false;
      await tick();
      markButton?.focus();
    } catch (e) {
      toasts.error('Failed to save mark', e instanceof Error ? e.message : undefined);
    } finally {
      saving = false;
    }
  }

  // Re-highlight existing marks for this page whenever the rendered HTML or
  // the annotation list changes.
  $effect(() => {
    void html;
    const anns = browser.annotations;
    const root = articleEl;
    if (!root) return;
    root.querySelectorAll('[data-mark-id]').forEach((el) => {
      el.removeAttribute('data-mark-id');
      el.classList.remove('marked');
    });
    for (const a of anns) {
      if (page && a.url !== page.url) continue;
      let el: Element | null = null;
      try {
        el = root.querySelector(a.selector);
      } catch {
        el = null;
      }
      if (el) {
        el.setAttribute('data-mark-id', a.id);
        el.classList.add('marked');
      }
    }
  });
</script>

<div class="reader">
  {#if loading || error}
    <!-- Loading → skeleton; a failed fetch → inline cause + Retry (never a
         dead end, never a raw toast). -->
    <LoadState what="this page" variant="page" {loading} {error} empty {onretry} />
  {:else if !page}
    <EmptyState
      variant="page"
      icon="compass"
      title="Open a page"
      body="Type a URL in the address bar to read it here as clean text. Mark passages to hand them to an agent."
      actionLabel={onopenurl ? 'Enter a URL' : undefined}
      actionIcon="search"
      onaction={onopenurl}
    />
  {:else}
    {#if page.degraded}
      <div class="degraded">
        Degraded fetch — no JavaScript ran ({page.engine}). Some content may be missing.
      </div>
    {/if}

    <div class="toolbar">
      {#if markMode}<span class="mark-hint" role="status">Choose a passage · ↑↓ to move · Enter to mark · Esc to stop</span>{/if}
      <button
        class="btn small"
        class:mark-on={markMode}
        bind:this={markButton}
        onclick={toggleMark}
        aria-pressed={markMode}
        title={markMode ? 'Stop marking' : 'Mark a passage to hand to an agent'}
      >
        <Icon name="target" size={12} />
        {markMode ? 'Stop marking' : 'Mark passage'}
      </button>
    </div>

    <!-- eslint-disable-next-line svelte/no-static-element-interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <article
      class="page md-body"
      class:mark-armed={markMode}
      bind:this={articleEl}
      onclick={onArticleClick}
      onkeydown={onArticleKey}
    >
      <h1 class="page-title">{page.title || page.url}</h1>
      <!-- eslint-disable-next-line svelte/no-at-html-tags -->
      {@html html}
    </article>

    {#if pending}
      <div class="mark-composer">
        <p class="composer-excerpt">{pending.text.slice(0, 140)}</p>
        <textarea
          bind:value={noteText}
          placeholder="Add a note for the agent (optional)"
          aria-label="Note for this mark"
          rows="2"
          spellcheck="false"
          onkeydown={(e) => {
            if (e.key === 'Escape') { e.preventDefault(); cancelMark(); }
            else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); void saveMark(); }
          }}
        ></textarea>
        <div class="composer-actions">
          <button class="btn" onclick={cancelMark}>Cancel</button>
          <button class="btn primary" disabled={saving} onclick={saveMark} title="Save mark (⌘↩)">
            {saving ? 'Saving…' : 'Save mark'}
          </button>
        </div>
      </div>
    {/if}
  {/if}
</div>

<svelte:window onkeydown={(e) => { if (e.key === 'Escape' && markMode && !pending) { markMode = false; markButton?.focus(); } }} />

<style>
  .reader {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 16px 20px 32px;
    position: relative;
  }
  .degraded {
    max-width: 72ch;
    margin: 0 auto 12px;
    background: var(--warning-soft);
    color: var(--text);
    border-radius: var(--radius-s);
    padding: 8px 12px;
    font-size: var(--fs-s);
  }
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    max-width: 72ch;
    margin: 0 auto 8px;
  }
  .mark-hint {
    color: var(--accent-text);
    font-size: var(--fs-s);
  }
  .btn.mark-on {
    background: var(--accent-soft);
    color: var(--accent-text);
    border-color: var(--accent);
  }
  .page {
    max-width: 72ch;
    margin: 0 auto;
    color: var(--text);
    line-height: 1.6;
    overflow-wrap: anywhere;
  }
  .page.mark-armed {
    cursor: crosshair;
  }
  .page.mark-armed :global(*:hover),
  .page.mark-armed :global([role="button"]:focus-visible) {
    outline: 1px dashed var(--accent);
    outline-offset: 2px;
  }
  .page :global(.marked) {
    background: color-mix(in srgb, var(--warning) 30%, transparent);
    border-radius: 2px;
  }
  .page-title {
    font-size: var(--fs-2xl);
    font-weight: 600;
    line-height: 1.25;
    margin: 0 0 16px;
  }
  .mark-composer {
    position: sticky;
    bottom: 0;
    max-width: 72ch;
    margin: 12px auto 0;
    padding: 10px 12px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-m);
    background: var(--surface);
    box-shadow: var(--shadow);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .composer-excerpt {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mark-composer textarea {
    width: 100%;
    box-sizing: border-box;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 10px;
    font: inherit;
    font-size: var(--fs-m);
    resize: vertical;
  }
  .composer-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
</style>
