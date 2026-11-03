<script lang="ts">
  // Changes tab: the working tree, or — once the branch is ahead of its
  // upstream — the local commits a Create PR would send (T3 0.1.6 "review
  // local commits in the Diff pane"): all of them as one range, or one commit.
  import { api, isAbortError } from '../../lib/api/client';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { plural } from '../../lib/plural';
  import { onTabKey } from '../../lib/tabKeys';
  import { git } from '../../lib/stores/git.svelte';
  import type { CommitInfo, DiffLine, DiffResp, RepoStatusResp } from '../../lib/api/types';
  import { matchRepoForCwd } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';

  interface Props {
    workspaceId: string | null;
    sessionCwd: string | null;
    wsKey: string;
  }
  let { workspaceId, sessionCwd, wsKey }: Props = $props();

  /** One row of the file list, from either the working tree or a commit diff. */
  interface ChangeRow {
    path: string;
    oldPath: string | null;
    kind: string;
    added: number | null;
    deleted: number | null;
  }

  const MAX_COMMITS = 50;

  let status = $state<RepoStatusResp | null>(null);
  let summaryByPath = $state<Record<string, { added: number; deleted: number }>>({});
  let statusLoading = $state(false);
  let statusError = $state<string | null>(null);
  let statusRetry = $state(0);

  let scope = $state<'working' | 'commits'>('working');
  let commits = $state<CommitInfo[]>([]);
  /** `range:<upstream>...HEAD` (all local commits) or `commit:<sha>`. */
  let commitTarget = $state<string | null>(null);
  let commitFiles = $state<ChangeRow[]>([]);
  let commitsLoading = $state(false);
  let commitsError = $state<string | null>(null);
  let commitsRetry = $state(0);

  let selectedPath = $state<string | null>(null);
  let diffResp = $state<DiffResp | null>(null);
  let diffLoading = $state(false);
  let diffError = $state<string | null>(null);
  let diffMode = $state<'unified' | 'split'>('unified');

  // Ensure workspace repos are loaded.
  $effect(() => {
    if (workspaceId) void git.loadRepos(workspaceId);
  });

  const repo = $derived(matchRepoForCwd(sessionCwd, git.repos));
  const ahead = $derived(status?.upstream ? (status.ahead ?? 0) : 0);
  const allRange = $derived(status?.upstream ? `range:${status.upstream}...HEAD` : null);
  const activeScope = $derived(scope === 'commits' && ahead > 0 ? 'commits' : 'working');
  const diffTarget = $derived(activeScope === 'commits' ? commitTarget : 'working');

  const workingRows = $derived<ChangeRow[]>(
    (status?.changes ?? []).map((c) => ({
      path: c.path,
      oldPath: c.orig_path,
      kind: c.kind,
      added: summaryByPath[c.path]?.added ?? null,
      deleted: summaryByPath[c.path]?.deleted ?? null,
    })),
  );
  const rows = $derived(activeScope === 'commits' ? commitFiles : workingRows);
  const selectedRow = $derived(rows.find((r) => r.path === selectedPath) ?? null);

  // Keep a valid selection as the list changes.
  $effect(() => {
    if (rows.length === 0) selectedPath = null;
    else if (!selectedPath || !rows.some((r) => r.path === selectedPath)) selectedPath = rows[0].path;
  });

  // Load working-tree status + numstat summary for the active repo.
  $effect(() => {
    const repoId = repo?.id ?? null;
    void statusRetry;
    if (!repoId) {
      status = null;
      summaryByPath = {};
      return;
    }
    const ctl = new AbortController();
    statusLoading = true;
    statusError = null;
    void Promise.all([
      api.get<RepoStatusResp>(`/repos/${repoId}/status`, ctl.signal),
      api.get<DiffResp>(`/repos/${repoId}/diff?target=working&summary=true`, ctl.signal).catch(() => ({ files: [] })),
    ])
      .then(([st, sum]) => {
        if (ctl.signal.aborted) return;
        status = st;
        const map: Record<string, { added: number; deleted: number }> = {};
        for (const f of sum.files) map[f.path] = { added: f.added ?? 0, deleted: f.deleted ?? 0 };
        summaryByPath = map;
      })
      .catch((e) => {
        if (ctl.signal.aborted || isAbortError(e)) return;
        statusError = loadErrorText(e);
      })
      .finally(() => {
        if (!ctl.signal.aborted) statusLoading = false;
      });
    return () => ctl.abort();
  });

  // Local commits (HEAD's newest `ahead` — what the upstream doesn't have yet).
  $effect(() => {
    const repoId = repo?.id ?? null;
    const n = Math.min(ahead, MAX_COMMITS);
    void commitsRetry;
    if (activeScope !== 'commits' || !repoId || n === 0) {
      commits = [];
      return;
    }
    const ctl = new AbortController();
    void api
      .get<CommitInfo[]>(`/repos/${repoId}/log?limit=${n}`, ctl.signal)
      .then((list) => {
        if (!ctl.signal.aborted) commits = list.slice(0, n);
      })
      .catch(() => {
        if (!ctl.signal.aborted) commits = [];
      });
    return () => ctl.abort();
  });

  // Default (and re-aim after a push/rebase) to "all local commits".
  $effect(() => {
    if (activeScope !== 'commits') return;
    if (!commitTarget || (commitTarget.startsWith('range:') && commitTarget !== allRange)) commitTarget = allRange;
  });

  // Files touched by the chosen commit / range.
  $effect(() => {
    const repoId = repo?.id ?? null;
    const target = activeScope === 'commits' ? commitTarget : null;
    void commitsRetry;
    if (!repoId || !target) {
      commitFiles = [];
      return;
    }
    const ctl = new AbortController();
    commitsLoading = true;
    commitsError = null;
    void api
      .get<DiffResp>(`/repos/${repoId}/diff?target=${encodeURIComponent(target)}&summary=true`, ctl.signal)
      .then((r) => {
        if (ctl.signal.aborted) return;
        commitFiles = r.files.map((f) => ({
          path: f.path,
          oldPath: f.old_path,
          kind: f.status ?? 'modified',
          added: f.added ?? null,
          deleted: f.deleted ?? null,
        }));
      })
      .catch((e) => {
        if (ctl.signal.aborted || isAbortError(e)) return;
        commitFiles = [];
        commitsError = loadErrorText(e);
      })
      .finally(() => {
        if (!ctl.signal.aborted) commitsLoading = false;
      });
    return () => ctl.abort();
  });

  // Load per-file hunks when the selection or the target changes.
  $effect(() => {
    const repoId = repo?.id ?? null;
    const path = selectedPath;
    const target = diffTarget;
    const oldPath = selectedRow?.oldPath ?? null;
    if (!repoId || !path || !target) {
      diffResp = null;
      diffError = null;
      diffLoading = false;
      return;
    }
    const ctl = new AbortController();
    diffLoading = true;
    diffError = null;
    const q = `target=${encodeURIComponent(target)}&path=${encodeURIComponent(path)}${
      oldPath && oldPath !== path && target !== 'working' ? `&old_path=${encodeURIComponent(oldPath)}` : ''
    }`;
    void api
      .get<DiffResp>(`/repos/${repoId}/diff?${q}`, ctl.signal)
      .then((res) => {
        if (!ctl.signal.aborted) diffResp = res;
      })
      .catch((e) => {
        if (ctl.signal.aborted || isAbortError(e)) return;
        diffResp = null;
        diffError = loadErrorText(e);
      })
      .finally(() => {
        if (!ctl.signal.aborted) diffLoading = false;
      });
    return () => ctl.abort();
  });

  const activeFileDiff = $derived(diffResp?.files[0] ?? null);

  function kindTone(kind: string): 'ok' | 'warn' | 'bad' | 'info' | 'neutral' {
    switch (kind) {
      case 'added':
      case 'untracked':
        return 'ok';
      case 'deleted':
      case 'conflicted':
        return 'bad';
      case 'modified':
        return 'warn';
      default:
        return 'info';
    }
  }

  function buildSplitRows(lines: DiffLine[]): { left: DiffLine | null; right: DiffLine | null }[] {
    const out: { left: DiffLine | null; right: DiffLine | null }[] = [];
    let i = 0;
    while (i < lines.length) {
      const cur = lines[i];
      if (cur.origin === 'context') {
        out.push({ left: cur, right: cur });
        i++;
      } else if (cur.origin === 'del') {
        const dels: DiffLine[] = [];
        while (i < lines.length && lines[i].origin === 'del') dels.push(lines[i++]);
        const adds: DiffLine[] = [];
        while (i < lines.length && lines[i].origin === 'add') adds.push(lines[i++]);
        const max = Math.max(dels.length, adds.length);
        for (let k = 0; k < max; k++) {
          out.push({ left: dels[k] ?? null, right: adds[k] ?? null });
        }
      } else {
        out.push({ left: null, right: cur });
        i++;
      }
    }
    return out;
  }
</script>

<div class="tch-wrap" data-testid="inspector-changes-tab">
  {#if !repo}
    <EmptyState
      variant="panel"
      icon="branch"
      title="No Git repository detected"
      body="Register a repository in Git or start a thread inside a git working tree to inspect live diffs."
    />
  {:else}
    <div class="tch-head">
      {#if ahead > 0}
        <div class="segmented" role="group" aria-label="What to review">
          <button
            type="button"
            class:active={activeScope === 'working'}
            aria-pressed={activeScope === 'working'}
            onkeydown={onTabKey}
            title="Uncommitted changes in the working tree"
            data-testid="changes-scope-working"
            onclick={() => (scope = 'working')}
          >
            Uncommitted{status && status.changes.length > 0 ? ` ${status.changes.length}` : ''}
          </button>
          <button
            type="button"
            class:active={activeScope === 'commits'}
            aria-pressed={activeScope === 'commits'}
            onkeydown={onTabKey}
            title="Commits not on {status?.upstream ?? 'the upstream'} yet — what a push or PR would send"
            data-testid="changes-scope-commits"
            onclick={() => (scope = 'commits')}
          >
            Unpushed {ahead}
          </button>
        </div>
      {:else}
        <div class="tch-repo-meta">
          <Icon name="branch" size={13} />
          <span class="mono ellipsis" dir="ltr">{status?.branch ?? repo.name}</span>
          {#if status && status.changes.length > 0}
            <Badge tone="info" label={`${status.changes.length}`} />
          {/if}
        </div>
      {/if}
      <div class="tch-head-actions">
        <div class="segmented" role="group" aria-label="Diff layout">
          <button
            type="button"
            class:active={diffMode === 'unified'}
            aria-pressed={diffMode === 'unified'}
            onkeydown={onTabKey}
            onclick={() => (diffMode = 'unified')}
          >
            Unified
          </button>
          <button
            type="button"
            class:active={diffMode === 'split'}
            aria-pressed={diffMode === 'split'}
            onkeydown={onTabKey}
            onclick={() => (diffMode = 'split')}
          >
            Split
          </button>
        </div>
        <button
          type="button"
          class="icon-btn"
          aria-label="Refresh changes"
          title="Refresh changes"
          onclick={() => {
            statusRetry++;
            commitsRetry++;
          }}
        >
          <Icon name="refresh" size={12} />
        </button>
      </div>
    </div>

    {#if activeScope === 'commits'}
      <div class="tch-commit-pick">
        <select
          class="input"
          aria-label="Commit to review"
          data-testid="changes-commit-select"
          bind:value={commitTarget}
        >
          {#if allRange}
            <option value={allRange}>All {plural(ahead, 'unpushed commit')}</option>
          {/if}
          {#each commits as c (c.sha)}
            <option value="commit:{c.sha}">{c.short_sha} · {c.subject}</option>
          {/each}
        </select>
      </div>
    {/if}

    <LoadState
      what={activeScope === 'commits' ? 'commit changes' : 'working-tree changes'}
      variant="panel"
      loading={activeScope === 'commits' ? commitsLoading : statusLoading}
      error={activeScope === 'commits' ? commitsError : statusError}
      empty={rows.length === 0}
      onretry={() => (activeScope === 'commits' ? commitsRetry++ : statusRetry++)}
    >
      {#snippet emptyView()}
        <EmptyState
          variant="panel"
          icon="check"
          title={activeScope === 'commits' ? 'No file changes' : 'Working tree clean'}
          body={activeScope === 'commits'
            ? 'The selected commits change no files.'
            : `No uncommitted changes in ${repo?.name ?? 'this repo'}.`}
        />
      {/snippet}

      {#if rows.length > 0}
        <div class="tch-files">
          {#each rows as ch (ch.path)}
            <div class="tch-file-row" class:selected={selectedPath === ch.path}>
              <button
                type="button"
                class="tch-file-btn"
                dir="ltr"
                title={ch.oldPath && ch.oldPath !== ch.path ? `${ch.oldPath} → ${ch.path}` : ch.path}
                onclick={() => (selectedPath = ch.path)}
              >
                <Badge tone={kindTone(ch.kind)} variant="outline" label={ch.kind.slice(0, 1).toUpperCase()} />
                <span class="mono ellipsis">{ch.path}</span>
                {#if ch.added || ch.deleted}
                  <span class="tch-counts mono">
                    {#if ch.added}<span class="tch-add">+{ch.added}</span>{/if}
                    {#if ch.deleted}<span class="tch-del">-{ch.deleted}</span>{/if}
                  </span>
                {/if}
              </button>
              <button
                type="button"
                class="icon-btn"
                aria-label="Open {ch.path} in Files"
                title="Open in Files tab"
                onclick={() => repo && threads.openFileTab(wsKey, `${repo.path.replace(/\/+$/, '')}/${ch.path}`, { pin: true })}
              >
                <Icon name="file" size={12} />
              </button>
            </div>
          {/each}
        </div>

        <div class="tch-diff-pane">
          <LoadState
            what="file diff"
            variant="compact"
            loading={diffLoading}
            error={diffError}
            empty={!activeFileDiff || activeFileDiff.hunks.length === 0}
            onretry={() => {
              const p = selectedPath;
              selectedPath = null;
              selectedPath = p;
            }}
          >
            {#snippet emptyView()}
              <p class="tch-diff-empty">
                {activeFileDiff?.is_binary ? 'Binary file changed.' : 'Select a changed file to inspect its hunks.'}
              </p>
            {/snippet}

            {#if activeFileDiff}
              <div class="tch-hunks mono" dir="ltr">
                {#each activeFileDiff.hunks as hunk, hIdx (hIdx)}
                  <div class="tch-hunk-head">{hunk.header}</div>
                  {#if diffMode === 'unified'}
                    {#each hunk.lines as ln, lIdx (lIdx)}
                      <div class="tch-line {ln.origin}">
                        <span class="tch-ln">{ln.old_line ?? ''}</span>
                        <span class="tch-ln">{ln.new_line ?? ''}</span>
                        <span class="tch-sig">{ln.origin === 'add' ? '+' : ln.origin === 'del' ? '-' : ' '}</span>
                        <span class="tch-code">{ln.content}</span>
                      </div>
                    {/each}
                  {:else}
                    {#each buildSplitRows(hunk.lines) as pair, pIdx (pIdx)}
                      <div class="tch-split-row">
                        <div class="tch-half {pair.left?.origin ?? 'empty'}">
                          <span class="tch-ln">{pair.left?.old_line ?? ''}</span>
                          <span class="tch-code">{pair.left?.content ?? ''}</span>
                        </div>
                        <div class="tch-half {pair.right?.origin ?? 'empty'}">
                          <span class="tch-ln">{pair.right?.new_line ?? ''}</span>
                          <span class="tch-code">{pair.right?.content ?? ''}</span>
                        </div>
                      </div>
                    {/each}
                  {/if}
                {/each}
              </div>
            {/if}
          </LoadState>
        </div>
      {/if}
    </LoadState>
  {/if}
</div>

<style>
  .tch-wrap {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }
  .tch-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 8px;
    border-block-end: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
  }
  .tch-repo-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-size: var(--fs-xs);
    font-weight: 500;
  }
  .tch-head-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .tch-commit-pick {
    padding: 6px 8px;
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .tch-commit-pick select {
    inline-size: 100%;
    font-size: var(--fs-s);
  }
  .tch-files {
    max-block-size: 192px;
    overflow-y: auto;
    padding: 4px 6px;
    border-block-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex-shrink: 0;
  }
  .tch-file-row {
    display: flex;
    align-items: center;
    gap: 4px;
    border-radius: var(--radius-s);
  }
  .tch-file-row:hover {
    background: var(--hover);
  }
  .tch-file-row.selected {
    background: var(--accent-soft);
  }
  .tch-file-btn {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
  }
  .tch-counts {
    margin-inline-start: auto;
    display: inline-flex;
    gap: 4px;
    font-size: var(--fs-xs);
    flex-shrink: 0;
  }
  .tch-add {
    color: var(--success);
  }
  .tch-del {
    color: var(--danger);
  }
  .tch-diff-pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: auto;
    background: var(--bg);
  }
  .tch-diff-empty {
    margin: 0;
    padding: 16px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .tch-hunks {
    font-size: var(--fs-xs);
    line-height: 1.5;
  }
  .tch-hunk-head {
    padding: 4px 8px;
    background: var(--surface-2);
    color: var(--text-dim);
    border-block: 1px solid var(--border);
  }
  .tch-line {
    display: flex;
    white-space: pre;
    min-width: max-content;
  }
  .tch-line.add,
  .tch-half.add {
    background: var(--success-soft);
  }
  .tch-line.del,
  .tch-half.del {
    background: var(--danger-soft);
  }
  .tch-ln {
    inline-size: 36px;
    padding-inline: 4px;
    text-align: end;
    color: var(--text-dim);
    user-select: none;
    flex-shrink: 0;
  }
  .tch-sig {
    inline-size: 16px;
    text-align: center;
    color: var(--text-dim);
    user-select: none;
    flex-shrink: 0;
  }
  .tch-code {
    padding-inline-end: 12px;
  }
  .tch-split-row {
    display: grid;
    grid-template-columns: repeat(2, minmax(220px, 1fr));
    border-block-end: 1px solid var(--separator);
  }
  .tch-half {
    display: flex;
    white-space: pre;
    overflow-x: auto;
  }
  .tch-half + .tch-half {
    border-inline-start: 1px solid var(--border);
  }
</style>
