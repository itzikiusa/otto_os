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
  import Icon from '../../../lib/components/Icon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
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
  import { SCRATCH_ID } from '../../../lib/stores/sessionBuckets';
  import type { Session, SessionScreen } from '../../../lib/api/types';
  import { home, type HomeBox } from '../home.svelte';
  import { livePoll, type Poller } from './poll';
  import { attentionQueue, buildSchool, busiestRoom, type Kid } from '../school/model';
  import { kickOut, release, sendToDetention } from '../school/actions';
  import { pollScreens, visibleLines, type ScreenFeed } from '../school/screens';
  import { mountSchool, webglAvailable, type SchoolHandle, type SchoolPick, type SchoolView } from '../school/scene';
  import HoverTag from './classrooms/HoverTag.svelte';
  import KidCard from './classrooms/KidCard.svelte';
  import NeedsYouQueue from './classrooms/NeedsYouQueue.svelte';
  import SchoolBar from './classrooms/SchoolBar.svelte';
  import SchoolList from './classrooms/SchoolList.svelte';
  import { hintFor, keySurface, kidMenuItems, place, queueLabelFor } from './classrooms/helpers';

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
  // Current list page per classroom (SchoolList pages its members).
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
  const queueLabel = $derived(queueLabelFor(queue));
  let queueOpen = $state(true);
  const peek = $derived.by(() => {
    void feedTick;
    if (!hoverKid || view.kind === 'corridor') return null;
    const f = feeds.get(hoverKid.id);
    return f ? visibleLines(f.lines, 8, 64) : null;
  });

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
    return keySurface(node, onStageKey, (key) => handle?.key(new KeyboardEvent('keyup', { key }), false));
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
    ctxMenu.show(e, kidMenuItems(k, inRoom, { open, free, checkOn, detention, kick, lookAtScreen: (id) => void handle?.lookAtScreen(id) }));
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
  const hint = $derived(hintFor(view));
</script>

<div class="school" bind:this={rootEl}>
  <SchoolBar boxId={box.id} {mode} {gl} {zoomed} {school} {handle} {crumbs} {setMode} />

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
            <HoverTag {hover} {hoverKid} {hoverRoom} {peek} bind:el={tagEl} />
          </div>
          <NeedsYouQueue {queue} {queueLabel} bind:queueOpen {view} {busiest} {handle} {open} {goTo} />
          <p class="hint">{hint}</p>
          {#if selKid && view.kind !== 'corridor'}
            <KidCard {selKid} {view} {handle} bind:el={cardEl} onclose={() => (selected = null)} {open} {free} {checkOn} {detention} {kick} />
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
    <SchoolList {mode} {empty} {school} {queue} {handle} bind:memberPages {open} {goTo} {kidMenu} {showIn3d} />
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
  .bubble {
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
</style>
