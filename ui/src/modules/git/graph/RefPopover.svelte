<script lang="ts">
  // Collapsed-row ref popover: the branches (then tags) on one commit, opened
  // from a row's "▾ +N" expander. GraphView mounts it at the component root
  // (position:fixed) so it clears the scroll container. A full-screen backdrop
  // closes it on any outside click, Escape closes it (dialogFocus), focus moves
  // in, is trapped, and returns to the expander. Behaviour lives in
  // refPopoverState.svelte.ts; this file renders and places it.
  import { untrack } from 'svelte';
  import { rowMenu } from '../../../lib/rowMenu';
  import { dialogFocus } from '../../../lib/dialogFocus';
  import { ui } from '../../../lib/stores/ui.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { clampPopover, popoverMaxHeight, type RefMenu, type RefPopover } from './refPopoverState.svelte';

  interface Props {
    pop: RefPopover;
    menu: RefMenu;
  }
  let { pop, menu }: Props = $props();

  // Clamp into the viewport once the popover's size is known. The height cap
  // is set first (so the measured height already respects it), then the
  // corner is clamped on the next frame.
  let el = $state<HTMLDivElement | null>(null);
  let x = $state(0);
  let y = $state(0);
  let maxH = $state(320);
  $effect(() => {
    const m = menu;
    x = m.x;
    y = m.y;
    maxH = popoverMaxHeight(window.innerHeight, m.y);
    requestAnimationFrame(() => {
      if (!el || pop.menu !== m) return;
      const at = clampPopover(m, { w: el.offsetWidth, h: el.offsetHeight }, { w: window.innerWidth, h: window.innerHeight });
      x = at.x;
      y = at.y;
    });
  });

  // A modal surface: the shell's global shortcuts stand down while it's open.
  $effect(() => {
    untrack(() => ui.pushModal());
    return () => untrack(() => ui.popModal());
  });
</script>

<button type="button" class="ref-pop-backdrop" aria-label="Close" onclick={pop.close}></button>
<div class="ref-popover" role="dialog" aria-modal="true" aria-label="Commit references" use:dialogFocus={pop.close} bind:this={el} style="left: {x}px; top: {y}px; max-height: {maxH}px;">
  {#if menu.branches.length > 0}
    <div class="ref-pop-group">Branches</div>
    {#each menu.branches as chip (chip.kind + chip.label)}
      <div class="ref-action-row">
      <button use:rowMenu
        type="button"
        class="ref-pop-row kind-{chip.kind}"
        class:is-current={chip.current}
        class:is-worktree={chip.worktree && !chip.current}
        title={chip.current
          ? `Checked out · ${chip.label} — double-click to re-checkout`
          : chip.worktree
            ? `Worktree · ${chip.label} — double-click to open worktree`
            : `Double-click to checkout · ${chip.label}`}
        onclick={() => pop.rowSelect(menu.commit)}
        ondblclick={() => pop.rowCheckout(chip)}
        oncontextmenu={(e) => pop.rowMenu(e, chip)}
      >
        {#if chip.current}
          <Icon name="check" size={12} />
        {:else if chip.worktree}
          <Icon name="worktree" size={12} />
        {:else if chip.kind === 'remote' || chip.onRemote}
          <Icon name="globe" size={12} />
        {:else}
          <Icon name="branch" size={12} />
        {/if}
        <span class="ref-pop-label">{chip.label}</span>
        {#if chip.current}
          <span class="ref-pop-tag">current</span>
        {:else if chip.worktree}
          <span class="ref-pop-tag">worktree</span>
        {/if}
      </button>
      <button class="icon-btn ref-action" title="Actions for {chip.label}" aria-label="Actions for {chip.label}" aria-haspopup="menu" onclick={(e) => pop.rowMenu(e, chip)}><Icon name="more" size={13} /></button>
      </div>
    {/each}
  {/if}
  {#if menu.tags.length > 0}
    <div class="ref-pop-group">Tags</div>
    {#each menu.tags as chip (chip.kind + chip.label)}
      <div class="ref-action-row">
      <button use:rowMenu
        type="button"
        class="ref-pop-row kind-tag"
        title={`Double-click to checkout tag (detached) · ${chip.label}`}
        onclick={() => pop.rowSelect(menu.commit)}
        ondblclick={() => pop.rowCheckout(chip)}
        oncontextmenu={(e) => pop.rowMenu(e, chip)}
      >
        <Icon name="tag" size={12} />
        <span class="ref-pop-label">{chip.label}</span>
      </button>
      <button class="icon-btn ref-action" title="Actions for {chip.label}" aria-label="Actions for {chip.label}" aria-haspopup="menu" onclick={(e) => pop.rowMenu(e, chip)}><Icon name="more" size={13} /></button>
      </div>
    {/each}
  {/if}
</div>

<style>
  /* Full-screen click-catcher: any outside click closes the popover. */
  .ref-pop-backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--z-popover-backdrop);
    background: transparent;
    border: 0;
    padding: 0;
    cursor: default;
  }
  .ref-popover {
    position: fixed;
    z-index: var(--z-popover);
    min-width: 200px;
    max-width: 340px;
    overflow-y: auto;
    padding: 4px;
    background: var(--surface);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-l);
    box-shadow: var(--glass-shadow);
  }
  .ref-action-row {
    display: flex;
    align-items: stretch;
    min-width: 0;
  }
  .ref-action {
    flex-shrink: 0;
    align-self: center;
  }
  @media (max-width: 1024px) {
    .ref-action { min-width: 36px; min-height: 36px; }
  }
  .ref-pop-group {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    padding: 6px 8px 2px;
  }
  .ref-pop-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    text-align: start;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .ref-pop-row:hover,
  .ref-pop-row:focus-visible {
    background: var(--accent-soft);
    color: var(--accent-text);
    outline: none;
  }
  .ref-pop-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1 1 auto;
    min-width: 0;
  }
  .ref-pop-tag {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--accent-soft-strong);
    color: var(--accent-text);
  }
  .ref-pop-row.is-worktree {
    color: var(--worktree-text);
  }
  .ref-pop-row.is-worktree .ref-pop-tag {
    color: var(--worktree-text);
    border-color: color-mix(in srgb, var(--worktree) 40%, transparent);
  }
  .ref-pop-row:hover .ref-pop-tag,
  .ref-pop-row:focus-visible .ref-pop-tag {
    background: var(--surface);
    color: var(--accent-text);
    border-color: var(--accent-line);
  }
</style>
