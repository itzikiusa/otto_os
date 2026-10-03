<script lang="ts">
  // Workbench — per-user scratch files (scripts, JSON, Markdown, diagrams…)
  // with formatting, a live preview, placeholders, Send to… and the full,
  // append-only edit history. Layout: files | tabs + editor (+ preview) |
  // side panel (Placeholders or History). On a phone the files list and the
  // editor are two panes behind a segmented control.
  import { onMount, untrack } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { keyContext } from '../../lib/keys';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { uploadWorkbenchAsset } from '../../lib/api/workbench';
  import { workbench } from './workbench.svelte';
  import FileList from './FileList.svelte';
  import EditorTabs from './EditorTabs.svelte';
  import HistoryPanel from './HistoryPanel.svelte';
  import QuickOpen from './QuickOpen.svelte';
  import PlaceholdersPanel from './PlaceholdersPanel.svelte';
  import SendToButton from './send/SendToButton.svelte';
  import Preview from './preview/Preview.svelte';
  import { canPreview } from './preview/kinds';
  import { WB_LANGUAGES, cmExtFor, detectLanguage, effectiveLanguage, extensionFor } from './lib/detect';
  import { canFormat, formatContent, validateContent, type ValidationIssue } from './lib/format';

  let quickOpen = $state(false);
  let mobilePane: 'files' | 'editor' = $state('editor');
  let issue: ValidationIssue | null = $state(null);
  /** The issue came from an explicit Format (sticky until the next edit). */
  let issueFromFormat = $state(false);
  let gotoLine: number | null = $state(null);
  let gotoCol: number | null = $state(null);
  let cursor = 0;
  let formatting = $state(false);
  let dragOver = $state(false);

  const wsId = $derived(ws.currentId);
  const active = $derived(workbench.activeDoc);
  const doc = $derived(active?.doc ?? null);
  const lang = $derived(doc ? effectiveLanguage(doc.language, doc.name, active?.buffer ?? '') : 'txt');
  const detected = $derived(doc ? detectLanguage(doc.name, active?.buffer ?? '') : 'txt');
  const detectedLabel = $derived(WB_LANGUAGES.find((l) => l.id === detected)?.label ?? detected);
  const isImage = $derived(lang === 'image');
  const showPreview = $derived(!!doc && (isImage || (!!workbench.previewOn[doc.id] && canPreview(lang))));
  const editorPath = $derived(doc ? `workbench/${doc.id}.${cmExtFor(lang) || 'txt'}` : '');
  const values = $derived(doc ? (workbench.values[doc.id] ?? {}) : {});
  const isPhone = $derived(viewport.isPhone);

  /** Bound per doc so a late emit can never land in another tab. */
  const onEdit = $derived.by(() => {
    const id = doc?.id;
    return (v: string) => {
      if (id) workbench.setBuffer(id, v);
    };
  });

  // Attach to the current workspace (re-attach on switch).
  $effect(() => {
    const id = wsId;
    if (id) untrack(() => void workbench.attach(id));
  });

  onMount(() => {
    const flush = () => workbench.flushAll();
    const onVis = () => {
      if (document.visibilityState === 'hidden') flush();
    };
    window.addEventListener('beforeunload', flush);
    window.addEventListener('pagehide', flush);
    document.addEventListener('visibilitychange', onVis);
    return () => {
      window.removeEventListener('beforeunload', flush);
      window.removeEventListener('pagehide', flush);
      document.removeEventListener('visibilitychange', onVis);
      workbench.detach();
    };
  });

  // ⌘S save-now (checkpoint) · ⌘W close tab · ⌘P quick open — claimed from
  // the global key map while this page is mounted (⌘W must not ALSO close the
  // agents tab underneath).
  $effect(() => {
    const claim = (e: KeyboardEvent): boolean => {
      if (!(e.metaKey || e.ctrlKey) || e.shiftKey || ui.overlayOpen) return false;
      const k = e.key.toLowerCase();
      if (k === 's') {
        e.preventDefault();
        saveNow();
        return true;
      }
      if (k === 'w') {
        if (!workbench.active) return false;
        e.preventDefault();
        workbench.closeTab(workbench.active);
        return true;
      }
      if (k === 'p') {
        e.preventDefault();
        quickOpen = true;
        return true;
      }
      return false;
    };
    keyContext.pageChords = claim;
    return () => {
      if (keyContext.pageChords === claim) keyContext.pageChords = null;
    };
  });

  // ⇧⌥F format (no ⌘, so the global map never sees it). `code`, since ⌥
  // rewrites `key` on macOS.
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.shiftKey && e.altKey && !e.metaKey && !e.ctrlKey && e.code === 'KeyF' && !ui.overlayOpen) {
        e.preventDefault();
        void format();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  // Live validation (debounced) — cleared by an edit unless it came from Format.
  $effect(() => {
    const text = active?.buffer ?? '';
    const l = lang;
    const id = doc?.id;
    if (!id || isImage) {
      issue = null;
      return;
    }
    const t = setTimeout(() => {
      void validateContent(l, text).then((res) => {
        if (doc?.id !== id) return;
        if (issueFromFormat && res) return;
        issueFromFormat = false;
        issue = res;
      });
    }, 400);
    return () => clearTimeout(t);
  });

  // ── actions ────────────────────────────────────────────────────────────────

  async function newFile(): Promise<void> {
    try {
      await workbench.create({});
      mobilePane = 'editor';
      focusEditorSoon();
    } catch (e) {
      toasts.error("Couldn't create a file", loadErrorText(e));
    }
  }

  function focusEditorSoon(): void {
    setTimeout(() => {
      (document.querySelector('[data-testid="wb-editor"] .cm-content') as HTMLElement | null)?.focus();
    }, 50);
  }

  function open(id: string): void {
    workbench.openDoc(id);
    mobilePane = 'editor';
  }

  function saveNow(): void {
    const id = workbench.active;
    if (!id) return;
    // Always checkpoint: unchanged content seals the open autosave burst so
    // History keeps this exact state as its own version.
    void workbench.save(id, true);
  }

  async function format(): Promise<void> {
    if (!active || !doc || isImage) return;
    if (!canFormat(lang)) {
      toasts.info('Nothing to format', `No formatter for ${WB_LANGUAGES.find((l) => l.id === lang)?.label ?? lang}.`);
      return;
    }
    formatting = true;
    try {
      const res = await formatContent(lang, active.buffer);
      if (res.ok) {
        issue = null;
        issueFromFormat = false;
        if (res.changed) workbench.setBuffer(doc.id, res.text);
      } else {
        issue = { error: res.error, line: res.line, col: res.col };
        issueFromFormat = true;
      }
    } finally {
      formatting = false;
    }
  }

  function jumpToIssue(): void {
    if (!issue?.line) return;
    gotoLine = null;
    gotoCol = null;
    const line = issue.line;
    const col = issue.col ?? null;
    queueMicrotask(() => {
      gotoLine = line;
      gotoCol = col;
    });
  }

  async function rename(id: string): Promise<void> {
    const cur = workbench.metaOf(id);
    if (!cur) return;
    const name = await confirmer.promptText('New name for this file', {
      title: 'Rename file',
      confirmLabel: 'Rename',
      initial: cur.name,
    });
    if (!name || name === cur.name) return;
    try {
      await workbench.patchMeta(id, { name });
    } catch (e) {
      toasts.error("Couldn't rename", loadErrorText(e));
    }
  }

  async function setLanguage(value: string): Promise<void> {
    if (!doc) return;
    try {
      await workbench.patchMeta(doc.id, { language: value });
    } catch (e) {
      toasts.error("Couldn't change the language", loadErrorText(e));
    }
  }

  function download(): void {
    if (!doc || !active || isImage) return;
    const ext = extensionFor(lang);
    const name = /\.[a-z0-9]+$/i.test(doc.name) ? doc.name : `${doc.name}.${ext}`;
    const url = URL.createObjectURL(new Blob([active.buffer], { type: 'text/plain;charset=utf-8' }));
    const a = document.createElement('a');
    a.href = url;
    a.download = name;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  async function trashActive(): Promise<void> {
    if (!doc) return;
    const name = doc.name;
    try {
      await workbench.moveToTrash(doc.id);
      toasts.success('Moved to trash', `“${name}” and its history can be restored from the trash.`);
    } catch (e) {
      toasts.error("Couldn't move to trash", loadErrorText(e));
    }
  }

  async function reloadRemote(): Promise<void> {
    if (!doc) return;
    const ok = await confirmer.ask(
      'Load the version saved in the other window? Your unsaved edits in this tab will be discarded (everything already saved stays in History).',
      { title: 'Reload from the other window?', confirmLabel: 'Reload', danger: false },
    );
    if (ok) await workbench.reload(doc.id);
  }

  function moreMenu(e: MouseEvent): void {
    const has = !!doc;
    const items: MenuItem[] = [
      { label: 'Format', icon: 'format', hint: '⇧⌥F', disabled: !has || isImage, action: () => void format() },
      { label: 'Toggle preview', icon: 'eye', disabled: !has || !canPreview(lang) || isImage, action: () => doc && workbench.togglePreview(doc.id) },
      { label: 'History', icon: 'clock', disabled: !has, action: () => workbench.setPanel('history') },
      { label: 'Placeholders', icon: 'filter', disabled: !has, action: () => workbench.setPanel('placeholders') },
      { label: 'Quick open…', icon: 'search', hint: '⌘P', action: () => (quickOpen = true) },
      { separator: true },
      { label: 'Rename…', icon: 'edit', disabled: !has, action: () => doc && void rename(doc.id) },
      {
        label: 'Duplicate',
        icon: 'copy',
        disabled: !has,
        action: () => doc && void workbench.duplicate(doc.id).catch((err) => toasts.error("Couldn't duplicate", loadErrorText(err))),
      },
      { label: 'Download', icon: 'download', disabled: !has || isImage, action: download },
      { separator: true },
      { label: workbench.showTrash ? 'Back to files' : 'Show trash', icon: 'trash', action: toggleTrash },
      { label: 'Move to trash', icon: 'trash', danger: true, disabled: !has, action: () => void trashActive() },
    ];
    ctxMenu.show(e, items);
  }

  function toggleTrash(): void {
    workbench.showTrash = !workbench.showTrash;
    if (workbench.showTrash) void workbench.loadTrash();
    mobilePane = 'files';
  }

  // ── images: paste / drop ───────────────────────────────────────────────────

  function imageFiles(list: ArrayLike<File> | DataTransferItemList | null | undefined): File[] {
    if (!list) return [];
    const out: File[] = [];
    for (const it of Array.from(list as ArrayLike<File | DataTransferItem>)) {
      const f = it instanceof File ? it : (it as DataTransferItem).kind === 'file' ? (it as DataTransferItem).getAsFile() : null;
      if (f && f.type.startsWith('image/')) out.push(f);
    }
    return out;
  }

  async function addImages(files: File[], intoMarkdown: boolean): Promise<void> {
    const id = wsId;
    if (!id) return;
    for (const f of files) {
      try {
        const asset = await uploadWorkbenchAsset(id, f, f.type || 'application/octet-stream');
        const name = f.name || `image-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')}.${(f.type.split('/')[1] || 'png').replace('svg+xml', 'svg')}`;
        if (intoMarkdown && active && doc) {
          const snippet = `![${name}](asset:${asset.id})`;
          const at = Math.min(cursor, active.buffer.length);
          workbench.setBuffer(doc.id, active.buffer.slice(0, at) + snippet + active.buffer.slice(at));
        } else {
          await workbench.create({ name, language: 'image', content: asset.id });
        }
      } catch (e) {
        toasts.error(`Couldn't add ${f.name || 'the image'}`, loadErrorText(e));
      }
    }
  }

  function onPaste(e: ClipboardEvent): void {
    const files = imageFiles(e.clipboardData?.items);
    if (files.length === 0) return;
    e.preventDefault();
    void addImages(files, lang === 'md');
  }

  function onDragOver(e: DragEvent): void {
    if (!e.dataTransfer || !Array.from(e.dataTransfer.types).includes('Files')) return;
    e.preventDefault();
    dragOver = true;
  }

  function onDrop(e: DragEvent): void {
    dragOver = false;
    const files = imageFiles(e.dataTransfer?.files);
    if (files.length === 0) return;
    e.preventDefault();
    const onEditor = !!(e.target as Element | null)?.closest?.('[data-testid="wb-editor"]');
    void addImages(files, onEditor && lang === 'md');
  }

  function saveLabel(): string {
    if (!active || !doc) return '';
    if (active.saveError) return 'Not saved';
    if (active.saving) return 'Saving…';
    if (active.buffer !== active.saved) return 'Unsaved';
    return `Saved · rev ${doc.rev}`;
  }
</script>

<div class="wb-page">
  <PageHeader title="Workbench" icon="workbench" subtitle="Scratch files with full history">
    {#snippet actions()}
      {#if workbench.docs.length > 0 || workbench.showTrash}
        <button class="btn small primary" data-testid="wb-new" onclick={() => void newFile()} title="New scratch file">
          <Icon name="plus" size={12} /> New file
        </button>
      {/if}
      <button class="icon-btn" data-keep onclick={moreMenu} aria-label="More actions" title="More actions" aria-haspopup="menu">
        <Icon name="more" size={14} />
      </button>
    {/snippet}
  </PageHeader>

  <PageBody fill padded={false}>
    {#if !wsId}
      <EmptyState
        variant="page"
        icon="workbench"
        title="Add a workspace to get started"
        body="Workbench files belong to a workspace. Add your project folder first."
        actionLabel="Add workspace"
        actionIcon="plus"
        onaction={() => (ui.newWorkspaceOpen = true)}
      />
    {:else if !workbench.loaded && (workbench.loading || workbench.error)}
      <LoadState
        what="your workbench"
        variant="page"
        loading={workbench.loading}
        error={workbench.error}
        empty
        onretry={() => void workbench.loadList()}
      />
    {:else if workbench.loaded && workbench.docs.length === 0 && !workbench.showTrash}
      <EmptyState
        variant="page"
        icon="workbench"
        title="Create your first scratch file"
        body="Paste a script, JSON or notes — format it, preview it, fill its placeholders and send it to a database, the API client or an agent. Every save is kept in History."
      >
        <div class="wb-empty-actions">
          <button class="btn primary" data-testid="wb-new" onclick={() => void newFile()}>
            <Icon name="plus" size={13} /> New file
          </button>
          <button class="btn" onclick={toggleTrash}>Show trash</button>
        </div>
      </EmptyState>
    {:else}
      {#if isPhone}
        <div class="wb-seg" role="tablist" aria-label="Workbench view">
          <button role="tab" aria-selected={mobilePane === 'files'} class:on={mobilePane === 'files'} onclick={() => (mobilePane = 'files')}>Files</button>
          <button role="tab" aria-selected={mobilePane === 'editor'} class:on={mobilePane === 'editor'} onclick={() => (mobilePane = 'editor')}>Editor</button>
        </div>
      {/if}
      <div
        class="wb-grid"
        class:phone={isPhone}
        class:with-side={!!doc && workbench.panel !== 'none'}
        class:drag={dragOver}
        role="group"
        aria-label="Workbench"
        ondragover={onDragOver}
        ondragleave={() => (dragOver = false)}
        ondrop={onDrop}
        onpaste={onPaste}
      >
        {#if !isPhone || mobilePane === 'files'}
          <nav class="wb-col-files" aria-label="Workbench files">
            <FileList onopen={open} onnew={() => void newFile()} onrename={(id) => void rename(id)} />
          </nav>
        {/if}

        {#if !isPhone || mobilePane === 'editor'}
          <section class="wb-col-main" aria-label="Editor">
            <EditorTabs />
            {#if !active}
              <EmptyState icon="file" title="No file open" body="Pick a file from the list, or press ⌘P to open one." />
            {:else if active.loadError && !doc}
              <div class="wb-inline-err" role="alert">
                <Icon name="warning" size={13} />
                <span>Couldn't load this file: {active.loadError}</span>
                <button class="btn small" onclick={() => void workbench.retryLoad(active.id)}>Retry</button>
              </div>
            {:else if !doc}
              <div class="wb-skel" aria-busy="true" aria-label="Loading file">
                <span></span><span></span><span></span>
              </div>
            {:else}
              <div class="wb-toolbar" role="toolbar" aria-label="File tools">
                {#if isImage}
                  <span class="wb-lang-static"><Icon name="image" size={12} /> Image</span>
                {:else}
                  <label class="wb-lang">
                    <span class="sr-only">Language</span>
                    <select
                      value={doc.language && WB_LANGUAGES.some((l) => l.id === doc.language) ? doc.language : 'auto'}
                      onchange={(e) => void setLanguage((e.currentTarget as HTMLSelectElement).value)}
                      aria-label="Language"
                      title="Language"
                      data-testid="wb-lang"
                    >
                      {#each WB_LANGUAGES.filter((l) => l.id !== 'image') as l (l.id)}
                        <option value={l.id}>{l.id === 'auto' ? `Auto (${detectedLabel})` : l.label}</option>
                      {/each}
                    </select>
                  </label>
                  <button
                    class="btn small"
                    data-testid="wb-format"
                    onclick={() => void format()}
                    disabled={formatting || !canFormat(lang)}
                    title={canFormat(lang) ? 'Format (⇧⌥F)' : 'No formatter for this language'}
                  >
                    <Icon name="format" size={12} /> Format
                  </button>
                  <button
                    class="btn small wb-toggle"
                    class:on={!!workbench.previewOn[doc.id]}
                    data-testid="wb-preview-toggle"
                    aria-pressed={!!workbench.previewOn[doc.id]}
                    disabled={!canPreview(lang)}
                    onclick={() => workbench.togglePreview(doc.id)}
                    title={canPreview(lang) ? 'Toggle preview' : 'No preview for this language'}
                  >
                    <Icon name="eye" size={12} /> Preview
                  </button>
                {/if}
                <button
                  class="btn small wb-toggle"
                  class:on={workbench.panel === 'placeholders'}
                  aria-pressed={workbench.panel === 'placeholders'}
                  data-testid="wb-placeholders-toggle"
                  onclick={() => workbench.setPanel('placeholders')}
                  title="Placeholders"
                >
                  <Icon name="filter" size={12} /> Placeholders
                </button>
                <button
                  class="btn small wb-toggle"
                  class:on={workbench.panel === 'history'}
                  aria-pressed={workbench.panel === 'history'}
                  data-testid="wb-history-toggle"
                  onclick={() => workbench.setPanel('history')}
                  title="History"
                >
                  <Icon name="clock" size={12} /> History
                </button>
                {#if wsId}
                  <SendToButton ws={wsId} {doc} content={active.buffer} {lang} {values} />
                {/if}
                <span class="wb-save" class:err={!!active.saveError} class:dirty={active.buffer !== active.saved} data-testid="wb-save-state" aria-live="polite">
                  {saveLabel()}
                  {#if active.saveError}
                    <button class="btn small" onclick={() => void workbench.save(active.id, false)} title={active.saveError}>Retry</button>
                  {/if}
                </span>
              </div>

              {#if active.remoteChanged}
                <div class="wb-banner" role="status">
                  <Icon name="info" size={13} />
                  <span>This file changed in another window.</span>
                  <button class="btn small" onclick={() => void reloadRemote()}>Reload</button>
                  <button class="btn small" onclick={() => workbench.keepMine(active.id)}>Keep mine</button>
                </div>
              {/if}
              {#if active.saveError}
                <div class="wb-inline-err" role="alert">
                  <Icon name="warning" size={13} />
                  <span>Couldn't save: {active.saveError}. Your text is kept on this device until it saves.</span>
                  <button class="btn small" onclick={() => void workbench.save(active.id, false)}>Retry</button>
                </div>
              {/if}
              {#if issue}
                <div class="wb-issue" role="alert" data-testid="wb-issue">
                  <Icon name="warning" size={13} />
                  {#if issue.line}
                    <button class="wb-issue-pos" onclick={jumpToIssue} title="Go to the error">Line {issue.line}{issue.col ? `:${issue.col}` : ''}</button>
                  {/if}
                  <span class="wb-issue-msg">{issue.error}</span>
                </div>
              {/if}

              <div class="wb-split" class:previewing={showPreview && !isImage} class:image-only={isImage}>
                {#if !isImage}
                  <div class="wb-editor" data-testid="wb-editor">
                    <CodeEditor
                      path={editorPath}
                      root="workbench"
                      content={active.buffer}
                      readOnly={false}
                      onchange={onEdit}
                      onselect={(s) => (cursor = s.cursor)}
                      keepStates
                      wrap={lang === 'md' || lang === 'txt'}
                      highlightLineLimit={5000}
                      {gotoLine}
                      {gotoCol}
                      placeholder="Type or paste anything — it saves as you go."
                    />
                  </div>
                {/if}
                {#if showPreview && wsId}
                  <div class="wb-preview-pane">
                    <Preview ws={wsId} {lang} content={active.buffer} name={doc.name} />
                  </div>
                {/if}
              </div>
            {/if}
          </section>

          {#if doc && wsId && workbench.panel !== 'none'}
            <div class="wb-col-side">
              {#if workbench.panel === 'history'}
                <HistoryPanel
                  ws={wsId}
                  docId={doc.id}
                  rev={doc.rev}
                  current={active?.buffer ?? ''}
                  onrestored={(d) => workbench.applyDoc(d)}
                  onclose={() => workbench.setPanel('history')}
                />
              {:else}
                <div class="wb-side-scroll">
                  <div class="wb-side-head">
                    <h3>Placeholders</h3>
                    <button class="icon-btn" onclick={() => workbench.setPanel('placeholders')} aria-label="Close placeholders" title="Close placeholders">
                      <Icon name="x" size={12} />
                    </button>
                  </div>
                  <PlaceholdersPanel content={active?.buffer ?? ''} {lang} {values} onchange={(v) => workbench.setValues(doc.id, v)} />
                </div>
              {/if}
            </div>
          {/if}
        {/if}
      </div>
    {/if}
  </PageBody>
</div>

{#if quickOpen}
  <QuickOpen onclose={() => (quickOpen = false)} onopen={open} />
{/if}

<style>
  .wb-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .wb-empty-actions {
    display: flex;
    gap: 8px;
    justify-content: center;
    flex-wrap: wrap;
  }
  .wb-seg {
    display: flex;
    gap: 2px;
    margin: 8px 16px;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .wb-seg button {
    flex: 1;
    padding: 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .wb-seg button.on {
    background: var(--surface);
    color: var(--text);
  }
  .wb-grid {
    display: grid;
    grid-template-columns: minmax(200px, 250px) minmax(0, 1fr);
    height: 100%;
    min-height: 0;
    overflow: hidden;
  }
  .wb-grid.with-side {
    grid-template-columns: minmax(200px, 250px) minmax(0, 1fr) minmax(260px, 340px);
  }
  .wb-grid.phone,
  .wb-grid.phone.with-side {
    grid-template-columns: minmax(0, 1fr);
    grid-auto-rows: minmax(0, auto);
    overflow-y: auto;
  }
  .wb-grid.drag {
    outline: 2px dashed var(--accent);
    outline-offset: -4px;
  }
  .wb-col-files {
    min-height: 0;
    border-inline-end: 1px solid var(--border);
    background: var(--surface);
  }
  .wb-col-main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--bg);
  }
  .wb-col-side {
    min-height: 0;
    min-width: 0;
    border-inline-start: 1px solid var(--border);
    background: var(--surface);
  }
  .phone .wb-col-files,
  .phone .wb-col-side {
    border: 0;
    border-block-start: 1px solid var(--border);
  }
  .phone .wb-col-main {
    min-height: 60vh;
  }
  .wb-side-scroll {
    height: 100%;
    overflow-y: auto;
    padding: 8px 10px;
    box-sizing: border-box;
  }
  .wb-side-head {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-block-end: 6px;
  }
  .wb-side-head h3 {
    flex: 1;
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .wb-toolbar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    padding: 6px 8px;
    border-block-end: 1px solid var(--border);
    background: var(--surface);
  }
  .wb-toggle.on {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .wb-lang select {
    font-size: var(--fs-s);
    max-width: 180px;
  }
  .wb-lang-static {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .wb-save {
    margin-inline-start: auto;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .wb-save.dirty {
    color: var(--warning);
  }
  .wb-save.err {
    color: var(--danger);
  }
  .wb-banner,
  .wb-inline-err,
  .wb-issue {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    font-size: var(--fs-s);
    border-block-end: 1px solid var(--border);
  }
  .wb-banner {
    background: var(--info-soft);
    color: var(--text);
  }
  .wb-inline-err {
    background: var(--danger-soft);
    color: var(--text);
  }
  .wb-issue {
    background: var(--warning-soft);
    color: var(--text);
  }
  .wb-issue-pos {
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--accent-text);
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    text-decoration: underline;
    cursor: pointer;
  }
  .wb-issue-msg {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .wb-split {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
  }
  .wb-split.previewing {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  }
  .phone .wb-split.previewing {
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: minmax(240px, 1fr) minmax(240px, 1fr);
  }
  .wb-editor {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  .wb-editor > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .wb-preview-pane {
    min-width: 0;
    min-height: 0;
    overflow: auto;
    border-inline-start: 1px solid var(--border);
    background: var(--surface);
  }
  .image-only .wb-preview-pane {
    border: 0;
  }
  .phone .wb-preview-pane {
    border: 0;
    border-block-start: 1px solid var(--border);
  }
  .wb-skel {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 16px;
  }
  .wb-skel span {
    height: 12px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .wb-skel span:nth-child(2) {
    width: 70%;
  }
  .wb-skel span:nth-child(3) {
    width: 45%;
  }
  @media (prefers-reduced-motion: no-preference) {
    .wb-skel span {
      animation: wb-pulse 1.2s ease-in-out infinite;
    }
  }
  @keyframes wb-pulse {
    50% {
      opacity: 0.5;
    }
  }
</style>
