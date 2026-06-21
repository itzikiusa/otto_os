<script lang="ts">
  // Context Library (root-only): author and edit the Otto-owned library of
  // skills, souls, and context snippets — the single source of truth that gets
  // materialized into each workspace's CLIs. Also sets the instance-wide default
  // soul. Library writes are root-only on the server; this page is gated to root
  // in Settings.svelte, mirroring Providers/Users/Daemon.
  import { contextApi } from '../../lib/api/context';
  import type {
    LibraryContext,
    LibrarySkill,
    LibrarySoul,
  } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  type Tab = 'skills' | 'souls' | 'context';

  // A library entry as shown in the list (skills carry a description).
  interface Entry {
    name: string;
    description?: string;
  }

  let tab: Tab = $state('skills');
  let entries: Entry[] = $state([]);
  let defaultSoul: string | null = $state(null);
  let loading = $state(false);

  // Editor pane state.
  let selected: string | null = $state(null); // entry name being edited; null = none
  let isNew = $state(false);
  let editName = $state('');
  let editBody = $state('');
  let bodyLoading = $state(false);
  let saving = $state(false);

  const TAB_META: Record<Tab, { label: string; singular: string; ext: string }> = {
    skills: { label: 'Skills', singular: 'skill', ext: 'md' },
    souls: { label: 'Souls', singular: 'soul', ext: 'md' },
    context: { label: 'Context', singular: 'context snippet', ext: 'md' },
  };

  const meta = $derived(TAB_META[tab]);

  // CodeEditor wants a path (for language detection) + a root. The library is not
  // a real workspace dir, so root is empty (LSP simply won't attach — fine for md).
  const editorPath = $derived(`${editName || 'untitled'}.md`);

  // ---------------------------------------------------------------------------
  // Load lists on tab change
  // ---------------------------------------------------------------------------

  $effect(() => {
    void loadTab(tab);
  });

  $effect(() => {
    void loadDefaultSoul();
  });

  async function loadTab(t: Tab): Promise<void> {
    loading = true;
    // Reset the editor pane when switching tabs.
    closeEditor();
    try {
      if (t === 'skills') {
        const list = await contextApi.listSkills();
        entries = list.map((s: LibrarySkill) => ({ name: s.name, description: s.description }));
      } else if (t === 'souls') {
        const list = await contextApi.listSouls();
        entries = list.map((s: LibrarySoul) => ({ name: s.name }));
      } else {
        const list = await contextApi.listContext();
        entries = list.map((c: LibraryContext) => ({ name: c.name }));
      }
    } catch (e) {
      toasts.error('Could not load library', e instanceof Error ? e.message : String(e));
      entries = [];
    } finally {
      loading = false;
    }
  }

  async function loadDefaultSoul(): Promise<void> {
    try {
      const resp = await contextApi.getDefaultSoul();
      defaultSoul = resp.name;
    } catch {
      // non-fatal: leave default-soul selector empty
    }
  }

  // ---------------------------------------------------------------------------
  // Editor pane
  // ---------------------------------------------------------------------------

  function closeEditor(): void {
    selected = null;
    isNew = false;
    editName = '';
    editBody = '';
  }

  function startNew(): void {
    closeEditor();
    isNew = true;
    selected = '';
    editName = '';
    editBody = '';
  }

  async function openEntry(name: string): Promise<void> {
    isNew = false;
    selected = name;
    editName = name;
    editBody = '';
    bodyLoading = true;
    try {
      if (tab === 'skills') editBody = (await contextApi.getSkill(name)).body;
      else if (tab === 'souls') editBody = (await contextApi.getSoul(name)).body;
      else editBody = (await contextApi.getContext(name)).body;
    } catch (e) {
      toasts.error('Could not load entry', e instanceof Error ? e.message : String(e));
    } finally {
      bodyLoading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Save (create / update / rename = new name). The server upserts by name, so
  // renaming is "save under a new name"; we delete the old name afterwards.
  // ---------------------------------------------------------------------------

  async function save(): Promise<void> {
    const name = editName.trim();
    if (name === '') {
      toasts.error('Name required', 'Give this entry a name first.');
      return;
    }
    const previous = isNew ? null : selected;
    saving = true;
    try {
      if (tab === 'skills') await contextApi.putSkill(name, editBody);
      else if (tab === 'souls') await contextApi.putSoul(name, editBody);
      else await contextApi.putContext(name, editBody);

      // Rename: the name changed on an existing entry → remove the old one.
      if (previous && previous !== name) {
        if (tab === 'skills') await contextApi.deleteSkill(previous);
        else if (tab === 'souls') await contextApi.deleteSoul(previous);
        else await contextApi.deleteContext(previous);
      }

      toasts.success(`${meta.singular} saved`, name);
      selected = name;
      isNew = false;
      await loadTab(tab);
      void openEntry(name);
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Delete
  // ---------------------------------------------------------------------------

  async function remove(name: string): Promise<void> {
    if (
      !(await confirmer.ask(`Delete the ${meta.singular} “${name}” from the library?`, {
        title: `Delete ${meta.singular}`,
        confirmLabel: 'Delete',
      }))
    )
      return;
    try {
      if (tab === 'skills') await contextApi.deleteSkill(name);
      else if (tab === 'souls') await contextApi.deleteSoul(name);
      else await contextApi.deleteContext(name);
      toasts.info(`${meta.singular} deleted`, name);
      if (selected === name) closeEditor();
      await loadTab(tab);
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  // ---------------------------------------------------------------------------
  // Global default soul
  // ---------------------------------------------------------------------------

  async function setDefaultSoul(name: string): Promise<void> {
    try {
      const resp = await contextApi.setDefaultSoul(name);
      defaultSoul = resp.name;
      toasts.success('Default soul updated', resp.name ?? '(none)');
    } catch (e) {
      toasts.error('Could not set default soul', e instanceof Error ? e.message : String(e));
    }
  }
</script>

<div class="page">
  <!-- Header -->
  <div class="page-header">
    <div>
      <h1>Context Library</h1>
      <div class="sub">
        The Otto-owned library of skills, souls, and context snippets — the single source of truth
        materialized into each workspace's CLIs. Edits here propagate at the next session spawn.
      </div>
    </div>
  </div>

  <!-- Default soul selector (souls tab only) -->
  {#if tab === 'souls'}
    <div class="default-soul card">
      <label for="lib-default-soul">Global default soul</label>
      <select
        id="lib-default-soul"
        class="input"
        value={defaultSoul ?? ''}
        onchange={(e) => setDefaultSoul(e.currentTarget.value)}
      >
        <option value="">(none)</option>
        {#each entries as e (e.name)}
          <option value={e.name}>{e.name}</option>
        {/each}
      </select>
      <span class="hint">Used by any workspace whose soul is set to “(global default)”.</span>
    </div>
  {/if}

  <!-- Tabs -->
  <div class="tabs">
    {#each Object.entries(TAB_META) as [t, m] (t)}
      <button class="tab" class:active={tab === t} onclick={() => (tab = t as Tab)}>
        {m.label}
      </button>
    {/each}
  </div>

  <div class="lib-body">
    <!-- List pane -->
    <div class="list-pane">
      <div class="list-head">
        <span class="list-title">{meta.label}</span>
        <button class="btn sm primary" onclick={startNew}>New</button>
      </div>
      {#if loading}
        <Skeleton rows={3} height={32} />
      {:else if entries.length === 0}
        <EmptyState icon="box" title={`No ${meta.label.toLowerCase()} yet`} body={`Click “New” to add a ${meta.singular}.`} />
      {:else}
        <div class="entry-list">
          {#each entries as e (e.name)}
            <button
              class="entry"
              class:active={selected === e.name && !isNew}
              onclick={() => openEntry(e.name)}
            >
              <span class="entry-name mono">{e.name}</span>
              {#if e.description}
                <span class="entry-desc dim">{e.description}</span>
              {/if}
            </button>
          {/each}
        </div>
      {/if}
    </div>

    <!-- Editor pane -->
    <div class="editor-pane">
      {#if selected === null}
        <EmptyState
          icon="edit"
          title="Select an entry"
          body={`Pick a ${meta.singular} from the list, or click “New” to create one.`}
        />
      {:else}
        <div class="field">
          <label for="lib-name">Name</label>
          <input
            id="lib-name"
            class="input mono"
            bind:value={editName}
            spellcheck="false"
            autocomplete="off"
            placeholder={tab === 'skills' ? 'support-triage-router' : 'otto'}
          />
          <span class="hint">
            Letters, numbers, “-” and “_” only. Changing the name of an existing entry renames it.
          </span>
        </div>

        <div class="editor-label">Body (markdown)</div>
        <div class="editor-box">
          {#if bodyLoading}
            <Skeleton rows={4} height={40} />
          {:else}
            <CodeEditor
              path={editorPath}
              root=""
              content={editBody}
              language="md"
              readOnly={false}
              onchange={(v) => (editBody = v)}
            />
          {/if}
        </div>

        <div class="actions">
          <button class="btn primary" disabled={saving} onclick={save}>
            {saving ? 'Saving…' : 'Save'}
          </button>
          {#if !isNew && selected}
            <button class="btn" onclick={() => selected && remove(selected)}>Delete</button>
          {/if}
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .default-soul {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 14px 16px;
    max-width: 560px;
    margin-bottom: 16px;
  }
  .default-soul label {
    font-size: 12.5px;
    font-weight: 600;
  }

  .tabs {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 14px;
  }
  .tab {
    height: 30px;
    padding: 0 14px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 12.5px;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }
  .tab:hover {
    color: var(--text);
  }
  .tab.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
    font-weight: 500;
  }

  .lib-body {
    display: grid;
    grid-template-columns: 260px 1fr;
    gap: 16px;
    align-items: start;
    max-width: min(960px, 92vw);
  }

  .list-pane {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .list-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .list-title {
    font-size: 12.5px;
    font-weight: 600;
  }

  .entry-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .entry {
    display: flex;
    flex-direction: column;
    gap: 2px;
    text-align: start;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    cursor: pointer;
    min-width: 0;
  }
  .entry:hover {
    background: var(--surface-2);
  }
  .entry.active {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .entry-name {
    font-size: 12px;
    font-weight: 500;
  }
  .entry-desc {
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .editor-pane {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .field label {
    font-size: 12.5px;
    font-weight: 600;
  }
  .hint {
    font-size: 11.5px;
    color: var(--text-dim);
  }

  .editor-label {
    font-size: 12.5px;
    font-weight: 600;
  }
  .editor-box {
    height: 420px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .btn.sm {
    font-size: 11.5px;
    height: 24px;
    padding: 0 10px;
  }
</style>
