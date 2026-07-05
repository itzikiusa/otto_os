<script lang="ts">
  // Skills Lab → Skills. A full view of every skill (editable Otto library +
  // read-only bundled catalog) with a multi-file viewer/editor, create, import
  // (.zip), install-to-library, delete, and per-skill Review / Evaluate actions.
  import type {
    LibrarySkill,
    BundledSkillView,
    ProviderSkillInfo,
    SkillFileEntry,
  } from '../../lib/api/types';
  import { skillLabApi } from '../../lib/api/skillLab';
  import { toasts } from '../../lib/toast.svelte';

  interface Props {
    onreview?: (name: string, source: string) => void;
    onevaluate?: (name: string, source: string) => void;
  }
  let { onreview, onevaluate }: Props = $props();

  // `source` is "library" | "bundled" | a provider name (claude/codex/agy).
  type Row = { name: string; source: string; category: string; description: string; state?: string };

  let library = $state<LibrarySkill[]>([]);
  let bundled = $state<BundledSkillView[]>([]);
  let providerSkills = $state<ProviderSkillInfo[]>([]);
  let query = $state('');
  let selected = $state<{ name: string; source: string } | null>(null);

  // Detail state.
  let files = $state<SkillFileEntry[]>([]);
  let currentFile = $state('SKILL.md');
  let content = $state('');
  let contentBinary = $state(false);
  let editMode = $state(false);
  let saving = $state(false);

  // New-skill form.
  let showNew = $state(false);
  let nName = $state('');
  let nCategory = $state('development');
  let nDescription = $state('');

  const rows = $derived.by<Row[]>(() => {
    const out: Row[] = [];
    const libNames = new Set(library.map((s) => s.name));
    for (const s of library) out.push({ name: s.name, source: 'library', category: s.category || 'uncategorized', description: s.description });
    for (const b of bundled) if (!libNames.has(b.name)) out.push({ name: b.name, source: 'bundled', category: b.category, description: b.description, state: b.state });
    for (const p of providerSkills) out.push({ name: p.name, source: p.provider, category: p.category || 'provider', description: p.description });
    const q = query.trim().toLowerCase();
    const filtered = q ? out.filter((r) => r.name.toLowerCase().includes(q) || r.description.toLowerCase().includes(q)) : out;
    filtered.sort((a, b) => a.category.localeCompare(b.category) || a.name.localeCompare(b.name));
    return filtered;
  });

  const grouped = $derived.by(() => {
    const m = new Map<string, Row[]>();
    for (const r of rows) {
      const arr = m.get(r.category) ?? [];
      arr.push(r);
      m.set(r.category, arr);
    }
    return [...m.entries()];
  });

  const isLibrary = $derived(selected?.source === 'library');
  const isBundled = $derived(selected?.source === 'bundled');
  const isProvider = $derived(!!selected && selected.source !== 'library' && selected.source !== 'bundled');

  async function loadAll(): Promise<void> {
    const [lib, bun, prov] = await Promise.all([
      skillLabApi.listLibrary().catch(() => [] as LibrarySkill[]),
      skillLabApi.listBundled().catch(() => [] as BundledSkillView[]),
      skillLabApi.listProvider().catch(() => [] as ProviderSkillInfo[]),
    ]);
    library = lib;
    bundled = bun;
    providerSkills = prov;
  }

  async function select(name: string, source: string): Promise<void> {
    selected = { name, source };
    editMode = false;
    currentFile = 'SKILL.md';
    files = [];
    content = '';
    contentBinary = false;
    try {
      if (source === 'library') {
        files = await skillLabApi.listFiles(name);
        await openFile('SKILL.md');
      } else if (source === 'bundled') {
        const b = await skillLabApi.getBundled(name);
        files = b.files;
        content = b.body;
      } else {
        // provider skill (claude/codex/agy)
        const p = await skillLabApi.getProvider(source, name);
        files = p.files;
        content = p.body;
      }
    } catch (e) {
      toasts.error('Open skill failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function openFile(path: string): Promise<void> {
    currentFile = path;
    editMode = false;
    if (!selected) return;
    if (selected.source === 'bundled') {
      if (path === 'SKILL.md') return; // body already loaded
      content = 'Install this skill to the library to view/edit its files.';
      contentBinary = false;
      return;
    }
    if (selected.source !== 'library') {
      // provider skill — read the file directly, view-only.
      try {
        const r = await skillLabApi.getProviderFile(selected.source, selected.name, path);
        content = r.content;
        contentBinary = r.binary;
      } catch (e) {
        toasts.error('Open file failed', e instanceof Error ? e.message : String(e));
      }
      return;
    }
    try {
      const r = await skillLabApi.getFile(selected.name, path);
      content = r.content;
      contentBinary = r.binary;
    } catch (e) {
      toasts.error('Open file failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function save(): Promise<void> {
    if (!selected || selected.source !== 'library' || saving) return;
    saving = true;
    try {
      files = await skillLabApi.putFile(selected.name, { path: currentFile, content });
      editMode = false;
      await loadAll();
      toasts.info('Saved');
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  async function addFile(): Promise<void> {
    if (!selected || selected.source !== 'library') return;
    const path = prompt('New file path (relative, e.g. references/notes.md)');
    if (!path) return;
    try {
      files = await skillLabApi.putFile(selected.name, { path, content: '' });
      await openFile(path);
      editMode = true;
    } catch (e) {
      toasts.error('Add file failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function deleteFile(path: string): Promise<void> {
    if (!selected || selected.source !== 'library' || path === 'SKILL.md') return;
    if (!confirm(`Delete ${path}?`)) return;
    try {
      await skillLabApi.deleteFile(selected.name, path);
      files = await skillLabApi.listFiles(selected.name);
      if (currentFile === path) await openFile('SKILL.md');
    } catch (e) {
      toasts.error('Delete file failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function createSkill(): Promise<void> {
    if (!nName.trim()) return;
    try {
      const s = await skillLabApi.create({ name: nName.trim(), category: nCategory, description: nDescription });
      showNew = false;
      nName = ''; nDescription = '';
      await loadAll();
      await select(s.name, 'library');
      editMode = true;
    } catch (e) {
      toasts.error('Create failed', e instanceof Error ? e.message : String(e));
    }
  }

  let importing = $state(false);
  async function onImport(ev: Event): Promise<void> {
    const input = ev.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    importing = true;
    try {
      const s = await skillLabApi.importZip(file);
      await loadAll();
      await select(s.name, 'library');
      toasts.info(`Imported ${s.name}`);
    } catch (e) {
      toasts.error('Import failed', e instanceof Error ? e.message : String(e));
    } finally {
      importing = false;
      input.value = '';
    }
  }

  async function installSkill(): Promise<void> {
    if (!selected || selected.source !== 'bundled') return;
    try {
      await skillLabApi.install(selected.name);
      await loadAll();
      await select(selected.name, 'library');
      toasts.info('Installed to library — now editable');
    } catch (e) {
      toasts.error('Install failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function deleteSkill(): Promise<void> {
    if (!selected || selected.source !== 'library') return;
    if (!confirm(`Delete skill "${selected.name}" from the library?`)) return;
    try {
      await skillLabApi.remove(selected.name);
      selected = null;
      await loadAll();
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  $effect(() => {
    void loadAll();
  });

  function badgeKind(source: string): string {
    return source === 'library' ? 'library' : source === 'bundled' ? 'bundled' : 'provider';
  }
  function badgeText(source: string): string {
    return source === 'library' ? 'lib' : source;
  }
</script>

<div class="skills-browser" data-testid="skills-browser">
  <aside class="sb-side">
    <div class="sb-actions">
      <button class="btn small primary" onclick={() => { showNew = !showNew; }} data-testid="new-skill">+ New</button>
      <label class="btn small ghost sb-import" title="Import a skill .zip">
        {importing ? 'Importing…' : 'Import'}
        <input type="file" accept=".zip" onchange={onImport} hidden />
      </label>
    </div>
    {#if showNew}
      <div class="sb-newform card">
        <input placeholder="skill-name (kebab-case)" bind:value={nName} data-testid="new-skill-name" />
        <input placeholder="category" bind:value={nCategory} />
        <input placeholder="description" bind:value={nDescription} />
        <button class="btn small primary" onclick={createSkill} data-testid="create-skill">Create</button>
      </div>
    {/if}
    <input class="sb-search" placeholder="Search skills…" bind:value={query} />
    <div class="sb-list">
      {#each grouped as [cat, items] (cat)}
        <div class="sb-cat">{cat}</div>
        {#each items as r (r.source + ':' + r.name)}
          <button class="sb-item" class:active={selected?.name === r.name && selected?.source === r.source} onclick={() => select(r.name, r.source)}>
            <span class="sb-item-name">{r.name}</span>
            <span class="chip sb-badge sb-badge-{badgeKind(r.source)}">{badgeText(r.source)}</span>
          </button>
        {/each}
      {/each}
      {#if rows.length === 0}
        <p class="sb-empty">No skills match.</p>
      {/if}
    </div>
  </aside>

  <main class="sb-main">
    {#if !selected}
      <div class="sb-placeholder">
        <p>Select a skill to view or edit it, or create a new one.</p>
        <p class="sb-dim">Library skills are editable; bundled skills can be installed to the library to edit.</p>
      </div>
    {:else}
      <div class="sb-head">
        <h3>{selected.name}</h3>
        <span class="chip sb-badge sb-badge-{badgeKind(selected.source)}">{selected.source}</span>
        <span class="grow"></span>
        <button class="btn small ghost" onclick={() => selected && onreview?.(selected.name, selected.source)} data-testid="review-skill">Review</button>
        <button class="btn small ghost" onclick={() => selected && onevaluate?.(selected.name, selected.source)}>Evaluate</button>
        {#if isLibrary}
          {#if editMode}
            <button class="btn small primary" disabled={saving} onclick={save} data-testid="save-skill">{saving ? 'Saving…' : 'Save'}</button>
            <button class="btn small ghost" onclick={() => { editMode = false; void openFile(currentFile); }}>Cancel</button>
          {:else}
            <button class="btn small primary" onclick={() => { editMode = true; }} data-testid="edit-skill">Edit</button>
          {/if}
          <button class="btn small ghost danger" onclick={deleteSkill} data-testid="delete-skill">Delete</button>
        {:else if isBundled}
          <button class="btn small primary" onclick={installSkill}>Install to library</button>
        {:else if isProvider}
          <span class="sb-readonly">read-only</span>
        {/if}
      </div>

      <div class="sb-body">
        <nav class="sb-files">
          <div class="sb-files-head">
            <span>Files</span>
            {#if isLibrary}<button class="btn tiny ghost" onclick={addFile} title="Add file">＋</button>{/if}
          </div>
          {#each files as f (f.path)}
            <div class="sb-file-row">
              <button class="sb-file" class:active={currentFile === f.path} onclick={() => openFile(f.path)}>{f.path}</button>
              {#if isLibrary && f.path !== 'SKILL.md'}
                <button class="sb-file-del" title="Delete file" onclick={() => deleteFile(f.path)}>✕</button>
              {/if}
            </div>
          {/each}
        </nav>

        <section class="sb-editor">
          <div class="sb-editor-head"><span class="mono">{currentFile}</span></div>
          {#if editMode && isLibrary}
            <textarea class="sb-textarea" bind:value={content} spellcheck="false" data-testid="skill-editor"></textarea>
          {:else if contentBinary}
            <p class="sb-dim">Binary file ({content.length} bytes shown lossily) — not editable here.</p>
          {:else}
            <pre class="sb-view" data-testid="skill-view">{content}</pre>
          {/if}
        </section>
      </div>
    {/if}
  </main>
</div>

<style>
  .skills-browser { display: grid; grid-template-columns: 260px 1fr; gap: 12px; height: 100%; min-height: 0; }
  .sb-side { display: flex; flex-direction: column; gap: 8px; min-height: 0; }
  .sb-actions { display: flex; gap: 6px; }
  .sb-import { cursor: pointer; }
  .sb-newform { padding: 10px; display: flex; flex-direction: column; gap: 6px; }
  .sb-newform input { padding: 6px 8px; border-radius: var(--radius-m); border: 1px solid var(--border); background: var(--surface); color: var(--text); font-size: 12.5px; }
  .sb-search { padding: 7px 9px; border-radius: var(--radius-m); border: 1px solid var(--border); background: var(--surface); color: var(--text); font-size: 12.5px; }
  .sb-list { overflow-y: auto; display: flex; flex-direction: column; gap: 2px; min-height: 0; }
  .sb-cat { font-size: 10px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-dim); margin: 8px 4px 2px; }
  .sb-item { display: flex; align-items: center; gap: 6px; text-align: left; background: transparent; border: 1px solid transparent; border-radius: var(--radius-m); padding: 6px 8px; cursor: pointer; color: var(--text); }
  .sb-item:hover { background: color-mix(in srgb, var(--text) 5%, transparent); }
  .sb-item.active { border-color: var(--accent); background: color-mix(in srgb, var(--accent) 8%, transparent); }
  .sb-item-name { flex: 1; min-width: 0; font-size: 12.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sb-badge { font-size: 9.5px; }
  .sb-badge-library { color: var(--status-working); }
  .sb-badge-bundled { color: var(--text-dim); }
  .sb-badge-provider { color: var(--accent); }
  .sb-readonly { font-size: 11px; color: var(--text-dim); align-self: center; }
  .sb-empty { color: var(--text-dim); font-size: 12.5px; padding: 8px; }

  .sb-main { display: flex; flex-direction: column; min-height: 0; overflow: hidden; }
  .sb-placeholder { color: var(--text-dim); padding: 20px; }
  .sb-dim { color: var(--text-dim); font-size: 12px; }
  .sb-head { display: flex; align-items: center; gap: 8px; margin-bottom: 10px; flex-wrap: wrap; }
  .sb-head h3 { margin: 0; }
  .grow { flex: 1; }
  .danger { color: var(--status-exited); }

  .sb-body { display: grid; grid-template-columns: 200px 1fr; gap: 10px; flex: 1; min-height: 0; }
  .sb-files { border: 1px solid var(--border); border-radius: var(--radius-m); overflow-y: auto; padding: 6px; display: flex; flex-direction: column; gap: 2px; }
  .sb-files-head { display: flex; align-items: center; justify-content: space-between; font-size: 10px; font-weight: 700; text-transform: uppercase; color: var(--text-dim); padding: 2px 4px 4px; }
  .sb-file-row { display: flex; align-items: center; gap: 2px; }
  .sb-file { flex: 1; min-width: 0; text-align: left; background: transparent; border: none; color: var(--text); padding: 4px 6px; border-radius: var(--radius-s); cursor: pointer; font-size: 11.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sb-file.active { background: color-mix(in srgb, var(--accent) 12%, transparent); color: var(--accent); }
  .sb-file-del { background: transparent; border: none; color: var(--text-dim); cursor: pointer; font-size: 11px; padding: 2px 4px; }

  .sb-editor { display: flex; flex-direction: column; min-height: 0; border: 1px solid var(--border); border-radius: var(--radius-m); overflow: hidden; }
  .sb-editor-head { padding: 6px 10px; border-bottom: 1px solid var(--border); background: var(--surface); font-size: 11.5px; }
  .sb-textarea { flex: 1; min-height: 0; resize: none; border: none; background: var(--term-bg, var(--surface)); color: var(--text); font-family: var(--font-mono, monospace); font-size: 12.5px; line-height: 1.5; padding: 10px; }
  .sb-view { flex: 1; min-height: 0; margin: 0; overflow: auto; padding: 10px; font-family: var(--font-mono, monospace); font-size: 12.5px; line-height: 1.55; white-space: pre-wrap; word-break: break-word; }
  .mono { font-family: var(--font-mono, monospace); }

  @media (max-width: 900px) {
    .skills-browser { grid-template-columns: 1fr; }
    .sb-body { grid-template-columns: 1fr; }
  }
</style>
