<script lang="ts">
  // The Git landing (no repository open): one list of repositories as inbox
  // rows (lib InboxRow) — the open ones first, each saying where it stands in
  // words ("Clean", "3 files changed · ↑2 not pushed") from the status the
  // page already holds (no request per row), the rest after. A row opens the
  // repository; its ⋯ (and long-press) holds Pull requests, the hosting
  // account, Copy path and Remove. The accounts are read on the first ⋯.
  import { api } from '../../lib/api/client';
  import type { GitAccount, Repo } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import { forgeLabel, stateWords } from '../../lib/gitNextStep';
  import { git } from '../../lib/stores/git.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import Icon from '../../lib/components/Icon.svelte';
  import InboxRow from '../../lib/components/InboxRow.svelte';
  import InboxFold from '../../lib/components/InboxFold.svelte';
  import ProjectBadge from '../../lib/components/ProjectBadge.svelte';
  import { graphCache } from './graph-cache';

  interface Props {
    onopen: (repoId: string, sub?: string) => void;
  }
  let { onopen }: Props = $props();

  let filter = $state('');
  const matches = (r: Repo, q: string): boolean =>
    r.name.toLowerCase().includes(q) || r.path.toLowerCase().includes(q) || (r.remote_url ?? '').toLowerCase().includes(q);
  const shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return q ? git.allRepos.filter((r) => matches(r, q)) : git.allRepos;
  });
  const openSet = $derived(new Set(git.openRepoIds));
  const openRows = $derived(shown.filter((r) => openSet.has(r.id)));
  const restRows = $derived(shown.filter((r) => !openSet.has(r.id)));

  /** The row's word, from a status the page already read (else none). */
  function wordOf(r: Repo): string | null {
    const s = git.statusById[r.id];
    if (!s) return null;
    const branch = s.branch === '(detached)' ? null : s.branch;
    return stateWords({ changes: s.changes.length, ahead: s.ahead, behind: s.behind, upstream: s.upstream, branch });
  }

  let accounts: GitAccount[] | null = null;
  async function loadAccounts(): Promise<GitAccount[]> {
    accounts ??= await api.get<GitAccount[]>('/git/accounts').catch(() => []);
    return accounts;
  }

  /** (Re)bind a repo to a hosting account. Registration resolves the account
   *  once, so a repo added before its account existed needs this — otherwise
   *  every provider call fails with "repo has no git account". */
  async function setRepoAccount(r: Repo, accountId: string | null): Promise<void> {
    try {
      const updated = await api.patch<Repo>(`/repos/${r.id}`, { git_account_id: accountId });
      await git.loadAllRepos(true);
      toasts.success(updated.git_account_id ? 'Account linked' : 'Account unlinked', r.name);
    } catch (e) {
      toastError('Couldn’t link account', e);
    }
  }

  async function removeRepo(r: Repo): Promise<void> {
    if (!(await confirmer.ask(`Remove “${r.name}” from Otto? Its tab closes and its saved review findings are deleted. The folder and its git history on disk are not touched, so you can add it again later.`, { title: 'Remove repository', confirmLabel: 'Remove', danger: true }))) return;
    try {
      await api.del(`/repos/${r.id}`);
      git.closeRepoTab(r.id);
      graphCache.drop(r.id);
      await git.loadAllRepos(true);
      toasts.info('Repository removed', r.name);
    } catch (e) {
      toastError('Couldn’t remove the repository', e);
    }
  }

  async function rowMenu(e: MouseEvent, r: Repo): Promise<void> {
    e.preventDefault();
    const target = e.currentTarget as HTMLElement;
    const forge = forgeLabel(r);
    const accs = r.provider ? (await loadAccounts()).filter((a) => a.provider === r.provider) : [];
    const items: MenuItem[] = [
      { label: 'Open', icon: 'branch', action: () => onopen(r.id) },
      { label: 'Open pull requests', icon: 'pr', disabled: !forge, title: forge ? undefined : 'This repository has no supported remote', action: () => onopen(r.id, 'prs') },
      ...(r.provider
        ? [{
            label: r.git_account_id ? 'Hosting account' : `Link a ${forge ?? r.provider} account`,
            icon: 'key',
            children: [
              { label: 'No account', checked: !r.git_account_id, action: () => void setRepoAccount(r, null) },
              ...accs.map((a): MenuItem => ({ label: a.label, checked: r.git_account_id === a.id, action: () => void setRepoAccount(r, a.id) })),
            ],
          } satisfies MenuItem]
        : []),
      { label: 'Copy path', icon: 'copy', action: () => void copyWithToast(r.path, 'Path') },
      { separator: true },
      { label: 'Remove from Otto…', icon: 'trash', danger: true, action: () => void removeRepo(r) },
    ];
    if (target?.isConnected) ctxMenu.showAt(target, items, { align: 'end' });
    else ctxMenu.show(e, items);
  }
</script>

{#snippet row(r: Repo)}
  <InboxRow
    title={r.name}
    word={wordOf(r)}
    onPage
    testid="git-repo-row"
    tip="Open {r.name}"
    onclick={() => onopen(r.id)}
    oncontextmenu={(e) => void rowMenu(e, r)}
  >
    {#snippet context()}
      <ProjectBadge name={r.name} />
      <span>{forgeLabel(r) ?? (r.remote_url ? 'Remote' : 'Local')}</span>
      {#if r.provider && !r.git_account_id}<span class="gl-warn" title="Pull requests need a linked account">· no account</span>{/if}
    {/snippet}
    {#snippet line3()}
      <span class="ellipsis mono" dir="ltr" title={r.path}>{r.path}</span>
    {/snippet}
    {#snippet actions()}
      <button type="button" class="icon-btn" aria-label="More for {r.name}" title="Pull requests, account, copy path, remove" onclick={(e) => void rowMenu(e, r)}>
        <Icon name="more" size={14} />
      </button>
    {/snippet}
  </InboxRow>
{/snippet}

<div class="gl list-col">
  <div class="gl-search">
    <Icon name="search" size={13} />
    <input dir="ltr" class="gl-input" type="search" bind:value={filter} placeholder="Filter repositories by name, path or remote…" aria-label="Filter repositories" spellcheck="false"
      onkeydown={(e) => { if (e.key === 'Escape' && filter !== '') { e.stopPropagation(); filter = ''; } }} />
    {#if filter.trim() !== ''}<span class="gl-count dim">{shown.length} / {git.allRepos.length}</span>{/if}
  </div>
  {#if shown.length === 0}
    <p class="gl-empty dim">No repositories match “{filter.trim()}”.</p>
  {/if}
  {#if openRows.length > 0}
    <InboxFold label="Open">
      {#each openRows as r (r.id)}{@render row(r)}{/each}
    </InboxFold>
  {/if}
  {#if restRows.length > 0}
    <InboxFold label={openRows.length ? 'All repositories' : `Repositories · ${git.allRepos.length}`}>
      {#each restRows as r (r.id)}{@render row(r)}{/each}
    </InboxFold>
  {/if}
</div>

<style>
  .gl {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }
  .gl-search {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .gl-search:focus-within {
    border-color: var(--accent-text);
  }
  .gl-input {
    flex: 1;
    min-width: 0;
    height: 36px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-m);
    outline: none;
  }
  .gl-count {
    font-size: var(--fs-s);
    white-space: nowrap;
  }
  .gl-empty {
    margin: 0;
    padding: 18px 4px;
    text-align: center;
    font-size: var(--fs-m);
  }
  .gl-warn {
    color: var(--warning);
  }
  @media (max-width: 1024px) {
    .gl-input {
      min-height: 44px;
      font-size: 16px; /* ui-guards: allow — 16px stops iOS zoom-on-focus */
    }
  }
</style>
