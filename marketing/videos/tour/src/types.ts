// Every time inside a shot is in BEATS from the shot's start (120 BPM → one
// beat is 15 frames), so moves, callouts and cuts land on the music.

/** Camera key: focus point (normalized footage coords) + zoom, reached at beat `b`. */
export interface CamKey {
  b: number;
  x: number;
  y: number;
  z: number;
}

/** A label pinned to a point of the footage (normalized coords), shown from beat b to `until`. */
export interface Callout {
  b: number;
  until?: number;
  x: number;
  y: number;
  label: string;
  /** Where the pill sits relative to the point. */
  side?: 'left' | 'right' | 'top' | 'bottom';
  /** Optional keyboard hint rendered as keycaps (e.g. "⌘ K"). */
  keys?: string;
}

/** A glowing highlight ring around a region (normalized coords); dims the rest. */
export interface Highlight {
  b: number;
  until?: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** How a shot arrives. Chapter starts always use the chapter entrance. */
export type Transition = 'whip' | 'push' | 'wipe';

export interface ScreenShot {
  kind: 'screen';
  /** Path under public/ (jpg/png still or mp4 clip). */
  src: string;
  /** Length in beats (whole or half beats). */
  beats: number;
  /** For clips: start offset (s) and speed. */
  from?: number;
  rate?: number;
  cam?: CamKey[];
  callouts?: Callout[];
  highlights?: Highlight[];
  /** The line under the chapter title while this shot is on screen. */
  label?: string;
  /** A big glass card in the window's lower-left (quick-cut montages). */
  card?: { title: string; sub?: string };
  /** Wipe to the other colour scheme between beats [a, b]. */
  wipeTo?: { src: string; b0: number; b1: number };
  transition?: Transition;
}

export interface CustomShot {
  kind: 'custom';
  id: 'intro' | 'desktop' | 'outro';
  beats: number;
  label?: string;
}

export type Shot = ScreenShot | CustomShot;

export interface ChapterSpec {
  id: string;
  group: string;
  /** Title on the chapter's kinetic card (Help keeps its own, longer title). */
  title: string;
  /** One-line subtitle under the title. */
  kicker?: string;
  shots: Shot[];
}

export interface Timing {
  edition: string;
  bpm: number;
  fps: number;
  beatFrames: number;
  barFrames: number;
  totalBars: number;
  totalFrames: number;
  totalSeconds: number;
  chapters: {
    id: string;
    section: string;
    title: string;
    music: string | null;
    startBar: number;
    bars: number;
    startFrame: number;
    frames: number;
    lines: { text: string; start: number; end: number }[];
  }[];
}
