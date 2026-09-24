import timing from './generated/timing.json';
import { SCENES } from './scenes';
import type { ChapterSpec, Shot, Timing } from './types';

export const T = timing as Timing;
export const FPS = T.fps;

export interface PlacedShot {
  shot: Shot;
  from: number; // absolute frame
  frames: number;
  index: number;
  count: number;
}

export interface PlacedChapter {
  id: string;
  spec: ChapterSpec;
  from: number;
  frames: number;
  voFrom: number;
  voFrames: number;
  hasVo: boolean;
  shots: PlacedShot[];
  number: number;
}

/** Intro and outro are bespoke; everything else is numbered on its card. */
export const NUMBERED = T.chapters.filter((c) => c.id !== 'intro' && c.id !== 'outro').map((c) => c.id);

export function layout(): PlacedChapter[] {
  return T.chapters.map((c) => {
    const spec = SCENES[c.id];
    if (!spec) throw new Error(`no scene spec for chapter ${c.id}`);
    const total = spec.shots.reduce((s, sh) => s + (sh.weight ?? 1), 0);
    let at = c.startFrame;
    const shots = spec.shots.map((shot, i) => {
      const isLast = i === spec.shots.length - 1;
      const frames = isLast ? c.startFrame + c.frames - at : Math.round((c.frames * (shot.weight ?? 1)) / total);
      const placed = { shot, from: at, frames, index: i, count: spec.shots.length };
      at += frames;
      return placed;
    });
    return {
      id: c.id,
      spec,
      from: c.startFrame,
      frames: c.frames,
      voFrom: c.startFrame + Math.round(c.voOffset * FPS),
      voFrames: Math.round(c.voDuration * FPS),
      hasVo: c.hasVo,
      shots,
      number: NUMBERED.indexOf(c.id) + 1,
    };
  });
}
