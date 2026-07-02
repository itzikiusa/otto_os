<script lang="ts">
  // ListPane — the chrome shared by every "220px list + detail" tab on the
  // Product page (ChatTab/RefineTab's chat lists, MockupsTab's mockup list):
  // a bordered sidebar-style column with a title + action-buttons header and a
  // scrollable body. The body renders `empty` INSTEAD OF `children` only when
  // `isEmpty` is true and an `empty` snippet was actually passed — callers that
  // fold loading/error states into their own `children` markup (e.g. RefineTab,
  // which must keep its discovery-run picker visible even with zero threads)
  // simply omit `isEmpty`/`empty` and keep full control.
  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    width?: number;
    /** When true (and `empty` is given), render `empty` instead of `children`. */
    isEmpty?: boolean;
    actions?: Snippet;
    children?: Snippet;
    empty?: Snippet;
  }
  let { title, width = 220, isEmpty = false, actions, children, empty }: Props = $props();
</script>

<aside class="list-pane" style="width: {width}px">
  <div class="pane-head">
    <span class="pane-title">{title}</span>
    {#if actions}
      <div class="pane-actions">{@render actions()}</div>
    {/if}
  </div>
  <div class="pane-body">
    {#if isEmpty && empty}
      {@render empty()}
    {:else if children}
      {@render children()}
    {/if}
  </div>
</aside>

<style>
  .list-pane {
    flex-shrink: 0;
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .pane-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 6px;
    padding: 8px 10px 6px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .pane-title {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .pane-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .pane-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }
</style>
