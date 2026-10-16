// TEMP stub.
import type { ShooterEvent, ShooterInput, ShooterState } from './sim.ts';
import type { FighterId } from './catalog.ts';
export function encodeSnapshot(_s: ShooterState, _e: ShooterEvent[]): unknown { return {}; }
export function decodeSnapshot(_d: unknown, _n: (slot: number) => string): ShooterState | null { return null; }
export function encodeInput(_i: ShooterInput, _f: FighterId): unknown { return {}; }
export function decodeInput(_d: unknown): { input: ShooterInput; fighter: FighterId } | null { return null; }
