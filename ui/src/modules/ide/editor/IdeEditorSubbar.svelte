<script lang="ts">
  // Locked 30px Editor Subbar (CLS = 0): path & symbol crumbs, CSV/JSON tools,
  // compact Selection Agent bar, Inspection Pill, Color Scheme Quick Picker, and Prefix/Suffix bar.
  import { copyWithToast } from '../../../lib/clipboard';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { extractGoBuildTagsFromSource } from '../../../lib/goBuildTags';
  import { plural } from '../../../lib/plural';
  import { toasts } from '../../../lib/toast.svelte';
  import { ide, type IdeOpenTab, type IdeSymbolNode } from '../ideStore.svelte';
  import { ideAgent } from '../store/ideAgentController.svelte';
  import { pushIdeNavLocation, revealInIdeProjectTree, toggleFormatOnSave } from './ideEditorStoreBridge.svelte';
  import type { IdeOutlineNode } from './ideLangAnalyzer';

  interface Props {
    activeTab: IdeOpenTab;
    pathSegments: string[];
    enclosingSymbol: IdeSymbolNode | null;
    effectiveSymbols: IdeSymbolNode[];
    activeJsonPath: string | null;
    sdkLabel: string | null;
    activeDocKind: string;
    effectiveViewMode: 'table' | 'text';
    hasNonEmptySelection: boolean;
    blameActive: boolean;
    errorCount: number;
    warnCount: number;
    onsetviewmode: (mode: 'table' | 'text') => void;
    onapplyjson: (nextText: string) => void;
    onjumpsymbol: (delta: -1 | 1) => void;
    ongotoline: (line: number, col: number) => void;
    onopenstructure: () => void;
    ontoggleblame: () => void;
    onopenquickfix: (e: MouseEvent) => void;
    oncyclediag: (e: MouseEvent) => void;
    onfocusagent: () => void;
  }

  let {
    activeTab, pathSegments, enclosingSymbol, effectiveSymbols, activeJsonPath, sdkLabel,
    activeDocKind, effectiveViewMode, hasNonEmptySelection, blameActive, errorCount, warnCount,
    onsetviewmode, onapplyjson, onjumpsymbol, ongotoline, onopenstructure, ontoggleblame,
    onopenquickfix, oncyclediag, onfocusagent,
  }: Props = $props();

  let linePrefix = $state('');
  let lineSuffix = $state('');

  const isGoTab = $derived(activeTab.language === 'go' || activeTab.path.toLowerCase().endsWith('.go'));
  const repoGoTags = $derived(isGoTab ? ide.getRepoGoBuildTags(activeTab.path) : []);
  const fileGoTags = $derived(isGoTab ? extractGoBuildTagsFromSource(activeTab.content) : []);
  const missingGoTags = $derived(fileGoTags.filter((t) => !repoGoTags.includes(t)));

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

  function openSymbolBreadcrumbMenu(e: MouseEvent): void {
    const anchor = e.currentTarget as HTMLElement | null;
    if (!effectiveSymbols.length) { onopenstructure(); return; }
    const items: MenuItem[] = effectiveSymbols.slice(0, 50).map((s) => ({
      label: s.name,
      icon: s.line === enclosingSymbol?.line ? 'check' : 'function',
      hint: `L${s.line}`,
      action: () => {
        pushIdeNavLocation(activeTab.path, ide.cursorInfo.line, ide.cursorInfo.col);
        ongotoline(s.line, (s as IdeOutlineNode).column || 1);
      },
    }));
    items.push({ separator: true }, { label: 'Search file structure…', icon: 'search', hint: '⌘F12', action: onopenstructure });
    ctxMenu.showAt(anchor, items, { align: 'start' });
  }

  async function openSchemePickerMenu(e: MouseEvent): Promise<void> {
    const anchor = e.currentTarget as HTMLElement | null;
    const {
      IDE_COLOR_SCHEME_OPTIONS,
      readIdeEditorPreferences,
      writeIdeEditorPreferences,
    } = await import('../../../lib/components/cm-ide-theme');
    const cur = readIdeEditorPreferences();
    const items: MenuItem[] = IDE_COLOR_SCHEME_OPTIONS.map((opt) => ({
      label: opt.label,
      icon: cur.colorScheme === opt.id ? 'check' : 'palette',
      action: () => { writeIdeEditorPreferences({ colorScheme: opt.id }); },
    }));
    items.push(
      { separator: true },
      { label: `Rainbow Brackets: ${cur.rainbowBrackets ? 'On' : 'Off'}`, icon: cur.rainbowBrackets ? 'check' : 'dot', action: () => { writeIdeEditorPreferences({ rainbowBrackets: !cur.rainbowBrackets }); } },
      { label: `Inlay Parameter Hints: ${cur.inlayHints ? 'On' : 'Off'}`, icon: cur.inlayHints ? 'check' : 'dot', action: () => { writeIdeEditorPreferences({ inlayHints: !cur.inlayHints }); } },
      { label: `CodeVision Usages: ${cur.codeVision ? 'On' : 'Off'}`, icon: cur.codeVision ? 'check' : 'dot', action: () => { writeIdeEditorPreferences({ codeVision: !cur.codeVision }); } },
      { label: `Auto-fold Imports: ${cur.autoFoldImports ? 'On' : 'Off'}`, icon: cur.autoFoldImports ? 'check' : 'dot', action: () => { writeIdeEditorPreferences({ autoFoldImports: !cur.autoFoldImports }); } },
      { label: `Breadcrumbs Bar: ${cur.showBreadcrumbs ? 'On' : 'Off'}`, icon: cur.showBreadcrumbs ? 'check' : 'dot', action: () => { writeIdeEditorPreferences({ showBreadcrumbs: !cur.showBreadcrumbs }); } },
      { label: `Auto-Save: ${cur.autoSave ? 'On' : 'Off'}`, icon: cur.autoSave ? 'check' : 'dot', action: () => { writeIdeEditorPreferences({ autoSave: !cur.autoSave }); } },
      { label: `Format on Save: ${cur.formatOnSave ? 'On' : 'Off'}`, icon: cur.formatOnSave ? 'check' : 'dot', action: () => { toggleFormatOnSave(); writeIdeEditorPreferences({ formatOnSave: !cur.formatOnSave }); } },
      { separator: true },
      { label: 'Optimize Imports', icon: 'sparkle', hint: '⌃⌥O', disabled: activeTab.readOnly, action: () => void ide.organizeImportsActiveFile() },
    );
    ctxMenu.showAt(anchor, items, { align: 'end' });
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

<div class="ide-subbar">
  <div class="ide-crumbs" dir="ltr">
    {#each pathSegments as seg, idx (`${seg}:${idx}`)}
      {#if idx > 0}<span class="crumb-sep" aria-hidden="true"><Icon name="chevronRight" size={12} noflip /></span>{/if}
      <button type="button" class="crumb-btn" class:last={idx === pathSegments.length - 1} onclick={() => void revealInIdeProjectTree(activeTab.path)} title="Reveal in project tree">{seg}</button>
    {/each}
    {#if enclosingSymbol}
      <span class="crumb-sep" aria-hidden="true"><Icon name="chevronRight" size={12} noflip /></span>
      <button type="button" class="crumb-sym-btn" onclick={openSymbolBreadcrumbMenu} title="Jump to symbol in file (⌘F12)">
        <Icon name="function" size={12} /><span>{enclosingSymbol.name}</span>
      </button>
    {/if}
    {#if activeJsonPath && activeJsonPath !== '$'}
      <span class="crumb-sep" aria-hidden="true"><Icon name="chevronRight" size={12} noflip /></span>
      <button type="button" class="crumb-sym-btn mono" onclick={() => void copyWithToast(activeJsonPath, 'JSONPath')} title="Click to copy JSONPath" data-testid="ide-jsonpath-crumb">{activeJsonPath}</button>
    {/if}
    {#if sdkLabel}<Badge variant="outline" tone="info" label={sdkLabel} />{/if}
    {#if activeTab.readOnly}<Badge variant="outline" tone="neutral" label="Read-only" />{/if}
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
    {#if isGoTab}
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
    {#if activeDocKind === 'csv' || activeDocKind === 'jsonl'}
      <div class="segmented" role="group" aria-label="Editor view mode" data-testid="ide-csv-mode-toggle">
        <button type="button" aria-pressed={effectiveViewMode === 'table'} onclick={() => onsetviewmode('table')}>Table</button>
        <button type="button" aria-pressed={effectiveViewMode === 'text'} onclick={() => onsetviewmode('text')}>Text</button>
      </div>
    {/if}
    {#if (activeDocKind === 'json' || activeDocKind === 'jsonl') && !activeTab.readOnly && effectiveViewMode === 'text' && !hasNonEmptySelection}
      <button type="button" class="btn small ghost" onclick={() => applyJsonTransform('pretty')} title="Pretty-print JSON">Pretty</button>
      <button type="button" class="btn small ghost" onclick={() => applyJsonTransform('minify')} title="Minify JSON">Minify</button>
      <button type="button" class="btn small ghost" aria-label="Sort keys" onclick={() => applyJsonTransform('sort')} title="Sort JSON keys alphabetically">Sort</button>
    {/if}

    {#if hasNonEmptySelection && effectiveViewMode === 'text'}
      <div class="ide-sel-bar" role="group" aria-label="Selection actions" data-testid="ide-selection-agent-bar">
        <button type="button" class="btn small ghost ide-sel-accent" title="Add selection to AI Agent context (⇧⌘L)" onclick={() => { ideAgent.markSelectionFromEditor({ openPanel: true }); onfocusagent(); }} data-testid="ide-selection-add-agent">
          <Icon name="assistant" size={12} />Agent ⇧⌘L
        </button>
        <button type="button" class="btn small ghost" disabled={activeTab.readOnly || !ide.canEditCollab} title="Inline AI edit on selection (⌘I)" onclick={() => ideAgent.openInlineEdit()} data-testid="ide-selection-inline-edit">
          <Icon name="sparkle" size={12} />Edit ⌘I
        </button>
        <button type="button" class="btn small ghost" aria-pressed={ide.prefixSuffixOpen} title="Add prefix or suffix to every selected line" onclick={() => (ide.prefixSuffixOpen = true)} data-testid="ide-prefix-suffix-toggle">
          Wrap
        </button>
        <button type="button" class="icon-btn" aria-label="Split selection into lines" title="Split selection into multiple cursors (⌥⇧I)" onclick={() => ide.splitSelectionToLines()}>
          <Icon name="layers" size={12} />
        </button>
      </div>
    {/if}

    <span class="ide-cursor-meta" dir="ltr">Ln {ide.cursorInfo.line}, Col {ide.cursorInfo.col}</span>
    {#if ide.cursorInfo.cursors > 1}<Badge variant="outline" tone="accent" label={plural(ide.cursorInfo.cursors, 'cursor')} />{/if}

    <button
      type="button"
      class="ide-inspect-btn"
      class:has-err={errorCount > 0}
      class:has-warn={errorCount === 0 && warnCount > 0}
      onclick={oncyclediag}
      title={errorCount || warnCount ? `${errorCount} errors, ${warnCount} warnings — click for next issue (Shift+click for Problems)` : 'No problems found'}
      aria-label={errorCount || warnCount ? `${errorCount} errors, ${warnCount} warnings` : 'No problems found'}
      data-testid="ide-inspection-pill"
    >
      {#if errorCount > 0 || warnCount > 0}
        {#if errorCount > 0}<span class="ide-ins-err">✕ {errorCount}</span>{/if}
        {#if warnCount > 0}<span class="ide-ins-warn">⚠ {warnCount}</span>{/if}
      {:else}
        <Icon name="check" size={12} />
      {/if}
    </button>

    <button type="button" class="icon-btn" aria-label="Editor Color Scheme and Appearance" title="Color Scheme & Editor Appearance" onclick={openSchemePickerMenu} data-testid="ide-scheme-picker-btn">
      <Icon name="palette" size={13} />
    </button>
    <button type="button" class="icon-btn" aria-label="Previous function or symbol" title="Previous function (⌃↑)" disabled={effectiveSymbols.length === 0} onclick={() => onjumpsymbol(-1)}>
      <Icon name="arrowUp" size={13} />
    </button>
    <button type="button" class="icon-btn" aria-label="Next function or symbol" title="Next function (⌃↓)" disabled={effectiveSymbols.length === 0} onclick={() => onjumpsymbol(1)}>
      <Icon name="arrowDown" size={13} />
    </button>
    <button type="button" class="icon-btn" aria-label="File structure" title="File structure (⌘F12)" onclick={onopenstructure} data-testid="ide-structure-btn">
      <Icon name="layers" size={13} />
    </button>
    <button type="button" class="icon-btn" aria-label="Toggle git blame annotations" title="Toggle git blame annotations" aria-pressed={blameActive} onclick={ontoggleblame} data-testid="ide-blame-btn">
      <Icon name="commit" size={13} />
    </button>
    <button type="button" class="icon-btn" aria-label="Quick fixes" title="Show quick fixes and refactorings (⌥⏎)" onclick={onopenquickfix} data-testid="ide-quickfix-btn">
      <Icon name="bulb" size={13} />
    </button>
    <button type="button" class="icon-btn" aria-label="Reformat file" title="Reformat code (⌘⌥L)" disabled={activeTab.readOnly} onclick={() => void ide.formatActiveFile()}>
      <Icon name="format" size={13} />
    </button>
  </div>
</div>

{#if ide.prefixSuffixOpen}
  <div class="ide-ps-bar" role="region" aria-label="Multi-line prefix and suffix editor">
    <input class="ide-ps-input" type="text" dir="ltr" aria-label="Before each line" placeholder="Before each line…" bind:value={linePrefix} onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); applyLineWrap(); } else if (e.key === 'Escape') { e.preventDefault(); ide.prefixSuffixOpen = false; } }} data-testid="ide-prefix-input" />
    <input class="ide-ps-input" type="text" dir="ltr" aria-label="After each line" placeholder="After each line…" bind:value={lineSuffix} onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); applyLineWrap(); } else if (e.key === 'Escape') { e.preventDefault(); ide.prefixSuffixOpen = false; } }} data-testid="ide-suffix-input" />
    <button type="button" class="btn small" disabled={(!linePrefix && !lineSuffix) || activeTab.readOnly} onclick={applyLineWrap} data-testid="ide-apply-prefix-suffix">Apply to lines</button>
    <span class="ide-ps-tip">Tip: Hold ⌥ and drag vertically for column selection</span>
    <button type="button" class="icon-btn" aria-label="Close prefix and suffix bar" title="Close" onclick={() => (ide.prefixSuffixOpen = false)}><Icon name="x" size={12} /></button>
  </div>
{/if}

<style>
  .ide-subbar {
    display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3);
    block-size: 30px; flex-shrink: 0; flex-wrap: nowrap; overflow: hidden;
    padding-inline: 10px; background: var(--surface); border-block-end: 1px solid var(--border); min-inline-size: 0;
  }
  .ide-crumbs { display: flex; align-items: center; gap: var(--sp-2); flex: 1; min-inline-size: 0; overflow: hidden; font-size: var(--fs-xs); color: var(--text-dim); }
  .crumb-sep { display: inline-flex; opacity: var(--disabled-opacity); flex-shrink: 0; }
  .crumb-btn { background: transparent; border: none; padding: 0; color: var(--text-dim); font-size: var(--fs-xs); cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .crumb-btn:hover { color: var(--accent-text); }
  .crumb-btn.last { color: var(--text); font-weight: 500; }
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
  .ide-cursor-meta { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); padding-inline: var(--sp-1); white-space: nowrap; }
  .ide-inspect-btn {
    display: inline-flex; align-items: center; gap: var(--sp-2); block-size: 22px; padding-inline: var(--sp-2);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-s);
    color: var(--success); font-family: var(--font-mono); font-size: var(--fs-xs); cursor: pointer; white-space: nowrap;
  }
  .ide-inspect-btn.has-err { border-color: var(--danger); background: var(--danger-soft); }
  .ide-inspect-btn.has-warn { border-color: var(--warning); background: var(--warning-soft); }
  .ide-ins-err { color: var(--danger); font-weight: 600; }
  .ide-ins-warn { color: var(--warning); font-weight: 600; }
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
  @media (max-width: 640px) { .ide-ps-tip, .ide-cursor-meta { display: none; } }
</style>
