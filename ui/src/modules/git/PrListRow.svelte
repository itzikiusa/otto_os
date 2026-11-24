<script lang="ts">
  // One PR list row (PrList) on the kit's InboxRow (patterns.md §10):
  //   1. #N (+ lazy +/- counts) … what the PR needs, in words (prStatus);
  //   2. the title at --fs-l (bold when it needs you);
  //   3. branch → base · author · time.
  // The row is one button (⌘-click / middle-click = background tab, shift-
  // click = select a range). "Select" is the hover action on line 3 and the
  // row's right-click / long-press menu, so several can open together.
  import type { PrSummary } from '../../lib/api/types';
  import InboxRow from '../../lib/components/InboxRow.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import type { WordTone } from '../../lib/statusWords';
  import { prRowWord } from './prStatus';
  import type { PrStat } from './pr-stats.svelte';

  interface Props {
    pr: PrSummary;
    stat: PrStat | null;
    active: boolean;
    selected: boolean;
    /** It needs you (prStatus.needsYou): the title goes bold. */
    needs: boolean;
    /** The PR has an open tab (and isn't the active one): a faint tint. */
    openTab: boolean;
    /** Attachment that counts the row's +/- once it scrolls into view. */
    observe: ((el: Element) => void | (() => void)) | null;
    ontoggle: (shift: boolean) => void;
    onrowclick: (e: MouseEvent) => void;
    onopen: (background: boolean) => void;
  }
  let { pr, stat, active, selected, needs, openTab, observe, ontoggle, onrowclick, onopen }: Props = $props();

  const word = $derived(prRowWord(pr));
  const tone = $derived<WordTone>(word.tone === 'calm' ? '' : word.tone);
  const compact = (n: number): string => (n >= 10_000 ? `${Math.round(n / 1000)}k` : n.toLocaleString());
  const noop = () => {};
  /** Middle-click anywhere on the row: a background tab. */
  function middleClick(el: HTMLElement): () => void {
    const h = (e: MouseEvent): void => { if (e.button === 1) { e.preventDefault(); onopen(true); } };
    el.addEventListener('auxclick', h);
    return () => el.removeEventListener('auxclick', h);
  }
  function menu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Open in a background tab', icon: 'external', action: () => onopen(true) },
      { label: selected ? 'Deselect' : 'Select', icon: 'check', action: () => ontoggle(false) },
    ]);
  }
</script>

<div class="pr-row" role="listitem" data-pr={pr.number} class:picked={selected} {@attach observe ?? noop} {@attach middleClick}>
  <InboxRow title={pr.title} word={word.word} {tone} strong={needs} selected={active} inPane={openTab && !active} onPage
    tip="Open #{pr.number} · ⌘-click opens it in a background tab" statusTestid="pr-word" onclick={onrowclick} oncontextmenu={menu}>
    {#snippet context()}
      <span class="pr-num">#{pr.number}</span>
      {#if stat}<span class="pr-stat" title="{stat.files} files · +{stat.add} −{stat.del}"><span class="add">+{compact(stat.add)}</span> <span class="del">−{compact(stat.del)}</span></span>{/if}
    {/snippet}
    {#snippet line3()}
      <span class="pr-branches mono" dir="ltr" title="{pr.source_branch} → {pr.target_branch}">{pr.source_branch} → {pr.target_branch}</span>
      <span class="pr-meta">· {pr.author} · <span title="Updated {new Date(pr.updated_at).toLocaleString()}">{rel(pr.updated_at)}</span></span>
    {/snippet}
    {#snippet actions()}
      <input type="checkbox" class="pr-check" checked={selected} aria-label="Select #{pr.number}"
        title="Select to open several together (shift-click for a range)" onclick={(e) => ontoggle(e.shiftKey)} />
    {/snippet}
  </InboxRow>
</div>

<style>
  .pr-row { min-width: 0; }
  .pr-row.picked :global(.tr-row) { --tr-tint: var(--accent-soft); }
  .pr-num { flex: none; font-variant-numeric: tabular-nums; }
  .pr-stat { flex: none; font-variant-numeric: tabular-nums; }
  .add { color: var(--success); }
  .del { color: var(--danger); }
  .pr-branches { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pr-meta { flex: none; white-space: nowrap; }
  .pr-check { margin: 0 var(--sp-2); inline-size: 14px; block-size: 14px; cursor: pointer; }
</style>
