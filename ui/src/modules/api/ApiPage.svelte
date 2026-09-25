<script lang="ts">
  // The API module page (workbench archetype).
  //
  //   PageHeader: API · [Environment: Staging ▾] [Import…] [Sync with Git…] [New request]
  //   ┌ left pane ─────────────────┬ main ─────────────────────────────────────┐
  //   │ Collections|Automations|History │ tabs: requests + Environments/automation │
  //   │ (things you pick)          │ request editor ↕ response                 │
  //   └────────────────────────────┴───────────────────────────────────────────┘
  //
  // The left pane only lists things; whatever you EDIT (a request, the
  // environments, an automation) opens in the main area. A brand-new
  // workspace shows an onboarding empty state instead of an empty editor.
  import { untrack } from 'svelte';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import RequestBuilder from './RequestBuilder.svelte';
  import ResponseViewer from './ResponseViewer.svelte';
  import CollectionsTree from './CollectionsTree.svelte';
  import HistoryList from './HistoryList.svelte';
  import AutomationsView from './AutomationsView.svelte';
  import AutomationEditor from './AutomationEditor.svelte';
  import EnvironmentsView from './EnvironmentsView.svelte';
  import ImportDialog from './ImportDialog.svelte';
  import GitSyncDialog from './GitSyncDialog.svelte';
  import MethodTag from './MethodTag.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { keyContext } from '../../lib/keys';
  import type { Id } from '../../lib/api/types';

  type Side = 'collections' | 'automations' | 'history';
  const SIDE_KEY = 'otto_api_side';
  const SIDES: { id: Side; label: string; icon: IconName }[] = [
    { id: 'collections', label: 'Collections', icon: 'folder' },
    { id: 'automations', label: 'Automations', icon: 'zap' },
    { id: 'history', label: 'History', icon: 'clock' },
  ];
  function readSide(): Side {
    try {
      const v = localStorage.getItem(SIDE_KEY);
      return v === 'automations' || v === 'history' ? v : 'collections';
    } catch {
      return 'collections';
    }
  }
  let side = $state<Side>(readSide());
  function setSide(s: Side): void {
    side = s;
    try { localStorage.setItem(SIDE_KEY, s); } catch { /* per-device convenience only */ }
  }
  function onSideKey(e: KeyboardEvent, i: number): void {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
    e.preventDefault();
    const j = (i + (e.key === 'ArrowRight' ? 1 : SIDES.length - 1)) % SIDES.length;
    setSide(SIDES[j].id);
    (e.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLElement>('[role=tab]')[j]?.focus();
  }

  // What the main area shows.
  type View = { kind: 'request' } | { kind: 'environments'; envId: Id | null } | { kind: 'automation'; id: Id };
  let view = $state<View>({ kind: 'request' });
  let builderTab = $state<'params' | 'headers' | 'body' | 'auth' | 'settings' | 'scripts' | 'docs'>('params');
  // Phone: push navigation between the list and the editor.
  let phonePane = $state<'list' | 'main'>('main');

  function showRequest(): void {
    view = { kind: 'request' };
    phonePane = 'main';
  }
  /** New request: reuse the active tab while it is an untouched blank
   *  draft (no pile of empty tabs), else open a new one; focus the URL. */
  function newRequest(): void {
    started = true;
    const d = apiClient.draft;
    if (d.requestId || apiClient.isDirty(d)) apiClient.newDraft();
    showRequest();
    requestAnimationFrame(() => document.querySelector<HTMLInputElement>('input[aria-label="Request URL"]')?.focus());
  }
  /** Close a request tab. A SAVED request with unsaved edits asks first
   *  (patterns §7 "Discard"); a scratch draft still closes silently — its
   *  sends are in History. */
  async function closeRequestTab(i: number): Promise<void> {
    const t = apiClient.tabs[i];
    if (t?.requestId && apiClient.isDirty(t)) {
      const ok = await confirmer.ask(`“${apiClient.tabLabel(t)}” has unsaved changes. Close the tab and discard them? The saved request is kept.`, {
        title: 'Discard changes?',
        confirmLabel: 'Discard changes',
        danger: true,
      });
      if (!ok) return;
    }
    // The tab list may have shifted while the dialog was open.
    const at = t?.tabId ? apiClient.tabs.findIndex((x) => x.tabId === t.tabId) : i;
    if (at >= 0) apiClient.closeTab(at);
  }
  function openEnvironments(envId: Id | null = null): void {
    view = { kind: 'environments', envId };
    phonePane = 'main';
  }
  function openAutomation(id: Id): void {
    view = { kind: 'automation', id };
    phonePane = 'main';
  }

  // Load everything when the workspace changes. Keyed on the workspace ONLY:
  // loadAll's synchronous prologue reads (and rewrites) the open tabs, which
  // must not become dependencies of this effect.
  $effect(() => {
    if (ws.currentId) {
      untrack(() => {
        view = { kind: 'request' };
        void apiClient.loadAll();
        void apiClient.loadAutomations();
      });
    }
  });

  // ── onboarding ─────────────────────────────────────────────────────────────
  // A workspace with nothing in it (no saved requests, collections, history or
  // edited tab) opens on "Create your first request", not an empty editor.
  let started = $state(false);
  const onboarding = $derived(
    !started &&
      !apiClient.loading &&
      !apiClient.loadError &&
      apiClient.collections.length === 0 &&
      apiClient.requests.length === 0 &&
      apiClient.history.length === 0 &&
      apiClient.tabs.length === 1 &&
      !apiClient.isDirty(apiClient.tabs[0]),
  );
  function tryExample(): void {
    started = true;
    apiClient.newDraft();
    apiClient.draft = { ...apiClient.draft, name: 'Example: get JSON', method: 'GET', url: 'https://httpbin.org/json' };
    showRequest();
  }

  // ── environment switcher (header) ─────────────────────────────────────────
  async function newEnvironment(): Promise<void> {
    const name = await confirmer.promptText('Name', { title: 'New environment', confirmLabel: 'Create', initial: '' });
    if (!name) return;
    const saved = await apiClient.saveEnvironment({ name }, undefined);
    if (saved) openEnvironments(saved.id);
  }
  function envMenu(e: MouseEvent): void {
    const canEdit = ws.myRole !== 'viewer';
    const items: MenuItem[] = apiClient.environments.map((env) => ({
      label: env.name,
      icon: env.is_active ? 'check' : 'dot',
      disabled: !canEdit,
      action: () => void apiClient.activateEnvironment(env.id),
    }));
    if (items.length) items.push({ separator: true });
    items.push({ label: 'Manage environments…', icon: 'globe', action: () => openEnvironments() });
    if (canEdit) items.push({ label: 'New environment…', icon: 'plus', action: () => void newEnvironment() });
    ctxMenu.show(e, items);
  }

  let importOpen = $state(false);
  let gitOpen = $state(false);

  // ── drag-to-resize: sidebar width + builder height (both persisted) ────────
  function startSideResize(e: MouseEvent): void {
    e.preventDefault();
    const startX = e.clientX;
    const startW = ui.apiSideWidth;
    const rtl = document.documentElement.dir === 'rtl';
    const onMove = (ev: MouseEvent) => ui.setApiSideWidth(startW + (rtl ? startX - ev.clientX : ev.clientX - startX));
    const onUp = () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
  }
  let builderEl: HTMLDivElement | null = $state(null);
  function startBuilderResize(e: MouseEvent): void {
    e.preventDefault();
    const startY = e.clientY;
    // Until the first drag the height is CSS-driven — seed from the rendered
    // height so the divider doesn't jump on grab.
    const startH = ui.apiBuilderHeight || builderEl?.offsetHeight || 300;
    const onMove = (ev: MouseEvent) => ui.setApiBuilderHeight(startH + (ev.clientY - startY));
    const onUp = () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'row-resize';
    document.body.style.userSelect = 'none';
  }
  function resizeKey(e: KeyboardEvent, axis: 'x' | 'y'): void {
    const step = e.shiftKey ? 40 : 10;
    if (axis === 'x' && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
      e.preventDefault();
      ui.setApiSideWidth(ui.apiSideWidth + (e.key === 'ArrowRight' ? step : -step));
    } else if (axis === 'y' && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      e.preventDefault();
      const h = ui.apiBuilderHeight || builderEl?.offsetHeight || 300;
      ui.setApiBuilderHeight(h + (e.key === 'ArrowDown' ? step : -step));
    }
  }

  // ── keyboard + ⌘K ─────────────────────────────────────────────────────────
  // ⌘T → new request tab and ⌘D → duplicate, instead of the global
  // new-session / split chords (which would act on the Agents page unseen).
  $effect(() => {
    keyContext.pageChords = (e) => {
      if (!(e.metaKey || e.ctrlKey) || e.shiftKey || ui.overlayOpen) return false;
      const k = e.key.toLowerCase();
      if (k === 't') { e.preventDefault(); newRequest(); return true; }
      if (k === 'd') { e.preventDefault(); apiClient.duplicateDraft(); showRequest(); return true; }
      return false;
    };
    return () => { keyContext.pageChords = null; };
  });
  $effect(() => {
    return registry.register('api', [
      { id: 'api.new', title: 'New API request', group: 'API', shortcut: '⌘T', keywords: 'http rest call postman', run: newRequest },
      { id: 'api.duplicate', title: 'Duplicate API request', group: 'API', shortcut: '⌘D', keywords: 'copy clone', run: () => { apiClient.duplicateDraft(); showRequest(); } },
      { id: 'api.import', title: 'Import into API…', group: 'API', keywords: 'curl postman openapi swagger har', run: () => (importOpen = true) },
      { id: 'api.envs', title: 'Manage API environments', group: 'API', keywords: 'variables base_url staging production secrets', run: () => openEnvironments() },
      { id: 'api.history', title: 'Show API history', group: 'API', keywords: 'recent sent requests', run: () => { setSide('history'); phonePane = 'list'; } },
    ]);
  });

  const activeEnvName = $derived(apiClient.activeEnv?.name ?? null);
  const automationName = $derived(view.kind === 'automation' ? (apiClient.automations.find((a) => a.id === (view as { id: Id }).id)?.name ?? 'Automation') : '');
  const showList = $derived(!viewport.isPhone || phonePane === 'list');
  const showMain = $derived(!viewport.isPhone || phonePane === 'main');
</script>

<div class="api-root">
  <PageHeader title="API" subtitle="Requests run from Otto’s daemon, not the browser">
    {#snippet leading()}
      {#if viewport.isPhone && phonePane === 'main' && !onboarding}
        <button class="icon-btn" onclick={() => (phonePane = 'list')} aria-label="Show saved requests" title="Show saved requests"><Icon name="chevronLeft" size={16} /></button>
      {/if}
    {/snippet}
    {#snippet actions()}
      <button class="btn small env-btn" data-keep onclick={envMenu} aria-haspopup="menu"
        aria-label="Environment: {activeEnvName ?? 'none'}" title="Environment: where {'{{variables}}'} get their values">
        <Icon name="globe" size={12} />
        {#if !viewport.isPhone}<span class="env-k">Environment</span>{/if}
        <span class="env-v" class:none={!activeEnvName}>{activeEnvName ?? 'None'}</span>
        <Icon name="chevronDown" size={12} />
      </button>
      <button class="btn small" data-icon="download" data-overflow="0" onclick={() => (importOpen = true)}><Icon name="download" size={12} />Import…</button>
      <button class="btn small" data-icon="branch" data-overflow="-1" onclick={() => (gitOpen = true)} disabled={ws.myRole === 'viewer'} title={ws.myRole === 'viewer' ? 'Viewers can’t sync API collections with Git' : undefined}><Icon name="branch" size={12} />Sync with Git…</button>
      {#if !onboarding}
        <button class="btn small primary" onclick={newRequest} title="New request (⌘T)" aria-label="New request"><Icon name="plus" size={12} />{#if !viewport.isPhone}New request{/if}</button>
      {/if}
    {/snippet}
  </PageHeader>

  <PageBody fill padded={false}>
    {#if onboarding}
      <div class="onboard">
        <EmptyState
          variant="page"
          icon="send"
          title="Create your first request"
          body={'Call any HTTP API and see the status, headers and body. Save requests into collections, keep base URLs and tokens in environments as {{variables}}, and chain requests into automated checks.'}
          actionLabel="New request"
          actionIcon="plus"
          onaction={newRequest}
        >
          <div class="onboard-more">
            <button class="btn ghost" onclick={() => (importOpen = true)}><Icon name="download" size={13} />Import from curl, Postman or OpenAPI…</button>
            <button class="btn ghost" onclick={tryExample}><Icon name="play" size={13} />Try an example request</button>
          </div>
          <p class="onboard-note">Requests go out from Otto’s daemon, so browser CORS rules don’t apply. Localhost and private networks are blocked unless a workspace admin allows them.</p>
        </EmptyState>
      </div>
    {:else}
      <div class="api-page">
        {#if showList}
          <aside class="api-side" style:width={viewport.isPhone ? null : `${ui.apiSideWidth}px`} aria-label="Collections, automations and history">
            <div class="segmented side-seg" role="tablist" aria-label="Show">
              {#each SIDES as s, i (s.id)}
                <button role="tab" aria-selected={side === s.id} class:active={side === s.id} tabindex={side === s.id ? 0 : -1}
                  onclick={() => setSide(s.id)} onkeydown={(e) => onSideKey(e, i)} title={s.label}>
                  <span class="seg-label">{s.label}</span>
                </button>
              {/each}
            </div>
            <div class="side-body">
              {#if side === 'collections'}
                <CollectionsTree onopen={showRequest} />
              {:else if side === 'automations'}
                <AutomationsView selectedId={view.kind === 'automation' ? view.id : null} onselect={openAutomation} />
              {:else}
                <HistoryList onopen={showRequest} />
              {/if}
            </div>
          </aside>

          {#if !viewport.isPhone}
            <button
              class="resizer col"
              aria-label="Resize the sidebar"
              title="Drag or use ←/→ to resize · double-click to reset"
              onmousedown={startSideResize}
              onkeydown={(e) => resizeKey(e, 'x')}
              ondblclick={() => ui.setApiSideWidth(280)}
            ></button>
          {/if}
        {/if}

        {#if showMain}
          <div class="api-main">
            <div class="req-tabs">
              <div class="req-tablist">
                {#each apiClient.tabs as t, i (t.tabId ?? i)}
                  {@const active = view.kind === 'request' && apiClient.activeTab === i}
                  <div class="req-tab" class:active>
                    <button class="req-tab-main" aria-current={active ? 'true' : undefined} onclick={() => { apiClient.switchTab(i); showRequest(); }}
                      title={apiClient.isDirty(t) ? `${apiClient.tabLabel(t)} (unsaved changes)` : apiClient.tabLabel(t)}>
                      <MethodTag method={t.kind === 'http' || t.kind === 'sse' ? t.method : t.kind === 'grpc' ? 'gRPC' : 'WS'} />
                      <span class="req-tab-label">{apiClient.tabLabel(t)}</span>
                      {#if apiClient.isDirty(t)}<span class="req-tab-dirty" aria-label="Unsaved changes"></span>{/if}
                    </button>
                    <button class="req-tab-close icon-btn" title="Close tab" aria-label="Close tab" onclick={() => void closeRequestTab(i)}><Icon name="x" size={12} /></button>
                  </div>
                {/each}
                {#if view.kind !== 'request'}
                  <div class="req-tab active special">
                    <button class="req-tab-main" aria-current="true">
                      <Icon name={view.kind === 'environments' ? 'globe' : 'zap'} size={12} />
                      <span class="req-tab-label">{view.kind === 'environments' ? 'Environments' : automationName}</span>
                    </button>
                    <button class="req-tab-close icon-btn" title="Close" aria-label="Close {view.kind === 'environments' ? 'environments' : 'automation'}" onclick={showRequest}><Icon name="x" size={12} /></button>
                  </div>
                {/if}
              </div>
              <button class="req-tab-new icon-btn" title="New request (⌘T)" aria-label="New request tab" onclick={newRequest}><Icon name="plus" size={14} /></button>
            </div>

            {#if view.kind === 'environments'}
              <EnvironmentsView initialId={view.envId} />
            {:else if view.kind === 'automation'}
              {#key view.id}
                <AutomationEditor automationId={view.id} ondeleted={showRequest} />
              {/key}
            {:else}
              <div
                class="builder-pane"
                bind:this={builderEl}
                style:height={!viewport.isPhone && ui.apiBuilderHeight > 0 ? `${ui.apiBuilderHeight}px` : null}
                style:max-height={!viewport.isPhone && ui.apiBuilderHeight > 0 ? 'none' : null}
              >
                <RequestBuilder bind:tab={builderTab} />
              </div>
              {#if !viewport.isPhone}
                <button
                  class="resizer row"
                  aria-label="Resize the request and response panes"
                  title="Drag or use ↑/↓ to resize · double-click to reset"
                  onmousedown={startBuilderResize}
                  onkeydown={(e) => resizeKey(e, 'y')}
                  ondblclick={() => ui.resetApiBuilderHeight()}
                ></button>
              {/if}
              <section class="resp-pane" aria-label="Response">
                <ResponseViewer onsettings={() => (builderTab = 'settings')} />
              </section>
            {/if}
          </div>
        {/if}
      </div>
    {/if}
  </PageBody>
</div>

{#if importOpen}
  <ImportDialog initial="curl" onclose={() => (importOpen = false)} onimported={() => { started = true; showRequest(); }} />
{/if}
{#if gitOpen}
  <GitSyncDialog onclose={() => (gitOpen = false)} />
{/if}

<style>
  .api-root {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .env-btn {
    gap: 6px;
    max-width: 260px;
  }
  .env-k {
    color: var(--text-dim);
  }
  .env-v {
    /* A flex item won't shrink below its text without this — the ellipsis never shows. */
    min-width: 0;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .env-v.none {
    font-weight: 500;
    color: var(--text-dim);
  }
  .onboard {
    flex: 1;
    overflow-y: auto;
  }
  .onboard :global(.empty.page p) {
    max-width: 520px;
  }
  .onboard-more {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
    margin-top: 4px;
  }
  .onboard-note {
    max-width: 460px;
    margin: 12px 0 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .api-page {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .api-side {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-height: 0;
    padding: 12px 10px 10px;
    border-inline-end: 1px solid var(--border);
    background: var(--bg);
    container-type: inline-size;
  }
  /* Three segments sized to their labels, never wrapping; below ~230 px the
     labels shrink a step (each keeps its tooltip and accessible name). */
  .side-seg {
    display: flex;
    width: 100%;
    flex-shrink: 0;
  }
  .side-seg > button {
    flex: 1 1 auto;
    min-width: 0;
    padding: 0 6px;
    white-space: nowrap;
  }
  .seg-label {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  @container (max-width: 230px) {
    .side-seg > button {
      font-size: var(--fs-xs);
      padding: 0 4px;
    }
  }
  .side-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    display: flex;
    flex-direction: column;
  }
  .api-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .req-tabs {
    display: flex;
    align-items: stretch;
    gap: 2px;
    padding: 6px 8px 0;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    min-width: 0;
  }
  .req-tablist {
    display: flex;
    gap: 2px;
    overflow-x: auto;
    scrollbar-width: thin;
    min-width: 0;
  }
  .req-tab {
    display: flex;
    align-items: center;
    max-width: 220px;
    flex-shrink: 0;
    padding-inline-end: 2px;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: var(--radius-s) var(--radius-s) 0 0;
    color: var(--text-dim);
  }
  .req-tab:hover {
    background: var(--hover);
  }
  .req-tab.active {
    background: var(--surface);
    border-color: var(--border);
    color: var(--text);
    margin-bottom: -1px;
    padding-bottom: 1px;
  }
  .req-tab-main {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    height: 30px;
    padding: 0 6px 0 10px;
    border: none;
    background: transparent;
    color: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .req-tab-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .req-tab-dirty {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--warning);
    flex-shrink: 0;
  }
  .req-tab-close {
    width: 20px;
    height: 20px;
  }
  .req-tab-new {
    align-self: center;
    flex-shrink: 0;
  }
  .builder-pane {
    padding: 14px 16px 12px;
    overflow-y: auto;
    flex: 0 0 auto;
    max-height: 60%;
    background: var(--surface);
  }
  .resp-pane {
    flex: 1;
    min-height: 0;
    padding: 10px 16px 14px;
    display: flex;
    flex-direction: column;
  }
  .resizer {
    padding: 0;
    border: none;
    background: transparent;
    flex-shrink: 0;
  }
  .resizer.col {
    width: 6px;
    margin-inline-start: -3px;
    margin-inline-end: -3px;
    cursor: col-resize;
    position: relative;
    z-index: 2;
  }
  .resizer.row {
    height: 6px;
    margin-top: -3px;
    margin-bottom: -3px;
    cursor: row-resize;
    border-top: 1px solid var(--border);
    background-clip: content-box;
    position: relative;
    z-index: 2;
  }
  .resizer:hover,
  .resizer:focus-visible {
    background: color-mix(in srgb, var(--accent) 30%, transparent);
  }

  @media (max-width: 640px) {
    .api-page {
      flex-direction: column;
    }
    .api-side {
      width: 100%;
      flex: 1;
      border-inline-end: none;
      padding: 10px 12px;
    }
    .builder-pane {
      max-height: none;
      padding: 12px;
      border-bottom: 1px solid var(--border);
    }
    .api-main {
      overflow-y: auto;
    }
    .resp-pane {
      flex: 0 0 auto;
      min-height: 320px;
      padding: 10px 12px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .resizer {
      transition: none;
    }
  }
</style>
