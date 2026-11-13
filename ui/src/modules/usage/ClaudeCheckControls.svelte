<script lang="ts">
  // A Claude row's opt-in "Check with Anthropic" switch (off by default).
  // On: the daemon runs that login's own `claude` CLI and asks it for plan
  // usage (PUT /usage/limits/claude-check, POST …/run); Otto never reads the
  // login. Turning it on confirms first and says what runs and where it goes.
  // While on, a check that is missing or older than the daemon's 5-minute
  // cache runs when the page opens and again on the page's 60 s poll once it
  // is due (never while the tab is hidden, never more than once per 5 min);
  // "Check now" runs one on demand.
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import Switch from '../../lib/components/Switch.svelte';
  import type { AccountLimits, ClaudeCheck, LimitsResp } from '../../lib/api/types/usage-limits';
  import { CHECK_CONFIRM, autoCheckDue, checkStatusText } from './limitsText';

  interface Props {
    row: AccountLimits;
    /** The row's check state (the parent renders this only when it is known). */
    check: ClaudeCheck;
    now: number;
    /** The refreshed page answer after a change. */
    onresp: (resp: LimitsResp) => void;
  }
  let { row, check, now, onresp }: Props = $props();

  let busy = $state(false);
  /** The page clock at our last automatic run (not reactive: bookkeeping). */
  let lastAutoAt: number | null = null;

  async function run(): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      onresp(await api.post<LimitsResp>('/usage/limits/claude-check/run', { key: row.key }));
    } catch (e) {
      toasts.error('Couldn’t check with Anthropic', loadErrorText(e));
    } finally {
      busy = false;
    }
  }

  async function toggle(next: boolean): Promise<void> {
    if (busy) return;
    if (next && !(await confirmer.ask(CHECK_CONFIRM, { title: `Check ${row.label} with Anthropic?`, confirmLabel: 'Turn on', danger: false }))) return;
    busy = true;
    try {
      onresp(await api.put<LimitsResp>('/usage/limits/claude-check', { key: row.key, enabled: next }));
    } catch (e) {
      toasts.error(next ? 'Couldn’t turn on the check' : 'Couldn’t turn off the check', loadErrorText(e));
      return;
    } finally {
      busy = false;
    }
    if (next) await run();
  }

  // Re-evaluated whenever `now` moves (the parent's poll, which pauses while
  // the tab is hidden) or the check state changes.
  $effect(() => {
    const hidden = typeof document !== 'undefined' && document.visibilityState === 'hidden';
    if (autoCheckDue(check, now, lastAutoAt, hidden)) {
      lastAutoAt = now;
      void run();
    }
  });
</script>

<div class="check" data-testid="limits-check-{row.key}">
  <Switch checked={check.enabled} onchange={(v) => void toggle(v)} label="Check {row.label} with Anthropic" disabled={busy} />
  <span class="check-text">
    <span class="check-name">Check with Anthropic</span>
    <span class="small dim" data-testid="limits-check-status">{checkStatusText(check, busy, now)}</span>
  </span>
  {#if check.enabled}
    <button class="btn small" disabled={busy || check.checking} onclick={() => void run()}>Check now</button>
  {/if}
</div>

<style>
  .check {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px 10px;
    margin-block-start: 8px;
    min-width: 0;
  }
  .check-text {
    display: flex;
    flex-direction: column;
    flex: 1 1 160px;
    min-width: 0;
  }
  .check-name {
    font-size: var(--fs-s);
    color: var(--text);
  }
  .small {
    font-size: var(--fs-s);
  }
  .dim {
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
</style>
