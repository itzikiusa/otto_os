/** Camera key: focus point (normalized content coords) + zoom at shot progress t∈[0,1]. */
export interface CamKey {
  t: number;
  x: number;
  y: number;
  z: number;
}

/** A label pinned to a point of the footage (normalized coords), shown from t0 to t1. */
export interface Callout {
  t0: number;
  t1?: number;
  x: number;
  y: number;
  label: string;
  /** Where the pill sits relative to the point. */
  side?: 'left' | 'right' | 'top' | 'bottom';
  /** Optional keyboard hint rendered as keycaps (e.g. "⌘K"). */
  keys?: string;
}

/** A spotlight rectangle (normalized coords): dims everything else. */
export interface Spot {
  t0: number;
  t1: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface ScreenShot {
  kind: 'screen';
  /** Path under public/ (png/jpg still or mp4 clip). */
  src: string;
  /** Relative length inside its chapter. */
  weight?: number;
  /** For clips: start offset (s) and speed. */
  from?: number;
  rate?: number;
  cam?: CamKey[];
  callouts?: Callout[];
  spots?: Spot[];
  /** Small caption chip at the bottom (feature name). */
  chip?: string;
  /** Show the light-scheme variant half-way (wipe) — src2 is the other scheme. */
  wipeTo?: string;
}

export interface MontageShot {
  kind: 'montage';
  weight?: number;
  items: { src: string; label: string; sub?: string; x?: number; y?: number; z?: number }[];
}

export interface CustomShot {
  kind: 'custom';
  weight?: number;
  id: 'desktop' | 'phone';
}

export type Shot = ScreenShot | MontageShot | CustomShot;

export interface ChapterSpec {
  id: string;
  group: string;
  /** Title shown on the chapter card. */
  title: string;
  /** One-line subtitle on the chapter card. */
  kicker?: string;
  shots: Shot[];
}

export interface Timing {
  fps: number;
  totalFrames: number;
  totalSeconds: number;
  chapters: {
    id: string;
    section: string;
    title: string;
    startFrame: number;
    frames: number;
    voOffset: number;
    voDuration: number;
    hasVo: boolean;
    lines: { text: string; start: number; end: number }[];
  }[];
}
