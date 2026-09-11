<script lang="ts">
  // Run-time prompt for query variables that have no value yet (`:name` /
  // `{name}` / `{{name}}`): one row per missing name with the same value +
  // type + escape controls as the editor's variables bar. Enter (or Run)
  // hands every row back through `onsubmit`; the caller persists them with
  // `database.setVar` and re-runs. Opened by Run when a value is missing and
  // when a saved query containing placeholders is opened.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { defaultVarSpec, type VarSpec } from './sql-util';

  interface Props {
    /** The names still lacking a value, in statement order. */
    names: string[];
    /** The tab's current specs (type / escape are kept even when the value is empty). */
    vars: Record<string, VarSpec>;
    onsubmit: (vals: Record<string, VarSpec>) => void;
    oncancel: () => void;
  }
  let { names, vars, onsubmit, oncancel }: Props = $props();

  // Local drafts seeded ONCE from the tab's specs (the owner mounts a fresh
  // prompt per run, so the initial props are the whole story) — nothing is
  // written back until Run.
  let drafts = $state<Record<string, VarSpec>>(
    untrack(() => Object.fromEntries(names.map((n) => [n, { ...(vars[n] ?? defaultVarSpec()) }]))),
  );
  const missing = $derived(names.filter((n) => !(drafts[n]?.value ?? '').trim()));

  function submit(): void {
    if (missing.length > 0) return;
    onsubmit(drafts);
  }
  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      submit();
    }
  }
</script>

<Modal title="Query variables" width={480} onclose={oncancel}>
  <div class="vp" data-testid="vars-prompt">
    <p class="vp-hint">
      This query references {names.length === 1 ? 'a variable' : `${names.length} variables`} without a
      value. Fill them in to run — values are kept on the tab for next time.
    </p>
    {#each names as name (name)}
      <div class="vp-row">
        <span class="vp-name mono" title={name}>{name}</span>
        <input
          class="input vp-input"
          bind:value={drafts[name].value}
          placeholder={drafts[name].type === 'number' ? '123' : 'value'}
          spellcheck="false"
          onkeydown={onKeydown}
          aria-label="Value for {name}"
        />
        <select class="input vp-type" bind:value={drafts[name].type} title="How to substitute: string (quoted), number (raw), or raw (verbatim)" aria-label="Type for {name}">
          <option value="string">string</option>
          <option value="number">number</option>
          <option value="raw">raw</option>
        </select>
        {#if drafts[name].type === 'string'}
          <label class="vp-esc" title="Escape quotes inside the value">
            <input type="checkbox" bind:checked={drafts[name].escape} />
            esc
          </label>
        {/if}
      </div>
    {/each}
  </div>

  {#snippet footer()}
    <button class="btn" onclick={oncancel}>Cancel</button>
    <button class="btn primary" disabled={missing.length > 0} onclick={submit} title={missing.length > 0 ? `Still missing: ${missing.join(', ')}` : 'Run with these values (Enter)'}>Run</button>
  {/snippet}
</Modal>

<style>
  .vp {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .vp-hint {
    margin: 0 0 4px;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-dim);
  }
  .vp-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .vp-name {
    flex: 0 0 110px;
    font-size: 12px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .vp-input {
    flex: 1;
    min-width: 0;
  }
  .vp-type {
    flex: 0 0 82px;
  }
  .vp-esc {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text-dim);
  }
</style>
