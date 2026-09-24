<script lang="ts">
  // Home: Otto's one "desktop". The ambient backdrop fills the page; on it sit
  // a greeting + today's glance cards (HomeToday) and the active space's
  // widgets — up to 8 live boxes (Agents, Mission Control, DB dashboards,
  // Kubernetes, Insights, Usage) on a 12-column grid. Spaces 01–04 are Home's
  // views, shared with the floating bar through lib/stores/spaces.svelte.ts;
  // they slide (tabs, ←/→, swipe, ⌘K) and can cycle every 30 s. Any widget
  // zooms to fill the page. Layout is per device (see home.svelte.ts).
  import { untrack } from 'svelte';
  import { fly } from 'svelte/transition';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { registry } from '../../lib/commands.svelte';
  import HomeBox from './HomeBox.svelte';
  import HomeToday from './HomeToday.svelte';
  import { spaces, spaceNumber } from '../../lib/stores/spaces.svelte';
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
    const name = await confirmer.promptText('Space name', { title: `New space ${spaceNumber(home.views.length)}`, confirmLabel: 'Create', initial: spaces.label(home.views.length) });
    if (name) home.addView(name);
  }
  async function renameView(): Promise<void> {
    const v = home.active;
    if (!v) return;
    const name = await confirmer.promptText('Space name', { title: 'Rename space', confirmLabel: 'Rename', initial: v.name });
    if (name && name !== v.name) home.renameView(v.id, name);
  }
  async function removeView(): Promise<void> {
    const v = home.active;
    if (!v) return;
    if (await confirmer.ask(`Delete space “${v.name}” and its ${v.boxes.length} widget${v.boxes.length === 1 ? '' : 's'}?`, { title: 'Delete space' })) {
      home.removeView(v.id);
    }
  }
  function viewMenu(e: MouseEvent | KeyboardEvent): void {
    ctxMenu.show(e, [
      { label: 'Rename space…', icon: 'edit', action: () => void renameView() },
      { label: 'Add space', icon: 'plus', disabled: home.views.length >= MAX_VIEWS, action: () => void addView() },
      { separator: true },
      { label: 'Delete space', icon: 'trash', danger: true, disabled: home.views.length <= 1, action: () => void removeView() },
    ]);
  }

  // The floating bar (another window, or this one) switched space: restart
  // the rotation countdown like any manual navigation.
  let lastSpace = spaces.active;
  $effect(() => {
    const s = spaces.active;
    if (s === lastSpace) return;
    lastSpace = s;
    home.zoomedId = null;
    home.rotationEpoch += 1;
  });

  // A space renamed in the floating bar renames Home's view too (Home's own
  // renames reach the bar through home.svelte.ts → spaces.setNames).
  $effect(() => {
    const names = spaces.names;
    untrack(() => {
      home.views.forEach((v, i) => {
        if (names[i] && names[i] !== v.name) home.renameView(v.id, names[i]);
      });
    });
  });

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

  // ⌘K: switch spaces / toggle cycling from anywhere on the page.
  $effect(() => {
    const cmds = home.views.map((v, i) => ({
      id: `home.view-${v.id}`,
      title: `Home: space ${spaceNumber(i)} · ${v.name}`,
      group: 'Home',
      keywords: `home desktop space view slide ${spaceNumber(i)} ${i + 1}`,
      run: () => home.goTo(i),
    }));
    const unreg = registry.register('home', [
      ...cmds,
      { id: 'home.add-widget', title: 'Home: add widget…', group: 'Home', keywords: 'home desktop widget box add pin', run: () => (picking = true) },
      { id: 'home.rotate', title: home.autoRotate ? 'Home: stop cycling spaces' : 'Home: cycle spaces every 30 s', group: 'Home', keywords: 'home dashboard slide auto rotate 30 seconds spaces', run: () => home.setAutoRotate(!home.autoRotate) },
    ]);
    return unreg;
  });

  const full = $derived((home.active?.boxes.length ?? 0) >= MAX_BOXES);
</script>

<svelte:window onkeydown={onKey} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="home" role="region" aria-label="Home dashboard" ontouchstart={onTouchStart} ontouchend={onTouchEnd}>
  <PageHeader title="Home">
    {#snippet tabs()}
      <div class="spaces-bar">
        <div class="spaces segmented" role="tablist" aria-label="Spaces">
          {#each home.views as v, i (v.id)}
            <button
              class="space"
              class:active={i === home.activeIndex}
              role="tab"
              aria-selected={i === home.activeIndex}
              aria-label={v.name}
              title="Space {spaceNumber(i)} · {v.name}"
              onclick={() => home.goTo(i)}
              oncontextmenu={(e) => {
                home.goTo(i);
                viewMenu(e);
              }}
            >
              <span class="num">{spaceNumber(i)}</span>
              {#if i === home.activeIndex}<span class="sname">{v.name}</span>{/if}
            </button>
          {/each}
        </div>
        {#if home.views.length < MAX_VIEWS}
          <!-- Outside the tablist: a tablist may only contain tabs (axe aria-required-children). -->
          <button class="icon-btn" onclick={addView} title="Add space" aria-label="Add space"><Icon name="plus" size={14} /></button>
        {/if}
        {#if home.active}
          <button class="icon-btn" onclick={viewMenu} title="Space options" aria-label="Space options" aria-haspopup="menu"><Icon name="more" size={14} /></button>
        {/if}
      </div>
    {/snippet}
    {#snippet actions()}
      {#if home.views.length > 1}
        <button
          class="btn small ghost rot"
          class:on={home.autoRotate}
          onclick={() => home.setAutoRotate(!home.autoRotate)}
          title={home.autoRotate ? 'Cycling spaces every 30 s — click to pause' : 'Space cycling is paused — click to resume'}
          aria-pressed={home.autoRotate}
          aria-label="Auto-rotate spaces"
          data-label={home.autoRotate ? 'Pause cycling spaces' : 'Cycle spaces every 30 s'}
          data-icon={home.autoRotate ? 'refresh' : 'play'}
        >
          <Icon name={home.autoRotate ? 'refresh' : 'play'} size={12} />
          {home.autoRotate ? '30s' : 'Paused'}
        </button>
      {/if}
      <!-- One primary per page: an empty space's EmptyState owns "Add widget". -->
      {#if home.active && home.active.boxes.length > 0}
        <button class="btn small primary" onclick={() => (picking = true)} disabled={full} title={full ? `A space holds at most ${MAX_BOXES} widgets` : 'Add a widget to this space'}>
          <Icon name="plus" size={12} />Add widget
        </button>
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody padded={false} fill>
  {#if rotating}
    {#key home.rotationEpoch}
      <div class="progress" aria-hidden="true"><i style:animation-duration="{ROTATE_MS}ms"></i></div>
    {/key}
  {/if}

  <div class="stage">
    {#if home.zoomed && home.active}
      <div class="zoom">
        <HomeBox box={home.zoomed} viewId={home.active.id} zoomed />
      </div>
    {:else}
      <div class="desk" class:phone={viewport.isPhone}>
        <HomeToday />
        {#if home.active}
          {#key home.active.id}
            <div
              class="view"
              class:phone={viewport.isPhone}
              in:fly={{ x: 48 * home.slideDir, duration: 220 }}
              style:--row="{ROW_PX}px"
              style:--gap="{GAP_PX}px"
              aria-label="Space {spaceNumber(home.activeIndex)} · {home.active.name}"
            >
              {#if home.active.boxes.length === 0}
                <div class="empty">
                  <EmptyState
                    variant="page"
                    icon="grid"
                    title="This space is empty"
                    body="Pin live widgets here: your agents, Mission Control, usage, clusters or a database dashboard. Resize from the corner, drag the grip to reorder."
                    actionLabel="Add widget"
                    actionIcon="plus"
                    onaction={() => (picking = true)}
                  >
                    {#if kinds.length}
                      <div class="quick" aria-label="Quick add">
                        {#each kinds.slice(0, 4) as k (k.kind)}
                          <button class="chip quick-chip" onclick={() => addBox(k.kind)}><Icon name={k.icon} size={12} />{k.label}</button>
                        {/each}
                      </div>
                    {/if}
                  </EmptyState>
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
            <EmptyState variant="page" icon="grid" title="No spaces yet" body="Create a space, then pin widgets to it." actionLabel="Add space" actionIcon="plus" onaction={addView} />
          </div>
        {/if}
      </div>
    {/if}
  </div>
  </PageBody>
</div>

{#if picking}
  <Modal title="Add widget" width={520} onclose={() => (picking = false)}>
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
        <p class="dim">No widgets are available to your role.</p>
      {/each}
    </div>
  </Modal>
{/if}

<style>
  /* The desktop: the ambient backdrop painted full-bleed under the widgets
     (fixed to the viewport, so it continues seamlessly into the toolbar and
     sidebar glass). "None" — or reduced transparency — leaves plain --bg. */
  .home {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background-color: var(--bg);
    background-image: var(--ambient-image);
    background-attachment: fixed;
    background-size: cover;
    background-position: center;
  }
  .spaces-bar {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: none;
  }
  .spaces .space {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 30px;
    justify-content: center;
    font-variant-numeric: tabular-nums;
  }
  .spaces .num {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    letter-spacing: 0.02em;
  }
  .spaces .space.active .num {
    color: var(--accent-text);
    font-weight: 600;
  }
  .sname {
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .rot {
    gap: 4px;
  }
  .rot.on {
    color: var(--accent-text);
  }
  /* Cycling countdown: a hairline under the toolbar, only while cycling. */
  .progress {
    height: 2px;
    flex: none;
    overflow: hidden;
  }
  .progress i {
    display: block;
    height: 100%;
    width: 100%;
    background: color-mix(in srgb, var(--accent) 70%, transparent);
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
  @media (prefers-reduced-motion: reduce) {
    .progress i {
      animation: none;
      transform: scaleX(1);
      opacity: 0.4;
    }
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .desk {
    position: absolute;
    inset: 0;
    overflow-y: auto;
    /* Edges line up with the toolbar title (20px inset); the bottom clears
       the floating bar when it overlays the page (--fb-clearance). */
    padding: 22px 20px max(28px, var(--fb-clearance, 0px));
    display: flex;
    flex-direction: column;
    gap: 22px;
  }
  .desk.phone {
    padding: 16px 14px max(24px, var(--fb-clearance, 0px));
    gap: 16px;
  }
  .view {
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
    padding: 16px;
  }
  .empty {
    grid-column: 1 / -1;
    grid-row: span 5;
    display: grid;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow-card);
  }
  .quick {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
    margin-top: 12px;
  }
  .quick-chip {
    height: 24px;
    padding: 0 10px;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text);
    cursor: pointer;
  }
  .quick-chip:hover {
    border-color: var(--border-strong);
    background: var(--hover);
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
    color: var(--accent-text);
  }
  .kt {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .kt b {
    font-size: var(--fs-m);
  }
  .kt small {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    line-height: 1.35;
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
</style>
