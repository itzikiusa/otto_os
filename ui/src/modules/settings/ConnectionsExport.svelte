<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { ConnectionExportFormat, ConnectionExportResult } from '../../lib/api/types';
  import { downloadText } from '../../lib/components/exporters';

  let formats: ConnectionExportFormat[] = $state([]);
  let workspaces: { id: string; name: string }[] = $state([]);
  let format = $state('json');
  let scope: 'all' | 'workspaces' = $state('all');
  let selectedIds: string[] = $state([]);
  let includePasswords = $state(false);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let result: ConnectionExportResult | null = $state(null);
  let generation = 0;
  const selectedFormat = $derived(formats.find(item => item.id === format));
  onMount(() => { void load(); });
  onDestroy(() => { generation++; result = null; });

  async function load() {
    const seq = generation;
    loading = true; error = '';
    try {
      const [available, spaces] = await Promise.all([
        api.get<{ formats: ConnectionExportFormat[] }>('/state/connections/export/formats'),
        api.get<{ id: string; name: string }[]>('/workspaces'),
      ]);
      if (seq !== generation) return;
      formats = available.formats; workspaces = spaces;
      if (!formats.some(item => item.id === format)) format = formats[0]?.id ?? '';
    } catch (e) { if (seq === generation) error = e instanceof Error ? e.message : String(e); }
    finally { if (seq === generation) loading = false; }
  }
  function clearResult() { result = null; error = ''; }
  function toggleWorkspace(id: string) { selectedIds = selectedIds.includes(id) ? selectedIds.filter(value => value !== id) : [...selectedIds, id]; clearResult(); }
  async function prepare() {
    const seq = generation;
    busy = true; result = null; error = '';
    try {
      const exported = await api.post<ConnectionExportResult>('/state/connections/export', {
        format, scope, workspace_ids: scope === 'workspaces' ? selectedIds : undefined,
        include_passwords: includePasswords,
      });
      if (seq === generation) result = exported;
    } catch (e) { if (seq === generation) error = e instanceof Error ? e.message : String(e); }
    finally { if (seq === generation) busy = false; }
  }
</script>

<section class="connections-export" aria-label="Connection export">
  <h2 class="card-title">Export connections</h2>
  <p>Export connection profiles across Otto, including database and SSH connections. Use JSON or CSV
    for all connection types, or choose a format supported by your database application.</p>
  {#if loading}<p class="dim" role="status">Loading export formats…</p>{:else if formats.length}
    <div class="controls">
      <label class="fld">Export format<select class="input" aria-label="Export format" bind:value={format} disabled={busy} onchange={clearResult}>{#each formats as item}<option value={item.id}>{item.label}</option>{/each}</select></label>
      <label class="fld">Connections to export<select class="input" bind:value={scope} disabled={busy} onchange={clearResult}><option value="all">All workspaces</option><option value="workspaces">Selected workspaces</option></select></label>
    </div>
    {#if scope === 'workspaces'}
      <p class="dim">Global connections are included with the selected workspaces.</p>
      <div class="workspaces" role="group" aria-label="Export workspaces">{#each workspaces as workspace}<label><input type="checkbox" checked={selectedIds.includes(workspace.id)} disabled={busy} onchange={() => toggleWorkspace(workspace.id)} />{workspace.name}</label>{:else}<p>No workspaces available.</p>{/each}</div>
    {/if}
    {#if selectedFormat}<p class="dim">{selectedFormat.description}</p>{/if}
    <label class="password-option"><input type="checkbox" bind:checked={includePasswords} disabled={busy} onchange={clearResult} />Include saved passwords and credentials</label>
    {#if includePasswords}
      <p class="password-note">These files will contain readable credentials. Keep them private and out of Git.</p>
      {#if selectedFormat?.password_support === 'sidecar'}<p>The target format does not carry passwords. Credentials are provided in a separate file.</p>{/if}
    {:else}<p class="dim">Passwords are excluded. Re-enter credentials in the target application after import.</p>{/if}
    <button class="btn" disabled={busy || !format || (scope === 'workspaces' && selectedIds.length === 0)} onclick={prepare}>{busy ? 'Preparing export…' : includePasswords ? 'Prepare export with passwords' : 'Prepare export'}</button>
  {/if}
  {#if result}
    <div class="result" aria-label="Prepared connection export">
      <p role="status">{result.exported_connections} of {result.total_connections} connections exported{result.contains_passwords ? ' with credentials' : ' without passwords'}.</p>
      <div class="controls">{#each result.files as file}<button class="btn" onclick={() => downloadText(file.content, file.name, file.mime)}>Download {file.name}</button>{/each}</div>
      <p class="instructions">{result.import_instructions}</p>
      {#if result.warnings.length}<ul>{#each result.warnings as warning}<li>{warning}</li>{/each}</ul>{/if}
      {#if result.skipped.length}<details><summary>Connections not included ({result.skipped.length})</summary><ul>{#each result.skipped as skipped}<li>{skipped.name} ({skipped.kind}): {skipped.reason}</li>{/each}</ul></details>{/if}
      <button class="btn ghost" onclick={() => result = null}>Clear prepared export</button>
    </div>
  {/if}
  {#if error}<p class="error" role="alert"><strong>{formats.length ? 'Couldn’t prepare the export.' : 'Couldn’t load export formats.'}</strong> {error}</p>{#if !formats.length}<button class="btn small" onclick={load}>Retry</button>{/if}{/if}
</section>

<style>
  .connections-export { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-m); box-shadow: var(--shadow-card); padding: 16px 18px; margin: 0 0 16px; max-width: var(--settings-col); }
  .card-title { margin: 0 0 6px; font-size: var(--fs-m); font-weight: 600; }
  p { margin: 0 0 8px; font-size: var(--fs-s); line-height: 1.5; }
  .controls { display: flex; flex-wrap: wrap; gap: 8px 12px; margin: 12px 0; }
  .controls label.fld { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-s); font-weight: 500; color: var(--text-dim); max-width: 100%; }
  select { max-width: 100%; }
  .dim { color: var(--text-dim); }
  .password-option, .workspaces label { display: flex; align-items: center; gap: 7px; font-size: var(--fs-m); margin: 0 0 6px; }
  .password-note { color: var(--warning); }
  .workspaces { display: grid; gap: 6px; max-height: 220px; overflow: auto; margin: 12px 0; }
  .result { border-top: 1px solid var(--border); margin-top: 16px; padding-top: 12px; }
  .instructions { white-space: pre-line; }
  details, ul { font-size: var(--fs-s); }
  summary { cursor: pointer; }
  ul { max-height: 230px; overflow: auto; overflow-wrap: anywhere; }
  li { margin: 4px 0; }
  .error { color: var(--danger); overflow-wrap: anywhere; }
</style>
