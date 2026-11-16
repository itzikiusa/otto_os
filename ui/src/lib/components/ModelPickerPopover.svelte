<script lang="ts">
  // One model panel (T3 Code's composer model chip): a thin provider rail
  // (★ favorites, then each harness), the provider's models with search,
  // ⌘1–⌘4 and a ☆ per row, a Reasoning column, and the access mode in the
  // footer. Anchored to its chip, preferring above it, and clamped into the
  // viewport with a capped, scrollable height (AGENTS.md floating-UI rule).
  import { onMount, tick, untrack } from 'svelte';
  import { placeAnchored } from '../anchorPlace';
  import { confirmer } from '../confirm.svelte';
  import {
    canonicalModelId,
    effortsForModel,
    formatEffortLabel,
    INTERACTION_MODES,
    modelsForProvider,
    normalizeEffort,
    supportsEffortSelection,
    supportsInteractionMode,
    supportsModelSelection,
    type EffortLevel,
    type InteractionMode,
  } from '../providerModels';
  import { harnessLabel, providerReadiness } from '../providers';
  import { auth } from '../stores/auth.svelte';
  import Icon from './Icon.svelte';
  import ProviderIcon from './ProviderIcon.svelte';

  interface Props {
    anchor: HTMLElement;
    providers: readonly string[];
    provider: string;
    /** A live thread can't change harness: other providers are shown disabled. */
    lockProvider?: boolean;
    model: string | null;
    effectiveModel?: string | null;
    effort: EffortLevel | null;
    mode: InteractionMode;
    onprovider?: (p: string) => void;
    onmodel: (m: string | null) => void;
    /** Omitted: the target takes no effort / access mode (no control shown). */
    oneffort?: (e: EffortLevel | null) => void;
    /** The levels the target's agent reports it takes (a runtime thread's
     *  `capabilities.efforts`); given, they bound the list. */
    effortLevels?: readonly string[];
    onmode?: (m: InteractionMode) => void;
    onclose: () => void;
  }
  let {
    anchor,
    providers,
    provider,
    lockProvider = false,
    model,
    effectiveModel = null,
    effort,
    mode,
    onprovider,
    onmodel,
    oneffort,
    effortLevels,
    onmode,
    onclose,
  }: Props = $props();

  const FAV_KEY = 'otto_model_favorites';
  const PAGE = 12;

  function readFavs(): string[] {
    try {
      const v: unknown = JSON.parse(localStorage.getItem(FAV_KEY) ?? '[]');
      return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
    } catch {
      return [];
    }
  }
  let favs = $state<string[]>(readFavs());
  function toggleFav(key: string): void {
    favs = favs.includes(key) ? favs.filter((k) => k !== key) : [...favs, key];
    try {
      localStorage.setItem(FAV_KEY, JSON.stringify(favs));
    } catch {
      /* private mode: favorites last for this panel only */
    }
  }

  /** The rail's selection: a provider, or the favorites list. */
  let view = $state<string>(untrack(() => provider));
  let query = $state('');
  let showAll = $state(false);
  let activeIdx = $state(0);
  let panelEl = $state<HTMLDivElement | null>(null);
  let searchEl = $state<HTMLInputElement | null>(null);
  let pos = $state<{ top: number; left: number; maxH: number }>({ top: -9999, left: 0, maxH: 480 });

  const meta = $derived(auth.meta);
  const current = $derived(model ?? effectiveModel);
  const currentId = $derived(canonicalModelId(provider, current, meta));
  const rows = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list =
      view === '★'
        ? (() => {
            const seen = new Set<string>();
            return favs.flatMap((key) => {
              const i = key.indexOf(':');
              const p = key.slice(0, i);
              if (!providers.includes(p)) return [];
              const resolved = canonicalModelId(p, key.slice(i + 1), meta);
              const m = resolved
                ? modelsForProvider(p, meta).find((x) => x.id === resolved)
                : undefined;
              if (!m || seen.has(`${p}:${m.id}`)) return [];
              seen.add(`${p}:${m.id}`);
              return [{ p, m }];
            });
          })()
        : modelsForProvider(view, meta).map((m) => ({ p: view, m }));
    return q ? list.filter(({ m }) => `${m.label} ${m.id}`.toLowerCase().includes(q)) : list;
  });
  const shown = $derived(showAll || query.trim() ? rows : rows.slice(0, PAGE));
  const hasDefaultRow = $derived(view !== '★' && supportsModelSelection(view, meta) && !query);
  const selectableItems = $derived.by(() => {
    const out: Array<{ p: string; id: string | null }> = [];
    if (hasDefaultRow && pickable(view)) out.push({ p: view, id: null });
    for (let i = 0; i < shown.length; i++) {
      const r = shown[i];
      if (pickable(r.p)) out.push({ p: r.p, id: r.m.id });
    }
    return out;
  });

  $effect(() => {
    void view;
    void query;
    activeIdx = 0;
  });
  const efforts = $derived.by((): EffortLevel[] => {
    if (!oneffort) return [];
    if (!effortLevels) return supportsEffortSelection(provider, meta) ? effortsForModel(provider, current, meta) : [];
    const reported = effortLevels.map(normalizeEffort).filter((e): e is EffortLevel => e !== null);
    // The model's own levels, bounded by what the agent takes.
    const model = supportsEffortSelection(provider, meta) ? effortsForModel(provider, current, meta).filter((e) => reported.includes(e)) : [];
    return model.length ? model : reported;
  });

  function pickable(p: string): boolean {
    return providerReadiness(p).available && (!lockProvider || p === provider);
  }

  function pick(p: string, id: string | null): void {
    if (!pickable(p)) return;
    if (p !== provider) {
      onprovider?.(p);
    }
    // Drop an effort the new pick can't take BEFORE the model lands: a host
    // that acts on `onmodel` at once (Ask another agent adds its chip there)
    // must not send the previous provider's level.
    if (effort !== null && oneffort) {
      const nextEfforts = supportsEffortSelection(p, meta) ? effortsForModel(p, id, meta) : [];
      if (!nextEfforts.includes(effort)) {
        oneffort(null);
      }
    }
    onmodel(id);
  }

  function pickEffort(lvl: EffortLevel | null): void {
    if (model !== null && currentId !== null && model !== currentId) {
      onmodel(currentId);
    }
    oneffort?.(lvl);
  }

  function place(): void {
    if (panelEl) pos = placeAnchored(anchor, panelEl, { prefer: 'above', align: 'end' });
  }

  function closePicker(): void {
    onclose();
    anchor.focus();
  }

  async function expandMore(): Promise<void> {
    const firstNewIdx = (hasDefaultRow && pickable(view) ? 1 : 0) + PAGE;
    showAll = true;
    await tick();
    const target = panelEl?.querySelector<HTMLElement>(`[data-mp-idx="${firstNewIdx}"]`) ?? searchEl;
    target?.focus();
  }

  onMount(() => {
    void tick().then(() => {
      place();
      searchEl?.focus();
    });
    const onResize = (): void => place();
    const onWinKey = (e: KeyboardEvent): void => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopPropagation();
      closePicker();
    };
    window.addEventListener('resize', onResize);
    window.addEventListener('keydown', onWinKey, true);
    // The panel grows after it opens (models load, the reasoning row appears for the picked
    // model): re-place on every size change so it never ends below the window.
    const grown = typeof ResizeObserver === 'undefined' || !panelEl ? null : new ResizeObserver(() => place());
    if (grown && panelEl) grown.observe(panelEl);
    return () => {
      grown?.disconnect();
      window.removeEventListener('resize', onResize);
      window.removeEventListener('keydown', onWinKey, true);
    };
  });

  $effect(() => {
    void shown.length;
    void view;
    void tick().then(place);
  });

  function onSearchKeydown(e: KeyboardEvent): void {
    if (selectableItems.length === 0) return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const count = selectableItems.length;
      const clamped = Math.min(Math.max(activeIdx, 0), count - 1);
      activeIdx = (clamped + (e.key === 'ArrowDown' ? 1 : count - 1)) % count;
      void tick().then(() => {
        panelEl?.querySelector<HTMLElement>(`[data-mp-idx="${activeIdx}"]`)?.scrollIntoView({ block: 'nearest' });
      });
      return;
    }
    if (e.key === 'Enter') {
      const target = selectableItems[Math.min(Math.max(activeIdx, 0), selectableItems.length - 1)];
      if (target) {
        e.preventDefault();
        pick(target.p, target.id);
      }
    }
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      closePicker();
      return;
    }
    if ((e.metaKey || e.ctrlKey) && /^[1-4]$/.test(e.key)) {
      const r = shown[Number(e.key) - 1];
      if (r) {
        e.preventDefault();
        pick(r.p, r.m.id);
      }
    }
  }

  async function custom(): Promise<void> {
    const picked = await confirmer.promptText('Enter an exact model ID or alias supported by this CLI harness.', {
      title: 'Custom model',
      confirmLabel: 'Use model',
      initial: model ?? '',
      placeholder: 'Model ID',
      validate: (v) => (v.trim() ? null : 'Model ID is required'),
    });
    if (picked && picked.trim()) onmodel(picked.trim());
  }
</script>

<button type="button" class="mp-catch" aria-label="Close model picker" title="Close model picker" tabindex="-1" onclick={onclose}></button>
<div
  bind:this={panelEl}
  class="mp-panel"
  role="dialog"
  aria-label="Model, reasoning and access"
  tabindex="-1"
  style:top="{pos.top}px"
  style:left="{pos.left}px"
  style:max-height="{pos.maxH}px"
  data-testid="model-picker"
  onkeydown={onKey}
>
  <nav class="mp-rail" aria-label="Providers">
    <button
      type="button"
      class="mp-prov"
      class:on={view === '★'}
      aria-pressed={view === '★'}
      aria-label="Favorite models"
      title="Favorite models"
      onclick={() => (view = '★')}
    >
      <Icon name="star" size={14} />
    </button>
    {#each providers as p (p)}
      {@const ready = providerReadiness(p)}
      {@const provTitle = !ready.available
        ? `${harnessLabel(p)} — ${ready.message}`
        : lockProvider && p !== provider
          ? `${harnessLabel(p)} — start a new thread to use it`
          : harnessLabel(p)}
      <button
        type="button"
        class="mp-prov"
        class:on={view === p}
        aria-pressed={view === p}
        aria-label={provTitle}
        title={provTitle}
        disabled={!ready.available}
        data-testid="model-picker-provider-{p}"
        onclick={() => {
          view = p;
          showAll = false;
          if (!lockProvider && p !== provider) onprovider?.(p);
        }}
      >
        <ProviderIcon provider={p} size={16} />
      </button>
    {/each}
  </nav>

  <div class="mp-main">
    <div class="mp-cols">
      <section class="mp-models" aria-label="Models">
        <h3 class="mp-provname" data-testid="model-picker-provider-name">
          {#if view === '★'}<Icon name="star" size={13} />Favorite models{:else}<ProviderIcon provider={view} size={14} />{harnessLabel(view)}{/if}
        </h3>
        <div class="mp-search">
          <Icon name="search" size={13} />
          <input
            bind:this={searchEl}
            bind:value={query}
            type="search"
            dir="auto"
            placeholder="Search models…"
            aria-label="Search models"
            data-testid="model-picker-search"
            onkeydown={onSearchKeydown}
          />
        </div>
        {#if hasDefaultRow}
          {@const defaultSelectable = pickable(view)}
          <button
            type="button"
            class="mp-row"
            class:cur={view === provider && model === null}
            class:kbd-active={defaultSelectable && activeIdx === 0}
            data-mp-idx={defaultSelectable ? 0 : undefined}
            disabled={!defaultSelectable}
            onclick={() => pick(view, null)}
          >
            <span class="mp-check">{#if view === provider && model === null}<Icon name="check" size={12} />{/if}</span>
            <span class="mp-name">Default model</span>
          </button>
        {/if}
        {#each shown as r, i (`${r.p}:${r.m.id}`)}
          {@const key = `${r.p}:${r.m.id}`}
          {@const isCur = r.p === provider && model !== null && currentId === r.m.id}
          {@const rowSelectable = pickable(r.p)}
          {@const rowIdx = rowSelectable ? selectableItems.findIndex((s) => s.p === r.p && s.id === r.m.id) : -1}
          {@const favTitle = favs.includes(key) ? `Remove ${r.m.label} from favorites` : `Add ${r.m.label} to favorites`}
          <div class="mp-rowwrap">
            <button
              type="button"
              class="mp-row"
              class:cur={isCur}
              class:kbd-active={rowIdx >= 0 && activeIdx === rowIdx}
              data-mp-idx={rowIdx >= 0 ? rowIdx : undefined}
              title={r.m.description || r.m.id}
              disabled={!rowSelectable}
              onclick={() => pick(r.p, r.m.id)}
            >
              <span class="mp-check">{#if isCur}<Icon name="check" size={12} />{/if}</span>
              <span class="mp-name ellipsis">{r.m.label}</span>
              {#if view === '★'}<span class="mp-hint">{harnessLabel(r.p)}</span>{/if}
              {#if i < 4}<span class="mp-hint mono mp-kbd">⌘{i + 1}</span>{/if}
            </button>
            <button
              type="button"
              class="icon-btn mp-fav"
              class:on={favs.includes(key)}
              aria-pressed={favs.includes(key)}
              aria-label={favTitle}
              title={favTitle}
              onclick={() => toggleFav(key)}
            >
              <Icon name="star" size={12} />
            </button>
          </div>
        {:else}
          <p class="mp-empty">{view === '★' ? 'Star a model to keep it here.' : 'No models match.'}</p>
        {/each}
        {#if !showAll && !query.trim() && rows.length > PAGE}
          <button type="button" class="btn small ghost mp-more" onclick={() => void expandMore()}>
            Show {rows.length - PAGE} more
          </button>
        {/if}
        {#if view !== '★' && supportsModelSelection(view, meta) && pickable(view)}
          <button type="button" class="btn small ghost mp-more" onclick={() => void custom()}>Custom model…</button>
        {/if}
      </section>
    </div>

    {#if efforts.length > 0}
      <section class="mp-effort" aria-label="Reasoning" data-testid="picker-effort">
        <span class="mp-foot-label">Reasoning</span>
        <div class="segmented mp-effort-row" role="group" aria-label="Reasoning effort">
          <button type="button" class:active={effort === null} aria-pressed={effort === null} onclick={() => pickEffort(null)}>Default</button>
          {#each efforts as lvl (lvl)}
            <button type="button" class:active={effort === lvl} aria-pressed={effort === lvl} onclick={() => pickEffort(lvl)}>{formatEffortLabel(lvl)}</button>
          {/each}
        </div>
      </section>
    {/if}

    {#if onmode && supportsInteractionMode(provider)}
      <footer class="mp-foot">
        <span class="mp-foot-label">Access</span>
        <div class="segmented" role="group" aria-label="Access mode" data-testid="picker-mode-pop">
          {#each INTERACTION_MODES as m (m.id)}
            <button type="button" class:active={mode === m.id} aria-pressed={mode === m.id} title={m.hint} onclick={() => onmode?.(m.id)}>
              {m.shortLabel}
            </button>
          {/each}
        </div>
      </footer>
    {/if}
  </div>
</div>

<style>
  .mp-catch {
    position: fixed;
    inset: 0;
    z-index: var(--z-popover-backdrop);
    border: 0;
    padding: 0;
    background: transparent;
    cursor: default;
  }
  .mp-panel {
    position: fixed;
    z-index: var(--z-popover);
    display: flex;
    inline-size: min(560px, calc(100vw - 16px));
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    box-shadow: var(--glass-shadow);
    color: var(--text);
    font-size: var(--fs-m);
  }
  .mp-rail {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 4px;
    border-inline-end: 1px solid var(--border);
    overflow-y: auto;
    flex-shrink: 0;
  }
  .mp-prov {
    display: grid;
    place-items: center;
    inline-size: 32px;
    block-size: 32px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .mp-prov:hover:not(:disabled) {
    background: var(--hover);
  }
  .mp-prov.on {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .mp-prov:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .mp-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .mp-cols {
    flex: 1;
    min-height: 0;
    display: flex;
    overflow: hidden;
  }
  .mp-models {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 6px;
    overflow-y: auto;
  }
  .mp-models {
    flex: 1;
    min-width: 0;
  }
  /* Reasoning: one compact row under the models (was a tall column). */
  .mp-effort {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-block-start: 1px solid var(--border);
    flex-shrink: 0;
    min-width: 0;
  }
  .mp-effort-row {
    flex-wrap: wrap;
    min-width: 0;
  }
  /* The rail is icons only: the picked provider is named here. */
  .mp-provname {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    padding: 4px 8px 2px;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }
  .mp-search {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-inline: 8px;
    margin-block-end: 4px;
    color: var(--text-dim);
  }
  .mp-search input {
    flex: 1;
    min-width: 0;
    block-size: 30px;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    outline: none;
  }
  .mp-search:focus-within {
    color: var(--text);
    box-shadow: 0 1px 0 0 var(--accent-text);
  }
  .mp-rowwrap {
    display: flex;
    align-items: center;
    min-width: 0;
  }
  .mp-row {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    block-size: 30px;
    padding-inline: 6px 8px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .mp-row:hover:not(:disabled),
  .mp-row.kbd-active:not(:disabled) {
    background: var(--hover);
  }
  .mp-row.kbd-active:not(:disabled) {
    outline: 1px solid var(--accent-line);
    outline-offset: -1px;
  }
  .mp-row.cur {
    font-weight: 500;
  }
  .mp-row:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .mp-check {
    inline-size: 14px;
    display: inline-flex;
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .mp-name {
    flex: 1;
    min-width: 0;
  }
  /* A touch screen has no ⌘ key: no shortcut hints there. */
  @media (pointer: coarse) {
    .mp-kbd {
      display: none;
    }
  }
  .mp-hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .mp-fav {
    color: var(--text-dim);
  }
  .mp-fav.on {
    color: var(--warning);
  }
  .mp-fav.on :global(svg path) {
    fill: currentColor;
  }
  .mp-empty {
    margin: 0;
    padding: 10px 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .mp-more {
    align-self: flex-start;
    margin: 4px;
  }
  .mp-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
    border-block-start: 1px solid var(--border);
    flex-shrink: 0;
  }
  .mp-foot-label {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
