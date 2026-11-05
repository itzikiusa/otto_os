<script lang="ts">
  // The phone side of pairing (`#/pair/<code>[/<next>]`, pre-auth). The router
  // captured the one-time code and stripped it from the URL. Claim it → show
  // the 6-digit comparison code the Mac also shows → wait for the person on
  // the Mac to approve → store the device token like a normal login token and
  // land on the requested route (a session or thread), signed in.
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { announce } from '../../lib/announce.svelte';
  import { phoneApi } from '../../lib/api/phone';
  import { ApiError, setToken } from '../../lib/api/client';
  import { loadPairState, savePairState, secondsLeft, validNext, type PairState } from '../../lib/phonePair';
  import { loadErrorText } from '../../lib/loadError';
  import { now } from '../../lib/stores/now.svelte';

  type Phase = 'claiming' | 'waiting' | 'approved' | 'denied' | 'expired' | 'not-tailnet' | 'invalid' | 'throttled' | 'error';

  let pairState = $state<PairState | null>(loadPairState());
  let phase = $state<Phase>('claiming');
  let errorText = $state('');
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const left = $derived(secondsLeft(pairState?.expiresAt, now()));

  function persist(next: PairState | null): void {
    pairState = next;
    savePairState(next);
  }

  async function claim(): Promise<void> {
    const s = pairState;
    if (s?.pairingId && s.pollSecret) {
      phase = 'waiting';
      void poll();
      return;
    }
    if (!s?.code) {
      phase = 'invalid';
      return;
    }
    phase = 'claiming';
    try {
      const r = await phoneApi.claim(s.code);
      // The code is spent: keep only the poll handle.
      persist({
        code: null,
        next: s.next,
        pairingId: r.pairing_id,
        pollSecret: r.poll_secret,
        confirmCode: r.confirm_code,
        expiresAt: r.expires_at,
      });
      phase = 'waiting';
      void poll();
    } catch (e) {
      persist(null);
      if (e instanceof ApiError && e.status === 403) phase = 'not-tailnet';
      else if (e instanceof ApiError && (e.status === 404 || e.status === 410 || e.status === 409)) phase = 'invalid';
      else if (e instanceof ApiError && e.status === 429) phase = 'throttled';
      else {
        phase = 'error';
        errorText = loadErrorText(e);
      }
    }
  }

  async function poll(): Promise<void> {
    if (stopped) return;
    const s = pairState;
    if (!s?.pairingId || !s.pollSecret) return;
    try {
      const r = await phoneApi.poll(s.pairingId, s.pollSecret);
      if (stopped) return;
      if (r.status === 'approved' && r.token) {
        phase = 'approved';
        const dest = validNext(r.next ?? s.next) ?? '';
        persist(null);
        setToken(r.token);
        history.replaceState(null, '', `#/${dest}`);
        location.reload();
        return;
      }
      if (r.status === 'denied' || r.status === 'expired') {
        persist(null);
        phase = r.status;
        return;
      }
    } catch (e) {
      if (e instanceof ApiError && (e.status === 404 || e.status === 410 || e.status === 403)) {
        persist(null);
        phase = 'expired';
        return;
      }
      /* transient network blip: keep polling */
    }
    timer = setTimeout(() => void poll(), 1500);
  }

  // One announcement per phase edge (the page's lines are inserted text).
  const SPOKEN: Record<Phase, string> = {
    claiming: 'Connecting to your Mac.',
    waiting: 'Approve this phone on your Mac.',
    approved: 'Approved. Signing you in.',
    denied: 'The pairing was denied on the Mac.',
    expired: 'This pairing expired.',
    'not-tailnet': 'This phone isn’t on your tailnet.',
    throttled: 'Too many attempts.',
    invalid: 'This pairing code is invalid or was already used.',
    error: 'Couldn’t reach your Mac.',
  };
  $effect(() => announce(SPOKEN[phase], { assertive: phase !== 'claiming' && phase !== 'waiting' && phase !== 'approved' }));

  onMount(() => {
    void claim();
    return () => {
      stopped = true;
      if (timer) clearTimeout(timer);
    };
  });
</script>

<main class="pair">
  <div class="pair-card">
    <div class="pair-mark"><Icon name="phone" size={24} /></div>
    <h1 class="pair-title">Pair with Otto</h1>

    {#if phase === 'claiming'}
      <p class="pair-dim">Connecting to your Mac…</p>
    {:else if phase === 'waiting'}
      <p class="pair-dim">Approve this phone on your Mac. Check both screens show the same code:</p>
      <div class="pair-code" aria-label="Confirmation code {pairState?.confirmCode ?? ''}">{pairState?.confirmCode ?? '——'}</div>
      <p class="pair-dim">
        <span class="spinner" aria-hidden="true"></span> Waiting for approval{left > 0 ? ` · ${left}s` : ''}
      </p>
      <p class="pair-hint">Didn’t start this? Don’t approve it on the Mac — the code expires on its own.</p>
    {:else if phase === 'approved'}
      <p class="pair-dim">Approved — signing you in…</p>
    {:else if phase === 'denied'}
      <p class="pair-err">The pairing was denied on the Mac.</p>
      <p class="pair-dim">Ask for a new code on the Mac (Settings → Phone access → Pair phone) and scan it again.</p>
    {:else if phase === 'expired'}
      <p class="pair-err">This pairing expired.</p>
      <p class="pair-dim">Codes last two minutes. Create a new one on the Mac and scan it again.</p>
    {:else if phase === 'not-tailnet'}
      <p class="pair-err">This phone isn’t on your tailnet.</p>
      <p class="pair-dim">Open the Tailscale app, sign in to the same tailnet as your Mac, connect, then scan the code again.</p>
    {:else if phase === 'throttled'}
      <p class="pair-err">Too many attempts.</p>
      <p class="pair-dim">Wait a minute, then scan a fresh code from the Mac.</p>
    {:else if phase === 'invalid'}
      <p class="pair-err">This pairing code is invalid or was already used.</p>
      <p class="pair-dim">Each code works once. Create a new one on the Mac and scan it again.</p>
    {:else}
      <EmptyState tone="error" icon="warning" title="Couldn’t reach your Mac" body={errorText} actionLabel="Try again" onaction={() => void claim()} />
    {/if}
  </div>
</main>

<style>
  .pair {
    min-height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px 16px;
    background: var(--bg);
    box-sizing: border-box;
  }
  .pair-card {
    width: 100%;
    max-width: 380px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    text-align: center;
  }
  .pair-mark {
    color: var(--accent-text);
  }
  .pair-title {
    margin: 0;
    font-size: var(--fs-xl);
    font-weight: 600;
    color: var(--text);
  }
  .pair-dim {
    margin: 0;
    font-size: var(--fs-m);
    color: var(--text-dim);
    display: inline-flex;
    align-items: center;
    gap: 8px;
    justify-content: center;
    flex-wrap: wrap;
  }
  .pair-hint {
    margin: 8px 0 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .pair-err {
    margin: 0;
    font-size: var(--fs-l);
    color: var(--danger);
  }
  .pair-code {
    font-family: var(--font-mono);
    font-size: var(--fs-hero);
    color: var(--text);
    padding-block: 8px;
  }
</style>
