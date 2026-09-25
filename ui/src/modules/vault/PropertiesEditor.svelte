<script lang="ts">
  import { vault } from './vault.svelte';
  import { PROPERTY_FIELDS, patchProperties, readProperties } from './properties';
  let editing = $state(false);
  let base = $state('');
  let values = $state<Record<string, string>>({});
  let error = $state('');
  const preview = $derived.by(() => {
    if (!editing) return '';
    try { return patchProperties(base, values); } catch { return ''; }
  });
  function begin() {
    error = '';
    try {
      base = vault.draft;
      values = readProperties(base);
      editing = true;
    } catch (e) { error = String(e); }
  }
  async function apply() {
    if (vault.draft !== base) { error = 'This note changed. Cancel and reopen properties to use the latest version.'; return; }
    if (!preview) return;
    vault.onDraftChange(preview);
    if (await vault.saveNow()) editing = false;
    else error = 'Properties remain in your draft. Resolve the save error or conflict before leaving.';
  }
</script>
{#if error}<p class="error" role="alert">{error}</p>{/if}
{#if editing}
  <form onsubmit={(e) => {e.preventDefault(); void apply();}}>
    {#each PROPERTY_FIELDS as key (key)}
      <label>{key}<input aria-label={`Property ${key}`} bind:value={values[key]} placeholder={key === 'tags' || key === 'aliases' ? 'Comma-separated values' : ''} /></label>
    {/each}
    <details><summary>Preview source</summary><pre>{preview}</pre></details>
    <div class="actions"><button type="submit" disabled={vault.saving || vault.conflict}>Save properties</button><button type="button" onclick={() => {editing = false; error = '';}}>Cancel</button></div>
  </form>
{:else}
  <button class="edit" disabled={!vault.note || vault.note.meta.parse_error} onclick={begin}>Edit properties</button>
{/if}
<style>
  form { display: grid; gap: 8px; padding: 6px; }
  label { display: grid; gap: 3px; font-size: var(--fs-s); color: var(--text-dim); }
  input { width: 100%; min-width: 0; box-sizing: border-box; background: var(--bg); color: var(--text); border: 1px solid var(--border); border-radius: 4px; padding: 6px; }
  button { background: var(--bg); color: var(--text); border: 1px solid var(--border); border-radius: 4px; padding: 5px 8px; cursor: pointer; }
  .actions { display: flex; gap: 6px; flex-wrap: wrap; } .edit { margin: 5px; }
  .error { color: var(--status-exited); font-size: var(--fs-s); }
  pre { max-height: 250px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; font-size: var(--fs-xs); }
  summary { cursor: pointer; font-size: var(--fs-s); }
</style>
