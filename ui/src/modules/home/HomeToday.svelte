<script lang="ts">
  // The top of Home: one centred reading column (app.css .read-col) — the
  // greeting and today's line, then "Needs you" and "Working" as inbox rows
  // (lib/components/InboxRow, status words from lib/statusWords.ts), then
  // "Up next" and "Recent" only when they hold something (data:
  // today.svelte.ts). Approvals stay answerable in their row. The composer is
  // the floating bar, docked under this column on desktop; a phone or tablet
  // (no in-app bar) gets the same "Ask anything" entry here, opening the
  // command sheet. Text that sits straight on the ambient backdrop is --text
  // only (AA over any backdrop pixel — unit/ambient.test.ts); dim text lives on
  // the cards.
  import { untrack } from 'svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import ApprovalActions from '../../lib/components/ApprovalActions.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import InboxRow from '../../lib/components/InboxRow.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { wordTone } from '../../lib/statusWords';
  import { auth } from '../../lib/stores/auth.svelte';
  import { barStore } from '../../lib/stores/bar.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { now, rel } from '../../lib/stores/now.svelte';
  import { today, type TodayRow } from './today.svelte';

  const ROWS = 5;
  let expanded = $state<Record<string, boolean>>({});

  // untrack: start() runs the first load synchronously, and that load reads
  // ws.currentId / auth.can(). Tracked, every boot change to them (grants
  // arriving, the saved workspace resolving) stopped the poller — aborting
  // its load AFTER the requests went out — and started a new one: three
  // full Today loads before Home's first paint.
  $effect(() => {
    untrack(() => today.start());
    return () => today.stop();
  });
  // A workspace switch changes every fetched list, and on a cold boot the
  // first load waits for the list to settle (today.start). Not on mount:
  // start() already loads once the list is in, and a refresh then queued a
  // second full load (needs-you, tasks, approvals, designs…).
  let seenWs: string | undefined;
  $effect(() => {
    const key = ws.listSettled ? `ws:${ws.currentId}` : 'pending';
    if (seenWs !== undefined && key !== seenWs) today.refresh();
    seenWs = key;
  });

  const name = $derived(auth.me ? auth.me.display_name || auth.me.username : '');
  const greeting = $derived.by(() => {
    void now();
    const h = new Date().getHours();
    return h < 5 ? 'Good evening' : h < 12 ? 'Good morning' : h < 18 ? 'Good afternoon' : 'Good evening';
  });
  const dateLine = $derived.by(() => {
    void now();
    return new Date().toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long' });
  });
  const summary = $derived.by(() => {
    const parts: string[] = [];
    if (today.needs.length) parts.push(`${today.needs.length} ${today.needs.length === 1 ? 'needs' : 'need'} you`);
    if (today.running.length) parts.push(`${today.running.length} working`);
    return parts.length ? parts.join(' · ') : 'All quiet';
  });

  interface Section {
    id: string;
    title: string;
    rows: TodayRow[];
    /** Shown even when empty (with this line); else only with rows. */
    empty?: string;
  }
  const sections: Section[] = $derived(
    [
      { id: 'needs', title: 'Needs you', rows: today.needs, empty: 'Nothing needs you right now.' },
      { id: 'running', title: 'Working', rows: today.running, empty: 'Nothing is working right now.' },
      { id: 'next', title: 'Up next', rows: today.upNext },
      { id: 'recent', title: 'Recent', rows: today.recent },
    ].filter((s) => s.empty || s.rows.length > 0),
  );
</script>

<section class="today read-col" aria-label="Today">
  <div class="greet">
    <h2 class="hello">{greeting}{name ? `, ${name}` : ''}</h2>
    <p class="line">{dateLine} · {summary}</p>
    {#if today.failed}
      <!-- A source failed: say so via LoadState instead of letting an empty
           section read as "nothing here". Retry re-polls every source. -->
      <div class="today-err">
        <LoadState what="some Today items" variant="compact" error="One or more sources didn’t answer." empty={false} onretry={() => today.refresh()} />
      </div>
    {/if}
  </div>

  {#if !barStore.mounted}
    <button type="button" class="ask" data-testid="home-ask" onclick={() => ui.openPalette('english')}>
      <Icon name="sparkle" size={16} />
      <span>Ask anything, or run a command</span>
    </button>
  {/if}

  {#each sections as sec (sec.id)}
    <section class="sec" data-card={sec.id} aria-labelledby="today-{sec.id}">
      <h3 class="sec-title" id="today-{sec.id}">
        {sec.title}
        {#if sec.rows.length}<span class="sec-count">{sec.rows.length}</span>{/if}
      </h3>
      {#if sec.rows.length === 0}
        <p class="sec-empty">{sec.empty}</p>
      {:else}
        <div class="sec-card">
          {#each sec.rows.slice(0, expanded[sec.id] ? undefined : ROWS) as r (r.id)}
            <div class="sec-row" class:has-actions={!!r.approval}>
              <InboxRow
                title={r.title}
                project={r.project ?? null}
                word={r.word ?? null}
                tone={wordTone(r.word)}
                time={r.at ? rel(r.at) : ''}
                tip={r.title}
                strong={sec.id === 'needs'}
                onPage
                testid="home-row-{r.id}"
                state={sec.id}
                onclick={r.open}
              >
                {#snippet context()}
                  {#if !r.project && r.kind}<span class="sec-kind ellipsis">{r.kind}</span>{/if}
                {/snippet}
                {#snippet line3()}
                  <span class="sec-detail ellipsis" title={r.detail}>{r.detail}</span>
                {/snippet}
              </InboxRow>
              {#if r.approval}
                {@const appr = r.approval}
                {@const err = today.decideErrors[appr.id]}
                <div class="sec-actions">
                  <ApprovalActions
                    size="small"
                    busy={today.deciding[appr.id] ?? null}
                    disabled={!today.canDecideApproval(appr)}
                    disabledReason={today.decideBlockedReason(appr)}
                    denyTarget={appr.title}
                    denyTitle="Deny approval request"
                    testid={`home-approval-${appr.id}`}
                    onapprove={() => void today.decideApproval(appr, true)}
                    ondeny={(reason) => void today.decideApproval(appr, false, reason)}
                  />
                </div>
                {#if err}
                  <p class="sec-err" use:announceShown={{ text: err, assertive: true }}>{err}</p>
                {/if}
              {/if}
            </div>
          {/each}
          {#if sec.rows.length > ROWS}
            <button type="button" class="sec-more" aria-expanded={!!expanded[sec.id]} onclick={() => (expanded[sec.id] = !expanded[sec.id])}>
              {expanded[sec.id] ? 'Show less' : `${sec.rows.length - ROWS} more`}
            </button>
          {/if}
        </div>
      {/if}
    </section>
  {/each}
</section>

<style>
  .today {
    display: flex;
    flex-direction: column;
    gap: var(--sp-8);
  }
  .hello {
    margin: 0;
    font-size: var(--fs-2xl);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text);
  }
  .line {
    margin: var(--sp-2) 0 0;
    font-size: var(--fs-l);
    color: var(--text);
  }
  .today-err {
    margin-block-start: var(--sp-4);
  }
  /* The phone / tablet composer: the same "Ask" entry the desktop bar is. */
  .ask {
    display: flex;
    align-items: center;
    gap: var(--sp-5);
    inline-size: 100%;
    min-block-size: 52px;
    padding-inline: var(--sp-6);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    box-shadow: var(--shadow-card);
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-l);
    text-align: start;
    cursor: text;
  }
  .ask :global(svg) {
    color: var(--accent-text);
  }
  .sec {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
  }
  .sec-title {
    display: flex;
    align-items: baseline;
    gap: var(--sp-4);
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
  }
  .sec-count {
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }
  .sec-empty {
    margin: 0;
    font-size: var(--fs-m);
    color: var(--text);
  }
  .sec-card {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: var(--sp-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    box-shadow: var(--shadow-card);
  }
  .sec-row {
    display: flex;
    flex-direction: column;
  }
  .sec-row.has-actions {
    padding-block-end: var(--sp-3);
  }
  .sec-kind,
  .sec-detail {
    min-width: 0;
  }
  .sec-actions {
    padding-inline: var(--sp-5);
  }
  .sec-err {
    margin: var(--sp-2) 0 0;
    padding-inline: var(--sp-5);
    font-size: var(--fs-s);
    color: var(--danger);
  }
  .sec-more {
    align-self: flex-start;
    margin: var(--sp-2) var(--sp-5) var(--sp-3);
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  @media (pointer: coarse) {
    .sec-more {
      min-block-size: var(--hit-min);
    }
  }
</style>
