<script lang="ts">
  // Otto School — the accessible companion list (ClassroomsBox): the
  // "needs you" section and every classroom with its kids as real buttons,
  // paged so a big workspace never mounts every row at once. Visible in List
  // view; in 3D it stays keyboard / screen-reader reachable.
  import { rowMenu } from '../../../../lib/rowMenu';
  import Icon from '../../../../lib/components/Icon.svelte';
  import EmptyState from '../../../../lib/components/EmptyState.svelte';
  import ProviderIcon from '../../../../lib/components/ProviderIcon.svelte';
  import { ui } from '../../../../lib/stores/ui.svelte';
  import { plural } from '../../../../lib/plural';
  import { relTime } from '../../../mission-control/lib';
  import { kidAriaLabel, roomSummary, type Kid, type School } from '../../school/model';
  import type { SchoolHandle } from '../../school/scene';

  interface Props {
    mode: '3d' | 'list';
    empty: boolean;
    school: School;
    queue: Kid[];
    handle: SchoolHandle | null;
    /** Current page per classroom (kept by the box across re-mounts). */
    memberPages: Record<string, number>;
    open: (k: Kid) => Promise<void>;
    /** Release from detention (un-archive): an explicit action, never a row click. */
    free: (k: Kid) => Promise<void>;
    goTo: (k: Kid) => Promise<void>;
    kidMenu: (e: MouseEvent | KeyboardEvent, k: Kid) => void;
    showIn3d: (k: Kid) => Promise<void>;
  }
  let { mode, empty, school, queue, handle, memberPages = $bindable(), open, free, goTo, kidMenu, showIn3d }: Props = $props();
  /** A detention row's main click shows the kid (its card in 3D, its actions in
   *  List view); it never un-archives the session. */
  function rowClick(e: MouseEvent, k: Kid): void {
    if (!k.detention) void open(k);
    else if (mode === '3d' && handle) void showIn3d(k);
    else kidMenu(e, k);
  }

  // Keep complete membership without mounting every session row at once.
  const LIST_PAGE_SIZE = 100;
</script>

<div class="list" class:sr-only={mode === '3d'} role="region" aria-label="School list">
  {#if mode === 'list' && empty}
    <EmptyState icon="people" title="School’s out" body="Start a session and a kid takes a seat in its workspace’s classroom.">
      <button class="btn small" onclick={() => (ui.newSessionOpen = true)}><Icon name="plus" size={12} />New session</button>
    </EmptyState>
  {/if}
  {#if queue.length > 0}
    <section class="lroom lattn" aria-label="Needs you">
      <h4 class="lroom-head"><span class="ellipsis">Needs you</span><small>{plural(queue.length, 'session')}</small></h4>
      <ul>
        {#each queue as k (k.id)}
          <li class="lrow" data-attn-id={k.id}>
            <button class="lmain" aria-label="Go to {kidAriaLabel(k, relTime(k.lastActiveAt))} in {k.workspaceName}" onclick={() => void goTo(k)}>
              <i class="dot" data-pose={k.pose}></i>
              <ProviderIcon provider={k.provider} size={12} />
              <span class="ellipsis t">{k.title}</span>
              <span class="st">{k.workspaceName} · {k.stateLabel}</span>
            </button>
          </li>
        {/each}
      </ul>
    </section>
  {/if}
  {#each school.rooms as r (r.id)}
    {@const lastPage = Math.max(0, Math.ceil(r.members.length / LIST_PAGE_SIZE) - 1)}
    {@const page = Math.min(memberPages[r.id] ?? 0, lastPage)}
    {#if r.members.length > 0 || mode === 'list'}
      <section class="lroom" aria-label="Classroom {r.name}">
        <h4 class="lroom-head">
          <span class="ellipsis">{r.name}</span>
          {#if r.current}<span class="cur">current</span>{/if}
          <small>{roomSummary(r)}</small>
          {#if mode === '3d'}
            <button class="btn small" onclick={() => void handle?.enterRoom(r.id)} disabled={!handle}>Walk in</button>
          {/if}
        </h4>
        {#if r.members.length > 0}
          <ul>
            {#each r.members.slice(page * LIST_PAGE_SIZE, (page + 1) * LIST_PAGE_SIZE) as k (k.id)}
              <li use:rowMenu class="lrow" data-kid-id={k.id} oncontextmenu={(e) => kidMenu(e, k)}>
                <button
                  class="lmain"
                  aria-label="{k.detention ? 'Show' : 'Open'} {kidAriaLabel(k, relTime(k.lastActiveAt))}{k.detention ? ', in detention' : ''}"
                  onclick={(e) => rowClick(e, k)}
                  onfocus={() => void showIn3d(k)}
                >
                  <i class="dot" data-pose={k.detention ? 'bench' : k.pose}></i>
                  <ProviderIcon provider={k.provider} size={12} />
                  <span class="ellipsis t">{k.title}</span>
                  {#if k.background}<span class="bg-tag">back row</span>{/if}
                  {#if k.detention}<span class="bg-tag">detention</span>{/if}
                  <span class="st">{k.stateLabel}</span>
                </button>
                {#if k.detention}
                  <button
                    class="btn small"
                    disabled={!k.canManage}
                    title={k.canManage ? 'Bring this session back from the archive' : 'You can’t manage this session, so you can’t release it'}
                    aria-label="Release {k.title} from detention"
                    onclick={() => void free(k)}
                  >
                    Release
                  </button>
                {/if}
                <button class="icon-btn" aria-label="More actions for {k.title}" title="More actions for {k.title}" aria-haspopup="menu" onclick={(e) => kidMenu(e, k)}>
                  <Icon name="more" size={12} />
                </button>
              </li>
            {/each}
          </ul>
        {/if}
        {#if lastPage > 0}
          <nav class="member-pages" aria-label="Sessions in classroom {r.name}">
            <button class="btn small" aria-label="Previous sessions in {r.name}" disabled={page === 0} onclick={() => (memberPages[r.id] = page - 1)}>Previous</button>
            <span aria-live="polite">{page * LIST_PAGE_SIZE + 1}–{Math.min((page + 1) * LIST_PAGE_SIZE, r.members.length)} of {r.members.length}</span>
            <button class="btn small" aria-label="Next sessions in {r.name}" disabled={page === lastPage} onclick={() => (memberPages[r.id] = page + 1)}>Next</button>
          </nav>
        {/if}
        {#if r.overflow > 0}<p class="more">{r.overflow} not seated in 3D</p>{/if}
      </section>
    {/if}
  {/each}
</div>

<style>
  .list {
    padding-inline: 4px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    overflow-y: auto;
    min-height: 0;
  }
  /* Keep the plain action path available to screen readers, and reveal its
     context as soon as any keyboard control receives focus. */
  .list.sr-only:focus-within {
    position: static;
    width: auto;
    height: auto;
    flex: 0 1 45%;
    min-block-size: 100px;
    max-block-size: 45%;
    margin: 0;
    padding: 6px;
    overflow: auto;
    clip: auto;
    clip-path: none;
    white-space: normal;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
  }
  .member-pages {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding-block: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .lroom-head {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 4px;
    font-size: var(--fs-s);
  }
  .lroom-head small {
    flex: 1;
    color: var(--text-dim);
    font-weight: 400;
    font-size: var(--fs-xs);
  }
  .cur {
    font-size: var(--fs-xs);
    color: var(--accent-text);
  }
  .lroom ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .lrow {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .lmain {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border: 0;
    background: none;
    border-radius: var(--radius-s);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .lmain:hover {
    background: var(--surface-2);
  }
  .t {
    flex: 1;
    min-width: 0;
  }
  .st,
  .bg-tag,
  .more {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .more {
    margin: 2px 6px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
    background: var(--text-dim);
  }
  .dot[data-pose='working'] {
    background: var(--success);
  }
  .dot[data-pose='needs-you'] {
    background: var(--warning);
  }
  .dot[data-pose='error'] {
    background: var(--danger);
  }
  .dot[data-pose='done'] {
    background: var(--info);
  }
  .dot[data-pose='away'],
  .dot[data-pose='bench'] {
    background: var(--border-strong);
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
