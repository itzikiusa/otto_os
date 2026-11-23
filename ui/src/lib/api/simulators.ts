// Mobile Simulators & Emulators API client (iOS Simulator + Android Emulator).
// Mirrors docs/contracts/api.md "Mobile Simulators" and docs/contracts/ws.md.

import { api, authedBlobUrl, wsConnect } from './client';
import { decodeBinaryFrame } from './binaryFrame';
import type {
  BootSimDeviceReq,
  CreateSimDeviceReq,
  CreateSimPortForwardReq,
  DecodedSimFrame,
  OpenSimUrlReq,
  OpenSimUrlResp,
  RestartSimDeviceReq,
  SimActionOkResp,
  SimAppearanceReq,
  SimCatalogResp,
  SimClipboardResp,
  SimDevice,
  SimDevicesResp,
  SimDeviceState,
  SimFrameHeader,
  SimInputReq,
  SimInstallAppReq,
  SimInstallReq,
  SimInstallStatus,
  SimLaunchAppReq,
  SimOrientationReq,
  SimPlatform,
  SimPortForward,
  SimPortsResp,
  SimScreenshotFormat,
  SimScreenshotJsonResp,
  SimStatusBarReq,
  SimTerminateAppReq,
  SimulatorsStatus,
} from './types';

function encId(id: string): string {
  return encodeURIComponent(id);
}

function buildQuery(params: Record<string, string | number | boolean | undefined>): string {
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== '') qs.set(k, String(v));
  }
  const s = qs.toString();
  return s ? `?${s}` : '';
}

export const simulatorsApi = {
  /** Dynamic host readiness/diagnostics for iOS Simulator and Android Emulator. */
  status(refresh = false): Promise<SimulatorsStatus> {
    return api.get<SimulatorsStatus>(`/simulators/status${buildQuery({ refresh: refresh || undefined })}`);
  },

  /** List all discovered iOS simulators, Android AVDs, and connected devices + readiness. */
  devices(opts?: {
    platform?: SimPlatform;
    state?: SimDeviceState;
    refresh?: boolean;
  }): Promise<SimDevicesResp> {
    return api.get<SimDevicesResp>(
      `/simulators/devices${buildQuery({
        platform: opts?.platform,
        state: opts?.state,
        refresh: opts?.refresh || undefined,
      })}`,
    );
  },

  /** List installable/creatable runtimes, device types, Android system images, and profiles. */
  catalog(platform?: SimPlatform): Promise<SimCatalogResp> {
    return api.get<SimCatalogResp>(`/simulators/catalog${buildQuery({ platform })}`);
  },

  /** Fetch one device by unified ID (`ios:<udid>`, `android:avd:<name>`, `android:serial:<serial>`). */
  getDevice(id: string): Promise<SimDevice> {
    return api.get<SimDevice>(`/simulators/devices/${encId(id)}`);
  },

  /** Create a new iOS Simulator or Android AVD. */
  createDevice(body: CreateSimDeviceReq): Promise<SimDevice> {
    return api.long.post<SimDevice>('/simulators/devices', body);
  },

  /** Delete an iOS Simulator or Android AVD. */
  deleteDevice(id: string): Promise<void> {
    return api.del<void>(`/simulators/devices/${encId(id)}`);
  },

  /** Boot a simulator/emulator (optionally waiting for boot completion). */
  boot(id: string, body: BootSimDeviceReq = {}): Promise<SimDevice> {
    return api.long.post<SimDevice>(`/simulators/devices/${encId(id)}/boot`, body);
  },

  /** Shut down a booted simulator/emulator. */
  shutdown(id: string): Promise<SimDevice> {
    return api.long.post<SimDevice>(`/simulators/devices/${encId(id)}/shutdown`, {});
  },

  /** Restart a simulator/emulator. */
  restart(id: string, body: RestartSimDeviceReq = {}): Promise<SimDevice> {
    return api.long.post<SimDevice>(`/simulators/devices/${encId(id)}/restart`, body);
  },

  /** Open a URL / deep link on the device (with automatic `adb reverse` for localhost on Android). */
  openUrl(id: string, body: OpenSimUrlReq): Promise<OpenSimUrlResp> {
    return api.post<OpenSimUrlResp>(`/simulators/devices/${encId(id)}/open-url`, body);
  },

  /** Fetch a screenshot as a JSON base64 payload. */
  screenshotJson(id: string, format: SimScreenshotFormat = 'png'): Promise<SimScreenshotJsonResp> {
    return api.get<SimScreenshotJsonResp>(
      `/simulators/devices/${encId(id)}/screenshot${buildQuery({ format, encoding: 'base64' })}`,
    );
  },

  /** Fetch a screenshot as an object URL (caller must revoke with `URL.revokeObjectURL`). */
  screenshotBlobUrl(id: string, format: SimScreenshotFormat = 'png'): Promise<string> {
    return authedBlobUrl(
      `/simulators/devices/${encId(id)}/screenshot${buildQuery({ format, encoding: 'binary' })}`,
    );
  },

  /** Send touch, swipe, text, or key event input to the device. */
  input(id: string, body: SimInputReq): Promise<SimActionOkResp> {
    return api.post<SimActionOkResp>(`/simulators/devices/${encId(id)}/input`, body);
  },

  /** Set device orientation (portrait/landscape). */
  setOrientation(id: string, body: SimOrientationReq): Promise<SimActionOkResp> {
    return api.post<SimActionOkResp>(`/simulators/devices/${encId(id)}/orientation`, body);
  },

  /** Set device appearance (light/dark). */
  setAppearance(id: string, body: SimAppearanceReq): Promise<SimActionOkResp> {
    return api.post<SimActionOkResp>(`/simulators/devices/${encId(id)}/appearance`, body);
  },

  /** Override or clear the iOS simulator status bar. */
  setStatusBar(id: string, body: SimStatusBarReq): Promise<SimActionOkResp> {
    return api.post<SimActionOkResp>(`/simulators/devices/${encId(id)}/status-bar`, body);
  },

  /** Read device clipboard text. */
  getClipboard(id: string): Promise<SimClipboardResp> {
    return api.get<SimClipboardResp>(`/simulators/devices/${encId(id)}/clipboard`);
  },

  /** Write device clipboard text. */
  setClipboard(id: string, text: string): Promise<SimActionOkResp> {
    return api.post<SimActionOkResp>(`/simulators/devices/${encId(id)}/clipboard`, { text });
  },

  /** Install an app bundle (`.app` on iOS, `.apk` on Android) from a host path. */
  installApp(id: string, body: SimInstallAppReq): Promise<SimActionOkResp> {
    return api.long.post<SimActionOkResp>(`/simulators/devices/${encId(id)}/apps/install`, body);
  },

  /** Launch an installed app by bundle ID / package name. */
  launchApp(id: string, body: SimLaunchAppReq): Promise<SimActionOkResp> {
    return api.post<SimActionOkResp>(`/simulators/devices/${encId(id)}/apps/launch`, body);
  },

  /** Terminate a running app by bundle ID / package name. */
  terminateApp(id: string, body: SimTerminateAppReq): Promise<SimActionOkResp> {
    return api.post<SimActionOkResp>(`/simulators/devices/${encId(id)}/apps/terminate`, body);
  },

  /** List active reverse port forwards (`adb reverse`). */
  listPorts(id: string): Promise<SimPortsResp> {
    return api.get<SimPortsResp>(`/simulators/devices/${encId(id)}/ports`);
  },

  /** Add a reverse port forward (`adb reverse tcp:<remote> tcp:<local>`). */
  addPortForward(id: string, body: CreateSimPortForwardReq): Promise<SimPortForward> {
    return api.post<SimPortForward>(`/simulators/devices/${encId(id)}/ports`, body);
  },

  /** Remove a reverse port forward by remote port. */
  removePortForward(id: string, remotePort: number): Promise<void> {
    return api.del<void>(`/simulators/devices/${encId(id)}/ports/${remotePort}`);
  },

  /** Poll the built-in Android Emulator / iOS Runtime installer status. */
  installStatus(): Promise<SimInstallStatus> {
    return api.get<SimInstallStatus>('/simulators/install/status');
  },

  /** Start a single-flight built-in installation (`android_emulator`, `android_platform_tools`, `ios_runtime`). */
  startInstall(body: SimInstallReq): Promise<SimInstallStatus> {
    return api.post<SimInstallStatus>('/simulators/install', body);
  },

  /** Cancel an in-flight built-in installation. */
  cancelInstall(): Promise<SimInstallStatus> {
    return api.post<SimInstallStatus>('/simulators/install/cancel', {});
  },

  /** Open the live screen + input WebSocket (`WS /ws/simulators/{id}/stream`). */
  openStreamSocket(
    id: string,
    opts?: { fps?: number; format?: SimScreenshotFormat },
  ): WebSocket {
    const q = buildQuery({ fps: opts?.fps, format: opts?.format });
    const sock = wsConnect(`/ws/simulators/${encId(id)}/stream${q}`);
    sock.binaryType = 'arraybuffer';
    return sock;
  },

  /** Open the live device log WebSocket (`WS /ws/simulators/{id}/logs`). */
  openLogsSocket(id: string, opts?: { filter?: string; level?: string }): WebSocket {
    const q = buildQuery({ filter: opts?.filter, level: opts?.level });
    return wsConnect(`/ws/simulators/${encId(id)}/logs${q}`);
  },
};

/**
 * Decode a binary frame from `WS /ws/simulators/{id}/stream`.
 * Delegates to the shared `decodeBinaryFrame` codec (`DUP-8`).
 */
export function decodeSimFrame(buf: ArrayBuffer): DecodedSimFrame | null {
  return decodeBinaryFrame<SimFrameHeader>(buf);
}
