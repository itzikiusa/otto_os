<script lang="ts">
  // Android `adb reverse` port-forwarding sub-pane for the IDE Devices tool window.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { SimDevice } from '../../../lib/api/types';
  import { toasts } from '../../../lib/toast.svelte';
  import type { DetectedDevServer } from './ideDevServerDetect';
  import { ideDevices } from './ideDevicesState.svelte';

  interface Props {
    device: SimDevice;
    detectedServers: readonly DetectedDevServer[];
  }

  let { device, detectedServers }: Props = $props();

  let portInput = $state<string | number | null>('');

  // Deduplicate by port so multiple URLs on the same port never collide as each keys.
  const detectedPorts = $derived([...new Set(detectedServers.map((s) => s.port))]);
  const activePorts = $derived(ideDevices.portsByDevice[device.id] ?? []);

  async function submitPortForward(): Promise<void> {
    const port = Number(String(portInput ?? '').trim());
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      toasts.error('Invalid port', 'Enter a port number between 1 and 65535.');
      return;
    }
    if (await ideDevices.addPortForward(device.id, port, port)) {
      portInput = '';
    }
  }
</script>

<div class="sim-ports-pane" data-testid="ide-sim-ports">
  <form
    class="sim-port-form"
    onsubmit={(e) => {
      e.preventDefault();
      void submitPortForward();
    }}
  >
    <input
      type="number"
      min="1"
      max="65535"
      class="input sim-port-input"
      dir="ltr"
      aria-label="Local port to forward via adb reverse"
      placeholder="Port (e.g. 5173)"
      bind:value={portInput}
    />
    <button
      type="submit"
      class="btn small"
      disabled={device.state !== 'booted' || !device.capabilities.port_forward}
    >
      <Icon name="plus" size={12} />
      Forward port
    </button>
    {#each detectedPorts as port (port)}
      <button
        type="button"
        class="btn small ghost"
        disabled={device.state !== 'booted' || !device.capabilities.port_forward}
        onclick={() => void ideDevices.addPortForward(device.id, port, port)}
      >
        + :{port}
      </button>
    {/each}
  </form>

  <div class="sim-port-list">
    {#if activePorts.length === 0}
      <div class="sim-ports-empty">
        No active <code>adb reverse</code> forwards. Forward a local dev-server port so the emulator can reach <code>127.0.0.1</code>.
      </div>
    {:else}
      {#each activePorts as pf (pf.remote_port)}
        <div class="sim-port-row">
          <span class="mono" dir="ltr">device:{pf.remote_port} → localhost:{pf.local_port}</span>
          <button
            type="button"
            class="icon-btn"
            aria-label={`Remove port forward ${pf.remote_port}`}
            title="Remove port forward"
            onclick={() => void ideDevices.removePortForward(device.id, pf.remote_port)}
          >
            <Icon name="trash" size={12} />
          </button>
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .sim-ports-pane {
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .sim-port-form {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    padding: 8px;
    border-block-end: 1px solid var(--separator);
    flex-shrink: 0;
  }
  .sim-port-input {
    inline-size: 130px;
    block-size: 26px;
    font-size: var(--fs-xs);
  }
  .sim-port-list {
    flex: 1;
    overflow-y: auto;
    padding: 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .sim-ports-empty {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    text-align: center;
    padding: 12px;
  }
  .sim-port-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-size: var(--fs-xs);
  }
</style>
