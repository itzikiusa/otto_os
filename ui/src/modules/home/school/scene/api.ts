// Otto School — the scene's public surface: what ClassroomsBox drives
// (SchoolHandle, the view / pick / debug shapes) and the status-pin palette.
// Re-exported from ../scene.ts; no three.js here.

import type { Pose, School } from '../model.ts';
import type { ScreenInfo } from '../screens.ts';

export type SchoolView = { kind: 'corridor' } | { kind: 'room'; roomId: string } | { kind: 'screen'; roomId: string; kidId: string };
export type SchoolPick = { kind: 'kid'; id: string } | { kind: 'door'; id: string } | { kind: 'head'; id: 'headmaster' };

export interface ScreenPoint {
  x: number;
  y: number;
  visible: boolean;
}

export interface SchoolDebug {
  view: SchoolView;
  frames: number;
  missing: string[];
  kids: { id: string; clip: string; mode: string; visible: boolean; x: number; z: number; pin: PinKind | null }[];
  head: { clip: string; mode: string; x: number; z: number; target: string | null } | null;
  camera: { x: number; y: number; z: number };
  screens: Record<string, string>;
  /** Desk tag of the monitor painted at 2× (the selected / screen-view kid). */
  hiRes: string | null;
  /** Warm ceiling lamps lighting the open room. */
  lamps: number;
  /** Rooms and the corridor are closed by a ceiling. */
  ceiling: boolean;
}

/** What the pin over a kid's head says (null = no pin: idle / away). */
export type PinKind = 'working' | 'needs-you' | 'error' | 'done' | 'stale';
export const PIN_COLORS: Record<PinKind, number> = {
  working: 0x3fdc6e,
  'needs-you': 0xffb020,
  error: 0xff4d4d,
  done: 0x5aa8ff,
  stale: 0x9aa4b2,
};
export function pinFor(pose: Pose, detention: boolean): PinKind | null {
  if (detention) return null;
  return pose === 'working' || pose === 'needs-you' || pose === 'error' || pose === 'done' || pose === 'stale' ? pose : null;
}

export interface SchoolHandle {
  update(school: School): void;
  view(): SchoolView;
  onView(fn: (v: SchoolView) => void): void;
  enterRoom(id: string): Promise<void>;
  toCorridor(): Promise<void>;
  lookAtScreen(kidId: string): Promise<void>;
  /** Screen view → back to the room overview. */
  backToRoom(): Promise<void>;
  pick(clientX: number, clientY: number): SchoolPick | null;
  setHover(p: SchoolPick | null): void;
  setSelected(kidId: string | null): void;
  project(p: SchoolPick): ScreenPoint | null;
  /** What a kid's monitor shows (the host feeds the live screen). */
  setScreen(kidId: string, info: ScreenInfo): void;
  /** Kids in the open room whose monitor the camera can see, nearest first. */
  visibleScreens(): string[];
  /** The door the corridor camera faces (Enter walks in). */
  facingDoor(): string | null;
  kickOut(id: string): Promise<void>;
  detention(id: string): Promise<void>;
  restore(id: string): void;
  inspect(id: string): void;
  /** Keyboard (walk / orbit). Returns true when handled. */
  key(e: KeyboardEvent, down: boolean): boolean;
  drag(dx: number, dy: number): void;
  wheel(dy: number): void;
  resetView(): void;
  setActive(on: boolean): void;
  setTheme(dark: boolean): void;
  onFrame(fn: () => void): void;
  onContextLost(fn: () => void): void;
  debug(): SchoolDebug;
  destroy(): void;
}

export interface MountOpts {
  reducedMotion: boolean;
  label: string;
  dark: boolean;
}
