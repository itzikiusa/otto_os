<script lang="ts">
  // Otto School — the Home "Classrooms" box: a real 3D school of your agent
  // sessions. You start in the corridor (one door per workspace, with a live
  // sign: who needs you, who's working); walk in and every session is a kid at
  // a desk (one character per provider) whose PC shows that session's live
  // terminal, the robot headmaster patrols and checks on them, idle kids
  // wander around, archived sessions sit on the detention bench.
  //
  // Data: the cross-workspace `GET /sessions?archived=false` (incl. engine
  // rows) + the last archived ones for the bench, refreshed on session events
  // and overlaid with the workspace store's live rows; status / needs-you from
  // the store's event-fed maps. Live screens: `GET /sessions/{id}/screen`, for
  // the kids the camera can see in the room you're in (school/screens.ts).
  //
  // The canvas is never the only path: the same kids are a list of real
  // buttons (List view, no-WebGL, and screen readers / keyboard in 3D view),
  // and the selected kid's card carries every action as a button.
  //
  // At a glance: a "needs you" queue (school-wide, real buttons — the list
  // carries the same section), status pins over kids (scene.ts), ✋ / ❗
  // badges over doors, a jump
  // into the busiest room from the corridor, and a hover tooltip with the
  // session's state, last activity and a crisp peek at its live terminal.
  // The scene pauses whenever the widget is scrolled off screen, its space is
  // inactive or the document is hidden; a lost WebGL context remounts itself
  // (twice) before it asks for Retry.
  import { onDestroy, untrack } from 'svelte';
  import { isNativePane } from '../../../lib/desktop';
  import { observeNativePane } from '../../../lib/nativePane';
  import { rowMenu } from '../../../lib/rowMenu';
  import Icon from '../../../lib/components/Icon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import { ws, visibleOnThisDevice } from '../../../lib/stores/workspace.svelte';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { ui } from '../../../lib/stores/ui.svelte';
  import { events } from '../../../lib/events.svelte';
  import { api } from '../../../lib/api/client';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import { loadErrorText } from '../../../lib/loadError';
  import { now } from '../../../lib/stores/now.svelte';
  import { plural } from '../../../lib/plural';
  import { relTime } from '../../mission-control/lib';
  import { SCRATCH_ID } from '../../../lib/stores/sessionBuckets';
  import type { Session, SessionScreen } from '../../../lib/api/types';
  import { home, type HomeBox } from '../home.svelte';
  import { livePoll, type Poller } from './poll';
  import { attentionQueue, buildSchool, busiestRoom, isAsleep, kidActivity, kidAriaLabel, kidLines, providerLabel, roomSummary, type Kid } from '../school/model';
  import { kickOut, release, sendToDetention } from '../school/actions';
  import { pollScreens, visibleLines, type ScreenFeed } from '../school/screens';
  import { mountSchool, webglAvailable, type SchoolHandle, type SchoolPick, type SchoolView } from '../school/scene';

  interface Props {
    box: HomeBox;
    viewId: string;
    zoomed: boolean;
    tick: number;
    active?: boolean;
  }
  let { box, viewId, zoomed, tick, active: spaceActive = true }: Props = $props();
  // A hidden native child can still report document.hidden=false. Its owner
  // publishes visibility explicitly; pause both rendering and live screen IO.
  let nativeVisible = $state(true);
  $effect(() => {
    if (!isNativePane) return;
    return observeNativePane((state) => { nativeVisible = state?.visible !== false; });
  });
  // Scrolled out of view (a tall Home page): pause rendering + screen reads.
  let inView = $state(true);
  let rootEl = $state<HTMLElement | null>(null);
  $effect(() => {
    const el = rootEl;
    if (!el || typeof IntersectionObserver === 'undefined') return;
    const io = new IntersectionObserver((entries) => {
      for (const e of entries) inView = e.isIntersecting;
    });
    io.observe(el);
    return () => io.disconnect();
  });
  const onScreen = $derived(spaceActive && nativeVisible && inView);

  const reducedMotion = (): boolean =>
    typeof window !== 'undefined' && !!window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  // ── Data ─────────────────────────────────────────────────────────────────
  let fetched = $state.raw<Session[]>([]);
  let archivedRows = $state.raw<Session[]>([]);
  let loaded = $state(false);
  let error = $state('');
  /** Kicked out here: hidden at once, before the refetch confirms it. */
  let removed = $state.raw<Set<string>>(new Set());
  /** Rows held in the model while their walk (out / to the bench) plays. */
  let held = $state.raw<Map<string, Session>>(new Map());

  async function load(signal?: AbortSignal): Promise<boolean> {
    try {
      const [rows, arch] = await Promise.all([
        api.bg.get<Session[]>('/sessions?archived=false&limit=1000', signal),
        api.bg.get<Session[]>('/sessions?archived=true&limit=60', signal).catch(() => [] as Session[]),
      ]);
      fetched = rows.filter(visibleOnThisDevice);
      archivedRows = arch.filter(visibleOnThisDevice);
      error = '';
      return true;
    } catch (e) {
      if (signal?.aborted) return true;
      error = loadErrorText(e);
      return false;
    } finally {
      loaded = true;
    }
  }

  let poller: Poller | null = null;
  $effect(() => {
    if (!onScreen) return;
    const p = livePoll(load, 30_000, ['session_created', 'session_removed', 'session_archive_changed', 'session_renamed'], { immediate: true });
    poller = p;
    return () => p.stop();
  });
  let seenTick = untrack(() => tick);
  $effect(() => {
    if (tick === seenTick) return;
    seenTick = tick;
    untrack(() => poller?.now());
  });

  const sessions = $derived.by(() => {
    const m = new Map<string, Session>();
    for (const s of fetched) m.set(s.id, s);
    for (const s of ws.sessions) m.set(s.id, s);
    for (const s of ws.otherWsSessions) m.set(s.id, s);
    for (const [id, row] of held) m.set(id, { ...row, archived: false });
    for (const id of removed) m.delete(id);
    return [...m.values()];
  });
  const archived = $derived(archivedRows.filter((s) => !removed.has(s.id) && !held.has(s.id) && !sessions.some((x) => x.id === s.id && !x.archived)));

  const school = $derived(
    buildSchool({
      workspaces: ws.workspaces,
      currentId: ws.currentId,
      sessions,
      archived,
      statusOf: (id) => ws.statusMap[id],
      needsYou: (id) => ws.needsYou[id] === true,
      stale: events.state !== 'connected',
      canEditAgents: auth.can('agents', 'edit'),
    }),
  );
  const byId = $derived(new Map(school.kids.map((k) => [k.id, k] as const)));
  // Keep complete membership without mounting every session row at once.
  const LIST_PAGE_SIZE = 100;
  let memberPages = $state<Record<string, number>>({});

  // ── View mode ────────────────────────────────────────────────────────────
  const gl = typeof document !== 'undefined' && webglAvailable();
  const mode = $derived<'3d' | 'list'>(gl && box.config.view !== 'list' ? '3d' : 'list');
  function setMode(m: '3d' | 'list'): void {
    home.updateBoxConfig(viewId, box.id, { view: m });
  }

  // ── Scene ────────────────────────────────────────────────────────────────
  let host = $state<HTMLElement | null>(null);
  let stage = $state<HTMLElement | null>(null);
  let handle = $state<SchoolHandle | null>(null);
  let sceneError = $state('');
  let sceneAttempt = $state(0);
  let view = $state<SchoolView>({ kind: 'corridor' });
  /** Kid the camera / card is on. */
  let selected = $state<string | null>(null);
  let hover = $state<SchoolPick | null>(null);
  const dark = $derived(ui.resolvedScheme === 'dark');

  $effect(() => {
    const el = host;
    void sceneAttempt;
    if (!el || mode !== '3d') return;
    let dead = false;
    let h: SchoolHandle | null = null;
    sceneError = '';
    // Capture before async imports/assets; list mode and the initial corridor
    // are not navigation and must never clear the remembered classroom.
    pendingRoom = untrack(() => box.config.room as string | undefined) ?? null;
    mountSchool(el, { reducedMotion: reducedMotion(), label: 'Otto School: workspaces are classrooms, agent sessions are the kids', dark: untrack(() => dark) }).then(
      (x) => {
        if (dead) {
          x.destroy();
          return;
        }
        h = x;
        x.onFrame(placeOverlay);
        x.onView((v) => {
          if (dead || h !== x) return;
          view = v;
          if (v.kind === 'corridor') selected = null;
          // Only scene-originated navigation persists. Initial state and
          // async mounting (including List → 3D) do not emit this callback.
          const room = v.kind === 'corridor' ? null : v.roomId;
          if ((box.config.room ?? null) !== room) home.updateBoxConfig(viewId, box.id, { room });
        });
        x.onContextLost(() => {
          if (dead || h !== x) return;
          h = null;
          handle = null;
          x.destroy();
          // The browser reclaims GPU contexts under pressure: remount on our
          // own (twice a minute at most), keeping the room, before we ask.
          const t = Date.now();
          lostAt = lostAt.filter((at) => t - at < 60_000).concat(t);
          if (lostAt.length <= 2) {
            setTimeout(() => {
              if (!dead) sceneAttempt += 1;
            }, 1200);
            return;
          }
          sceneError = 'The 3D school lost its graphics context (too many 3D views open at once).';
        });
        x.setActive(untrack(() => onScreen));
        handle = x;
        // The update effect restores the captured room when data is ready.
      },
      (e: unknown) => {
        if (!dead) sceneError = loadErrorText(e);
      },
    );
    return () => {
      dead = true;
      h?.destroy();
      handle = null;
    };
  });
  let pendingRoom: string | null = null;
  /** Recent context losses (auto-remount budget). */
  let lostAt: number[] = [];
  $effect(() => {
    const h = handle;
    const s = school;
    if (!h) return;
    h.update(s);
    if (pendingRoom && s.rooms.some((r) => r.id === pendingRoom)) {
      const id = pendingRoom;
      pendingRoom = null;
      void h.enterRoom(id);
    }
  });
  $effect(() => {
    handle?.setActive(onScreen);
  });
  $effect(() => {
    handle?.setTheme(dark);
  });
  $effect(() => {
    handle?.setSelected(selected);
  });
  $effect(() => {
    handle?.setHover(hover);
  });
  // A kid that left takes the card with it.
  $effect(() => {
    if (selected && !byId.has(selected) && !held.has(selected)) selected = null;
  });
  // e2e: the stage carries the handle (read-only inspection in Playwright).
  $effect(() => {
    if (stage) (stage as unknown as { __ottoSchool?: SchoolHandle | null }).__ottoSchool = handle;
  });

  const openRoomModel = $derived.by(() => {
    const v = view;
    return v.kind === 'corridor' ? null : (school.rooms.find((r) => r.id === v.roomId) ?? null);
  });

  // ── Live screens ─────────────────────────────────────────────────────────
  const feeds = new Map<string, ScreenFeed>();
  /** Bumped on every feed so the hover peek re-reads `feeds`. */
  let feedTick = $state(0);
  function pushScreen(k: Kid): void {
    handle?.setScreen(k.id, { title: k.title, provider: k.provider, character: k.character, pose: k.pose, feed: feeds.get(k.id) ?? null });
  }
  $effect(() => {
    const r = openRoomModel;
    if (!handle || !r) return;
    for (const k of r.kids) pushScreen(k);
  });
  $effect(() => {
    const h = handle;
    if (!h || !openRoomModel || !onScreen) return;
    const p = pollScreens({
      wanted: () =>
        h
          .visibleScreens()
          .filter((id) => {
            const k = byId.get(id);
            return !!k && k.pose !== 'away';
          }),
      fetch: async (id, signal) => {
        const r = await api.bg.get<SessionScreen>(`/sessions/${encodeURIComponent(id)}/screen`, signal);
        return { live: r.live, lines: r.lines };
      },
      onFeed: (id, f) => {
        feeds.set(id, f);
        if (hover?.kind === 'kid' && hover.id === id) feedTick++;
        const k = byId.get(id);
        if (k) pushScreen(k);
      },
    });
    return () => p.stop();
  });

  // ── Overlay (HTML: crisp text, ≥ 11 px) ──────────────────────────────────
  const bubbleEls = new Map<string, HTMLElement>();
  let tagEl = $state<HTMLElement | null>(null);
  let cardEl = $state<HTMLElement | null>(null);
  // The tag grows when its terminal peek arrives (or wraps a longer title)
  // between rendered frames — re-clamp it inside the stage right away.
  $effect(() => {
    const el = tagEl;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => placeOverlay());
    ro.observe(el);
    return () => ro.disconnect();
  });
  function bubble(node: HTMLElement, id: string) {
    bubbleEls.set(id, node);
    queueMicrotask(placeOverlay);
    return { destroy: () => bubbleEls.delete(id) };
  }
  const hoverKid = $derived(hover?.kind === 'kid' && hover.id !== selected ? (byId.get(hover.id) ?? null) : null);
  const hoverRoom = $derived(hover?.kind === 'door' && view.kind === 'corridor' ? (school.rooms.find((r) => r.id === hover!.id) ?? null) : null);
  const selKid = $derived(selected ? (byId.get(selected) ?? null) : null);
  const needDoors = $derived(view.kind === 'corridor' ? school.rooms.filter((r) => r.counts.needsYou > 0 || r.counts.error > 0) : []);
  const queue = $derived(attentionQueue(school));
  const busiest = $derived(busiestRoom(school));
  const queueLabel = $derived.by(() => {
    const need = queue.filter((k) => k.pose === 'needs-you').length;
    const failed = queue.length - need;
    return [need ? `${need} ${need === 1 ? 'needs' : 'need'} you` : '', failed ? `${failed} failed` : ''].filter(Boolean).join(' · ');
  });
  let queueOpen = $state(true);
  const QUEUE_SHOWN = 5;
  const peek = $derived.by(() => {
    void feedTick;
    if (!hoverKid || view.kind === 'corridor') return null;
    const f = feeds.get(hoverKid.id);
    return f ? visibleLines(f.lines, 8, 64) : null;
  });

  function place(el: HTMLElement | null | undefined, p: { x: number; y: number; visible: boolean } | null, origin: DOMRect, below = false): void {
    if (!el) return;
    if (!p || !p.visible) {
      el.style.visibility = 'hidden';
      return;
    }
    el.style.visibility = 'visible';
    // Centred over the point, then CLAMPED inside the stage (a tall peek
    // over a back-row kid must never leave the top edge).
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    const x = Math.min(Math.max(4, p.x - origin.left - w / 2), Math.max(4, origin.width - w - 4));
    const y = Math.min(Math.max(4, p.y - origin.top - (below ? 0 : h)), Math.max(4, origin.height - h - 4));
    el.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
  }
  function placeOverlay(): void {
    const h = handle;
    if (!h || !stage) return;
    const origin = stage.getBoundingClientRect();
    for (const [id, el] of bubbleEls)
      place(el, id.startsWith('door:') ? h.project({ kind: 'door', id: id.slice(5) }) : h.project({ kind: 'kid', id }), origin);
    if (hover) place(tagEl, h.project(hover), origin);
    else if (tagEl) tagEl.style.visibility = 'hidden';
  }

  // ── Pointer ──────────────────────────────────────────────────────────────
  let down: { x: number; y: number; lx: number; ly: number; dragging: boolean; id: number } | null = null;
  let lastClick = { id: '', at: 0 };
  function onPointerDown(e: PointerEvent): void {
    if (e.button !== 0 && e.pointerType === 'mouse') return;
    down = { x: e.clientX, y: e.clientY, lx: e.clientX, ly: e.clientY, dragging: false, id: e.pointerId };
    stage?.focus({ preventScroll: true });
  }
  function onPointerMove(e: PointerEvent): void {
    const h = handle;
    if (!h) return;
    if (down && down.id === e.pointerId) {
      if (!down.dragging && Math.hypot(e.clientX - down.x, e.clientY - down.y) > 5) {
        down.dragging = true;
        host?.setPointerCapture(e.pointerId);
      }
      if (down.dragging) {
        h.drag(e.clientX - down.lx, e.clientY - down.ly);
        down.lx = e.clientX;
        down.ly = e.clientY;
        return;
      }
    }
    if (e.pointerType === 'touch') return;
    const p = h.pick(e.clientX, e.clientY);
    if (host) host.style.cursor = p && p.kind !== 'head' ? 'pointer' : '';
    if (JSON.stringify(p) !== JSON.stringify(hover)) hover = p;
  }
  function onPointerUp(e: PointerEvent): void {
    const d = down;
    down = null;
    if (host?.hasPointerCapture(e.pointerId)) host.releasePointerCapture(e.pointerId);
    if (!d || d.dragging || !handle) return;
    const p = handle.pick(e.clientX, e.clientY);
    void activate(p, e.timeStamp);
  }
  async function activate(p: SchoolPick | null, at = performance.now()): Promise<void> {
    const h = handle;
    if (!h) return;
    if (!p) {
      selected = null;
      return;
    }
    if (p.kind === 'door') {
      if (view.kind === 'corridor') await h.enterRoom(p.id);
      else await h.toCorridor();
      return;
    }
    if (p.kind === 'head') {
      toasts.info('Otto, the headmaster', 'Walks the aisles, checks every screen — and goes straight to anyone who raised a hand.');
      return;
    }
    const k = byId.get(p.id);
    if (!k) return;
    // Double click opens the session.
    if (lastClick.id === k.id && at - lastClick.at < 400) {
      lastClick = { id: '', at: 0 };
      void open(k);
      return;
    }
    lastClick = { id: k.id, at };
    selected = k.id;
  }
  function onContextMenu(e: MouseEvent): void {
    const p = handle?.pick(e.clientX, e.clientY);
    const k = p?.kind === 'kid' ? byId.get(p.id) : null;
    if (!k) return;
    e.preventDefault();
    selected = k.id;
    kidMenu(e, k);
  }
  function onWheel(e: WheelEvent): void {
    // Zoom only when the school has focus (or fills the page) — otherwise the
    // wheel scrolls Home like everywhere else.
    if (!handle || (!zoomed && document.activeElement !== stage)) return;
    e.preventDefault();
    handle.wheel(e.deltaY);
  }
  function stopTouch(e: TouchEvent): void {
    e.stopPropagation();
  }
  function onPointerLeave(): void {
    if (!down?.dragging) hover = null;
  }
  $effect(() => {
    const el = host;
    if (!el) return;
    const on: [string, (e: never) => void, AddEventListenerOptions?][] = [
      ['pointerdown', onPointerDown],
      ['pointermove', onPointerMove],
      ['pointerup', onPointerUp],
      ['pointercancel', onPointerUp],
      ['pointerleave', onPointerLeave],
      ['contextmenu', onContextMenu],
      ['wheel', onWheel, { passive: false }],
      ['touchstart', stopTouch],
      ['touchend', stopTouch],
    ];
    for (const [t, f, o] of on) el.addEventListener(t, f as EventListener, o);
    return () => {
      for (const [t, f] of on) el.removeEventListener(t, f as EventListener);
    };
  });

  // ── Keyboard (on the focused stage) ──────────────────────────────────────
  function onStageKey(e: KeyboardEvent, isDown: boolean): void {
    // Card buttons keep their native Enter/Space behavior. Camera shortcuts
    // belong only to the stage, never its interactive descendants.
    if (e.defaultPrevented || e.target !== e.currentTarget) return;
    const h = handle;
    if (!h) return;
    if (h.key(e, isDown)) {
      e.preventDefault();
      return;
    }
    if (!isDown) return;
    if (e.key === 'Enter') {
      e.preventDefault();
      if (view.kind === 'corridor') {
        const d = h.facingDoor();
        if (d) void h.enterRoom(d);
      } else if (selKid) {
        if (view.kind === 'screen') void open(selKid);
        else void h.lookAtScreen(selKid.id);
      }
    } else if (e.key === 'Escape') {
      if (view.kind === 'corridor' && !selected) return; // Home's Esc (exit zoom)
      e.preventDefault();
      back();
    }
  }
  /** Make the stage a keyboard surface (focusable; keys move the camera). */
  function stageKeys(node: HTMLElement) {
    node.tabIndex = 0;
    const pressed = new Set<string>();
    const kd = (e: KeyboardEvent) => {
      if (e.target === node && !e.defaultPrevented) pressed.add(e.key);
      onStageKey(e, true);
    };
    const ku = (e: KeyboardEvent) => {
      pressed.delete(e.key);
      onStageKey(e, false);
    };
    const bl = () => {
      for (const key of pressed) handle?.key(new KeyboardEvent('keyup', { key }), false);
      pressed.clear();
    };
    node.addEventListener('keydown', kd);
    node.addEventListener('keyup', ku);
    node.addEventListener('blur', bl);
    return {
      destroy() {
        node.removeEventListener('keydown', kd);
        node.removeEventListener('keyup', ku);
        node.removeEventListener('blur', bl);
      },
    };
  }

  function back(): void {
    const h = handle;
    if (!h) return;
    if (view.kind === 'screen') void h.backToRoom();
    else if (selected) selected = null;
    else if (view.kind === 'room') void h.toCorridor();
  }

  // ── Actions ──────────────────────────────────────────────────────────────
  async function open(k: Kid | null | undefined): Promise<void> {
    if (!k) return;
    if (k.detention) {
      await free(k);
      return;
    }
    if (k.workspaceId === SCRATCH_ID || k.workspaceId === ws.currentId) ws.navigateToSession(k.id);
    else await ws.openInWorkspace(k.workspaceId, k.id);
  }

  function hold(id: string): void {
    const row = sessions.find((x) => x.id === id);
    if (row) held = new Map(held).set(id, row);
  }
  function unhold(id: string): void {
    const next = new Map(held);
    next.delete(id);
    held = next;
  }

  async function kick(k: Kid): Promise<void> {
    const gone = await kickOut(k, {
      ask: (message, opts) => confirmer.ask(message, opts),
      kill: (id) => ws.killSession(id),
      animate: (id) => {
        if (!handle || view.kind === 'corridor') return Promise.resolve();
        hold(id);
        return handle.kickOut(id).finally(() => unhold(id));
      },
      restore: (id) => {
        unhold(id);
        handle?.restore(id);
      },
      done: (title, body) => toasts.success(title, body),
      failed: (title, e) => toastError(title, e),
    });
    if (gone) {
      removed = new Set([...removed, k.id]);
      if (selected === k.id) selected = null;
    }
  }

  async function detention(k: Kid): Promise<void> {
    const ok = await sendToDetention(k, {
      archive: (id, hint) => ws.requestArchive(id, hint),
      animate: (id) => {
        if (!handle || view.kind === 'corridor') return Promise.resolve();
        hold(id);
        return handle.detention(id);
      },
      restore: (id) => {
        unhold(id);
        handle?.restore(id);
      },
      failed: (title, e) => toastError(title, e),
    });
    if (ok) {
      await poller?.now();
      unhold(k.id);
      if (selected === k.id) selected = null;
    }
  }

  async function free(k: Kid): Promise<void> {
    if (
      await release(k, {
        unarchive: (id) => ws.unarchiveSession(id),
        done: (title) => toasts.success(title),
        failed: (title, e) => toastError(title, e),
      })
    )
      await poller?.now();
  }

  function checkOn(k: Kid): void {
    handle?.inspect(k.id);
  }

  function kidMenu(e: MouseEvent | KeyboardEvent, k: Kid): void {
    const inRoom = view.kind !== 'corridor' && openRoomModel?.id === k.workspaceId
      && [...openRoomModel.kids, ...openRoomModel.bench].some((kid) => kid.id === k.id);
    if (k.detention) {
      ctxMenu.show(e, [
        { label: 'Release from detention', icon: 'undo', disabled: !k.canManage, action: () => void free(k) },
      ]);
      return;
    }
    ctxMenu.show(e, [
      { label: 'Open session', icon: 'external', action: () => void open(k) },
      ...(inRoom ? [{ label: 'Look at the screen', icon: 'eye' as const, action: () => void handle?.lookAtScreen(k.id) }] : []),
      ...(inRoom ? [{ label: 'Headmaster: check on them', icon: 'search' as const, action: () => checkOn(k) }] : []),
      ...(k.canManage
        ? [
            { separator: true as const },
            { label: 'Send to detention (archive)', icon: 'archive' as const, action: () => void detention(k) },
            { label: 'Kick out…', icon: 'trash' as const, danger: true, action: () => void kick(k) },
          ]
        : []),
    ]);
  }

  /** List-view row → into the 3D room and onto that kid. */
  async function showIn3d(k: Kid): Promise<void> {
    if (mode !== '3d' || !handle) return;
    const room = school.rooms.find((r) => r.id === k.workspaceId);
    if (!room || ![...room.kids, ...room.bench].some((kid) => kid.id === k.id)) return;
    if (view.kind === 'corridor' || openRoomModel?.id !== k.workspaceId) await handle.enterRoom(k.workspaceId);
    selected = k.id;
  }

  /** Queue row → into its room, onto the kid (List mode: open it). */
  async function goTo(k: Kid): Promise<void> {
    if (mode !== '3d' || !handle) {
      await open(k);
      return;
    }
    const room = school.rooms.find((r) => r.id === k.workspaceId);
    const seated = !!room && [...room.kids, ...room.bench].some((kid) => kid.id === k.id);
    if (!seated) {
      await open(k);
      return;
    }
    if (view.kind === 'corridor' || openRoomModel?.id !== k.workspaceId) await handle.enterRoom(k.workspaceId);
    selected = k.id;
  }

  onDestroy(() => {
    handle?.destroy();
  });

  const empty = $derived(school.total === 0);
  const crumbs = $derived.by(() => {
    const out: { label: string; go?: () => void }[] = [{ label: 'Corridor', go: view.kind === 'corridor' ? undefined : () => void handle?.toCorridor() }];
    if (openRoomModel) out.push({ label: openRoomModel.name, go: view.kind === 'screen' ? () => void handle?.backToRoom() : undefined });
    if (view.kind === 'screen') out.push({ label: `${byId.get(view.kidId)?.title ?? 'Screen'}` });
    return out;
  });
  const hint = $derived(
    view.kind === 'corridor'
      ? 'Click a door to walk in · drag to look · W A S D to walk'
      : view.kind === 'room'
        ? 'Hover a kid to peek · click for its card · double-click opens it · Esc for the corridor'
        : 'Enter opens the session · Esc goes back',
  );
</script>

<div class="school" bind:this={rootEl}>
  <div class="bar">
    {#if mode === '3d'}
      <nav class="crumbs" aria-label="Where you are in the school">
        {#each crumbs as c, i (i)}
          {#if i > 0}<Icon name="chevronRight" size={12} />{/if}
          {#if c.go}
            <button class="crumb link" onclick={c.go}>{c.label}</button>
          {:else}
            <span class="crumb ellipsis" aria-current="location">{c.label}</span>
          {/if}
        {/each}
      </nav>
    {:else}
      <span class="sum">{plural(school.rooms.length, 'classroom')} · {plural(school.total, 'kid')}</span>
    {/if}
    {#if gl}
      <div class="segmented" role="group" aria-label="School view">
        <button class="seg" class:active={mode === '3d'} aria-pressed={mode === '3d'} onclick={() => setMode('3d')}>3D</button>
        <button class="seg" class:active={mode === 'list'} aria-pressed={mode === 'list'} onclick={() => setMode('list')}>List</button>
      </div>
    {/if}
    {#if mode === '3d'}
      <button class="icon-btn" onclick={() => handle?.resetView()} disabled={!handle} title="Reset the school view" aria-label="Reset the school view"><Icon name="refresh" size={12} /></button>
      <button class="icon-btn" onclick={() => home.toggleZoom(box.id)} title={zoomed ? 'Exit full page' : 'Fill the page'} aria-label={zoomed ? 'Exit full page' : 'Fill the page'}>
        <Icon name={zoomed ? 'minimize' : 'maximize'} size={12} />
      </button>
    {/if}
  </div>

  {#if !loaded && fetched.length === 0 && ws.sessions.length === 0}
    <LoadState what="the school" empty={true} loading={true} variant="compact" rows={3} />
  {:else if error && fetched.length === 0 && ws.sessions.length === 0}
    <LoadState what="the school" empty={true} error={error} variant="compact" onretry={() => poller?.now()} />
  {:else}
    {#if error}
      <LoadState what="the school" error={error} empty={false} variant="compact" onretry={() => poller?.now()} />
    {/if}
    {#if !gl}
      <p class="note" role="note"><Icon name="info" size={12} />3D isn’t available here (WebGL is off), so the school is listed instead.</p>
    {/if}
    {#if mode === '3d'}
      <!-- Focusable 3D surface: tabindex + key listeners are attached in
           script (stageKeys) — it is a widget (role=application) driven by
           the keyboard like a game, with the list below as the plain path. -->
      <div
        class="stage"
        bind:this={stage}
        use:stageKeys
        role="application"
        aria-label="Otto School 3D view. Arrow keys or W A S D move, Enter walks through the door you face or looks at the selected kid’s screen, Escape goes back."
      >
        <div class="canvas" bind:this={host}></div>
        {#if handle}
          <div class="overlay" aria-hidden="true">
            {#each needDoors as r (r.id)}
              <div class="bubble door" class:err={r.counts.needsYou === 0} use:bubble={`door:${r.id}`}>
                {r.counts.needsYou > 0 ? `✋ ${r.counts.needsYou}` : `❗ ${r.counts.error}`}
              </div>
            {/each}
            <div class="hover-tag" class:wide={!!peek} bind:this={tagEl}>
              {#if hoverKid}
                <b>{hoverKid.title}</b>
                <small>{providerLabel(hoverKid.provider)} · {kidActivity(hoverKid, now() ? relTime(hoverKid.lastActiveAt) : '', isAsleep(hoverKid, now()))}</small>
                {#if peek}
                  <pre class="peek">{peek.length ? peek.join('\n') : 'No output yet'}</pre>
                {/if}
              {:else if hoverRoom}
                <b>{hoverRoom.name}</b><small>{roomSummary(hoverRoom)} · click to walk in</small>
              {:else if hover?.kind === 'head'}
                <b>Otto</b><small>the headmaster</small>
              {:else if hover?.kind === 'door'}
                <b>Back to the corridor</b>
              {/if}
            </div>
          </div>
          {#if queue.length > 0}
            <!-- Corridor: top-end (doors start at the near left); room: top-start
                 (the kid card owns top-end). -->
            <section class="queue" class:end={view.kind === 'corridor'} aria-label="Sessions that need you">
              <button class="queue-head" aria-expanded={queueOpen} onclick={() => (queueOpen = !queueOpen)}>
                <Icon name="hand" size={12} />
                <span>{queueLabel}</span>
                <Icon name={queueOpen ? 'chevronUp' : 'chevronDown'} size={12} />
              </button>
              {#if queueOpen}
                <ul>
                  {#each queue.slice(0, QUEUE_SHOWN) as k (k.id)}
                    <li class="qrow" class:err={k.pose === 'error'}>
                      <button class="qmain" onclick={() => void goTo(k)} title="Go to {k.title} in {k.workspaceName}">
                        <ProviderIcon provider={k.provider} size={12} />
                        <span class="ellipsis qt">{k.title}</span>
                        <small class="ellipsis">{k.workspaceName}</small>
                      </button>
                      <button class="icon-btn" onclick={() => void open(k)} aria-label="Open {k.title}" title="Open {k.title}"><Icon name="external" size={12} /></button>
                    </li>
                  {/each}
                </ul>
                {#if queue.length > QUEUE_SHOWN}<p class="qmore">+{queue.length - QUEUE_SHOWN} more in the list</p>{/if}
              {/if}
            </section>
          {:else if view.kind === 'corridor' && busiest}
            <button class="jump" onclick={() => busiest && void handle?.enterRoom(busiest.id)} title="Walk into the busiest classroom">
              <span class="ellipsis">{busiest.name}</span><small>{roomSummary(busiest)}</small><Icon name="chevronRight" size={12} />
            </button>
          {/if}
          <p class="hint">{hint}</p>
          {#if selKid && view.kind !== 'corridor'}
            <div class="kid-card" role="group" aria-label="{selKid.title} — details" bind:this={cardEl}>
              <div class="card-head">
                <ProviderIcon provider={selKid.provider} size={16} />
                <b class="ellipsis">{selKid.title}</b>
                <button class="icon-btn" onclick={() => (selected = null)} aria-label="Close details" title="Close details"><Icon name="x" size={12} /></button>
              </div>
              {#each kidLines(selKid, now() ? relTime(selKid.lastActiveAt) : '') as line, i (i)}
                <div class="card-line" class:needs={i === 0 && selKid.pose === 'needs-you'}>{line}</div>
              {/each}
              <div class="card-actions">
                {#if selKid.detention}
                  <button class="btn small primary" disabled={!selKid.canManage} onclick={() => selKid && void free(selKid)}>Release from detention</button>
                {:else}
                  <button class="btn small primary" onclick={() => void open(selKid)}>Open session</button>
                  {#if view.kind === 'screen'}
                    <button class="btn small" onclick={() => void handle?.backToRoom()}>Back to the room</button>
                  {:else}
                    <button class="btn small" onclick={() => selKid && void handle?.lookAtScreen(selKid.id)}>Look at screen</button>
                  {/if}
                  <button class="btn small" onclick={() => selKid && checkOn(selKid)} title="Send the headmaster over">Check on</button>
                  {#if selKid.canManage}
                    <button class="btn small" onclick={() => selKid && void detention(selKid)} title="Archive — resumable, with Undo">Detention</button>
                    <button class="btn small danger" onclick={() => selKid && void kick(selKid)} title="Delete this session and its history">Kick out…</button>
                  {/if}
                {/if}
              </div>
            </div>
          {/if}
        {:else if sceneError}
          <div class="scene-state">
            <LoadState what="the 3D school" empty={true} error={sceneError} variant="compact" onretry={() => (sceneAttempt += 1)} />
          </div>
        {:else}
          <div class="scene-state"><LoadState what="the 3D school" empty={true} loading={true} variant="compact" /></div>
        {/if}
        {#if empty && handle && view.kind === 'corridor'}
          <div class="empty-over">
            <EmptyState icon="people" title="School’s out" body="Start a session and a kid takes a seat in its workspace’s classroom.">
              <button class="btn small" onclick={() => (ui.newSessionOpen = true)}><Icon name="plus" size={12} />New session</button>
            </EmptyState>
          </div>
        {/if}
      </div>
    {/if}

    <!-- The accessible companion: every classroom and kid as real buttons.
         Visible in List view; in 3D it stays keyboard / screen-reader
         reachable, and activating a row walks into that room onto the kid. -->
    <div class="list" class:sr-only={mode === '3d'} role="region" aria-label="School list">
      {#if mode === 'list' && empty}
        <EmptyState icon="people" title="School’s out" body="Start a session and a kid takes a seat in its workspace’s classroom.">
          <button class="btn small" onclick={() => (ui.newSessionOpen = true)}><Icon name="plus" size={12} />New session</button>
        </EmptyState>
      {/if}
      {#if queue.length > 0}
        <section class="lroom lattn" aria-label="Needs you">
          <h4 class="lroom-head"><span class="ellipsis">Needs you</span><small>{plural(queue.length, 'session')}</small></h4>
          <ul>
            {#each queue as k (k.id)}
              <li class="lrow" data-attn-id={k.id}>
                <button class="lmain" aria-label="Go to {kidAriaLabel(k, now() ? relTime(k.lastActiveAt) : '')} in {k.workspaceName}" onclick={() => void goTo(k)}>
                  <i class="dot" data-pose={k.pose}></i>
                  <ProviderIcon provider={k.provider} size={12} />
                  <span class="ellipsis t">{k.title}</span>
                  <span class="st">{k.workspaceName} · {k.stateLabel}</span>
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {/if}
      {#each school.rooms as r (r.id)}
        {@const lastPage = Math.max(0, Math.ceil(r.members.length / LIST_PAGE_SIZE) - 1)}
        {@const page = Math.min(memberPages[r.id] ?? 0, lastPage)}
        {#if r.members.length > 0 || mode === 'list'}
          <section class="lroom" aria-label="Classroom {r.name}">
            <h4 class="lroom-head">
              <span class="ellipsis">{r.name}</span>
              {#if r.current}<span class="cur">current</span>{/if}
              <small>{roomSummary(r)}</small>
              {#if mode === '3d'}
                <button class="btn small" onclick={() => void handle?.enterRoom(r.id)} disabled={!handle}>Walk in</button>
              {/if}
            </h4>
            {#if r.members.length > 0}
              <ul>
                {#each r.members.slice(page * LIST_PAGE_SIZE, (page + 1) * LIST_PAGE_SIZE) as k (k.id)}
                  <li use:rowMenu class="lrow" data-kid-id={k.id} oncontextmenu={(e) => kidMenu(e, k)}>
                    <button
                      class="lmain"
                      aria-label="{k.detention ? 'Release' : 'Open'} {kidAriaLabel(k, now() ? relTime(k.lastActiveAt) : '')}"
                      onclick={() => void open(k)}
                      onfocus={() => void showIn3d(k)}
                    >
                      <i class="dot" data-pose={k.detention ? 'bench' : k.pose}></i>
                      <ProviderIcon provider={k.provider} size={12} />
                      <span class="ellipsis t">{k.title}</span>
                      {#if k.background}<span class="bg-tag">back row</span>{/if}
                      {#if k.detention}<span class="bg-tag">detention</span>{/if}
                      <span class="st">{k.stateLabel}</span>
                    </button>
                    <button class="icon-btn" aria-label="More actions for {k.title}" title="More actions for {k.title}" aria-haspopup="menu" onclick={(e) => kidMenu(e, k)}>
                      <Icon name="more" size={12} />
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
            {#if lastPage > 0}
              <nav class="member-pages" aria-label="Sessions in classroom {r.name}">
                <button class="btn small" aria-label="Previous sessions in {r.name}" disabled={page === 0} onclick={() => (memberPages[r.id] = page - 1)}>Previous</button>
                <span aria-live="polite">{page * LIST_PAGE_SIZE + 1}–{Math.min((page + 1) * LIST_PAGE_SIZE, r.members.length)} of {r.members.length}</span>
                <button class="btn small" aria-label="Next sessions in {r.name}" disabled={page === lastPage} onclick={() => (memberPages[r.id] = page + 1)}>Next</button>
              </nav>
            {/if}
            {#if r.overflow > 0}<p class="more">{r.overflow} not seated in 3D</p>{/if}
          </section>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .school {
    display: flex;
    flex-direction: column;
    gap: 6px;
    height: 100%;
    min-height: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
    min-width: 0;
  }
  .crumbs {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 1;
    min-width: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .crumb {
    min-width: 0;
    max-width: 220px;
  }
  .crumb.link {
    background: none;
    border: 0;
    padding: 2px 4px;
    border-radius: var(--radius-s);
    color: var(--accent-text);
    cursor: pointer;
    font: inherit;
  }
  .crumb.link:hover {
    background: var(--surface-2);
  }
  span.crumb {
    color: var(--text);
    font-weight: 600;
  }
  .sum {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .note {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 260px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    overflow: hidden;
    outline: none;
  }
  .stage:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .canvas {
    position: absolute;
    inset: 0;
  }
  .overlay {
    position: absolute;
    inset: 0;
    pointer-events: none;
    overflow: hidden;
  }
  .bubble,
  .hover-tag {
    position: absolute;
    top: 0;
    left: 0; /* ui-guards: allow — origin of projected screen coordinates, not layout */
    visibility: hidden;
    white-space: nowrap;
  }
  .bubble {
    font-size: var(--fs-l);
    line-height: 1;
    padding: 4px 6px;
    border-radius: var(--radius-m);
    background: var(--warning-soft);
    border: 1px solid var(--warning);
    animation: otto-pulse 1.4s ease-in-out infinite;
  }
  .bubble.err {
    background: var(--danger-soft);
    border-color: var(--danger);
  }
  .bubble.door {
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
    background: var(--surface);
    box-shadow: var(--shadow-xs);
  }
  @media (prefers-reduced-motion: reduce) {
    .bubble {
      animation: none;
    }
  }
  .hover-tag {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-xs);
    max-width: 260px;
  }
  .hover-tag:empty {
    display: none;
  }
  .hover-tag b {
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .hover-tag small {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .hover-tag.wide {
    max-width: min(460px, 90%);
    align-items: stretch;
  }
  .hover-tag.wide b {
    max-width: none;
  }
  /* The zoom lens on a kid's monitor: its last terminal rows as real text. */
  .peek {
    margin: 4px 0 0;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--term-bg);
    border: 1px solid var(--border);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.35;
    white-space: pre;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .queue {
    position: absolute;
    inset-inline-start: 8px;
    top: 8px;
    width: min(280px, calc(100% - 16px));
    max-height: calc(100% - 56px);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 4px;
    border-radius: var(--radius-m);
    background: var(--surface);
    border: 1px solid var(--warning);
    box-shadow: var(--shadow-card);
  }
  .queue.end {
    inset-inline-start: auto;
    inset-inline-end: 8px;
  }
  .queue ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .queue-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 600;
    cursor: pointer;
  }
  .queue-head span {
    flex: 1;
    text-align: start;
  }
  .qrow {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .qmain {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .qmain:hover,
  .jump:hover {
    background: var(--surface-2);
  }
  .qmain small {
    flex: none;
    max-width: 40%;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .qrow.err .qt {
    color: var(--danger);
  }
  .qt {
    flex: 1;
    min-width: 0;
  }
  .qmore {
    margin: 2px 6px 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .jump {
    position: absolute;
    inset-inline-start: 8px;
    top: 8px;
    max-width: min(320px, calc(100% - 16px));
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-radius: var(--radius-m);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-card);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 600;
    cursor: pointer;
  }
  .jump small {
    font-weight: 400;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .queue-head:focus-visible,
  .qmain:focus-visible,
  .jump:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .hint {
    position: absolute;
    inset-inline-start: 8px;
    bottom: 8px;
    margin: 0;
    padding: 4px 8px;
    max-width: calc(100% - 16px);
    border-radius: var(--radius-s);
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    pointer-events: none;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kid-card {
    position: absolute;
    inset-inline-end: 8px;
    top: 8px;
    width: min(320px, calc(100% - 16px));
    max-height: calc(100% - 16px);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px;
    border-radius: var(--radius-m);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-card);
  }
  .card-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .card-head b {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-m);
  }
  .card-line {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .card-line.needs {
    color: var(--warning);
    font-weight: 600;
  }
  .card-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 4px;
  }
  .scene-state {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
  }
  .empty-over {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: none;
  }
  .empty-over :global(*) {
    pointer-events: auto;
  }
  .list {
    padding-inline: 4px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    overflow-y: auto;
    min-height: 0;
  }
  /* Keep the plain action path available to screen readers, and reveal its
     context as soon as any keyboard control receives focus. */
  .list.sr-only:focus-within {
    position: static;
    width: auto;
    height: auto;
    flex: 0 1 45%;
    min-block-size: 100px;
    max-block-size: 45%;
    margin: 0;
    padding: 6px;
    overflow: auto;
    clip: auto;
    clip-path: none;
    white-space: normal;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
  }
  .member-pages {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding-block: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .lroom-head {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 4px;
    font-size: var(--fs-s);
  }
  .lroom-head small {
    flex: 1;
    color: var(--text-dim);
    font-weight: 400;
    font-size: var(--fs-xs);
  }
  .cur {
    font-size: var(--fs-xs);
    color: var(--accent-text);
  }
  .lroom ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .lrow {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .lmain {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border: 0;
    background: none;
    border-radius: var(--radius-s);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .lmain:hover {
    background: var(--surface-2);
  }
  .t {
    flex: 1;
    min-width: 0;
  }
  .st,
  .bg-tag,
  .more {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .more {
    margin: 2px 6px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
    background: var(--text-dim);
  }
  .dot[data-pose='working'] {
    background: var(--success);
  }
  .dot[data-pose='needs-you'] {
    background: var(--warning);
  }
  .dot[data-pose='error'] {
    background: var(--danger);
  }
  .dot[data-pose='done'] {
    background: var(--info);
  }
  .dot[data-pose='away'],
  .dot[data-pose='bench'] {
    background: var(--border-strong);
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
