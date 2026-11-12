<script lang="ts">
  // "Remaining" — the top of the Usage page: how much of each subscription
  // limit is left, per provider login, and when it resets (GET /usage/limits).
  // Numbers come from what the CLIs already write locally (Codex rollouts);
  // a login Otto has nothing for says so, with why. A percent is never shown
  // without its age once it is old (limitsText.ts keeps that rule).
  import { onMount } from 'svelte';
  import { api } from '../../lib/api/client';
  import { pollWhileVisible } from '../../lib/poll';
  import { loadErrorText } from '../../lib/loadError';
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import ClaudeCheckControls from './ClaudeCheckControls.svelte';
  import type { LimitsResp } from '../../lib/api/types/usage-limits';
  import {
    ageText,
    barTone,
    creditsText,
    isStale,
    meterLabel,
    planLabel,
    providerName,
    windowReset,
    windowText,
  } from './limitsText';

  let data = $state<LimitsResp | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  /** The clock "resets in" and the stale verdict are read against; moves
   *  with every poll (once a minute while the page is visible). */
  let now = $state(Date.now());
  let poller: { now(): void; stop(): void } | null = null;

  onMount(() => {
    poller = pollWhileVisible(
      async (signal) => {
        try {
          data = await api.get<LimitsResp>('/usage/limits', signal);
          error = null;
        } catch (e) {
          if (signal.aborted) return;
          error = loadErrorText(e);
          return false;
        } finally {
          loading = false;
          now = Date.now();
        }
      },
      { ms: 60_000 },
    );
    return () => poller?.stop();
  });

  const staleAfter = $derived.by(() => data?.stale_after_secs ?? 1800);
  function retry(): void {
    loading = true;
    poller?.now();
  }
</script>

<section class="card limits" aria-labelledby="usage-limits-title" data-testid="usage-remaining">
  <div class="limits-head">
    <h3 id="usage-limits-title">Remaining</h3>
    <span class="dim small">Plan limits per login, from the CLIs’ own files</span>
    <button
      class="icon-btn refresh-btn"
      aria-label="Refresh remaining limits"
      title="Refresh remaining limits"
      disabled={loading}
      onclick={retry}
    ><Icon name="refresh" size={14} /></button>
  </div>

  <!-- A failed refresh keeps the last answer under LoadState's "Couldn’t refresh" bar. -->
  <LoadState what="remaining limits" variant="compact" rows={2} {loading} {error} empty={!data || data.accounts.length === 0} onretry={retry}>
    {#snippet emptyView()}
      <EmptyState
        variant="panel"
        icon="gauge"
        headingLevel={4}
        title="No provider logins yet"
        body="Run a Codex session, or add a subscription account in Settings ▸ Providers. Its remaining limits show here."
      />
    {/snippet}
    {#if data}
      <ul class="acct-list">
        {#each data.accounts as a (a.key)}
          {@const stale = isStale(a, staleAfter, now)}
          <li class="acct" data-testid="limits-row-{a.key}" data-status={stale ? 'stale' : a.status}>
            <div class="acct-head">
              <span class="acct-name">{a.label}</span>
              <span class="dim small">{providerName(a.provider)}</span>
              {#if a.plan}<Badge tone="neutral" variant="outline" label={planLabel(a.plan)} />{/if}
              <span class="acct-age small">
                {#if a.status === 'unknown'}
                  <Badge tone="neutral" label="Unknown" testid="limits-unknown" />
                {:else if stale}
                  <Badge tone="warn" label="Stale" testid="limits-stale" title="This reading is older than {Math.round(staleAfter / 60)} min; use since then is not counted" />
                  <span class="dim">read {ageText(a.taken_at, now)}</span>
                {:else}
                  <span class="dim">updated {ageText(a.taken_at, now)}</span>
                {/if}
              </span>
            </div>

            {#if a.status === 'unknown'}
              <p class="reason small dim">{a.reason ?? 'Otto has no reading for this login.'}</p>
            {:else}
              {#if a.limit_reached && !stale}
                <p class="reached small"><Icon name="warning" size={12} /> Limit reached — new turns wait for the reset.</p>
              {/if}
              <div class="wins">
                {#each a.windows as w (w.id)}
                  {@const reset = windowReset(w, now)}
                  {@const text = windowText(w, stale, a.taken_at, now)}
                  {@const left = reset ? null : w.left_percent}
                  <div class="win" class:stale class:reset class:inactive={w.active === false} data-testid="limits-window-{w.id}">
                    <span class="win-name">{w.name}</span>
                    <div
                      class="bar {barTone(left)}"
                      role="meter"
                      aria-valuemin={0}
                      aria-valuemax={100}
                      aria-valuenow={left ?? undefined}
                      aria-valuetext={text}
                      aria-label={meterLabel(a, w, text)}
                    >
                      {#if left !== null}<div class="used" style="inline-size: {Math.max(0, Math.min(100, 100 - left))}%"></div>{/if}
                    </div>
                    <span class="win-text small" class:dim={stale || reset}>{text}</span>
                  </div>
                {/each}
              </div>
              {#if creditsText(a.credits)}
                <p class="credits small dim">{creditsText(a.credits)}</p>
              {/if}
            {/if}
            {#if a.provider === 'claude' && a.check}
              <ClaudeCheckControls row={a} check={a.check} {now} onresp={(r) => { data = r; error = null; now = Date.now(); }} />
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </LoadState>
</section>

<style>
  .limits {
    padding: 14px 16px;
    margin: 0 0 14px;
    min-width: 0;
  }
  .limits-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 10px;
    margin-bottom: 10px;
  }
  .limits-head h3 {
    font-size: var(--fs-m);
    font-weight: 600;
    margin: 0;
    color: var(--text);
  }
  .refresh-btn {
    margin-inline-start: auto;
  }
  .small {
    font-size: var(--fs-s);
  }
  .dim {
    color: var(--text-dim);
  }
  .acct-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .acct {
    padding-block: 10px;
    border-block-start: 1px solid var(--separator);
    min-width: 0;
  }
  .acct:first-child {
    border-block-start: none;
    padding-block-start: 0;
  }
  .acct-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 8px;
    margin-bottom: 6px;
    min-width: 0;
  }
  .acct-name {
    font-weight: 600;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .acct-age {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-inline-start: auto;
  }
  .reason,
  .credits,
  .reached {
    margin: 4px 0 0;
  }
  .reached {
    color: var(--danger);
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .wins {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  /* Name · bar · text on one line on desktop; on a phone the text wraps to a
     line of its own under the bar (grid areas, no horizontal scroll). */
  .win {
    display: grid;
    grid-template-columns: minmax(64px, max-content) minmax(80px, 1fr) minmax(0, auto);
    align-items: center;
    gap: 4px 10px;
    min-width: 0;
  }
  .win-name {
    font-size: var(--fs-s);
    color: var(--text);
    white-space: nowrap;
  }
  .win-text {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .bar {
    block-size: 8px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    overflow: hidden;
    min-width: 0;
  }
  .bar .used {
    block-size: 100%;
    background: var(--accent-solid);
  }
  .bar.warning .used {
    background: var(--warning);
  }
  .bar.danger .used {
    background: var(--danger);
  }
  /* An old reading is visibly not live: the fill fades. */
  .win.stale .bar .used {
    opacity: 0.45;
  }
  /* Listed by the provider but not enforced now (Claude `is_active: false`). */
  .win.inactive .bar .used {
    opacity: 0.45;
  }
  .win.inactive .win-name {
    color: var(--text-dim);
  }
  .win.reset .bar {
    background: var(--surface-3);
  }
  @media (max-width: 640px) {
    .win {
      grid-template-columns: minmax(56px, max-content) minmax(0, 1fr);
    }
    .win-text {
      grid-column: 1 / -1;
    }
    .acct-age {
      margin-inline-start: 0;
      inline-size: 100%;
    }
  }
</style>
