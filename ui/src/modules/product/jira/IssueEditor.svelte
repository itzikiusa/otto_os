<script lang="ts">
  // Inline editor for one editable Jira field, dispatched by its editmeta
  // `schema_type`. The draft, the pickers' data and the save live on the
  // shared `JiraIssueEditor` (issueEditorState.svelte.ts), so the host only
  // decides WHERE a field's editor shows.
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import type { EditableField } from '../types';
  import type { JiraIssueEditor } from './issueEditorState.svelte';
  import { focusFirstField } from '../../../lib/inlineEdit';

  let { editor, ef, issueKey }: { editor: JiraIssueEditor; ef: EditableField; issueKey: string } = $props();

  const isSelect = $derived(
    ['option', 'priority', 'version', 'component'].includes(ef.schema_type) && ef.allowed_values.length > 0,
  );
</script>

<div class="field-editor" use:focusFirstField>
  {#if ef.schema_type === 'number'}
    <input aria-label={ef.name} class="field-input" type="number" step="any" bind:value={editor.fieldDraft} />
  {:else if ef.schema_type === 'date'}
    <input aria-label={ef.name} class="field-input" type="date" bind:value={editor.fieldDraft} />
  {:else if ef.schema_type === 'datetime'}
    <input aria-label={ef.name} class="field-input" type="datetime-local" bind:value={editor.fieldDraft} />
  {:else if ef.schema_type === 'user'}
    {#if editor.assignablesLoading && editor.assignables.length === 0}
      <Skeleton rows={1} height={27} label="people" />
    {:else}
      <select aria-label={ef.name} class="field-input" bind:value={editor.fieldDraft}>
        <option value="">Unassigned</option>
        {#each editor.assignables as u (u.account_id)}
          <option value={u.account_id}>{u.display_name}</option>
        {/each}
      </select>
    {/if}
  {:else if isSelect}
    <select aria-label={ef.name} class="field-input" bind:value={editor.fieldDraft}>
      {#if !ef.required}
        <option value="">— None —</option>
      {/if}
      {#each ef.allowed_values as opt (opt.id)}
        <option value={opt.id}>{opt.label}</option>
      {/each}
    </select>
  {:else if ef.schema_type === 'array' && ef.items !== 'string' && ef.allowed_values.length > 0}
    <div class="field-multiselect">
      {#each ef.allowed_values as opt (opt.id)}
        <label class="field-check">
          <input
            type="checkbox"
            checked={Array.isArray(editor.fieldDraft) && (editor.fieldDraft as string[]).includes(opt.id)}
            onchange={() => editor.toggleArrayOption(opt.id)}
          />
          {opt.label}
        </label>
      {/each}
    </div>
  {:else if ef.schema_type === 'array'}
    <!-- labels / free-text array (no allowed values) → comma-separated text -->
    <input dir="auto" aria-label={ef.name} class="field-input" type="text" placeholder="e.g. backend, payments" bind:value={editor.fieldDraft} />
  {:else}
    <!-- string / unknown → raw text -->
    <input dir="auto" aria-label={ef.name} class="field-input" type="text" bind:value={editor.fieldDraft} />
    {#if ef.schema_type !== 'string'}
      <span class="field-raw-note">raw ({ef.schema_type})</span>
    {/if}
  {/if}
  <div class="field-editor-actions">
    <button class="btn small primary" onclick={() => editor.saveField(ef)} disabled={editor.fieldSaving} title="Writes to the live Jira issue — everyone with access sees it">
      {editor.fieldSaving ? 'Saving…' : `Save to ${issueKey || 'Jira'}`}
    </button>
    <button class="btn small" onclick={() => editor.cancelEdit()} disabled={editor.fieldSaving}>Cancel</button>
  </div>
</div>

<style>
  .field-editor {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
  }
  .field-input {
    width: 100%;
    max-width: 240px;
    height: 26px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .field-input:focus {
    outline: none;
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .field-multiselect {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 160px;
    overflow-y: auto;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .field-check {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text);
    cursor: pointer;
  }
  .field-raw-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-style: italic;
  }
  .field-editor-actions {
    display: flex;
    gap: 6px;
  }
</style>
