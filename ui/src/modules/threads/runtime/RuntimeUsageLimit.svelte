<script lang="ts">
  // The composer line of a thread whose last turn hit its provider's usage
  // limit (usageLimit.ts): when it resets, "Resume at reset" (the daemon
  // sends a short continue at that time, once — kept on the thread, so a
  // restart or another device keeps it; Cancel clears it) and "Continue with
  // <other provider>" (arms the switch, providerSwitch.ts). With no known
  // reset time it says so and offers only the switch.
  import { api } from '../../../lib/api/client';
  import type { ResumeAtReq, RuntimeSession } from '../../../lib/api/types';
  import { agentProviders, harnessLabel, isRuntimeProvider, orderThreadHarnesses, providerReadiness } from '../../../lib/providers';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { limits } from '../../../lib/stores/usageLimits.svelte';
  import { toastError } from '../../../lib/toastError';
  import { armSwitch, switchTargets } from './providerSwitch';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { resetClock, resetFromLimits, resetFromText } from './usageLimit';

  interface Props {
    sessionId: string;
    /** The thread's runtime provider (`claude`, `codex`, `gemini`, …). */
    provider: string;
    /** The provider's own words (they may say when it resets). */
    detail: string;
    draft: { provider: string | null; model: string | null | undefined; effort: unknown };
    onfocus: () => void;
  }
  let { sessionId, provider, detail, draft, onfocus }: Props = $props();

  const fromText = $derived(resetFromText(detail, Date.now()));
  // Only when the error does not say: the shared usage-meter poller (visible-only).
  $effect(() => (fromText === null ? limits.mount() : undefined));
  const reset = $derived(fromText ?? resetFromLimits(limits.data?.accounts, provider, limits.now));
  const armedRaw = $derived((runtimeThreads.get(sessionId)?.meta as Record<string, unknown> | null | undefined)?.resume_at);
  const armed = $derived(typeof armedRaw === 'string' && Date.parse(armedRaw) > 0 ? Date.parse(armedRaw) : null);
  const other = $derived(
    switchTargets(provider, orderThreadHarnesses(agentProviders()), (p) => isRuntimeProvider(p, auth.meta) && providerReadiness(p).available)[1] ?? null, // [0] is its own
  );
  const name = $derived(harnessLabel(provider));
  const line = $derived(
    armed ? `${name} limit reached · resumes at ${resetClock(armed, Date.now())}` : `${name} limit reached · ${reset ? `resets ${resetClock(reset, Date.now())}` : 'reset time unknown'}`,
  );
  let busy = $state(false);

  async function setResume(at: number | null): Promise<void> {
    busy = true;
    try {
      const s = await api.post<RuntimeSession>(`/runtime-sessions/${encodeURIComponent(sessionId)}/resume-at`, { at: at ? new Date(at).toISOString() : null } satisfies ResumeAtReq);
      const row = runtimeThreads.byId[sessionId];
      if (row) runtimeThreads.byId[sessionId] = { ...row, meta: s.meta ?? null };
    } catch (e) {
      toastError(at ? 'Couldn’t schedule the resume' : 'Couldn’t cancel the resume', e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="ul" data-testid="runtime-usage-limit" data-armed={armed ? 'true' : undefined} role="status">
  <span class="ul-line" dir="auto">{line}</span>
  {#if armed}
    <button type="button" class="btn small ghost" disabled={busy} onclick={() => void setResume(null)}>Cancel</button>
  {:else}
    {#if reset}
      <button type="button" class="btn small" disabled={busy} data-testid="runtime-resume-at-reset" onclick={() => void setResume(reset)}>Resume at reset</button>
    {/if}
    {#if other}
      <button type="button" class="btn small ghost" data-testid="runtime-limit-switch" onclick={() => { armSwitch(draft, provider, other); onfocus(); }}>Continue with {harnessLabel(other)}</button>
    {/if}
  {/if}
</div>

<style>
  .ul {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .ul-line {
    flex: 1 1 auto;
    min-inline-size: 0;
    color: var(--warning);
  }
  @media (pointer: coarse) {
    .ul :global(.btn) {
      min-block-size: 44px;
    }
  }
</style>
