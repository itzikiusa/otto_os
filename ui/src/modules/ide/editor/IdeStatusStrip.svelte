<script lang="ts">
  // IDE status strip (22px, under the dock): one home for the editor's state, as text. Every
  // item is a button that opens what it describes — the branch menu, Problems, Go to Line,
  // the language's settings, the language server's health. What the store does not know
  // (encoding) is left out rather than guessed. Not mounted in the stacked phone layout.
  // While it is up it REPLACES the app's status bar (two stacked status rows read as noise),
  // so it also carries the two things a person must not lose there: agents working and
  // sessions that need them. The app bar still returns by itself while the daemon is
  // disconnected (StatusBar.svelte).
  import { IDE_LSP_LINK_EVENT, recentLspLinks, retryLspNow, type IdeLspLinkDetail } from '../../../lib/components/cm-ide-lsp-reconnect';
  import Icon from '../../../lib/components/Icon.svelte';
  import { plural } from '../../../lib/plural';
  import { router } from '../../../lib/router.svelte';
  import { agentRunsWorking } from '../../../lib/stores/agentRunsWorking.svelte';
  import { git } from '../../../lib/stores/git.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { ideGit } from '../git/ideGitController.svelte';
  import { ide } from '../ideStore.svelte';
  import { openGoToLine } from '../search/ideQuickOpen.svelte';
  import {
    applyIdeLspLink, ideEolLabel, ideLanguageLabel, ideLanguageSettingsSection, ideLspItem, ideLspLangFor,
    ideLspRecordFor, ideProblemCounts, type IdeLspRecord,
  } from './ideStatusModel';

  let lspRecords = $state<IdeLspRecord[]>(recentLspLinks().reduce(applyIdeLspLink, [] as IdeLspRecord[]));
  $effect(() => {
    const onLink = (e: Event): void => {
      const d = (e as CustomEvent<IdeLspLinkDetail>).detail;
      if (d?.rootPath) lspRecords = applyIdeLspLink(lspRecords, d);
    };
    window.addEventListener(IDE_LSP_LINK_EVENT, onLink);
    return () => window.removeEventListener(IDE_LSP_LINK_EVENT, onLink);
  });

  const working = $derived(ws.workingCount + agentRunsWorking.count);
  function showNeedsYou(): void {
    ws.needsYouFilter = true;
    router.go('agents');
  }

  const tab = $derived(ide.activeTab);
  const branch = $derived.by((): string | null => {
    const path = ide.activePath ?? ide.target.root;
    if (!path) return null;
    const repo = ideGit.findKnownRepoForPath(path);
    return repo ? (git.statusById[repo.id]?.branch ?? null) : null;
  });
  const counts = $derived(ideProblemCounts(ide.problems));
  const langSection = $derived(tab ? ideLanguageSettingsSection(tab.path, tab.language) : null);
  const eol = $derived(tab ? ideEolLabel(tab) : null);
  const lsp = $derived.by(() => {
    if (!tab) return null;
    const lang = ideLspLangFor(tab.path, tab.language);
    return lang ? ideLspItem(lang, ideLspRecordFor(lspRecords, lang, tab.path)) : null;
  });

  function openBranches(e: MouseEvent): void {
    const anchor = e.currentTarget as HTMLElement;
    const path = ide.activePath ?? ide.target.root;
    void import('../git/ideGitQuickMenu').then((m) => m.gitOpenBranchesMenu(path, anchor));
  }

  function onLsp(): void {
    if (lsp?.action === 'retry' && tab) {
      const rec = ideLspRecordFor(lspRecords, ideLspLangFor(tab.path, tab.language) ?? '', tab.path);
      if (rec) { retryLspNow(rec.rootPath); return; }
    }
    ide.openSettings('plugins');
  }
</script>

<footer class="ide-status" aria-label="Editor status" data-testid="ide-status-strip">
  <div class="st-group">
    {#if working > 0}
      <button type="button" class="st-item" title="Agents working — open Agents" onclick={() => router.go('agents')} data-testid="ide-status-working">
        <span class="st-dot busy" aria-hidden="true"></span><span class="st-text">{working} working</span>
      </button>
    {/if}
    {#if ws.needsYouCount > 0}
      <button type="button" class="st-item st-needs" title="Show the sessions waiting on you" onclick={showNeedsYou} data-testid="ide-status-needs-you">
        {ws.needsYouCount} {ws.needsYouCount === 1 ? 'needs' : 'need'} you
      </button>
    {/if}
    {#if branch}
      <button type="button" class="st-item" title="Branches — check out or create" aria-haspopup="menu" onclick={openBranches} data-testid="ide-status-branch">
        <Icon name="branch" size={12} /><span class="st-text">{branch}</span>
      </button>
    {/if}
    <button
      type="button"
      class="st-item"
      title="Open Problems"
      aria-label={counts.errors || counts.warnings ? `${plural(counts.errors, 'error')}, ${plural(counts.warnings, 'warning')} — open Problems` : 'No problems — open Problems'}
      onclick={() => ide.openBottomTab('problems')}
      data-testid="ide-status-problems"
    >
      {#if counts.errors || counts.warnings}
        <span class="st-err" class:zero={!counts.errors}><Icon name="x" size={12} />{counts.errors}</span>
        <span class="st-warn" class:zero={!counts.warnings}><Icon name="warning" size={12} />{counts.warnings}</span>
      {:else}
        <span class="st-ok"><Icon name="check" size={12} /></span><span class="st-text">No problems</span>
      {/if}
    </button>
  </div>

  {#if tab}
    <div class="st-group end">
      <button type="button" class="st-item mono" title="Go to line (⌘G)" onclick={() => openGoToLine()} data-testid="ide-status-cursor" dir="ltr">
        Ln {ide.cursorInfo.line}, Col {ide.cursorInfo.col}{#if ide.cursorInfo.cursors > 1}<span class="st-dim"> · {plural(ide.cursorInfo.cursors, 'cursor')}</span>{/if}
      </button>
      {#if langSection}
        <button type="button" class="st-item" title="Language settings" onclick={() => ide.openSettings(langSection)} data-testid="ide-status-language">{ideLanguageLabel(tab.path, tab.language)}</button>
      {:else}
        <span class="st-item static" data-testid="ide-status-language">{ideLanguageLabel(tab.path, tab.language)}</span>
      {/if}
      {#if eol}<span class="st-item static" title="Line endings as read from disk; kept on save" data-testid="ide-status-eol">{eol}</span>{/if}
      {#if lsp}
        <button type="button" class="st-item" title={lsp.title} onclick={onLsp} data-testid="ide-status-lsp" data-tone={lsp.tone}>
          <span class="st-dot {lsp.tone}" aria-hidden="true"></span><span class="st-text">{lsp.label}</span>
        </button>
      {/if}
    </div>
  {/if}
</footer>

<style>
  .ide-status {
    display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3);
    block-size: 22px; flex-shrink: 0; padding-inline: var(--sp-2); min-inline-size: 0; overflow: hidden;
    background: var(--surface); border-block-start: 1px solid var(--border);
    font-size: var(--fs-xs); color: var(--text-dim);
  }
  .st-group { display: flex; align-items: center; gap: var(--sp-1); min-inline-size: 0; }
  .st-group.end { flex-shrink: 0; }
  .st-item {
    display: inline-flex; align-items: center; gap: var(--sp-2); block-size: 20px; padding-inline: var(--sp-3);
    background: transparent; border: none; border-radius: var(--radius-s); color: inherit; font-size: var(--fs-xs);
    white-space: nowrap; min-inline-size: 0;
  }
  button.st-item { cursor: pointer; }
  button.st-item:hover { background: var(--hover); color: var(--text); }
  .st-item.static { cursor: default; }
  .st-item.mono { font-family: var(--font-mono); }
  .st-text { overflow: hidden; text-overflow: ellipsis; }
  .st-dim { color: var(--text-dim); }
  .st-needs { color: var(--warning); font-weight: 600; }
  .st-err, .st-warn, .st-ok { display: inline-flex; align-items: center; gap: var(--sp-1); }
  .st-err { color: var(--danger); }
  .st-warn { color: var(--warning); }
  .st-err.zero, .st-warn.zero { color: var(--text-dim); }
  .st-ok { color: var(--success); }
  .st-dot { inline-size: 7px; block-size: 7px; border-radius: 50%; flex-shrink: 0; background: var(--text-dim); }
  .st-dot.ok { background: var(--success); }
  .st-dot.busy { background: var(--warning); }
  .st-dot.bad { background: var(--danger); }
</style>
