<script lang="ts">
  import { untrack } from 'svelte';
  // Stash browser: every stash with its message, branch, age and file count;
  // pick one to see its files and patch (`git stash show -p
  // --include-untracked`), then Apply / Pop / Drop it — each addressed by the
  // stash's SHA, never a positional stash@{N}. Opened from the graph's
  // STASHES list and the toolbar's Stashes button.
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { api } from '../../lib/api/client';
  import type { RepoStatusResp, StashDetail, StashInfo } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { loadErrorText } from '../../lib/loadError';
  import { plural } from '../../lib/plural';
  import { relativeAge, splitPatch, stashTitle } from './gitopsPlan';

  interface Props {
    repoId: string;
    /** Stash to open on (its SHA); the newest when absent. */
    initialSha?: string | null;
    onclose: () => void;
    /** After apply / pop / drop: the fresh repo status. */
    onchanged: (status: RepoStatusResp) => void;
  }
  let { repoId, initialSha = null, onclose, onchanged }: Props = $props();

  let stashes = $state<StashInfo[]>([]);
  let listLoading = $state(true);
  let listError = $state<string | null>(null);
  let selected = $state<string | null>(null);
  let detail = $state<StashDetail | null>(null);
  let detailLoading = $state(false);
  let detailError = $state<string | null>(null);
  let busy = $state<'' | 'apply' | 'pop' | 'drop'>('');
  let openFile = $state<string | null>(null);

  const current = $derived(stashes.find((s) => s.sha === selected) ?? null);
  const fileDiffs = $derived(detail ? splitPatch(detail.patch) : []);

  async function loadList(keep?: string | null): Promise<void> {
    listLoading = true;
    listError = null;
    try {
      stashes = await api.get<StashInfo[]>(`/repos/${repoId}/stashes`);
      const want = keep ?? selected ?? initialSha;
      const next = stashes.find((s) => s.sha === want)?.sha ?? stashes[0]?.sha ?? null;
      if (next !== selected || !detail) void select(next);
    } catch (e) {
      listError = loadErrorText(e);
    } finally {
      listLoading = false;
    }
  }

  let seq = 0;
  async function select(sha: string | null): Promise<void> {
    selected = sha;
    detail = null;
    openFile = null;
    detailError = null;
    if (!sha) return;
    const mine = ++seq;
    detailLoading = true;
    try {
      const d = await api.get<StashDetail>(`/repos/${repoId}/stashes/${encodeURIComponent(sha)}`);
      if (mine === seq) {
        detail = d;
        openFile = d.files[0]?.path ?? null;
      }
    } catch (e) {
      if (mine === seq) detailError = loadErrorText(e);
    } finally {
      if (mine === seq) detailLoading = false;
    }
  }

  $effect(() => {
    const sha = initialSha;
    untrack(() => void loadList(sha));
  });

  async function run(op: 'apply' | 'pop' | 'drop'): Promise<void> {
    const s = current;
    if (!s) return;
    const label = stashTitle(s.message);
    const files = s.files != null ? ` (${plural(s.files, 'file')})` : '';
    if (op === 'drop') {
      const ok = await confirmer.ask(
        `Drop ${s.ref} — “${label}”${files}? Its changes are discarded and cannot be recovered from Otto.`,
        { title: 'Drop stash', confirmLabel: 'Drop', danger: true },
      );
      if (!ok) return;
    } else if (op === 'pop') {
      const ok = await confirmer.ask(
        `Apply “${label}”${files} to your working tree and remove it from the stash list? If it conflicts, the stash is kept.`,
        { title: 'Pop stash', confirmLabel: 'Pop', danger: false },
      );
      if (!ok) return;
    }
    busy = op;
    try {
      const status = await api.post<RepoStatusResp>(`/repos/${repoId}/stash`, { op, sha: s.sha });
      onchanged(status);
      const conflicts = status.changes.filter((c) => c.kind === 'conflicted').length;
      if (conflicts > 0 && op !== 'drop') {
        toasts.warn(
          op === 'pop' ? 'Popped with conflicts' : 'Applied with conflicts',
          `${plural(conflicts, 'file')} need resolution. The stash entry was kept.`,
        );
      } else {
        toasts.success(op === 'apply' ? 'Stash applied' : op === 'pop' ? 'Stash popped' : 'Stash dropped', label);
      }
      await loadList(op === 'apply' ? s.sha : null);
    } catch (e) {
      toastError(`Couldn’t ${op} the stash`, e);
    } finally {
      busy = '';
    }
  }

  function statusIcon(st: string): string {
    return st === 'added' ? 'A' : st === 'deleted' ? 'D' : st === 'untracked' ? 'U' : st === 'renamed' ? 'R' : 'M';
  }
  function lineClass(l: string): string {
    if (l.startsWith('+++') || l.startsWith('---') || l.startsWith('diff --git') || l.startsWith('index ')) return 'meta';
    if (l.startsWith('@@')) return 'hunk';
    if (l.startsWith('+')) return 'add';
    if (l.startsWith('-')) return 'del';
    return '';
  }
</script>

<Modal title="Stashes" width={960} height={620} {onclose} dismissable={busy === ''}>
  <div class="browser">
    <div class="list" role="listbox" aria-label="Stashes">
      <LoadState what="stashes" loading={listLoading && stashes.length === 0} error={listError} empty={!listLoading && !listError && stashes.length === 0} onretry={() => loadList()} variant="compact" rows={4}>
        {#snippet emptyView()}
          <EmptyState icon="stash" title="No stashes" body="Stash your working changes from the toolbar to set them aside." variant="panel" headingLevel={4} />
        {/snippet}
        {#each stashes as s (s.sha)}
          <button
            type="button"
            role="option"
            aria-selected={s.sha === selected}
            class="sb-row"
            class:sel={s.sha === selected}
            onclick={() => void select(s.sha)}
            title={s.message}
          >
            <span class="row-top">
              <span class="msg">{stashTitle(s.message)}</span>
            </span>
            <span class="row-meta">
              <span class="mono">{s.ref}</span>
              {#if s.branch}<span class="mono branch"><Icon name="branch" size={12} />{s.branch}</span>{/if}
              <span>{relativeAge(s.date)}</span>
              {#if s.files != null}<span>{plural(s.files, 'file')}</span>{/if}
            </span>
          </button>
        {/each}
      </LoadState>
    </div>

    <div class="detail">
      {#if !current}
        {#if !listLoading && stashes.length > 0}
          <EmptyState icon="stash" title="Pick a stash" body="Its files and changes show here." variant="panel" headingLevel={4} />
        {/if}
      {:else}
        <div class="head">
          <div class="head-text">
            <div class="title">{stashTitle(current.message)}</div>
            <div class="sub dim">
              <span class="mono">{current.ref}</span>
              {#if current.branch}· on <span class="mono">{current.branch}</span>{/if}
              · {relativeAge(current.date)}
              {#if detail}· {plural(detail.files.length, 'file')}{/if}
            </div>
          </div>
          <div class="actions">
            <button class="btn small" disabled={busy !== '' || !detail} onclick={() => run('apply')} title="Apply to the working tree and keep the stash">
              {busy === 'apply' ? 'Applying…' : 'Apply'}
            </button>
            <button class="btn small" disabled={busy !== '' || !detail} onclick={() => run('pop')} title="Apply to the working tree and remove the stash">
              {busy === 'pop' ? 'Popping…' : 'Pop'}
            </button>
            <button class="btn small danger" disabled={busy !== ''} onclick={() => run('drop')} title="Discard this stash">
              {busy === 'drop' ? 'Dropping…' : 'Drop…'}
            </button>
          </div>
        </div>
        <LoadState what="the stash" loading={detailLoading} error={detailError} onretry={() => select(current.sha)} variant="compact" rows={6}>
          {#if detail}
            <div class="files" aria-label="Files in this stash">
              {#each detail.files as f (f.path + f.status)}
                <button type="button" class="file" class:sel={openFile === f.path} onclick={() => (openFile = openFile === f.path ? null : f.path)} aria-pressed={openFile === f.path} title={`${f.status} · ${f.path}`}>
                  <span class="st st-{f.status}" aria-label={f.status}>{statusIcon(f.status)}</span>
                  <span class="mono path">{f.path}</span>
                  {#if f.additions != null}<span class="add-n">+{f.additions}</span>{/if}
                  {#if f.deletions != null}<span class="del-n">−{f.deletions}</span>{/if}
                  {#if f.additions == null}<span class="dim">binary</span>{/if}
                </button>
              {/each}
            </div>
            <div class="patch" aria-label="Changes">
              {#each fileDiffs.filter((d) => openFile === null || d.path === openFile) as d (d.path)}
                <pre class="mono diff">{#each d.text.split('\n') as l, i (i)}<span class={lineClass(l)}>{l}</span>{'\n'}{/each}</pre>
              {:else}
                <div class="dim pad">No textual changes to show.</div>
              {/each}
              {#if detail.truncated}
                <div class="dim pad">The patch is large; only the first part is shown.</div>
              {/if}
            </div>
          {/if}
        </LoadState>
      {/if}
    </div>
  </div>

  {#snippet footer()}
    <button class="btn ghost" onclick={onclose} disabled={busy !== ''}>Close</button>
  {/snippet}
</Modal>

<style>
  .browser {
    display: grid;
    grid-template-columns: minmax(200px, 300px) 1fr;
    gap: var(--sp-3);
    min-height: 0;
    height: 100%;
  }
  @media (max-width: 640px) {
    .browser {
      grid-template-columns: 1fr;
    }
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
    max-height: 70vh;
    border-inline-end: 1px solid var(--separator);
    padding-inline-end: var(--sp-2);
  }
  .sb-row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    text-align: start;
    padding: var(--sp-2);
    border-radius: var(--radius-s);
    background: none;
    border: 1px solid transparent;
    color: var(--text);
    cursor: pointer;
  }
  .sb-row:hover {
    background: var(--hover);
  }
  .sb-row.sel {
    background: var(--accent-soft);
    border-color: var(--accent-line);
  }
  .msg {
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: block;
  }
  .row-meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .branch {
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }
  .detail {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    min-width: 0;
    overflow: hidden;
  }
  .head {
    display: flex;
    gap: var(--sp-3);
    align-items: flex-start;
    justify-content: space-between;
    flex-wrap: wrap;
  }
  .head-text {
    min-width: 0;
  }
  .title {
    font-size: var(--fs-m);
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .sub {
    font-size: var(--fs-xs);
  }
  .actions {
    display: flex;
    gap: var(--sp-2);
  }
  .files {
    display: flex;
    flex-direction: column;
    max-height: 30%;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .file {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-1) var(--sp-2);
    background: none;
    border: none;
    color: var(--text);
    text-align: start;
    cursor: pointer;
    font-size: var(--fs-xs);
  }
  .file:hover {
    background: var(--hover);
  }
  .file.sel {
    background: var(--accent-soft);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .st {
    width: 14px;
    text-align: center;
    font-weight: 600;
    color: var(--warning);
  }
  .st-added,
  .st-untracked {
    color: var(--success);
  }
  .st-deleted {
    color: var(--danger);
  }
  .add-n {
    color: var(--success);
  }
  .del-n {
    color: var(--danger);
  }
  .patch {
    flex: 1;
    min-height: 120px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
  }
  .diff {
    margin: 0;
    padding: var(--sp-2);
    font-size: var(--fs-xs);
    line-height: 1.5;
    white-space: pre;
  }
  .diff .add {
    color: var(--success);
    background: var(--success-soft);
  }
  .diff .del {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .diff .hunk {
    color: var(--info);
  }
  .diff .meta {
    color: var(--text-dim);
  }
  .pad {
    padding: var(--sp-3);
    font-size: var(--fs-xs);
  }
</style>
