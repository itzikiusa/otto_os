<script lang="ts">
  // Right-side detail drawer for the AWS service views (EC2, RDS) — the same
  // look as the Kubernetes ResourceDrawer: header (state pill + name + id,
  // close), tab strip, scrollable body. Desktop: a fixed-width column next to
  // the table; phone: a full-screen sheet. Esc closes; ←/→ move between tabs.
  import type { Snippet } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';

  interface Props {
    /** Small uppercase kind label ("instance", "db instance"). */
    kind: string;
    name: string;
    /** Secondary id shown after the name (instance id, endpoint…). */
    id?: string;
    /** Status pill text + `pill-*` class suffix for colouring. */
    status?: string;
    statusClass?: string;
    tabs: { id: string; label: string }[];
    tab: string;
    ontab: (id: string) => void;
    onclose: () => void;
    children: Snippet;
  }
  let { kind, name, id = '', status = '', statusClass = '', tabs, tab, ontab, onclose, children }: Props =
    $props();

  function onKey(e: KeyboardEvent): void {
    if (e.key !== 'Escape') return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    e.stopPropagation();
    onclose();
  }

  function tabKey(e: KeyboardEvent, i: number): void {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
    e.preventDefault();
    const j = (i + (e.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length;
    ontab(tabs[j].id);
  }
</script>

<svelte:window onkeydown={onKey} />

<aside class="drawer" class:sheet={viewport.isPhone} aria-label="{kind} details" data-testid="aws-drawer">
  <header class="dr-head">
    <div class="dr-title">
      <span class="dr-kind">{kind}</span>
      <span class="dr-name" title={name}>{name}</span>
      {#if id && id !== name}<span class="dr-id mono" title={id}>{id}</span>{/if}
      {#if status}<span class="pill {statusClass}">{status}</span>{/if}
    </div>
    <button class="icon-btn" onclick={onclose} aria-label="Close details" title="Close (Esc)"><Icon name="x" size={14} /></button>
  </header>
  <div class="dr-tabs" role="tablist" aria-label="Detail tabs">
    {#each tabs as t, i (t.id)}
      <button
        role="tab"
        aria-selected={tab === t.id}
        tabindex={tab === t.id ? 0 : -1}
        class:active={tab === t.id}
        onclick={() => ontab(t.id)}
        onkeydown={(e) => tabKey(e, i)}
      >{t.label}</button>
    {/each}
  </div>
  <div class="dr-body" role="tabpanel">
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
    border-left: 1px solid var(--border);
  }
  .drawer.sheet {
    position: fixed;
    inset: 0;
    z-index: 40;
    width: auto;
    max-width: none;
    border-left: none;
  }
  .dr-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px 6px 14px;
    border-bottom: 1px solid var(--border);
  }
  .dr-title {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: 12.5px;
  }
  .dr-kind {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
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
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .pill {
    display: inline-block;
    font-size: 10.5px;
    font-weight: 600;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    text-transform: lowercase;
  }
  .pill.ok {
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
  }
  .pill.warn {
    color: var(--status-warn);
    background: color-mix(in srgb, var(--status-warn) 16%, transparent);
  }
  .pill.bad {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 14%, transparent);
  }
  .icon-btn {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .dr-tabs {
    display: flex;
    gap: 2px;
    padding: 4px 8px 0;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }
  .dr-tabs button {
    border: none;
    background: transparent;
    padding: 6px 10px;
    font-size: 12px;
    color: var(--text-dim);
    cursor: pointer;
    border-bottom: 2px solid transparent;
    white-space: nowrap;
  }
  .dr-tabs button.active {
    color: var(--text);
    border-bottom-color: var(--accent);
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
