import timing from './generated/timing.json';
import { SCENES } from './scenes';
import type { ChapterSpec, Shot, Timing } from './types';

export const T = timing as Timing;
export const FPS = T.fps;
/** Frames per beat (15 at 120 BPM / 30 fps). */
export const BEAT = T.beatFrames;

export interface PlacedShot {
  shot: Shot;
  chapter: PlacedChapter;
  from: number; // absolute frame
  frames: number;
  index: number; // within its chapter
  /** Position in the whole film (for alternating transitions). */
  global: number;
}

export interface PlacedChapter {
  id: string;
  spec: ChapterSpec;
  from: number;
  frames: number;
  music: string | null;
  shots: PlacedShot[];
  /** 1-based among the numbered chapters (0 for intro/outro). */
  number: number;
}

/** Intro and outro are bespoke; everything else is numbered on the progress rail. */
export const NUMBERED = T.chapters.filter((c) => c.id !== 'intro' && c.id !== 'outro').map((c) => c.id);

/** Lay every shot on the beat grid; the shots of a chapter must fill it exactly. */
export function layout(): PlacedChapter[] {
  let global = 0;
  return T.chapters.map((c) => {
    const spec = SCENES[c.id];
    if (!spec) throw new Error(`no scene spec for chapter ${c.id}`);
    const beats = spec.shots.reduce((s, sh) => s + sh.beats, 0);
    if (beats * BEAT !== c.frames) throw new Error(`chapter ${c.id}: shots fill ${beats} beats, the edit has ${c.frames / BEAT}`);
    const chapter: PlacedChapter = {
      id: c.id,
      spec,
      from: c.startFrame,
      frames: c.frames,
      music: c.music,
      shots: [],
      number: NUMBERED.indexOf(c.id) + 1,
    };
    let at = c.startFrame;
    chapter.shots = spec.shots.map((shot, i) => {
      const frames = Math.round(shot.beats * BEAT);
      const placed = { shot, chapter, from: at, frames, index: i, global: global++ };
      at += frames;
      return placed;
    });
    return chapter;
  });
}

export const CHAPTERS = layout();
export const SHOTS = CHAPTERS.flatMap((c) => c.shots);
