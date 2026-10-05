<script lang="ts">
  import { plural } from '../../lib/plural';
  // Settings → Trust & safety → "Secret storage" (root only).
  // Shows which store holds integration secrets (GET /admin/secrets/status)
  // and, while they sit in the PLAINTEXT secrets.json, a warning banner plus
  // the explicit "Secure secrets…" action (POST /admin/secrets/secure): every
  // secret is copied into secrets.enc (sealed with one master key kept in the
  // macOS Keychain), verified to read back, and only then is the plaintext
  // file wiped and deleted. Never automatic — the Keychain may prompt.
  import { onMount } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { SecretsStatus, SecretsMigrationReport } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';

  let status = $state<SecretsStatus | null>(null);
  let loading = $state(true);
  let loadError = $state('');
  let securing = $state(false);
  let actionError = $state('');
  const msg = (e: unknown): string => (e instanceof Error ? e.message : String(e));

  async function load() {
    loading = true;
    loadError = '';
    try {
      status = await api.get<SecretsStatus>('/admin/secrets/status');
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  onMount(load);

  const MODE_LABEL: Record<SecretsStatus['mode'], string> = {
    plaintext: 'Plaintext file (not encrypted)',
    encrypted: 'Encrypted file, key in the macOS Keychain',
    keychain: 'macOS Keychain (one item per secret)',
  };
  const KEY_LABEL: Record<SecretsStatus['key_state'], string> = {
    unlocked: 'Unlocked',
    locked: 'Waiting for the Keychain — unlock it or approve the prompt',
    not_loaded: 'Not loaded yet',
    error: 'Keychain refused access',
  };

  async function secure() {
    if (!status) return;
    const n = status.plaintext_entries;
    const ok = await confirmer.ask(
      `Encrypt ${plural(n, 'stored secret')} (connection passwords, Slack/Telegram tokens, accounts)? ` +
        'Otto creates one encryption key in your macOS Keychain — macOS may ask you to allow access; choose “Always Allow”. ' +
        'Every secret is checked to read back from the encrypted store before the plaintext file is wiped and deleted. ' +
        'If anything fails, nothing changes. Integrations keep working; no restart needed.',
      { title: 'Secure secrets', confirmLabel: 'Encrypt secrets' },
    );
    if (!ok) return;
    securing = true;
    actionError = '';
    try {
      const r = await api.post<SecretsMigrationReport>('/admin/secrets/secure', { confirm: true });
      toasts.success('Secrets encrypted', `${plural(r.migrated, 'secret')} moved; the plaintext file was deleted.`);
      await load();
    } catch (e) {
      // A failure AFTER the switch (final live check) leaves the daemon on the
      // encrypted store with the plaintext already removed — "not changed"
      // would be false there. Re-read the status to tell the two apart.
      await load();
      const after = status as SecretsStatus | null;
      actionError =
        after && after.mode !== 'plaintext'
          ? `Secrets were encrypted and the plaintext file was removed, but a final check failed.` +
            (after.backup_present ? ' The encrypted backup secrets.migrate-backup.enc was kept.' : '') +
            ` ${msg(e)}`
          : `Secrets were not changed. ${msg(e)}`;
    } finally {
      securing = false;
    }
  }
</script>

<section class="secrets-card" aria-label="Secret storage">
  <h2 class="card-title"><Icon name="key" size={14} /> Secret storage</h2>
  {#if !status}
    <LoadState what="the secret store status" {loading} error={loadError} empty={true} onretry={load} />
  {:else}
    {#if status.mode === 'plaintext'}
      <div class="banner" role="alert">
        <Icon name="warning" size={14} />
        {#if status.plaintext_entries > 0}
          <p>
            <strong>Secrets are stored unencrypted.</strong>
            {status.plaintext_entries} secret{status.plaintext_entries === 1 ? ' is' : 's are'} in a plaintext file anyone
            with access to your user account can read. Encrypt them with a key kept in the macOS Keychain.
          </p>
        {:else}
          <p>
            <strong>The plaintext secret store is active.</strong>
            New secrets will be saved unencrypted (the daemon runs with <code>OTTO_SECRETS=file</code>).
          </p>
        {/if}
      </div>
    {/if}
    <dl class="facts">
      <div><dt>Store</dt><dd>{MODE_LABEL[status.mode]}</dd></div>
      {#if status.mode === 'encrypted'}
        <div><dt>Encryption key</dt><dd class:warn={status.key_state === 'locked' || status.key_state === 'error'}>{KEY_LABEL[status.key_state]}</dd></div>
      {/if}
    </dl>
    {#if status.backup_present}
      <p class="dim">An encrypted backup from an earlier, unfinished migration is still on disk.</p>
    {/if}
    {#if status.migration_available}
      <div class="controls">
        <button class="btn primary" disabled={securing || status.migrating} onclick={secure}>
          <Icon name="lock" size={13} />
          {securing || status.migrating ? 'Encrypting…' : 'Secure secrets…'}
        </button>
      </div>
    {/if}
    {#if actionError}
      <p class="error" role="alert">{actionError}</p>
    {/if}
  {/if}
</section>

<style>
  .secrets-card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-m); box-shadow: var(--shadow-card); padding: 16px 18px; margin: 16px; max-width: var(--settings-col); }
  .card-title { display: flex; align-items: center; gap: 6px; margin: 0 0 8px; font-size: var(--fs-m); font-weight: 600; }
  p { margin: 0 0 8px; font-size: var(--fs-s); line-height: 1.5; }
  .banner { display: flex; gap: 8px; align-items: flex-start; padding: 10px 12px; margin: 0 0 10px; border-radius: var(--radius-s); background: var(--warning-soft); color: var(--text); border: 1px solid var(--warning); }
  .banner :global(svg) { flex: none; color: var(--warning); margin-block-start: 2px; }
  .banner p { margin: 0; }
  .dim { color: var(--text-dim); }
  .banner code { font-family: var(--font-mono); font-size: var(--fs-xs); }
  .facts { display: flex; flex-wrap: wrap; gap: 8px 24px; margin: 6px 0 0; font-size: var(--fs-s); }
  .facts dt { color: var(--text-dim); }
  .facts dd { margin: 2px 0 0; font-weight: 500; }
  .facts dd.warn { color: var(--warning); }
  .controls { display: flex; flex-wrap: wrap; gap: 8px; margin: 12px 0 0; }
  .error { margin: 10px 0 0; color: var(--danger); overflow-wrap: anywhere; }
</style>
