<script lang="ts">
  // IntelliJ IDEA / GoLand Two-Pane Sectioned Settings & Project Structure Modal (`⌘,`).
  import { untrack } from 'svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import PaneDivider from '../../../lib/components/PaneDivider.svelte';
  import { loadPaneWidth } from '../../../lib/paneResizer';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { ide } from '../ideStore.svelte';
  import IdeBuildAndToolsSection from './IdeBuildAndToolsSection.svelte';
  import IdeEditorAppearanceSection from './IdeEditorAppearanceSection.svelte';
  import IdeLanguagesSection from './IdeLanguagesSection.svelte';
  import IdeProjectStructureSection from './IdeProjectStructureSection.svelte';
  import {
    filterSettingsTree,
    findSettingsNode,
    ideSettings,
    normalizeSectionId,
    SETTINGS_TREE,
  } from './ideSettingsStore.svelte';

  const NAV_W_KEY = 'otto.ide.settingsNavW';
  let navW = $state(loadPaneWidth(NAV_W_KEY, 250, 190, 380));
  // Narrow (a phone): categories and the page are one screen each — list →
  // page with a back button — instead of a 250 px list beside a 100 px page.
  let modalW = $state(0);
  const narrow = $derived(modalW > 0 && modalW < 600);
  let showList = $state(false);
  let searchQuery = $state('');
  let expandedGroups = $state<Record<string, boolean>>({
    project: true,
    appearance: true,
    editor: true,
    vcs: true,
    build: true,
    lang: true,
  });
  let activeId = $state<string>('project.project');
  let navHistory = $state<string[]>(['project.project']);
  let navIdx = $state(0);
  let applying = $state(false);

  // Open where the person left off (else Editor › General & Font). `project.project` is the
  // store's "nothing asked for" default, so only another section is a real deep link.
  const LS_SECTION = 'otto.ide.settingsSection';
  function rememberedSection(): string {
    try {
      return localStorage.getItem(LS_SECTION) || 'editor.font';
    } catch {
      return 'editor.font';
    }
  }
  function rememberSection(id: string): void {
    try {
      localStorage.setItem(LS_SECTION, id);
    } catch {
      /* private window: the default next time */
    }
  }

  $effect(() => {
    const raw = ide.settingsSection;
    const requested = normalizeSectionId(raw && raw !== 'project.project' ? raw : rememberedSection());
    untrack(() => {
      ideSettings.syncOnOpen();
      activeId = requested;
      navHistory = [requested];
      navIdx = 0;
      const found = findSettingsNode(requested);
      if (found?.parent) {
        expandedGroups = { ...expandedGroups, [found.parent.id]: true };
      }
      if (ide.hasTarget) {
        void ide.loadToolchains({ includeCatalog: true });
        if (!ide.projectEnv && !ide.envLoading) void ide.loadEnvironments();
        if (auth.isRoot && !ide.shellEnv && !ide.shellEnvLoading && !ide.shellEnvAttempted) {
          void ide.loadShellEnv(false);
        }
      }
      if (ide.plugins.length === 0 && !ide.pluginsLoading) void ide.loadPlugins();
    });
  });

  $effect(() => {
    void ide.projectEnv;
    untrack(() => ideSettings.syncEnvFromStore());
  });

  const visibleTree = $derived(filterSettingsTree(searchQuery));

  // Auto-select first matching leaf when searching if activeId is filtered out.
  $effect(() => {
    const q = searchQuery.trim();
    if (!q) return;
    const flatIds: string[] = [];
    for (const top of visibleTree) {
      flatIds.push(top.id);
      for (const c of top.children ?? []) flatIds.push(c.id);
    }
    if (flatIds.length > 0 && !flatIds.includes(activeId)) {
      const firstLeaf = visibleTree[0]?.children?.[0]?.id ?? visibleTree[0]?.id;
      if (firstLeaf) activeId = firstLeaf;
    }
  });

  const currentLookup = $derived(findSettingsNode(activeId) ?? findSettingsNode('project.project')!);
  const canGoBack = $derived(navIdx > 0);
  const canGoForward = $derived(navIdx >= 0 && navIdx < navHistory.length - 1);

  function selectSection(nextId: string, pushHistory = true): void {
    const clean = normalizeSectionId(nextId);
    showList = false;
    activeId = clean;
    rememberSection(clean);
    ide.settingsSection = clean;
    const hit = findSettingsNode(clean);
    if (hit?.parent) expandedGroups = { ...expandedGroups, [hit.parent.id]: true };
    else if (hit?.node.children?.length) expandedGroups = { ...expandedGroups, [hit.node.id]: true };
    if (pushHistory && navHistory[navIdx] !== clean) {
      const trimmed = navHistory.slice(0, navIdx + 1);
      trimmed.push(clean);
      navHistory = trimmed;
      navIdx = trimmed.length - 1;
    }
  }

  function goBack(): void {
    if (!canGoBack) return;
    navIdx -= 1;
    const target = navHistory[navIdx];
    if (target) {
      activeId = target;
      rememberSection(target);
      ide.settingsSection = target;
    }
  }

  function goForward(): void {
    if (!canGoForward) return;
    navIdx += 1;
    const target = navHistory[navIdx];
    if (target) {
      activeId = target;
      rememberSection(target);
      ide.settingsSection = target;
    }
  }

  function toggleExpand(groupId: string, e: MouseEvent): void {
    e.stopPropagation();
    expandedGroups = { ...expandedGroups, [groupId]: !expandedGroups[groupId] };
  }

  function visibleNavOrder(): string[] {
    const out: string[] = [];
    const searching = searchQuery.trim().length > 0;
    for (const top of visibleTree) {
      out.push(top.id);
      if ((searching || expandedGroups[top.id]) && top.children?.length) {
        for (const c of top.children) out.push(c.id);
      }
    }
    return out;
  }

  function onTreeKeydown(e: KeyboardEvent): void {
    const order = visibleNavOrder();
    const idx = order.indexOf(activeId);
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      const next = order[Math.min(order.length - 1, idx + 1)];
      if (next) selectSection(next);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      const prev = order[Math.max(0, idx - 1)];
      if (prev) selectSection(prev);
    } else if (e.key === 'ArrowRight') {
      const top = SETTINGS_TREE.find((t) => t.id === activeId);
      if (top?.children?.length) {
        e.preventDefault();
        if (!expandedGroups[top.id]) expandedGroups = { ...expandedGroups, [top.id]: true };
        else selectSection(top.children[0].id);
      }
    } else if (e.key === 'ArrowLeft') {
      const hit = findSettingsNode(activeId);
      if (hit?.node.children?.length && expandedGroups[hit.node.id]) {
        e.preventDefault();
        expandedGroups = { ...expandedGroups, [hit.node.id]: false };
      } else if (hit?.parent) {
        e.preventDefault();
        selectSection(hit.parent.id);
      }
    }
  }

  async function handleApply(): Promise<void> {
    if (!ideSettings.isDirty || applying) return;
    applying = true;
    try {
      await ideSettings.apply();
    } finally {
      applying = false;
    }
  }

  async function handleOk(): Promise<void> {
    applying = true;
    try {
      if (ideSettings.isDirty) {
        const ok = await ideSettings.apply();
        if (!ok) return;
      }
      ide.settingsOpen = false;
    } finally {
      applying = false;
    }
  }
</script>

<Modal
  title="Settings & Project Structure"
  width={960}
  height={660}
  onclose={() => ideSettings.cancel()}
  dismissable={!applying}
>
  <div class="st-modal" class:narrow class:list={narrow && showList} data-testid="ide-settings-modal" bind:clientWidth={modalW}>
    <aside class="st-left" style:inline-size={narrow ? undefined : `${navW}px`} aria-label="Settings categories">
      <div class="st-search-box">
        <Icon name="search" size={13} />
        <input
          class="st-search-in"
          type="search"
          dir="ltr"
          aria-label="Search settings"
          placeholder="Search settings (goroot, jdk, font, inlay…)"
          bind:value={searchQuery}
          data-autofocus
          data-testid="ide-settings-search"
        />
      </div>

      <div
        class="st-tree"
        role="tree"
        tabindex="0"
        aria-label="Settings sections"
        onkeydown={onTreeKeydown}
        data-testid="ide-settings-tree"
      >
        {#each visibleTree as group (group.id)}
          {@const hasKids = Boolean(group.children?.length)}
          {@const open = searchQuery.trim().length > 0 || Boolean(expandedGroups[group.id])}
          <div class="st-tree-group" role="treeitem" aria-expanded={hasKids ? open : undefined} aria-selected={activeId === group.id}>
            <div class="st-tree-row" class:is-active={activeId === group.id}>
              {#if hasKids}
                <button
                  type="button"
                  class="st-chev-btn"
                  aria-label={open ? `Collapse ${group.label}` : `Expand ${group.label}`}
                  title={open ? 'Collapse' : 'Expand'}
                  onclick={(e) => toggleExpand(group.id, e)}
                >
                  <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} noflip />
                </button>
              {:else}
                <span class="st-chev-spacer" aria-hidden="true"></span>
              {/if}
              <button
                type="button"
                class="st-node-btn is-parent"
                onclick={() => selectSection(group.id)}
                data-testid={`ide-settings-nav-${group.id}`}
              >
                <span>{group.label}</span>
                {#if group.id === 'plugins' && ide.plugins.length > 0}
                  <Badge tone="accent" variant="outline" label={String(ide.plugins.length)} />
                {/if}
              </button>
            </div>

            {#if hasKids && open}
              <div class="st-children" role="group">
                {#each group.children ?? [] as leaf (leaf.id)}
                  <button
                    type="button"
                    class="st-node-btn is-leaf"
                    class:is-active={activeId === leaf.id}
                    role="treeitem"
                    aria-selected={activeId === leaf.id}
                    onclick={() => selectSection(leaf.id)}
                    data-testid={`ide-settings-nav-${leaf.id}`}
                  >
                    <span>{leaf.label}</span>
                  </button>
                {/each}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    </aside>

    {#if !narrow}<PaneDivider
      bind:width={navW}
      storageKey={NAV_W_KEY}
      min={190}
      max={380}
      defaultWidth={250}
      label="Resize settings navigation"
    />{/if}

    <section class="st-right" aria-label="Selected settings page">
      <header class="st-right-hdr">
        {#if narrow}
          <button type="button" class="btn small ghost st-list-btn" onclick={() => (showList = true)} data-testid="ide-settings-categories-btn">
            <Icon name="chevronLeft" size={14} />
            All settings
          </button>
        {/if}
        <div class="st-nav-arrows" class:hidden-narrow={narrow}>
          <button
            type="button"
            class="icon-btn"
            aria-label="Back to previous settings page"
            title="Back"
            disabled={!canGoBack}
            onclick={goBack}
            data-testid="ide-settings-back-btn"
          >
            <Icon name="chevronLeft" size={14} />
          </button>
          <button
            type="button"
            class="icon-btn"
            aria-label="Forward to next settings page"
            title="Forward"
            disabled={!canGoForward}
            onclick={goForward}
            data-testid="ide-settings-forward-btn"
          >
            <Icon name="chevronRight" size={14} />
          </button>
        </div>

        <nav class="st-crumbs" aria-label="Settings breadcrumb" data-testid="ide-settings-breadcrumb">
          {#if currentLookup.parent}
            <button type="button" class="st-crumb-link" onclick={() => selectSection(currentLookup.parent!.id)}>
              {currentLookup.parent.label}
            </button>
            <Icon name="chevronRight" size={12} />
          {/if}
          <span class="st-crumb-cur">{currentLookup.node.label}</span>
        </nav>
      </header>

      <div class="st-right-scroll">
        {#if currentLookup.node.children?.length}
          <div class="st-overview" data-testid="ide-settings-overview">
            <p class="st-ov-desc">{currentLookup.node.desc}</p>
            <div class="st-ov-links">
              {#each currentLookup.node.children as child (child.id)}
                <button
                  type="button"
                  class="st-ov-card"
                  onclick={() => selectSection(child.id)}
                  data-testid={`ide-settings-child-link-${child.id}`}
                >
                  <span class="st-ov-link">{child.label}</span>
                  <span class="st-ov-sub">{child.desc}</span>
                </button>
              {/each}
            </div>
          </div>
        {:else if activeId.startsWith('project.')}
          <IdeProjectStructureSection section={activeId} onnavigate={(id) => selectSection(id)} />
        {:else if activeId.startsWith('lang.')}
          <IdeLanguagesSection section={activeId} onnavigate={(id) => selectSection(id)} />
        {:else if activeId.startsWith('build.') || activeId === 'plugins'}
          <IdeBuildAndToolsSection section={activeId} />
        {:else}
          <IdeEditorAppearanceSection section={activeId} />
        {/if}
      </div>
    </section>
  </div>

  {#snippet footer()}
    <button
      type="button"
      class="icon-btn st-help-btn"
      aria-label="Keyboard shortcuts reference"
      title="Open Keymap & keyboard shortcuts reference"
      onclick={() => selectSection('keymap')}
      data-testid="ide-settings-help-btn"
    >
      <Icon name="info" size={14} />
    </button>
    <div class="st-foot-right">
      <button type="button" class="btn ghost" onclick={() => ideSettings.cancel()} data-testid="ide-settings-cancel">
        Cancel
      </button>
      <button
        type="button"
        class="btn"
        disabled={!ideSettings.isDirty || applying}
        onclick={() => void handleApply()}
        data-testid="ide-settings-apply"
      >
        Apply
      </button>
      <button
        type="button"
        class="btn primary"
        disabled={applying}
        onclick={() => void handleOk()}
        data-testid="ide-settings-ok"
      >
        OK
      </button>
    </div>
  {/snippet}
</Modal>

<style>
  /* Bleed over the Modal body's padding (4 px top, 16 px sides and bottom) exactly: a flat
     −16 px clipped the search field at the top and left a 32 px band above the footer. */
  .st-modal { display: flex; block-size: calc(100% + 20px); min-block-size: 460px; min-inline-size: 0; margin: -4px -16px -16px; overflow: hidden; }
  .st-modal.narrow { min-block-size: 0; }
  .st-modal.narrow .st-left { flex: 1; }
  .st-modal.narrow:not(.list) .st-left, .st-modal.narrow.list .st-right { display: none; }
  .st-nav-arrows.hidden-narrow { display: none; }
  .st-list-btn { min-block-size: 36px; flex-shrink: 0; }
  /* The -16px modal margin tucks the column's top under the header: pad it back so the search box isn't clipped. */
  .st-left { flex-shrink: 0; display: flex; flex-direction: column; background: var(--surface-2); min-inline-size: 0; min-block-size: 0; padding-block-start: 16px; }
  .st-search-box {
    display: flex; align-items: center; gap: 6px; margin: 10px 10px 6px; padding: 6px 8px;
    border: 1px solid var(--control-border); border-radius: var(--radius-s); background: var(--surface); color: var(--text-dim);
  }
  .st-search-box:focus-within { border-color: var(--accent); }
  .st-search-in { flex: 1; min-inline-size: 0; border: 0; background: transparent; color: var(--text); font-size: var(--fs-xs); outline: none; }
  .st-tree { flex: 1; min-block-size: 0; overflow-y: auto; padding: 4px 8px 10px; display: flex; flex-direction: column; gap: 2px; outline: none; }
  .st-tree:focus-visible { outline: 2px solid var(--accent-text); outline-offset: -2px; border-radius: var(--radius-s); }
  .st-tree-row { display: flex; align-items: center; border-radius: var(--radius-s); }
  .st-tree-row:hover { background: var(--hover); }
  .st-tree-row.is-active { background: var(--accent-soft); }
  .st-chev-btn { display: inline-flex; align-items: center; justify-content: center; inline-size: 20px; block-size: 24px; border: 0; background: transparent; color: var(--text-dim); cursor: pointer; padding: 0; flex-shrink: 0; }
  .st-chev-spacer { inline-size: 20px; flex-shrink: 0; }
  .st-node-btn {
    flex: 1; min-inline-size: 0; display: flex; align-items: center; justify-content: space-between; gap: 6px;
    padding-block: 4px; padding-inline: 4px 8px; border: 0; border-radius: var(--radius-s);
    background: transparent; color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer;
  }
  .st-node-btn.is-parent { font-weight: 600; }
  .st-children { display: flex; flex-direction: column; gap: 2px; padding-inline-start: 20px; margin-block-start: 2px; }
  .st-node-btn.is-leaf { padding-inline: 8px; font-weight: 400; color: var(--text-dim); }
  .st-node-btn.is-leaf:hover { background: var(--hover); color: var(--text); }
  .st-node-btn.is-leaf.is-active, .st-tree-row.is-active .st-node-btn { background: var(--accent-soft); color: var(--accent-text); font-weight: 600; }
  .st-right { flex: 1; min-inline-size: 0; min-block-size: 0; display: flex; flex-direction: column; background: var(--bg); }
  .st-right-hdr {
    display: flex; align-items: center; gap: 10px; padding: 8px 16px;
    border-block-end: 1px solid var(--border); background: var(--surface); flex-shrink: 0;
  }
  .st-nav-arrows { display: inline-flex; align-items: center; gap: 2px; }
  .st-crumbs { display: flex; align-items: center; gap: 6px; font-size: var(--fs-s); min-inline-size: 0; }
  .st-crumb-link { border: 0; background: transparent; padding: 0; color: var(--accent-text); font-size: var(--fs-s); font-weight: 500; cursor: pointer; }
  .st-crumb-link:hover { text-decoration: underline; }
  .st-crumb-cur { font-weight: 600; color: var(--text); }
  .st-right-scroll { flex: 1; min-block-size: 0; overflow-y: auto; padding: 16px; }
  .st-overview { display: flex; flex-direction: column; gap: 14px; }
  .st-ov-desc { margin: 0; font-size: var(--fs-s); color: var(--text-dim); line-height: 1.5; }
  .st-ov-links { display: flex; flex-direction: column; gap: 8px; }
  .st-ov-card {
    display: flex; flex-direction: column; align-items: flex-start; gap: 2px; padding: 10px 12px;
    border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface); text-align: start; cursor: pointer;
  }
  .st-ov-card:hover { border-color: var(--accent-line); background: var(--hover); }
  .st-ov-link { font-size: var(--fs-s); font-weight: 600; color: var(--accent-text); }
  .st-ov-sub { font-size: var(--fs-xs); color: var(--text-dim); }
  .st-help-btn { margin-inline-end: auto; }
  .st-foot-right { display: flex; align-items: center; gap: 8px; }
</style>
