<script lang="ts">
  // The lobby's side rail — only what asks for a person: designs waiting for
  // review, and what agents drafted lately (with their live session). Both
  // read the shared library snapshot (newest 500, live-patched).
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { events } from '../../lib/events.svelte';
  import { sessionState } from '../../lib/status';
  import StudioBadge from './StudioBadge.svelte';
  import { library } from './library.svelte';

  interface Props {
    /** Show every design in review in the main grid (the Status chip). */
    onshowreview: () => void;
  }
  let { onshowreview }: Props = $props();

  const newest = $derived(
    [...library.hits].sort((a, b) => b.artifact.updated_at.localeCompare(a.artifact.updated_at)).map((h) => h.artifact),
  );
  const review = $derived(newest.filter((a) => a.status === 'review'));
  const agentWork = $derived(
    newest
      .filter((a) => a.created_by_kind === 'agent' && a.status !== 'archived')
      .slice(0, 4)
      .map((a) => ({ a, session: a.created_session_id ? (ws.getSession(a.created_session_id) ?? null) : null })),
  );
</script>

<aside class="rail" aria-label="Needs you">
  <section class="card box" aria-labelledby="dh-review-h" data-testid="design-needs-review">
    <header>
      <span class="ico"><Icon name="eye" size={14} /></span>
      <h2 id="dh-review-h">Needs review</h2>
      {#if review.length}<span class="n">{review.length}{library.truncated ? '+' : ''}</span>{/if}
    </header>
    {#if library.loading && !library.loaded}
      <Skeleton rows={3} height={28} />
    {:else if library.error && !library.loaded}
      <LoadState variant="compact" what="designs in review" error={library.error} empty onretry={() => void library.load()} />
    {:else if review.length === 0}
      <p class="dim">Nothing is waiting for review.</p>
    {:else}
      <ul>
        {#each review.slice(0, 5) as a (a.id)}
          <li>
            <StudioBadge studio={a.studio} />
            <a href={`#/design/a/${encodeURIComponent(a.id)}`} title={a.title}>{a.title}</a>
            <span class="when" title={new Date(a.updated_at).toLocaleString()}>{rel(a.updated_at)}</span>
          </li>
        {/each}
      </ul>
      <button class="linkbtn" onclick={onshowreview} data-testid="design-review-all">Show all in review</button>
    {/if}
  </section>

  {#if agentWork.length}
    <section class="card box" aria-labelledby="dh-activity-h">
      <header>
        <span class="ico"><Icon name="user" size={14} /></span>
        <h2 id="dh-activity-h">Agent activity</h2>
      </header>
      <ul>
        {#each agentWork as w (w.a.id)}
          <li class="act">
            <div class="act-line">
              {#if w.session}<StatusDot
                  state={sessionState(w.session, ws.statusMap[w.session.id], ws.needsYou[w.session.id] === true, { stale: events.state !== 'connected' })}
                />{/if}
              <span><span class="who">{w.session?.title ?? w.a.created_session_title ?? 'An agent'}</span> drafted
                <a href={`#/design/a/${encodeURIComponent(w.a.id)}`}>{w.a.title}</a></span>
            </div>
            <div class="act-meta">
              <span title={new Date(w.a.updated_at).toLocaleString()}>{rel(w.a.updated_at)}</span>
              {#if w.session}
                <button class="linkbtn" onclick={() => ws.navigateToSession(w.session!.id)}>Open session</button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    </section>
  {/if}
</aside>

<style>
  .rail { display: flex; flex-direction: column; gap: 16px; min-width: 0; }
  .box { padding: 14px; display: flex; flex-direction: column; gap: 8px; min-width: 0; }
  .box header { display: flex; align-items: center; gap: 8px; }
  .box h2 { margin: 0; font-size: var(--fs-m); font-weight: 600; flex: 1; }
  .n { font-size: var(--fs-s); font-weight: 600; color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .ico { width: 24px; height: 24px; display: grid; place-items: center; border-radius: var(--radius-s); background: var(--surface-2); color: var(--text-dim); }
  ul { list-style: none; margin: 0; padding: 0; }
  li { display: flex; align-items: center; gap: 8px; padding: 6px 0; border-block-start: 1px solid var(--border); font-size: var(--fs-s); min-width: 0; }
  li > a { flex: 1; min-width: 0; color: var(--text); font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .when { font-size: var(--fs-xs); color: var(--text-dim); flex: none; }
  li.act { display: block; padding: 8px 0; }
  .act-line { display: flex; align-items: baseline; gap: 6px; }
  .act-line a { color: var(--text); font-weight: 500; }
  .who { font-weight: 600; }
  .act-meta { display: flex; justify-content: space-between; font-size: var(--fs-xs); color: var(--text-dim); margin-block-start: 2px; }
  .dim { margin: 0; color: var(--text-dim); font-size: var(--fs-s); }
  .linkbtn { align-self: flex-start; border: 0; background: none; padding: 0; color: var(--accent-text); font: inherit; font-size: var(--fs-s); cursor: pointer; }
  .linkbtn:hover { text-decoration: underline; }
</style>
