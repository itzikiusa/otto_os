<script lang="ts">
  // One run in the Runs inbox (ux.md §3): the shared InboxRow, words only (no
  // ring, DR-020) — line 1 "{workflow} · {repository}" … the run's word (else
  // the time), line 2 the PR or ticket, line 3 the engine's headline, who
  // started it and where. Right-click / long-press: the row's menu.
  import InboxRow from '../../../lib/components/InboxRow.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { inboxFold, runContext, runLine3, runTitle, runWord, ago, type InboxRun } from './runWords';

  interface Props {
    run: InboxRun;
    /** Finished since the person last looked and never opened. */
    unread: boolean;
    now: number;
    onopen: (run: InboxRun) => void;
    /** The row menu's other actions (absent ones are left out). */
    menu?: (run: InboxRun) => MenuItem[];
  }
  let { run, unread, now, onopen, menu }: Props = $props();

  const w = $derived(run.overview ? runWord(run.overview, { startedAt: run.startedAt, now }) : runWord({ state: run.state, attention: [] }, { startedAt: run.startedAt, now, raw: run.status }));
  // A finished run that was read shows when it finished instead of "Done".
  const word = $derived(run.state === 'succeeded' && !unread ? null : w.word);
  const time = $derived(run.finishedAt ? ago(now - Date.parse(run.finishedAt)) : '');

  function openMenu(e: MouseEvent): void {
    e.preventDefault();
    ctxMenu.show(e, [{ label: 'Open', icon: 'external', action: () => onopen(run) }, ...(menu?.(run) ?? [])]);
  }
</script>

<div class="run-row" data-testid="wf-run-row" data-run-id={run.id} data-state={run.state}>
  <InboxRow
    title={runTitle(run)}
    {word}
    tone={word ? w.tone : ''}
    {time}
    onPage
    strong={inboxFold(run.state) === 'needs' || unread}
    tip={run.overview?.headline}
    onclick={() => onopen(run)}
    oncontextmenu={openMenu}
  >
    {#snippet context()}<span class="ctx ellipsis" dir="auto">{runContext(run)}</span>{/snippet}
    {#snippet line3()}<span class="ellipsis" dir="auto">{runLine3(run, now)}</span>{/snippet}
  </InboxRow>
</div>

<style>
  .ctx {
    min-inline-size: 0;
  }
</style>
