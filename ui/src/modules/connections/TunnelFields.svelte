<script lang="ts">
  // The DB kinds' SSH-tunnel fields (params.ssh): host, port, user, jump
  // host(s) and identity file. Split out of ConnectionForm (over its size
  // budget); the form keeps the toggle, the frame and buildParams.
  import PathField from '../../lib/components/PathField.svelte';

  interface Props {
    host: string;
    port: string;
    user: string;
    /** `params.ssh.proxy_jump` — `ssh -J`; comma-separated hops. */
    jump: string;
    identity: string;
  }
  let {
    host = $bindable(''),
    port = $bindable(''),
    user = $bindable(''),
    jump = $bindable(''),
    identity = $bindable(''),
  }: Props = $props();
</script>

<div class="tun-row">
  <div class="field grow">
    <label for="cf-tun-host">Tunnel host</label>
    <input dir="ltr" id="cf-tun-host" class="input mono" bind:value={host} placeholder="bastion.example.com" spellcheck="false" />
    {#if !host.trim()}
      <span class="hint">Required — without a host the tunnel isn’t saved.</span>
    {/if}
  </div>
  <div class="field port">
    <label for="cf-tun-port">Port <span class="dim">(opt)</span></label>
    <input id="cf-tun-port" class="input mono" type="number" bind:value={port} placeholder="22" />
  </div>
</div>
<div class="field">
  <label for="cf-tun-user">Tunnel user <span class="dim">(optional)</span></label>
  <input dir="ltr" id="cf-tun-user" class="input mono" bind:value={user} placeholder="ec2-user" spellcheck="false" />
</div>
<div class="field">
  <label for="cf-tun-jump">Jump host <span class="dim">(optional)</span></label>
  <input dir="ltr" id="cf-tun-jump" class="input mono" bind:value={jump} placeholder="user@bastion:22" spellcheck="false" />
  <span class="hint">Reached before the tunnel host. Comma-separate several hops.</span>
</div>
<div class="field">
  <label for="cf-tun-identity">Identity file <span class="dim">(optional)</span></label>
  <PathField bind:value={identity} files>
    <input dir="ltr" id="cf-tun-identity" class="input mono" bind:value={identity} placeholder="~/.ssh/id_rsa" spellcheck="false" />
  </PathField>
  <span class="hint">Key file must be private (<span class="mono">chmod 600</span>) or ssh ignores it.</span>
</div>

<style>
  .tun-row {
    display: flex;
    gap: 12px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .port {
    flex: 0 0 90px;
  }
</style>
