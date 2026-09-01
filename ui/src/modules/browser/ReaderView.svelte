<script lang="ts">
  // Reader-mode page render. The fetched page's markdown is rendered through
  // the SAME sanitizing renderer Vault's reading view uses (`renderNote` from
  // `modules/vault/mdRender.ts`) — never `{@html}` on raw page content. No
  // wikilink/attachment resolution applies here (this isn't a vault note), so
  // `resolve`/`assetUrl` are no-ops.
  //
  // Mark mode: toggling "Mark element" arms a click-to-annotate overlay on the
  // rendered `.page` tree. A click on a rendered element (while armed) builds a
  // short, stable CSS selector scoped to `.page` (tag + nth-of-type per
  // ancestor step — the DOM here is our own sanitized render, not the
  // original page, so there's no id/class to lean on), snapshots its outerHTML
  // (excerpt) + textContent (text), and opens an inline note composer. Saving
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
  import type { BrowserPage } from '../../lib/api/types';

  let { page, loading, error }: { page: BrowserPage | null; loading: boolean; error: string } =
    $props();

  const html = $derived(
    page ? renderNote(page.markdown, { resolve: () => null, assetUrl: () => null }) : '',
  );

  let markMode = $state(false);
  let articleEl: HTMLElement | null = $state(null);
  let pending: { selector: string; excerpt: string; text: string } | null = $state(null);
  let noteText = $state('');
  let saving = $state(false);

  function toggleMark(): void {
    markMode = !markMode;
    pending = null;
  }

  /** Build a selector for `el`, scoped to `root`, as a chain of
   *  `tag:nth-of-type(n)` steps from `root` down to `el`. Stops climbing once
   *  it reaches `root` (exclusive) or runs out of parents. */
  function buildSelector(el: Element, root: Element): string {
    const steps: string[] = [];
    let cur: Element | null = el;
    while (cur && cur !== root) {
      const tag = cur.tagName.toLowerCase();
      const parent: Element | null = cur.parentElement;
      if (!parent) {
        steps.unshift(tag);
        break;
      }
      const currentTag = cur.tagName;
      const siblings = Array.from(parent.children).filter((c) => c.tagName === currentTag);
      const idx = siblings.indexOf(cur) + 1;
      steps.unshift(siblings.length > 1 ? `${tag}:nth-of-type(${idx})` : tag);
      cur = parent === root ? null : parent;
    }
    return steps.join(' > ');
  }

  async function onArticleClick(e: MouseEvent): Promise<void> {
    if (!markMode || !articleEl) return;
    const target = e.target as Element | null;
    if (!target || target === articleEl) return;
    e.preventDefault();
    const selector = buildSelector(target, articleEl);
    const excerpt = target.outerHTML.slice(0, 2000);
    const text = (target.textContent || '').trim().slice(0, 500);
    pending = { selector, excerpt, text };
    noteText = '';
    await tick();
    document.querySelector<HTMLTextAreaElement>('.mark-composer textarea')?.focus();
  }

  function cancelMark(): void {
    pending = null;
    noteText = '';
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
  {#if loading}
    <p class="muted">Loading…</p>
  {:else if error}
    <div class="error">{error}</div>
  {:else if !page}
    <div class="empty">
      <p>Enter a URL above to fetch it in reader mode.</p>
    </div>
  {:else}
    {#if page.degraded}
      <div class="degraded">
        Degraded fetch — no JavaScript ran ({page.engine}). Some content may be missing.
      </div>
    {/if}

    <div class="toolbar">
      <button
        class="mark-toggle"
        class:active={markMode}
        onclick={toggleMark}
        aria-pressed={markMode}
      >
        <Icon name="note" size={13} />
        {markMode ? 'Marking…' : 'Mark element'}
      </button>
    </div>

    <!-- eslint-disable-next-line svelte/no-static-element-interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <article
      class="page"
      class:mark-armed={markMode}
      bind:this={articleEl}
      onclick={onArticleClick}
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
          placeholder="Add a note"
          rows="2"
          spellcheck="false"
        ></textarea>
        <div class="composer-actions">
          <button class="btn" onclick={cancelMark}>Cancel</button>
          <button class="btn primary" disabled={saving} onclick={saveMark}>
            {saving ? 'Saving…' : 'Save mark'}
          </button>
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .reader {
    flex: 1;
    overflow-y: auto;
    padding: 1rem 1.25rem 2rem;
    position: relative;
  }
  .muted {
    color: var(--text-dim);
    padding: 0.75rem 0;
  }
  .error {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 12%, transparent);
    border-radius: var(--radius-s);
    padding: 0.6rem 0.75rem;
    font-size: 0.85rem;
  }
  .empty {
    color: var(--text-dim);
    padding: 2rem 0;
    text-align: center;
  }
  .degraded {
    background: color-mix(in srgb, var(--status-warn) 16%, transparent);
    color: var(--status-warn);
    border-radius: var(--radius-s);
    padding: 0.5rem 0.75rem;
    font-size: 0.82rem;
    margin-bottom: 0.75rem;
  }
  .toolbar {
    display: flex;
    justify-content: flex-end;
    max-width: 72ch;
    margin: 0 auto 0.5rem;
  }
  .mark-toggle {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text-dim);
    font-size: 0.78rem;
    padding: 0.3rem 0.6rem;
    cursor: pointer;
  }
  .mark-toggle:hover {
    color: var(--text);
  }
  .mark-toggle.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
    border-color: var(--accent);
  }
  .page {
    max-width: 72ch;
    margin: 0 auto;
    color: var(--text);
    line-height: 1.6;
  }
  .page.mark-armed {
    cursor: crosshair;
  }
  .page.mark-armed :global(*:hover) {
    outline: 1px dashed var(--accent);
    outline-offset: 2px;
  }
  .page :global(.marked) {
    background: color-mix(in srgb, yellow 35%, transparent);
    border-radius: 2px;
  }
  .page-title {
    font-size: 1.4rem;
    margin: 0 0 1rem;
  }
  .mark-composer {
    max-width: 72ch;
    margin: 0.75rem auto 0;
    padding: 0.6rem 0.75rem;
    border: 1px solid var(--accent);
    border-radius: var(--radius-m);
    background: var(--surface);
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .composer-excerpt {
    margin: 0;
    font-size: 0.8rem;
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
    padding: 0.4rem 0.6rem;
    font: inherit;
    font-size: 0.85rem;
    resize: vertical;
  }
  .composer-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.4rem;
  }
  .btn {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: 0.8rem;
    padding: 0.3rem 0.65rem;
    cursor: pointer;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--accent-contrast, #fff);
    border-color: var(--accent);
  }
  .btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
</style>
