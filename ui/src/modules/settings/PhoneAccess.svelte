<script lang="ts">
  // Settings → Phone access. Reach Otto from your own phone over Tailscale
  // (tailnet-only, WireGuard-encrypted — never Funnel), pair it with a QR +
  // a 6-digit comparison code approved here, and manage paired devices.
  // Routes: /api/v1/phone/* (docs/contracts/api.md "Phone access").
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import SectionIntro from './SectionIntro.svelte';
  import PhoneHandoffModal from '../../lib/components/PhoneHandoffModal.svelte';
  import { sectionLabel } from './sections';
  import { phoneApi } from '../../lib/api/phone';
  import type { PhoneAccessStatus, PhoneDevice } from '../../lib/api/types';
  import { phoneDevice } from '../../lib/stores/phoneDevice.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { rel } from '../../lib/stores/now.svelte';
  import { setToken } from '../../lib/api/client';
  import { plural } from '../../lib/plural';

  let status = $state<PhoneAccessStatus | null>(null);
  let loading = $state(true);
  let loadError = $state('');
  let devices = $state<PhoneDevice[]>([]);
  let devicesError = $state('');
  let devicesLoading = $state(true);
  let switching = $state(false);
  let pairOpen = $state(false);
  let busy = $state<Set<string>>(new Set());

  /** The note reads like an HTTPS certificate failure (the DNS clash). */
  const httpsFailed = $derived(!!status?.note && /https|certificate|cert|let.?s encrypt|dns/i.test(status.note));

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    await phoneDevice.ensure();
    if (phoneDevice.isPairedPhone) {
      loading = false;
      return;
    }
    try {
      status = await phoneApi.status();
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
    void loadDevices();
  }

  async function loadDevices(): Promise<void> {
    devicesLoading = true;
    devicesError = '';
    try {
      devices = await phoneApi.devices();
    } catch (e) {
      devicesError = loadErrorText(e);
    } finally {
      devicesLoading = false;
    }
  }

  $effect(() => {
    void load();
  });

  async function setServing(enabled: boolean): Promise<void> {
    if (switching || !status) return;
    if (enabled) {
      const ok = await confirmer.ask(
        `Serve Otto to your tailnet at ${status.self_name ?? 'this Mac’s Tailscale name'}? Only devices signed in to your tailnet${status.tailnet ? ` (${status.tailnet})` : ''} can reach it, and each still needs a paired device or your password. Otto never uses Tailscale Funnel (public internet).`,
        { title: 'Turn on Phone access', confirmLabel: 'Turn on' },
      );
      if (!ok) return;
    } else {
      const ok = await confirmer.ask(
        'Stop serving Otto to your tailnet? Paired phones stay paired but can’t reach Otto until you turn it back on. Other Tailscale serve entries on this Mac are left alone.',
        { title: 'Turn off Phone access', confirmLabel: 'Turn off', danger: true },
      );
      if (!ok) return;
    }
    switching = true;
    try {
      status = await phoneApi.serve(enabled);
      toasts.success(enabled ? 'Phone access on' : 'Phone access off', enabled ? status.serving?.url : undefined);
    } catch (e) {
      toasts.error(enabled ? 'Couldn’t turn on Phone access' : 'Couldn’t turn off Phone access', loadErrorText(e));
    } finally {
      switching = false;
    }
  }

  function mark(id: string, on: boolean): void {
    const next = new Set(busy);
    if (on) next.add(id);
    else next.delete(id);
    busy = next;
  }

  async function rename(d: PhoneDevice): Promise<void> {
    const name = await confirmer.promptText('Name this phone', {
      title: 'Rename phone',
      confirmLabel: 'Rename',
      initial: d.name,
      placeholder: 'My iPhone',
    });
    const trimmed = name?.trim();
    if (!trimmed || trimmed === d.name) return;
    mark(d.id, true);
    try {
      const upd = await phoneApi.rename(d.id, trimmed.slice(0, 80));
      devices = devices.map((x) => (x.id === d.id ? upd : x));
    } catch (e) {
      toasts.error('Couldn’t rename the phone', loadErrorText(e));
    } finally {
      mark(d.id, false);
    }
  }

  async function revoke(d: PhoneDevice): Promise<void> {
    const ok = await confirmer.ask(
      `Revoke “${d.name}”? It is signed out at once — open terminals and threads on it disconnect — and it must be paired again to come back.`,
      { title: 'Revoke phone', confirmLabel: 'Revoke', danger: true },
    );
    if (!ok) return;
    mark(d.id, true);
    try {
      await phoneApi.revoke(d.id);
      devices = devices.filter((x) => x.id !== d.id);
      toasts.success('Phone revoked', d.name);
    } catch (e) {
      toasts.error('Couldn’t revoke the phone', loadErrorText(e));
    } finally {
      mark(d.id, false);
    }
  }

  async function revokeAll(): Promise<void> {
    const ok = await confirmer.ask(
      `Revoke all ${plural(devices.length, 'paired phone')}? Each is signed out at once and must be paired again.`,
      { title: 'Revoke all phones', confirmLabel: 'Revoke all', danger: true },
    );
    if (!ok) return;
    try {
      const r = await phoneApi.revokeAll();
      devices = [];
      toasts.success('Phones revoked', plural(r.revoked, 'phone'));
    } catch (e) {
      toasts.error('Couldn’t revoke the phones', loadErrorText(e));
    }
  }

  async function unpairSelf(): Promise<void> {
    const ok = await confirmer.ask('Unpair this phone? You are signed out here and must pair it again from your Mac to come back.', {
      title: 'Unpair this phone',
      confirmLabel: 'Unpair',
      danger: true,
    });
    if (!ok) return;
    try {
      await phoneApi.unpairSelf();
    } catch (e) {
      toasts.error('Couldn’t unpair this phone', loadErrorText(e));
      return;
    }
    setToken(null);
    location.reload();
  }

  const abs = (ts: string | null | undefined): string | undefined => (ts ? new Date(ts).toLocaleString() : undefined);
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('phone-access')} subtitle="Use Otto from your own phone over Tailscale">
    {#snippet actions()}
      {#if status?.serving && !phoneDevice.isPairedPhone}
        <button class="btn primary" onclick={() => (pairOpen = true)}><Icon name="phone" size={13} /> Pair phone</button>
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody width="readable">
    {#if phoneDevice.isPairedPhone && phoneDevice.device}
      <div class="card s-card">
        <EmptyState icon="phone" variant="panel" title="You’re on a paired phone"
          body={`Signed in as “${phoneDevice.device.name}”. Pairing, approving phones and turning Phone access on or off are done on the Mac.`} />
        <div class="row-end">
          <button class="btn danger" onclick={() => void unpairSelf()}>Unpair this phone</button>
        </div>
      </div>
    {:else}
      <SectionIntro>
        Otto stays on <code>127.0.0.1</code>. Tailscale serves it only to devices in <strong>your tailnet</strong>
        (WireGuard-encrypted, never the public internet). A phone is paired once with a QR code you approve here,
        then signs in without a password. You can revoke it any time.
      </SectionIntro>

      <LoadState what="phone access" {loading} error={loadError} onretry={() => void load()} rows={3}>
        {#if status && !status.installed}
          <div class="card s-card">
            <EmptyState icon="phone" variant="panel" title="Tailscale isn’t installed">
              <ol class="steps">
                <li>Install Tailscale on this Mac: <code>brew install --cask tailscale</code> or the Mac App Store.</li>
                <li>Install Tailscale on your phone (App Store / Play Store).</li>
                <li>Sign in to the <strong>same</strong> tailnet on both.</li>
                <li>Come back here and turn Phone access on.</li>
              </ol>
              <button class="btn" onclick={() => void load()}><Icon name="refresh" size={12} /> Check again</button>
            </EmptyState>
          </div>
        {:else if status}
          <div class="section-title">Tailscale</div>
          <div class="card s-card kv">
            <div class="kv-row"><span class="k">Status</span><span class="v">
              {#if status.logged_in}<Badge tone="ok" label="Signed in" />{:else}<Badge tone="bad" label="Not signed in" />{/if}
              {#if status.backend_state}<span class="dim"> · {status.backend_state}</span>{/if}
            </span></div>
            {#if status.tailnet}<div class="kv-row"><span class="k">Tailnet</span><span class="v">{status.tailnet}</span></div>{/if}
            {#if status.self_name}<div class="kv-row"><span class="k">This Mac</span><span class="v mono" dir="ltr">{status.self_name}</span></div>{/if}
            {#if status.self_ips.length}<div class="kv-row"><span class="k">Tailnet IP</span><span class="v mono" dir="ltr">{status.self_ips.join(', ')}</span></div>{/if}
            {#if !status.magic_dns}<div class="kv-row"><span class="k">MagicDNS</span><span class="v warn">Off — turn it on in the Tailscale admin console</span></div>{/if}
            {#if status.error}<div class="pa-err">{status.error}</div>{/if}
          </div>

          <div class="section-title">Serving</div>
          <div class="card s-card">
            {#if status.serving}
              <div class="serve-row">
                <Badge tone="ok" label="On" />
                <Badge tone={status.serving.https ? 'ok' : 'info'} label={status.serving.https ? 'HTTPS' : 'HTTP over Tailscale'} />
                <a class="mono url" dir="ltr" href={status.serving.url} target="_blank" rel="noreferrer">{status.serving.url}</a>
              </div>
            {:else}
              <div class="serve-row"><Badge label="Off" /><span class="dim">Otto isn’t reachable from your phone.</span></div>
            {/if}
            {#if status.note}<p class="note"><Icon name="info" size={12} /> {status.note}</p>{/if}
            {#if httpsFailed}
              <p class="note">
                No padlock: HTTPS needs a Let’s Encrypt certificate, which tailscaled fetches through DNS. If another VPN
                puts its DNS server inside Tailscale’s <code>100.64.0.0/10</code> range, that lookup fails. Traffic is still
                end-to-end encrypted by WireGuard; only passkeys (which need HTTPS) are unavailable.
              </p>
            {/if}
            {#if status.other_serves}<p class="note"><Icon name="warning" size={12} /> Other Tailscale serve entries exist on this Mac — Otto leaves them alone.</p>{/if}
            {#if status.can_manage}
              <div class="row-end">
                {#if status.serving}
                  <button class="btn danger" disabled={switching} onclick={() => void setServing(false)}>{switching ? 'Turning off…' : 'Turn off'}</button>
                {:else}
                  <button class="btn" disabled={switching || !status.logged_in} onclick={() => void setServing(true)}>{switching ? 'Turning on…' : 'Turn on'}</button>
                {/if}
              </div>
            {:else if !status.serving}
              <p class="note">Only the Otto owner (root) can turn Phone access on.</p>
            {/if}
          </div>

          {#if status.devices.length}
            <div class="section-title">Devices in your tailnet</div>
            <div class="card s-card list">
              {#each status.devices as d (d.dns_name ?? d.name)}
                <div class="dev">
                  <span class="dot" class:on={d.online} aria-hidden="true"></span>
                  <span class="dev-name">{d.name}</span>
                  <span class="dim">{d.os ?? ''}{d.ips[0] ? ` · ${d.ips[0]}` : ''}</span>
                  <span class="sr-only">{d.online ? 'online' : 'offline'}</span>
                </div>
              {/each}
            </div>
          {/if}
        {/if}
      </LoadState>

      <div class="head-row">
        <div class="section-title">Paired phones</div>
        {#if devices.length > 1}
          <button class="btn small danger" onclick={() => void revokeAll()}>Revoke all…</button>
        {/if}
      </div>
      <LoadState what="paired phones" loading={devicesLoading} error={devicesError} empty={devices.length === 0}
        onretry={() => void loadDevices()} rows={2}>
        {#snippet emptyView()}
          <div class="card s-card">
            <EmptyState icon="phone" variant="panel" title="No paired phones"
              body={status?.serving ? 'Use Pair phone to sign a phone in with a QR code.' : 'Turn on Phone access, then pair a phone.'} />
          </div>
        {/snippet}
        <div class="card s-card list" role="list" aria-label="Paired phones">
          {#each devices as d (d.id)}
            <div class="pd" role="listitem">
              <div class="pd-main">
                <span class="dev-name">{d.name}{#if d.current}<Badge tone="accent" label="This device" />{/if}</span>
                <span class="dim" title={abs(d.last_seen_at)}>Last seen {rel(d.last_seen_at)}{d.last_ip ? ` · ${d.last_ip}` : ''}</span>
                <span class="dim">Paired <span title={abs(d.created_at)}>{rel(d.created_at)}</span> · signs out if idle <span title={abs(d.idle_expires_at)}>{rel(d.idle_expires_at)}</span> · expires <span title={abs(d.expires_at)}>{rel(d.expires_at)}</span></span>
              </div>
              <div class="pd-actions">
                <button class="btn small" disabled={busy.has(d.id)} onclick={() => void rename(d)} aria-label="Rename {d.name}" title="Rename"><Icon name="edit" size={12} /></button>
                <button class="btn small danger" disabled={busy.has(d.id)} onclick={() => void revoke(d)}>Revoke…</button>
              </div>
            </div>
          {/each}
        </div>
      </LoadState>
    {/if}
  </PageBody>
</div>

{#if pairOpen}
  <PhoneHandoffModal onclose={() => (pairOpen = false)} onpaired={() => void loadDevices()} />
{/if}

<style>
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .s-card {
    padding: 12px 16px;
    max-width: var(--settings-col);
  }
  .kv,
  .list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .kv-row {
    display: flex;
    gap: 12px;
    font-size: var(--fs-m);
    flex-wrap: wrap;
  }
  .k {
    min-width: 96px;
    color: var(--text-dim);
  }
  .v {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .warn {
    color: var(--warning);
  }
  .pa-err {
    color: var(--danger);
    font-size: var(--fs-s);
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .serve-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .url {
    min-width: 0;
    overflow-wrap: anywhere;
    font-size: var(--fs-s);
  }
  .note {
    display: flex;
    gap: 6px;
    align-items: baseline;
    margin: 8px 0 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .row-end {
    display: flex;
    justify-content: flex-end;
    margin-top: 10px;
  }
  .steps {
    text-align: start;
    margin: 0 0 10px;
    padding-inline-start: 20px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .dev {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-m);
    flex-wrap: wrap;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-dim);
  }
  .dot.on {
    background: var(--success);
  }
  .dev-name {
    display: inline-flex;
    gap: 6px;
    align-items: center;
    font-weight: 500;
  }
  .head-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    max-width: var(--settings-col);
  }
  .pd {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-block: 6px;
  }
  .pd + .pd {
    border-top: 1px solid var(--border);
  }
  .pd-main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .pd-actions {
    display: flex;
    gap: 6px;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  code {
    font-family: var(--font-mono);
  }
</style>
