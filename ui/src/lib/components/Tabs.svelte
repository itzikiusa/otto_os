<script module lang="ts">
  import type { IconName } from './Icon.svelte';

  export interface TabItem<I extends string = string> {
    id: I;
    label: string;
    icon?: IconName;
    /** Shown after the label in --text-dim ("Pending 3"). */
    count?: number | string;
    disabled?: boolean;
    /** Tooltip — e.g. why a tab is disabled. */
    title?: string;
  }
</script>

<script lang="ts" generics="T extends string">
  // The one in-pane tab strip (components.md §3): an underline under the
  // selected tab, `role="tablist"` + `role="tab"` / `aria-selected`, roving
  // tabindex with ←/→ (visual direction — RTL flips), Home/End, disabled tabs
  // skipped (lib/tabKeys.ts). Page-level tabs still go in PageHeader's `tabs`
  // snippet; 2–5 options that pick a VALUE stay a `.segmented`.
  //
  //   <Tabs label="Queue details" tabs={[{ id: 'messages', label: 'Messages' }, …]}
  //         value={tab} onchange={(id) => (tab = id)} idBase="sqs" />
  //   <div role="tabpanel" id="sqs-panel-{tab}" aria-labelledby="sqs-tab-{tab}">…</div>
  //
  // `onchange` is the single place that switches the view, for pointer and
  // keyboard alike (arrow keys click the next tab). Pass `activate: 'click'`
  // when the switch is async or can be refused (an unsaved-changes guard), so
  // focus never lands on a tab that didn't take.
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import { tabKeys, type TabKeyOptions } from '../tabKeys';

  interface Props {
    tabs: readonly TabItem<T>[];
    value: T;
    onchange: (id: T) => void;
    /** aria-label of the tablist ("Queue details"). */
    label: string;
    /** When set, tab `i` gets id `{idBase}-tab-{i}` and aria-controls
     *  `{idBase}-panel-{i}` — give the panel that id + aria-labelledby. */
    idBase?: string;
    size?: 'm' | 's';
    activate?: TabKeyOptions['activate'];
    /** Extra controls at the end of the strip (outside the tablist). */
    trailing?: Snippet;
    testid?: string;
  }
  let { tabs, value, onchange, label, idBase, size = 'm', activate = 'focus', trailing, testid }: Props = $props();
  const onkeydown = $derived(tabKeys({ activate }));

  // ── Overflow affordance (S19-303) ─────────────────────────────────────────
  // The strip scrolls horizontally with its scrollbar hidden, so a hidden tab
  // used to give no hint it existed: fade the edge(s) that hide tabs, map a
  // plain mouse wheel to horizontal scroll, keep the selected tab in view, and
  // move the `trailing` controls to their own row when tabs + trailing don't
  // fit side by side (measured from the tabs' own widths, so it can't flap).
  let rootEl = $state<HTMLDivElement>();
  let listEl = $state<HTMLDivElement>();
  let trailEl = $state<HTMLDivElement>();
  let fadeStart = $state(false);
  let fadeEnd = $state(false);
  let wrapTrailing = $state(false);

  function measure(): void {
    const list = listEl;
    if (!list) return;
    const max = list.scrollWidth - list.clientWidth;
    const rtl = getComputedStyle(list).direction === 'rtl';
    const pos = rtl ? -list.scrollLeft : list.scrollLeft; // 0 = at the start edge
    fadeStart = max > 1 && pos > 1;
    fadeEnd = max > 1 && pos < max - 1;
    const root = rootEl;
    const trail = trailEl;
    if (root && trail) {
      let tabsW = 0;
      for (const el of Array.from(list.children)) tabsW += (el as HTMLElement).offsetWidth + 2;
      const cs = getComputedStyle(root);
      const inner = root.clientWidth - parseFloat(cs.paddingInlineStart || '0') - parseFloat(cs.paddingInlineEnd || '0');
      wrapTrailing = tabsW + trail.offsetWidth + 8 > inner;
    } else {
      wrapTrailing = false;
    }
  }

  $effect(() => {
    void tabs; // re-observe when the tab set changes
    const root = rootEl;
    const list = listEl;
    if (!root || !list || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => measure());
    ro.observe(root);
    ro.observe(list);
    for (const el of Array.from(list.children)) ro.observe(el);
    return () => ro.disconnect();
  });

  // Keep the selected tab visible when `value` changes in code (and after the
  // tab list itself changes); `nearest` never scrolls the page vertically.
  $effect(() => {
    void value;
    void tabs.length;
    const list = listEl;
    if (!list) return;
    queueMicrotask(() => {
      const on = list.querySelector<HTMLElement>('[aria-selected="true"]');
      on?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
      measure();
    });
  });

  /** A vertical mouse wheel scrolls an overflowing strip sideways (a
   *  non-passive listener: it has to preventDefault the page scroll). */
  $effect(() => {
    const list = listEl;
    if (!list) return;
    list.addEventListener('wheel', onwheel, { passive: false });
    return () => list.removeEventListener('wheel', onwheel);
  });
  function onwheel(e: WheelEvent): void {
    const list = listEl;
    if (!list || list.scrollWidth <= list.clientWidth) return;
    if (Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
    list.scrollLeft += getComputedStyle(list).direction === 'rtl' ? -e.deltaY : e.deltaY;
    e.preventDefault();
  }
</script>

<div class="otabs" class:small={size === 's'} class:wrap-trailing={wrapTrailing} bind:this={rootEl}>
  <div
    class="otabs-list"
    class:fade-start={fadeStart}
    class:fade-end={fadeEnd}
    role="tablist"
    aria-label={label}
    data-testid={testid}
    bind:this={listEl}
    onscroll={measure}
  >
    {#each tabs as t (t.id)}
      <button
        type="button"
        role="tab"
        class="otab"
        class:on={t.id === value}
        id={idBase ? `${idBase}-tab-${t.id}` : undefined}
        aria-controls={idBase ? `${idBase}-panel-${t.id}` : undefined}
        aria-selected={t.id === value}
        tabindex={t.id === value ? 0 : -1}
        disabled={t.disabled}
        title={t.title || undefined}
        onclick={() => {
          if (t.id !== value) onchange(t.id);
        }}
        {onkeydown}
      >
        {#if t.icon}<Icon name={t.icon} size={size === 's' ? 12 : 13} />{/if}
        <span class="otab-label">{t.label}</span>
        {#if t.count !== undefined && t.count !== ''}<span class="otab-count">{t.count}</span>{/if}
      </button>
    {/each}
  </div>
  {#if trailing}<div class="otabs-trailing" bind:this={trailEl}>{@render trailing()}</div>{/if}
</div>

<style>
  .otabs {
    display: flex;
    align-items: stretch;
    gap: 8px;
    padding-inline: 8px;
    border-block-end: 1px solid var(--border);
    min-width: 0;
    flex-shrink: 0;
  }
  .otabs-list {
    display: flex;
    gap: 2px;
    flex: 1;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .otabs-list::-webkit-scrollbar {
    display: none;
  }
  /* Edge fades: a tab hides past this edge. Masks read alpha only, so the
     opaque stop just needs an opaque colour token. Physical directions are
     resolved per writing direction below (RTL's start edge is the right). */
  .otabs-list.fade-end {
    -webkit-mask-image: linear-gradient(to right, var(--text) calc(100% - 28px), transparent);
    mask-image: linear-gradient(to right, var(--text) calc(100% - 28px), transparent);
  }
  .otabs-list.fade-start {
    -webkit-mask-image: linear-gradient(to left, var(--text) calc(100% - 28px), transparent);
    mask-image: linear-gradient(to left, var(--text) calc(100% - 28px), transparent);
  }
  .otabs-list:dir(rtl).fade-end {
    -webkit-mask-image: linear-gradient(to left, var(--text) calc(100% - 28px), transparent);
    mask-image: linear-gradient(to left, var(--text) calc(100% - 28px), transparent);
  }
  .otabs-list:dir(rtl).fade-start {
    -webkit-mask-image: linear-gradient(to right, var(--text) calc(100% - 28px), transparent);
    mask-image: linear-gradient(to right, var(--text) calc(100% - 28px), transparent);
  }
  .otabs-list.fade-start.fade-end {
    -webkit-mask-image: linear-gradient(to right, transparent, var(--text) 28px, var(--text) calc(100% - 28px), transparent);
    mask-image: linear-gradient(to right, transparent, var(--text) 28px, var(--text) calc(100% - 28px), transparent);
  }
  /* Tabs + trailing controls don't fit side by side: the controls take their
     own row under the strip instead of covering tabs. */
  .otabs.wrap-trailing {
    flex-wrap: wrap;
  }
  .otabs.wrap-trailing .otabs-list {
    flex-basis: 100%;
  }
  .otabs.wrap-trailing .otabs-trailing {
    padding-block: 4px;
  }
  .otabs-trailing {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .otab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    /* The underline sits on the strip's border (−1 px) so it reads as one line. */
    margin-block-end: -1px;
    border: 0;
    border-block-end: 2px solid transparent;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    white-space: nowrap;
    cursor: pointer;
    transition: color var(--dur-fast) ease-out, border-color var(--dur-fast) ease-out;
  }
  .small .otab {
    padding: 4px 8px;
    font-size: var(--fs-xs);
  }
  .otab:hover:not(:disabled) {
    color: var(--text);
  }
  .otab.on {
    color: var(--text);
    font-weight: 500;
    border-block-end-color: var(--accent);
  }
  .otab:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
    border-radius: var(--radius-s);
  }
  .otab:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .otab-count {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
</style>
