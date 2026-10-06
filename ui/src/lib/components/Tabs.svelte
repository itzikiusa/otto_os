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
</script>

<div class="otabs" class:small={size === 's'}>
  <div class="otabs-list" role="tablist" aria-label={label} data-testid={testid}>
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
  {#if trailing}<div class="otabs-trailing">{@render trailing()}</div>{/if}
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
