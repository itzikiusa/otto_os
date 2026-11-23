// Mobile Simulators & Emulators (iOS Simulator + Android Emulator).
// Mirrors crates/otto-simulators/src/types.rs and docs/contracts/api.md + ws.md.

export type SimPlatform = 'ios' | 'android';

export type SimDeviceState =
  | 'booted'
  | 'booting'
  | 'shutdown'
  | 'shutting_down'
  | 'offline'
  | 'unknown';

export type SimFormFactor = 'phone' | 'tablet' | 'watch' | 'tv' | 'other';

export type SimOrientation =
  | 'portrait'
  | 'landscape'
  | 'portrait_upside_down'
  | 'landscape_right'
  | 'unknown';

export type SimAppearance = 'light' | 'dark';

export type SimReadinessState = 'ready' | 'degraded' | 'unavailable';

export type SimReadinessCode =
  | 'ready'
  | 'xcode_not_installed'
  | 'xcode_downloading'
  | 'xcode_select_required'
  | 'xcode_license_required'
  | 'ios_runtime_missing'
  | 'no_devices'
  | 'sdk_not_found'
  | 'studio_installed_sdk_missing'
  | 'adb_missing'
  | 'emulator_missing'
  | 'no_avds'
  | 'unsupported_os';

export interface SimToolStatus {
  name: string;
  available: boolean;
  path?: string | null;
  version?: string | null;
  detail?: string | null;
}

export interface SimReadinessIssue {
  code: string;
  severity: 'error' | 'warning' | 'info';
  message: string;
}

export interface SimSetupStep {
  title: string;
  command?: string | null;
  url?: string | null;
  detail: string;
}

export interface SimPlatformReadiness {
  platform: SimPlatform;
  state: SimReadinessState;
  code: SimReadinessCode;
  summary: string;
  sdk_path?: string | null;
  tools: SimToolStatus[];
  issues: SimReadinessIssue[];
  setup_steps: SimSetupStep[];
}

export interface SimulatorsStatus {
  ios: SimPlatformReadiness;
  android: SimPlatformReadiness;
  checked_at: string;
}

export interface SimScreenMetrics {
  width: number;
  height: number;
  scale?: number | null;
  dpi?: number | null;
}

export interface SimDeviceCapabilities {
  boot: boolean;
  shutdown: boolean;
  delete: boolean;
  open_url: boolean;
  screenshot: boolean;
  stream: boolean;
  tap: boolean;
  swipe: boolean;
  text: boolean;
  key_event: boolean;
  rotate: boolean;
  appearance: boolean;
  status_bar: boolean;
  clipboard: boolean;
  port_forward: boolean;
  app_install: boolean;
  logs: boolean;
}

export interface SimDevice {
  id: string;
  platform: SimPlatform;
  name: string;
  udid?: string | null;
  avd_name?: string | null;
  serial?: string | null;
  state: SimDeviceState;
  os_version: string;
  api_level?: number | null;
  runtime_id?: string | null;
  device_type?: string | null;
  form_factor: SimFormFactor;
  screen?: SimScreenMetrics | null;
  orientation: SimOrientation;
  appearance?: SimAppearance | null;
  is_physical: boolean;
  capabilities: SimDeviceCapabilities;
}

export interface SimDevicesResp {
  devices: SimDevice[];
  status: SimulatorsStatus;
}

export interface SimRuntime {
  id: string;
  name: string;
  version: string;
  platform: SimPlatform;
  available: boolean;
}

export interface SimDeviceType {
  id: string;
  name: string;
  form_factor: SimFormFactor;
}

export interface AndroidSystemImage {
  id: string;
  api_level?: number | null;
  abi?: string | null;
  tag?: string | null;
  installed: boolean;
}

export interface AndroidDeviceProfile {
  id: string;
  name: string;
  form_factor: SimFormFactor;
}

export interface SimCatalogResp {
  ios_runtimes: SimRuntime[];
  ios_device_types: SimDeviceType[];
  android_system_images: AndroidSystemImage[];
  android_device_profiles: AndroidDeviceProfile[];
}

export type CreateSimDeviceReq =
  | {
      platform: 'ios';
      name: string;
      device_type_id: string;
      runtime_id: string;
    }
  | {
      platform: 'android';
      name: string;
      system_image_id: string;
      device_profile?: string;
    };

export interface BootSimDeviceReq {
  wait?: boolean;
  timeout_secs?: number;
  /** Open macOS Simulator.app window on iOS (defaults to `false`, embedded inside Otto). */
  open_gui?: boolean;
  /** Run Android Emulator with `-no-window` (defaults to `true`, embedded inside Otto). */
  headless?: boolean;
  wipe_data?: boolean;
}

export type RestartSimDeviceReq = Omit<BootSimDeviceReq, 'wipe_data'>;

export interface SimRecentLogEntry {
  ts: string;
  level: SimLogLevel;
  tag?: string | null;
  pid?: number | null;
  message: string;
}

export interface SimRecentLogsResp {
  device_id: string;
  lines: SimRecentLogEntry[];
  truncated: boolean;
}

export interface OpenSimUrlReq {
  url: string;
  auto_forward_localhost?: boolean;
}

export interface OpenSimUrlResp {
  ok: boolean;
  url: string;
  effective_url: string;
  forwarded_ports: number[];
}

export type SimScreenshotFormat = 'png' | 'jpeg';

export interface SimScreenshotJsonResp {
  mime: string;
  width: number;
  height: number;
  base64: string;
  captured_at: string;
}

export type SimInputReq =
  | { type: 'tap'; x: number; y: number; normalized?: boolean }
  | {
      type: 'swipe';
      x1: number;
      y1: number;
      x2: number;
      y2: number;
      duration_ms?: number;
      normalized?: boolean;
    }
  | { type: 'text'; text: string }
  | { type: 'key_event'; key: string };

export interface SimOrientationReq {
  orientation: Exclude<SimOrientation, 'unknown'>;
}

export interface SimAppearanceReq {
  appearance: SimAppearance;
}

export interface SimStatusBarReq {
  clear?: boolean;
  time?: string;
  battery_level?: number;
  battery_state?: 'charging' | 'charged' | 'discharging';
  wifi_bars?: number;
  cellular_bars?: number;
  operator_name?: string;
}

export interface SimClipboardResp {
  text: string;
}

export interface SimInstallAppReq {
  path: string;
}

export interface SimLaunchAppReq {
  app_id: string;
  args?: string[];
}

export interface SimTerminateAppReq {
  app_id: string;
}

export interface SimPortForward {
  remote_port: number;
  local_port: number;
  protocol: string;
}

export interface SimPortsResp {
  ports: SimPortForward[];
}

export interface CreateSimPortForwardReq {
  remote_port: number;
  local_port: number;
}

export interface SimActionOkResp {
  ok: boolean;
  detail?: string | null;
}

// ---------------------------------------------------------------------------
// Built-in SDK / Runtime Installer
// ---------------------------------------------------------------------------

export type SimInstallTarget =
  | 'android_emulator'
  | 'android_platform_tools'
  | 'ios_runtime';

export type SimInstallStage =
  | 'idle'
  | 'downloading'
  | 'extracting'
  | 'licensing'
  | 'installing_packages'
  | 'creating_avds'
  | 'done'
  | 'failed'
  | 'cancelled';

export interface SimInstallReq {
  target: SimInstallTarget;
  create_default_avds?: boolean;
}

export interface SimInstallStatus {
  running: boolean;
  target?: SimInstallTarget | null;
  stage: SimInstallStage;
  progress_pct?: number | null;
  message: string;
  log_tail: string[];
  error?: string | null;
  updated_at: string;
}

// ---------------------------------------------------------------------------
// WebSocket wire protocol (WS /ws/simulators/{id}/stream & /logs)
// ---------------------------------------------------------------------------

export interface SimFrameHeader {
  seq: number;
  mime: string;
  width: number;
  height: number;
  orientation: SimOrientation;
  timestamp_ms: number;
}

export interface DecodedSimFrame {
  header: SimFrameHeader;
  bytes: Uint8Array;
}

export type SimStreamServerMsg =
  | { type: 'hello'; device: SimDevice; fps: number; can_drive: boolean }
  | { type: 'state'; device: SimDevice }
  | { type: 'error'; code: string; message: string }
  | { type: 'closed'; reason: string };

export type SimStreamClientMsg =
  | { type: 'ack'; seq: number }
  | { type: 'set_fps'; fps: number }
  | { type: 'pause' }
  | { type: 'resume' }
  | { type: 'snapshot' }
  | { type: 'tap'; x: number; y: number; normalized?: boolean }
  | {
      type: 'swipe';
      x1: number;
      y1: number;
      x2: number;
      y2: number;
      duration_ms?: number;
      normalized?: boolean;
    }
  | { type: 'text'; text: string }
  | { type: 'key_event'; key: string }
  | { type: 'open_url'; url: string; auto_forward_localhost?: boolean }
  | { type: 'orientation'; orientation: Exclude<SimOrientation, 'unknown'> }
  | { type: 'appearance'; appearance: SimAppearance };

export type SimLogLevel = 'verbose' | 'debug' | 'info' | 'warn' | 'error' | 'fatal' | 'unknown';

export type SimLogServerMsg =
  | {
      type: 'log';
      ts: string;
      level: SimLogLevel;
      tag?: string | null;
      pid?: number | null;
      message: string;
    }
  | { type: 'status'; state: string; detail?: string | null }
  | { type: 'error'; code: string; message: string };
