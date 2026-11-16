<script lang="ts">
  // Settings › Appearance › Sidebar (lazy: below the fold).
  import Icon from '../../lib/components/Icon.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { plugins } from '../../lib/stores/plugins.svelte';
  import {
    FAVORITES_ID,
    availableModules,
    moveAmong,
    moveWithinGroup,
    resolveGroupOrder,
    resolveOrder,
    sidebarSections,
    type SidebarPluginEntry,
  } from '../../lib/sidebar';

  // The full resolved sidebar list (same logic as the Navigator/Rail): built-ins
  // the user may see + permitted plugins, in the saved order, including hidden
  // ones (so they can be toggled back on here).
  const sidebarPlugins = $derived(
    plugins.list
      .filter((p) => auth.canPlugin(p.slug, 'view'))
      .map((p): SidebarPluginEntry => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name })),
  );
  const sidebarResolved = $derived(
    resolveOrder(availableModules((f) => auth.can(f, 'view'), sidebarPlugins), ui.sidebarOrder),
  );
  // Section by section exactly as the sidebar shows them: Favorites first
  // (while it has any), then the sections in the user's order.
  const sidebarSecs = $derived(sidebarSections(sidebarResolved, ui.sidebarFavorites, ui.sidebarGroupOrder));
  const sidebarFavIds = $derived(
    sidebarSecs[0]?.group.id === FAVORITES_ID ? sidebarSecs[0].modules.map((m) => m.id) : [],
  );
  const sidebarMovableSecs = $derived(
    sidebarSecs.filter((s) => s.group.id !== FAVORITES_ID).map((s) => s.group.id as string),
  );
  /** Up/down within the module's sidebar section (same rule as the Navigator);
   *  a favorite moves among the favorites. */
  function moveSidebarItem(id: string, delta: -1 | 1): void {
    if (sidebarFavIds.includes(id)) {
      const favs = moveAmong(ui.sidebarFavorites, id, delta, (x) => sidebarFavIds.includes(x));
      if (favs) ui.setSidebarFavorites(favs);
      return;
    }
    const next = moveWithinGroup(sidebarResolved, id, delta);
    if (next) ui.setSidebarOrder(next);
  }
  /** Up/down for a whole section (Favorites always stays first). */
  function moveSidebarSection(id: string, delta: -1 | 1): void {
    const all = resolveGroupOrder(ui.sidebarGroupOrder).map((g) => g.id as string);
    const next = moveAmong(all, id, delta, (g) => sidebarMovableSecs.includes(g));
    if (next) ui.setSidebarGroupOrder(next);
  }
</script>

<div class="section-title">Sidebar</div>
<p class="hint-line">
  Show, hide and reorder the items and sections of the left sidebar — keep only what you use. Star
  an item to pin it to Favorites at the top. Hidden items can be brought back here anytime. You
  can also drag directly in the sidebar (“Customize sidebar” at the bottom of the expanded
  sidebar). Saved per device.
</p>
<div class="sidebar-list" data-testid="settings-sidebar-list">
  <!-- Section by section, as the sidebar shows them; item moves stay
       in-section (a favorite moves among the favorites). -->
  {#each sidebarSecs as sec (sec.group.id)}
    {@const isFavSec = sec.group.id === FAVORITES_ID}
    {@const si = sidebarMovableSecs.indexOf(sec.group.id)}
    <div class="sidebar-group-label">
      {#if isFavSec}<span class="sb-group-star"><Icon name="star" size={12} /></span>{/if}
      <span class="grow">{sec.group.label}</span>
      {#if !isFavSec}
        <button
          class="sb-btn"
          onclick={() => moveSidebarSection(sec.group.id, -1)}
          disabled={si <= 0}
          title="Move section up"
          aria-label={`Move ${sec.group.label} section up`}
        >
          <Icon name="arrowUp" size={12} />
        </button>
        <button
          class="sb-btn"
          onclick={() => moveSidebarSection(sec.group.id, 1)}
          disabled={si >= sidebarMovableSecs.length - 1}
          title="Move section down"
          aria-label={`Move ${sec.group.label} section down`}
        >
          <Icon name="arrowDown" size={12} />
        </button>
      {/if}
    </div>
    {#each sec.modules as m, i (m.id)}
      {@const fav = sidebarFavIds.includes(m.id)}
      <div class="sidebar-row" class:row-hidden={ui.sidebarHidden.includes(m.id)}>
        <Icon name={m.icon} size={14} />
        <span class="grow">{m.label}</span>
        <button
          class="sb-btn sb-star"
          class:on={fav}
          onclick={() => ui.toggleSidebarFavorite(m.id)}
          title={fav ? 'Remove from Favorites' : 'Add to Favorites'}
          aria-label={`Favorite ${m.label}`}
          aria-pressed={fav}
        >
          <Icon name="star" size={13} />
        </button>
        <button
          class="sb-btn"
          onclick={() => moveSidebarItem(m.id, -1)}
          disabled={i === 0}
          title="Move up"
          aria-label={`Move ${m.label} up`}
        >
          <Icon name="arrowUp" size={12} />
        </button>
        <button
          class="sb-btn"
          onclick={() => moveSidebarItem(m.id, 1)}
          disabled={i === sec.modules.length - 1}
          title="Move down"
          aria-label={`Move ${m.label} down`}
        >
          <Icon name="arrowDown" size={12} />
        </button>
        <label class="sb-toggle" title={ui.sidebarHidden.includes(m.id) ? 'Hidden' : 'Shown'}>
          <input
            type="checkbox"
            checked={!ui.sidebarHidden.includes(m.id)}
            onchange={() => ui.toggleSidebarHidden(m.id)}
            aria-label={`Show ${m.label}`}
          />
        </label>
      </div>
    {/each}
  {/each}
</div>
<div class="row">
  <button class="btn small" onclick={() => ui.resetSidebar()}>Reset to default</button>
</div>

<style>
  .hint-line {
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text-dim);
    margin: 8px 0 0;
    max-width: var(--settings-col);
  }
  .sidebar-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-width: min(420px, 92vw);
    margin-top: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 6px;
    background: var(--surface);
  }
  .sidebar-row {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding-block: 0; padding-inline: 8px 4px;
    border-radius: var(--radius-s);
    font-size: var(--fs-m);
    color: var(--text);
  }
  .sidebar-row:hover {
    background: var(--hover);
  }
  .sidebar-group-label {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 24px;
    padding-block: 8px 2px; padding-inline: 8px 4px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .sidebar-group-label .grow {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Section moves sit in the same column as the rows' up/down buttons. */
  .sidebar-group-label .sb-btn {
    width: 24px;
    height: 22px;
  }
  .sidebar-group-label .sb-btn:last-child {
    /* align with the rows' down arrow: skip the Show checkbox column
       (row gap 8px + the 19px checkbox label) */
    margin-inline-end: 28px;
  }
  .sb-group-star {
    display: grid;
    place-items: center;
  }
  /* Favorites toggle: an outline star, filled in the accent when on. */
  .sb-star.on {
    color: var(--accent-text);
  }
  .sb-star.on :global(svg path),
  .sb-group-star :global(svg path) {
    fill: currentColor;
  }
  .sidebar-group-label:first-child {
    padding-top: 2px;
  }
  .sidebar-row .grow {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* A hidden module is dimmed but still listed so it can be re-shown. */
  .sidebar-row.row-hidden {
    color: var(--text-dim);
    opacity: 0.7;
  }
  .sb-btn {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .sb-btn:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .sb-btn:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .sb-toggle {
    display: grid;
    place-items: center;
    cursor: pointer;
    padding-inline-start: 4px;
  }
  .sb-toggle input {
    /* No UA margins (they differ between WebKit and Chromium): the section
       arrows' column offset above depends on this column's exact width. */
    margin: 0;
    width: 15px;
    height: 15px;
    accent-color: var(--accent);
    cursor: pointer;
  }
</style>
