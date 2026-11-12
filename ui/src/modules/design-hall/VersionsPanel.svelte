<script lang="ts">
  // The Versions tab of the artifact view's side panel: the history, newest
  // first. Each row says what the version is (message, author — agents marked
  // — kind, approved) and offers View (that version alone, read-only), Compare
  // with current, Restore (as a NEW version — history never rewinds) and
  // Approve. Runs of autosaves fold into "N autosaves". "Load older" pages
  // with offset/limit against the server's `X-Total-Count`. ↑/↓ move through
  // the list. A failed refresh over loaded rows is LoadState's stale bar
  // (spoken once, with Retry). Nothing here ever deletes.
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { plural } from '../../lib/plural';
  import type { DesignVersion } from '../../lib/api/types';
  import { versionAuthor } from './model';
  import { versionKindLabel, versionRows } from './versionRows';
  import type { VersionList } from './versionList.svelte';

  interface Props {
    list: VersionList;
    headId: string | null;
    approvedId: string | null;
    meId: string | null | undefined;
    /** The version shown alone in the stage (null: the working copy). */
    viewingId: string | null;
    canEdit: boolean;
    onview: (v: DesignVersion | null) => void;
    oncompare: (v: DesignVersion) => void;
    onrestore: (v: DesignVersion) => void;
    onapprove: (v: DesignVersion) => void;
  }
  let { list, headId, approvedId, meId, viewingId, canEdit, onview, oncompare, onrestore, onapprove }: Props = $props();

  let expanded = $state<Set<string>>(new Set());
  const rows = $derived(versionRows(list.items, { headId, approvedId, keep: viewingId ? [viewingId] : [], expanded }));
  let listEl = $state<HTMLUListElement | null>(null);

  function when(v: DesignVersion): string {
    return new Date(v.created_at).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
  }
  function title(v: DesignVersion): string {
    return v.message?.trim() || (v.kind === 'autosave' ? 'Autosave' : `Version ${v.seq}`);
  }

  function onKey(e: KeyboardEvent): void {
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key)) return;
    const btns = [...(listEl?.querySelectorAll<HTMLButtonElement>('button.row-main') ?? [])];
    const i = btns.indexOf(e.target as HTMLButtonElement);
    if (i < 0) return;
    e.preventDefault();
    const j = e.key === 'Home' ? 0 : e.key === 'End' ? btns.length - 1 : Math.max(0, Math.min(btns.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1)));
    btns[j]?.focus();
  }

  function rowMenu(e: MouseEvent, v: DesignVersion): void {
    const isHead = v.id === headId;
    const items: MenuItem[] = [
      { label: isHead ? 'Back to current' : `View v${v.seq}`, icon: 'eye', action: () => onview(isHead ? null : v) },
      { label: 'Compare with current', icon: 'columns', disabled: isHead, action: () => oncompare(v) },
      { label: `Restore v${v.seq}…`, icon: 'refresh', disabled: isHead || !canEdit, action: () => onrestore(v) },
      { label: 'Approve this version…', icon: 'check', disabled: !canEdit || v.id === approvedId, action: () => onapprove(v) },
    ];
    ctxMenu.show(e, items);
  }
</script>

<div class="versions" data-testid="design-versions-panel">
  <LoadState what="versions" variant="panel" loading={list.loading} error={list.error} empty={list.items.length === 0} onretry={() => void list.load()}>
    {#snippet emptyView()}<p class="dim pad">No versions yet — the first save makes v1.</p>{/snippet}
    <ul class="list" bind:this={listEl} aria-label="Versions, newest first">
      {#each rows as r (r.kind === 'fold' ? r.key : r.version.id)}
        {#if r.kind === 'fold'}
          <li>
            <button class="row-main fold" onkeydown={onKey} onclick={() => (expanded = new Set([...expanded, r.key]))} aria-expanded="false" data-testid="design-version-fold">
              <Icon name="chevronRight" size={12} />
              <span>{plural(r.versions.length, 'autosave')}</span>
              <span class="dim">v{r.versions.at(-1)?.seq}–v{r.versions[0].seq}</span>
            </button>
          </li>
        {:else}
          {@const v = r.version}
          {@const isHead = v.id === headId}
          {@const viewed = v.id === viewingId}
          <li class="vrow" class:viewed class:named={v.kind === 'named'} data-testid="design-version-row" data-seq={v.seq} data-kind={v.kind}>
            <div class="line">
              <button class="row-main" onkeydown={onKey} aria-current={viewed || (isHead && !viewingId) ? 'true' : undefined}
                onclick={() => onview(isHead ? null : v)} title={isHead ? 'The current version' : `View v${v.seq} on its own (read-only)`}>
                <span class="seq">v{v.seq}</span>
                <span class="msg">{title(v)}</span>
                <span class="meta">
                  <span class="av" class:agent={v.author_kind === 'agent'} class:system={v.author_kind === 'system'} aria-hidden="true"></span>
                  {versionAuthor(v, meId)}{#if v.author_kind === 'agent'}<span class="agent-tag">agent</span>{/if}
                  · {versionKindLabel(v.kind)} · {when(v)}{#if isHead} · current{/if}
                </span>
              </button>
              {#if v.id === approvedId}<span class="appr" title="Approved version"><Icon name="check" size={12} /> Approved</span>{/if}
              <button class="icon-btn" onclick={(e) => rowMenu(e, v)} aria-haspopup="menu" aria-label={`Actions for v${v.seq}`} title={`Actions for v${v.seq}`} data-testid="design-version-menu">
                <Icon name="more" size={14} />
              </button>
            </div>
            {#if viewed}
              <div class="acts">
                <button class="btn small" onclick={() => oncompare(v)} data-testid="design-version-compare"><Icon name="columns" size={12} /> Compare with current</button>
                {#if canEdit}
                  <button class="btn small" onclick={() => onrestore(v)} data-testid="design-version-restore"><Icon name="refresh" size={12} /> Restore</button>
                  {#if v.id !== approvedId}<button class="btn small" onclick={() => onapprove(v)} data-testid="design-version-approve"><Icon name="check" size={12} /> Approve</button>{/if}
                {/if}
              </div>
            {/if}
          </li>
        {/if}
      {/each}
    </ul>
    {#if list.hasOlder}
      <button class="btn small ghost older" onclick={() => void list.loadOlder()} disabled={list.loadingOlder} data-testid="design-versions-older">
        {list.loadingOlder ? 'Loading older versions…' : `Load older · ${list.items.length} of ${list.total}`}
      </button>
    {/if}
  </LoadState>
</div>

<style>
  .versions {
    padding: 8px 0 12px;
  }
  .pad {
    padding: 8px 14px;
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .vrow {
    border-block-end: 1px solid var(--border);
  }
  .vrow.viewed {
    background: var(--accent-soft);
  }
  .line {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-inline-end: 8px;
  }
  .row-main {
    flex: 1;
    min-width: 0;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    column-gap: 8px;
    padding-block: 8px;
    padding-inline: 14px 6px;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
    min-block-size: var(--hit-min);
  }
  .row-main:hover {
    background: var(--hover);
  }
  .seq {
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    font-size: var(--fs-s);
  }
  .msg {
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .vrow:not(.named) .msg {
    color: var(--text-dim);
  }
  .named .msg {
    font-weight: 600;
  }
  .meta {
    grid-column: 2;
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .av {
    inline-size: 8px;
    block-size: 8px;
    border-radius: 50%;
    background: var(--text-dim);
    flex: none;
  }
  /* Agent-authored versions: a hollow ring (never the accent colour). */
  .av.agent {
    background: transparent;
    border: 2px solid var(--text);
  }
  .av.system {
    background: transparent;
    border: 1px dashed var(--text-dim);
  }
  .agent-tag {
    padding: 0 4px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
  }
  .appr {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    color: var(--success);
    font-size: var(--fs-xs);
    flex: none;
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 0 14px 10px;
  }
  .fold {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    border-block-end: 1px solid var(--border);
  }
  .older {
    margin: 10px 14px 0;
  }
</style>
