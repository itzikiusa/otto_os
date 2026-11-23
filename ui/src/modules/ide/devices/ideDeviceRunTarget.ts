// Lightweight, zero-dependency holder for the active IDE mobile simulator/emulator Run Target.
// Kept separate from `ideDevicesState.svelte.ts` so `ideDebugAndEnvController` stays decoupled
// from the simulators API client and `node:vm` unit test harnesses.

export interface IdeMobileRunTarget {
  device_id?: string;
  device_platform?: 'ios' | 'android';
}

let activeTarget: IdeMobileRunTarget = {};

export function setIdeMobileRunTarget(next: IdeMobileRunTarget): void {
  activeTarget = next.device_id ? { ...next } : {};
}

export function getIdeMobileRunTarget(): IdeMobileRunTarget {
  return activeTarget.device_id ? { ...activeTarget } : {};
}
