<script lang="ts">
  import { toastError } from '../../lib/toastError';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  import SettingToggle from './SettingToggle.svelte';
  import AppearanceLightMode from './AppearanceLightMode.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  // Theme (native / pro-dark / warm), scheme (auto / light / dark), accent.
  import { ui, TERM_FONT_OPTIONS, type SchemePref, type ThemeName, type Direction } from '../../lib/stores/ui.svelte';
  import { agentEmoji } from '../../lib/stores/agentEmoji.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { radioKey } from '../../lib/radioKey';
  import { toasts } from '../../lib/toast.svelte';
  import { AMBIENT_MODES, ambientImage, type AmbientMode } from '../../lib/ambient';
  import { processWallpaper } from '../../lib/wallpaper';
  import { barStore } from '../../lib/stores/bar.svelte';
  import type { BarPref } from '../../lib/floatingBar';

  // Below the fold: their own chunks, so the default Settings page paints light.
  const DbSection = lazyComponent(() => import('./AppearanceDatabase.svelte'));
  const SidebarSection = lazyComponent(() => import('./AppearanceSidebar.svelte'));

  // The "Type or speak… ⌘K" bar over the content column (FloatingBar.svelte).
  const barPrefs: { id: BarPref; label: string; hint: string }[] = [
    { id: 'auto', label: 'Auto', hint: 'A short pill at rest, full on Home; it docks into the status bar while you scroll or type in a terminal or editor.' },
    { id: 'pinned', label: 'Always full', hint: 'The whole pill (model, spaces) stays up; it only shrinks while a terminal or editor has the keyboard.' },
    { id: 'docked', label: 'Docked', hint: 'A small chip in the status bar that never covers content; ⌘K opens the full bar.' },
    { id: 'hidden', label: 'Hidden', hint: 'No bar; ⌘K opens the command palette sheet instead.' },
  ];

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
      else toasts.error('Couldn’t store the photo', 'This browser refused the storage — try a smaller image.');
    } catch (err) {
      toastError('Couldn’t use that image', err);
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
        onclick={() => ui.setScheme(s.id)}>{s.label}</button>
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
  <AppearanceLightMode />

  <div class="section-title">Accent color</div>
  <div class="row">
    <input
      type="color"
      class="accent-input"
      value={ui.accent || (/^#[0-9a-f]{6}$/i.test(accentNow) ? accentNow : '#0a84ff')}
      oninput={(e) => ui.setAccent(e.currentTarget.value)}
      aria-label="Accent color"
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
        role="radio" aria-checked={ui.ambient === m.id} tabindex={ui.ambient === m.id ? 0 : -1} onkeydown={radioKey}
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
  <!-- Same store setters as the session header’s terminal controls, so the
       two can’t drift; also the only way back when those controls are hidden. -->
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
      label="Use Option as Meta key"
      hint="⌥ sends Meta, so ⌥← / ⌥→ jump words and ⌥⌫ deletes a word in shells and agents. Turn it off if your keyboard layout types characters such as @, braces, brackets, | or accents with ⌥."
      checked={ui.termOptionAsMeta}
      onchange={(v) => ui.setTermOptionAsMeta(v)}
    />
    <SettingToggle
      label="Screen reader support"
      hint="Announces terminal output to VoiceOver and lets you read the screen line by line. Uses the slower non-GPU renderer; toggling reloads open terminals."
      checked={ui.termScreenReader}
      onchange={(v) => ui.setTermScreenReader(v)}
    />
    <SettingToggle
      label="Terminal toolbar"
      hint="Show the font-size and copy-on-select controls on each terminal."
      checked={ui.termToolbar}
      onchange={(v) => ui.setTermToolbar(v)}
    />
  </div>

  <div class="section-title">Right-to-left text</div>
  <p class="hint-line">Bidirectional (BiDi) text layout works across Threads, Chat, Markdown, Rooms, and Terminals — each prose block aligns to its language while code, paths, and URLs stay isolated left-to-right.</p>
  <div class="toggle-block">
    <SettingToggle
      label="Right-to-left text in the terminal"
      checked={ui.rtlBidi}
      onchange={(v) => ui.setRtlBidi(v)}
    >
      Lays out Hebrew and Arabic right-to-left with English embedded left-to-right, using the
      browser’s bidi engine (it switches the terminal off the GPU renderer). Text is reflowed for
      reading, so the monospace grid no longer lines up exactly: good for chat-style output,
      imperfect for TUI tables or box art. Toggling reloads open terminals.
    </SettingToggle>
  </div>

  <div class="section-title">Agent replies</div>
  <div class="toggle-block">
    <SettingToggle label="Show emoji in agent replies" hint="Shows :rocket: as 🚀 in agent replies and pull-request comments. Off shows shortcodes exactly as written. Your own messages are unchanged." checked={agentEmoji.on} onchange={(v) => agentEmoji.set(v)} testid="agent-emoji-toggle" />
  </div>

  <div class="section-title">Floating bar</div>
  <div class="segmented" role="radiogroup" aria-label="Floating bar">
    {#each barPrefs as b (b.id)}
      <button
        role="radio" aria-checked={barStore.pref === b.id} tabindex={barStore.pref === b.id ? 0 : -1} onkeydown={radioKey}
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
    "Always delete" also skips the confirm on a session’s Delete command. Closing or deleting
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
      <span>Always delete — stop it and remove its history for good (can’t be undone)</span>
    </label>
  </div>

  <LazyMount lazy={DbSection} what="the Database Explorer settings" variant="compact" />
  <LazyMount lazy={SidebarSection} what="the sidebar settings" variant="compact" />
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
    transition: border-color var(--dur-fast) ease-out;
  }
  .theme-card:hover:not(.selected) {
    border-color: var(--border-strong);
  }
  .theme-card.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
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
    outline: 2px solid color-mix(in srgb, var(--accent-text) 70%, transparent);
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
    border-radius: var(--radius-s);
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
  .accent-input {
    width: 36px;
    height: 27px;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    cursor: pointer;
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
</style>
