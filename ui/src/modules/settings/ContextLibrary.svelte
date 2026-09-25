<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import { guardUnsaved } from '../../lib/leaveGuard';
  import SectionIntro from './SectionIntro.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
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
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { initialSelection, rememberSelection } from '../../lib/lastSelection';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  type Tab = 'skills' | 'souls' | 'context';
  const TABS: Tab[] = ['skills', 'souls', 'context'];

  // A library entry as shown in the list (skills carry a description).
  interface Entry {
    name: string;
    description?: string;
  }

  let tab: Tab = $state('skills');
  let entries: Entry[] = $state([]);
  let defaultSoul: string | null = $state(null);
  let loading = $state(false);
  let loadError = $state('');

  // Editor pane state.
  let selected: string | null = $state(null); // entry name being edited; null = none
  let isNew = $state(false);
  let editName = $state('');
  let editBody = $state('');
  // The body/name as loaded, so switching away can warn about unsaved edits
  // (they used to vanish silently on a tab or entry click).
  let loadedName = $state('');
  let loadedBody = $state('');
  let bodyLoading = $state(false);
  let bodyError = $state('');
  let saving = $state(false);
  let nameError = $state('');
  // CodeEditor is uncontrolled after mount — bump to remount it on a new body.
  let editorKey = $state(0);

  const TAB_META: Record<Tab, { label: string; singular: string; ext: string }> = {
    skills: { label: 'Skills', singular: 'skill', ext: 'md' },
    souls: { label: 'Souls', singular: 'soul', ext: 'md' },
    context: { label: 'Context', singular: 'context snippet', ext: 'md' },
  };

  const meta = $derived(TAB_META[tab]);
  const dirty = $derived(selected !== null && !bodyLoading && (editName !== loadedName || editBody !== loadedBody));
  // Leaving Settings (or this page) with unsaved edits asks first.
  $effect(() => guardUnsaved(() => dirty, { what: isNew ? `the new ${meta.singular}` : `“${loadedName}”` }));

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

  async function fetchEntries(t: Tab): Promise<Entry[]> {
    if (t === 'skills') {
      const list = await contextApi.listSkills();
      return list.map((s: LibrarySkill) => ({ name: s.name, description: s.description }));
    }
    if (t === 'souls') {
      const list = await contextApi.listSouls();
      return list.map((s: LibrarySoul) => ({ name: s.name }));
    }
    const list = await contextApi.listContext();
    return list.map((c: LibraryContext) => ({ name: c.name }));
  }

  /** Load a tab's list and open an entry: `open` if given, else the
   *  remembered/first one — never a bare "pick one" pane when items exist. */
  async function loadTab(t: Tab, open?: string): Promise<void> {
    loading = true;
    try {
      const list = await fetchEntries(t);
      if (tab !== t) return;
      entries = list;
      loadError = '';
      const pick = open && list.some((e) => e.name === open) ? open : initialSelection(`context-library.${t}`, list, (e) => e.name);
      if (pick) void openEntry(pick);
      else closeEditor();
    } catch (e) {
      if (tab !== t) return;
      loadError = loadErrorText(e);
    } finally {
      if (tab === t) loading = false;
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

  /** Ask before throwing away unsaved edits. Resolves true when it's OK to go on. */
  async function okToLeave(): Promise<boolean> {
    if (!dirty) return true;
    return confirmer.ask(
      `Your changes to ${isNew ? `the new ${meta.singular}` : `“${loadedName}”`} haven't been saved.`,
      { title: 'Discard changes?', confirmLabel: 'Discard' },
    );
  }

  function closeEditor(): void {
    selected = null;
    isNew = false;
    editName = '';
    editBody = '';
    loadedName = '';
    loadedBody = '';
    nameError = '';
    bodyError = '';
  }

  async function switchTab(t: Tab): Promise<void> {
    if (t === tab || !(await okToLeave())) return;
    closeEditor();
    entries = [];
    tab = t;
  }

  function onTabKey(e: KeyboardEvent): void {
    const i = TABS.indexOf(tab);
    let next: Tab | null = null;
    if (e.key === 'ArrowRight') next = TABS[(i + 1) % TABS.length];
    else if (e.key === 'ArrowLeft') next = TABS[(i - 1 + TABS.length) % TABS.length];
    else if (e.key === 'Home') next = TABS[0];
    else if (e.key === 'End') next = TABS[TABS.length - 1];
    if (!next) return;
    e.preventDefault();
    void switchTab(next).then(() => {
      (document.getElementById(`lib-tab-${tab}`) as HTMLButtonElement | null)?.focus();
    });
  }

  async function startNew(): Promise<void> {
    if (!(await okToLeave())) return;
    closeEditor();
    isNew = true;
    selected = '';
    editorKey++;
    queueMicrotask(() => document.getElementById('lib-name')?.focus());
  }

  async function pickEntry(name: string): Promise<void> {
    if (name === selected && !isNew) return;
    if (!(await okToLeave())) return;
    void openEntry(name);
  }

  async function openEntry(name: string): Promise<void> {
    isNew = false;
    selected = name;
    editName = name;
    loadedName = name;
    editBody = '';
    loadedBody = '';
    nameError = '';
    bodyError = '';
    bodyLoading = true;
    rememberSelection(`context-library.${tab}`, name);
    try {
      let body: string;
      if (tab === 'skills') body = (await contextApi.getSkill(name)).body;
      else if (tab === 'souls') body = (await contextApi.getSoul(name)).body;
      else body = (await contextApi.getContext(name)).body;
      if (selected !== name) return;
      editBody = body;
      loadedBody = body;
      editorKey++;
    } catch (e) {
      if (selected === name) bodyError = loadErrorText(e);
    } finally {
      if (selected === name) bodyLoading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Save (create / update / rename = new name). The server upserts by name, so
  // renaming is "save under a new name"; we delete the old name afterwards.
  // ---------------------------------------------------------------------------

  async function save(): Promise<void> {
    const name = editName.trim();
    if (name === '') {
      nameError = `Give this ${meta.singular} a name first.`;
      return;
    }
    if (!/^[A-Za-z0-9_-]+$/.test(name)) {
      nameError = 'Use letters, numbers, “-” and “_” only.';
      return;
    }
    const previous = isNew ? null : selected;
    if (name !== previous && entries.some((e) => e.name === name)) {
      nameError = `A ${meta.singular} named “${name}” already exists.`;
      return;
    }
    nameError = '';
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

      toasts.success(`${meta.singular[0].toUpperCase()}${meta.singular.slice(1)} saved`, name);
      // Mark clean before reloading so the reload doesn't ask to discard.
      loadedName = name;
      loadedBody = editBody;
      isNew = false;
      selected = name;
      await loadTab(tab, name);
    } catch (e) {
      toasts.error(`Couldn’t save the ${meta.singular}`, e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Delete
  // ---------------------------------------------------------------------------

  async function remove(name: string): Promise<void> {
    if (
      !(await confirmer.ask(
        `Delete the ${meta.singular} “${name}” from the library? Workspaces stop receiving it at their next session spawn.`,
        {
          title: `Delete ${meta.singular}`,
          confirmLabel: 'Delete',
        },
      ))
    )
      return;
    try {
      if (tab === 'skills') await contextApi.deleteSkill(name);
      else if (tab === 'souls') await contextApi.deleteSoul(name);
      else await contextApi.deleteContext(name);
      toasts.info(`Deleted ${name}`);
      if (selected === name) closeEditor();
      rememberSelection(`context-library.${tab}`, null);
      await loadTab(tab);
    } catch (e) {
      toasts.error(`Couldn’t delete ${name}`, e instanceof Error ? e.message : String(e));
    }
  }

  // ---------------------------------------------------------------------------
  // Global default soul
  // ---------------------------------------------------------------------------

  async function setDefaultSoul(name: string): Promise<void> {
    const prev = defaultSoul;
    try {
      const resp = await contextApi.setDefaultSoul(name);
      defaultSoul = resp.name;
      toasts.success('Default soul updated', resp.name ?? 'None');
    } catch (e) {
      defaultSoul = prev;
      toasts.error('Couldn’t set the default soul', e instanceof Error ? e.message : String(e));
    }
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('context-library')} subtitle="Otto’s library of skills, souls and context snippets">
    {#snippet actions()}
      {#if selected !== null}
        <button
          class="btn small primary"
          disabled={saving || bodyLoading || (!dirty && !isNew)}
          title={dirty || isNew ? `Save this ${meta.singular}` : 'No unsaved changes'}
          onclick={save}
        >
          {saving ? 'Saving…' : isNew ? `Create ${meta.singular}` : 'Save'}
        </button>
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody width="readable">
  <SectionIntro>The single source of truth materialized into each workspace's CLIs. Edits here reach agents at the <strong>next session spawn</strong>, not running sessions.</SectionIntro>

  <!-- Tabs (a real tablist: ←/→, Home/End) -->
  <div class="segmented lib-tabs" role="tablist" aria-label="Library">
    {#each TABS as t (t)}
      <button
        id={`lib-tab-${t}`}
        role="tab"
        aria-selected={tab === t}
        aria-controls="lib-panel"
        tabindex={tab === t ? 0 : -1}
        class:active={tab === t}
        onclick={() => void switchTab(t)}
        onkeydown={onTabKey}
      >
        {TAB_META[t].label}
      </button>
    {/each}
  </div>

  <div id="lib-panel" role="tabpanel" aria-labelledby={`lib-tab-${tab}`}>
    <!-- Default soul selector (souls tab only) -->
    {#if tab === 'souls' && entries.length > 0}
      <div class="default-soul field">
        <label for="lib-default-soul">Global default soul</label>
        <select
          id="lib-default-soul"
          class="input"
          value={defaultSoul ?? ''}
          onchange={(e) => setDefaultSoul(e.currentTarget.value)}
        >
          <option value="">None</option>
          {#each entries as e (e.name)}
            <option value={e.name}>{e.name}</option>
          {/each}
        </select>
        <span class="hint">Used by any workspace whose soul is set to “Global default”.</span>
      </div>
    {/if}

    <LoadState what={`library ${meta.label.toLowerCase()}`} {loading} error={loadError} empty={entries.length === 0 && !isNew} rows={4} onretry={() => void loadTab(tab)}>
      {#snippet emptyView()}
        <EmptyState
          variant="panel"
          icon="book"
          title={`No ${meta.label.toLowerCase()} yet`}
          body={tab === 'skills'
            ? 'Library skills are materialized into every workspace’s agent CLIs. Install bundled ones from Settings → Skills, or write your own.'
            : tab === 'souls'
              ? 'A soul is a persona injected into every interaction in a workspace.'
              : 'Context snippets are reusable markdown blocks added to agent context.'}
          actionLabel={`New ${meta.singular}`}
          actionIcon="plus"
          onaction={() => void startNew()}
        />
      {/snippet}
      <div class="lib-body">
        <!-- List pane -->
        <div class="list-pane">
          <div class="list-head">
            <h2 class="section-title">{meta.label} <span class="count">{entries.length}</span></h2>
            <button
              class="icon-btn"
              onclick={() => void startNew()}
              aria-label={`New ${meta.singular}`}
              title={`New ${meta.singular}`}
            >
              <Icon name="plus" size={14} />
            </button>
          </div>
          <div class="entry-list" role="group" aria-label={meta.label}>
            {#if isNew}
              <div class="entry active new" aria-current="true">
                <span class="entry-name mono">{editName.trim() || `New ${meta.singular}`}</span>
                <span class="entry-desc dim">Not saved yet</span>
              </div>
            {/if}
            {#each entries as e (e.name)}
              <button
                class="entry"
                aria-current={selected === e.name && !isNew ? 'true' : undefined}
                class:active={selected === e.name && !isNew}
                title={e.description ? `${e.name} — ${e.description}` : e.name}
                onclick={() => void pickEntry(e.name)}
              >
                <span class="entry-name mono">{e.name}</span>
                {#if e.description}
                  <span class="entry-desc dim">{e.description}</span>
                {/if}
              </button>
            {/each}
          </div>
        </div>

        <!-- Editor pane -->
        <div class="editor-pane">
          {#if selected === null}
            <EmptyState
              icon="book"
              title={`${entries.length} ${entries.length === 1 ? meta.singular : meta.label.toLowerCase()} in the library`}
              body={`Open one from the list to edit it, or add a new ${meta.singular}.`}
              actionLabel={`New ${meta.singular}`}
              actionIcon="plus"
              onaction={() => void startNew()}
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
                aria-invalid={nameError ? 'true' : undefined}
                aria-describedby="lib-name-hint"
                oninput={() => (nameError = '')}
              />
              {#if nameError}
                <span class="field-err" id="lib-name-hint" role="alert">{nameError}</span>
              {:else}
                <span class="hint" id="lib-name-hint">
                  Letters, numbers, “-” and “_” only. Changing the name of an existing entry renames it.
                </span>
              {/if}
            </div>

            <div class="editor-label">
              Body (markdown)
              {#if dirty}<span class="unsaved">Unsaved changes</span>{/if}
            </div>
            <div class="editor-box">
              {#if bodyLoading || bodyError}
                <LoadState what={`“${selected}”`} loading={bodyLoading} error={bodyError || null} empty rows={4} onretry={() => selected && void openEntry(selected)} />
              {:else}
                {#key editorKey}
                  <CodeEditor
                    path={editorPath}
                    root=""
                    content={editBody}
                    language="md"
                    readOnly={false}
                    placeholder={tab === 'skills'
                      ? 'Markdown: front matter (name, description), then the instructions'
                      : `Write the ${meta.singular} in markdown`}
                    onchange={(v) => (editBody = v)}
                  />
                {/key}
              {/if}
            </div>

            <div class="actions">
              {#if !isNew && selected}
                <button class="btn small danger" onclick={() => selected && remove(selected)}>
                  <Icon name="trash" size={12} /> Delete…
                </button>
              {:else}
                <button class="btn small ghost" onclick={() => { closeEditor(); if (entries[0]) void openEntry(entries[0].name); }}>
                  Cancel
                </button>
              {/if}
            </div>
          {/if}
        </div>
      </div>
    </LoadState>
  </div>
  </PageBody>
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .lib-tabs {
    margin-bottom: 14px;
  }
  .default-soul {
    max-width: 360px;
    margin-bottom: 16px;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .field-err {
    font-size: var(--fs-xs);
    color: var(--danger);
  }
  .lib-body {
    display: grid;
    grid-template-columns: 260px minmax(0, 1fr);
    gap: 16px;
    align-items: start;
    max-width: 1000px;
  }
  .list-pane {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .list-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .list-head .section-title {
    margin: 0;
  }
  .count {
    font-weight: 500;
  }
  .entry-list {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    overflow: hidden auto;
    max-height: 520px;
  }
  .entry {
    display: flex;
    flex-direction: column;
    gap: 2px;
    text-align: start;
    padding: 6px 10px;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    min-width: 0;
    font: inherit;
  }
  .entry + .entry {
    border-top: 1px solid var(--border);
  }
  .entry:hover {
    background: var(--hover);
  }
  .entry.active {
    background: var(--accent-soft);
  }
  .entry-name {
    font-size: var(--fs-s);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .entry-desc {
    font-size: var(--fs-xs);
    color: var(--text-dim);
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
  .editor-pane .field {
    margin: 0;
  }
  .editor-label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .unsaved {
    font-weight: 400;
    color: var(--warning);
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
  @media (max-width: 640px) {
    .lib-body {
      grid-template-columns: 1fr;
    }
    .entry-list {
      max-height: 240px;
    }
  }
</style>
