<script lang="ts">
  // Docked detail column for list/detail pages (AWS EC2/RDS, Kubernetes
  // resources). Desktop: an inline column of `width` beside the content — not
  // modal, Esc closes it unless an input or a stacked dialog owns the key.
  // Phone (or `modal`): the shell Drawer as a full-width right sheet, which
  // owns the modal plumbing (pushModal, focus trap, Esc, ✕). Both modes share
  // one header row — the `title` heading, or the richer `head` snippet — so a
  // caller never re-implements the drawer chrome.
  import type { Snippet } from 'svelte';
  import Drawer from '../../shell/Drawer.svelte';
  import Icon from './Icon.svelte';
  import { viewport } from '../stores/viewport.svelte';

  interface Props {
    open: boolean;
    /** Heading and accessible name ("Instance details"). */
    title: string;
    onclose: () => void;
    /** Desktop column width (CSS length); capped at 55% of the row. */
    width?: string;
    /** Present as the modal sheet; defaults to phone width. */
    modal?: boolean;
    /** Richer header content (kind, name, status badge) in place of the title. */
    head?: Snippet;
    testid?: string;
    children: Snippet;
  }
  let { open, title, onclose, width = '460px', modal, head, testid, children }: Props = $props();

  const sheet = $derived(modal ?? viewport.isPhone);

  function onKey(e: KeyboardEvent): void {
    if (!open || sheet || e.key !== 'Escape' || e.defaultPrevented) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)) return;
    // A dialog stacked over the column (confirm, picker) owns its own Esc.
    if (document.querySelector('[aria-modal="true"]')) return;
    e.stopPropagation();
    onclose();
  }
</script>

<svelte:window onkeydown={onKey} />

{#if sheet}
  <Drawer {open} side="right" {title} {head} width="100vw" {onclose}>
    <div class="dd-body" data-testid={testid}>{@render children()}</div>
  </Drawer>
{:else if open}
  <aside class="docked" style:--dd-width={width} aria-label={title} data-testid={testid}>
    <header class="dd-head">
      {#if head}<div class="dd-head-main">{@render head()}</div>{:else}<h2>{title}</h2>{/if}
      <button class="icon-btn" onclick={onclose} aria-label="Close {title}" title="Close {title}" aria-keyshortcuts="Escape">
        <Icon name="x" size={14} />
      </button>
    </header>
    <div class="dd-body">{@render children()}</div>
  </aside>
{/if}

<style>
  .docked {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    width: var(--dd-width);
    max-width: 55%;
    height: 100%;
    min-height: 0;
    min-width: 0;
    background: var(--surface);
    border-inline-start: 1px solid var(--border);
  }
  .dd-head {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding-block: var(--sp-4); padding-inline: var(--sp-5) var(--sp-4);
    border-bottom: 1px solid var(--border);
  }
  .dd-head h2 {
    flex: 1;
    min-width: 0;
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dd-head-main {
    flex: 1;
    min-width: 0;
  }
  .dd-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
  }
  @media (max-width: 1024px) {
    .docked {
      width: min(var(--dd-width), 380px);
    }
  }
</style>
