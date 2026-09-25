<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  import SettingToggle from './SettingToggle.svelte';
  // Theme (native / pro-dark / warm), scheme (auto / light / dark), accent.
  import {
    ui,
    TERM_FONT_OPTIONS,
    type SchemePref,
    type ThemeName,
    type Direction,
  } from '../../lib/stores/ui.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { AUTO_VERTICAL_ENGINES } from '../../lib/db-view-prefs';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { AMBIENT_MODES, ambientImage, type AmbientMode } from '../../lib/ambient';
  import { processWallpaper } from '../../lib/wallpaper';
  import { barStore } from '../../lib/stores/bar.svelte';
  import type { BarPref } from '../../lib/floatingBar';
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

  // The "Type or speak… ⌘K" bar over the content column (FloatingBar.svelte).
  const barPrefs: { id: BarPref; label: string; hint: string }[] = [
    { id: 'auto', label: 'Auto', hint: 'A short pill at rest, full on Home; it docks into the status bar while you scroll or type in a terminal or editor.' },
    { id: 'pinned', label: 'Always full', hint: 'The whole pill (model, spaces) stays up; it only shrinks while a terminal or editor has the keyboard.' },
    { id: 'docked', label: 'Docked', hint: 'A small chip in the status bar that never covers content; ⌘K opens the full bar.' },
    { id: 'hidden', label: 'Hidden', hint: 'No bar; ⌘K opens the command palette sheet instead.' },
  ];

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

  const themes: { id: ThemeName; name: string; desc: string }[] = [
    { id: 'native', name: 'Native', desc: 'macOS vibrancy, blue accent' },
    { id: 'pro-dark', name: 'Pro Dark', desc: 'Always-dark, violet accent' },
    { id: 'warm', name: 'Warm', desc: 'Paper tones, green accent' },
  ];
  const schemes: { id: SchemePref; label: string }[] = [
    { id: 'auto', label: 'Auto' },
    { id: 'light', label: 'Light' },
    { id: 'dark', label: 'Dark' },
  ];
  const directions: { id: Direction; label: string }[] = [
    { id: 'ltr', label: 'Left-to-right' },
    { id: 'rtl', label: 'Right-to-left' },
  ];

  // Backdrop previews: the real generated art for the accent + scheme in
  // effect (re-derived when either changes), drawn over the window colour.
  let accentNow = $state('');
  $effect(() => {
    void ui.theme;
    void ui.accent;
    void ui.resolvedScheme;
    accentNow = ui.accent || getComputedStyle(document.documentElement).getPropertyValue('--accent').trim();
  });
  function previewOf(mode: AmbientMode): string {
    const photo = ui.ambientPhoto ? ui.ambientPhoto[ui.resolvedScheme] : null;
    return ambientImage(mode, accentNow, ui.resolvedScheme, photo);
  }

  let photoBusy = $state(false);
  async function pickPhoto(e: Event & { currentTarget: HTMLInputElement }): Promise<void> {
    const input = e.currentTarget;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    photoBusy = true;
    try {
      const photo = await processWallpaper(file);
      if (ui.setAmbientPhoto(photo)) toasts.success('Wallpaper set', 'Stored on this device only.');
      else toasts.error('Could not store the photo', 'This browser refused the storage — try a smaller image.');
    } catch (err) {
      toasts.error('Could not use that image', err instanceof Error ? err.message : String(err));
    } finally {
      photoBusy = false;
    }
  }

  const swatches: Record<ThemeName, { bg: string; fg: string; acc: string }> = {
    native: { bg: '#1e1e23', fg: '#f2f2f5', acc: '#0a84ff' },
    'pro-dark': { bg: '#16161c', fg: '#e8e8ee', acc: '#6c5ce7' },
    warm: { bg: '#211f1b', fg: '#e8e4da', acc: '#2bb673' },
  };
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('appearance')} subtitle="Themes apply instantly and persist per device" />
  <PageBody width="readable">

  <div class="section-title">Theme</div>
  <div class="theme-grid">
    {#each themes as t (t.id)}
      <button class="theme-card" class:selected={ui.theme === t.id} aria-pressed={ui.theme === t.id} onclick={() => ui.setTheme(t.id)}>
        <div class="theme-preview" style="background: {swatches[t.id].bg}">
          <div class="tp-bar" style="background: {swatches[t.id].acc}"></div>
          <div class="tp-line" style="background: {swatches[t.id].fg}; opacity: 0.8"></div>
          <div class="tp-line short" style="background: {swatches[t.id].fg}; opacity: 0.4"></div>
        </div>
        <div class="theme-name">{t.name}</div>
        <div class="theme-desc">{t.desc}</div>
      </button>
    {/each}
  </div>

  <div class="section-title">Scheme</div>
  <!-- Pro Dark resolves to dark whatever the scheme says (ui.applyTheme), so
       the picker is shown as not applying instead of silently doing nothing. -->
  <div class="segmented" class:off={ui.theme === 'pro-dark'} role="group" aria-label="Scheme">
    {#each schemes as s (s.id)}
      <button
        class:active={ui.scheme === s.id}
        aria-pressed={ui.scheme === s.id}
        disabled={ui.theme === 'pro-dark'}
        title={ui.theme === 'pro-dark' ? 'Pro Dark is always dark' : undefined}
        onclick={() => ui.setScheme(s.id)}>{s.label}</button
      >
    {/each}
  </div>
  <p class="hint-line">
    {ui.theme === 'pro-dark'
      ? 'Pro Dark is always dark. Pick Native or Warm to use a light scheme.'
      : 'Auto follows the system light/dark preference.'}
  </p>

  <div class="section-title">Direction</div>
  <div class="segmented" role="group" aria-label="Direction">
    {#each directions as d (d.id)}
      <button class:active={ui.direction === d.id} aria-pressed={ui.direction === d.id} onclick={() => ui.setDirection(d.id)}>{d.label}</button>
    {/each}
  </div>
  <p class="hint-line">Right-to-left mirrors the layout for RTL languages (Hebrew, Arabic).</p>

  <div class="section-title">Accent colour</div>
  <div class="row">
    <input
      type="color"
      class="accent-input"
      value={ui.accent || (/^#[0-9a-f]{6}$/i.test(accentNow) ? accentNow : '#0a84ff')}
      oninput={(e) => ui.setAccent(e.currentTarget.value)}
      aria-label="Accent colour"
    />
    <span class="accent-val">{ui.accent ? ui.accent.toUpperCase() : 'Theme default'}</span>
    {#if ui.accent}
      <button class="btn small" onclick={() => ui.setAccent('')}>Reset</button>
    {/if}
  </div>

  <div class="section-title">Backdrop</div>
  <div class="theme-grid" role="radiogroup" aria-label="Backdrop">
    {#each AMBIENT_MODES as m (m.id)}
      <button
        class="theme-card"
        class:selected={ui.ambient === m.id}
        role="radio"
        aria-checked={ui.ambient === m.id}
        data-ambient-option={m.id}
        onclick={() => ui.setAmbient(m.id)}
      >
        <div class="theme-preview ambient-preview" style:background-image={previewOf(m.id)}>
          <span class="ap-side"></span>
          <span class="ap-card"></span>
        </div>
        <div class="theme-name">{m.label}</div>
        <div class="theme-desc">{m.id === 'wallpaper' && ui.ambientPhoto ? 'Your photo' : m.desc}</div>
      </button>
    {/each}
  </div>
  <div class="row photo-row">
    <label class="btn small" class:busy={photoBusy}>
      <Icon name="image" size={12} />
      {ui.ambientPhoto ? 'Change photo…' : 'Use your own photo…'}
      <input type="file" accept="image/*" class="visually-hidden" onchange={pickPhoto} disabled={photoBusy} />
    </label>
    {#if ui.ambientPhoto}
      <button class="btn small ghost" onclick={() => ui.setAmbientPhoto(null)}>Remove photo</button>
    {/if}
  </div>
  <p class="hint-line">
    The backdrop shows through the sidebar, toolbar and status bar, and fills the Home desktop.
    Pages, tables, editors and terminals stay solid. A photo never leaves this device: Otto
    blurs it and tones it for light and dark so the text over it stays readable.
  </p>
  <div class="toggle-block">
    <SettingToggle
      label="Reduce transparency"
      hint="Solid sidebar, toolbar and menus with no backdrop. Otto also follows the macOS “Reduce transparency” accessibility setting."
      checked={ui.reduceTransparency}
      onchange={(v) => ui.setReduceTransparency(v)}
    />
  </div>

  <div class="section-title">Terminal font</div>
  <div class="segmented" role="group" aria-label="Terminal font">
    {#each TERM_FONT_OPTIONS as f (f.id)}
      <button
        class:active={ui.termFontFamily === f.id}
        aria-pressed={ui.termFontFamily === f.id}
        title={f.desc}
        onclick={() => ui.setTermFontFamily(f.id)}>{f.name}</button
      >
    {/each}
  </div>
  <p class="hint-line">
    Hebrew &amp; other right-to-left text renders crisply via the bundled Cousine font in every
    option. Change applies to open terminals instantly.
  </p>
  <!-- Same store setters as the session header's terminal controls, so the
       two can't drift; also the only way back when those controls are hidden. -->
  <div class="row term-size-row" role="group" aria-label="Terminal font size">
    <span class="term-size-label">Font size</span>
    <button class="sb-btn" onclick={() => ui.termZoomOut()} disabled={ui.termFontSize <= 8} title="Smaller (⌘− in a terminal)" aria-label="Terminal font smaller"><Icon name="minus" size={12} /></button>
    <span class="mono term-size-val" aria-live="polite">{ui.termFontSize}px</span>
    <button class="sb-btn" onclick={() => ui.termZoomIn()} disabled={ui.termFontSize >= 28} title="Larger (⌘+ in a terminal)" aria-label="Terminal font larger"><Icon name="plus" size={12} /></button>
    {#if ui.termFontSize !== 13}
      <button class="btn small ghost" onclick={() => ui.termZoomReset()}>Reset</button>
    {/if}
  </div>
  <div class="toggle-block">
    <SettingToggle
      label="Copy on select"
      hint="Selecting text in a terminal copies it to the clipboard."
      checked={ui.termCopyOnSelect}
      onchange={(v) => ui.setTermCopyOnSelect(v)}
    />
    <SettingToggle
      label="Terminal toolbar"
      hint="Show the font-size and copy-on-select controls on each terminal."
      checked={ui.termToolbar}
      onchange={(v) => ui.setTermToolbar(v)}
    />
  </div>

  <div class="section-title">Right-to-left text <span class="chip exp-tag">Experimental</span></div>
  <div class="toggle-block">
    <SettingToggle
      label="Right-to-left text in the terminal"
      checked={ui.rtlBidi}
      onchange={(v) => ui.setRtlBidi(v)}
    >
      Lays out Hebrew right-to-left with English embedded left-to-right, using the browser's bidi
      engine (it switches the terminal off the GPU renderer). Text is reflowed for reading, so the
      monospace grid no longer lines up exactly: good for chat-style output, imperfect for TUI
      tables or box art. Toggling reloads open terminals.
    </SettingToggle>
  </div>

  <div class="section-title">Floating bar</div>
  <div class="segmented" role="radiogroup" aria-label="Floating bar">
    {#each barPrefs as b (b.id)}
      <button
        role="radio"
        aria-checked={barStore.pref === b.id}
        class:active={barStore.pref === b.id}
        onclick={() => barStore.setPref(b.id)}
      >{b.label}</button>
    {/each}
  </div>
  <p class="hint-line">
    {barPrefs.find((b) => b.id === barStore.pref)?.hint} Desktop window only — phones and tablets
    keep the ⌘K sheet. Saved per device.
  </p>

  <div class="section-title">Sessions on this device</div>
  <div class="toggle-block">
    <SettingToggle
      label="Isolate sessions to this device"
      hint="Only show sessions started on this device. Other devices' sessions stay hidden here (they still run on the daemon)."
      checked={ui.sessionIsolation}
      onchange={(v) => ui.setSessionIsolation(v)}
      testid="session-isolation-toggle"
    />
  </div>
  <div class="section-title">Closing a session tab</div>
  <p class="hint-line">
    Closing a tab (×, ⌘W, sidebar ×) ends the session — the same as Archive or Delete from its
    menu. Choose what happens, or be asked each time. A remembered choice applies without asking —
    "Always delete" also skips the confirm on a session's Delete command. Closing or deleting
    several sessions at once still asks once, naming the count.
  </p>
  <div class="radio-col" role="radiogroup" aria-label="When closing a session tab">
    <label class="switch-row">
      <input type="radio" name="close-tab-pref" checked={ui.closeTabPref === ''} onchange={() => ui.setCloseTabPref('')} />
      <span>Ask every time (Archive / Delete)</span>
    </label>
    <label class="switch-row">
      <input type="radio" name="close-tab-pref" checked={ui.closeTabPref === 'archive'} onchange={() => ui.setCloseTabPref('archive')} />
      <span>Always archive — stop it, keep the history (resumable from Archived)</span>
    </label>
    <label class="switch-row">
      <input type="radio" name="close-tab-pref" checked={ui.closeTabPref === 'delete'} onchange={() => ui.setCloseTabPref('delete')} />
      <span>Always delete — stop it and remove its history for good (can't be undone)</span>
    </label>
  </div>

  <div class="section-title">Database Explorer</div>
  <p class="hint-line">
    Open wide results in the Vertical view (one record per block) instead of the grid. Set per
    engine: on for MongoDB, whose documents are nested; off for the SQL engines, where a wide
    table is what the grid is for. MongoDB results open in Vertical anyway until you pick Grid
    or JSON on one of the connection's tabs — this threshold then still sends wide ones back to
    Vertical.
  </p>
  <div class="av-table" role="group" aria-label="Auto-Vertical by engine" data-testid="db-auto-vertical">
    {#each AUTO_VERTICAL_ENGINES as eng (eng.id)}
      {@const n = ui.dbAutoVertical[eng.id]}
      <div class="av-row">
        <label class="switch-row av-toggle">
          <input
            type="checkbox"
            checked={n > 0}
            onchange={(e) => ui.setDbAutoVertical(eng.id, e.currentTarget.checked ? 10 : 0)}
          />
          <span>{eng.label}</span>
        </label>
        <label class="av-num" class:off={n === 0}>
          <span>more than</span>
          <input
            type="number"
            class="input num-input mono"
            min="1"
            max="500"
            step="1"
            disabled={n === 0}
            value={n === 0 ? '' : n}
            placeholder="—"
            oninput={(e) => {
              // A cleared box is mid-edit, not "off" — keep the setting until a number lands.
              const v = e.currentTarget.value;
              if (v !== '' && Number(v) > 0) ui.setDbAutoVertical(eng.id, Number(v));
            }}
            onchange={(e) => {
              // Leaving the box: show what was actually saved (a cleared box,
              // 0, a decimal or a value past 500 would otherwise keep showing
              // a number that isn't in effect).
              e.currentTarget.value = String(ui.dbAutoVertical[eng.id] || '');
            }}
            aria-label="{eng.label}: column threshold"
          />
          <span>columns</span>
        </label>
      </div>
    {/each}
  </div>
  <p class="hint-line">
    A view you pick on a tab (the Grid / Vertical / JSON switch or ⇧⌘V) always wins. Saved per
    device.
  </p>

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
  </PageBody>
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .theme-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 12px;
    max-width: var(--settings-col);
  }
  .theme-card {
    text-align: start;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 10px;
    cursor: pointer;
    transition: border-color 130ms ease-out;
  }
  .theme-card:hover:not(.selected) {
    border-color: var(--border-strong);
  }
  .theme-card.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .theme-preview {
    height: 72px;
    border-radius: var(--radius-s);
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-bottom: 8px;
  }
  /* Backdrop preview: the window colour under the generated art, with a
     sketch of the sidebar glass and a content card on top. */
  .ambient-preview {
    position: relative;
    background-color: var(--bg);
    background-size: cover;
    border: 1px solid var(--border);
    overflow: hidden;
  }
  .ap-side {
    position: absolute;
    inset-block: 0;
    inset-inline-start: 0;
    width: 30%;
    background: var(--glass-tint);
    border-inline-end: 1px solid var(--separator);
  }
  .ap-card {
    position: absolute;
    inset-block: 14px;
    inset-inline: 40% 10%;
    border-radius: var(--radius-s);
    background: var(--surface);
    box-shadow: var(--shadow-card);
  }
  .photo-row {
    margin-top: 10px;
    gap: 6px;
  }
  .photo-row label:focus-within {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent);
    outline-offset: 1px;
  }
  .photo-row .busy {
    opacity: 0.6;
    pointer-events: none;
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  /* SettingToggle rows under a picker or hint (their own 8px padding sets
     the rhythm; the block only separates them from what's above). */
  .toggle-block {
    max-width: var(--settings-col);
    margin-top: 6px;
  }
  /* A control that doesn't apply under the current theme (Scheme on Pro Dark). */
  .segmented.off {
    opacity: 0.55;
  }
  .segmented.off > button {
    cursor: default;
  }
  .term-size-row {
    margin-top: 12px;
    gap: 4px;
  }
  .term-size-label {
    font-size: var(--fs-m);
    color: var(--text);
    margin-inline-end: 6px;
  }
  .term-size-val {
    min-width: 40px;
    text-align: center;
  }
  .term-size-row .sb-btn {
    border: 1px solid var(--border);
    background: var(--surface-2);
    font-size: var(--fs-m);
    color: var(--text);
  }
  .tp-bar {
    width: 34px;
    height: 8px;
    border-radius: 3px;
  }
  .tp-line {
    width: 80%;
    height: 5px;
    border-radius: 2px;
  }
  .tp-line.short {
    width: 55%;
  }
  .theme-name {
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .theme-desc {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .hint-line {
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text-dim);
    margin: 8px 0 0;
    max-width: var(--settings-col);
  }
  .accent-val {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .switch-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-m);
    color: var(--text);
    cursor: pointer;
    user-select: none;
  }
  .radio-col {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 10px 0;
  }
  .switch-row input {
    width: 15px;
    height: 15px;
    accent-color: var(--accent);
    cursor: pointer;
  }
  /* A sentence-case chip inside the uppercase section title. */
  .exp-tag {
    text-transform: none;
    letter-spacing: normal;
    margin-inline-start: 6px;
    vertical-align: middle;
  }
  .num-input {
    width: 64px;
    text-align: end;
  }
  /* Auto-Vertical per engine: toggle · "more than N columns", one row each. */
  /* Two columns (engine · threshold) so every "more than N columns" sits in
     one aligned column right beside its engine, instead of being flung to
     the far edge of the box by space-between. */
  .av-table {
    display: grid;
    grid-template-columns: max-content max-content;
    column-gap: 20px;
    row-gap: 4px;
    margin-top: 8px;
  }
  .av-row {
    display: contents;
  }
  .av-toggle {
    min-height: 30px;
  }
  .av-num {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .av-num.off {
    opacity: 0.55;
  }
  .accent-input {
    width: 36px;
    height: 27px;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    cursor: pointer;
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
    padding: 0 4px 0 8px;
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
    padding: 8px 4px 2px 8px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.07em;
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
    margin-inline-end: 27px;
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
    background: var(--surface-2);
    color: var(--text);
  }
  .sb-btn:disabled {
    opacity: 0.25;
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
