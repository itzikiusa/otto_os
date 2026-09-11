<script lang="ts">
  // Home: a personal dashboard of up to 4 views, each a 12-column grid of up
  // to 8 live boxes (Agents, Mission Control, DB dashboards, Kubernetes,
  // Insights, Usage). Views slide (arrows, dots, ←/→, swipe) and auto-rotate
  // every 30 s; any box zooms to fill the page. Layout is per device (see
  // home.svelte.ts).
  import { fly } from 'svelte/transition';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { registry } from '../../lib/commands.svelte';
  import HomeBox from './HomeBox.svelte';
  import { home, MAX_BOXES, MAX_VIEWS, ROTATE_MS, ROW_PX, GAP_PX } from './home.svelte';
  import { HOME_KINDS, type HomeBoxKind } from './kinds';

  const can = (f: Parameters<typeof auth.can>[0]): boolean => auth.can(f, 'view');
  const kinds = $derived(HOME_KINDS.filter((k) => can(k.feature)));

  // First visit seeds a sensible view — only once the RBAC snapshot is in so
  // the seed reflects what this user can actually see.
  $effect(() => {
    if (auth.me) home.ensureDefault(can);
  });

  // ── Auto-rotation ────────────────────────────────────────────────────────
  // A single timeout per epoch; any manual navigation / toggle bumps the epoch
  // (store) which re-arms the countdown from zero. Paused while zoomed, mid-
  // gesture, or with a sheet open — those all read as "the user is busy here".
  let picking = $state(false);
  $effect(() => {
    const on = home.rotating && !picking;
    void home.rotationEpoch;
    if (!on) return;
    const h = window.setTimeout(() => {
      if (document.visibilityState === 'visible') home.next();
      else home.rotationEpoch += 1; // re-arm; try again next period
    }, ROTATE_MS);
    return () => clearTimeout(h);
  });
  const rotating = $derived(home.rotating && !picking);

  // ── Keyboard + swipe ─────────────────────────────────────────────────────
  function onKey(e: KeyboardEvent): void {
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)) return;
    if (e.key === 'Escape' && home.zoomedId) {
      e.preventDefault();
      home.zoomedId = null;
    } else if (e.key === 'ArrowLeft' && !e.metaKey && !e.altKey && !home.zoomedId && t?.getAttribute('role') !== 'slider') {
      home.prev();
    } else if (e.key === 'ArrowRight' && !e.metaKey && !e.altKey && !home.zoomedId && t?.getAttribute('role') !== 'slider') {
      home.next();
    }
  }
  let touchX: number | null = null;
  function onTouchStart(e: TouchEvent): void {
    touchX = e.touches[0]?.clientX ?? null;
  }
  function onTouchEnd(e: TouchEvent): void {
    if (touchX == null) return;
    const dx = (e.changedTouches[0]?.clientX ?? touchX) - touchX;
    touchX = null;
    if (Math.abs(dx) < 60) return;
    if (dx < 0) home.next();
    else home.prev();
  }

  // ── Views ────────────────────────────────────────────────────────────────
  async function addView(): Promise<void> {
    const name = await confirmer.promptText('View name', { title: 'New view', confirmLabel: 'Create', initial: `View ${home.views.length + 1}` });
    if (name) home.addView(name);
  }
  async function renameView(): Promise<void> {
    const v = home.active;
    if (!v) return;
    const name = await confirmer.promptText('Rename view', { title: 'Rename view', confirmLabel: 'Rename', initial: v.name });
    if (name && name !== v.name) home.renameView(v.id, name);
  }
  async function removeView(): Promise<void> {
    const v = home.active;
    if (!v) return;
    if (await confirmer.ask(`Delete view “${v.name}” and its ${v.boxes.length} box${v.boxes.length === 1 ? '' : 'es'}?`, { title: 'Delete view' })) {
      home.removeView(v.id);
    }
  }
  function viewMenu(e: MouseEvent | KeyboardEvent): void {
    ctxMenu.show(e, [
      { label: 'Rename view…', icon: 'edit', action: () => void renameView() },
      { label: 'Add view', icon: 'plus', disabled: home.views.length >= MAX_VIEWS, action: () => void addView() },
      { separator: true },
      { label: 'Delete view', icon: 'trash', danger: true, action: () => void removeView() },
    ]);
  }

  function addBox(kind: HomeBoxKind): void {
    const v = home.active;
    if (!v) return;
    home.addBox(v.id, kind);
    picking = false;
  }

  // ── Drag reorder ─────────────────────────────────────────────────────────
  let dragId: string | null = $state(null);
  function dropOn(targetId: string): void {
    const v = home.active;
    if (!v || !dragId || dragId === targetId) return;
    const to = v.boxes.findIndex((b) => b.id === targetId);
    if (to >= 0) home.moveBox(v.id, dragId, to);
    dragId = null;
  }

  // ⌘K: switch views / toggle rotation from anywhere on the page.
  $effect(() => {
    const cmds = home.views.map((v, i) => ({
      id: `home.view-${v.id}`,
      title: `Home: show “${v.name}”`,
      group: 'Home',
      keywords: 'home dashboard view slide',
      run: () => home.goTo(i),
    }));
    const unreg = registry.register('home', [
      ...cmds,
      { id: 'home.rotate', title: home.autoRotate ? 'Home: pause auto-rotation' : 'Home: resume auto-rotation', group: 'Home', keywords: 'home dashboard slide auto rotate 30 seconds', run: () => home.setAutoRotate(!home.autoRotate) },
    ]);
    return unreg;
  });

  const full = $derived((home.active?.boxes.length ?? 0) >= MAX_BOXES);
</script>

<svelte:window onkeydown={onKey} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="home" role="region" aria-label="Home dashboard" ontouchstart={onTouchStart} ontouchend={onTouchEnd}>
  <div class="bar">
    {#if home.views.length > 1}
      <button class="icon-btn" onclick={() => home.prev()} title="Previous view (←)" aria-label="Previous view"><Icon name="chevronLeft" size={14} /></button>
    {/if}
    <button class="vname" onclick={viewMenu} oncontextmenu={viewMenu} title="View options">
      <span class="ellipsis">{home.active?.name ?? 'Home'}</span>
      <Icon name="chevronDown" size={11} />
    </button>
    {#if home.views.length > 1}
      <button class="icon-btn" onclick={() => home.next()} title="Next view (→)" aria-label="Next view"><Icon name="chevronRight" size={14} /></button>
    {/if}
    <div class="dots" role="tablist" aria-label="Views">
      {#each home.views as v, i (v.id)}
        <button
          class="dot"
          class:on={i === home.activeIndex}
          role="tab"
          aria-selected={i === home.activeIndex}
          aria-label={v.name}
          title={v.name}
          onclick={() => home.goTo(i)}
        ></button>
      {/each}
    </div>
    {#if home.views.length < MAX_VIEWS}
      <!-- Outside the tablist: a tablist may only contain tabs (axe aria-required-children). -->
      <button class="dot add" onclick={addView} title="Add view" aria-label="Add view"><Icon name="plus" size={9} /></button>
    {/if}
    <span class="spacer"></span>
    {#if home.views.length > 1}
      <button
        class="btn small ghost rot"
        class:on={home.autoRotate}
        onclick={() => home.setAutoRotate(!home.autoRotate)}
        title={home.autoRotate ? 'Auto-rotate every 30 s — click to pause' : 'Auto-rotate is paused — click to resume'}
        aria-pressed={home.autoRotate}
        aria-label="Auto-rotate views"
      >
        <Icon name={home.autoRotate ? 'refresh' : 'play'} size={11} />
        {home.autoRotate ? '30s' : 'Paused'}
      </button>
    {/if}
    <button class="btn small" onclick={() => (picking = true)} disabled={!home.active || full} title={full ? `A view holds at most ${MAX_BOXES} boxes` : 'Add a box to this view'}>
      <Icon name="plus" size={11} />Add box
    </button>
  </div>
  {#if rotating}
    {#key home.rotationEpoch}
      <div class="progress" aria-hidden="true"><i style:animation-duration="{ROTATE_MS}ms"></i></div>
    {/key}
  {:else}
    <div class="progress idle" aria-hidden="true"></div>
  {/if}

  <div class="stage">
    {#if home.zoomed && home.active}
      <div class="zoom">
        <HomeBox box={home.zoomed} viewId={home.active.id} zoomed />
      </div>
    {:else if home.active}
      {#key home.active.id}
        <div
          class="view"
          class:phone={viewport.isPhone}
          in:fly={{ x: 48 * home.slideDir, duration: 220 }}
          style:--row="{ROW_PX}px"
          style:--gap="{GAP_PX}px"
        >
          {#if home.active.boxes.length === 0}
            <div class="empty">
              <EmptyState icon="grid" title="This view is empty" body="Add up to {MAX_BOXES} boxes — resize them from the corner, drag the grip to reorder, double-click a header to zoom." actionLabel="Add a box" onaction={() => (picking = true)} />
            </div>
          {:else}
            {#each home.active.boxes as b, i (b.id)}
              <HomeBox box={b} viewId={home.active.id} index={i} count={home.active.boxes.length} ondragbox={(id) => (dragId = id)} ondropon={dropOn} />
            {/each}
          {/if}
        </div>
      {/key}
    {:else}
      <div class="empty">
        <EmptyState icon="grid" title="No views yet" body="Create a view, then fill it with boxes." actionLabel="Add view" onaction={addView} />
      </div>
    {/if}
  </div>
</div>

{#if picking}
  <Modal title="Add box" width={520} onclose={() => (picking = false)}>
    <div class="kinds">
      {#each kinds as k (k.kind)}
        <button class="kind" onclick={() => addBox(k.kind)}>
          <span class="ki"><Icon name={k.icon} size={16} /></span>
          <span class="kt">
            <b>{k.label}</b>
            <small>{k.blurb}</small>
          </span>
        </button>
      {:else}
        <p class="dim">No box kinds are available to your role.</p>
      {/each}
    </div>
  </Modal>
{/if}

<style>
  .home {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--bg);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    flex: none;
    flex-wrap: wrap;
  }
  .vname {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 240px;
    padding: 4px 8px;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 14px;
    font-weight: 700;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .vname:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  .dots {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-inline-start: 6px;
  }
  .dot {
    width: 9px;
    height: 9px;
    padding: 0;
    border: none;
    border-radius: 50%;
    background: color-mix(in srgb, var(--text-dim) 40%, transparent);
    cursor: pointer;
    transition: background 130ms ease-out, transform 130ms ease-out;
  }
  .dot.on {
    background: var(--accent);
    transform: scale(1.25);
  }
  .dot.add {
    display: grid;
    place-items: center;
    width: 14px;
    height: 14px;
    background: transparent;
    border: 1px dashed var(--text-dim);
    color: var(--text-dim);
  }
  .dot.add:hover {
    color: var(--accent);
    border-color: var(--accent);
  }
  .spacer {
    flex: 1;
  }
  .rot {
    gap: 4px;
  }
  .rot.on {
    color: var(--accent);
  }
  .progress {
    height: 2px;
    flex: none;
    background: color-mix(in srgb, var(--border) 60%, transparent);
    overflow: hidden;
  }
  .progress i {
    display: block;
    height: 100%;
    width: 100%;
    background: var(--accent);
    transform-origin: left;
    animation: fill linear forwards;
  }
  :global([dir='rtl']) .progress i {
    transform-origin: right;
  }
  @keyframes fill {
    from {
      transform: scaleX(0);
    }
    to {
      transform: scaleX(1);
    }
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .view {
    position: absolute;
    inset: 0;
    overflow-y: auto;
    padding: 12px;
    display: grid;
    grid-template-columns: repeat(12, minmax(0, 1fr));
    grid-auto-rows: var(--row);
    grid-auto-flow: dense;
    gap: var(--gap);
    align-content: start;
  }
  .view.phone {
    grid-template-columns: minmax(0, 1fr);
  }
  .zoom {
    position: absolute;
    inset: 0;
    padding: 12px;
  }
  .empty {
    grid-column: 1 / -1;
    grid-row: span 5;
    display: grid;
    place-items: center;
  }
  .kinds {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 8px;
  }
  .kind {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    border-radius: var(--radius-m);
    cursor: pointer;
    text-align: start;
    font: inherit;
  }
  .kind:hover {
    border-color: var(--accent);
  }
  .ki {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    flex: none;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .kt {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .kt b {
    font-size: 12.5px;
  }
  .kt small {
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.35;
  }
  .dim {
    color: var(--text-dim);
    font-size: 12px;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
