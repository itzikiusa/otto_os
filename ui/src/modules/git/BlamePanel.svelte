<script lang="ts">
  // `git blame` for one file, opened from a diff header's ⋯ menu (via
  // `gitBridge`). The daemon returns one row per RUN of consecutive lines, so
  // the gutter shows a line range per commit instead of repeating the same
  // author 40 times. Clicking a row selects that commit in the graph.
  import type { BlameResp } from '../../lib/api/types';
  import { api } from '../../lib/api/client';
  import { gitBridge } from './gitBridge.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import Skeleton from '../../lib/components/Skeleton.svelte';

  interface Props {
    repoId: string;
    path: string;
    /** Revision to blame; the daemon defaults to HEAD. */
    rev?: string;
    onclose: () => void;
  }
  let { repoId, path, rev, onclose }: Props = $props();

  let blame = $state<BlameResp | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let retryRev = $state(0);

  $effect(() => {
    void retryRev;
    const id = repoId;
    const p = path;
    const r = rev ?? 'HEAD';
    loading = true;
    error = null;
    api
      .get<BlameResp>(
        `/repos/${id}/blame?path=${encodeURIComponent(p)}&rev=${encodeURIComponent(r)}`,
      )
      .then((b) => {
        blame = b;
      })
      .catch((e: unknown) => {
        blame = null;
        error = loadErrorText(e);
      })
      .finally(() => {
        loading = false;
      });
  });

  function day(at: string): string {
    const d = new Date(at);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString();
  }

  /** "12" for a single line, "12–15" for a run. */
  function range(start: number, count: number): string {
    return count > 1 ? `${start}–${start + count - 1}` : `${start}`;
  }
</script>

<section class="bl" aria-label="Blame">
  <header class="bl-head">
    <Icon name="user" size={13} />
    <span class="bl-title mono" title={path}>{path}</span>
    {#if blame}<span class="chip mono">{blame.rev}</span>{/if}
    <span class="grow"></span>
    <button class="icon-btn" onclick={onclose} aria-label="Close blame" title="Close blame">
      <Icon name="x" size={14} />
    </button>
  </header>

  <div class="bl-body">
    {#if loading}
      <div class="bl-pad"><Skeleton rows={6} height={22} /></div>
    {:else if error}
      <div class="bl-pad"><LoadState what="blame" {error} empty onretry={() => retryRev++} variant="compact" /></div>
    {:else if !blame || blame.lines.length === 0}
      <p class="bl-msg">Nothing to blame — the file is empty at this revision.</p>
    {:else}
      <table class="bl-table">
        <tbody>
          {#each blame.lines as l, i (`${l.sha}-${l.line_start}-${i}`)}
            <tr>
              <td class="bl-gutter mono">{range(l.line_start, l.count)}</td>
              <td class="bl-who">
                <button
                  class="bl-commit"
                  onclick={() => gitBridge.focusCommit(repoId, l.sha)}
                  title={l.summary}
                >
                  <span class="bl-author">{l.author}</span>
                  <span class="mono bl-sha">{l.short_sha}</span>
                  <span class="bl-date">{day(l.at)}</span>
                </button>
              </td>
              <td class="bl-summary" title={l.summary}>{l.summary}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
</section>

<style>
  .bl {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--surface);
    border-inline-start: 1px solid var(--border);
  }
  .bl-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
  }
  .bl-title {
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }
  .bl-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .bl-pad {
    padding: 10px;
  }
  .bl-msg {
    padding: 14px 12px;
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .bl-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-xs);
  }
  .bl-table tr:hover {
    background: var(--surface-2);
  }
  .bl-gutter {
    width: 1%;
    white-space: nowrap;
    padding: 3px 8px;
    color: var(--text-dim);
    text-align: end;
    vertical-align: top;
    border-inline-end: 1px solid var(--border);
  }
  .bl-who {
    width: 1%;
    white-space: nowrap;
    padding: 2px 8px;
    vertical-align: top;
  }
  .bl-commit {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-xs);
    padding: 1px 0;
    cursor: pointer;
  }
  .bl-commit:hover .bl-sha {
    color: var(--accent-text);
  }
  .bl-sha,
  .bl-date {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .bl-summary {
    padding: 3px 8px;
    color: var(--text-dim);
    vertical-align: top;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 0;
  }
</style>
