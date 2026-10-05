<script lang="ts">
  // Right-side detail drawer for the AWS service views (EC2, RDS) — the same
  // look as the Kubernetes ResourceDrawer: header (state pill + name + id,
  // close), tab strip, scrollable body. The chrome (desktop column / phone
  // sheet, ✕, Esc, modal registration, focus) is the shared DockedDrawer;
  // this adds the kind/name/id/status header and the tab strip (←/→ move).
  import type { BadgeTone } from '../../lib/status';
  import { sentenceCase } from '../../lib/labels';
  import Badge from '../../lib/components/Badge.svelte';
  import type { Snippet } from 'svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import DockedDrawer from '../../lib/components/DockedDrawer.svelte';

  interface Props {
    /** Small uppercase kind label ("instance", "db instance"). */
    kind: string;
    name: string;
    /** Secondary id shown after the name (instance id, endpoint…). */
    id?: string;
    /** Status text (the raw AWS state; shown sentence-cased) + its Badge tone. */
    status?: string;
    statusTone?: BadgeTone;
    tabs: { id: string; label: string }[];
    tab: string;
    ontab: (id: string) => void;
    onclose: () => void;
    children: Snippet;
  }
  let { kind, name, id = '', status = '', statusTone = 'neutral', tabs, tab, ontab, onclose, children }: Props =
    $props();

</script>

<DockedDrawer open title="{kind} details" {onclose} testid="aws-drawer">
  {#snippet head()}
    <div class="dr-title">
      <span class="dr-kind">{kind}</span>
      <span class="dr-name" title={name}>{name}</span>
      {#if id && id !== name}<span class="dr-id mono" title={id}>{id}</span>{/if}
      {#if status}<Badge tone={statusTone} label={sentenceCase(status)} testid="aws-drawer-status" />{/if}
    </div>
  {/snippet}
  <div class="dr-tabs">
   <div class="segmented" role="tablist" aria-label="Detail tabs">
    {#each tabs as t (t.id)}
      <button
        role="tab"
        id="dr-tab-{t.id}"
        aria-controls="dr-panel"
        aria-selected={tab === t.id}
        tabindex={tab === t.id ? 0 : -1}
        class:active={tab === t.id}
        onclick={() => ontab(t.id)}
        onkeydown={onTabKey}
      >{t.label}</button>
    {/each}
   </div>
  </div>
  <div class="dr-body" id="dr-panel" role="tabpanel" aria-labelledby="dr-tab-{tab}">
    {@render children()}
  </div>
</DockedDrawer>

<style>
  .dr-title {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: var(--fs-m);
  }
  .dr-kind {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .dr-name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .dr-id {
    color: var(--text-dim);
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  /* The shared .segmented control, scrollable when the tabs outgrow the drawer. */
  .dr-tabs {
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }
  .dr-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
  }
</style>
