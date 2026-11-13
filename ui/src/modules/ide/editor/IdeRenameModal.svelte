<script lang="ts">
  // Workspace-wide Rename Symbol Modal (`⇧F6`).
  import { announceShown } from '../../../lib/announce.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { plural } from '../../../lib/plural';
  import { ide } from '../ideStore.svelte';

  let renameMode = $state<'preview' | 'apply' | null>(null);

  async function runRenameAction(mode: 'preview' | 'apply'): Promise<void> {
    renameMode = mode;
    try {
      if (mode === 'preview') await ide.previewRename();
      else await ide.applyRename();
    } finally {
      renameMode = null;
    }
  }
</script>

{#if ide.renameState.open}
  <Modal
    title="Rename symbol across workspace"
    width={580}
    dismissable={!ide.renameState.loading}
    onclose={() => (ide.renameState.open = false)}
  >
    <div class="rename-modal">
      <div class="rename-row">
        <input
          class="rename-input"
          type="text"
          dir="ltr"
          data-autofocus
          aria-label="New symbol name"
          placeholder="New identifier name…"
          bind:value={ide.renameState.newName}
          onkeydown={(e) => {
            if (e.key === 'Enter' && !ide.renameState.loading) {
              e.preventDefault();
              void runRenameAction('apply');
            }
          }}
        />
        <button
          type="button"
          class="btn small"
          disabled={!ide.renameState.newName.trim() || ide.renameState.loading}
          onclick={() => void runRenameAction('preview')}
        >
          {ide.renameState.loading && renameMode === 'preview' ? 'Previewing…' : 'Preview'}
        </button>
      </div>

      {#if ide.renameState.error}
        <p class="rename-err" use:announceShown={{ text: ide.renameState.error, assertive: true }}>
          {ide.renameState.error}
        </p>
      {/if}

      {#if ide.renameState.edits.length > 0}
        <div class="rename-preview">
          <div class="rename-summary">Updates across {plural(ide.renameState.edits.length, 'file')}</div>
          {#each ide.renameState.edits as edit (edit.path)}
            <div class="rename-file" dir="ltr">
              <div class="rename-file-head">
                {ide.relativePath(edit.path)} ({plural(edit.occurrences, 'occurrence')})
              </div>
              {#each edit.preview_lines.slice(0, 3) as pl (`${edit.path}:${pl.line}`)}
                <div class="rename-before">- L{pl.line}: {pl.before}</div>
                <div class="rename-after">+ L{pl.line}: {pl.after}</div>
              {/each}
            </div>
          {/each}
        </div>
      {/if}
    </div>

    {#snippet footer()}
      <button type="button" class="btn" disabled={ide.renameState.loading} onclick={() => (ide.renameState.open = false)}>
        Cancel
      </button>
      <button
        type="button"
        class="btn primary"
        disabled={!ide.renameState.newName.trim() || ide.renameState.loading}
        onclick={() => void runRenameAction('apply')}
      >
        {ide.renameState.loading && renameMode === 'apply' ? 'Renaming…' : 'Rename'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .rename-modal { display: flex; flex-direction: column; gap: 10px; }
  .rename-row { display: flex; gap: var(--sp-3); }
  .rename-input {
    flex: 1; block-size: 32px; padding-inline: 10px; font-family: var(--font-mono); font-size: var(--fs-s);
    background: var(--surface-2); color: var(--text); border: 1px solid var(--control-border); border-radius: var(--radius-s);
  }
  .rename-err { margin: 0; color: var(--danger); font-size: var(--fs-xs); }
  .rename-preview {
    display: flex; flex-direction: column; gap: var(--sp-3); max-block-size: 240px; overflow-y: auto;
    padding: var(--sp-4) 10px; background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-s); font-size: var(--fs-xs);
  }
  .rename-summary { font-weight: 600; }
  .rename-file { font-family: var(--font-mono); font-size: var(--fs-xs); border-block-start: 1px solid var(--border); padding-block-start: var(--sp-2); }
  .rename-file-head { color: var(--text-dim); }
  .rename-before { color: var(--danger); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rename-after { color: var(--success); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
