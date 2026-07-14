<script lang="ts">
  // Focus view (GitKraken-style): everything "mine" across the whole install in
  // one place — MY PULL REQUESTS (open PRs aggregated across every registered
  // repo, grouped by forge) and MY WORK (Jira issues assigned to me, rebuilt
  // into project → epic → story → subtask hierarchy). Rows deep-link both ways:
  // into the Otto repo tab and out to the provider.
  import { api } from '../../lib/api/client';
  import type {
    GitAccount,
    IssueAccount,
    IssueDetail,
    MyWorkIssue,
    PrSummary,
    Repo,
  } from '../../lib/api/types';
  import { git } from '../../lib/stores/git.svelte';
  import { router } from '../../lib/router.svelte';
  import { openExternal } from '../../lib/external';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';

  // ── MY PULL REQUESTS ────────────────────────────────────────────────────────
  interface PrRow {
    repo: Repo;
    pr: PrSummary;
  }
  let prRows = $state<PrRow[]>([]);
  let prsLoading = $state(true);
  /** Repos whose PR listing failed (no forge account, network) — shown once. */
  let prErrors = $state(0);
  let prFilter = $state<'all' | 'mine'>('all');
  let myHandles = $state<Set<string>>(new Set());

  async function loadPrs(): Promise<void> {
    prsLoading = true;
    prErrors = 0;
    try {
      if (!git.allReposLoaded) await git.loadAllRepos();
      // Forge usernames — powers the "Mine" filter (best-effort).
      void api
        .get<GitAccount[]>('/git/accounts')
        .then((accs) => {
          myHandles = new Set(accs.map((a) => a.username.toLowerCase()).filter(Boolean));
        })
        .catch(() => {});
      const candidates = git.allRepos.filter(
        (r) => r.forge && r.forge !== 'unrecognized',
      );
      const settled = await Promise.allSettled(
        candidates.map(async (repo) => {
          const prs = await api.get<PrSummary[]>(`/repos/${repo.id}/prs?state=open`);
          return prs.map((pr) => ({ repo, pr }));
        }),
      );
      const rows: PrRow[] = [];
      for (const s of settled) {
        if (s.status === 'fulfilled') rows.push(...s.value);
        else prErrors++;
      }
      rows.sort((a, b) => (b.pr.updated_at ?? '').localeCompare(a.pr.updated_at ?? ''));
      prRows = rows;
    } finally {
      prsLoading = false;
    }
  }

  const visiblePrRows = $derived(
    prFilter === 'mine' && myHandles.size > 0
      ? prRows.filter((r) => myHandles.has(r.pr.author.toLowerCase()))
      : prRows,
  );

  /** Rows grouped by forge, insertion-ordered github → bitbucket → gitlab. */
  const prGroups = $derived.by(() => {
    const groups = new Map<string, PrRow[]>();
    for (const row of visiblePrRows) {
      const forge = (row.repo.forge as string) ?? 'other';
      const list = groups.get(forge);
      if (list) list.push(row);
      else groups.set(forge, [row]);
    }
    return [...groups.entries()];
  });

  // ── MY WORK (Jira) ──────────────────────────────────────────────────────────
  let issueAccounts = $state<IssueAccount[]>([]);
  let issueAccountId = $state<string>('');
  let work = $state<MyWorkIssue[]>([]);
  let workLoading = $state(true);
  let workError = $state<string | null>(null);

  async function loadAccounts(): Promise<void> {
    try {
      issueAccounts = await api.get<IssueAccount[]>('/issue/accounts');
      if (!issueAccountId && issueAccounts[0]) issueAccountId = issueAccounts[0].id;
    } catch {
      issueAccounts = [];
    } finally {
      if (issueAccounts.length === 0) workLoading = false;
    }
  }

  async function loadWork(accountId: string): Promise<void> {
    workLoading = true;
    workError = null;
    try {
      work = await api.get<MyWorkIssue[]>(
        `/issue/my-work?account_id=${encodeURIComponent(accountId)}`,
      );
    } catch (e) {
      work = [];
      workError = e instanceof Error ? e.message : String(e);
    } finally {
      workLoading = false;
    }
  }

  $effect(() => {
    void loadPrs();
    void loadAccounts();
  });
  $effect(() => {
    const id = issueAccountId;
    if (id) void loadWork(id);
  });

  // Hierarchy: project → (parent group | top-level issue) → children.
  interface WorkNode {
    issue: MyWorkIssue;
    children: WorkNode[];
  }
  interface ParentGroup {
    key: string;
    summary: string;
    type: string;
    /** True when the parent itself is one of MY issues (renders as a node). */
    children: WorkNode[];
  }
  interface ProjectGroup {
    key: string;
    name: string;
    groups: ParentGroup[];
    loose: WorkNode[];
  }

  const projects = $derived.by((): ProjectGroup[] => {
    const byKey = new Map<string, WorkNode>();
    for (const w of work) byKey.set(w.key, { issue: w, children: [] });

    const projs = new Map<string, ProjectGroup>();
    const projOf = (w: MyWorkIssue): ProjectGroup => {
      let p = projs.get(w.project_key);
      if (!p) {
        p = { key: w.project_key, name: w.project_name, groups: [], loose: [] };
        projs.set(w.project_key, p);
      }
      return p;
    };

    for (const w of work) {
      const node = byKey.get(w.key)!;
      if (w.parent_key && byKey.has(w.parent_key)) {
        // Parent is also mine → nest directly under it.
        byKey.get(w.parent_key)!.children.push(node);
        continue;
      }
      const proj = projOf(w);
      if (w.parent_key) {
        // Parent exists but isn't mine → synthetic group header (epic/story).
        let g = proj.groups.find((g) => g.key === w.parent_key);
        if (!g) {
          g = {
            key: w.parent_key,
            summary: w.parent_summary ?? w.parent_key,
            type: w.parent_type ?? '',
            children: [],
          };
          proj.groups.push(g);
        }
        g.children.push(node);
      } else {
        proj.loose.push(node);
      }
    }
    return [...projs.values()];
  });

  // ── Issue quick-view (right side panel) ─────────────────────────────────────
  let quickKey = $state<string | null>(null);
  let quickIssue = $state<IssueDetail | null>(null);
  let quickLoading = $state(false);

  async function openQuick(key: string): Promise<void> {
    quickKey = key;
    quickIssue = null;
    quickLoading = true;
    try {
      quickIssue = await api.get<IssueDetail>(
        `/issue/${encodeURIComponent(issueAccountId)}/${encodeURIComponent(key)}`,
      );
    } catch {
      quickIssue = null;
    } finally {
      quickLoading = false;
    }
  }

  function closeQuick(): void {
    quickKey = null;
    quickIssue = null;
  }

  // ── Formatting ──────────────────────────────────────────────────────────────
  function ago(iso: string | null): string {
    if (!iso) return '';
    const t = Date.parse(iso);
    if (Number.isNaN(t)) return '';
    const mins = Math.max(0, Math.round((Date.now() - t) / 60_000));
    if (mins < 60) return `${mins}m`;
    const hours = Math.round(mins / 60);
    if (hours < 24) return `${hours}h`;
    const days = Math.round(hours / 24);
    return `${days}d`;
  }

  const forgeLabel: Record<string, string> = {
    github: 'GitHub',
    bitbucket: 'Bitbucket',
    gitlab: 'GitLab',
    other: 'Other',
  };
</script>

{#snippet issueRow(node: WorkNode, depth: number)}
  <div class="fx-issue" style="padding-inline-start:{10 + depth * 18}px">
    <button
      class="fx-issue-main"
      class:quick-open={quickKey === node.issue.key}
      onclick={() => openQuick(node.issue.key)}
      title="Quick view {node.issue.key}"
    >
      <span class="fx-type t-{node.issue.issue_type.toLowerCase().replace(/[^a-z]+/g, '-')}">{node.issue.issue_type}</span>
      <span class="mono fx-key">{node.issue.key}</span>
      <span class="fx-summary">{node.issue.summary}</span>
      <span class="grow"></span>
      {#if node.issue.priority}<span class="fx-prio dim">{node.issue.priority}</span>{/if}
      <span class="fx-status">{node.issue.status}</span>
      <span class="fx-ago dim">{ago(node.issue.updated_at)}</span>
    </button>
    <button
      class="fx-ext"
      onclick={() => openExternal(node.issue.url)}
      title="Open in Jira"
      aria-label="Open {node.issue.key} in Jira"
    >
      <Icon name="globe" size={12} />
    </button>
  </div>
  {#each node.children as child (child.issue.key)}
    {@render issueRow(child, depth + 1)}
  {/each}
{/snippet}

<div class="focus" class:quick={quickKey !== null}>
  <div class="focus-main">
    <!-- ── MY PULL REQUESTS ── -->
    <section class="fx-section">
      <header class="fx-head">
        <span class="fx-title">MY PULL REQUESTS</span>
        <span class="fx-count">{visiblePrRows.length}</span>
        <span class="grow"></span>
        {#if myHandles.size > 0}
          <div class="fx-seg">
            <button class:active={prFilter === 'all'} onclick={() => (prFilter = 'all')}>All open</button>
            <button class:active={prFilter === 'mine'} onclick={() => (prFilter = 'mine')}>Opened by me</button>
          </div>
        {/if}
        <button class="btn small ghost" onclick={() => void loadPrs()} title="Refresh pull requests">
          <Icon name="refresh" size={12} />
        </button>
      </header>

      {#if prsLoading}
        <div style="padding: 10px"><Skeleton rows={4} height={26} /></div>
      {:else if prGroups.length === 0}
        <div class="fx-empty dim">
          No open pull requests{prFilter === 'mine' ? ' opened by you' : ''} across
          {git.allRepos.length} registered repo{git.allRepos.length === 1 ? '' : 's'}.
          {#if prErrors > 0}({prErrors} repo{prErrors === 1 ? '' : 's'} failed to list){/if}
        </div>
      {:else}
        {#if prErrors > 0}
          <div class="fx-warn dim">{prErrors} repo{prErrors === 1 ? '' : 's'} failed to list PRs (missing account/permissions) — showing the rest.</div>
        {/if}
        {#each prGroups as [forge, rows] (forge)}
          <div class="fx-group-head">
            <ProviderIcon provider={forge} size={13} />
            <span>{forgeLabel[forge] ?? forge}</span>
            <span class="fx-count">{rows.length}</span>
          </div>
          {#each rows as row (row.repo.id + '#' + row.pr.number)}
            <div class="fx-pr">
              <span class="fx-ago dim" title={row.pr.updated_at}>{ago(row.pr.updated_at)}</span>
              <button
                class="fx-pr-title"
                onclick={() => openExternal(row.pr.url)}
                title="Open !{row.pr.number} on the provider"
              >
                <span class="mono fx-pr-num">#{row.pr.number}</span>
                {#if row.pr.draft}<span class="fx-draft">draft</span>{/if}
                <span class="fx-pr-text">{row.pr.title}</span>
              </button>
              <span class="dim fx-pr-author" title="Author">{row.pr.author}</span>
              <button
                class="fx-repo-link"
                onclick={() => {
                  git.openRepoTab(row.repo.id, 'prs');
                  router.go(`git/${row.repo.id}/prs`);
                }}
                title="Open {row.repo.name} in Otto"
              >
                <Icon name="branch" size={11} />
                {row.repo.name}
              </button>
              <span class="fx-branch mono" title="{row.pr.source_branch} → {row.pr.target_branch}">
                {row.pr.source_branch}
              </span>
              {#if row.pr.ci_status}
                <span class="fx-ci ci-{row.pr.ci_status}">{row.pr.ci_status}</span>
              {/if}
            </div>
          {/each}
        {/each}
      {/if}
    </section>

    <!-- ── MY WORK (Jira) ── -->
    <section class="fx-section">
      <header class="fx-head">
        <span class="fx-title">MY WORK</span>
        {#if !workLoading && work.length > 0}<span class="fx-count">{work.length}</span>{/if}
        <span class="grow"></span>
        {#if issueAccounts.length > 1}
          <select class="input fx-account" bind:value={issueAccountId} title="Jira account">
            {#each issueAccounts as a (a.id)}
              <option value={a.id}>{a.label}</option>
            {/each}
          </select>
        {/if}
        {#if issueAccountId}
          <button class="btn small ghost" onclick={() => void loadWork(issueAccountId)} title="Refresh my work">
            <Icon name="refresh" size={12} />
          </button>
        {/if}
      </header>

      {#if issueAccounts.length === 0 && !workLoading}
        <div class="fx-empty dim">
          No Jira account connected — add one under Settings → Integrations to see your assigned work here.
        </div>
      {:else if workLoading}
        <div style="padding: 10px"><Skeleton rows={5} height={24} /></div>
      {:else if workError}
        <div class="fx-empty dim">Failed to load your Jira work: {workError}</div>
      {:else if work.length === 0}
        <div class="fx-empty dim">Nothing assigned to you 🎉</div>
      {:else}
        {#each projects as proj (proj.key)}
          <div class="fx-group-head">
            <Icon name="folder" size={12} />
            <span>{proj.name || proj.key}</span>
            <span class="mono dim">{proj.key}</span>
          </div>
          {#each proj.groups as g (g.key)}
            <div class="fx-parent">
              <span class="fx-type t-epic">{g.type || 'Parent'}</span>
              <span class="mono fx-key dim">{g.key}</span>
              <span class="fx-parent-summary">{g.summary}</span>
            </div>
            {#each g.children as node (node.issue.key)}
              {@render issueRow(node, 1)}
            {/each}
          {/each}
          {#each proj.loose as node (node.issue.key)}
            {@render issueRow(node, 0)}
          {/each}
        {/each}
      {/if}
    </section>
  </div>

  <!-- ── Issue quick-view side panel ── -->
  {#if quickKey !== null}
    <aside class="fx-quick">
      <header class="fx-quick-head">
        <span class="mono fx-key">{quickKey}</span>
        <span class="grow"></span>
        <button
          class="btn small ghost"
          onclick={() => quickIssue && openExternal(quickIssue.url)}
          disabled={!quickIssue}
          title="Open in Jira"
        >
          <Icon name="globe" size={12} /> Jira
        </button>
        <button class="fx-quick-close" onclick={closeQuick} title="Close" aria-label="Close issue quick view">✕</button>
      </header>
      {#if quickLoading}
        <div style="padding: 12px"><Skeleton rows={6} height={20} /></div>
      {:else if quickIssue}
        <div class="fx-quick-body">
          <div class="fx-quick-summary">{quickIssue.summary}</div>
          <div class="fx-quick-meta">
            <span class="fx-type">{quickIssue.issue_type}</span>
            <span class="fx-status">{quickIssue.status}</span>
            {#if quickIssue.assignee}<span class="dim">→ {quickIssue.assignee}</span>{/if}
          </div>
          {#if quickIssue.description}
            <pre class="fx-quick-desc">{quickIssue.description}</pre>
          {:else}
            <div class="dim" style="font-size: 12px">No description.</div>
          {/if}
        </div>
      {:else}
        <div class="fx-empty dim">Failed to load {quickKey}.</div>
      {/if}
    </aside>
  {/if}
</div>

<style>
  .focus {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .focus-main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: 14px 16px 24px;
    display: flex;
    flex-direction: column;
    gap: 22px;
  }
  .fx-section {
    display: flex;
    flex-direction: column;
  }
  .fx-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-bottom: 6px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 4px;
  }
  .fx-title {
    font-size: 12px;
    font-weight: 800;
    letter-spacing: 0.08em;
  }
  .fx-count {
    font-size: 10px;
    font-weight: 700;
    min-width: 16px;
    padding: 0 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
    text-align: center;
  }
  .grow {
    flex: 1;
  }
  .fx-seg {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .fx-seg button {
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    padding: 3px 10px;
    cursor: pointer;
  }
  .fx-seg button.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
    font-weight: 600;
  }
  .fx-empty {
    padding: 14px 10px;
    font-size: 12px;
  }
  .fx-warn {
    padding: 4px 10px;
    font-size: 11px;
  }
  .fx-group-head {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 8px 4px 4px;
    font-size: 11px;
    font-weight: 700;
    color: var(--text);
  }
  .fx-group-head .mono {
    font-weight: 500;
    font-size: 10px;
  }

  /* ── PR rows ── */
  .fx-pr {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 6px;
    border-radius: var(--radius-s);
    min-height: 28px;
  }
  .fx-pr:hover {
    background: var(--surface-2);
  }
  .fx-ago {
    font-size: 10.5px;
    width: 26px;
    flex-shrink: 0;
    text-align: end;
  }
  .fx-pr-title {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 2 1 0;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
    font-size: 12px;
    padding: 0;
  }
  .fx-pr-title:hover .fx-pr-text {
    color: var(--accent);
    text-decoration: underline;
  }
  .fx-pr-num {
    color: var(--accent);
    font-size: 11px;
    font-weight: 600;
    flex-shrink: 0;
  }
  .fx-draft {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    padding: 0 5px;
    border-radius: 3px;
    background: var(--surface-2);
    color: var(--text-dim);
    border: 1px dashed var(--border);
    flex-shrink: 0;
  }
  .fx-pr-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .fx-pr-author {
    font-size: 11px;
    flex: 0 0 auto;
    max-width: 130px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fx-repo-link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: none;
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    color: var(--accent);
    border-radius: 999px;
    font-size: 10.5px;
    font-weight: 600;
    padding: 2px 8px;
    cursor: pointer;
    flex-shrink: 0;
    max-width: 160px;
    overflow: hidden;
    white-space: nowrap;
  }
  .fx-repo-link:hover {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .fx-branch {
    font-size: 10.5px;
    color: var(--text-dim);
    background: var(--surface-2);
    border-radius: 3px;
    padding: 1px 6px;
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex-shrink: 1;
  }
  .fx-ci {
    font-size: 9.5px;
    font-weight: 700;
    padding: 1px 6px;
    border-radius: 999px;
    flex-shrink: 0;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .fx-ci.ci-success {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    color: var(--status-working);
  }
  .fx-ci.ci-failed,
  .fx-ci.ci-failure {
    background: color-mix(in srgb, var(--status-exited) 18%, transparent);
    color: var(--status-exited);
  }

  /* ── Jira rows ── */
  .fx-parent {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 5px 6px 3px;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .fx-parent-summary {
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .fx-issue {
    display: flex;
    align-items: center;
    gap: 2px;
    border-radius: var(--radius-s);
  }
  .fx-issue:hover {
    background: var(--surface-2);
  }
  .fx-issue-main {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
    min-height: 28px;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
    font-size: 12px;
    padding: 2px 6px;
    border-radius: var(--radius-s);
  }
  .fx-issue-main.quick-open {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .fx-type {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding: 1px 5px;
    border-radius: 3px;
    background: var(--surface-2);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .fx-type.t-epic {
    background: color-mix(in srgb, #c678dd 20%, transparent);
    color: #c678dd;
  }
  .fx-type.t-story {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    color: var(--status-working);
  }
  .fx-type.t-bug {
    background: color-mix(in srgb, var(--status-exited) 18%, transparent);
    color: var(--status-exited);
  }
  .fx-type.t-sub-task,
  .fx-type.t-subtask {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .fx-key {
    font-size: 11px;
    font-weight: 600;
    color: var(--accent);
    flex-shrink: 0;
  }
  .fx-summary {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .fx-prio {
    font-size: 10px;
    flex-shrink: 0;
  }
  .fx-status {
    font-size: 10px;
    font-weight: 600;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    flex-shrink: 0;
    white-space: nowrap;
  }
  .fx-ext {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    border-radius: var(--radius-s);
    opacity: 0;
    flex-shrink: 0;
  }
  .fx-issue:hover .fx-ext {
    opacity: 1;
  }
  .fx-ext:hover {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .fx-account {
    height: 26px;
    font-size: 11.5px;
    max-width: 180px;
  }

  /* ── Quick-view side panel ── */
  .fx-quick {
    width: 380px;
    flex-shrink: 0;
    border-inline-start: 1px solid var(--border);
    background: var(--surface);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .fx-quick-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .fx-quick-close {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 12px;
    padding: 2px 6px;
    border-radius: 4px;
  }
  .fx-quick-close:hover {
    color: var(--text);
    background: var(--surface-2);
  }
  .fx-quick-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .fx-quick-summary {
    font-size: 13.5px;
    font-weight: 600;
    line-height: 1.4;
  }
  .fx-quick-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    flex-wrap: wrap;
  }
  .fx-quick-desc {
    margin: 0;
    font-size: 12px;
    line-height: 1.55;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: inherit;
    color: var(--text);
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* ── ≤1024px: quick view becomes an overlay; PR rows wrap. ── */
  @media (max-width: 1024px) {
    .fx-quick {
      position: absolute;
      inset-inline-end: 0;
      top: 0;
      bottom: 0;
      width: min(92vw, 380px);
      z-index: 5;
      box-shadow: -8px 0 24px rgba(0, 0, 0, 0.25);
    }
    .focus {
      position: relative;
    }
    .fx-pr {
      flex-wrap: wrap;
      row-gap: 2px;
    }
    .fx-issue-main {
      min-height: 38px;
    }
    .fx-ext {
      opacity: 1;
      width: 34px;
      height: 34px;
    }
  }
</style>
