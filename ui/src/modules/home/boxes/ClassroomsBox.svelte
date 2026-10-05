<script lang="ts">
  // Classrooms box: a 3D live view of every session — workspaces are
  // classrooms, sessions are students at desks, and the user is the headmaster
  // standing at the door of the current classroom. Students type while their
  // agent works, raise a hand when it needs you, sit still when idle and fade
  // to a ghost at an empty desk when suspended/ended; engine sessions
  // (workflow / swarm / review…) sit in a separate back row.
  //
  // Data: the cross-workspace `GET /sessions?archived=false` (#17b — the same
  // feed as the all-workspaces sidebar, but with background rows), refreshed on
  // session events, overlaid with the workspace store's live rows; status and
  // "needs you" come from the store's event-fed maps. three.js is lazy-loaded
  // by classrooms/scene.ts; the pure mapping lives in classrooms/model.ts.
  //
  // The canvas is never the only path: the same students are a list of real
  // buttons — visible in List view (and whenever WebGL is unavailable), and
  // screen-reader/keyboard reachable in 3D view, where focusing a row
  // highlights that student and shows its tooltip.
  import { onDestroy, tick as svelteTick, untrack } from 'svelte';
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
  import { readTokenColors } from '../../../lib/cssColor';
  import { now } from '../../../lib/stores/now.svelte';
  import { plural } from '../../../lib/plural';
  import { relTime } from '../../mission-control/lib';
  import { SCRATCH_ID } from '../../../lib/stores/sessionBuckets';
  import type { Session } from '../../../lib/api/types';
  import { home, type HomeBox } from '../home.svelte';
  import { livePoll, type Poller } from './poll';
  import { buildClassrooms, providerLabel, roomSummary, studentAriaLabel, tooltipLines, type Student } from '../classrooms/model';
  import { kickOut, sendToDetention } from '../classrooms/actions';
  import { mountClassroomScene, SCENE_TOKENS, webglAvailable, type SceneHandle } from '../classrooms/scene';

  interface Props {
    box: HomeBox;
    viewId: string;
    zoomed: boolean;
    tick: number;
    active?: boolean;
  }
  let { box, viewId, zoomed: _zoomed, tick, active: onScreen = true }: Props = $props();

  const reducedMotion = (): boolean =>
    typeof window !== 'undefined' && !!window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  // ── Data ─────────────────────────────────────────────────────────────────
  let fetched = $state.raw<Session[]>([]);
  let loaded = $state(false);
  let error = $state('');
  /** Kicked out here: hidden at once, before the next refetch confirms it. */
  let removed = $state.raw<Set<string>>(new Set());

  async function load(signal?: AbortSignal): Promise<boolean> {
    try {
      const rows = await api.bg.get<Session[]>('/sessions?archived=false&limit=1000', signal);
      fetched = rows.filter(visibleOnThisDevice);
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
    // The store's rows are event-patched (titles, archive state) — fresher.
    for (const s of ws.sessions) m.set(s.id, s);
    for (const s of ws.otherWsSessions) m.set(s.id, s);
    for (const id of removed) m.delete(id);
    return [...m.values()];
  });

  const model = $derived(
    buildClassrooms({
      workspaces: ws.workspaces,
      currentId: ws.currentId,
      sessions,
      statusOf: (id) => ws.statusMap[id],
      needsYou: (id) => ws.needsYou[id] === true,
      stale: events.state !== 'connected',
      canEditAgents: auth.can('agents', 'edit'),
    }),
  );
  const byId = $derived(new Map(model.students.map((s) => [s.id, s] as const)));

  // ── View mode ────────────────────────────────────────────────────────────
  const gl = typeof document !== 'undefined' && webglAvailable();
  const mode = $derived<'3d' | 'list'>(gl && box.config.view !== 'list' ? '3d' : 'list');
  function setMode(m: '3d' | 'list'): void {
    home.updateBoxConfig(viewId, box.id, { view: m });
  }

  // ── Scene ────────────────────────────────────────────────────────────────
  let host = $state<HTMLElement | null>(null);
  let stage = $state<HTMLElement | null>(null);
  let handle = $state<SceneHandle | null>(null);
  let sceneError = $state('');
  let sceneAttempt = $state(0);

  $effect(() => {
    const el = host;
    void sceneAttempt;
    if (!el || mode !== '3d') return;
    let dead = false;
    let h: SceneHandle | null = null;
    sceneError = '';
    // scene.ts lazy-loads three.js itself (the box chunk stays small).
    mountClassroomScene(el, { reducedMotion: reducedMotion(), label: 'Classrooms: workspaces as rooms, sessions as students' })
      .then(
        (x) => {
          if (dead) {
            x.destroy();
            return;
          }
          h = x;
          x.onFrame(placeOverlay);
          x.setColors(readTokenColors(SCENE_TOKENS));
          x.setActive(untrack(() => onScreen));
          handle = x;
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
  // Push every model change (and the model a fresh handle starts from).
  $effect(() => {
    const h = handle;
    const m = model;
    h?.update(m);
  });
  $effect(() => {
    handle?.setActive(onScreen);
  });
  // Theme / scheme / accent changes repaint from the resolved tokens.
  $effect(() => {
    const h = handle;
    if (!h) return;
    const mo = new MutationObserver(() => h.setColors(readTokenColors(SCENE_TOKENS)));
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'data-scheme', 'style', 'class'] });
    return () => mo.disconnect();
  });

  // ── Overlay labels (HTML, so text stays crisp and ≥ 11 px) ───────────────
  const roomEls = new Map<string, HTMLElement>();
  const nameEls = new Map<string, HTMLElement>();
  let youEl = $state<HTMLElement | null>(null);
  function roomLabel(node: HTMLElement, id: string) {
    roomEls.set(id, node);
    return { destroy: () => roomEls.delete(id) };
  }
  function nameLabel(node: HTMLElement, id: string) {
    nameEls.set(id, node);
    return { destroy: () => nameEls.delete(id) };
  }
  /** Students that carry a floating provider tag: all of them on a small
   *  campus, the current room's on a big one (labels would pile up). */
  const tagged = $derived.by(() => {
    if (model.total <= 60) return model.students;
    const cur = model.rooms.find((r) => r.current) ?? model.rooms[0];
    return cur ? [...cur.students, ...cur.backRow].slice(0, 40) : [];
  });
  function place(el: HTMLElement | null | undefined, p: { x: number; y: number; visible: boolean } | null, origin: DOMRect): void {
    if (!el) return;
    if (!p || !p.visible) {
      el.style.visibility = 'hidden';
      return;
    }
    el.style.visibility = 'visible';
    el.style.transform = `translate(${Math.round(p.x - origin.left)}px, ${Math.round(p.y - origin.top)}px) translate(-50%, -100%)`;
  }
  function placeOverlay(): void {
    const h = handle;
    if (!h || !stage) return;
    const origin = stage.getBoundingClientRect();
    for (const [id, el] of roomEls) place(el, h.projectRoom(id), origin);
    for (const [id, el] of nameEls) place(el, h.projectStudent(id), origin);
    place(youEl, h.projectHeadmaster(), origin);
    positionTip();
  }

  // ── Tooltip ──────────────────────────────────────────────────────────────
  let tip = $state<{ id: string; pinned: boolean } | null>(null);
  let tipEl = $state<HTMLElement | null>(null);
  let focusId = $state<string | null>(null);
  let hideTimer: ReturnType<typeof setTimeout> | null = null;
  const tipStudent = $derived(tip ? (byId.get(tip.id) ?? null) : null);
  const tipText = $derived(tipStudent ? tooltipLines(tipStudent, now() ? relTime(tipStudent.lastActiveAt) : '') : null);

  function cancelHide(): void {
    if (hideTimer) clearTimeout(hideTimer);
    hideTimer = null;
  }
  function scheduleHide(): void {
    cancelHide();
    hideTimer = setTimeout(() => {
      if (!tip?.pinned) tip = null;
    }, 250);
  }
  function showTip(id: string, pinned = false): void {
    cancelHide();
    tip = { id, pinned };
  }
  function closeTip(): void {
    cancelHide();
    tip = null;
  }
  $effect(() => {
    handle?.setHighlight(tip?.id ?? focusId);
  });
  // A student that left (kicked out, archived) takes its tooltip with it.
  $effect(() => {
    if (tip && !byId.has(tip.id)) tip = null;
  });
  $effect(() => {
    void tip;
    void tipEl;
    void svelteTick().then(positionTip);
  });

  /** Fixed-position tooltip anchored above the student, CLAMPED into the
   *  viewport (AGENTS.md floating-UI rule) — never flipped without a floor. */
  function positionTip(): void {
    const el = tipEl;
    if (!el || !tip) return;
    let ax: number;
    let ay: number;
    let below = 0;
    const p = mode === '3d' ? handle?.projectStudent(tip.id) : null;
    if (p && p.visible) {
      ax = p.x;
      ay = p.y;
    } else {
      // List view / off-camera: anchor to the student's row.
      const row = listEl?.querySelector<HTMLElement>(`[data-student-id="${CSS.escape(tip.id)}"]`);
      const r = row?.getBoundingClientRect() ?? stage?.getBoundingClientRect();
      if (!r) return;
      ax = r.left + Math.min(r.width / 2, 160);
      ay = r.top;
      below = r.height;
    }
    const m = 8;
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let x = ax - w / 2;
    let y = ay - h - 10;
    if (y < m) y = ay + below + 10;
    x = Math.min(Math.max(m, x), Math.max(m, vw - w - m));
    y = Math.min(Math.max(m, y), Math.max(m, vh - h - m));
    el.style.left = `${Math.round(x)}px`;
    el.style.top = `${Math.round(y)}px`;
  }

  // ── Pointer on the canvas ────────────────────────────────────────────────
  let down: { x: number; y: number } | null = null;
  function onPointerMove(e: PointerEvent): void {
    if (!handle || e.pointerType === 'touch' || down) return;
    const id = handle.pick(e.clientX, e.clientY);
    if (host) host.style.cursor = id ? 'pointer' : '';
    if (id) showTip(id, tip?.pinned && tip.id === id);
    else if (tip && !tip.pinned) scheduleHide();
  }
  function onPointerDown(e: PointerEvent): void {
    down = { x: e.clientX, y: e.clientY };
  }
  function onPointerUp(e: PointerEvent): void {
    const d = down;
    down = null;
    if (!d || !handle || Math.hypot(e.clientX - d.x, e.clientY - d.y) > 6) return; // an orbit drag
    const id = handle.pick(e.clientX, e.clientY);
    if (!id) {
      closeTip();
      return;
    }
    if (e.pointerType === 'touch') {
      // Touch: first tap shows the tooltip, a second tap on the same student opens it.
      if (tip?.id === id && tip.pinned) void open(byId.get(id));
      else showTip(id, true);
      return;
    }
    if (e.button === 0) void open(byId.get(id));
  }
  function onContextMenu(e: MouseEvent): void {
    const id = handle?.pick(e.clientX, e.clientY);
    const s = id ? byId.get(id) : null;
    if (!s) return;
    closeTip();
    studentMenu(e, s);
  }

  // ── Actions ──────────────────────────────────────────────────────────────
  async function open(s: Student | null | undefined): Promise<void> {
    if (!s) return;
    closeTip();
    if (s.workspaceId === SCRATCH_ID || s.workspaceId === ws.currentId) ws.navigateToSession(s.id);
    else await ws.openInWorkspace(s.workspaceId, s.id);
  }

  async function kick(s: Student): Promise<void> {
    closeTip();
    const gone = await kickOut(s, {
      confirm: (message, opts) => confirmer.ask(message, opts),
      kill: (id) => ws.killSession(id),
      animate: (id) => handle?.kickOut(id) ?? Promise.resolve(),
      restore: (id) => handle?.restore(id),
      done: (title, body) => toasts.success(title, body),
      failed: (title, e) => toastError(title, e),
    });
    if (gone) removed = new Set([...removed, s.id]);
  }

  async function detention(s: Student): Promise<void> {
    closeTip();
    if (await sendToDetention(s, { archive: (id) => ws.archiveSession(id), failed: (title, e) => toastError(title, e) })) {
      removed = new Set([...removed, s.id]);
      void poller?.now();
    }
  }

  function studentMenu(e: MouseEvent | KeyboardEvent, s: Student): void {
    ctxMenu.show(e, [
      { label: 'Open session', icon: 'external', action: () => void open(s) },
      ...(s.canManage
        ? [
            { separator: true as const },
            { label: 'Send to detention (archive)', icon: 'archive' as const, action: () => void detention(s) },
            { label: 'Kick out…', icon: 'trash' as const, danger: true, action: () => void kick(s) },
          ]
        : []),
    ]);
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Escape' && tip) {
      e.stopPropagation();
      closeTip();
    }
  }

  let listEl = $state<HTMLElement | null>(null);
  onDestroy(cancelHide);

  const empty = $derived(model.total === 0);
  const nRooms = $derived(model.rooms.length);
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="cr" onkeydown={onKey}>
  <div class="bar">
    <span class="sum">
      {plural(nRooms, 'classroom')} · {plural(model.total, 'student')}
    </span>
    {#if gl}
      <div class="segmented" role="group" aria-label="Classrooms view">
        <button class="seg" class:active={mode === '3d'} aria-pressed={mode === '3d'} onclick={() => setMode('3d')}>3D</button>
        <button class="seg" class:active={mode === 'list'} aria-pressed={mode === 'list'} onclick={() => setMode('list')}>List</button>
      </div>
    {/if}
    {#if mode === '3d'}
      <button class="icon-btn" onclick={() => handle?.resetView()} disabled={!handle} title="Reset view" aria-label="Reset classrooms view"><Icon name="refresh" size={12} /></button>
    {/if}
  </div>

  {#if !loaded && fetched.length === 0 && ws.sessions.length === 0}
    <LoadState what="classrooms" loading={true} variant="compact" rows={3} />
  {:else if error && fetched.length === 0 && ws.sessions.length === 0}
    <LoadState what="classrooms" error={error} variant="compact" onretry={() => poller?.now()} />
  {:else}
    {#if !gl}
      <p class="note" role="note"><Icon name="info" size={12} />3D isn’t available here (WebGL is off), so the classrooms are listed instead.</p>
    {/if}
    {#if mode === '3d'}
      <div class="stage" bind:this={stage}>
        <div
          class="canvas"
          bind:this={host}
          onpointermove={onPointerMove}
          onpointerdown={onPointerDown}
          onpointerup={onPointerUp}
          onpointerleave={() => {
            down = null;
            if (tip && !tip.pinned) scheduleHide();
          }}
          oncontextmenu={onContextMenu}
        ></div>
        {#if handle}
          <div class="overlay" aria-hidden="true">
            {#each model.rooms as r (r.id)}
              <div class="room-label" class:current={r.current} use:roomLabel={r.id}>
                <b>{r.name}</b><small>{roomSummary(r)}{r.overflow > 0 ? ` · +${r.overflow} more` : ''}</small>
              </div>
            {/each}
            {#each tagged as s (s.id)}
              <div class="tag" class:needs={s.visual === 'needs-you'} use:nameLabel={s.id}>{s.initials}</div>
            {/each}
            <div class="you" bind:this={youEl}>You · headmaster</div>
          </div>
        {:else if sceneError}
          <div class="scene-state">
            <LoadState what="the 3D view" error={sceneError} variant="compact" onretry={() => (sceneAttempt += 1)} />
          </div>
        {:else}
          <div class="scene-state"><span class="spinner" aria-hidden="true"></span><span class="dim">Setting up the classrooms…</span></div>
        {/if}
        {#if empty && handle}
          <div class="empty-over">
            <EmptyState icon="people" title="Every desk is empty" body="Start a session and a student takes a seat in its workspace’s classroom.">
              <button class="btn small" onclick={() => (ui.newSessionOpen = true)}><Icon name="plus" size={12} />New session</button>
            </EmptyState>
          </div>
        {/if}
      </div>
    {/if}

    <!-- The accessible companion: every classroom and student as real
         buttons. Visible in List view; in 3D view it stays keyboard /
         screen-reader reachable, and focusing a student highlights it in 3D
         and shows its tooltip. -->
    <div class="list" class:sr-only={mode === '3d'} bind:this={listEl} role="region" aria-label="Classrooms list">
      {#if mode === 'list' && empty}
        <EmptyState icon="people" title="Every desk is empty" body="Start a session and a student takes a seat in its workspace’s classroom.">
          <button class="btn small" onclick={() => (ui.newSessionOpen = true)}><Icon name="plus" size={12} />New session</button>
        </EmptyState>
      {/if}
      {#each model.rooms as r (r.id)}
        {#if r.counts.total > 0 || mode === 'list'}
          <section class="lroom" aria-label="Classroom {r.name}">
            <h4 class="lroom-head">
              <span class="ellipsis">{r.name}</span>
              {#if r.current}<span class="cur">current</span>{/if}
              <small>{roomSummary(r)}</small>
            </h4>
            {#if r.students.length + r.backRow.length > 0}
              <ul>
                {#each [...r.students, ...r.backRow] as s (s.id)}
                  <li class="lrow" data-student-id={s.id} oncontextmenu={(e) => studentMenu(e, s)}>
                    <button
                      class="lmain"
                      aria-label="Open {studentAriaLabel(s, now() ? relTime(s.lastActiveAt) : '')}"
                      onclick={() => void open(s)}
                      onfocus={() => {
                        focusId = s.id;
                        if (mode === '3d') showTip(s.id);
                      }}
                      onblur={() => {
                        if (focusId === s.id) focusId = null;
                        if (mode === '3d') scheduleHide();
                      }}
                      onmouseenter={() => mode === 'list' && showTip(s.id)}
                      onmouseleave={() => mode === 'list' && scheduleHide()}
                    >
                      <i class="dot" data-visual={s.visual}></i>
                      <ProviderIcon provider={s.provider} size={12} />
                      <span class="ellipsis t">{s.title}</span>
                      {#if s.background}<span class="bg-tag">back row</span>{/if}
                      <span class="st">{s.stateLabel}</span>
                    </button>
                    <button class="icon-btn" aria-label="More actions for {s.title}" title="More actions" aria-haspopup="menu" onclick={(e) => studentMenu(e, s)}>
                      <Icon name="more" size={12} />
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </section>
        {/if}
      {/each}
    </div>
  {/if}
</div>

{#if tip && tipStudent && tipText}
  <!-- Hover / focus / tap card. Fixed + clamped into the viewport; it holds
       its own actions so a pointer user can act without the menu. -->
  <div
    class="tip"
    role="tooltip"
    id="classrooms-tip-{box.id}"
    bind:this={tipEl}
    onpointerenter={cancelHide}
    onpointerleave={() => {
      if (!tip?.pinned) scheduleHide();
    }}
  >
    <div class="tip-head">
      <ProviderIcon provider={tipStudent.provider} size={14} />
      <b class="ellipsis">{tipText.title}</b>
    </div>
    {#each tipText.lines as line, i (i)}
      <div class="tip-line" class:needs={i === 0 && tipStudent.visual === 'needs-you'}>{line}</div>
    {/each}
    <div class="tip-actions">
      <button class="btn small primary" onclick={() => void open(tipStudent)}>Open</button>
      {#if tipStudent.canManage}
        <button class="btn small" onclick={() => tipStudent && void detention(tipStudent)} title="Archive — resumable, with Undo">Detention</button>
        <button class="btn small danger" onclick={() => tipStudent && void kick(tipStudent)} title="Delete this session and its history">Kick out…</button>
      {/if}
    </div>
    <span class="sr-only">{providerLabel(tipStudent.provider)}</span>
  </div>
{/if}

<style>
  .cr {
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
    min-height: 0;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    overflow: hidden;
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
  .room-label,
  .tag,
  .you {
    position: absolute;
    top: 0;
    left: 0; /* ui-guards: allow — origin of projected screen coordinates, not layout */
    visibility: hidden;
    white-space: nowrap;
  }
  .room-label {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 2px 8px;
    border-radius: var(--radius-s);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-xs);
    max-width: 220px;
  }
  .room-label b {
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .room-label small {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .room-label.current {
    border-color: var(--accent-line-strong);
  }
  .room-label.current b {
    color: var(--accent-text);
  }
  .tag {
    padding: 0 4px;
    border-radius: var(--radius-s);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.4;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
  }
  .tag.needs {
    color: var(--warning);
    border-color: var(--warning);
  }
  .you {
    padding: 0 6px;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    color: var(--accent-contrast);
    background: var(--accent-solid);
  }
  .scene-state {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    align-content: center;
    gap: 8px;
  }
  .empty-over {
    position: absolute;
    inset-inline: 0;
    bottom: 8px;
    display: grid;
    place-items: center;
    pointer-events: none;
  }
  .empty-over :global(button) {
    pointer-events: auto;
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .list:not(.sr-only) {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .lroom ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .lroom-head {
    display: flex;
    align-items: baseline;
    gap: 6px;
    margin: 0 0 2px;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }
  .lroom-head small {
    font-weight: 400;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-inline-start: auto;
    white-space: nowrap;
  }
  .cur {
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--accent-text);
  }
  .lrow {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .lmain {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border: none;
    background: transparent;
    color: var(--text);
    border-radius: var(--radius-s);
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .lmain:hover {
    background: var(--hover);
  }
  .lmain:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .t {
    flex: 1;
    min-width: 0;
  }
  .st,
  .bg-tag {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
    background: var(--status-idle);
  }
  .dot[data-visual='working'] {
    background: var(--status-working);
  }
  .dot[data-visual='needs-you'] {
    background: var(--status-warn);
  }
  .dot[data-visual='away'],
  .dot[data-visual='stale'] {
    background: transparent;
    border: 1px solid var(--status-idle);
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tip {
    position: fixed;
    z-index: var(--z-drawer);
    width: max-content;
    max-width: min(320px, calc(100vw - 16px));
    max-height: calc(100vh - 16px);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-card);
    font-size: var(--fs-s);
    color: var(--text);
  }
  .tip-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-size: var(--fs-m);
  }
  .tip-head b {
    font-weight: 600;
  }
  .tip-line {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  .tip-line.needs {
    color: var(--warning);
  }
  .tip-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }
</style>
