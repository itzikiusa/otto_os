<script lang="ts">
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import { brokers } from '../../lib/stores/brokers.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type {
    BrokerCluster,
    Environment,
    SaslMechanism,
    SecurityProtocol,
    SshTunnelConfig,
    UpsertClusterReq,
  } from '../../lib/api/types';

  interface Props {
    cluster?: BrokerCluster | null;
    onclose: () => void;
  }
  let { cluster = null, onclose }: Props = $props();

  // The modal is remounted per open, so a one-time snapshot of the prop seeds
  // the form (untrack keeps these out of the reactive graph).
  const init = untrack(() => cluster);
  const editing = init !== null;

  let name = $state(init?.name ?? '');
  let bootstrap = $state(init?.bootstrap_servers ?? '');
  let security = $state<SecurityProtocol>(init?.security_protocol ?? 'plaintext');
  let mechanism = $state<SaslMechanism>(init?.sasl_mechanism ?? 'plain');
  let saslUser = $state(init?.sasl_username ?? '');
  let saslPass = $state('');
  let tlsSkip = $state(init?.tls_skip_verify ?? false);
  let srUrl = $state(init?.schema_registry_url ?? '');
  let srUser = $state(init?.schema_registry_username ?? '');
  let srPass = $state('');
  let metricsUrl = $state(init?.metrics_url ?? '');
  let environment = $state<Environment>(init?.environment ?? 'dev');
  let readOnly = $state(init?.read_only ?? false);
  let color = $state(init?.color ?? '');
  let saving = $state(false);

  // SSH tunnel (bastion) — for private clusters like AWS MSK in a VPC.
  let tunnelOpen = $state(!!init?.ssh);
  let tunHost = $state(init?.ssh?.host ?? '');
  let tunPort = $state(init?.ssh?.port != null ? String(init.ssh.port) : '');
  let tunUser = $state(init?.ssh?.user ?? '');
  let tunIdentity = $state(init?.ssh?.identity_file ?? '');
  let showTunnelFilePicker = $state(false);

  const usesSasl = $derived(security === 'sasl_plaintext' || security === 'sasl_ssl');
  const usesTls = $derived(security === 'ssl' || security === 'sasl_ssl');

  async function save() {
    if (!name.trim() || !bootstrap.trim()) {
      toasts.error('Name and bootstrap servers are required');
      return;
    }
    // ssh: omit on create (keep) when off; send null on edit to clear.
    let ssh: SshTunnelConfig | null | undefined;
    if (tunnelOpen && tunHost.trim() && tunUser.trim()) {
      ssh = {
        host: tunHost.trim(),
        port: tunPort.trim() ? Number(tunPort) : undefined,
        user: tunUser.trim(),
        identity_file: tunIdentity.trim() || null,
      };
    } else if (editing) {
      ssh = null;
    } else {
      ssh = undefined;
    }
    if (tunnelOpen && (!tunHost.trim() || !tunUser.trim())) {
      toasts.error('SSH tunnel needs a host and user');
      return;
    }
    const req: UpsertClusterReq = {
      name: name.trim(),
      bootstrap_servers: bootstrap.trim(),
      security_protocol: security,
      sasl_mechanism: usesSasl ? mechanism : null,
      sasl_username: usesSasl ? saslUser.trim() || null : null,
      sasl_password: saslPass ? saslPass : undefined,
      tls_skip_verify: tlsSkip,
      schema_registry_url: srUrl.trim() || null,
      schema_registry_username: srUser.trim() || null,
      schema_registry_password: srPass ? srPass : undefined,
      metrics_url: metricsUrl.trim() || null,
      color: color.trim() || null,
      ssh,
      environment,
      read_only: readOnly,
    };
    saving = true;
    try {
      if (editing && cluster) await brokers.update(cluster.id, req);
      else await brokers.create(req);
      toasts.success(editing ? 'Cluster updated' : 'Cluster added');
      onclose();
    } catch (e) {
      toasts.error('Save failed', String(e));
    } finally {
      saving = false;
    }
  }
</script>

<Modal title={editing ? 'Edit cluster' : 'Add Kafka cluster'} width={540} {onclose}>
  <div class="form">
    <label class="field">
      <span>Name</span>
      <input bind:value={name} placeholder="prod-kafka" />
    </label>
    <label class="field">
      <span>Bootstrap servers</span>
      <input bind:value={bootstrap} placeholder="broker1:9092,broker2:9092" />
    </label>

    <div class="row">
      <label class="field">
        <span>Security</span>
        <select bind:value={security}>
          <option value="plaintext">PLAINTEXT</option>
          <option value="ssl">SSL</option>
          <option value="sasl_plaintext">SASL_PLAINTEXT</option>
          <option value="sasl_ssl">SASL_SSL</option>
        </select>
      </label>
      {#if usesSasl}
        <label class="field">
          <span>SASL mechanism</span>
          <select bind:value={mechanism}>
            <option value="plain">PLAIN</option>
            <option value="scram_sha_256">SCRAM-SHA-256</option>
            <option value="scram_sha_512">SCRAM-SHA-512</option>
          </select>
        </label>
      {/if}
    </div>

    {#if usesSasl}
      <div class="row">
        <label class="field">
          <span>SASL username</span>
          <input bind:value={saslUser} autocomplete="off" />
        </label>
        <label class="field">
          <span>SASL password</span>
          <input
            type="password"
            bind:value={saslPass}
            autocomplete="new-password"
            placeholder={cluster?.has_sasl_password ? '•••••• (unchanged)' : ''}
          />
        </label>
      </div>
    {/if}
    {#if usesTls}
      <label class="check">
        <input type="checkbox" bind:checked={tlsSkip} />
        <span>Skip TLS certificate verification (self-signed brokers)</span>
      </label>
    {/if}

    <label class="field">
      <span>Schema registry URL <em>(optional — enables Avro decode)</em></span>
      <input bind:value={srUrl} placeholder="http://schema-registry:8081" />
    </label>
    {#if srUrl.trim()}
      <div class="row">
        <label class="field">
          <span>Registry username</span>
          <input bind:value={srUser} autocomplete="off" />
        </label>
        <label class="field">
          <span>Registry password</span>
          <input
            type="password"
            bind:value={srPass}
            autocomplete="new-password"
            placeholder={cluster?.has_sr_password ? '•••••• (unchanged)' : ''}
          />
        </label>
      </div>
    {/if}

    <label class="field">
      <span>Metrics URL <em>(optional — Prometheus, e.g. Redpanda :9644/public_metrics)</em></span>
      <input bind:value={metricsUrl} placeholder="http://broker:9644/public_metrics" />
    </label>

    <label class="check">
      <input type="checkbox" bind:checked={tunnelOpen} />
      <span>SSH tunnel <em>(reach a private cluster — e.g. AWS MSK — through a bastion)</em></span>
    </label>
    {#if tunnelOpen}
      <div class="ssh-section">
        <div class="row">
          <label class="field" style="flex: 1;">
            <span>Tunnel host</span>
            <input bind:value={tunHost} placeholder="bastion.example.com" spellcheck="false" />
          </label>
          <label class="field" style="flex: 0 0 90px;">
            <span>Port</span>
            <input type="number" bind:value={tunPort} placeholder="22" />
          </label>
        </div>
        <label class="field">
          <span>Tunnel user</span>
          <input bind:value={tunUser} placeholder="ec2-user" spellcheck="false" />
        </label>
        <label class="field">
          <span>Identity file <em>(optional — defaults to ssh-agent)</em></span>
          <div class="file-input-row">
            <input bind:value={tunIdentity} placeholder="~/.ssh/id_rsa" spellcheck="false" />
            <button class="btn browse-btn" onclick={() => (showTunnelFilePicker = true)}>Browse…</button>
          </div>
        </label>
      </div>
    {/if}

    <div class="row">
      <label class="field">
        <span>Environment</span>
        <select bind:value={environment}>
          <option value="dev">Dev</option>
          <option value="staging">Staging</option>
          <option value="prod">Production</option>
        </select>
      </label>
      <label class="field">
        <span>Accent color</span>
        <input bind:value={color} placeholder="#0a84ff" />
      </label>
    </div>
    <label class="check">
      <input type="checkbox" bind:checked={readOnly} />
      <span>Read-only (block produce / delete / config edits without confirm)</span>
    </label>
  </div>

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={save} disabled={saving}>
      {saving ? 'Saving…' : editing ? 'Save' : 'Add cluster'}
    </button>
  {/snippet}
</Modal>

{#if showTunnelFilePicker}
  <FolderPicker
    title="Choose Identity File"
    start={tunIdentity ? tunIdentity.replace(/\/[^/]+$/, '') : ''}
    files={true}
    onpick={(path) => { tunIdentity = path; showTunnelFilePicker = false; }}
    onclose={() => (showTunnelFilePicker = false)}
  />
{/if}

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .row {
    display: flex;
    gap: 12px;
  }
  .row .field {
    flex: 1;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field span {
    font-size: 12px;
    color: var(--text-dim);
  }
  .field span em {
    font-style: normal;
    opacity: 0.7;
  }
  .field input,
  .field select {
    padding: 7px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 13px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .check span em {
    font-style: normal;
    opacity: 0.7;
  }
  .ssh-section {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-top: 2px;
    padding: 12px;
    border-radius: var(--radius-m, 8px);
    border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
    background: color-mix(in srgb, var(--accent) 5%, transparent);
  }
  .file-input-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .file-input-row input {
    flex: 1;
    min-width: 0;
  }
  .browse-btn {
    flex-shrink: 0;
    white-space: nowrap;
  }
</style>
