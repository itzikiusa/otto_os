<script lang="ts">
  // "Pair phone" and "Continue on phone" in one sheet.
  //
  // - Phone access off → how to turn it on (Settings → Phone access).
  // - `route` given and a phone is already paired → a QR of the deep link to
  //   exactly this session / thread on the served (tailnet) URL, plus
  //   "Pair a new phone" which pairs and then lands on the same route.
  // - Otherwise → pairing: a one-time code QR (2-minute expiry), then the
  //   phone's 6-digit comparison code with Approve / Deny. Only a person on
  //   this Mac can approve; nothing is granted until they do.
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';
  import LoadState from './LoadState.svelte';
  import { phoneApi } from '../api/phone';
  import { ApiError } from '../api/client';
  import type { PairingCreated, PairingView, PhoneAccessStatus } from '../api/types';
  import { continueUrl, secondsLeft, validNext } from '../phonePair';
  import { loadErrorText } from '../loadError';
  import { toasts } from '../toast.svelte';
  import { copyTextOrThrow } from '../clipboard';
  import { router } from '../router.svelte';
  import { now } from '../stores/now.svelte';
  import { announce } from '../announce.svelte';

  interface Props {
    /** The in-app route to continue on (`agents/<id>`, `threads/<id>`), or
     *  null to just pair a phone. */
    route?: string | null;
    /** What the route is, for the copy ("this session", "this thread"). */
    what?: string;
    onclose: () => void;
    /** Called once a phone is approved (Settings refreshes its device list). */
    onpaired?: () => void;
  }
  let { route = null, what = 'this session', onclose, onpaired }: Props = $props();

  const next = $derived(validNext(route));

  let status = $state<PhoneAccessStatus | null>(null);
  let loading = $state(true);
  let loadError = $state('');
  let deviceCount = $state(0);
  /** 'continue' shows the deep-link QR; 'pair' runs the pairing flow. */
  let mode = $state<'continue' | 'pair'>('pair');

  let pairing = $state<PairingCreated | null>(null);
  let view = $state<PairingView | null>(null);
  let creating = $state(false);
  let createError = $state('');
  let deciding = $state(false);
  let closed = false;

  let qrCanvas = $state<HTMLCanvasElement | null>(null);

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      status = await phoneApi.status();
      if (status.serving) {
        if (next) {
          try {
            deviceCount = (await phoneApi.devices()).length;
          } catch {
            deviceCount = 0;
          }
        }
        mode = next && deviceCount > 0 ? 'continue' : 'pair';
        if (mode === 'pair') void createCode();
      }
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void load();
    return () => {
      closed = true;
    };
  });

  async function createCode(): Promise<void> {
    if (creating) return;
    creating = true;
    createError = '';
    view = null;
    pairing = null;
    try {
      pairing = await phoneApi.createPairing(next);
    } catch (e) {
      createError =
        e instanceof ApiError && e.status === 409
          ? 'Phone access is off — turn it on in Settings → Phone access.'
          : loadErrorText(e);
    } finally {
      creating = false;
    }
  }

  function startPairing(): void {
    mode = 'pair';
    void createCode();
  }

  // Poll the pairing while it is open and undecided.
  $effect(() => {
    const p = pairing;
    if (!p) return;
    let stop = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const tick = async (): Promise<void> => {
      if (stop || closed) return;
      try {
        const v = await phoneApi.pairing(p.id);
        if (stop) return;
        view = v;
        if (v.status === 'approved' || v.status === 'consumed') {
          onpaired?.();
          return;
        }
        if (v.status === 'denied' || v.status === 'expired') return;
      } catch {
        /* transient: keep polling until expiry */
      }
      if (!stop) timer = setTimeout(() => void tick(), 1500);
    };
    void tick();
    return () => {
      stop = true;
      if (timer) clearTimeout(timer);
    };
  });

  const qrUrl = $derived(
    mode === 'continue' && status?.serving && next ? continueUrl(status.serving.url, next) : pairing?.url ?? null,
  );

  // Render the QR (the `qrcode` chunk loads only when a QR is shown).
  $effect(() => {
    const url = qrUrl;
    const canvas = qrCanvas;
    if (!url || !canvas) return;
    void import('qrcode').then((QR) => QR.toCanvas(canvas, url, { width: 208, margin: 1 }));
  });

  const left = $derived(secondsLeft(view?.expires_at ?? pairing?.expires_at, now()));
  const phase = $derived.by(() => {
    if (!pairing) return 'none';
    const s = view?.status ?? 'waiting';
    if ((s === 'waiting' || s === 'claimed') && left <= 0) return 'expired';
    return s === 'consumed' ? 'approved' : s;
  });

  async function decide(approve: boolean): Promise<void> {
    if (!pairing || deciding) return;
    deciding = true;
    try {
      view = approve ? await phoneApi.approve(pairing.id) : await phoneApi.deny(pairing.id);
      if (approve) {
        toasts.success('Phone paired', view.device_label ?? 'Your phone is signed in.');
        onpaired?.();
      }
    } catch (e) {
      toasts.error(approve ? 'Couldn’t approve the phone' : 'Couldn’t deny the phone', loadErrorText(e));
    } finally {
      deciding = false;
    }
  }

  async function copy(url: string): Promise<void> {
    try {
      await copyTextOrThrow(url);
      toasts.success('Link copied');
    } catch {
      toasts.error('Couldn’t copy the link', 'Select it and copy it manually.');
    }
  }

  function openSettings(): void {
    onclose();
    router.go('settings/phone-access');
  }

  // One announcement per state edge (the sheet's lines are inserted text).
  const spoken = $derived.by((): string => {
    if (status && !status.serving) return 'Phone access is off.';
    if (createError) return createError;
    if (mode === 'continue') return '';
    switch (phase) {
      case 'waiting': return 'Pairing code ready. Scan it with your phone.';
      case 'claimed': return `Pair ${view?.device_label ?? 'a phone'}? Confirmation code ${view?.confirm_code?.split('').join(' ') ?? ''}.`;
      case 'approved': return 'Phone paired.';
      case 'denied': return 'Pairing denied.';
      case 'expired': return 'The code expired.';
      default: return '';
    }
  });
  $effect(() => {
    if (spoken) announce(spoken, { assertive: !!createError });
  });

  const fmt = (s: number): string => `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
</script>

<Modal title={next ? 'Continue on phone' : 'Pair a phone'} width={440} {onclose}>
  <div class="ph">
    <LoadState what="phone access" {loading} error={loadError} onretry={() => void load()} rows={3}>
      {#if status && !status.serving}
        <div class="ph-off">
          <Icon name="phone" size={24} />
          <p class="ph-lead">Phone access is off.</p>
          <p class="ph-dim">
            {#if !status.installed}
              Install Tailscale on this Mac and your phone, sign in to the same tailnet, then turn Phone access on.
            {:else if !status.logged_in}
              Sign in to Tailscale on this Mac (and your phone), then turn Phone access on.
            {:else if status.can_manage}
              Turn it on in Settings → Phone access — Otto is then reachable only from devices in your tailnet.
            {:else}
              Ask the Otto owner (root) to turn on Phone access in Settings.
            {/if}
          </p>
          <button class="btn primary" onclick={openSettings}>Open Phone access settings</button>
        </div>
      {:else if mode === 'continue' && qrUrl}
        <p class="ph-dim">Scan with your paired phone to open {what} there. It attaches to the same live {what === 'this thread' ? 'thread' : 'terminal'}.</p>
        <canvas class="ph-qr" bind:this={qrCanvas} aria-label="QR code for {what}"></canvas>
        <div class="ph-url">
          <span class="ph-url-text mono" title={qrUrl}>{qrUrl}</span>
          <button class="btn small" onclick={() => void copy(qrUrl)} aria-label="Copy link" title="Copy link"><Icon name="copy" size={12} /></button>
        </div>
        {#if status && !status.serving?.https}
          <p class="ph-note"><Icon name="lock" size={12} /> Plain HTTP inside your tailnet — encrypted end-to-end by Tailscale.</p>
        {/if}
        <button class="btn ghost" onclick={startPairing}>Phone not paired? Pair &amp; open</button>
      {:else}
        {#if createError}
          <div class="ph-err">{createError}</div>
          <button class="btn" onclick={() => void createCode()}>Try again</button>
        {:else if creating || phase === 'none'}
          <div class="ph-dim">Creating a pairing code…</div>
        {:else if phase === 'waiting'}
          <p class="ph-dim">
            Scan with your phone’s camera{next ? ` — after you approve it opens ${what}` : ''}. The phone must be on your tailnet.
          </p>
          <canvas class="ph-qr" bind:this={qrCanvas} aria-label="Pairing QR code"></canvas>
          <p class="ph-dim">Code expires in {fmt(left)} · single use</p>
        {:else if phase === 'claimed' && view}
          <div class="ph-claim">
            <p class="ph-lead">Pair <strong>{view.device_label ?? 'a phone'}</strong>{view.peer_ip ? ` from ${view.peer_ip}` : ''}?</p>
            <div class="ph-code" aria-label="Confirmation code {view.confirm_code ?? ''}">{view.confirm_code ?? '——'}</div>
            <p class="ph-dim">Check the phone shows the same code. Approving signs that phone in as you (revocable any time).</p>
            <div class="ph-actions">
              <button class="btn danger ph-act" disabled={deciding} onclick={() => void decide(false)}>Deny</button>
              <button class="btn primary ph-act" disabled={deciding} onclick={() => void decide(true)}>{deciding ? 'Working…' : 'Approve'}</button>
            </div>
            <p class="ph-dim">Expires in {fmt(left)}</p>
          </div>
        {:else if phase === 'approved'}
          <div class="ph-off">
            <span class="ph-ok"><Icon name="check" size={24} /></span>
            <p class="ph-lead">Phone paired.</p>
            <p class="ph-dim">{next ? `It’s opening ${what} now.` : 'It’s signed in — no password needed.'}</p>
            <button class="btn primary" onclick={onclose}>Done</button>
          </div>
        {:else if phase === 'denied'}
          <div class="ph-off">
            <p class="ph-lead">Pairing denied.</p>
            <p class="ph-dim">That phone was not signed in.</p>
            <button class="btn" onclick={() => void createCode()}>New code</button>
          </div>
        {:else}
          <div class="ph-off">
            <p class="ph-lead">The code expired.</p>
            <p class="ph-dim">Codes last two minutes and work once.</p>
            <button class="btn primary" onclick={() => void createCode()}>New code</button>
          </div>
        {/if}
      {/if}
    </LoadState>
  </div>
</Modal>

<style>
  .ph {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    text-align: center;
    padding-block: 4px 8px;
  }
  .ph-off,
  .ph-claim {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
  }
  .ph-lead {
    margin: 0;
    font-size: var(--fs-l);
    color: var(--text);
  }
  .ph-dim {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    max-width: 360px;
  }
  .ph-note {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ph-err {
    color: var(--danger);
    font-size: var(--fs-s);
  }
  .ph-qr {
    width: 208px;
    height: 208px;
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .ph-url {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
  }
  .ph-url-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ph-code {
    font-family: var(--font-mono);
    font-size: var(--fs-hero);
    color: var(--text);
    padding-block: 4px;
  }
  .ph-ok {
    color: var(--success);
  }
  .ph-act {
    min-inline-size: 110px;
  }
  .ph-actions {
    display: flex;
    gap: 8px;
  }
</style>
