// Mock-mode `GET /machine/status` (ui/src/lib/api/mock.ts routes it). Calm by
// default; `localStorage.otto_mock_machine = 'busy'` reports a loaded Mac
// with two queued background agents, `'low-disk'` a nearly full disk — read
// per request, so an e2e can flip it and wait for the next poll.
import type { MachineStatus } from './types/machine';

type Out = { status?: number; json?: unknown } | undefined;
type MockRoute = { method: string; re: RegExp; handle: (m: RegExpMatchArray, body: unknown, q: URLSearchParams) => Out };

const GIB = 1024 ** 3;

function mode(): string {
  try {
    return localStorage.getItem('otto_mock_machine') ?? 'calm';
  } catch {
    return 'calm';
  }
}

export function mockMachineStatus(m: string = mode()): MachineStatus {
  const busy = m === 'busy';
  return {
    load1: busy ? 14.2 : 3.1,
    cores: 8,
    load_per_core: busy ? 1.775 : 0.3875,
    memory_pressure: busy ? 'warn' : 'normal',
    swap_used_bytes: busy ? 6 * GIB : 512 * 1024 * 1024,
    ram_bytes: 32 * GIB,
    admission_enabled: true,
    max_load_per_core: 1.5,
    max_wait_secs: 300,
    background_live: busy ? 6 : 2,
    background_max_live: 6,
    pressure_floor: 3,
    agents_live: busy ? 9 : 3,
    busy,
    busy_reason: busy ? 'memory pressure is high' : null,
    admission: busy ? { kind: 'queue', ahead: 2 } : null,
    queued: busy
      ? [
          {
            source: 'review',
            waiting_secs: 95,
            reason: 'Waiting for memory — 6 Otto agents are running',
            wait: { kind: 'memory_pressure', level: 'warn', live: 6, floor: 3 },
          },
          {
            source: 'workflow',
            waiting_secs: 40,
            reason: 'Waiting for a free agent slot — 1 ahead in line',
            wait: { kind: 'queue', ahead: 1 },
          },
        ]
      : [],
    disk_free_bytes: m === 'low-disk' ? 6 * GIB : 142 * GIB,
    low_disk_gb: 10,
    low_disk: m === 'low-disk',
  };
}

export const machineMockRoutes: MockRoute[] = [
  { method: 'GET', re: /^\/machine\/status$/, handle: () => ({ json: mockMachineStatus() }) },
];
