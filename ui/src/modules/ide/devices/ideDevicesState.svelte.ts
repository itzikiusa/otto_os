// Reactive state store for IDE Mobile Simulators & Emulators (`Devices` tool window + Run Target).
// Consumes `simulatorsApi` (`ui/src/lib/api/simulators.ts`) and shares the selected run target
// device with `IdeRunToolbar`, `IdeRunPanel`, and `ideDebugAndEnvController`.
// Built-in SDK installation state lives in the shared `ui/src/lib/simulators/simInstaller.svelte.ts`.

import { simulatorsApi } from '../../../lib/api/simulators';
import type {
  IdeMobileProject,
  SimAppearance,
  SimDevice,
  SimOrientation,
  SimPortForward,
  SimulatorsStatus,
} from '../../../lib/api/types';
import { loadErrorText } from '../../../lib/loadError';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import { setIdeMobileRunTarget } from './ideDeviceRunTarget';
import { nativeDeviceRunId } from './ideDevServerDetect';

const SELECTED_DEVICE_KEY = 'otto.ide.selectedSimDeviceId';

function readSelectedDevice(): string | null {
  try {
    return sessionStorage.getItem(SELECTED_DEVICE_KEY) || null;
  } catch {
    return null;
  }
}

function writeSelectedDevice(id: string | null): void {
  try {
    if (id) sessionStorage.setItem(SELECTED_DEVICE_KEY, id);
    else sessionStorage.removeItem(SELECTED_DEVICE_KEY);
  } catch {
    /* private browsing */
  }
}

function sortDevices(list: readonly SimDevice[]): SimDevice[] {
  const rank = (d: SimDevice): number => {
    if (d.state === 'booted') return 0;
    if (d.state === 'booting') return 1;
    if (d.state === 'shutdown') return 2;
    return 3;
  };
  return [...list].sort((a, b) => {
    const r = rank(a) - rank(b);
    if (r !== 0) return r;
    if (a.platform !== b.platform) return a.platform === 'ios' ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
}

class IdeDevicesStore {
  devices = $state<SimDevice[]>([]);
  status = $state<SimulatorsStatus | null>(null);
  loading = $state(false);
  loadedOnce = $state(false);
  error = $state<string | null>(null);
  busyById = $state<Record<string, string>>({});
  portsByDevice = $state<Record<string, SimPortForward[]>>({});
  mobileProjects = $state<IdeMobileProject[]>([]);
  #selectedDeviceId = $state<string | null>(readSelectedDevice());
  #refreshInflight: Promise<void> | null = null;
  #refreshSeq = 0;

  get selectedDeviceId(): string | null {
    return this.#selectedDeviceId;
  }

  set selectedDeviceId(next: string | null) {
    this.#selectedDeviceId = next;
    writeSelectedDevice(next);
    this.syncRunTarget();
  }

  private syncRunTarget(): void {
    const dev =
      this.devices.find((d) => d.id === this.#selectedDeviceId) ??
      this.devices.find((d) => d.state === 'booted') ??
      null;
    if (dev && dev.state === 'booted') {
      setIdeMobileRunTarget({
        device_id: nativeDeviceRunId(dev),
        device_platform: dev.platform,
      });
    } else {
      setIdeMobileRunTarget({});
    }
  }

  readonly sortedDevices = $derived(sortDevices(this.devices));

  readonly bootedDevices = $derived(this.sortedDevices.filter((d) => d.state === 'booted'));

  readonly selectedDevice = $derived.by((): SimDevice | null => {
    const id = this.#selectedDeviceId;
    if (id) {
      const hit = this.devices.find((d) => d.id === id);
      if (hit) return hit;
    }
    return this.bootedDevices[0] ?? this.sortedDevices[0] ?? null;
  });

  readonly hasMobileContext = $derived(
    this.mobileProjects.length > 0 || this.bootedDevices.length > 0,
  );

  setMobileProjects(projects: IdeMobileProject[]): void {
    this.mobileProjects = projects;
  }

  selectDevice(id: string | null): void {
    this.selectedDeviceId = id;
  }

  registerCreatedDevice(created: SimDevice): void {
    this.patchDevice(created);
    this.selectedDeviceId = created.id;
  }

  async refresh(force = false): Promise<void> {
    if (this.#refreshInflight && !force) return this.#refreshInflight;
    const seq = ++this.#refreshSeq;
    const task = (async () => {
      this.loading = true;
      this.error = null;
      try {
        const resp = await simulatorsApi.devices({ refresh: force });
        if (seq !== this.#refreshSeq) return;
        this.devices = resp.devices ?? [];
        this.status = resp.status ?? null;
        this.loadedOnce = true;
        // Keep selection valid; prefer a booted device if nothing is selected yet.
        if (this.#selectedDeviceId && !this.devices.some((d) => d.id === this.#selectedDeviceId)) {
          this.selectedDeviceId = this.bootedDevices[0]?.id ?? this.sortedDevices[0]?.id ?? null;
        } else if (!this.#selectedDeviceId && this.bootedDevices[0]) {
          this.selectedDeviceId = this.bootedDevices[0].id;
        }
        this.syncRunTarget();
      } catch (e) {
        if (seq !== this.#refreshSeq) return;
        this.error = loadErrorText(e);
      } finally {
        if (seq === this.#refreshSeq) {
          this.loading = false;
          this.#refreshInflight = null;
        }
      }
    })();
    this.#refreshInflight = task;
    return task;
  }

  async ensureLoaded(): Promise<void> {
    if (!this.loadedOnce && !this.loading) {
      await this.refresh(false);
    }
  }

  private setBusy(id: string, label: string | null): void {
    const next = { ...this.busyById };
    if (label) next[id] = label;
    else delete next[id];
    this.busyById = next;
  }

  private patchDevice(updated: SimDevice): void {
    const idx = this.devices.findIndex((d) => d.id === updated.id);
    if (idx >= 0) {
      const next = [...this.devices];
      next[idx] = updated;
      this.devices = next;
    } else {
      this.devices = [...this.devices, updated];
    }
    this.syncRunTarget();
  }

  async boot(device: SimDevice): Promise<boolean> {
    this.setBusy(device.id, 'Booting…');
    try {
      const updated = await simulatorsApi.boot(device.id, { wait: true });
      this.patchDevice(updated);
      this.selectedDeviceId = updated.id;
      toasts.success('Device ready', updated.name);
      return true;
    } catch (e) {
      toastError('Couldn’t boot device', e);
      return false;
    } finally {
      this.setBusy(device.id, null);
    }
  }

  async shutdown(device: SimDevice): Promise<boolean> {
    this.setBusy(device.id, 'Stopping…');
    try {
      const updated = await simulatorsApi.shutdown(device.id);
      this.patchDevice(updated);
      toasts.info('Device shut down', updated.name);
      return true;
    } catch (e) {
      toastError('Couldn’t shut down device', e);
      return false;
    } finally {
      this.setBusy(device.id, null);
    }
  }

  async restart(device: SimDevice): Promise<boolean> {
    this.setBusy(device.id, 'Restarting…');
    try {
      const updated = await simulatorsApi.restart(device.id, { wait: true });
      this.patchDevice(updated);
      toasts.success('Device restarted', updated.name);
      return true;
    } catch (e) {
      toastError('Couldn’t restart device', e);
      return false;
    } finally {
      this.setBusy(device.id, null);
    }
  }

  /** Explicitly open the simulator/emulator in its external macOS window. */
  async popOutNative(device: SimDevice): Promise<boolean> {
    this.setBusy(device.id, 'Opening window…');
    try {
      const updated =
        device.platform === 'ios'
          ? await simulatorsApi.boot(device.id, { wait: false, open_gui: true })
          : await simulatorsApi.restart(device.id, { wait: true, headless: false });
      this.patchDevice(updated);
      toasts.info('Opened native window', updated.name);
      return true;
    } catch (e) {
      toastError('Couldn’t open native window', e);
      return false;
    } finally {
      this.setBusy(device.id, null);
    }
  }

  async openUrl(device: SimDevice, rawUrl: string): Promise<boolean> {
    const url = rawUrl.trim();
    if (!url) return false;
    this.setBusy(device.id, 'Opening URL…');
    try {
      const res = await simulatorsApi.openUrl(device.id, {
        url,
        auto_forward_localhost: true,
      });
      const fwdNote =
        res.forwarded_ports.length > 0
          ? ` (forwarded tcp:${res.forwarded_ports.join(', ')})`
          : '';
      toasts.success(`Opened on ${device.name}`, `${res.effective_url}${fwdNote}`);
      if (res.forwarded_ports.length > 0 && device.capabilities.port_forward) {
        void this.loadPorts(device.id);
      }
      return true;
    } catch (e) {
      toastError('Couldn’t open URL on device', e);
      return false;
    } finally {
      this.setBusy(device.id, null);
    }
  }

  async setOrientation(
    device: SimDevice,
    orientation: Exclude<SimOrientation, 'unknown'>,
  ): Promise<void> {
    this.setBusy(device.id, 'Rotating…');
    try {
      await simulatorsApi.setOrientation(device.id, { orientation });
      this.patchDevice({ ...device, orientation });
    } catch (e) {
      toastError('Couldn’t rotate device', e);
    } finally {
      this.setBusy(device.id, null);
    }
  }

  async setAppearance(device: SimDevice, appearance: SimAppearance): Promise<void> {
    this.setBusy(device.id, 'Updating theme…');
    try {
      await simulatorsApi.setAppearance(device.id, { appearance });
      this.patchDevice({ ...device, appearance });
    } catch (e) {
      toastError('Couldn’t change device appearance', e);
    } finally {
      this.setBusy(device.id, null);
    }
  }

  async loadPorts(deviceId: string): Promise<void> {
    try {
      const res = await simulatorsApi.listPorts(deviceId);
      this.portsByDevice = { ...this.portsByDevice, [deviceId]: res.ports ?? [] };
    } catch {
      /* ignore when unsupported or device stopped */
    }
  }

  async addPortForward(deviceId: string, remotePort: number, localPort: number): Promise<boolean> {
    try {
      await simulatorsApi.addPortForward(deviceId, {
        remote_port: remotePort,
        local_port: localPort,
      });
      await this.loadPorts(deviceId);
      toasts.success('Port forwarded', `tcp:${remotePort} → localhost:${localPort}`);
      return true;
    } catch (e) {
      toastError('Couldn’t forward port', e);
      return false;
    }
  }

  async removePortForward(deviceId: string, remotePort: number): Promise<void> {
    try {
      await simulatorsApi.removePortForward(deviceId, remotePort);
      await this.loadPorts(deviceId);
    } catch (e) {
      toastError('Couldn’t remove port forward', e);
    }
  }
}

export const ideDevices = new IdeDevicesStore();
