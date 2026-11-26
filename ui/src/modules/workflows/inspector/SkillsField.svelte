<script lang="ts">
  // An agent step's skills: chips, and "Add skill…" — a filterable list of the
  // skills installed for the step's provider in the shared menu (clamped into
  // the viewport, its own scroll). A skill that isn't installed is a danger
  // chip that says so; saving and importing check the same (no silent skip).
  import Icon from '../../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import type { InspectorOptions } from './options.svelte';
  import { providerName } from '../editor/nodeSummary';
  import { missingSkills } from '../editor/stepRules';

  interface Props {
    skills: string[];
    provider: string;
    pickers: InspectorOptions;
    onchange: (skills: string[] | undefined) => void;
  }
  let { skills, provider, pickers, onchange }: Props = $props();

  $effect(() => pickers.skills.ensure(provider));
  const installed = $derived(pickers.skills.rows(provider));
  const missing = $derived(new Set(missingSkills(skills, installed ? new Set(installed.map((s) => s.name)) : null)));

  function add(e: MouseEvent): void {
    const rows = (installed ?? []).filter((s) => !skills.includes(s.name));
    ctxMenu.showAt(e.currentTarget as HTMLElement, rows.length
      ? rows.map((s) => ({ label: s.name, hint: s.category || undefined, title: s.description, action: () => onchange([...skills, s.name]) }))
      : [{ label: pickers.skills.loading(provider) ? 'Loading skills…' : `No other skills installed for ${providerName(provider)}`, disabled: true }],
      { filter: rows.length > 8, filterPlaceholder: 'Find a skill…' });
  }
</script>

<div class="sk">
  {#each skills as s (s)}
    <span class="sk-chip" class:missing={missing.has(s)} title={missing.has(s) ? `Not installed for ${providerName(provider)}` : s}>
      {s}
      <button type="button" class="sk-x" aria-label="Remove {s}" title="Remove {s}" onclick={() => onchange(skills.filter((x) => x !== s).length ? skills.filter((x) => x !== s) : undefined)}>
        <Icon name="x" size={12} />
      </button>
    </span>
  {/each}
  <button type="button" class="sk-chip sk-add" aria-haspopup="menu" onclick={add}><Icon name="plus" size={12} />Add skill…</button>
</div>
{#each [...missing] as s (s)}
  <p class="sk-err">{s} isn’t installed for {providerName(provider)} — install it in Settings → Skills, or remove it.</p>
{/each}

<style>
  .sk {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .sk-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
  }
  .sk-chip.missing {
    border-color: var(--danger);
    background: var(--danger-soft);
    color: var(--danger);
  }
  .sk-add {
    cursor: pointer;
    color: var(--text-dim);
    background: transparent;
  }
  .sk-add:hover {
    color: var(--text);
  }
  .sk-x {
    display: inline-flex;
    border: none;
    background: none;
    padding: 0;
    color: inherit;
    cursor: pointer;
  }
  .sk-err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
</style>
