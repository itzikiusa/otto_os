///<reference types="svelte" />
;// The floating "Type or speak… ⌘K" bar (layout.md §7) — Otto's front door.
// ONE component, two hosts:
//   • host="app"    — mounted once by the shell over the main window's
//                     content column. ⌘K focuses it (it IS the command
//                     surface on desktop; the palette sheet stays for phone/
//                     tablet, a hidden bar, and ⌘I). It gets out of the way:
//                     a short pill at rest, a chip docked in the status bar
//                     while you scroll or type in a terminal/editor, gone
//                     under sheets (see `barPresence` in lib/floatingBar.ts).
//   • host="window" — the ⌥Space panel (`#/bar`, BarHost.svelte): a
//                     transparent chromeless window over native HUD glass.
//                     The page reports its height with `bar.resize()` and
//                     the window grows UPWARD from the pill.
// Typing lists commands (the palette's registry + ranking) and an "Ask Otto"
// row; Enter runs the selection. Asking goes through `ask()` (lib/ask.ts —
// the one-line swap point) and the answer lands in the active space's
// thread, drawn above the pill in the SAME glass surface.

import { onMount, tick, untrack } from 'svelte';
import Icon, { type IconName } from './Icon.svelte';
import ProviderIcon from './ProviderIcon.svelte';
import ModelPicker from './ModelPicker.svelte';
import { api, isAbortError } from '../api/client';
import { loadErrorText } from '../loadError';
import type { Action, Repo, SearchHit, Session, WorkspaceWithRole } from '../api/types';
import { registry, type Command } from '../commands.svelte';
import { loadFrecency, rankCommands, recordUsage } from '../commandSearch';
import {
    SPACE_COUNT,
    buildRows,
    barKeyAction,
    barPresence,
    barWindowHeight,
    hitRoute,
    moveSelection,
    panelBudget,
    spaceLabel,
    type BarRow,
    type BarTurn,
  } from '../floatingBar';
import { ask, appHost, confirmClose, confirmPlan, windowHost, type AskReply } from '../ask';
import { describeAction } from '../orchestrate';
import { barStore } from '../stores/bar.svelte';
import { auth } from '../stores/auth.svelte';
import { plugins } from '../stores/plugins.svelte';
import { ui } from '../stores/ui.svelte';
import { isForeground, visibleOnThisDevice, ws } from '../stores/workspace.svelte';
import { router } from '../router.svelte';
import { keyContext } from '../keys';
import { radioKey } from '../radioKey';
import { agentProviders, defaultAgentProvider } from '../providers';
import { SIDEBAR_MODULES, availableModules, goToEntries } from '../sidebar';
import { bar, onDesktopEvent, openInOtto } from '../desktop';
;

  interface Props {
    host?: 'app' | 'window';
  };;

  // ── panel view ──────────────────────────────────────────────────────────
  type View = 'space' | 'results' | 'thread' | 'recent' | null;;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { host = 'window' }: Props = $props();
  const inApp = untrack(() => host) === 'app';
  const askHost = inApp ? appHost : windowHost;

  /** Pill heights: the ⌥Space window's is fixed by the shell (PILL_H = 56). */
  const PILL_H = inApp ? 46 : 56;
  const CMD_LIMIT = 8;
  const HIT_LIMIT = 5;
  const SEARCH_DEBOUNCE_MS = 250;
  const SCROLL_SETTLE_MS = 900;

  let rootEl: HTMLDivElement | undefined = $state();
  let pillEl: HTMLDivElement | undefined = $state();
  let inputEl: HTMLInputElement | undefined = $state();
  let threadEl: HTMLDivElement | undefined = $state();
  let panelInner: HTMLDivElement | undefined = $state();

  let query = $state('');
  let selected = $state(0);
  /** Focus is inside the bar. */
  let focused = $state(false);
  /** Opening from the dock/rest chip before the input has focus. */
  let opening = $state(false);
  /** The space editor (name, workspace, agent, model) is showing. */
  let editing = $state(false);
  /** A reply landed while the in-app bar was closed. */
  let unread = $state(false);
  // The unread dot is decorative; its meaning reaches AT as the buttons' description.
  const uid = $props.id();
  /** Turn ids with a request in flight (Run plan / Delete / Ask). */
  let busy: Record<string, boolean> = $state({});

  // ── presence (in-app) ───────────────────────────────────────────────────
  let workFocus = $state(false);
  let scrolling = $state(false);
  const open = $derived(focused || opening);
  const presence = $derived(
    inApp
      ? barPresence({
          pref: barStore.pref,
          open,
          surface: router.module === 'home',
          workFocus,
          scrolling,
          overlay: ui.overlayOpen,
        })
      : 'full',
  );

  // ── spaces ──────────────────────────────────────────────────────────────
  const spaceIdx = $derived(barStore.state.active);
  const space = $derived(barStore.active);
  const thread = $derived(space.thread);
  const provider = $derived(space.provider || defaultAgentProvider());

  let workspaces: WorkspaceWithRole[] = $state([]);
  const workspaceList = $derived(inApp ? ws.workspaces : workspaces);
  const spaceWorkspace = $derived(space.workspaceId ?? askHost.currentWorkspace());
  const spaceWorkspaceName = $derived(
    workspaceList.find((w) => w.id === spaceWorkspace)?.name ?? null,
  );

  function selectSpace(i: number): void {
    if (i === spaceIdx) {
      editing = !editing;
      return;
    }
    barStore.setActive(i);
    editing = false;
    // In the main window a space is a context: switch to its workspace.
    const target = barStore.active.workspaceId;
    if (inApp && target && target !== ws.currentId && ws.workspaces.some((w) => w.id === target)) {
      void ws.select(target);
    }
  }

  // ── commands ────────────────────────────────────────────────────────────
  // In-app: the palette's registry. In the ⌥Space window there is no shell to
  // register commands, so a small set is built from the API: Go to <module>,
  // Focus session, Open repo — each opens the main window at that route.
  let windowCommands: Command[] = $state([]);

  async function loadWindowCommands(): Promise<void> {
    if (auth.phase !== 'ready') {
      windowCommands = [];
      return;
    }
    const go = (route?: string) => () => openInOtto(route);
    // Go-to rows for every module the user may open — plugins included, like
    // the in-app list (App.svelte). Built at once from what is known, rebuilt
    // when the plugin list arrives (fetched alongside the workspaces).
    const buildBase = (): Command[] => {
      const pluginEntries = plugins.list
        .filter((p) => auth.canPlugin(p.slug, 'view'))
        .map((p) => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name }));
      const mods = availableModules((f) => auth.can(f, 'view'), pluginEntries);
      return [
        { id: 'bar.open-otto', title: 'Open Otto', group: 'Navigate', keywords: 'main window app show', run: go() },
        ...goToEntries(mods).map((e) => ({
          id: e.id,
          title: e.title,
          group: 'Navigate',
          detail: e.detail,
          keywords: e.keywords,
          run: go(e.route),
        })),
        { id: 'core.go-settings', title: 'Go to Settings', group: 'Navigate', keywords: 'preferences appearance', run: go('settings/appearance') },
      ];
    };
    windowCommands = buildBase();
    const pluginsLoaded = plugins.load();
    try {
      workspaces = await api.get<WorkspaceWithRole[]>('/workspaces');
    } catch {
      workspaces = [];
    }
    await pluginsLoaded;
    const base = buildBase();
    windowCommands = base;
    const wsId = spaceWorkspace;
    if (!wsId) return;
    // A failed fetch must not look like "no sessions": one disabled row says
    // so (the Go-to list above still works).
    let failed = false;
    const [sessions, repos] = await Promise.all([
      api.get<Session[]>(`/workspaces/${wsId}/sessions?archived=false`).catch(() => ((failed = true), [] as Session[])),
      api.get<Repo[]>(`/workspaces/${wsId}/repos`).catch(() => ((failed = true), [] as Repo[])),
    ]);
    windowCommands = [
      ...base,
      ...(failed
        ? [{ id: 'bar.load-failed', title: 'Couldn’t load sessions and repositories', group: 'Session', keywords: 'session repo focus open', disabled: true, run: () => {} }]
        : []),
      ...sessions
        .filter((s) => !s.archived && isForeground(s) && visibleOnThisDevice(s))
        .map((s) => ({
          id: `session.${s.id}`,
          title: `Focus ${s.title}`,
          detail: 'Session',
          group: 'Session',
          keywords: s.provider,
          run: go(`agents/${s.id}`),
        })),
      ...repos.map((r) => ({
        id: `repo.${r.id}`,
        title: `Open repository ${r.name}`,
        group: 'Git',
        keywords: `repository ${r.path}`,
        run: go(`git/${r.id}`),
      })),
    ];
  }

  const commands = $derived(inApp ? registry.all : windowCommands);
  const ranked = $derived(rankCommands(commands, query, loadFrecency(), Date.now(), CMD_LIMIT));

  // Cross-module search (repos, workflows, stories, …) — the palette's
  // /search fan-out, debounced; hits trail the commands.
  let hits: SearchHit[] = $state([]);
  let searching = $state(false);
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  let searchAbort: AbortController | null = null;

  $effect(() => {
    const q = query.trim();
    const wsId = spaceWorkspace;
    untrack(() => {
      if (searchTimer) clearTimeout(searchTimer);
      searchAbort?.abort();
      searchAbort = null;
      if (q.length < 2 || !wsId || auth.phase !== 'ready') {
        hits = [];
        searching = false;
        return;
      }
      searching = true;
      searchTimer = setTimeout(() => {
        const ctrl = new AbortController();
        searchAbort = ctrl;
        api
          .get<SearchHit[]>(`/workspaces/${wsId}/search?q=${encodeURIComponent(q)}`, ctrl.signal)
          .then((r) => (hits = (r ?? []).slice(0, HIT_LIMIT)))
          .catch((e: unknown) => {
            if (!isAbortError(e)) hits = [];
          })
          .finally(() => {
            if (searchAbort === ctrl) {
              searchAbort = null;
              searching = false;
            }
          });
      }, SEARCH_DEBOUNCE_MS);
    });
  });

  const rows: BarRow<Command, SearchHit>[] = $derived(
    buildRows(query, query.trim() === '' ? ranked.slice(0, 6).map((r) => r.cmd) : ranked.map((r) => r.cmd), hits),
  );

  // A new query puts the selection back on the Enter default (row 0). Rows
  // that arrive later (debounced search hits) only append, so they never
  // yank the selection out from under the arrow keys — just keep it in range.
  $effect(() => {
    void query;
    untrack(() => (selected = rows.length > 0 ? 0 : -1));
  });
  $effect(() => {
    // Closed → don't keep the command ranking alive just to clamp a selection
    // nobody sees; opening re-runs this and lands on row 0.
    const n = showPanel ? rows.length : 0;
    untrack(() => {
      if (selected >= n) selected = n - 1;
      else if (selected < 0 && n > 0) selected = 0;
    });
  });

  const view: View = $derived.by(() => {
    if (editing) return 'space';
    if (query.trim() !== '') return 'results';
    if (thread.length > 0) return 'thread';
    return inApp && rows.length > 0 ? 'recent' : null;
  });
  // A closed in-app bar short-circuits BEFORE `view` — reading it would rank
  // every command (plus a frecency lookup) on each registry change while
  // docked (SF-06). This was once reverted because ⌘K then showed a list
  // without the Go-to entries; the real cause was the registry rebuilding its
  // map from a teardown-time `$state` read (commands.svelte.ts,
  // unit/commandRegistry.test.ts), which this gate merely exposed.
  const showPanel = $derived(inApp && !open ? false : view !== null);
  const listOpen = $derived(showPanel && (view === 'results' || view === 'recent'));
  const activeId = $derived(listOpen && selected >= 0 ? `fb-opt-${selected}` : undefined);

  // ── actions ─────────────────────────────────────────────────────────────
  function closeBar(): void {
    editing = false;
    query = '';
    opening = false;
    if (inApp) inputEl?.blur();
  }

  async function openBar(): Promise<void> {
    opening = true;
    await tick();
    inputEl?.focus();
    inputEl?.select();
    opening = false;
    unread = false;
  }

  async function runCommand(cmd: Command): Promise<void> {
    if (cmd.disabled) return;
    recordUsage(cmd.id);
    closeBar();
    try {
      await cmd.run();
    } catch (e) {
      addTurn('', { tone: 'error', text: `Couldn’t run “${cmd.title}”.`, detail: loadErrorText(e), source: 'Otto' });
    }
  }

  function openRoute(route: string): void {
    if (inApp) {
      closeBar();
      router.go(route);
    } else {
      void openInOtto(route).catch(() => {});
    }
  }

  function newId(): string {
    return `t${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
  }

  function replyPatch(r: AskReply): Partial<BarTurn> {
    return {
      a: r.text,
      tone: r.tone,
      detail: r.detail,
      route: r.route,
      source: r.source,
      plan: r.plan,
      closeIds: r.closeIds,
      closePermanent: r.closePermanent,
    };
  }

  function addTurn(q: string, r: AskReply): void {
    barStore.addTurn(spaceIdx, { id: newId(), q, at: Date.now(), ...replyPatch(r), a: r.text, tone: r.tone });
  }

  async function submitAsk(text: string): Promise<void> {
    const t = text.trim();
    if (t === '') return;
    const idx = spaceIdx;
    const sp = barStore.state.spaces[idx];
    const id = newId();
    barStore.addTurn(idx, { id, q: t, a: '', tone: 'pending', at: Date.now(), source: 'Ask Otto' });
    busy[id] = true;
    query = '';
    editing = false;
    const reply = await ask(t, sp, askHost);
    barStore.updateTurn(idx, id, replyPatch(reply));
    delete busy[id];
    if (inApp && !focused) unread = true;
  }

  async function resolveTurn(turn: BarTurn, run: boolean): Promise<void> {
    const idx = spaceIdx;
    const sp = barStore.state.spaces[idx];
    if (!run) {
      barStore.updateTurn(idx, turn.id, {
        a: 'Canceled — nothing ran.',
        tone: 'info',
        detail: undefined,
        plan: undefined,
        closeIds: undefined,
      });
      return;
    }
    busy[turn.id] = true;
    const reply = turn.plan
      ? await confirmPlan(turn.plan as Action[], sp, askHost)
      : await confirmClose(turn.closeIds ?? [], sp, askHost, turn.closePermanent !== false);
    barStore.updateTurn(idx, turn.id, { ...replyPatch(reply), plan: undefined, closeIds: undefined });
    delete busy[turn.id];
  }

  function runRow(row: BarRow<Command, SearchHit> | undefined): void {
    if (!row) return;
    if (row.kind === 'cmd') void runCommand(row.cmd);
    else if (row.kind === 'ask') void submitAsk(row.text);
    else openRoute(hitRoute(row.hit));
  }

  function onKey(e: KeyboardEvent): void {
    const a = barKeyAction(e);
    if (!a) return;
    switch (a.type) {
      case 'space':
        e.preventDefault();
        selectSpace(a.index);
        return;
      case 'next':
      case 'prev':
        if (!listOpen) return;
        e.preventDefault();
        selected = moveSelection(selected, a.type === 'next' ? 1 : -1, rows.length);
        void tick().then(() =>
          document.getElementById(`fb-opt-${selected}`)?.scrollIntoView({ block: 'nearest' }),
        );
        return;
      case 'run':
        e.preventDefault();
        if (listOpen) runRow(rows[selected]);
        else if (query.trim() !== '') void submitAsk(query);
        return;
      case 'ask':
        e.preventDefault();
        void submitAsk(query);
        return;
      case 'escape':
        e.preventDefault();
        e.stopPropagation();
        if (editing) editing = false;
        else if (query !== '') query = '';
        else if (inApp) closeBar();
        else void bar.hide().catch(() => {});
        return;
    }
  }

  function onFocusIn(): void {
    focused = true;
    if (inApp) registry.want();
    unread = false;
    if (inApp) keyContext.barFocused = true;
  }

  function onFocusOut(e: FocusEvent): void {
    const next = e.relatedTarget as Node | null;
    if (next && rootEl?.contains(next)) return;
    focused = false;
    editing = false;
    if (inApp) keyContext.barFocused = false;
  }

  // ⌘K (shell keymap → barStore.requestFocus): focus the bar, or close it.
  let lastTick = untrack(() => barStore.focusTick);
  $effect(() => {
    const t = barStore.focusTick;
    if (!inApp || t === lastTick) return;
    lastTick = t;
    untrack(() => {
      if (focused) closeBar();
      else void openBar();
    });
  });

  // Register as THE ⌘K surface while mounted in-app and not hidden.
  $effect(() => {
    if (!inApp) return;
    barStore.mounted = barStore.pref !== 'hidden';
    return () => {
      barStore.mounted = false;
      keyContext.barFocused = false;
    };
  });

  // Newest turn in view.
  $effect(() => {
    void thread.length;
    void view;
    void tick().then(() => {
      if (threadEl) threadEl.scrollTop = threadEl.scrollHeight;
    });
  });

  // ── geometry ────────────────────────────────────────────────────────────
  // In-app: the panel may grow up to the top of the window (panelBudget).
  let budget = $state(420);
  /** Measured from the HOST column, not the pill: opening from the docked
   *  chip, the pill is still display:none on the first run (top 0), which
   *  capped the list at its 120px floor — two and a half rows. The pill
   *  always rests 16px above the host's bottom edge when it is open. */
  function measure(): void {
    const host = rootEl?.parentElement;
    if (host) {
      budget = panelBudget(host.getBoundingClientRect().bottom - 16 - PILL_H, 0);
      return;
    }
    if (pillEl) budget = panelBudget(pillEl.getBoundingClientRect().top, 0);
  }
  $effect(() => {
    if (!inApp || !showPanel) return;
    measure();
    void tick().then(measure);
    window.addEventListener('resize', measure);
    return () => window.removeEventListener('resize', measure);
  });

  // Toasts sit bottom-right over the same column: while the pill floats
  // there, lift the stack above it (barStore.toastLift → `--toast-lift` on the
  // toast stack only). While the panel (⌘K results / a reply) is open, the
  // stack goes above the whole surface — a toast used to sit on top of the
  // panel's right edge. Never on <html>: an inherited custom property there
  // restyles the whole document (0.4–0.8 s on a 100k-line diff) per change.
  $effect(() => {
    if (!inApp) return;
    const base = presence === 'full' ? PILL_H + 12 : presence === 'rest' ? 36 + 12 : 0;
    const apply = () => {
      let lift = base;
      const surface = showPanel ? rootEl?.querySelector<HTMLElement>('.surface') : null;
      if (surface) {
        // Toasts sit at bottom: 38px + lift; keep 12px clear of the surface top.
        lift = Math.max(base, Math.round(window.innerHeight - surface.getBoundingClientRect().top - 26));
      }
      barStore.setToastLift(lift);
    };
    apply();
    let ro: ResizeObserver | null = null;
    let raf = 0;
    if (showPanel) {
      void tick().then(apply);
      const surface = rootEl?.querySelector<HTMLElement>('.surface');
      if (surface) {
        // One measure per frame however many resize notifications land.
        ro = new ResizeObserver(() => {
          if (!raf) raf = requestAnimationFrame(() => ((raf = 0), apply()));
        });
        ro.observe(surface);
      }
    }
    return () => {
      ro?.disconnect();
      if (raf) cancelAnimationFrame(raf);
      barStore.setToastLift(0);
    };
  });

  // ⌥Space window: report the content height so the native panel grows upward.
  let lastH = 0;
  function reportHeight(): void {
    if (inApp) return;
    const panelH = showPanel && panelInner ? panelInner.scrollHeight + 1 : 0;
    const h = barWindowHeight(PILL_H, panelH);
    if (h === lastH) return;
    lastH = h;
    void bar.resize(h).catch(() => {});
  }
  $effect(() => {
    if (inApp) return;
    void showPanel;
    void view;
    void rows.length;
    void thread;
    void tick().then(reportHeight);
    if (!panelInner) return;
    const ro = new ResizeObserver(() => reportHeight());
    ro.observe(panelInner);
    return () => ro.disconnect();
  });

  // Scroll clearance: while the pill can rest over content, the content
  // column exposes its height as --fb-clearance so PageBody (and Home) pad
  // their scroll ends — the last row can always scroll clear of the pill.
  // Keyed on the preference, not the live presence, so docking while you
  // scroll never shifts the layout under you.
  $effect(() => {
    const host = inApp ? rootEl?.parentElement : null;
    if (!host) return;
    const clear = barStore.pref === 'auto' || barStore.pref === 'pinned';
    host.style.setProperty('--fb-clearance', clear ? `${PILL_H + 16 + 12}px` : '0px');
    return () => host.style.removeProperty('--fb-clearance');
  });

  // ── in-app: step aside while the user works ─────────────────────────────
  function isWorkTarget(el: Element | null): boolean {
    if (!el || (rootEl && rootEl.contains(el))) return false;
    const h = el as HTMLElement;
    return (
      h.tagName === 'INPUT' ||
      h.tagName === 'TEXTAREA' ||
      h.tagName === 'SELECT' ||
      h.isContentEditable ||
      !!h.closest?.('.xterm, .cm-editor')
    );
  }

  onMount(() => {
    if (!inApp) {
      let offShown: (() => void) | null = null;
      let offHidden: (() => void) | null = null;
      void loadWindowCommands();
      void onDesktopEvent('otto://bar-shown', () => {
        void loadWindowCommands();
        void openBar();
      }).then((fn) => (offShown = fn));
      void onDesktopEvent('otto://bar-hidden', () => {
        query = '';
        editing = false;
      }).then((fn) => (offHidden = fn));
      queueMicrotask(() => inputEl?.focus());
      return () => {
        offShown?.();
        offHidden?.();
      };
    }
    const onFocusChange = (): void => {
      workFocus = isWorkTarget(document.activeElement);
    };
    const onFocusOut = (): void => queueMicrotask(onFocusChange);
    document.addEventListener('focusin', onFocusChange);
    document.addEventListener('focusout', onFocusOut);
    return () => {
      document.removeEventListener('focusin', onFocusChange);
      document.removeEventListener('focusout', onFocusOut);
    };
  });

  // `scrolling` only moves the pill in auto mode on Home (barPresence), so the
  // app-wide capture scroll listener exists only there — not on every scroll
  // of a diff, grid, editor or terminal elsewhere.
  const watchScroll = $derived(inApp && barStore.pref === 'auto' && router.module === 'home');
  $effect(() => {
    if (!watchScroll) return;
    let scrollTimer: ReturnType<typeof setTimeout> | null = null;
    let armedAt = 0;
    const onScroll = (e: Event): void => {
      const t = e.target as Node | null;
      if (t && rootEl?.contains(t)) return;
      scrolling = true;
      // Re-arm the settle timer at most every 100 ms of continuous scrolling.
      const now = performance.now();
      if (scrollTimer && now - armedAt < 100) return;
      armedAt = now;
      if (scrollTimer) clearTimeout(scrollTimer);
      scrollTimer = setTimeout(() => ((scrollTimer = null), (scrolling = false)), SCROLL_SETTLE_MS);
    };
    document.addEventListener('scroll', onScroll, { capture: true, passive: true });
    return () => {
      if (scrollTimer) clearTimeout(scrollTimer);
      document.removeEventListener('scroll', onScroll, { capture: true });
      scrolling = false;
    };
  });

  // Reload the ⌥Space command set once sign-in completes / the space moves.
  $effect(() => {
    if (inApp) return;
    void auth.phase;
    void spaceWorkspace;
    untrack(() => void loadWindowCommands());
  });

  // ── rendering helpers ───────────────────────────────────────────────────
  function rowIcon(row: BarRow<Command, SearchHit>): IconName {
    if (row.kind === 'ask') return 'sparkle';
    if (row.kind === 'hit') {
      // Same glyph per kind as the ⌘K palette's results (Palette.svelte
      // hitIcon) — one search, one vocabulary.
      switch (row.hit.kind) {
        case 'repo': return 'branch';
        case 'workflow': return 'merge';
        case 'story': return 'ticket';
        case 'api_request': return 'zap';
        case 'swarm_task': return 'check';
        case 'swarm_project': return 'layers';
        case 'broker_cluster': return 'box';
        case 'memory': return 'db';
        default: return 'file';
      }
    }
    const g = row.cmd.group ?? '';
    if (row.cmd.id.startsWith('core.go-')) {
      const mod = SIDEBAR_MODULES.find((m) => `core.go-${m.id}` === row.cmd.id);
      return mod?.icon ?? 'chevronRight';
    }
    if (g === 'Navigate') return 'chevronRight';
    if (g === 'Sessions' || g === 'Session') return 'terminal';
    if (g === 'Git' || row.cmd.id.startsWith('repo.')) return 'branch';
    if (g === 'Workspaces') return 'folder';
    if (g === 'Connections') return 'plug';
    if (g === 'Appearance' || g === 'View') return 'layout';
    return 'command';
  }

  function timeOf(ms: number): string {
    return ms ? new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : '';
  }

  function providerName(p: string): string {
    return p === '' ? 'Default' : p.charAt(0).toUpperCase() + p.slice(1);
  }

  const modelLabel = $derived(space.model || 'default');
  const signedOut = $derived(!inApp && auth.phase !== 'ready' && auth.phase !== 'loading');
;
async () => {

if(presence !== 'off'){
   { const $$_div0 = svelteHTML.createElement("div", {                "class":`fb`,...__sveltets_2_empty({"data-presence":presence}),...__sveltets_2_empty({"data-testid":`floating-bar`}),"onfocusin":onFocusIn,"onfocusout":onFocusOut,"inert":presence === 'away' ? true : undefined,});inApp;!inApp;rootEl = $$_div0;
     { svelteHTML.createElement("span", {   "id":`${uid}-unread`,"class":`sr-only`,});  }
    if(presence === 'dock'){
      
       { svelteHTML.createElement("button", {             "class":`dock`,"onclick":() => void openBar(),"title":`Ask Otto or run a command`,"aria-label":`Open the Otto bar`,"aria-describedby":unread ? `${uid}-unread` : undefined,"aria-keyshortcuts":`Meta+K`,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});}
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("kbd", {});  }
        if(unread){ { svelteHTML.createElement("span", {   "class":`unread`,"aria-hidden":`true`,}); }}
       }
    }

     { svelteHTML.createElement("div", {        "class":`surface`,"hidden":presence === 'dock',});showPanel;inApp && focused;presence === 'rest';
      if(showPanel){
         { svelteHTML.createElement("div", {    "class":`panel`,});__sveltets_2_ensureType(String, Number, inApp ? `${Math.max(120, budget - PILL_H - 8)}px` : undefined);
           { const $$_div3 = svelteHTML.createElement("div", {  "class":`panel-inner`,});panelInner = $$_div3;
            if(signedOut){
               { svelteHTML.createElement("div", { "class":`note`,});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":14,}});}
                 { svelteHTML.createElement("span", {});        }
                 { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void openInOtto(),});  }
               }
            }

            if(view === 'space'){
               { svelteHTML.createElement("section", {   "class":`editor`,"aria-labelledby":`fb-space-title`,});
                 { svelteHTML.createElement("header", { "class":`ed-head`,});
                   { svelteHTML.createElement("span", { "class":`sp-num`,});spaceLabel(spaceIdx); }
                   { svelteHTML.createElement("h2", { "id":`fb-space-title`,});  }
                   { svelteHTML.createElement("span", { "class":`grow`,}); }
                   { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (editing = false),});  }
                 }
                 { svelteHTML.createElement("div", { "class":`ed-grid`,});
                   { svelteHTML.createElement("label", { "class":`ed-field`,});
                     { svelteHTML.createElement("span", {});  }
                     { svelteHTML.createElement("input", {          "dir":`auto`,"class":`input`,"value":space.name,"maxlength":24,"onchange":(e) => barStore.patchSpace(spaceIdx, { name: e.currentTarget.value.trim() || space.name }),});}
                   }
                   { svelteHTML.createElement("label", { "class":`ed-field`,});
                     { svelteHTML.createElement("span", {});  }
                     { svelteHTML.createElement("select", {       "class":`input`,"value":space.workspaceId ?? '',"onchange":(e) => barStore.patchSpace(spaceIdx, { workspaceId: e.currentTarget.value || null }),});
                       { svelteHTML.createElement("option", {"value":"",});    }
                         for(let w of __sveltets_2_ensureArray(workspaceList)){w.id;
                         { svelteHTML.createElement("option", { "value":w.id,});w.name; }
                      }
                     }
                   }
                   { svelteHTML.createElement("label", { "class":`ed-field`,});
                     { svelteHTML.createElement("span", {});  }
                     { svelteHTML.createElement("select", {       "class":`input`,"value":space.provider,"onchange":(e) => barStore.patchSpace(spaceIdx, { provider: e.currentTarget.value, model: '' }),});
                       { svelteHTML.createElement("option", {"value":"",}); providerName(defaultAgentProvider());  }
                         for(let p of __sveltets_2_ensureArray(agentProviders())){p;
                         { svelteHTML.createElement("option", { "value":p,});providerName(p); }
                      }
                     }
                   }
                   { svelteHTML.createElement("div", { "class":`ed-model`,});
                     { const $$_rekciPledoM7C = __sveltets_2_ensureComponent(ModelPicker); new $$_rekciPledoM7C({ target: __sveltets_2_any(), props: {          "id":`fb-space-model`,provider,"value":space.model,"onchange":(m) => barStore.patchSpace(spaceIdx, { model: m }),"hint":`Kept with this space for its assistant thread; Ask Otto plans with the orchestrator today.`,}});}
                   }
                 }
                 { svelteHTML.createElement("footer", { "class":`ed-foot`,});
                   { svelteHTML.createElement("span", { "class":`dim`,});       }
                   { svelteHTML.createElement("span", { "class":`grow`,}); }
                   { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":thread.length === 0,"onclick":() => barStore.clearThread(spaceIdx),});  }
                 }
               }
            } else if (view === 'thread'){
               { svelteHTML.createElement("div", { "class":`thread-head`,});
                 { svelteHTML.createElement("span", { "class":`sp-num`,});spaceLabel(spaceIdx); }
                 { svelteHTML.createElement("span", { "class":`sp-name`,});space.name; }
                if(spaceWorkspaceName){ { svelteHTML.createElement("span", { "class":`dim`,}); spaceWorkspaceName; }}
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                 { svelteHTML.createElement("button", {   "class":`link-btn`,"onclick":() => barStore.clearThread(spaceIdx),});  }
               }
               { const $$_div4 = svelteHTML.createElement("div", {    "class":`thread`,"aria-live":`polite`,});threadEl = $$_div4;
                   for(let t of __sveltets_2_ensureArray(thread)){t.id;
                   { svelteHTML.createElement("article", {   "class":`turn`,...__sveltets_2_empty({"data-tone":t.tone}),});
                    if(t.q){ { svelteHTML.createElement("p", { "class":`q`,});t.q; }}
                    if(t.tone === 'pending' && t.a === ''){
                       { svelteHTML.createElement("p", { "class":`a thinking`,}); { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 12px`,"aria-hidden":`true`,}); }   }
                    }else{
                       { svelteHTML.createElement("p", { "class":`a`,});
                        if(t.tone === 'error' || t.tone === 'warn'){
                           { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});}
                        } else if (t.tone === 'ok'){
                           { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":13,}});}
                        }
                         { svelteHTML.createElement("span", {});t.a; }
                       }
                    }
                    if(t.detail){ { svelteHTML.createElement("p", { "class":`detail`,});t.detail; }}
                    if(t.plan && t.plan.length > 0){
                       { svelteHTML.createElement("ol", { "class":`plan`,});
                            for(let a of __sveltets_2_ensureArray(t.plan)){let i = 1;i;
                           { svelteHTML.createElement("li", {});describeAction(a as Action); }
                        }
                       }
                    }
                    if((t.plan && t.plan.length > 0) || t.closeIds){
                       { svelteHTML.createElement("div", { "class":`confirm`,});
                         { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy[t.id],"onclick":() => void resolveTurn(t, false),});  }
                         { svelteHTML.createElement("button", {         "class":`btn small`,"disabled":busy[t.id],"onclick":() => void resolveTurn(t, true),});!t.closeIds || t.closePermanent === false;!!t.closeIds && t.closePermanent !== false;busy[t.id] ? 'Running…' : t.closeIds ? (t.closePermanent !== false ? 'Delete' : 'Archive') : 'Run plan'; }
                       }
                    }
                     { svelteHTML.createElement("footer", { "class":`meta`,});
                       { svelteHTML.createElement("span", { "class":`src`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});}t.source ?? 'Ask Otto'; }
                       { svelteHTML.createElement("span", {});timeOf(t.at); }
                      if(t.route){
                         { svelteHTML.createElement("span", { "class":`grow`,}); }
                         { svelteHTML.createElement("button", {   "class":`open-btn`,"onclick":() => openRoute(t.route ?? ''),});
                          inApp ? 'Open' : 'Open in Otto';  { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":inApp ? 'chevronRight' : 'external',"size":12,}});}
                         }
                      }
                     }
                   }
                }
               }
            } else if (view === 'results' || view === 'recent'){
               { svelteHTML.createElement("ul", {       "class":`list`,"id":`fb-listbox`,"role":`listbox`,"aria-label":view === 'recent' ? 'Recent commands' : 'Results',});
                if(view === 'recent'){
                   { svelteHTML.createElement("li", {   "class":`sec`,"role":`presentation`,});  }
                }
                    for(let row of __sveltets_2_ensureArray(rows)){let i = 1;row.key;
                  if(row.kind === 'hit' && (i === 0 || rows[i - 1].kind !== 'hit')){
                     { svelteHTML.createElement("li", {   "class":`sec`,"role":`presentation`,});searching ? 'Searching…' : 'In this workspace'; }
                  }
                  
                   { svelteHTML.createElement("li", {                  "id":`fb-opt-${i}`,"class":`opt`,"role":`option`,"aria-selected":i === selected,"aria-disabled":row.kind === 'cmd' && row.cmd.disabled ? 'true' : undefined,"onmousedown":(e) => { e.preventDefault(); runRow(row); },"onmousemove":() => (selected = i),});row.kind === 'ask';i === selected;row.kind === 'cmd' && row.cmd.disabled;
                     { svelteHTML.createElement("span", { "class":`opt-ic`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":rowIcon(row),"size":13,}});} }
                    if(row.kind === 'ask'){
                       { svelteHTML.createElement("span", { "class":`opt-title`,});   { svelteHTML.createElement("span", { "class":`q-inline`,}); row.text;  } }
                       { svelteHTML.createElement("span", { "class":`grow`,}); }
                       { svelteHTML.createElement("span", { "class":`opt-meta`,});space.name; }
                      if(i === selected){ { svelteHTML.createElement("kbd", {});  }}else{ { svelteHTML.createElement("kbd", {});  }}
                    } else if (row.kind === 'cmd'){
                       { svelteHTML.createElement("span", { "class":`opt-title`,});row.cmd.title; }
                      if(row.cmd.detail){ { svelteHTML.createElement("span", { "class":`opt-detail`,});row.cmd.detail; }}
                       { svelteHTML.createElement("span", { "class":`grow`,}); }
                      if(row.cmd.group){ { svelteHTML.createElement("span", { "class":`opt-meta`,});row.cmd.group; }}
                      if(row.cmd.shortcut){ { svelteHTML.createElement("kbd", {});row.cmd.shortcut; }}
                    }else{
                       { svelteHTML.createElement("span", { "class":`opt-title`,});row.hit.title; }
                      if(row.hit.subtitle){ { svelteHTML.createElement("span", { "class":`opt-detail`,});row.hit.subtitle; }}
                       { svelteHTML.createElement("span", { "class":`grow`,}); }
                       { svelteHTML.createElement("span", { "class":`opt-meta`,});row.hit.kind.replace('_', ' '); }
                    }
                   }
                }
               }
               { svelteHTML.createElement("div", {   "class":`keys`,"aria-hidden":`true`,});
                 { svelteHTML.createElement("span", {}); { svelteHTML.createElement("kbd", {});  }  }
                 { svelteHTML.createElement("span", {}); { svelteHTML.createElement("kbd", {});  }  }
                 { svelteHTML.createElement("span", {}); { svelteHTML.createElement("kbd", {});  }   }
                 { svelteHTML.createElement("span", {}); { svelteHTML.createElement("kbd", {});  }  }
                 { svelteHTML.createElement("span", {}); { svelteHTML.createElement("kbd", {});  } inApp ? 'close' : 'hide'; }
               }
            }
           }
         }
      }

       { const $$_div2 = svelteHTML.createElement("div", {  "class":`bar-row`,});pillEl = $$_div2;
         { svelteHTML.createElement("button", {             "class":`spark`,"tabindex":-1,"aria-label":`Ask Otto`,"aria-describedby":unread ? `${uid}-unread` : undefined,"title":`Ask Otto`,"onclick":() => void openBar(),});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":inApp ? 14 : 16,}});}
          if(unread){ { svelteHTML.createElement("span", {   "class":`unread`,"aria-hidden":`true`,}); }}
         }
         { const $$_input3 = svelteHTML.createElement("input", {                               "dir":`ltr`,"bind:value":query,"class":`input-main`,"type":`text`,"role":`combobox`,"aria-label":`Ask Otto or search commands`,"aria-expanded":listOpen,"aria-controls":listOpen ? 'fb-listbox' : undefined,"aria-activedescendant":activeId,"aria-autocomplete":`list`,"placeholder":`Type or speak…`,"spellcheck":`false`,"autocomplete":`off`,"onkeydown":onKey,"onfocus":() => (opening = false),});inputEl = $$_input3;/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        if(presence === 'full'){
           { svelteHTML.createElement("kbd", {   "class":`k-hint`,"title":inApp ? '⌘K focuses this bar from anywhere in Otto' : 'Commands and Ask Otto',});  }
           { svelteHTML.createElement("span", {   "class":`sep`,"aria-hidden":`true`,}); }
           { svelteHTML.createElement("button", {         "class":`fb-chip model`,"onclick":() => (editing = !editing),"aria-expanded":editing,"title":`${space.name}: ${providerName(provider)} · ${modelLabel} — change the space’s agent, model and workspace`,});
             { const $$_nocIredivorP4C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP4C({ target: __sveltets_2_any(), props: {   provider,"size":13,}});}
             { svelteHTML.createElement("span", { "class":`fb-chip-label`,});modelLabel; }
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
           }
           { svelteHTML.createElement("span", {   "class":`sep`,"aria-hidden":`true`,}); }
           { svelteHTML.createElement("div", {     "class":`spaces`,"role":`radiogroup`,"aria-label":`Spaces (⌃1–⌃4)`,});
               for(let i of __sveltets_2_ensureArray((Array.from({ length: SPACE_COUNT }, (_, i) => i)))){i;
              const sp = barStore.state.spaces[i];
               { svelteHTML.createElement("button", {                 "class":`space`,"role":`radio`,"aria-checked":i === spaceIdx,"tabindex":i === spaceIdx ? 0 : -1,"onkeydown":radioKey,"title":`${spaceLabel(i)} · ${sp.name}${i === spaceIdx ? ' — click again for settings' : ''} (⌃${i + 1})`,"aria-label":`Space ${i + 1}: ${sp.name}`,"onclick":() => selectSpace(i),});spaceLabel(i); }
            }
           }
           { svelteHTML.createElement("button", {           "class":`mic`,"aria-disabled":`true`,"aria-label":`Voice input (not available yet)`,"title":`Voice input arrives with the assistant update — type for now`,"onclick":(e) => e.preventDefault(),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`mic`,"size":14,}});}
           }
        }else{
           { svelteHTML.createElement("kbd", { "class":`k-hint`,});  }
        }
       }
     }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const FloatingBar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type FloatingBar__SvelteComponent_ = ReturnType<typeof FloatingBar__SvelteComponent_>;
/*Ωignore_endΩ*/export default FloatingBar__SvelteComponent_;