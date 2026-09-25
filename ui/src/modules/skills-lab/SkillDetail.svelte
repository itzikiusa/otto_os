<script lang="ts" module>
  export type DetailTab = 'overview' | 'edit' | 'evals' | 'usage';
</script>

<script lang="ts">
  // Skills Lab → Skills: the detail pane for one skill (grouped by name).
  //
  //   header   — name, description, category, the copies it has (Library ·
  //              Claude · Codex · Bundled) as a value picker, drift notes with
  //              "Compare with …", and the skill's actions (Review, Evaluate,
  //              Install/Update, Copy to library, Delete in ⋯)
  //   Overview — SKILL.md frontmatter as a metadata card (description,
  //              triggers, allowed tools, version…) + its files, then the body
  //              rendered through the sanitized GFM renderer
  //   Edit     — the multi-file editor (SkillEditor)
  //   Evals / Usage — SkillActivity
  import type { SkillFileEntry } from '../../lib/api/types';
  import { skillLabApi } from '../../lib/api/skillLab';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { renderMarkdownGfm } from '../../lib/md';
  import { formatBytes } from '../../lib/metric-format';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import DiffView from '../../lib/components/DiffView.svelte';
  import SkillEditor from './SkillEditor.svelte';
  import SkillActivity from './SkillActivity.svelte';
  import { metaList, parseFrontmatter, sourceLabel, type SkillGroup, type VariantSource } from './skillGroups';

  interface Props {
    group: SkillGroup;
    source: VariantSource;
    tab: DetailTab;
    wsId: string;
    /** Library SKILL.md bodies are in the list payload; others are fetched. */
    libraryBody: string | undefined;
    /** Fetched provider bodies (shared cache with the list's drift check). */
    bodyOf: (source: VariantSource) => string | undefined;
    onsource: (s: VariantSource) => void;
    ontab: (t: DetailTab) => void;
    onchanged: (select?: { name: string; source: VariantSource }) => void;
    ondeleted: () => void;
    onbody: (source: VariantSource, body: string) => void;
    onreview: () => void;
    onevaluate: () => void;
    /** The Edit tab has unsaved changes (the browser guards skill switches). */
    ondirty?: (dirty: boolean) => void;
    onopenrun?: (id: string) => void;
    /** Open one existing review in the Review tab. */
    onopenreview?: (id: string) => void;
  }
  let { group, source, tab, wsId, libraryBody, bodyOf, onsource, ontab, onchanged, ondeleted, onbody, onreview, onevaluate, ondirty, onopenrun, onopenreview }: Props = $props();

  // Unsaved edits in the Edit tab: leaving the tab (or the copy) unmounts the
  // editor, so ask first instead of silently dropping the draft.
  let editorDirty = $state(false);
  function setDirty(d: boolean): void {
    editorDirty = d;
    ondirty?.(d);
  }
  async function confirmDiscard(): Promise<boolean> {
    if (!editorDirty) return true;
    const ok = await confirmer.ask(`You have unsaved changes to ${group.name}. Leaving the editor discards them.`, { title: 'Discard unsaved changes?', confirmLabel: 'Discard', cancelLabel: 'Keep editing' });
    if (ok) setDirty(false);
    return ok;
  }
  async function goTab(t: DetailTab): Promise<void> {
    if (t === tab) return;
    if (tab === 'edit' && !(await confirmDiscard())) return;
    ontab(t);
  }
  async function goSource(s: VariantSource): Promise<void> {
    if (s === source) return;
    if (!(await confirmDiscard())) return;
    onsource(s);
  }

  const variant = $derived(group.variants.find((v) => v.source === source) ?? group.variants[0]);
  const isLibrary = $derived(variant.source === 'library');
  const isBundled = $derived(variant.source === 'bundled');
  const hasLibrary = $derived(group.variants.some((v) => v.source === 'library'));

  // ---- Load the selected copy's SKILL.md + file list ----------------------
  let body = $state<string>('');
  let files = $state<SkillFileEntry[]>([]);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let loadedKey = $state('');

  async function load(name: string, src: VariantSource): Promise<void> {
    const key = `${src}:${name}`;
    loading = true;
    loadError = null;
    try {
      if (src === 'library') {
        const [fs, f] = await Promise.all([skillLabApi.listFiles(name), skillLabApi.getFile(name, 'SKILL.md')]);
        if (`${variant.source}:${group.name}` !== key) return;
        files = fs;
        body = f.content;
      } else if (src === 'bundled') {
        const b = await skillLabApi.getBundled(name);
        if (`${variant.source}:${group.name}` !== key) return;
        files = b.files;
        body = b.body;
      } else {
        const p = await skillLabApi.getProvider(src, name);
        if (`${variant.source}:${group.name}` !== key) return;
        files = p.files;
        body = p.body;
        onbody(src, p.body);
      }
      loadedKey = key;
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => {
    const key = `${variant.source}:${group.name}`;
    if (key !== loadedKey) void load(group.name, variant.source);
  });

  const fm = $derived(parseFrontmatter(body));
  const metaMap = $derived(new Map(fm.meta));
  const rendered = $derived(renderMarkdownGfm(fm.body));
  const SKIP = new Set(['name', 'description', 'category', 'version']);
  const LIST_KEYS = new Set(['allowed-tools', 'allowed_tools', 'tools', 'triggers', 'tags', 'keywords']);
  const extraMeta = $derived(fm.meta.filter(([k]) => !SKIP.has(k)));

  // ---- Compare against the reference copy ----------------------------------
  let comparing = $state<VariantSource | null>(null);
  // A copy that can't be read ends the "Loading both copies…" wait with a
  // reason instead of leaving it up forever.
  let compareError = $state<string | null>(null);
  $effect(() => {
    void group.name;
    comparing = null;
    compareError = null;
  });
  const refBody = $derived(group.reference === 'library' ? libraryBody : bodyOf(group.reference));
  async function compare(s: VariantSource): Promise<void> {
    if (comparing !== s && tab === 'edit' && !(await confirmDiscard())) return;
    comparing = comparing === s ? null : s;
    compareError = null;
    // The diff lives on Overview, above the rendered SKILL.md.
    if (comparing && tab !== 'overview') ontab('overview');
    if (comparing && s !== 'library' && s !== 'bundled' && bodyOf(s) == null) {
      try {
        const p = await skillLabApi.getProvider(s, group.name);
        onbody(s, p.body);
      } catch (e) {
        compareError = `Couldn't read the ${sourceLabel(s)} copy: ${e instanceof Error ? e.message : String(e)}`;
      }
    }
    if (comparing && group.reference !== 'library' && bodyOf(group.reference) == null) {
      try {
        const p = await skillLabApi.getProvider(group.reference, group.name);
        onbody(group.reference, p.body);
      } catch (e) {
        compareError = `Couldn't read the ${sourceLabel(group.reference)} copy: ${e instanceof Error ? e.message : String(e)}`;
      }
    }
  }
  let bundledBody = $state<string | null>(null);
  $effect(() => {
    if (comparing === 'bundled' && bundledBody == null) {
      // On failure don't fake an empty body (that renders as "everything was
      // deleted"); say the bundled copy couldn't be read.
      void skillLabApi
        .getBundled(group.name)
        .then((b) => (bundledBody = b.body))
        .catch((e) => (compareError = `Couldn't read the bundled copy: ${e instanceof Error ? e.message : String(e)}`));
    }
  });
  $effect(() => {
    void group.name;
    bundledBody = null;
  });
  const compareAfter = $derived(comparing === 'bundled' ? bundledBody : comparing ? bodyOf(comparing) : null);
  const compareBefore = $derived(comparing === 'bundled' ? libraryBody : refBody);

  // ---- Actions ----------------------------------------------------------
  let busy = $state(false);
  async function install(): Promise<void> {
    const b = group.variants.find((v) => v.source === 'bundled');
    const updating = hasLibrary;
    if (updating && !(await confirmer.ask(`Replace the library copy of ${group.name} with bundled v${b?.bundledVersion}? Your current copy is backed up first.`, { title: 'Update from bundled', confirmLabel: 'Update', danger: false }))) return;
    busy = true;
    try {
      await skillLabApi.install(group.name);
      toasts.success(updating ? 'Library copy updated' : 'Installed to library', updating ? 'The previous copy was backed up.' : 'You can edit it now.');
      onchanged({ name: group.name, source: 'library' });
    } catch (e) {
      toasts.error(`Couldn't install ${group.name}`, e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }
  async function copyToLibrary(): Promise<void> {
    if (hasLibrary) return;
    busy = true;
    try {
      const src = bodyOf(variant.source) ?? body;
      await skillLabApi.create({ name: group.name, category: group.category === 'uncategorized' ? '' : group.category, description: group.description, body: src });
      toasts.success('Copied to library', `Only SKILL.md was copied from the ${sourceLabel(variant.source)} copy.`);
      onchanged({ name: group.name, source: 'library' });
    } catch (e) {
      toasts.error(`Couldn't copy ${group.name} to the library`, e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }
  async function remove(): Promise<void> {
    if (!(await confirmer.ask(`Delete "${group.name}" from the Otto library? Its files are removed; copies in ${group.variants.filter((v) => v.source !== 'library').map((v) => sourceLabel(v.source)).join(', ') || 'other places'} are not touched.`, { title: 'Delete skill' }))) return;
    try {
      await skillLabApi.remove(group.name);
      toasts.success('Skill deleted', group.name);
      ondeleted();
    } catch (e) {
      toasts.error(`Couldn't delete ${group.name}`, e instanceof Error ? e.message : String(e));
    }
  }
  function more(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'Evaluate', icon: 'target', action: onevaluate },
      ...(!hasLibrary && !isBundled ? [{ label: 'Copy to library', icon: 'copy' as const, action: () => void copyToLibrary() }] : []),
      ...(isLibrary ? [{ separator: true }, { label: 'Delete from library…', icon: 'trash' as const, danger: true, action: () => void remove() }] : []),
    ];
    ctxMenu.show(e, items);
  }

  // ---- Tabs (roving, arrow keys) ------------------------------------------
  const TABS: { id: DetailTab; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    // "Files", not "Edit": bundled / provider copies are read-only here.
    { id: 'edit', label: 'Files' },
    { id: 'evals', label: 'Evals' },
    { id: 'usage', label: 'Usage' },
  ];
  let tablist = $state<HTMLElement | null>(null);
  function onTabKey(e: KeyboardEvent): void {
    const i = TABS.findIndex((t) => t.id === tab);
    let n = -1;
    if (e.key === 'ArrowRight') n = (i + 1) % TABS.length;
    else if (e.key === 'ArrowLeft') n = (i - 1 + TABS.length) % TABS.length;
    else if (e.key === 'Home') n = 0;
    else if (e.key === 'End') n = TABS.length - 1;
    if (n < 0) return;
    e.preventDefault();
    void goTab(TABS[n].id).then(() => (tablist?.querySelectorAll('[role="tab"]')[TABS.findIndex((x) => x.id === tab)] as HTMLElement | undefined)?.focus());
  }

  let editorFile = $state('SKILL.md');
  function openFile(path: string): void {
    editorFile = path;
    ontab('edit');
  }

  function syncLabel(): string {
    switch (group.sync) {
      case 'in_sync':
        return 'In sync';
      case 'drifted':
        return 'Drifted';
      case 'single':
        return 'One copy';
      default:
        return 'Checking…';
    }
  }
</script>

<div class="detail" data-testid="skill-detail">
  <header class="d-head">
    <div class="d-title-row">
      <h2 class="d-name mono" dir="ltr" data-testid="skill-name">{group.name}</h2>
      <span class="sync {group.sync}" title={group.drift.join('\n') || syncLabel()}><span class="dot"></span>{syncLabel()}</span>
      <span class="grow"></span>
      <button class="btn small" onclick={onreview} data-testid="review-skill"><Icon name="eye" size={12} /> Review</button>
      {#if isBundled}
        {#if !hasLibrary || variant.bundledState === 'update_available'}
          <button class="btn small primary" disabled={busy} onclick={install}>{hasLibrary ? 'Update library copy' : 'Install to library'}</button>
        {/if}
      {:else if !hasLibrary}
        <button class="btn small primary" disabled={busy} onclick={copyToLibrary}>Copy to library</button>
      {/if}
      <button class="icon-btn" onclick={more} aria-label="More actions for {group.name}" title="More actions" aria-haspopup="menu"><Icon name="more" size={14} /></button>
    </div>
    {#if group.description}<p class="d-desc" title={group.description}>{group.description}</p>{/if}
    <div class="d-meta">
      <span class="chip">{group.category}</span>
      {#if group.variants.length === 1}
        <!-- One copy: a plain label, not a picker that looks selected. -->
        {@const v = group.variants[0]}
        <span class="chip" title="The only copy of this skill">
          {#if v.source === 'library'}<Icon name="book" size={12} />{:else if v.source === 'bundled'}<Icon name="box" size={12} />{:else}<ProviderIcon provider={v.source} size={12} />{/if}
          {sourceLabel(v.source)}
        </span>
      {:else}
      <div class="variants" role="group" aria-label="Copy to show">
        {#each group.variants as v (v.source)}
          <button class="variant" class:active={v.source === variant.source} aria-pressed={v.source === variant.source} onclick={() => goSource(v.source)} data-testid="variant-{v.source}">
            {#if v.source === 'library'}<Icon name="book" size={12} />{:else if v.source === 'bundled'}<Icon name="box" size={12} />{:else}<ProviderIcon provider={v.source} size={12} />{/if}
            {sourceLabel(v.source)}
            {#if group.driftedSources.includes(v.source)}<span class="vdot" title="Differs from {sourceLabel(group.reference)}"></span>{/if}
          </button>
        {/each}
      </div>
      {/if}
    </div>
    {#if group.drift.length > 0}
      <div class="drift" role="note">
        <Icon name="warning" size={14} />
        <ul>
          {#each group.drift as note, i (i)}
            {@const s = group.driftedSources[i]}
            <li>
              {note}
              {#if s}
                <button class="btn small ghost" aria-expanded={comparing === s} onclick={() => compare(s)}>{comparing === s ? 'Hide diff' : `Compare with ${sourceLabel(s === 'bundled' ? 'library' : group.reference)}`}</button>
              {/if}
            </li>
          {/each}
        </ul>
      </div>
    {/if}
    <div class="tabs" role="tablist" aria-label="Skill detail" tabindex="-1" bind:this={tablist} onkeydown={onTabKey}>
      {#each TABS as t (t.id)}
        <button role="tab" id="st-{t.id}" aria-selected={tab === t.id} aria-controls="sp-{t.id}" tabindex={tab === t.id ? 0 : -1} class:active={tab === t.id} onclick={() => goTab(t.id)}>{t.label}</button>
      {/each}
    </div>
  </header>

  <div class="d-body" role="tabpanel" id="sp-{tab}" aria-labelledby="st-{tab}">
    {#if comparing && tab === 'overview'}
      <section class="compare">
        <div class="compare-head">
          <span class="section-title">{comparing === 'bundled' ? 'Library → Bundled' : `${sourceLabel(group.reference)} → ${sourceLabel(comparing)}`}</span>
          <button class="icon-btn" onclick={() => (comparing = null)} aria-label="Close diff" title="Close diff"><Icon name="x" size={14} /></button>
        </div>
        {#if compareError}
          <p class="compare-err" role="alert">{compareError}</p>
        {:else if compareBefore == null || compareAfter == null}
          <p class="dim" role="status">Loading both copies…</p>
        {:else}
          <DiffView before={compareBefore} after={compareAfter} mode="word" contextLines={3} />
        {/if}
      </section>
    {/if}

    {#if loadError && (tab === 'overview' || tab === 'edit')}
      <div class="inline-error" role="alert">
        <Icon name="warning" size={14} />
        <div><strong>Couldn't open the {sourceLabel(variant.source)} copy of {group.name}.</strong> <span class="dim">{loadError}</span></div>
        <button class="btn small" onclick={() => load(group.name, variant.source)}>Retry</button>
      </div>
    {:else if tab === 'overview'}
      {#if loading && !body}
        <p class="dim" role="status">Loading {group.name}…</p>
      {:else}
        <div class="overview">
          <aside class="meta card" aria-label="Skill metadata">
            <dl>
              <!-- The header already shows the description (2 lines); repeat it here
                   only when it differs or was clipped there. -->
              {#if metaMap.get('description') && (metaMap.get('description') !== group.description || String(metaMap.get('description')).length > 160)}
                <div class="m-row wide"><dt>Description</dt><dd>{metaMap.get('description')}</dd></div>
              {/if}
              <!-- The header chip already shows the category; repeat only a mismatch. -->
              {#if metaMap.get('category') && metaMap.get('category') !== group.category}
                <div class="m-row"><dt>Category</dt><dd>{metaMap.get('category')}</dd></div>
              {/if}
              {#if metaMap.get('version')}<div class="m-row"><dt>Version</dt><dd>v{metaMap.get('version')}</dd></div>{/if}
              {#each extraMeta as [k, v] (k)}
                <div class="m-row wide">
                  <dt>{k.replace(/[-_]/g, ' ')}</dt>
                  <dd>
                    {#if Array.isArray(v) || LIST_KEYS.has(k)}
                      <span class="chips">{#each metaList(v) as item, i (i)}<span class="chip mono">{item}</span>{/each}</span>
                    {:else}
                      {v}
                    {/if}
                  </dd>
                </div>
              {/each}
              {#if fm.meta.length === 0}
                <div class="m-row wide"><dt>Frontmatter</dt><dd class="dim">None — agents only see the body. Add a description so they know when to use it.</dd></div>
              {/if}
            </dl>
            <div class="m-files">
              <div class="section-title">Files · {files.length}</div>
              <ul>
                {#each files as f (f.path)}
                  <li>
                    <button class="file-link" onclick={() => openFile(f.path)} title="Open {f.path}">
                      <Icon name="file" size={12} /><span class="mono ellipsis" dir="ltr">{f.path}</span><span class="dim size">{formatBytes(f.size)}</span>
                    </button>
                  </li>
                {/each}
              </ul>
            </div>
          </aside>
          <article class="md-body skill-md" data-testid="skill-preview">
            {#if fm.body.trim()}
              <!-- renderMarkdownGfm output goes through the allowlist sanitizer. -->
              {@html rendered}
            {:else}
              <p class="dim">SKILL.md has no body yet.</p>
            {/if}
          </article>
        </div>
      {/if}
    {:else if tab === 'edit'}
      <SkillEditor
        name={group.name}
        source={variant.source}
        {files}
        skillMd={body}
        initialFile={editorFile}
        onsaved={(next, md) => {
          files = next;
          if (md != null) {
            body = md;
            onchanged();
          }
        }}
        oninstall={install}
        oncopytolibrary={hasLibrary ? undefined : copyToLibrary}
        ondirty={setDirty}
      />
    {:else}
      <SkillActivity {group} view={tab} {wsId} {onevaluate} {onreview} {onopenrun} {onopenreview} onview={(v) => ontab(v)} />
    {/if}
  </div>
</div>

<style>
  .detail {
    container: skilldetail / inline-size;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    min-width: 0;
  }
  .d-head {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px 20px 0;
    border-bottom: 1px solid var(--border);
  }
  .d-title-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    min-width: 0;
  }
  .d-name {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
    color: var(--text);
    overflow-wrap: anywhere;
    min-width: 0;
  }
  .d-desc {
    margin: 0;
    color: var(--text-dim);
    max-width: 90ch;
    line-height: 1.45;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .d-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .variants {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .variant {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 500;
    cursor: pointer;
  }
  .variant:hover {
    background: var(--hover);
    color: var(--text);
  }
  .variant.active {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
    color: var(--text);
  }
  .vdot {
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: var(--status-warn);
  }
  .sync {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .sync .dot {
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: var(--status-idle);
  }
  .sync.in_sync {
    color: var(--success);
  }
  .sync.in_sync .dot {
    background: var(--status-working);
  }
  .sync.drifted {
    color: var(--warning);
  }
  .sync.drifted .dot {
    background: var(--status-warn);
  }
  .sync.single .dot {
    background: transparent;
    border: 1.5px solid var(--text-dim);
  }
  .drift {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 6px 10px;
    border: 1px solid color-mix(in srgb, var(--warning) 35%, transparent);
    border-radius: var(--radius-m);
    background: var(--warning-soft);
    font-size: var(--fs-s);
    color: var(--text);
  }
  .drift > :global(svg) {
    color: var(--warning);
    margin-top: 4px;
    flex: none;
  }
  .drift ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .drift li {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .tabs {
    display: flex;
    gap: 2px;
    margin-bottom: -1px;
  }
  .tabs > button {
    height: 32px;
    padding: 0 12px;
    border: none;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-weight: 500;
    cursor: pointer;
  }
  .tabs > button:hover {
    color: var(--text);
  }
  .tabs > button.active {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .d-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 16px 20px 28px;
    display: flex;
    flex-direction: column;
  }
  .compare {
    margin-bottom: 16px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 8px 10px 10px;
    max-height: 50vh;
    overflow: auto;
    flex: none;
  }
  .compare-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }
  .compare-head .section-title {
    margin: 0;
  }
  .compare-err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--danger);
    overflow-wrap: anywhere;
  }
  .overview {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 300px;
    gap: 20px;
    align-items: start;
  }
  .meta {
    order: 2;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    position: sticky;
    top: 0;
  }
  .meta dl {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .m-row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .m-row dt {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: capitalize;
  }
  .m-row dd {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text);
    line-height: 1.45;
    overflow-wrap: anywhere;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .chips .chip {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .m-files .section-title {
    margin: 0 0 4px;
  }
  .m-files ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .file-link {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 24px;
    padding: 0 4px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    cursor: pointer;
    text-align: start;
  }
  .file-link:hover {
    background: var(--hover);
  }
  .file-link :global(svg) {
    color: var(--text-dim);
    flex: none;
  }
  .ellipsis {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .size {
    font-size: var(--fs-xs);
    flex: none;
  }
  .skill-md {
    order: 1;
    min-width: 0;
    max-width: 82ch;
    color: var(--text);
  }
  .skill-md :global(h1) {
    font-size: var(--fs-l);
    margin: 0 0 10px;
  }
  .skill-md :global(h2) {
    font-size: var(--fs-m);
    margin: 20px 0 6px;
    padding-bottom: 4px;
    border-bottom: 1px solid var(--border);
  }
  .skill-md :global(h3) {
    font-size: var(--fs-m);
  }
  .skill-md :global(table) {
    border-collapse: collapse;
    display: block;
    overflow-x: auto;
    font-size: var(--fs-s);
    margin: 8px 0;
  }
  .skill-md :global(th),
  .skill-md :global(td) {
    border: 1px solid var(--border);
    padding: 4px 8px;
    text-align: start;
  }
  .skill-md :global(a) {
    color: var(--accent-text);
  }
  .skill-md :global(ul),
  .skill-md :global(ol) {
    padding-left: 0;
    padding-inline-start: 22px;
  }
  .inline-error {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-size: var(--fs-s);
  }
  .inline-error > :global(svg) {
    color: var(--danger);
  }
  .inline-error > div {
    flex: 1;
  }
  @container skilldetail (max-width: 760px) {
    .overview {
      grid-template-columns: minmax(0, 1fr);
    }
    .meta {
      order: 0;
      position: static;
    }
  }
  @media (max-width: 640px) {
    .d-head {
      padding: 12px 14px 0;
    }
    .d-body {
      padding: 12px 14px 24px;
    }
    .tabs {
      overflow-x: auto;
    }
  }
</style>
