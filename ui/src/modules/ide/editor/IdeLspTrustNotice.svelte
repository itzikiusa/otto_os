<script lang="ts">
  // One line above the code when the open file's language server was refused because it
  // would run this project's build code and the project isn't trusted (ideLspTrust.ts):
  // "Trust project…" (confirmed, then the server re-attaches) and a quiet Hide, for this
  // project until the app restarts. A refused save shows here, in words, with what to do.
  import { announceShown } from '../../../lib/announce.svelte';
  import { IDE_LSP_LINK_EVENT, recentLspLinks, type IdeLspLinkDetail } from '../../../lib/components/cm-ide-lsp-reconnect';
  import Icon from '../../../lib/components/Icon.svelte';
  import { ide, type IdeOpenTab } from '../ideStore.svelte';
  import { ideTargetKey } from '../store/ideTypes';
  import { dismissIdeTrustNotice, ideLspTrustNotice, ideTrustNoticeDismissed } from './ideLspTrust';
  import { ideLspTrust } from './ideLspTrustAction.svelte';
  import { applyIdeLspLink, ideLspLangFor, ideLspRecordFor, ideLspTrustedAt, type IdeLspRecord } from './ideStatusModel';

  let { tab }: { tab: IdeOpenTab } = $props();

  let records = $state<IdeLspRecord[]>(recentLspLinks().reduce(applyIdeLspLink, [] as IdeLspRecord[]));
  $effect(() => {
    const onLink = (e: Event): void => {
      const d = (e as CustomEvent<IdeLspLinkDetail>).detail;
      if (d?.rootPath) records = applyIdeLspLink(records, d);
    };
    window.addEventListener(IDE_LSP_LINK_EVENT, onLink);
    return () => window.removeEventListener(IDE_LSP_LINK_EVENT, onLink);
  });

  const project = $derived(ideTargetKey(ide.target));
  const lang = $derived(ideLspLangFor(tab.path, tab.language));
  const rec = $derived(lang ? ideLspRecordFor(records, lang, tab.path) : null);
  const text = $derived.by(() => {
    void ideLspTrust.rev;
    if (!lang || ideTrustNoticeDismissed(project)) return null;
    return ideLspTrustNotice(lang, rec, ideLspTrustedAt(ide.projectEnv?.env_vars, lang, rec, ide.target.root));
  });
  const error = $derived(ideLspTrust.error?.project === project ? ideLspTrust.error.text : null);

  function hide(): void {
    dismissIdeTrustNotice(project);
    ideLspTrust.rev++;
    queueMicrotask(() => ide.editorHandleForPath(tab.path)?.focus());
  }
</script>

{#if text && lang && rec}
  <div class="ide-trust-notice" use:announceShown={text} data-testid="ide-lsp-trust-notice">
    <Icon name="lock" size={12} />
    {#if error}
      <span class="msg err" use:announceShown={{ text: error, assertive: true }} data-testid="ide-lsp-trust-error">{error}</span>
    {:else}
      <span class="msg">{text}</span>
    {/if}
    <span class="actions">
      <button type="button" class="btn small" disabled={ideLspTrust.busy} onclick={() => void ideLspTrust.trust(lang, rec.rootPath)} data-testid="ide-lsp-trust-btn">Trust project…</button>
      <button type="button" class="icon-btn" aria-label="Hide trust notice" title="Hide for this project until Otto restarts" onclick={hide} data-testid="ide-lsp-trust-hide"><Icon name="x" size={12} /></button>
    </span>
  </div>
{/if}

<style>
  .ide-trust-notice {
    display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; flex-shrink: 0;
    min-block-size: 30px; padding-block: 2px; padding-inline: var(--sp-4) var(--sp-3);
    background: var(--surface-2); border-block-end: 1px solid var(--border);
    color: var(--text); font-size: var(--fs-xs);
  }
  .ide-trust-notice :global(svg) { color: var(--text-dim); flex-shrink: 0; }
  .msg { flex: 1 1 16em; min-inline-size: 0; }
  .msg.err { color: var(--danger); }
  .actions { display: inline-flex; align-items: center; gap: var(--sp-1); margin-inline-start: auto; }
</style>
