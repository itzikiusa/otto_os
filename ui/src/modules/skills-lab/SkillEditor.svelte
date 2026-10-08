<script lang="ts">
  // Skills Lab → Skills → Files: the multi-file viewer/editor for one copy of a
  // skill, on the shared CodeEditor (line numbers, markdown highlighting — the
  // same editor as Settings → Context library). Library copies are editable in
  // place (type, then Save / ⌘S; Revert drops the draft); bundled and provider
  // copies are read-only, with the way to make them editable spelled out.
  import { onDestroy, untrack } from 'svelte';
  import { toastError } from '../../lib/toastError';
  import { loadErrorText } from '../../lib/loadError';
  import type { SkillFileEntry } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { skillLabApi } from '../../lib/api/skillLab';
  import { toasts } from '../../lib/toast.svelte';
  import { formatBytes } from '../../lib/metric-format';
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import { sourceLabel } from './skillGroups';
  import { guardUnsaved } from '../../lib/leaveGuard';

  interface Props {
    name: string;
    /** "library" | "bundled" | a provider name. */
    source: string;
    files: SkillFileEntry[];
    /** SKILL.md text already loaded by the detail pane. */
    skillMd: string;
    /** File to open first (e.g. clicked in Overview). */
    initialFile?: string;
    onsaved: (files: SkillFileEntry[], skillMd: string | null) => void;
    /** Read-only copies: what makes them editable. */
    oninstall?: () => void;
    oncopytolibrary?: () => void;
    /** Unsaved-edit state, so the parents can ask before a tab / skill switch
     *  unmounts the editor and drops the draft. */
    ondirty?: (dirty: boolean) => void;
  }
  let { name, source, files, skillMd, initialFile = 'SKILL.md', onsaved, oninstall, oncopytolibrary, ondirty }: Props = $props();

  const editable = $derived(source === 'library');

  // svelte-ignore state_referenced_locally
  let currentFile = $state(initialFile);
  let content = $state('');
  let original = $state('');
  let binary = $state(false);
  let saving = $state(false);
  let loading = $state(false);
  let loadGeneration = 0;
  let disposed = false;
  onDestroy(() => { disposed = true; loadGeneration++; });
  let loadError = $state<string | null>(null);
  // Bumped whenever the text is replaced from outside (another file, Revert,
  // a reload) so the uncontrolled CodeEditor remounts on the new document.
  let docKey = $state(0);
  const dirty = $derived(editable && content !== original);
  $effect(() => {
    ondirty?.(dirty);
  });
  $effect(() => () => ondirty?.(false));
  // Leaving the page (sidebar, link, back) with an unsaved edit asks first.
  $effect(() => guardUnsaved(() => dirty, { what: currentFile }));

  async function open(path: string): Promise<void> {
    const owner = `${source}:${name}`;
    if (dirty && !(await confirmer.ask(`You have unsaved changes to ${currentFile}. Opening another file discards them.`, { title: 'Discard unsaved changes?', danger: true, confirmLabel: 'Discard', cancelLabel: 'Keep editing' }))) return;
    if (disposed || owner !== `${source}:${name}`) return;
    const generation = ++loadGeneration;
    currentFile = path;
    loadError = null;
    binary = false;
    content = original = '';
    loading = path !== 'SKILL.md';
    if (path === 'SKILL.md') {
      content = original = skillMd;
      docKey++;
      return;
    }
    try {
      if (source === 'library') {
        const r = await skillLabApi.getFile(name, path);
        if (generation !== loadGeneration) return;
        content = original = r.content;
        binary = r.binary;
        docKey++;
      } else if (source === 'bundled') {
        content = original = '';
        loadError = 'Install this skill to the library to open its other files.';
      } else {
        const r = await skillLabApi.getProviderFile(source, name, path);
        if (generation !== loadGeneration) return;
        content = original = r.content;
        binary = r.binary;
        docKey++;
      }
    } catch (e) {
      if (generation === loadGeneration) loadError = loadErrorText(e);
    } finally {
      if (generation === loadGeneration) loading = false;
    }
  }

  // (Re)open when the skill or copy changes.
  let openedFor = '';
  let openedOwner = '';
  $effect(() => {
    const owner = `${source}:${name}`;
    const key = `${owner}:${initialFile}`;
    if (key === openedFor) return;
    openedFor = key;
    // The parent already confirmed leaving the previous skill. Its draft
    // belongs to that owner and must not trigger another file-open prompt
    // under the replacement skill's heading.
    if (owner !== openedOwner) {
      openedOwner = owner;
      content = original = '';
      loadGeneration++;
    }
    currentFile = files.some((f) => f.path === initialFile) ? initialFile : 'SKILL.md';
    untrack(() => void open(currentFile));
  });
  // Keep SKILL.md in step when the parent reloads it (never over a draft).
  $effect(() => {
    const md = skillMd;
    if (currentFile === 'SKILL.md' && md !== original && !untrack(() => dirty)) {
      content = original = md;
      docKey++;
    }
  });
  function revert(): void {
    content = original;
    docKey++;
  }

  async function save(): Promise<void> {
    if (!editable || saving || loading || loadError || binary || !dirty) return;
    const owner = `${source}:${name}`;
    const target = name;
    const path = currentFile;
    const submitted = content;
    const generation = loadGeneration;
    saving = true;
    try {
      const next = await skillLabApi.putFile(target, { path, content: submitted });
      // A save acknowledges only the submitted snapshot, never later typing.
      if (disposed || owner !== `${source}:${name}` || generation !== loadGeneration) return;
      original = submitted;
      onsaved(next, path === 'SKILL.md' ? submitted : null);
      toasts.success('Saved', path);
    } catch (e) {
      toastError(`Couldn’t save ${path}`, e);
    } finally {
      saving = false;
    }
  }

  async function addFile(): Promise<void> {
    const target = name;
    const owner = `${source}:${name}`;
    const generation = loadGeneration;
    const path = await confirmer.promptText('Path inside the skill, for example references/notes.md', {
      title: 'New file',
      confirmLabel: 'Create',
      placeholder: 'references/notes.md',
    });
    if (!path || disposed || owner !== `${source}:${name}` || generation !== loadGeneration) return;
    // An existing path opens that file — "New file" must never empty it (the
    // server also refuses with 409 via `create_only`, for a file the tree
    // doesn't list yet).
    const wanted = path.trim().replace(/^\.?\/+/, '');
    if (files.some((f) => f.path === wanted)) {
      toasts.info('That file already exists', 'Opened it instead.');
      await open(wanted);
      return;
    }
    try {
      const next = await skillLabApi.putFile(target, { path: wanted, content: '', create_only: true });
      if (disposed || owner !== `${source}:${name}` || generation !== loadGeneration) return;
      onsaved(next, null);
      await open(wanted);
    } catch (e) {
      toastError('Couldn’t add the file', e);
    }
  }

  async function deleteFile(path: string): Promise<void> {
    const target = name;
    const owner = `${source}:${name}`;
    const generation = loadGeneration;
    if (!(await confirmer.ask(`Delete ${path} from ${name}? The file is removed from the library copy.`, { title: 'Delete file' }))) return;
    try {
      if (disposed || owner !== `${source}:${name}` || generation !== loadGeneration) return;
      await skillLabApi.deleteFile(target, path);
      const next = await skillLabApi.listFiles(target);
      if (disposed || owner !== `${source}:${name}` || generation !== loadGeneration) return;
      onsaved(next, null);
      if (currentFile === path) await open('SKILL.md');
    } catch (e) {
      toastError(`Couldn’t delete ${path}`, e);
    }
  }

  // ⌘S inside the editor saves (captured before CodeMirror / the app keymap).
  let codeEl = $state<HTMLElement | null>(null);
  $effect(() => {
    const el = codeEl;
    if (!el) return;
    el.addEventListener('keydown', onKey, true);
    return () => el.removeEventListener('keydown', onKey, true);
  });
  function onKey(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 's' && editable) {
      e.preventDefault();
      void save();
    }
  }
</script>

<div class="editor-wrap">
  {#if !editable}
    <div class="ro-note">
      <Icon name="lock" size={14} />
      <span>
        {#if source === 'bundled'}
          Bundled skills are read-only. Install it to the library to edit your own copy.
        {:else}
          This is the {sourceLabel(source)} copy on disk, shown read-only. Copy it to the library to edit it in Otto.
        {/if}
      </span>
      {#if source === 'bundled' && oninstall}
        <button class="btn small" onclick={oninstall}>Install to library</button>
      {:else if oncopytolibrary}
        <button class="btn small" onclick={oncopytolibrary}>Copy to library</button>
      {/if}
    </div>
  {/if}
  <div class="editor">
    <nav class="files" aria-label="Files in {name}">
      <div class="files-head">
        <span class="section-title">Files</span>
        {#if editable}
          <button class="icon-btn" onclick={addFile} aria-label="New file" title="New file"><Icon name="plus" size={14} /></button>
        {/if}
      </div>
      <ul>
        {#each files as f (f.path)}
          <li class="file-row">
            <button class="file" class:active={currentFile === f.path} aria-current={currentFile === f.path ? 'true' : undefined} onclick={() => open(f.path)} title="{f.path} · {formatBytes(f.size)}">
              <Icon name="file" size={12} />
              <span class="file-name mono" dir="ltr">{f.path}</span>
            </button>
            {#if editable && f.path !== 'SKILL.md'}
              <button class="icon-btn file-del" onclick={() => deleteFile(f.path)} aria-label="Delete {f.path}" title="Delete {f.path}"><Icon name="trash" size={12} /></button>
            {/if}
          </li>
        {/each}
      </ul>
    </nav>

    <section class="pane">
      <div class="pane-head">
        <span class="mono path" dir="ltr" title={currentFile}>{currentFile}</span>
        {#if dirty}<Badge tone="warn" label="Unsaved" />{/if}
        <span class="grow"></span>
        {#if editable && !loading && !binary && !loadError}
          {#if dirty}
            <button class="btn small ghost" onclick={revert} title="Drop your changes to {currentFile}">Revert</button>
          {:else}
            <span class="dim save-hint">Edit in place · ⌘S saves</span>
          {/if}
          <button class="btn small primary" disabled={saving || !dirty} title={dirty ? 'Save (⌘S)' : 'No changes to save'} onclick={save} data-testid="save-skill">{saving ? 'Saving…' : 'Save'}</button>
        {/if}
      </div>
      {#if loadError && source === 'bundled'}
        <!-- Not a failure: a bundled skill's other files open once installed. -->
        <p class="dim msg">{loadError}</p>
      {:else if loading || loadError}
        <LoadState what={currentFile} {loading} error={loadError} empty rows={6} onretry={() => open(currentFile)} />
      {:else if binary}
        <p class="dim msg">Binary file — not editable here.</p>
      {:else}
        <div class="code" dir="ltr" bind:this={codeEl} data-testid={editable ? 'skill-editor' : 'skill-view'}>
          {#key docKey}
            <CodeEditor
              path={`${name}/${currentFile}`}
              root=""
              {content}
              language={currentFile.toLowerCase().endsWith('.md') ? 'md' : undefined}
              readOnly={!editable}
              onchange={(v) => (content = v)}
            />
          {/key}
        </div>
      {/if}
    </section>
  </div>
</div>

<style>
  .editor-wrap {
    display: flex;
    flex-direction: column;
    gap: 10px;
    height: 100%;
    min-height: 0;
  }
  .ro-note {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    font-size: var(--fs-s);
    color: var(--text);
  }
  .ro-note > span {
    flex: 1;
  }
  .ro-note > :global(svg) {
    color: var(--text-dim);
  }
  .editor {
    flex: 1;
    min-height: 320px;
    display: grid;
    grid-template-columns: 220px minmax(0, 1fr);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--surface);
  }
  .files {
    border-inline-end: 1px solid var(--border);
    overflow-y: auto;
    min-height: 0;
  }
  .files-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-block: 6px 2px; padding-inline: 10px 8px;
  }
  .files-head .section-title {
    margin: 0;
  }
  .files ul {
    list-style: none;
    margin: 0;
    padding: 2px 6px 8px;
  }
  .file-row {
    display: flex;
    align-items: center;
  }
  .file {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
    font: inherit;
  }
  .file :global(svg) {
    flex: none;
    color: var(--text-dim);
  }
  .file:hover {
    background: var(--hover);
  }
  .file.active {
    background: var(--accent-soft);
  }
  .file-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .file-del {
    opacity: 0;
  }
  .file-row:hover .file-del,
  .file-del:focus-visible {
    opacity: 1;
  }
  .pane {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .pane-head {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 36px;
    flex-wrap: wrap;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
  }
  .path {
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .code {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .code > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .save-hint {
    font-size: var(--fs-xs);
  }
  .msg {
    padding: 14px;
    margin: 0;
  }
  @container skilldetail (max-width: 640px) {
    .editor {
      grid-template-columns: 1fr;
      grid-template-rows: auto 1fr;
    }
    .files {
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
      max-height: 160px;
    }
    .file-del {
      opacity: 1;
    }
  }
</style>
