<script lang="ts">
  import { personalAgentsApi } from '../../lib/api/personalAgents';
  import { listVaults } from '../../lib/api/vault';
  import type { PersonalAgentDocument, Vault } from '../../lib/api/types';
  import Markdown from '../agents/conversation/Markdown.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';

  interface Props {
    agentId: string;
    workspaceId: string;
    kind: 'memory' | 'context';
    editable?: boolean;
    sharedWorkspace?: boolean;
  }
  let { agentId, workspaceId, kind, editable = false, sharedWorkspace = false }: Props = $props();
  let document = $state<PersonalAgentDocument | null>(null);
  let draft = $state('');
  let editing = $state(false);
  let loading = $state(true);
  let saving = $state(false);
  let error = $state('');
  let picker = $state<string | null>(null);
  let vaults = $state<Vault[] | null>(null);
  let vaultId = $state('');
  let generation = 0;
  const label = $derived(kind === 'memory' ? 'Memory' : 'Context');

  async function load() {
    const seq = ++generation;
    loading = true;
    error = '';
    try {
      const result = await personalAgentsApi.document(agentId, kind);
      if (seq !== generation) return;
      document = result;
      draft = result.content;
      editing = false;
    } catch (e) {
      if (seq === generation) error = e instanceof Error ? e.message : String(e);
    } finally { if (seq === generation) loading = false; }
  }
  $effect(() => {
    agentId; kind;
    void load();
    return () => { generation++; };
  });
  async function save() {
    if (!document || !editable || saving) return;
    const seq = generation;
    saving = true;
    error = '';
    try {
      const result = await personalAgentsApi.saveDocument(agentId, kind, { content: draft, version: document.version });
      if (seq !== generation) return;
      document = result;
      draft = result.content;
      editing = false;
    } catch (e) {
      if (seq === generation) error = e instanceof Error ? e.message : String(e);
    } finally { if (seq === generation) saving = false; }
  }
  async function chooseVault() {
    try { vaults = await listVaults(workspaceId); vaultId = String(vaults[0]?.id ?? ''); }
    catch (e) { error = e instanceof Error ? e.message : String(e); }
  }
  function addReference(path: string) {
    draft += `${draft && !draft.endsWith('\n') ? '\n' : ''}\n- Reference: ${path}\n`;
    picker = null;
  }
</script>

<section class="document" data-testid="agent-document">
  <h2>{label}</h2>
  <p class="hint">{kind === 'memory'
    ? 'Notes the agent reads and updates between runs. Saving edits preserves the rest of your workspace.'
    : 'Background notes and references you maintain. New runs and new chats receive a snapshot; existing chats keep their current context.'}</p>
  {#if document?.path}<code class="path">{document.path}</code>{/if}
  {#if kind === 'memory' && sharedWorkspace}<p class="hint">Agents using this same working directory share this memory file.</p>{/if}
  {#if loading}<p role="status">Loading {label.toLowerCase()}…</p>
  {:else}
    {#if error}<div class="error" role="alert">{error}</div>{/if}
    {#if document}
      {#if editing}
        <textarea aria-label={`${label} Markdown`} bind:value={draft} disabled={saving} spellcheck="false"></textarea>
        {#if kind === 'context'}
          <div class="actions">
            <button type="button" class="btn" disabled={saving} onclick={() => (picker = '')}>Add file reference…</button>
            <button type="button" class="btn" disabled={saving} onclick={chooseVault}>Add Vault reference…</button>
          </div>
          {#if vaults}
            <div class="actions">
              <select aria-label="Vault" bind:value={vaultId}>{#each vaults as vault (vault.id)}<option value={String(vault.id)}>{vault.name}</option>{/each}</select>
              <button type="button" class="btn" disabled={!vaultId} onclick={() => (picker = vaults?.find((v) => String(v.id) === vaultId)?.root_path ?? '')}>Choose note…</button>
              {#if !vaults.length}<span class="hint">No Vaults in this workspace.</span>{/if}
            </div>
          {/if}
        {/if}
        <div class="actions">
          <button type="button" class="btn primary" disabled={saving} onclick={save}>{saving ? 'Saving…' : 'Save'}</button>
          <button type="button" class="btn" disabled={saving} onclick={() => { draft = document?.content ?? ''; editing = false; error = ''; }}>Cancel</button>
          {#if error}<button type="button" class="btn" disabled={saving} onclick={load}>Reload saved version</button>{/if}
        </div>
      {:else}
        {#if document.content}<Markdown md={document.content} />{:else}<p class="hint">No {label.toLowerCase()} yet.</p>{/if}
        <div class="actions">
          {#if editable}<button type="button" class="btn" onclick={() => (editing = true)}>Edit {label.toLowerCase()}</button>{/if}
          <button type="button" class="btn" onclick={load}>Reload</button>
        </div>
      {/if}
    {:else}<button type="button" class="btn" onclick={load}>Retry</button>{/if}
  {/if}
</section>
{#if picker !== null}<FolderPicker title="Choose context reference" start={picker} files onpick={addReference} onclose={() => (picker = null)} />{/if}
<style>
  .document { padding: 16px; border: 1px solid var(--border); border-radius: var(--radius-m); min-width: 0; }
  h2 { margin: 0 0 8px; font-size: 15px; }
  .hint { color: var(--text-dim); font-size: 12px; line-height: 1.5; }
  .path { display: block; overflow-wrap: anywhere; margin-bottom: 10px; }
  textarea { width: 100%; box-sizing: border-box; min-height: 280px; resize: vertical; padding: 10px; font: 12px var(--font-mono); color: var(--text); background: var(--surface-2); border: 1px solid var(--border); border-radius: 6px; }
  .actions { display: flex; gap: 8px; flex-wrap: wrap; margin-top: 10px; }
  select { min-width: 0; max-width: 100%; }
  .error { color: var(--danger); margin: 10px 0; }
</style>
