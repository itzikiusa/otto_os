<script lang="ts">
  import { rowMenu } from '../../lib/rowMenu';
  import { plural } from '../../lib/plural';
  // Home: one centred reading column on the ambient backdrop — the greeting,
  // "Needs you", "Working", then Up next / Recent when they hold something
  // (HomeToday), with the floating bar docked under it as the one primary.
  // Below, a "Pinned" section holds the active space's widgets — up to 8 live
  // boxes (Agents, DB dashboards, Kubernetes, Insights, Usage) on a 12-column
  // grid — with the spaces, Add widget and cycling in its header. Spaces 01–04 are Home's
  // views, shared with the floating bar through lib/stores/spaces.svelte.ts;
  // they slide (tabs, ←/→, swipe, ⌘K) and can cycle every 30 s. Any widget
  // zooms to fill the page. Layout is per device (see home.svelte.ts).
  import { untrack } from 'svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import HomeToday from './HomeToday.svelte';
  import { spaces, spaceNumber } from '../../lib/stores/spaces.svelte';
  import { home, MAX_BOXES, MAX_VIEWS, ROTATE_MS, ROW_PX, GAP_PX } from './home.svelte';
  import { HOME_KINDS, type HomeBoxKind } from '../../lib/home/kinds';

  // The Pinned widgets sit below the fold and are empty for most opens: their
  // frame loads as its own chunk the first time a space has a widget, behind a
  // placeholder of the box's own grid size (no layout jump).
  const HomeBoxView = lazyComponent(() => import('./HomeBox.svelte'));

  const can = (f: Parameters<typeof auth.can>[0]): boolean => auth.can(f, 'view');
  const kinds = $derived(HOME_KINDS.filter((k) => can(k.feature)));

  // First visit seeds a sensible view — only once the RBAC snapshot and the
  // workspace are in, so the seed reflects what this user can see and which
  // workspace's dashboards to look for (first visit only).
  $effect(() => {
    if (auth.me && (ws.currentId || ws.listSettled)) home.ensureDefault(can);
  });

  // ── Auto-rotation ────────────────────────────────────────────────────────
  // A single timeout per epoch; any manual navigation / toggle bumps the epoch
  // (store) which re-arms the countdown from zero. Paused while zoomed, mid-
  // gesture, or with a sheet open — those all read as "the user is busy here".
  let picking = $state(false);
  let hovering = $state(false);
  let focusInside = $state(false);
  $effect(() => {
    const on = home.rotating && !picking && !hovering && !focusInside;
    void home.rotationEpoch;
    if (!on) return;
    const h = window.setTimeout(() => {
      if (document.visibilityState === 'visible') home.next();
      else home.rotationEpoch += 1; // re-arm; try again next period
    }, ROTATE_MS);
    return () => clearTimeout(h);
  });
  const rotating = $derived(home.rotating && !picking && !hovering && !focusInside);

  // ── Keyboard + swipe ─────────────────────────────────────────────────────
  function onKey(e: KeyboardEvent): void {
    if (e.defaultPrevented || ui.overlayOpen || ui.navDrawerOpen || ctxMenu.open) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)) return;
    // A composite widget in a box (window radios, tabs) owns its arrow keys —
    // they must never slide the whole Home view away under it.
    if (t?.closest?.('[role="radiogroup"],[role="tablist"]')) return;
    const rtl = typeof document !== 'undefined' && document.documentElement.dir === 'rtl';
    if (e.key === 'Escape' && home.zoomedId) {
      e.preventDefault();
      home.zoomedId = null;
    } else if (e.key === 'ArrowLeft' && !e.metaKey && !e.altKey && !home.zoomedId && t?.getAttribute('role') !== 'slider') {
      if (rtl) home.next();
      else home.prev();
    } else if (e.key === 'ArrowRight' && !e.metaKey && !e.altKey && !home.zoomedId && t?.getAttribute('role') !== 'slider') {
      if (rtl) home.prev();
      else home.next();
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
    const rtl = typeof document !== 'undefined' && document.documentElement.dir === 'rtl';
    if (rtl ? dx > 0 : dx < 0) home.next();
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
    if (await confirmer.ask(`Delete space “${v.name}” and its ${plural(v.boxes.length, 'widget')}?`, { title: 'Delete space', confirmLabel: 'Delete', danger: true })) {
      home.removeView(v.id);
    }
  }
  function viewMenu(e: MouseEvent | KeyboardEvent): void {
    ctxMenu.show(e, [
      { label: 'Rename space…', icon: 'edit', action: () => void renameView() },
      { label: 'Add space', icon: 'plus', disabled: home.views.length >= MAX_VIEWS, action: () => void addView() },
      { separator: true },
      { label: 'Delete space…', icon: 'trash', danger: true, disabled: home.views.length <= 1, action: () => void removeView() },
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
      title: `Go to space ${spaceNumber(i)} · ${v.name}`,
      group: 'Home',
      keywords: `home desktop space view slide ${spaceNumber(i)} ${i + 1}`,
      run: () => home.goTo(i),
    }));
    const unreg = registry.register('home', [
      ...cmds,
      { id: 'home.add-widget', title: 'Add widget…', group: 'Home', keywords: 'home desktop widget box add pin', run: () => (picking = true) },
      { id: 'home.rotate', title: home.autoRotate ? 'Stop cycling spaces' : 'Cycle spaces every 30 s', group: 'Home', keywords: 'home dashboard slide auto rotate 30 seconds spaces', run: () => home.setAutoRotate(!home.autoRotate) },
    ]);
    return unreg;
  });

  const full = $derived((home.active?.boxes.length ?? 0) >= MAX_BOXES);
  // The zoomed box of the space on screen (null = the normal grid).
  const zoomId = $derived.by(() => {
    const id = home.zoomedId;
    return id && home.active?.boxes.some((b) => b.id === id) ? id : null;
  });
</script>

<svelte:window onkeydown={onKey} />

<!-- Touch swipe between spaces is a touch shortcut (the Spaces tabs and ←/→
     are the other paths); a named region takes touch handlers without a lint. -->
<div class="home" role="region" aria-label="Home dashboard" ontouchstart={onTouchStart} ontouchend={onTouchEnd}>
  <PageHeader title="Home" />
  <PageBody padded={false} fill>
  {#if rotating}
    {#key home.rotationEpoch}
      <div class="progress" aria-hidden="true"><i style:animation-duration="{ROTATE_MS}ms"></i></div>
    {/key}
  {/if}

  <div class="stage" role="group" aria-label="Home widgets">
    <!-- Every space stays mounted (review 06 F3): switching only toggles
         `hidden` + `inert`, and the off-screen boxes pause their pollers but
         keep their data — a slide no longer remounts and re-fetches every
         widget (or re-runs a "manual only" DB widget’s SQL). Zoom is in place
         too: the zoomed box fills the desk and everything else is hidden,
         inert and paused — no second copy, no re-fetch. -->
    <div class="desk" class:phone={viewport.isPhone} class:zooming={!!zoomId}>
      <div class="today" hidden={!!zoomId} inert={!!zoomId}><HomeToday /></div>
      <div class="pinned-head read-col" hidden={!!zoomId} inert={!!zoomId}>
        <h2 class="pinned-title">Pinned</h2>
        <div class="spaces-bar">
          <div class="spaces segmented" role="tablist" aria-label="Spaces">
            {#each home.views as v, i (v.id)}
              <button use:rowMenu
                class="space"
                onkeydown={onTabKey}
                class:active={i === home.activeIndex}
                role="tab"
                aria-selected={i === home.activeIndex}
                tabindex={i === home.activeIndex ? 0 : -1}
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
        <span class="pinned-grow"></span>
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
        <!-- Not the primary: the composer is Home's one primary. -->
        {#if home.active && home.active.boxes.length > 0}
          <button class="btn small ghost" onclick={() => (picking = true)} disabled={full} title={full ? `A space holds at most ${MAX_BOXES} widgets` : 'Add a widget to this space'}>
            <Icon name="plus" size={12} />Add widget
          </button>
        {/if}
      </div>
      <!-- Hovering or focusing a widget pauses cycling (the user is reading) —
           the widgets only, never the inbox or the Pinned controls above. -->
      <div
        class="views"
        role="presentation"
        onpointerover={() => (hovering = true)}
        onpointerout={(e) => {
          if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node | null)) hovering = false;
        }}
        onfocusin={() => (focusInside = true)}
        onfocusout={(e) => {
          if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node | null)) focusInside = false;
        }}
      >
      {#if home.active}
        {#each home.views as v, vi (v.id)}
          {@const on = v.id === home.active.id}
          <div
            class="view read-col"
            class:phone={viewport.isPhone}
            hidden={!on}
            inert={!on}
            style:--row="{ROW_PX}px"
            style:--gap="{GAP_PX}px"
            style:--slide-from="{48 * home.slideDir}px"
            aria-label="Space {spaceNumber(vi)} · {v.name}"
          >
            {#if v.boxes.length === 0}
              <!-- Calm: one line, never a second primary beside the composer. -->
              <div class="empty">
                <p class="empty-line">This space is empty — pin live widgets here.</p>
                <div class="quick" aria-label="Quick add">
                  {#each kinds.slice(0, 4) as k (k.kind)}
                    <button class="chip quick-chip" onclick={() => addBox(k.kind)}><Icon name={k.icon} size={12} />{k.label}</button>
                  {/each}
                  <button class="chip quick-chip" onclick={() => (picking = true)}><Icon name="plus" size={12} />Add widget</button>
                </div>
              </div>
            {:else}
              {#each v.boxes as b, i (b.id)}
                {@const z = on && zoomId === b.id}
                {#if HomeBoxView.component}
                  {@const Box = HomeBoxView.component}
                  <Box box={b} viewId={v.id} index={i} count={v.boxes.length} zoomed={z} covered={on && !!zoomId && !z} active={on && (!zoomId || z)} ondragbox={(id) => (dragId = id)} ondropon={dropOn} />
                {:else}
                  <div class="box-ph" style:grid-column={viewport.isPhone ? null : `span ${b.w}`} style:grid-row="span {b.h}">
                    {#if HomeBoxView.error}
                      <LoadState what="the widget" variant="compact" error={HomeBoxView.error} empty={false} onretry={() => HomeBoxView.retry()} />
                    {/if}
                  </div>
                {/if}
              {/each}
            {/if}
          </div>
        {/each}
      {:else}
        <div class="empty">
          <EmptyState variant="page" icon="grid" title="No spaces yet" body="Create a space, then pin widgets to it." actionLabel="Add space" actionIcon="plus" onaction={addView} />
        </div>
      {/if}
      </div>
    </div>
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
    letter-spacing: .06em;
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
    background: var(--accent-line-strong);
    transform-origin: left;
    animation: otto-grow-x linear forwards;
  }
  :global([dir='rtl']) .progress i {
    transform-origin: right;
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
    padding: var(--sp-9) 0 max(28px, var(--fb-clearance, 0px));
    display: flex;
    flex-direction: column;
    gap: var(--sp-6);
  }
  .desk.phone {
    padding: var(--sp-6) 0 max(24px, var(--fb-clearance, 0px));
    gap: var(--sp-6);
  }
  /* Zoomed: the desk is the zoomed box's containing block. Everything else in
     it is hidden, so nothing scrolls and the box sits over the whole stage. */
  .desk.zooming {
    overflow: hidden;
    --zoom-inset: 16px;
  }
  .desk.phone.zooming {
    --zoom-inset: 8px;
  }
  /* A slide-in mid-flight carries a transform, which would make the view
     (zero height while zoomed) the box's containing block. */
  .desk.zooming .view {
    animation: none;
  }
  .views {
    display: contents;
  }
  /* A widget while its frame's chunk loads: the box's own grid size, calm. */
  .box-ph {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    font-size: var(--fs-m);
    color: var(--text-dim);
  }
  .today[hidden],
  .pinned-head[hidden] {
    display: none;
  }
  /* "Pinned": the widgets' section, its controls in one calm row. */
  .pinned-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--sp-4);
    margin-block-start: var(--sp-6);
  }
  .pinned-title {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
  }
  .pinned-grow {
    flex: 1;
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
  /* `hidden` must win over the grid/flex display above. */
  .view[hidden] {
    display: none;
  }
  /* The slide-in plays each time a space is shown: a CSS animation restarts
     when its element leaves display:none (no remount needed). The shared
     edge slide (--slide-from is set per direction on the element) plus the
     entrance fade. */
  .view:not([hidden]) {
    animation:
      otto-slide-in var(--dur-enter) var(--ease-out),
      otto-fade-in var(--dur-enter) var(--ease-out);
  }
  @media (prefers-reduced-motion: reduce) {
    .view:not([hidden]) {
      animation: none;
    }
  }
  .empty {
    grid-column: 1 / -1;
    grid-row: span 1;
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
  }
  .empty-line {
    margin: 0;
    font-size: var(--fs-m);
    color: var(--text);
  }
  .quick {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-3);
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
    border-color: var(--border-strong);
    background: var(--hover);
  }
  .ki {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    flex: none;
    border-radius: var(--radius-s);
    /* Neutral like the widget header's icon tile: accent means selection or
       the primary action, not "this is a widget kind". */
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text-dim);
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
