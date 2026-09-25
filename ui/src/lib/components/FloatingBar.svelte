<script lang="ts">
  // The floating "Type or speak… ⌘K" bar (layout.md §7) — Otto's front door.
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
  import { ui } from '../stores/ui.svelte';
  import { isForeground, visibleOnThisDevice, ws } from '../stores/workspace.svelte';
  import { router } from '../router.svelte';
  import { keyContext } from '../keys';
  import { agentProviders, defaultAgentProvider } from '../providers';
  import { SIDEBAR_MODULES, availableModules, groupLabel } from '../sidebar';
  import { bar, onDesktopEvent, openInOtto } from '../desktop';

  interface Props {
    host?: 'app' | 'window';
  }
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
    const mods = availableModules((f) => auth.can(f, 'view'), []);
    const base: Command[] = [
      { id: 'bar.open-otto', title: 'Open Otto', group: 'Otto', keywords: 'main window app show', run: go() },
      ...mods.map((m) => ({
        id: `core.go-${m.id}`,
        title: `Go to ${m.label}`,
        group: 'Navigate',
        detail: groupLabel(m.group),
        keywords: `module ${m.id.replace(/[-/]/g, ' ')} ${m.keywords ?? ''}`,
        run: go(m.id),
      })),
      { id: 'core.go-settings', title: 'Open Settings', group: 'Navigate', keywords: 'preferences appearance', run: go('settings/appearance') },
    ];
    windowCommands = base;
    try {
      workspaces = await api.get<WorkspaceWithRole[]>('/workspaces');
    } catch {
      workspaces = [];
    }
    const wsId = spaceWorkspace;
    if (!wsId) return;
    const [sessions, repos] = await Promise.all([
      api.get<Session[]>(`/workspaces/${wsId}/sessions`).catch(() => [] as Session[]),
      api.get<Repo[]>(`/workspaces/${wsId}/repos`).catch(() => [] as Repo[]),
    ]);
    windowCommands = [
      ...base,
      ...sessions
        .filter((s) => !s.archived && isForeground(s) && visibleOnThisDevice(s))
        .map((s) => ({
          id: `session.${s.id}`,
          title: `Focus session: ${s.title}`,
          group: 'Sessions',
          keywords: s.provider,
          run: go(`agents/${s.id}`),
        })),
      ...repos.map((r) => ({
        id: `repo.${r.id}`,
        title: `Open repo: ${r.name}`,
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
    const n = rows.length;
    untrack(() => {
      if (selected >= n) selected = n - 1;
      else if (selected < 0 && n > 0) selected = 0;
    });
  });

  // ── panel view ──────────────────────────────────────────────────────────
  type View = 'space' | 'results' | 'thread' | 'recent' | null;
  const view: View = $derived.by(() => {
    if (editing) return 'space';
    if (query.trim() !== '') return 'results';
    if (thread.length > 0) return 'thread';
    return inApp && rows.length > 0 ? 'recent' : null;
  });
  const showPanel = $derived(view !== null && (inApp ? open : true));
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
    recordUsage(cmd.id);
    closeBar();
    try {
      await cmd.run();
    } catch (e) {
      addTurn('', { tone: 'error', text: `“${cmd.title}” failed.`, detail: e instanceof Error ? e.message : String(e), source: 'Otto' });
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
        a: 'Cancelled — nothing ran.',
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
      : await confirmClose(turn.closeIds ?? [], sp, askHost);
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
  // there, lift the stack above it (--toast-lift, read by Toasts.svelte).
  $effect(() => {
    if (!inApp) return;
    const lift = presence === 'full' ? PILL_H + 12 : presence === 'rest' ? 36 + 12 : 0;
    document.documentElement.style.setProperty('--toast-lift', `${lift}px`);
    return () => document.documentElement.style.removeProperty('--toast-lift');
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
    let scrollTimer: ReturnType<typeof setTimeout> | null = null;
    const onScroll = (e: Event): void => {
      const t = e.target as Node | null;
      if (t && rootEl?.contains(t)) return;
      scrolling = true;
      if (scrollTimer) clearTimeout(scrollTimer);
      scrollTimer = setTimeout(() => (scrolling = false), SCROLL_SETTLE_MS);
    };
    const onFocusChange = (): void => {
      workFocus = isWorkTarget(document.activeElement);
    };
    const onFocusOut = (): void => queueMicrotask(onFocusChange);
    document.addEventListener('scroll', onScroll, { capture: true, passive: true });
    document.addEventListener('focusin', onFocusChange);
    document.addEventListener('focusout', onFocusOut);
    return () => {
      if (scrollTimer) clearTimeout(scrollTimer);
      document.removeEventListener('scroll', onScroll, { capture: true });
      document.removeEventListener('focusin', onFocusChange);
      document.removeEventListener('focusout', onFocusOut);
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
</script>

{#if presence !== 'off'}
  <div
    class="fb"
    class:app={inApp}
    class:win={!inApp}
    data-presence={presence}
    data-testid="floating-bar"
    bind:this={rootEl}
    onfocusin={onFocusIn}
    onfocusout={onFocusOut}
    inert={presence === 'away' ? true : undefined}
  >
    {#if presence === 'dock'}
      <!-- Docked in the status bar: covers no content. -->
      <button
        class="dock"
        onclick={() => void openBar()}
        title="Ask Otto or run a command (⌘K)"
        aria-label="Open the Otto bar"
      >
        <Icon name="sparkle" size={12} />
        <span>Ask Otto</span>
        <kbd>⌘K</kbd>
        {#if unread}<span class="unread" aria-label="New reply"></span>{/if}
      </button>
    {/if}

    <div
      class="surface"
      class:expanded={showPanel}
      class:focused={inApp && focused}
      class:compact={presence === 'rest'}
      hidden={presence === 'dock'}
    >
      {#if showPanel}
        <div
          class="panel"
          style:max-height={inApp ? `${Math.max(120, budget - PILL_H - 8)}px` : undefined}
        >
          <div class="panel-inner" bind:this={panelInner}>
            {#if signedOut}
              <div class="note" role="status">
                <Icon name="lock" size={14} />
                <span>Sign in to Otto to use the bar.</span>
                <button class="btn small" onclick={() => void openInOtto()}>Open Otto</button>
              </div>
            {/if}

            {#if view === 'space'}
              <section class="editor" aria-labelledby="fb-space-title">
                <header class="ed-head">
                  <span class="sp-num">{spaceLabel(spaceIdx)}</span>
                  <h2 id="fb-space-title">Space settings</h2>
                  <span class="grow"></span>
                  <button class="btn small" onclick={() => (editing = false)}>Done</button>
                </header>
                <div class="ed-grid">
                  <label class="ed-field">
                    <span>Name</span>
                    <input
                      class="input"
                      value={space.name}
                      maxlength="24"
                      onchange={(e) => barStore.patchSpace(spaceIdx, { name: e.currentTarget.value.trim() || space.name })}
                    />
                  </label>
                  <label class="ed-field">
                    <span>Workspace</span>
                    <select
                      class="input"
                      value={space.workspaceId ?? ''}
                      onchange={(e) => barStore.patchSpace(spaceIdx, { workspaceId: e.currentTarget.value || null })}
                    >
                      <option value="">Follow Otto (selected workspace)</option>
                      {#each workspaceList as w (w.id)}
                        <option value={w.id}>{w.name}</option>
                      {/each}
                    </select>
                  </label>
                  <label class="ed-field">
                    <span>Agent</span>
                    <select
                      class="input"
                      value={space.provider}
                      onchange={(e) => barStore.patchSpace(spaceIdx, { provider: e.currentTarget.value, model: '' })}
                    >
                      <option value="">Default ({providerName(defaultAgentProvider())})</option>
                      {#each agentProviders() as p (p)}
                        <option value={p}>{providerName(p)}</option>
                      {/each}
                    </select>
                  </label>
                  <div class="ed-model">
                    <ModelPicker
                      id="fb-space-model"
                      {provider}
                      value={space.model}
                      onchange={(m) => barStore.patchSpace(spaceIdx, { model: m })}
                      hint="Kept with this space for its assistant thread; Ask Otto plans with the orchestrator today."
                    />
                  </div>
                </div>
                <footer class="ed-foot">
                  <span class="dim">Spaces live on this Mac. ⌃1–⌃4 switch.</span>
                  <span class="grow"></span>
                  <button
                    class="btn small"
                    disabled={thread.length === 0}
                    onclick={() => barStore.clearThread(spaceIdx)}
                  >Clear thread</button>
                </footer>
              </section>
            {:else if view === 'thread'}
              <div class="thread-head">
                <span class="sp-num">{spaceLabel(spaceIdx)}</span>
                <span class="sp-name">{space.name}</span>
                {#if spaceWorkspaceName}<span class="dim">· {spaceWorkspaceName}</span>{/if}
                <span class="grow"></span>
                <button class="link-btn" onclick={() => barStore.clearThread(spaceIdx)}>Clear</button>
              </div>
              <div class="thread" bind:this={threadEl} aria-live="polite">
                {#each thread as t (t.id)}
                  <article class="turn" data-tone={t.tone}>
                    {#if t.q}<p class="q">{t.q}</p>{/if}
                    {#if t.tone === 'pending' && t.a === ''}
                      <p class="a thinking"><span class="spin" aria-hidden="true"></span>Working on it…</p>
                    {:else}
                      <p class="a">
                        {#if t.tone === 'error' || t.tone === 'warn'}
                          <Icon name="warning" size={13} />
                        {:else if t.tone === 'ok'}
                          <Icon name="check" size={13} />
                        {/if}
                        <span>{t.a}</span>
                      </p>
                    {/if}
                    {#if t.detail}<p class="detail">{t.detail}</p>{/if}
                    {#if t.plan && t.plan.length > 0}
                      <ol class="plan">
                        {#each t.plan as a, i (i)}
                          <li>{describeAction(a as Action)}</li>
                        {/each}
                      </ol>
                    {/if}
                    {#if (t.plan && t.plan.length > 0) || t.closeIds}
                      <div class="confirm">
                        <button class="btn small" disabled={busy[t.id]} onclick={() => void resolveTurn(t, false)}>Cancel</button>
                        <button
                          class="btn small"
                          class:primary={!t.closeIds}
                          class:danger={!!t.closeIds}
                          disabled={busy[t.id]}
                          onclick={() => void resolveTurn(t, true)}
                        >{busy[t.id] ? 'Running…' : t.closeIds ? 'Delete' : 'Run plan'}</button>
                      </div>
                    {/if}
                    <footer class="meta">
                      <span class="src"><Icon name="sparkle" size={11} />{t.source ?? 'Ask Otto'}</span>
                      <span>{timeOf(t.at)}</span>
                      {#if t.route}
                        <span class="grow"></span>
                        <button class="open-btn" onclick={() => openRoute(t.route ?? '')}>
                          {inApp ? 'Open' : 'Open in Otto'} <Icon name={inApp ? 'chevronRight' : 'external'} size={11} />
                        </button>
                      {/if}
                    </footer>
                  </article>
                {/each}
              </div>
            {:else if view === 'results' || view === 'recent'}
              <ul class="list" id="fb-listbox" role="listbox" aria-label={view === 'recent' ? 'Recent commands' : 'Results'}>
                {#if view === 'recent'}
                  <li class="sec" role="presentation">Recent</li>
                {/if}
                {#each rows as row, i (row.key)}
                  {#if row.kind === 'hit' && (i === 0 || rows[i - 1].kind !== 'hit')}
                    <li class="sec" role="presentation">{searching ? 'Searching…' : 'In this workspace'}</li>
                  {/if}
                  <!-- Options are driven from the combobox input (aria-activedescendant);
                       mousedown keeps focus there. -->
                  <!-- svelte-ignore a11y_click_events_have_key_events -->
                  <li
                    id="fb-opt-{i}"
                    class="opt"
                    class:ask={row.kind === 'ask'}
                    class:sel={i === selected}
                    role="option"
                    aria-selected={i === selected}
                    onmousedown={(e) => e.preventDefault()}
                    onmousemove={() => (selected = i)}
                    onclick={() => runRow(row)}
                  >
                    <span class="opt-ic"><Icon name={rowIcon(row)} size={13} /></span>
                    {#if row.kind === 'ask'}
                      <span class="opt-title">Ask Otto <span class="q-inline">“{row.text}”</span></span>
                      <span class="grow"></span>
                      <span class="opt-meta">{space.name}</span>
                      {#if i === selected}<kbd>↵</kbd>{:else}<kbd>⌘↵</kbd>{/if}
                    {:else if row.kind === 'cmd'}
                      <span class="opt-title">{row.cmd.title}</span>
                      {#if row.cmd.detail}<span class="opt-detail">{row.cmd.detail}</span>{/if}
                      <span class="grow"></span>
                      {#if row.cmd.group}<span class="opt-meta">{row.cmd.group}</span>{/if}
                      {#if row.cmd.shortcut}<kbd>{row.cmd.shortcut}</kbd>{/if}
                    {:else}
                      <span class="opt-title">{row.hit.title}</span>
                      {#if row.hit.subtitle}<span class="opt-detail">{row.hit.subtitle}</span>{/if}
                      <span class="grow"></span>
                      <span class="opt-meta">{row.hit.kind.replace('_', ' ')}</span>
                    {/if}
                  </li>
                {/each}
              </ul>
              <div class="keys" aria-hidden="true">
                <span><kbd>↑↓</kbd> select</span>
                <span><kbd>↵</kbd> run</span>
                <span><kbd>⌘↵</kbd> ask Otto</span>
                <span><kbd>⌃1–4</kbd> space</span>
                <span><kbd>esc</kbd> {inApp ? 'close' : 'hide'}</span>
              </div>
            {/if}
          </div>
        </div>
      {/if}

      <div class="pill" bind:this={pillEl}>
        <button
          class="spark"
          tabindex="-1"
          aria-label="Ask Otto"
          title="Ask Otto"
          onclick={() => void openBar()}
        >
          <Icon name="sparkle" size={inApp ? 15 : 17} />
          {#if unread}<span class="unread" aria-label="New reply"></span>{/if}
        </button>
        <input
          bind:this={inputEl}
          bind:value={query}
          class="input-main"
          type="text"
          role="combobox"
          aria-label="Ask Otto or search commands"
          aria-expanded={listOpen}
          aria-controls={listOpen ? 'fb-listbox' : undefined}
          aria-activedescendant={activeId}
          aria-autocomplete="list"
          placeholder="Type or speak…"
          spellcheck="false"
          autocomplete="off"
          onkeydown={onKey}
          onfocus={() => (opening = false)}
        />
        {#if presence === 'full'}
          <kbd class="k-hint" title={inApp ? '⌘K focuses this bar from anywhere in Otto' : 'Commands and Ask Otto'}>⌘K</kbd>
          <span class="sep" aria-hidden="true"></span>
          <button
            class="fb-chip model"
            onclick={() => (editing = !editing)}
            aria-expanded={editing}
            title="{space.name}: {providerName(provider)} · {modelLabel} — change the space’s agent, model and workspace"
          >
            <ProviderIcon {provider} size={13} />
            <span class="fb-chip-label">{modelLabel}</span>
            <Icon name="chevronDown" size={10} />
          </button>
          <span class="sep" aria-hidden="true"></span>
          <div class="spaces" role="radiogroup" aria-label="Spaces (⌃1–⌃4)">
            {#each Array.from({ length: SPACE_COUNT }, (_, i) => i) as i (i)}
              {@const sp = barStore.state.spaces[i]}
              <button
                class="space"
                role="radio"
                aria-checked={i === spaceIdx}
                title="{spaceLabel(i)} · {sp.name}{i === spaceIdx ? ' — click again for settings' : ''} (⌃{i + 1})"
                aria-label="Space {i + 1}: {sp.name}"
                onclick={() => selectSpace(i)}
              >{spaceLabel(i)}</button>
            {/each}
          </div>
          <button
            class="mic"
            aria-disabled="true"
            aria-label="Voice input (not available yet)"
            title="Voice input arrives with the assistant update — type for now"
            onclick={(e) => e.preventDefault()}
          >
            <Icon name="mic" size={15} />
          </button>
        {:else}
          <kbd class="k-hint">⌘K</kbd>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  /* ── placement ───────────────────────────────────────────────────────── */
  .fb.app {
    position: absolute;
    inset-inline: 0;
    bottom: 16px;
    z-index: var(--z-floating-bar);
    display: flex;
    justify-content: center;
    pointer-events: none;
    padding-inline: 16px;
  }
  .fb.app > * {
    pointer-events: auto;
  }
  .fb.app[data-presence='away'] {
    visibility: hidden;
  }
  /* Docked: the chip sits INSIDE the 24px status bar below the content. */
  .fb.app[data-presence='dock'] {
    bottom: -22px;
  }
  .fb.win {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
  }

  /* ── the one glass surface (pill + thread) ─────────────────────────── */
  .surface {
    display: flex;
    flex-direction: column;
    min-width: 0;
    color: var(--text);
    background: color-mix(in srgb, var(--bg-sidebar) 78%, transparent);
    border-radius: 999px;
    transition:
      width 180ms ease,
      border-radius 180ms ease,
      box-shadow 140ms ease,
      border-color 140ms ease;
  }
  .surface[hidden] {
    display: none;
  }
  .app .surface {
    width: min(720px, 100%);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
    backdrop-filter: blur(24px) saturate(1.6);
    -webkit-backdrop-filter: blur(24px) saturate(1.6);
  }
  .app .surface.compact {
    width: min(260px, 100%);
  }
  .app .surface.compact:hover {
    border-color: var(--border-strong);
  }
  .app .surface.focused {
    border-color: color-mix(in srgb, var(--accent) 55%, var(--border));
    box-shadow:
      var(--shadow),
      0 0 0 3px color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .surface.expanded {
    border-radius: calc(var(--radius-l) + 6px);
  }
  /* The ⌥Space window: the native HUD glass is the material and the window
     is shaped natively, so the page only adds the token tint. */
  .win .surface {
    height: 100%;
    border-radius: 0;
    justify-content: flex-end;
  }
  /* No blur engine (older WebKit, some test browsers): opaque surface. */
  @supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
    .app .surface {
      background: var(--surface);
    }
  }
  @media (prefers-reduced-transparency: reduce) {
    .app .surface {
      background: var(--bg-sidebar);
      backdrop-filter: none;
      -webkit-backdrop-filter: none;
    }
    .win .surface {
      background: var(--bg-sidebar);
    }
  }

  /* ── pill ─────────────────────────────────────────────────────────────── */
  .pill {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 46px;
    flex-shrink: 0;
    padding-inline: 8px 7px;
    min-width: 0;
  }
  .compact .pill {
    height: 36px;
    padding-inline: 6px 8px;
  }
  .win .pill {
    height: 56px;
    padding-inline: 12px 12px;
  }
  .expanded .pill {
    border-block-start: 1px solid var(--border);
  }
  .spark {
    position: relative;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    flex-shrink: 0;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: var(--accent-text);
    cursor: pointer;
  }
  .compact .spark {
    width: 24px;
    height: 24px;
  }
  .unread {
    position: absolute;
    inset-block-start: 3px;
    inset-inline-end: 3px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 0 2px var(--bg-sidebar);
  }
  .input-main {
    flex: 1;
    min-width: 60px;
    height: 100%;
    border: none;
    background: transparent;
    outline: none;
    font: inherit;
    font-size: var(--fs-l);
    color: var(--text);
    padding: 0 2px;
  }
  .compact .input-main {
    font-size: var(--fs-m);
    cursor: pointer;
  }
  .win .input-main {
    font-size: var(--fs-xl);
  }
  .input-main::placeholder {
    color: var(--text-dim);
  }
  kbd {
    font-family: var(--font-ui);
    font-size: var(--fs-xs);
    line-height: 1;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 3px 5px;
    flex-shrink: 0;
  }
  .sep {
    width: 1px;
    height: 20px;
    background: var(--border);
    flex-shrink: 0;
  }
  .fb-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    max-width: 150px;
    padding-inline: 8px 7px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: color-mix(in srgb, var(--surface) 70%, transparent);
    color: var(--text);
    font-size: var(--fs-s);
    cursor: pointer;
    flex-shrink: 0;
  }
  .fb-chip:hover,
  .fb-chip[aria-expanded='true'] {
    background: var(--hover);
    border-color: var(--border-strong);
  }
  .fb-chip-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .spaces {
    display: flex;
    gap: 1px;
    flex-shrink: 0;
  }
  .space {
    height: 28px;
    min-width: 30px;
    padding: 0 6px;
    border: none;
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .space:hover {
    background: var(--hover);
    color: var(--text);
  }
  .space[aria-checked='true'] {
    background: var(--accent-soft);
    color: var(--accent-text);
    font-weight: 600;
  }
  .mic {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    border: 1px solid var(--border);
    border-radius: 50%;
    background: transparent;
    color: var(--text-dim);
    cursor: not-allowed;
    opacity: 0.7;
  }

  /* ── dock chip (in the status bar) ────────────────────────────────────── */
  .dock {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 20px;
    padding-inline: 8px 4px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .dock :global(svg) {
    color: var(--accent-text);
  }
  .dock:hover {
    color: var(--text);
    border-color: var(--border-strong);
  }
  .dock kbd {
    padding: 2px 4px;
  }
  .dock .unread {
    inset-block-start: -2px;
    inset-inline-end: -2px;
  }

  /* ── panel (above the pill, same surface) ─────────────────────────────── */
  .panel {
    overflow-y: auto;
    overscroll-behavior: contain;
    min-height: 0;
  }
  .win .panel {
    flex: 1;
  }
  .panel-inner {
    padding: 8px 8px 6px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .note {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: var(--warning-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .note span {
    flex: 1;
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .sec {
    padding: 6px 10px 3px;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .opt {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 32px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    font-size: var(--fs-m);
    cursor: pointer;
    min-width: 0;
  }
  .opt.sel {
    background: var(--accent-soft);
  }
  .opt-ic {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    background: var(--hover);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .opt.ask .opt-ic {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .opt-title {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .q-inline {
    color: var(--accent-text);
  }
  .opt-detail {
    font-size: var(--fs-s);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex-shrink: 1;
  }
  .opt-meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .keys {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    padding: 6px 10px 2px;
    border-block-start: 1px solid var(--border);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .keys kbd {
    padding: 1px 4px;
    margin-inline-end: 2px;
  }

  /* ── thread ───────────────────────────────────────────────────────────── */
  .thread-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 6px 0;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .sp-num {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--accent-text);
    background: var(--accent-soft);
    border-radius: 999px;
    padding: 2px 6px;
  }
  .sp-name {
    font-weight: 600;
  }
  .link-btn {
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-s);
    cursor: pointer;
    padding: 2px 6px;
    border-radius: var(--radius-s);
  }
  .link-btn:hover {
    color: var(--text);
    background: var(--hover);
  }
  .thread {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .turn {
    padding: 10px 12px;
    border-radius: var(--radius-l);
    background: color-mix(in srgb, var(--surface) 72%, transparent);
    border: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: var(--fs-m);
    line-height: 1.45;
  }
  .q {
    margin: 0;
    font-weight: 600;
  }
  .a {
    margin: 0;
    display: flex;
    gap: 6px;
    align-items: baseline;
  }
  .a :global(svg) {
    align-self: center;
  }
  .turn[data-tone='ok'] .a :global(svg) {
    color: var(--success);
  }
  .turn[data-tone='warn'] .a :global(svg) {
    color: var(--warning);
  }
  .turn[data-tone='error'] .a {
    color: var(--danger);
  }
  .thinking {
    color: var(--text-dim);
    align-items: center;
  }
  .spin {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid var(--border-strong);
    border-block-start-color: var(--accent);
    animation: fb-spin 800ms linear infinite;
  }
  .detail {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .plan {
    margin: 0;
    padding-inline-start: 20px;
    font-size: var(--fs-s);
  }
  .confirm {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .src {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .src :global(svg) {
    color: var(--accent-text);
  }
  .open-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .open-btn:hover {
    background: var(--hover);
  }

  /* ── space editor ─────────────────────────────────────────────────────── */
  .editor {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 4px 6px 2px;
  }
  .ed-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ed-head h2 {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .ed-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px 12px;
    align-items: start;
  }
  .ed-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .ed-field .input {
    width: 100%;
    min-width: 0;
  }
  @media (max-width: 560px) {
    .ed-grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .ed-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-xs);
  }
  .dim {
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }

  @keyframes fb-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .surface {
      transition: none;
    }
    .spin {
      animation: none;
    }
  }
</style>
