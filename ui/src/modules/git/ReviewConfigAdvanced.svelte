<script lang="ts">
  // The run settings of a review config that the agent rows above don't show —
  // execution mode, attempts, timeout and the PR-size thresholds. They used to
  // be settable only through the API / MCP / a preset, and a save from the
  // modal wiped them; now they are shown (defaults as placeholders) and kept.
  import { ADVANCED_DEFAULTS, type ReviewAdvanced } from './reviewConfigMerge';

  let { value = $bindable() }: { value: ReviewAdvanced } = $props();
  const id = $props.id();
  const set = (k: Exclude<keyof ReviewAdvanced, 'mode'>, v: number | null): void => {
    value = { ...value, [k]: v };
  };
  const num = (e: Event): number | null => {
    const v = (e.currentTarget as HTMLInputElement).valueAsNumber;
    return Number.isFinite(v) && v > 0 ? Math.round(v) : null;
  };
</script>

<details class="rca" data-testid="review-config-advanced">
  <summary>Advanced</summary>
  <p class="dim rca-note">Leave a field empty to use the default shown in it.</p>
  <div class="rca-grid">
    <label for="{id}-mode">Mode</label>
    <select id="{id}-mode" value={value.mode ?? ''}
      onchange={(e) => (value = { ...value, mode: ((e.currentTarget as HTMLSelectElement).value || null) as ReviewAdvanced['mode'] })}>
      <option value="">Default (fan-out)</option>
      <option value="fan_out">Fan-out — one session per lens</option>
      <option value="orchestrator">Orchestrator — one session per provider runs every lens</option>
    </select>
    <label for="{id}-att">Max attempts per agent</label>
    <input id="{id}-att" type="number" min="1" inputmode="numeric" placeholder={String(ADVANCED_DEFAULTS.max_attempts)}
      value={value.max_attempts ?? ''} onchange={(e) => set('max_attempts', num(e))} />
    <label for="{id}-to">Agent timeout (seconds)</label>
    <input id="{id}-to" type="number" min="1" inputmode="numeric" placeholder="Sized to the diff"
      value={value.timeout_secs ?? ''} onchange={(e) => set('timeout_secs', num(e))} />
    <label for="{id}-med">Medium PR above (changed lines)</label>
    <input id="{id}-med" type="number" min="1" inputmode="numeric" placeholder={String(ADVANCED_DEFAULTS.medium_pr_lines)}
      value={value.medium_pr_lines ?? ''} onchange={(e) => set('medium_pr_lines', num(e))} />
    <label for="{id}-lg">Large PR above (changed lines)</label>
    <input id="{id}-lg" type="number" min="1" inputmode="numeric" placeholder={String(ADVANCED_DEFAULTS.large_pr_lines)}
      value={value.large_pr_lines ?? ''} onchange={(e) => set('large_pr_lines', num(e))} />
  </div>
  <p class="dim rca-note">A large PR skips the scope assessment and runs every lens on every provider.</p>
</details>

<style>
  .rca { margin-block-start: 14px; font-size: var(--fs-s); }
  .rca summary { cursor: pointer; font-weight: 600; min-height: 28px; display: flex; align-items: center; }
  .rca-note { margin-block: 6px; font-size: var(--fs-xs); }
  .rca-grid { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 6px 10px; align-items: center; }
  .rca-grid label { color: var(--text-dim); font-size: var(--fs-xs); }
  .rca-grid input, .rca-grid select {
    min-width: 0; width: 100%; background: var(--surface-2); border: 1px solid var(--border);
    border-radius: var(--radius-s); color: var(--text); font-size: var(--fs-s); padding: 4px 8px;
  }
  @media (max-width: 640px) {
    .rca-grid { grid-template-columns: minmax(0, 1fr); }
    .rca-grid input, .rca-grid select { min-height: var(--hit-min); font-size: 16px; } /* ui-guards: allow — 16px stops iOS zoom-on-focus */
  }
</style>
