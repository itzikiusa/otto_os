<script lang="ts">
  // "Save request" sheet: a name and WHERE it goes (collection / folder picked
  // from a tree-ordered list, or a new collection created on the spot).
  // Replaces the old two-prompt flow with its numbered collection list.
  import Modal from '../../lib/components/Modal.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { collectionPaths } from '../../lib/api/apiVars';
  import type { Id } from '../../lib/api/types';

  interface Props {
    title?: string;
    initialName: string;
    initialCollection: Id | null;
    onclose: () => void;
    /** Resolves once the request is saved (the sheet stays open on failure). */
    onsave: (name: string, collectionId: Id | null) => Promise<boolean>;
  }
  let { title = 'Save request', initialName, initialCollection, onclose, onsave }: Props = $props();

  const NEW = '__new__';
  const paths = $derived(collectionPaths(apiClient.collections));
  // svelte-ignore state_referenced_locally
  let name = $state(initialName);
  // svelte-ignore state_referenced_locally
  let target = $state<string>(initialCollection ?? '');
  let newCollection = $state('');
  let busy = $state(false);
  let error = $state('');

  const valid = $derived(name.trim() !== '' && (target !== NEW || newCollection.trim() !== ''));

  async function submit(e?: Event): Promise<void> {
    e?.preventDefault();
    if (!valid || busy) return;
    busy = true;
    error = '';
    try {
      let collectionId: Id | null = target === '' ? null : target;
      if (target === NEW) {
        const created = await apiClient.saveCollection({ name: newCollection.trim(), parent_id: null });
        if (!created) { error = 'Couldn’t create the collection. Try again, or pick an existing one.'; return; }
        collectionId = created.id;
      }
      if (await onsave(name.trim(), collectionId)) onclose();
    } finally {
      busy = false;
    }
  }
</script>

<Modal {title} width={440} {onclose}>
  <form class="save-form" onsubmit={submit}>
    <div class="field">
      <label for="save-req-name">Name</label>
      <input id="save-req-name" class="input" bind:value={name} placeholder="List customers" autocomplete="off" />
    </div>
    <div class="field">
      <label for="save-req-where">Save to</label>
      <select id="save-req-where" class="input" bind:value={target}>
        <option value="">No collection (ungrouped)</option>
        {#each paths as p (p.id)}
          <option value={p.id}>{p.path}</option>
        {/each}
        <option value={NEW}>New collection…</option>
      </select>
      {#if target === NEW}
        <input class="input new-col" bind:value={newCollection} placeholder="Collection name, e.g. Payments API" aria-label="New collection name" />
      {/if}
      <span class="hint">Collections group requests by API or feature. Everyone in this workspace can see saved requests; stored credentials go to the macOS Keychain.</span>
    </div>
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <button type="submit" hidden aria-hidden="true" tabindex="-1"></button>
  </form>
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={() => void submit()} disabled={!valid || busy}>{busy ? 'Saving…' : 'Save'}</button>
  {/snippet}
</Modal>

<style>
  .save-form {
    display: flex;
    flex-direction: column;
  }
  .new-col {
    margin-top: 6px;
  }
  .err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
</style>
