<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Theme (native / pro-dark / warm), scheme (auto / light / dark), accent.
  import {
    ui,
    TERM_FONT_OPTIONS,
    type SchemePref,
    type ThemeName,
    type Direction,
  } from '../../lib/stores/ui.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { AMBIENT_MODES, ambientImage, type AmbientMode } from '../../lib/ambient';
  import { processWallpaper } from '../../lib/wallpaper';
  import { barStore } from '../../lib/stores/bar.svelte';
  import type { BarPref } from '../../lib/floatingBar';
  import { plugins } from '../../lib/stores/plugins.svelte';
  import {
    availableModules,
    groupModules,
    moveWithinGroup,
    resolveOrder,
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
  /** Up/down within the module's sidebar section (same rule as the Navigator). */
  function moveSidebarItem(id: string, delta: -1 | 1): void {
    const next = moveWithinGroup(sidebarResolved, id, delta);
    if (next) ui.setSidebarOrder(next);
  }

  const themes: { id: ThemeName; name: string; desc: string }[] = [
    { id: 'native', name: 'Native', desc: 'macOS vibrancy, system accent' },
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
  <PageHeader title="Appearance" subtitle="Themes apply instantly and persist per device." />
  <PageBody width="readable">

  <div class="section-title">Theme</div>
  <div class="theme-grid">
    {#each themes as t (t.id)}
      <button class="theme-card" class:selected={ui.theme === t.id} onclick={() => ui.setTheme(t.id)}>
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
  <div class="segmented">
    {#each schemes as s (s.id)}
      <button class:active={ui.scheme === s.id} onclick={() => ui.setScheme(s.id)}>{s.label}</button>
    {/each}
  </div>
  <p class="hint-line">Auto follows the system light/dark preference.</p>

  <div class="section-title">Direction</div>
  <div class="segmented">
    {#each directions as d (d.id)}
      <button class:active={ui.direction === d.id} onclick={() => ui.setDirection(d.id)}>{d.label}</button>
    {/each}
  </div>
  <p class="hint-line">Right-to-left mirrors the layout for RTL languages (Hebrew, Arabic).</p>

  <div class="section-title">Accent color</div>
  <div class="row">
    <input
      type="color"
      class="accent-input"
      value={ui.accent || '#0a84ff'}
      oninput={(e) => ui.setAccent(e.currentTarget.value)}
      aria-label="Accent color"
    />
    <span class="mono dim">{ui.accent || 'theme default'}</span>
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
  <label class="switch-row reduce-row">
    <input
      type="checkbox"
      checked={ui.reduceTransparency}
      onchange={(e) => ui.setReduceTransparency(e.currentTarget.checked)}
    />
    <span>Reduce transparency</span>
  </label>
  <p class="hint-line">
    Solid sidebar, toolbar and menus with no backdrop. Otto also follows the macOS “Reduce
    transparency” accessibility setting.
  </p>

  <div class="section-title">Terminal font</div>
  <div class="segmented">
    {#each TERM_FONT_OPTIONS as f (f.id)}
      <button
        class:active={ui.termFontFamily === f.id}
        title={f.desc}
        onclick={() => ui.setTermFontFamily(f.id)}>{f.name}</button
      >
    {/each}
  </div>
  <p class="hint-line">
    Hebrew &amp; other right-to-left text renders crisply via the bundled Cousine font in every
    option. Change applies to open terminals instantly.
  </p>

  <div class="section-title">Right-to-left text <span class="exp-tag">Experimental</span></div>
  <label class="switch-row">
    <input
      type="checkbox"
      checked={ui.rtlBidi}
      onchange={(e) => ui.setRtlBidi(e.currentTarget.checked)}
    />
    <span>Right-to-left text in the terminal</span>
  </label>
  <p class="hint-line warn">
    ⚠ Lays out Hebrew right-to-left with English embedded left-to-right, using the browser's bidi
    engine (switches the terminal off the GPU renderer). Because text is reflowed for reading, the
    monospace grid no longer lines up exactly — great for chat-style output, imperfect for TUI
    tables or box art. Toggling reloads open terminals.
  </p>

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
  <label class="switch-row">
    <input
      type="checkbox"
      checked={ui.sessionIsolation}
      onchange={(e) => ui.setSessionIsolation(e.currentTarget.checked)}
    />
    <span>Isolate sessions to this device</span>
  </label>
  <p class="hint-line">
    Only show sessions started on this device. Other devices' sessions stay hidden here (they
    still run on the daemon).
  </p>
  <div class="section-title">Closing a session tab</div>
  <p class="hint-line">
    Closing a tab (×, ⌘W, sidebar ×) ends the session — the same as Archive or Delete from its
    menu. Choose what happens, or be asked each time.
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
      <span>Always delete — stop it and remove its history</span>
    </label>
  </div>

  <div class="section-title">Database Explorer</div>
  <label class="row num-row">
    <span>Switch to Vertical view when a result has more than</span>
    <input
      type="number"
      class="input num-input mono"
      min="0"
      max="500"
      step="1"
      value={ui.dbAutoVerticalCols}
      oninput={(e) => {
        // A cleared box is mid-edit, not "0" — leave the setting until a number lands.
        if (e.currentTarget.value !== '') ui.setDbAutoVerticalCols(Number(e.currentTarget.value));
      }}
      aria-label="Auto-Vertical column threshold"
    />
    <span>columns</span>
  </label>
  <p class="hint-line">
    0 = never. Applies unless you picked a view for that tab (the Grid / Vertical / JSON switch
    or ⇧⌘V); MongoDB results open in Vertical by default. Saved per device.
  </p>

  <div class="section-title">Sidebar</div>
  <p class="hint-line">
    Show, hide and reorder the items in the left sidebar — keep only what you use. Hidden items
    can be brought back here anytime. You can also reorder by dragging directly in the sidebar
    (“Customize sidebar” at the bottom of the expanded sidebar). Saved per device.
  </p>
  <div class="sidebar-list" data-testid="settings-sidebar-list">
    <!-- Section by section, as the sidebar shows them; moves stay in-section. -->
    {#each groupModules(sidebarResolved) as sec (sec.group.id)}
      <div class="sidebar-group-label">{sec.group.label}</div>
      {#each sec.modules as m, i (m.id)}
        <div class="sidebar-row" class:row-hidden={ui.sidebarHidden.includes(m.id)}>
          <Icon name={m.icon} size={14} />
          <span class="grow">{m.label}</span>
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
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: 12px;
    max-width: min(620px, 92vw);
  }
  .theme-card {
    text-align: start;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 10px;
    cursor: pointer;
    transition: border-color 130ms ease-out, transform 130ms ease-out;
  }
  .theme-card:hover {
    transform: translateY(-1px);
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
  .reduce-row {
    margin-top: 12px;
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
    font-size: 12.5px;
    font-weight: 600;
  }
  .theme-desc {
    font-size: 11px;
    color: var(--text-dim);
  }
  .hint-line {
    font-size: 11.5px;
    color: var(--text-dim);
    margin: 8px 0 0;
    max-width: min(620px, 92vw);
  }
  .hint-line.warn {
    color: var(--status-exited);
  }
  .switch-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    color: var(--text);
    cursor: pointer;
    user-select: none;
  }
  .radio-col {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-bottom: 10px;
  }
  .switch-row input {
    width: 15px;
    height: 15px;
    accent-color: var(--accent);
    cursor: pointer;
  }
  .exp-tag {
    font-size: 9.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-weight: 700;
    color: var(--accent-text);
    border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
    border-radius: 999px;
    padding: 1px 6px;
    margin-inline-start: 6px;
    vertical-align: middle;
  }
  .num-row {
    font-size: 12.5px;
    color: var(--text);
    margin-top: 8px;
  }
  .num-input {
    width: 64px;
    text-align: end;
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
    font-size: 12.5px;
    color: var(--text);
  }
  .sidebar-row:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .sidebar-group-label {
    padding: 8px 8px 2px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--text-dim);
  }
  .sidebar-group-label:first-child {
    padding-top: 2px;
  }
  .sidebar-row .grow {
    flex: 1;
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
    width: 15px;
    height: 15px;
    accent-color: var(--accent);
    cursor: pointer;
  }
</style>
