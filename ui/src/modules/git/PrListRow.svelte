<script lang="ts">
  // One PR list row (PrList): state glyph, #N, title, lazy +/- counts, author,
  // branches, labels, CI, relative time. A hover/selection checkbox sits over
  // the glyph; the rest of the row is one button (⌘-click = background tab,
  // shift-click = select a range, middle-click = background tab).
  import type { PrState, PrSummary } from '../../lib/api/types';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { ciOf } from './pr-query';
  import type { PrStat } from './pr-stats.svelte';

  interface Props {
    pr: PrSummary;
    stat: PrStat | null;
    active: boolean;
    selected: boolean;
    /** Anything in the list is selected: every checkbox stays visible. */
    selecting: boolean;
    /** The PR has an open tab (and isn't the active one): a dot. */
    openTab: boolean;
    /** Attachment that counts the row's +/- once it scrolls into view. */
    observe: ((el: Element) => void | (() => void)) | null;
    ontoggle: (shift: boolean) => void;
    onrowclick: (e: MouseEvent) => void;
    onopen: (background: boolean) => void;
  }
  let { pr, stat, active, selected, selecting, openTab, observe, ontoggle, onrowclick, onopen }: Props = $props();

  const STATE_LABEL: Record<PrState, string> = { open: 'Open', merged: 'Merged', declined: 'Declined', all: 'All' };
  const ci = $derived.by((): { icon: IconName; cls: string; label: string } | null => {
    const c = ciOf(pr);
    if (c === 'passing') return { icon: 'check', cls: 'ok', label: 'CI passing' };
    if (c === 'failing') return { icon: 'x', cls: 'bad', label: 'CI failing' };
    if (c === 'pending') return { icon: 'clock', cls: 'warn', label: 'CI running' };
    return null;
  });
  const isDraft = $derived(!!pr.draft && pr.state === 'open');
  const tone = $derived(isDraft ? 'draft' : pr.state === 'merged' ? 'merged' : pr.state === 'declined' ? 'declined' : 'open');
  const toneLabel = $derived(isDraft ? 'Draft' : (STATE_LABEL[pr.state] ?? pr.state));
  const labels = $derived(pr.labels ?? []);
  const initial = $derived((pr.author.trim().charAt(0) || '?').toUpperCase());
  const compact = (n: number): string => (n >= 10_000 ? `${Math.round(n / 1000)}k` : n.toLocaleString());
  const noop = () => {};
</script>

<div class="pr-row" role="listitem" class:active class:selected class:selecting {@attach observe ?? noop}>
  <input
    type="checkbox"
    class="pr-check"
    checked={selected}
    aria-label="Select #{pr.number}"
    title="Select to open several together (shift-click for a range)"
    onclick={(e) => ontoggle(e.shiftKey)}
  />
  <button
    class="pr-open"
    aria-current={active ? 'true' : undefined}
    title="Open #{pr.number} · ⌘-click opens it in a background tab"
    onclick={onrowclick}
    onauxclick={(e) => {
      if (e.button === 1) {
        e.preventDefault();
        onopen(true);
      }
    }}
    onkeydown={(e) => {
      if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        onopen(true);
      }
    }}
  >
    <span class="pr-glyph {tone}" role="img" aria-label={toneLabel} title={toneLabel}>
      <Icon name={pr.state === 'merged' ? 'merge' : 'pr'} size={14} />
    </span>
    <span class="pr-body">
      <span class="pr-line1">
        <span class="pr-num mono">#{pr.number}</span>
        <span class="pr-title" dir="auto">{pr.title}</span>
        {#if openTab && !active}
          <span class="pr-opendot" title="Open in a tab" aria-label="Open in a tab" role="img"></span>
        {/if}
        {#if stat}
          <span class="pr-stat mono" title="{stat.files} files · +{stat.add} −{stat.del}">
            <span class="add">+{compact(stat.add)}</span>
            <span class="del">−{compact(stat.del)}</span>
          </span>
        {/if}
      </span>
      <span class="pr-line2">
        <span class="pr-avatar" aria-hidden="true">{initial}</span>
        <span class="pr-author">{pr.author}</span>
        <span class="pr-branches mono" title="{pr.source_branch} → {pr.target_branch}"
          >{pr.source_branch} <span class="dir-arrow">→</span> {pr.target_branch}</span
        >
        {#each labels.slice(0, 2) as l (l)}
          <span class="pr-label">{l}</span>
        {/each}
        {#if labels.length > 2}<span class="pr-label" title={labels.slice(2).join(', ')}>+{labels.length - 2}</span>{/if}
        {#if ci}
          <span class="ci-chip {ci.cls}" title={ci.label} role="img" aria-label={ci.label}><Icon name={ci.icon} size={12} /></span>
        {/if}
        <span class="grow"></span>
        <span class="pr-updated" title="Updated {new Date(pr.updated_at).toLocaleString()}">{rel(pr.updated_at)}</span>
      </span>
    </span>
  </button>
</div>

<style>
  .pr-row {
    position: relative;
    display: flex;
    align-items: stretch;
    border-radius: var(--radius-s);
  }
  .pr-row:hover,
  .pr-row:focus-within {
    background: var(--surface-2);
  }
  .pr-row.active {
    background: color-mix(in srgb, var(--accent) 12%, var(--surface-2));
  }
  .pr-row.selected {
    background: var(--accent-soft);
  }
  /* The select checkbox sits over the glyph and appears on hover/focus or
     while anything is selected. */
  .pr-check {
    position: absolute;
    inset-inline-start: 9px;
    top: 10px;
    width: 14px;
    height: 14px;
    margin: 0;
    opacity: 0;
    z-index: 1;
    cursor: pointer;
  }
  .pr-row:hover .pr-check,
  .pr-check:focus-visible,
  .pr-row.selecting .pr-check {
    opacity: 1;
  }
  .pr-row:hover .pr-glyph,
  .pr-row:has(.pr-check:focus-visible) .pr-glyph,
  .pr-row.selecting .pr-glyph {
    visibility: hidden;
  }
  .pr-open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px;
    border: none;
    background: none;
    color: var(--text);
    text-align: start;
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .pr-open:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .pr-glyph {
    flex: none;
    display: inline-flex;
    width: 16px;
    margin-top: 1px;
  }
  .pr-glyph.open {
    color: var(--success);
  }
  .pr-glyph.merged {
    color: var(--accent-text);
  }
  .pr-glyph.declined {
    color: var(--danger);
  }
  .pr-glyph.draft {
    color: var(--text-dim);
  }
  .pr-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .pr-line1,
  .pr-line2 {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .pr-line1 {
    font-size: var(--fs-m);
  }
  .pr-num {
    flex: none;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .pr-title {
    min-width: 0;
    flex: 1 1 auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pr-opendot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
  .pr-stat {
    flex: none;
    display: inline-flex;
    gap: 4px;
    font-size: var(--fs-xs);
  }
  .add {
    color: var(--success);
  }
  .del {
    color: var(--danger);
  }
  .pr-line2 {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .pr-avatar {
    flex: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent-text);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: var(--fs-xs);
    font-weight: 600;
    line-height: 1;
  }
  .pr-author {
    flex: none;
    max-width: 40%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pr-branches {
    min-width: 0;
    flex: 0 1 auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pr-label {
    flex: none;
    max-width: 90px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0 6px;
    border-radius: 999px;
    border: 1px solid var(--border);
    font-size: var(--fs-xs);
    line-height: 15px;
  }
  .pr-updated {
    flex: none;
    white-space: nowrap;
  }
  .ci-chip {
    flex: none;
    display: inline-flex;
    align-items: center;
  }
  .ci-chip.ok {
    color: var(--success);
  }
  .ci-chip.bad {
    color: var(--danger);
  }
  .ci-chip.warn {
    color: var(--warning);
  }
  /* source→target separator mirrors in place under RTL. */
  .dir-arrow {
    display: inline-block;
  }
  :global([dir='rtl']) .dir-arrow {
    transform: scaleX(-1);
  }

  /* ≤1024px: no hover, so the checkbox is always there (at touch size) and
     the meta line wraps instead of clipping. */
  @media (max-width: 1024px) {
    .pr-open {
      padding: 10px 8px;
    }
    .pr-check {
      opacity: 1;
      width: 18px;
      height: 18px;
      top: 12px;
    }
    .pr-glyph {
      visibility: hidden;
    }
    .pr-line1 {
      font-size: var(--fs-l);
    }
    .pr-line2 {
      flex-wrap: wrap;
      font-size: var(--fs-s);
    }
    .pr-line2 .grow {
      display: none;
    }
    .pr-branches {
      white-space: normal;
      overflow-wrap: anywhere;
    }
  }
</style>
