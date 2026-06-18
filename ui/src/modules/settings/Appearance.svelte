<script lang="ts">
  // Theme (native / pro-dark / warm), scheme (auto / light / dark), accent.
  import {
    ui,
    TERM_FONT_OPTIONS,
    type SchemePref,
    type ThemeName,
  } from '../../lib/stores/ui.svelte';

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

  const swatches: Record<ThemeName, { bg: string; fg: string; acc: string }> = {
    native: { bg: '#1e1e23', fg: '#f2f2f5', acc: '#0a84ff' },
    'pro-dark': { bg: '#16161c', fg: '#e8e8ee', acc: '#6c5ce7' },
    warm: { bg: '#211f1b', fg: '#e8e4da', acc: '#2bb673' },
  };
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>Appearance</h1>
      <div class="sub">Themes apply instantly and persist per device.</div>
    </div>
  </div>

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
</div>

<style>
  .theme-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: 12px;
    max-width: 620px;
  }
  .theme-card {
    text-align: left;
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
    max-width: 620px;
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
    color: var(--accent);
    border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
    border-radius: 999px;
    padding: 1px 6px;
    margin-left: 6px;
    vertical-align: middle;
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
</style>
