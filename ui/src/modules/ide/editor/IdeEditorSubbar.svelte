<script lang="ts">
  // Locked 30px crumbs row (CLS = 0): path › symbol crumbs, then only what applies to THIS
  // file — Go build tags when it has some, JSON tools for JSON, notices, the selection's
  // Agent / Edit / Wrap — and ONE ⋯ with the rest (ideEditorMoreMenu). Line/column,
  // problems, language and the language server live in the status strip under the dock.
  import { copyWithToast } from '../../../lib/clipboard';
  import Badge from '../../../lib/components/Badge.svelte';
  import { exceedsIdeDraftLimit } from '../../../lib/components/cm-ide-drafts';
  import { IDE_LSP_LINK_EVENT, retryLspNow, type IdeLspLinkDetail } from '../../../lib/components/cm-ide-lsp-reconnect';
  import { IDE_PREFS_EVENT, readIdePreferences } from '../../../lib/components/cm-ide-prefs';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { extractGoBuildTagsFromSource } from '../../../lib/goBuildTags';
  import type { Session } from '../../../lib/api/types';
  import { ws as wsStore } from '../../../lib/stores/workspace.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { ide, type IdeOpenTab, type IdeSymbolNode } from '../ideStore.svelte';
  import { ideAgent } from '../store/ideAgentController.svelte';
  import { ideLayout } from '../shell/ideLayout.svelte';
  import { ideCollabNotices } from '../store/ideCollabNotices.svelte';
  import { ideEditorChrome } from './ideEditorChrome.svelte';
  import { pushIdeNavLocation, revealInIdeProjectTree } from './ideEditorStoreBridge.svelte';
  import type { IdeOutlineNode } from './ideLangAnalyzer';
  import { collapseIdeCrumbs, ideCrumbAbsPath, ideSymbolChainAt, ideSymbolSiblings } from './ideCrumbs';
  import { openIdeEditorMoreMenu } from './ideEditorMoreMenu';
  import { isMultiLine } from '../filetools/send/pasteSafety';
  import SessionPickerModal from '../filetools/send/SessionPickerModal.svelte';

  interface Props {
    activeTab: IdeOpenTab;
    pathSegments: string[];
    effectiveSymbols: IdeSymbolNode[];
    activeJsonPath: string | null;
    sdkLabel: string | null;
    activeDocKind: string;
    effectiveViewMode: 'table' | 'text';
    hasNonEmptySelection: boolean;
    blameActive: boolean;
    onapplyjson: (nextText: string) => void;
    onjumpsymbol: (delta: -1 | 1) => void;
    ongotoline: (line: number, col: number) => void;
    onopenstructure: () => void;
    ontoggleblame: () => void;
    onopenquickfix: (e: MouseEvent) => void;
    onfocusagent: () => void;
  }

  let {
    activeTab, pathSegments, effectiveSymbols, activeJsonPath, sdkLabel,
    activeDocKind, effectiveViewMode, hasNonEmptySelection, blameActive,
    onapplyjson, onjumpsymbol, ongotoline, onopenstructure, ontoggleblame,
    onopenquickfix, onfocusagent,
  }: Props = $props();

  // Touch devices have no keyboard shortcuts to show: no "(⌘…)" hints there.
  const coarsePointer = typeof window !== 'undefined' && Boolean(window.matchMedia?.('(pointer: coarse)').matches);
  const kh = (keys: string, sep = ' ('): string => (coarsePointer ? '' : sep === ' (' ? ` (${keys})` : `${sep}${keys}`);

  let linePrefix = $state('');
  let lineSuffix = $state('');
  let sendPicking = $state(false);
  // "Path crumbs" (Settings › Editor, ⋯ › Appearance): off hides the path and symbol trail.
  let showCrumbs = $state(readIdePreferences().showBreadcrumbs);
  $effect(() => {
    const onPrefs = (): void => { showCrumbs = readIdePreferences().showBreadcrumbs; };
    window.addEventListener(IDE_PREFS_EVENT, onPrefs);
    return () => window.removeEventListener(IDE_PREFS_EVENT, onPrefs);
  });

  const crumbs = $derived(collapseIdeCrumbs(pathSegments));
  const symbolChain = $derived(ideSymbolChainAt(effectiveSymbols, ide.cursorInfo.line).slice(-3));
  const isGoTab = $derived(activeTab.language === 'go' || activeTab.path.toLowerCase().endsWith('.go'));
  const repoGoTags = $derived(isGoTab ? ide.getRepoGoBuildTags(activeTab.path) : []);
  const fileGoTags = $derived(isGoTab ? extractGoBuildTagsFromSource(activeTab.content) : []);
  const missingGoTags = $derived(fileGoTags.filter((t) => !repoGoTags.includes(t)));
  // The language-server socket dropped (daemon restart, server evicted): a quiet note while
  // it reattaches, and a Retry once the attempts ran out, instead of features going silent.
  let lspLink = $state<IdeLspLinkDetail | null>(null);
  $effect(() => {
    const onLink = (e: Event): void => {
      const d = (e as CustomEvent<IdeLspLinkDetail>).detail;
      if (!d?.rootPath) return;
      if (d.state === 'reconnecting' || d.state === 'waiting' || d.state === 'lost') lspLink = d;
      else if ((d.state === 'connected' || d.state === 'ready') && lspLink?.rootPath === d.rootPath) lspLink = null;
    };
    window.addEventListener(IDE_LSP_LINK_EVENT, onLink);
    return () => window.removeEventListener(IDE_LSP_LINK_EVENT, onLink);
  });
  const lspLinkHere = $derived.by(() => {
    const l = lspLink;
    return l && (activeTab.path === l.rootPath || activeTab.path.startsWith(`${l.rootPath}/`)) ? l : null;
  });
  // Unsaved edits to a very large file get no crash-recovery draft: say so while it is dirty.
  const noDraftKept = $derived(Boolean(activeTab.dirty) && exceedsIdeDraftLimit(activeTab.savedContent, activeTab.content));
  // Stacked layout: the row is hidden (the code gets it back) unless the person asked for it
  // from the tab strip's ▾ menu, or a notice they must see is up.
  const hasNotice = $derived(
    Boolean(lspLinkHere) || noDraftKept ||
      (ide.collabIsGuest && (ideCollabNotices.roomSync !== null || Boolean(ideCollabNotices.saveRefusedByPath[activeTab.path]))),
  );
  const stackCollapsed = $derived(ideLayout.stack && !ideEditorChrome.subbarShown && !hasNotice);

  async function enableMissingGoTags(): Promise<void> {
    if (missingGoTags.length === 0) return;
    await ide.setRepoGoBuildTags(activeTab.path, [...repoGoTags, ...missingGoTags]);
  }

  async function promptAddGoBuildTag(): Promise<void> {
    const next = await confirmer.promptText('Go build tags for this repository (comma-separated, e.g. component, integration)', {
      title: 'Go build tags',
      confirmLabel: 'Apply tags',
      initial: repoGoTags.join(', '),
      placeholder: 'component, integration',
    });
    if (next === null) return;
    await ide.setRepoGoBuildTags(activeTab.path, next);
  }

  function openGoBuildTagsMenu(e: MouseEvent): void {
    const anchor = e.currentTarget as HTMLElement | null;
    const allKnown = Array.from(new Set([...repoGoTags, ...fileGoTags]));
    const items: MenuItem[] = allKnown.map((tag) => {
      const enabled = repoGoTags.includes(tag);
      return {
        label: `Tag: ${tag}`,
        icon: enabled ? 'check' : 'tag',
        hint: enabled ? 'active' : 'in file',
        action: () => {
          const next = enabled ? repoGoTags.filter((t) => t !== tag) : [...repoGoTags, tag];
          void ide.setRepoGoBuildTags(activeTab.path, next);
        },
      };
    });
    if (items.length > 0) items.push({ separator: true });
    items.push({ label: 'Set Go build tags…', icon: 'plus', action: () => void promptAddGoBuildTag() });
    if (repoGoTags.length > 0) {
      items.push({
        label: 'Clear Go build tags',
        icon: 'x',
        action: () => void ide.setRepoGoBuildTags(activeTab.path, []),
      });
    }
    ctxMenu.showAt(anchor, items, { align: 'end' });
  }

  function jumpTo(s: IdeSymbolNode): void {
    pushIdeNavLocation(activeTab.path, ide.cursorInfo.line, ide.cursorInfo.col);
    ongotoline(s.line, (s as IdeOutlineNode).column || 1);
  }

  /** A symbol crumb opens its siblings (the level it sits in), then "Search file structure…". */
  function openSiblingMenu(e: MouseEvent, levelIdx: number): void {
    const anchor = e.currentTarget as HTMLElement | null;
    const full = ideSymbolChainAt(effectiveSymbols, ide.cursorInfo.line);
    const offset = full.length - symbolChain.length;
    const siblings = ideSymbolSiblings(effectiveSymbols, full, levelIdx + offset);
    if (!siblings.length) { onopenstructure(); return; }
    const activeSym = full[levelIdx + offset];
    const items: MenuItem[] = siblings.slice(0, 50).map((s) => ({
      label: s.name,
      icon: s.line === activeSym?.line ? 'check' : 'function',
      hint: `L${s.line}`,
      action: () => jumpTo(s),
    }));
    items.push({ separator: true }, { label: 'Search file structure…', icon: 'search', hint: coarsePointer ? undefined : '⌘F12', action: onopenstructure });
    ctxMenu.showAt(anchor, items, { align: 'start' });
  }

  function openHiddenFoldersMenu(e: MouseEvent, hidden: { label: string; index: number }[]): void {
    const anchor = e.currentTarget as HTMLElement | null;
    const items: MenuItem[] = hidden.map((h) => ({
      label: h.label,
      icon: 'folder',
      action: () => void revealInIdeProjectTree(ideCrumbAbsPath(activeTab.path, pathSegments.length, h.index)),
    }));
    ctxMenu.showAt(anchor, items, { align: 'start' });
  }

  function openMoreMenu(e: MouseEvent): void {
    const anchor = e.currentTarget as HTMLElement | null;
    const r = anchor?.getBoundingClientRect();
    void openIdeEditorMoreMenu(anchor, {
      tab: activeTab,
      hasSymbols: effectiveSymbols.length > 0,
      hasSelection: hasNonEmptySelection && effectiveViewMode === 'text',
      blameActive,
      coarse: coarsePointer,
      isGo: isGoTab,
      onjumpsymbol,
      onopenstructure,
      ontoggleblame,
      onquickfix: () => onopenquickfix(new MouseEvent('click', { clientX: (r?.right ?? 480) - 20, clientY: r?.bottom ?? 120 })),
      onsetgotags: () => void promptAddGoBuildTag(),
      onsendsession: () => (sendPicking = true),
    });
  }

  async function pasteToSession(s: Session): Promise<void> {
    sendPicking = false;
    const ctx = { name: activeTab.name, content: activeTab.content, lang: activeTab.path.split('.').pop()?.toLowerCase() ?? '' };
    await (await import('../filetools/send/sendTo')).pasteIntoSession(ctx, s);
  }

  async function applyJsonTransform(mode: 'pretty' | 'minify' | 'sort'): Promise<void> {
    if (activeTab.readOnly) return;
    const { transformJsonText } = await import('./ideLangAnalyzer');
    const res = transformJsonText(activeTab.content, mode, activeDocKind === 'jsonl');
    if (!res.ok) { toasts.error('Invalid JSON', res.error); return; }
    onapplyjson(res.text);
  }

  function applyLineWrap(): void {
    if (!linePrefix && !lineSuffix) return;
    ide.applyPrefixSuffix(linePrefix, lineSuffix);
  }
</script>

<div class="ide-subbar" class:stack-collapsed={stackCollapsed} class:asked={ideLayout.stack && ideEditorChrome.subbarShown} data-testid="ide-editor-subbar">
  <div class="ide-crumbs" dir="ltr" data-testid="ide-path-crumbs">
    {#if showCrumbs}
      {#each crumbs as c, idx (c.kind === 'seg' ? `s${c.index}` : 'more')}
        {#if idx > 0}<span class="crumb-sep" aria-hidden="true"><Icon name="chevronRight" size={12} noflip /></span>{/if}
        {#if c.kind === 'more'}
          <button type="button" class="crumb-btn crumb-more" aria-label={`${c.hidden.length} more folders`} title={c.hidden.map((h) => h.label).join(' / ')} onclick={(e) => openHiddenFoldersMenu(e, c.hidden)} data-testid="ide-crumb-more">…</button>
        {:else}
          <button type="button" class="crumb-btn" class:last={c.last} onclick={() => void revealInIdeProjectTree(activeTab.path)} title="Reveal in project tree" data-testid={c.last ? 'ide-crumb-file' : undefined}>{c.label}</button>
        {/if}
      {/each}
      {#each symbolChain as sym, idx (`${sym.name}:${sym.line}`)}
        <span class="crumb-sep" aria-hidden="true"><Icon name="chevronRight" size={12} noflip /></span>
        <button type="button" class="crumb-sym-btn" onclick={(e) => openSiblingMenu(e, idx)} title={`Jump to symbol${kh('⌘F12')}`} data-testid="ide-crumb-symbol">
          <Icon name="function" size={12} /><span>{sym.name}</span>
        </button>
      {/each}
      {#if activeJsonPath && activeJsonPath !== '$'}
        <span class="crumb-sep" aria-hidden="true"><Icon name="chevronRight" size={12} noflip /></span>
        <button type="button" class="crumb-sym-btn mono" onclick={() => void copyWithToast(activeJsonPath, 'JSONPath')} title="Click to copy JSONPath" data-testid="ide-jsonpath-crumb">{activeJsonPath}</button>
      {/if}
    {/if}
    {#if sdkLabel}<Badge variant="outline" tone="info" label={sdkLabel} />{/if}
    {#if activeTab.binary}<Badge variant="outline" tone="warn" label="Binary or not UTF-8 text" title="Otto shows UTF-8 text only. This file has bytes that aren’t UTF-8 text, so it is read-only and may look blank or garbled." testid="ide-binary-badge" />
    {:else if activeTab.truncated}<Badge variant="outline" tone="warn" label="Truncated · first 4 MB" title="Only the first 4 MB of this file is shown, read-only. Open it in another editor to see or edit the rest." testid="ide-truncated-badge" />
    {:else if activeTab.readOnly}<Badge variant="outline" tone="neutral" label="Read-only" />{/if}
    {#if lspLinkHere?.state === 'reconnecting' || lspLinkHere?.state === 'waiting'}<Badge variant="outline" tone="neutral" label="Language features restarting…" title={lspLinkHere.state === 'waiting' ? 'Otto is restarting; the language server reconnects when it is back.' : 'The language server connection dropped; reconnecting.'} testid="ide-lsp-reconnecting" />{/if}
    {#if lspLinkHere?.state === 'lost'}
      <LoadState what="language features" variant="compact" error="The language server connection dropped." onretry={() => { if (lspLinkHere) retryLspNow(lspLinkHere.rootPath); }} />
    {/if}
    {#if ide.collabIsGuest && ideCollabNotices.roomSync === 'reconnecting'}<Badge variant="outline" tone="neutral" label="Reconnecting to the room…" title="The room fell behind on edits. Your text is kept and will be sent again." testid="ide-collab-reconnecting" />{/if}
    {#if ide.collabIsGuest && ideCollabNotices.roomSync === 'failed'}<Badge variant="outline" tone="warn" label="Not synced with the room" title="Your text is still here. Keep typing or save (⌘S) to send it again." testid="ide-collab-sync-failed" />{/if}
    {#if ide.collabIsGuest && ideCollabNotices.saveRefusedByPath[activeTab.path]}<Badge variant="outline" tone="warn" label="Not saved: {ideCollabNotices.saveRefusedByPath[activeTab.path]}" title="The host doesn’t let guests save this file: {ideCollabNotices.saveRefusedByPath[activeTab.path]}" testid="ide-collab-save-refused" />{/if}
    {#if noDraftKept}<Badge variant="outline" tone="warn" label="Unsaved edits not kept on reload" title="This file is too large for a recovery draft. Save (⌘S) or turn on autosave so a reload or crash doesn’t lose your edits." testid="ide-no-draft-badge" />{/if}
  </div>

  <div class="ide-subbar-tools">
    {#if isGoTab && missingGoTags.length > 0}
      <button
        type="button"
        class="btn small ide-go-tag-enable"
        onclick={() => void enableMissingGoTags()}
        title="Enable Go build tag in this repository so gopls, Run, and Debug include this file"
        data-testid="ide-enable-go-build-tag-btn"
      >
        <Icon name="tag" size={12} />
        <span>Enable tag: {missingGoTags.join(', ')}</span>
      </button>
    {/if}
    {#if isGoTab && (repoGoTags.length > 0 || fileGoTags.length > 0)}
      <button
        type="button"
        class="btn small ghost ide-go-tags-pill"
        class:has-tags={repoGoTags.length > 0}
        onclick={openGoBuildTagsMenu}
        title={repoGoTags.length > 0 ? `Active Go build tags: ${repoGoTags.join(', ')} — click to manage` : 'Configure Go build tags (-tags) for this repository'}
        data-testid="ide-go-build-tags-btn"
      >
        <Icon name="tag" size={12} />
        <span>{repoGoTags.length > 0 ? repoGoTags.join(', ') : 'Tags'}</span>
      </button>
    {/if}
    {#if (activeDocKind === 'json' || activeDocKind === 'jsonl') && !activeTab.readOnly && effectiveViewMode === 'text' && !hasNonEmptySelection}
      <button type="button" class="btn small ghost" onclick={() => applyJsonTransform('pretty')} title="Pretty-print JSON">Pretty</button>
      <button type="button" class="btn small ghost" onclick={() => applyJsonTransform('minify')} title="Minify JSON">Minify</button>
      <button type="button" class="btn small ghost" aria-label="Sort keys" onclick={() => applyJsonTransform('sort')} title="Sort JSON keys alphabetically">Sort</button>
    {/if}

    {#if hasNonEmptySelection && effectiveViewMode === 'text'}
      <div class="ide-sel-bar" role="group" aria-label="Selection actions" data-testid="ide-selection-agent-bar">
        <button type="button" class="btn small ghost ide-sel-accent" title={`Add selection to AI Agent context${kh('⌘L')}`} onclick={() => { ideAgent.markSelectionFromEditor({ openPanel: true }); onfocusagent(); }} data-testid="ide-selection-add-agent">
          <Icon name="assistant" size={12} />Agent
        </button>
        <button type="button" class="btn small ghost" disabled={activeTab.readOnly || !ide.canEditCollab} title={`Inline AI edit on selection${kh('⌘I')}`} onclick={() => ideAgent.openInlineEdit()} data-testid="ide-selection-inline-edit">
          <Icon name="sparkle" size={12} />Edit
        </button>
        <button type="button" class="btn small ghost" aria-pressed={ide.prefixSuffixOpen} title="Add prefix or suffix to every selected line" onclick={() => (ide.prefixSuffixOpen = true)} data-testid="ide-prefix-suffix-toggle">
          Wrap
        </button>
      </div>
    {/if}

    <button type="button" class="icon-btn" aria-label="Editor actions" title="Editor actions: structure, functions, reformat, blame, send to, appearance" aria-haspopup="menu" onclick={openMoreMenu} data-testid="ide-editor-more-btn">
      <Icon name="more" size={14} />
    </button>
  </div>
</div>

{#if sendPicking}
  <SessionPickerModal ws={wsStore.currentId ?? ''} multiLine={isMultiLine(activeTab.content)} onpick={(s) => void pasteToSession(s)} onclose={() => (sendPicking = false)} />
{/if}

{#if ide.prefixSuffixOpen}
  <div class="ide-ps-bar" role="region" aria-label="Multi-line prefix and suffix editor">
    <input class="ide-ps-input" type="text" dir="ltr" aria-label="Before each line" placeholder="Before each line…" bind:value={linePrefix} onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); applyLineWrap(); } else if (e.key === 'Escape') { e.preventDefault(); ide.prefixSuffixOpen = false; } }} data-testid="ide-prefix-input" />
    <input class="ide-ps-input" type="text" dir="ltr" aria-label="After each line" placeholder="After each line…" bind:value={lineSuffix} onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); applyLineWrap(); } else if (e.key === 'Escape') { e.preventDefault(); ide.prefixSuffixOpen = false; } }} data-testid="ide-suffix-input" />
    <button type="button" class="btn small" disabled={(!linePrefix && !lineSuffix) || activeTab.readOnly} onclick={applyLineWrap} data-testid="ide-apply-prefix-suffix">Apply to lines</button>
    <span class="ide-ps-tip">Tip: Hold ⌥ and drag vertically for column selection</span>
    <button type="button" class="icon-btn" aria-label="Close prefix and suffix bar" title="Close prefix and suffix bar" onclick={() => (ide.prefixSuffixOpen = false)}><Icon name="x" size={12} /></button>
  </div>
{/if}

<style>
  .ide-subbar.stack-collapsed { display: none; }
  .ide-subbar {
    display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3);
    block-size: 30px; flex-shrink: 0; flex-wrap: nowrap; overflow: hidden;
    padding-inline: 10px; background: var(--surface); border-block-end: 1px solid var(--border); min-inline-size: 0;
  }
  .ide-crumbs { display: flex; align-items: center; gap: var(--sp-2); flex: 1; min-inline-size: 0; overflow: hidden; font-size: var(--fs-xs); color: var(--text-dim); }
  .crumb-sep { display: inline-flex; opacity: var(--disabled-opacity); flex-shrink: 0; }
  .crumb-btn { background: transparent; border: none; padding: 0; color: var(--text-dim); font-size: var(--fs-xs); cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-inline-size: 2ch; }
  .crumb-btn:hover { color: var(--accent-text); }
  /* The file name never shrinks below its full name; the folders before it give way. */
  .crumb-btn.last { color: var(--text); font-weight: 500; flex-shrink: 0; }
  .crumb-more { flex-shrink: 0; padding-inline: var(--sp-1); border-radius: var(--radius-s); }
  .crumb-more:hover { background: var(--hover); }
  .crumb-sym-btn {
    display: inline-flex; align-items: center; gap: var(--sp-1); padding: 2px var(--sp-2);
    border-radius: var(--radius-s); border: 1px solid var(--border); background: var(--surface-2);
    color: var(--accent-text); font-size: var(--fs-xs); cursor: pointer; max-inline-size: 200px;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex-shrink: 0;
  }
  .crumb-sym-btn:hover { background: var(--accent-soft); }
  .ide-subbar-tools { display: flex; align-items: center; gap: var(--sp-1); flex-wrap: nowrap; flex-shrink: 0; min-inline-size: 0; }
  .ide-subbar-tools :global(.btn.small) {
    block-size: 22px;
    min-block-size: 22px;
    padding-block: 0;
  }
    .ide-sel-bar {
    display: inline-flex; align-items: center; flex-wrap: nowrap; gap: 2px; padding-inline: 2px;
    block-size: 24px; background: var(--surface-2); border: 1px solid var(--accent-line); border-radius: var(--radius-s);
  }
  .ide-sel-accent { color: var(--accent-text); }
  .ide-go-tag-enable {
    border-color: var(--warning);
    background: var(--warning-soft);
    color: var(--warning);
    font-weight: 600;
  }
  .ide-go-tags-pill.has-tags {
    border-color: var(--accent-line);
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .ide-ps-bar {
    display: flex; align-items: center; gap: var(--sp-3); padding: var(--sp-2) 10px;
    background: var(--surface-2); border-block-end: 1px solid var(--border); flex-wrap: wrap; flex-shrink: 0;
  }
  .ide-ps-input {
    inline-size: 170px; max-inline-size: 100%; block-size: 26px; padding-inline: var(--sp-4);
    font-family: var(--font-mono); font-size: var(--fs-xs); background: var(--bg); color: var(--text);
    border: 1px solid var(--control-border); border-radius: var(--radius-s);
  }
  .ide-ps-tip { font-size: var(--fs-xs); color: var(--text-dim); margin-inline-start: auto; }
  @media (max-width: 640px) { .ide-ps-tip { display: none; } }
</style>
