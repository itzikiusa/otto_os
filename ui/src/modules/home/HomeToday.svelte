<script lang="ts">
  // The top of the Home desktop: a greeting + today's line, then four glance
  // cards — Needs you · Running · Up next · Recent (data: today.svelte.ts).
  // Text that sits straight on the ambient backdrop is --text only (AA over
  // any backdrop pixel — unit/ambient.test.ts); dim text lives on the cards.
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { now } from '../../lib/stores/now.svelte';
  import { router } from '../../lib/router.svelte';
  import { relTime } from '../mission-control/lib';
  import { today, type TodayRow } from './today.svelte';

  const ROWS = 3;

  $effect(() => {
    today.start();
    return () => today.stop();
  });
  // A workspace switch changes every fetched list.
  $effect(() => {
    void ws.currentId;
    today.refresh();
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
    if (today.needs.length) parts.push(`${today.needs.length} need${today.needs.length === 1 ? 's' : ''} you`);
    if (today.running.length) parts.push(`${today.running.length} running`);
    return parts.length ? parts.join(' · ') : 'All quiet';
  });

  /** "3m ago" for past rows, "in 2h" for what's coming (Up next) — the
   *  compact relTime clamps future times to "0s". */
  function when(iso: string): string {
    const t = Date.parse(iso);
    if (!Number.isFinite(t)) return '';
    if (t <= Date.now()) return `${relTime(iso)} ago`;
    const mins = Math.round((t - Date.now()) / 60_000);
    return mins < 1 ? 'now' : mins < 60 ? `in ${mins}m` : `in ${Math.round(mins / 60)}h`;
  }

  interface Card {
    id: string;
    title: string;
    icon: IconName;
    rows: TodayRow[];
    empty: string;
    /** Where "N more" goes. */
    more: () => void;
    /** Rows come from a fetch (skeleton until it settles). */
    fetched?: boolean;
  }
  const cards: Card[] = $derived([
    { id: 'needs', title: 'Needs you', icon: 'bell', rows: today.needs, empty: 'Nothing needs you right now.', more: () => router.go('agents') },
    { id: 'running', title: 'Running', icon: 'play', rows: today.running, empty: 'No agents or workflows are working.', more: () => router.go('mission-control') },
    {
      id: 'next',
      title: 'Up next',
      icon: 'calendar',
      rows: today.upNext,
      empty: 'Nothing due in the next 24 hours. Reminders and scheduled tasks land here.',
      more: () => router.go(today.upNext.some((r) => r.id.startsWith('scheduled:')) ? 'scheduled-tasks' : 'assistant/tasks'),
      fetched: true,
    },
    { id: 'recent', title: 'Recent', icon: 'clock', rows: today.recent, empty: 'Designs and pull requests you touch show up here.', more: () => router.go('design'), fetched: true },
  ]);
</script>

<section class="today" aria-label="Today">
  <div class="greet">
    <h2 class="hello">{greeting}{name ? `, ${name}` : ''}</h2>
    <p class="line">
      {dateLine} · {summary}
      {#if today.failed}
        <!-- A source failed: say so instead of letting an empty card read as
             "nothing here". Retry re-polls every source at once. -->
        <span class="warn-line"><Icon name="warning" size={12} /> Some items couldn't load</span>
        <button class="retry" onclick={() => today.refresh()}>Retry</button>
      {/if}
    </p>
  </div>

  <div class="glance">
    {#each cards as c (c.id)}
      <article class="gcard" data-card={c.id} aria-labelledby="today-{c.id}">
        <header class="gc-head">
          <span class="gc-icon" class:warn={c.id === 'needs' && c.rows.length > 0}><Icon name={c.icon} size={13} /></span>
          <h3 id="today-{c.id}">{c.title}</h3>
          {#if c.rows.length}<span class="gc-count" class:warn={c.id === 'needs'}>{c.rows.length}</span>{/if}
        </header>
        {#if c.fetched && !today.loaded}
          <div class="gc-skel"><Skeleton rows={2} height={20} /></div>
        {:else if c.rows.length === 0}
          <div class="gc-empty">
            <span class="gc-empty-mark"><Icon name={c.id === 'needs' ? 'check' : c.icon} size={14} /></span>
            <p>{c.empty}</p>
          </div>
        {:else}
          <ul class="gc-rows">
            {#each c.rows.slice(0, ROWS) as r (r.id)}
              <li>
                <button class="gc-row" onclick={r.open} title={r.title}>
                  <span class="gc-mark">
                    {#if r.tone === 'working'}
                      <StatusDot status="working" size={7} />
                    {:else}
                      <span class="gc-dot {r.tone}"></span>
                    {/if}
                  </span>
                  <span class="gc-text">
                    <span class="gc-title">{r.title}</span>
                    <span class="gc-sub" title={r.detail}>{r.detail}{#if r.at && now() && when(r.at)} · {when(r.at)}{/if}</span>
                  </span>
                </button>
              </li>
            {/each}
          </ul>
          {#if c.rows.length > ROWS}
            <button class="gc-more" onclick={c.more}>{c.rows.length - ROWS} more</button>
          {/if}
        {/if}
      </article>
    {/each}
  </div>
</section>

<style>
  .today {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .greet {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-inline: 2px;
  }
  /* Straight on the backdrop: --text only (see the header comment). */
  .hello {
    margin: 0;
    font-size: var(--fs-xl);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text);
  }
  .line {
    margin: 0;
    font-size: var(--fs-m);
    color: var(--text);
    opacity: 1;
  }
  .warn-line {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-inline-start: 8px;
  }
  .retry {
    margin-inline-start: 6px;
    padding: 0;
    border: none;
    background: none;
    color: var(--text);
    font: inherit;
    font-weight: 600;
    text-decoration: underline;
    cursor: pointer;
  }
  .glance {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 12px;
  }
  @media (max-width: 1024px) {
    .glance {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
  .gcard {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    min-height: 148px;
    padding: 12px 12px 10px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow-card);
  }
  .gc-head {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .gc-icon {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    flex: none;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .gc-icon.warn {
    background: var(--warning-soft);
    color: var(--warning);
  }
  h3 {
    flex: 1;
    min-width: 0;
    margin: 0;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }
  .gc-count {
    font-size: var(--fs-xs);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    min-width: 20px;
    height: 18px;
    padding: 0 6px;
    display: inline-grid;
    place-items: center;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .gc-count.warn {
    background: var(--warning-soft);
    color: var(--warning);
  }
  /* A calm, centred "nothing here" — never a blank card. */
  .gc-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 4px 8px 8px;
    text-align: center;
  }
  .gc-empty p {
    margin: 0;
    max-width: 30ch;
    font-size: var(--fs-s);
    line-height: 1.45;
    color: var(--text-dim);
  }
  .gc-empty-mark {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .gcard[data-card='needs'] .gc-empty-mark {
    background: var(--success-soft);
    color: var(--success);
  }
  .gc-skel {
    padding-top: 4px;
  }
  .gc-rows {
    list-style: none;
    margin: 0 -6px;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .gc-row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    width: 100%;
    padding: 5px 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
    transition: background 120ms ease-out;
  }
  .gc-row:hover {
    background: var(--hover);
  }
  .gc-mark {
    flex: none;
    display: inline-grid;
    place-items: center;
    width: 10px;
    height: 18px;
  }
  .gc-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--status-idle);
  }
  .gc-dot.warning {
    background: var(--status-warn);
  }
  .gc-dot.accent {
    background: var(--accent);
  }
  .gc-text {
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .gc-title,
  .gc-sub {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .gc-title {
    font-size: var(--fs-m);
    font-weight: 500;
  }
  .gc-sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .gc-more {
    align-self: flex-start;
    margin-top: auto;
    padding: 2px 0;
    border: none;
    background: none;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .gc-more:hover {
    text-decoration: underline;
  }
  /* Phone: one column, and a card with nothing in it is a single calm line
     so the widgets stay within reach. */
  @media (max-width: 640px) {
    .glance {
      grid-template-columns: minmax(0, 1fr);
      gap: 8px;
    }
    .gcard {
      min-height: 0;
    }
    .gcard:has(.gc-empty) {
      flex-direction: row;
      align-items: center;
    }
    .gcard:has(.gc-empty) .gc-head {
      flex: none;
    }
    .gc-empty {
      flex-direction: row;
      justify-content: flex-start;
      padding: 0;
      text-align: start;
    }
    .gc-empty-mark {
      display: none;
    }
  }
</style>
