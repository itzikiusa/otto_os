<script lang="ts">
  import { vault } from './vault.svelte';
  import { assetPath, restoreVaultRevision, restoreVaultTrash, vaultHistory, vaultRevision, vaultTrash } from '../../lib/api/vault';
  import { ApiError, authedBlobUrl } from '../../lib/api/client';
  import type { VaultRevision, VaultRevisionDetail, VaultTrashEntry } from '../../lib/api/types';
  import DiffView from '../../lib/components/DiffView.svelte';
  import { toasts } from '../../lib/toast.svelte';

  let {mode}: {mode: 'trash' | 'history'} = $props();
  let trash = $state<VaultTrashEntry[]>([]);
  let revisions = $state<VaultRevision[]>([]);
  let selected = $state<VaultRevisionDetail | null>(null);
  let currentHash = $state<string | null>(null);
  let busy = $state(false);
  let more = $state(false);
  let loading = $state(false);
  let error = $state('');
  let destination = $state<Record<string, string>>({});
  let seq = 0;
  let detailSeq = 0;
  const visibleRevisions = $derived(revisions.filter(r => !vault.historySince || Date.parse(r.created_at) >= Date.parse(vault.historySince)));

  async function load(id: number, path: string) {
    const request = ++seq, workspace = vault.wsId;
    loading = true; error = ''; selected = null; detailSeq++;
    trash = []; revisions = []; more = false;
    try {
      if (mode === 'trash') {
        const data = await vaultTrash(workspace, id);
        if (request === seq) trash = data;
      } else {
        const data = await vaultHistory(workspace, id, path);
        if (request === seq) {revisions = data; more = data.length === 200;}
      }
    } catch (e) { if (request === seq) error = String(e); }
    finally { if (request === seq) loading = false; }
  }
  $effect(() => {
    const id = vault.current?.id, path = vault.historyPath;
    void vault.wsId; void mode;
    if (id) void load(id, path);
    else { seq++; detailSeq++; trash = []; revisions = []; selected = null; loading = false; }
  });

  async function loadOlder() {
    if (!vault.current || !revisions.length || loading) return;
    const id = vault.current.id, request = seq;
    loading = true;
    try {
      const data = await vaultHistory(vault.wsId, id, vault.historyPath, revisions[revisions.length - 1].id);
      if (request === seq) { revisions = [...revisions, ...data]; more = data.length === 200; }
    } catch (e) { if (request === seq) error = String(e); }
    finally { if (request === seq) loading = false; }
  }

  async function selectRevision(entry: VaultRevision) {
    if (!vault.current) return;
    const request = ++detailSeq, id = vault.current.id, workspace = vault.wsId;
    error = ''; selected = null; currentHash = null;
    try {
      const detail = await vaultRevision(workspace, id, entry.id);
      let hash = '';
      try {
        const url = await authedBlobUrl(assetPath(workspace, id, entry.path));
        try {
          const bytes = await (await fetch(url)).arrayBuffer();
          const digest = await crypto.subtle.digest('SHA-256', bytes);
          hash = [...new Uint8Array(digest)].map(b => b.toString(16).padStart(2, '0')).join('');
        } finally { URL.revokeObjectURL(url); }
      } catch (e) { if (!(e instanceof ApiError && e.status === 404)) throw e; }
      if (request !== detailSeq || vault.current?.id !== id || vault.wsId !== workspace) return;
      selected = detail; currentHash = hash;
    } catch (e) { if (request === detailSeq) error = String(e); }
  }

  async function restoreTrash(entry: VaultTrashEntry) {
    if (!vault.current || busy) return;
    const id = vault.current.id, workspace = vault.wsId, request = seq;
    busy = true; error = '';
    try {
      const result = await restoreVaultTrash(workspace, id, entry.id, destination[entry.id]?.trim() || undefined);
      toasts.success(`Restored ${result.path}`);
      if (request !== seq || vault.current?.id !== id || vault.wsId !== workspace) return;
      await load(id, vault.historyPath);
      await vault.refreshTree();
      await vault.refreshStatus();
    } catch (e) { if (request === seq) error = String(e); }
    finally { busy = false; }
  }

  async function restoreRevision(version: 'before' | 'after') {
    if (!vault.current || !selected || currentHash === null || busy) return;
    const entry = selected, id = vault.current.id, workspace = vault.wsId, request = seq;
    busy = true; error = '';
    try {
      await restoreVaultRevision(workspace, id, entry.id, version, currentHash);
      toasts.success(`Restored ${version} version of ${entry.path}`);
      if (request !== seq || vault.current?.id !== id || vault.wsId !== workspace) return;
      await load(id, vault.historyPath);
      await vault.refreshTree();
      await vault.refreshOpenNote();
    } catch (e) { if (request === seq) error = String(e); }
    finally { busy = false; }
  }
</script>

<section class="recovery" aria-label={mode === 'trash' ? 'Vault trash' : 'Vault edit history'}>
  <header><h2>{mode === 'trash' ? 'Trash' : 'Edit history'}</h2><button disabled={loading} onclick={() => vault.current && void load(vault.current.id, vault.historyPath)}>Refresh</button></header>
  {#if error}<p role="alert" class="error">{error}</p>{/if}
  {#if mode === 'trash'}
    <p>Restore to the original path or enter a new destination. Existing files are never replaced.</p>
    {#if loading}<p>Loading trash…</p>{:else if trash.length === 0}<p>No deleted items.</p>{/if}
    {#each trash as entry (entry.id)}
      <article class="trash-entry">
        <div><strong>{entry.original_path}</strong><small>{new Date(entry.deleted_at).toLocaleString()} · {entry.kind}</small></div>
        <label>Restore path<input aria-label={`Restore path for ${entry.original_path}`} placeholder={entry.original_path} bind:value={destination[entry.id]} /></label>
        <button disabled={busy} onclick={() => void restoreTrash(entry)}>Restore</button>
      </article>
    {/each}
  {:else}
    <p>Review saved before/after versions of editor and agent writes. Restoring creates another recoverable edit.</p>
    <label>File path <input aria-label="History file path" bind:value={vault.historyPath} placeholder="All files" /></label>
    {#if vault.historySince}<p>Changes since {new Date(vault.historySince).toLocaleString()} <button onclick={() => {vault.historySince = ''; vault.persistView();}}>Show all dates</button></p>{/if}
    <div class="history-layout">
      <nav aria-label="Saved revisions">
        {#if loading}<p>Loading history…</p>{:else if visibleRevisions.length === 0}<p>No recorded edits. History begins with writes made after this feature was installed.</p>{/if}
        {#each visibleRevisions as entry (entry.id)}
          <button class:active={selected?.id === entry.id} onclick={() => void selectRevision(entry)}>
            <strong>{entry.path}</strong><small>{new Date(entry.created_at).toLocaleString()} · {entry.reason}{entry.committed ? '' : ' · write not confirmed'}</small>
          </button>
        {/each}
        {#if more}<button disabled={loading} onclick={() => void loadOlder()}>Load older edits</button>{/if}
      </nav>
      <div class="revision-detail">
        {#if selected}
          <h3>{selected.path}</h3>
          <div class="actions"><button disabled={busy || selected.before === null || currentHash === null} onclick={() => void restoreRevision('before')}>Restore before version</button><button disabled={busy || currentHash === null} onclick={() => void restoreRevision('after')}>Restore after version</button></div>
          {#if (selected.before?.length ?? 0) + selected.after.length <= 500_000}
            <DiffView before={selected.before ?? ''} after={selected.after} mode="line" contextLines={4} />
          {:else}
            <p>This revision is large. Full versions are available below; restore always uses the complete content.</p>
            <details><summary>Before version</summary><pre>{selected.before ?? '(File did not exist)'}</pre></details>
            <details><summary>After version</summary><pre>{selected.after}</pre></details>
          {/if}
        {:else}<p>Select an edit to compare its versions.</p>{/if}
      </div>
    </div>
  {/if}
</section>
<style>
  .recovery { padding: 18px; overflow: auto; min-height: 0; height: 100%; color: var(--text); }
  header, .actions { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; } header { justify-content: space-between; }
  h2 { margin: 0; font-size: 18px; } h3 { overflow-wrap: anywhere; font-size: 14px; } p, small, label { font-size: 12px; color: var(--text-dim); }
  button, input { padding: 7px 9px; border: 1px solid var(--border); background: var(--bg); color: var(--text); border-radius: 5px; }
  button { cursor: pointer; } button:disabled { opacity: .5; cursor: default; } input { min-width: 0; max-width: 100%; box-sizing: border-box; }
  .trash-entry { display: flex; align-items: center; gap: 12px; padding: 12px 0; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
  .trash-entry > div { flex: 1; min-width: 180px; overflow-wrap: anywhere; } small { display: block; margin-top: 5px; }
  label { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .history-layout { display: grid; grid-template-columns: minmax(190px, 30%) minmax(0, 1fr); gap: 16px; margin-top: 14px; }
  nav { display: flex; flex-direction: column; gap: 6px; } nav button { text-align: start; overflow-wrap: anywhere; }
  nav button.active { border-color: var(--accent); } .error { color: var(--status-exited); }
  .revision-detail { min-width: 0; overflow: auto; } .actions { margin-bottom: 12px; }
  pre { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 500px; overflow: auto; font-size: 12px; }
  @media (max-width: 700px) { .history-layout { grid-template-columns: minmax(0, 1fr); } nav { max-height: 240px; overflow: auto; } }
</style>
