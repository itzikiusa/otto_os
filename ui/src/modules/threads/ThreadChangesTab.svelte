<script lang="ts">
  import { api, isAbortError } from '../../lib/api/client';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { onTabKey } from '../../lib/tabKeys';
  import { git } from '../../lib/stores/git.svelte';
  import type { DiffLine, DiffResp, FileChange, RepoStatusResp } from '../../lib/api/types';
  import { matchRepoForCwd } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';

  interface Props {
    workspaceId: string | null;
    sessionCwd: string | null;
    wsKey: string;
  }
  let { workspaceId, sessionCwd, wsKey }: Props = $props();

  let status = $state<RepoStatusResp | null>(null);
  let summaryByPath = $state<Record<string, { added: number; deleted: number }>>({});
  let statusLoading = $state(false);
  let statusError = $state<string | null>(null);
  let statusRetry = $state(0);

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

  // Load working-tree status + numstat summary for the active repo.
  $effect(() => {
    const repoId = repo?.id ?? null;
    void statusRetry;
    if (!repoId) {
      status = null;
      summaryByPath = {};
      selectedPath = null;
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
        for (const f of sum.files) {
          map[f.path] = { added: f.added ?? 0, deleted: f.deleted ?? 0 };
        }
        summaryByPath = map;
        if (st.changes.length > 0 && (!selectedPath || !st.changes.some((c) => c.path === selectedPath))) {
          selectedPath = st.changes[0].path;
        } else if (st.changes.length === 0) {
          selectedPath = null;
        }
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

  // Load per-file hunks when selectedPath changes.
  $effect(() => {
    const repoId = repo?.id ?? null;
    const path = selectedPath;
    if (!repoId || !path) {
      diffResp = null;
      diffError = null;
      diffLoading = false;
      return;
    }
    const ctl = new AbortController();
    diffLoading = true;
    diffError = null;
    void api
      .get<DiffResp>(`/repos/${repoId}/diff?target=working&path=${encodeURIComponent(path)}`, ctl.signal)
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

  function kindTone(kind: FileChange['kind']): 'ok' | 'warn' | 'bad' | 'info' | 'neutral' {
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
      <div class="tch-repo-meta">
        <Icon name="branch" size={13} />
        <span class="mono ellipsis" dir="ltr">{status?.branch ?? repo.name}</span>
        {#if status && status.changes.length > 0}
          <Badge tone="info" label={`${status.changes.length}`} />
        {/if}
      </div>
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
          aria-label="Refresh working tree changes"
          title="Refresh changes"
          onclick={() => statusRetry++}
        >
          <Icon name="refresh" size={12} />
        </button>
      </div>
    </div>

    <LoadState
      what="working-tree changes"
      variant="panel"
      loading={statusLoading}
      error={statusError}
      empty={!status || status.changes.length === 0}
      onretry={() => statusRetry++}
    >
      {#snippet emptyView()}
        <EmptyState
          variant="panel"
          icon="check"
          title="Working tree clean"
          body="No uncommitted changes in {repo.name}."
        />
      {/snippet}

      {#if status}
        <div class="tch-files">
          {#each status.changes as ch (ch.path)}
            {@const stat = summaryByPath[ch.path]}
            <div class="tch-file-row" class:selected={selectedPath === ch.path}>
              <button
                type="button"
                class="tch-file-btn"
                dir="ltr"
                onclick={() => (selectedPath = ch.path)}
              >
                <Badge tone={kindTone(ch.kind)} variant="outline" label={ch.kind.slice(0, 1).toUpperCase()} />
                <span class="mono ellipsis">{ch.path}</span>
                {#if stat}
                  <span class="tch-counts mono">
                    {#if stat.added > 0}<span class="tch-add">+{stat.added}</span>{/if}
                    {#if stat.deleted > 0}<span class="tch-del">-{stat.deleted}</span>{/if}
                  </span>
                {/if}
              </button>
              <button
                type="button"
                class="icon-btn"
                aria-label="Open {ch.path} in Files"
                title="Open in Files tab"
                onclick={() => threads.openFileTab(wsKey, `${repo.path.replace(/\/+$/, '')}/${ch.path}`, { pin: true })}
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
