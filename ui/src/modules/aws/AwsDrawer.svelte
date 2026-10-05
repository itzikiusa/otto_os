<script lang="ts">
  // Right-side detail drawer for the AWS service views (EC2, RDS) — the same
  // look as the Kubernetes ResourceDrawer: header (state pill + name + id,
  // close), tab strip, scrollable body. Desktop: a fixed-width column next to
  // the table; phone: a full-screen sheet. Esc closes; ←/→ move between tabs.
  // The phone sheet sits on the Modal layer (above BottomNav) and registers
  // with ui.pushModal() so the native browser webview hides under it.
  import type { BadgeTone } from '../../lib/status';
  import { sentenceCase } from '../../lib/labels';
  import Badge from '../../lib/components/Badge.svelte';
  import type { Snippet } from 'svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import { untrack } from 'svelte';
  import { dialogFocus } from '../../lib/dialogFocus';
  import Icon from '../../lib/components/Icon.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ui } from '../../lib/stores/ui.svelte';

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

  let drawerEl = $state<HTMLElement | null>(null);

  // Phone: a full-screen sheet is a modal overlay — register it (untracked:
  // pushModal reads the counter it bumps) and move focus into it.
  $effect(() => {
    if (!viewport.isPhone || !drawerEl) return;
    untrack(() => ui.pushModal());
    const focus = dialogFocus(drawerEl, onclose);
    return () => { focus.destroy(); untrack(() => ui.popModal()); };
  });

  function onKey(e: KeyboardEvent): void {
    if (viewport.isPhone || e.key !== 'Escape' || e.defaultPrevented) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    // A dialog stacked over the drawer (confirm, picker) owns its own Esc.
    const modals = document.querySelectorAll('[aria-modal="true"]');
    if (Array.from(modals).some((m) => m !== drawerEl)) return;
    e.stopPropagation();
    onclose();
  }

</script>

<svelte:window onkeydown={onKey} />

<aside
  bind:this={drawerEl}
  class="drawer"
  class:sheet={viewport.isPhone}
  role={viewport.isPhone ? 'dialog' : undefined}
  aria-modal={viewport.isPhone ? 'true' : undefined}
  aria-label="{kind} details"
  data-testid="aws-drawer"
>
  <header class="dr-head">
    <div class="dr-title">
      <span class="dr-kind">{kind}</span>
      <span class="dr-name" title={name}>{name}</span>
      {#if id && id !== name}<span class="dr-id mono" title={id}>{id}</span>{/if}
      {#if status}<Badge tone={statusTone} label={sentenceCase(status)} testid="aws-drawer-status" />{/if}
    </div>
    <button class="icon-btn dr-close" onclick={onclose} aria-label="Close details" title="Close (Esc)"><Icon name="x" size={14} /></button>
  </header>
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
</aside>

<style>
  .drawer {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    min-width: 0;
    width: 460px;
    max-width: 55%;
    flex-shrink: 0;
    background: var(--surface);
    border-inline-start: 1px solid var(--border);
  }
  .drawer.sheet {
    position: fixed;
    inset: 0;
    /* The Modal layer: above BottomNav and its More sheet. */
    z-index: var(--z-modal);
    width: auto;
    max-width: none;
    border-inline-start: none;
  }
  .dr-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-block: 8px 6px; padding-inline: 14px 10px;
    border-bottom: 1px solid var(--border);
  }
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
  @media (max-width: 1024px) {
    .drawer {
      width: 380px;
    }
  }
</style>
