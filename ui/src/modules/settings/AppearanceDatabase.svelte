<script lang="ts">
  // Settings › Appearance › Database Explorer (lazy: below the fold).
  import SettingToggle from './SettingToggle.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { AUTO_VERTICAL_ENGINES } from '../../lib/db-view-prefs';
  import { dbPrefs } from '../../lib/stores/dbPrefs.svelte';
  import { clipHistory } from '../../lib/stores/clipHistory.svelte';
</script>

<div class="section-title">Database Explorer</div>
<p class="hint-line">
  Open wide results in the Vertical view (one record per block) instead of the grid. Set per
  engine: on for MongoDB, whose documents are nested; off for the SQL engines, where a wide
  table is what the grid is for. MongoDB results open in Vertical anyway until you pick Grid
  or JSON on one of the connection’s tabs — this threshold then still sends wide ones back to
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
            // a number that isn’t in effect).
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
<SettingToggle
  label="Connect restored tabs on click only"
  hint="After a reload only the active connection connects; the other tabs wait, marked “Not connected yet”, until you open one. Off: they connect in the background, three at a time."
  checked={dbPrefs.warmRestored === 'on-click'}
  onchange={(on) => dbPrefs.setWarmRestored(on ? 'on-click' : 'background')}
  testid="db-warm-on-click"
/>
<SettingToggle
  label="Keep open connections alive"
  hint="Pings each open, connected database every 4 minutes so its pool and SSH tunnel don’t close from inactivity, and a dropped one shows red before your next query."
  checked={dbPrefs.keepAlive}
  onchange={(on) => dbPrefs.setKeepAlive(on)}
  testid="db-keep-alive"
/>
<SettingToggle
  label="Clipboard history"
  hint="Remember the last 50 copies made in Otto (result cells, the query editor) so ⌥⌘V in the query editor can paste them again. Only copies made inside Otto, never from masked tabs. Off clears it."
  checked={clipHistory.enabled}
  onchange={(on) => clipHistory.setEnabled(on)}
  testid="clip-history-enabled"
/>

<style>
  .hint-line {
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text-dim);
    margin: 8px 0 0;
    max-width: var(--settings-col);
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
  .switch-row input {
    width: 15px;
    height: 15px;
    accent-color: var(--accent);
    cursor: pointer;
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
</style>
