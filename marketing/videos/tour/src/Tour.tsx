import React from 'react';
import { AbsoluteFill, Audio, Sequence, interpolate, staticFile, useCurrentFrame } from 'remotion';
import { Backdrop } from './components/Backdrop';
import { ScreenContent, WIN, WIN_LEFT, Window } from './components/Screen';
import { ChapterHeader, ProgressRail } from './components/Titles';
import { Desktop } from './components/Desktop';
import { Intro } from './components/Intro';
import { Outro } from './components/Outro';
import { BEAT, CHAPTERS, SHOTS, T, type PlacedShot } from './layout';
import { C, GROUP_TINT } from './theme';
import { SNAP, clamp, easeInOut, sp } from './motion';
import type { Transition } from './types';

/** Frames either side of a cut that its transition spans (cuts sit on the beat). */
const HALF = 6;
type Edge = Transition | 'chapter' | 'none';

/** How each shot arrives: the chapter entrance on a chapter start, else its own or the next in rotation. */
const ROTATION: Transition[] = ['whip', 'push', 'wipe', 'push'];
function inEdge(s: PlacedShot): Edge {
  if (s.global === 0) return 'none';
  if (s.index === 0) return s.chapter.id === 'outro' || s.chapter.id === 'intro' ? 'none' : 'chapter';
  if (s.shot.kind === 'custom') return 'push';
  return s.shot.transition ?? ROTATION[s.global % ROTATION.length];
}
const EDGES = SHOTS.map(inEdge);

export const Tour: React.FC = () => (
  <AbsoluteFill style={{ background: C.bgDeep }}>
    <Backdrop />
    {SHOTS.map((s, i) => {
      const lead = EDGES[i] === 'none' || EDGES[i] === 'chapter' ? 0 : HALF;
      const next = EDGES[i + 1];
      const tail = next == null ? 0 : next === 'none' ? 0 : HALF;
      return (
        <Sequence key={i} from={s.from - lead} durationInFrames={lead + s.frames + tail} name={`${s.chapter.id}:${s.index}`}>
          <ShotView s={s} lead={lead} inE={EDGES[i]} outE={next ?? 'none'} />
        </Sequence>
      );
    })}
    {CHAPTERS.filter((c) => c.number > 0).map((c) => (
      <Sequence key={`h-${c.id}`} from={c.from} durationInFrames={c.frames} name={`title:${c.id}`}>
        <ChapterHeader ch={c} />
      </Sequence>
    ))}
    <ProgressRail />
    {/* ── sound: the score carries the impacts; the edit adds whooshes and callout ticks ── */}
    <Audio src={staticFile('audio/music.wav')} volume={(f) => interpolate(f, [T.totalFrames - 30, T.totalFrames - 1], [1, 0], clamp)} />
    {SHOTS.map((s, i) =>
      EDGES[i] === 'whip' ? (
        <Sequence key={`wh-${i}`} from={s.from - HALF - 2} durationInFrames={18} name="sfx:whoosh">
          <Audio src={staticFile('audio/sfx-whoosh.wav')} volume={0.22} />
        </Sequence>
      ) : null,
    )}
    {SHOTS.flatMap((s) =>
      s.shot.kind === 'screen'
        ? (s.shot.callouts ?? []).map((co, j) => (
            <Sequence key={`tk-${s.global}-${j}`} from={s.from + Math.round(co.b * BEAT)} durationInFrames={14} name="sfx:tick">
              <Audio src={staticFile('audio/sfx-tick.wav')} volume={0.16} />
            </Sequence>
          ))
        : [],
    )}
  </AbsoluteFill>
);

/**
 * One shot with its entrance and exit. `f` below is frames since the shot's
 * nominal start (the cut, on the beat); the sequence starts `lead` earlier and
 * runs past the next cut so the two overlap for the transition.
 */
const ShotView: React.FC<{ s: PlacedShot; lead: number; inE: Edge; outE: Edge }> = ({ s, lead, inE, outE }) => {
  const f = useCurrentFrame() - lead;
  const n = s.frames;
  const tint = GROUP_TINT[s.chapter.spec.group] ?? C.accent;
  let style: React.CSSProperties = {};
  // ── entrance ──
  if (inE === 'chapter') {
    const k = sp(f, 0, SNAP);
    style = { transform: `translateY(${(1 - k) * 90}px) scale(${0.86 + 0.14 * k}) perspective(2400px) rotateX(${(1 - k) * 9}deg)`, opacity: interpolate(f, [0, 5], [0, 1], clamp) };
  } else if (inE === 'whip') {
    const q = easeInOut(interpolate(f, [-HALF, HALF], [0, 1], clamp));
    style = { transform: `translateX(${(1 - q) * 1920}px)`, filter: q < 1 ? `blur(${Math.sin(q * Math.PI) * 10}px)` : undefined };
  } else if (inE === 'push') {
    const k = sp(f, -HALF + 2, SNAP);
    style = { transform: `scale(${0.9 + 0.1 * k})`, opacity: interpolate(f, [-HALF, 0], [0, 1], clamp) };
  } else if (inE === 'wipe') {
    const q = easeInOut(interpolate(f, [-HALF, HALF], [0, 1], clamp));
    style = { clipPath: `polygon(0 0, ${q * 130}% 0, ${q * 130 - 30}% 100%, 0 100%)` };
  }
  // ── exit (the next shot's entrance decides it) ──
  if (outE === 'whip') {
    const q = easeInOut(interpolate(f, [n - HALF, n + HALF], [0, 1], clamp));
    const t = `translateX(${-q * 1920}px)`;
    style = { ...style, transform: `${style.transform ?? ''} ${t}`, filter: q > 0 && q < 1 ? `blur(${Math.sin(q * Math.PI) * 10}px)` : style.filter };
  } else if (outE === 'push') {
    const q = interpolate(f, [n - HALF, n + 2], [0, 1], clamp);
    style = { ...style, transform: `${style.transform ?? ''} scale(${1 + 0.08 * q})`, opacity: (style.opacity as number ?? 1) * (1 - q) };
  } else if (outE === 'chapter' || (outE === 'none' && s.chapter.id === 'rooms')) {
    // Zoom through into the next chapter.
    const q = interpolate(f, [n - HALF, n], [0, 1], { ...clamp, easing: easeInOut });
    style = { ...style, transform: `${style.transform ?? ''} scale(${1 + 0.3 * q})`, opacity: (style.opacity as number ?? 1) * (1 - q), filter: q > 0 ? `blur(${q * 12}px)` : style.filter };
  }
  // 'wipe' exits by being covered.

  let body: React.ReactNode = null;
  if (s.shot.kind === 'screen') {
    body = (
      <Window tint={tint}>
        <ScreenContent shot={s.shot} f={f} frames={n} lead={lead} tint={tint} />
      </Window>
    );
  } else if (s.shot.id === 'desktop') {
    // A whole Mac desktop, shown as a display in the window's place under the header.
    body = (
      <div style={{ position: 'absolute', left: WIN_LEFT, top: WIN.top + 14, width: WIN.w, height: WIN.h, borderRadius: 14, overflow: 'hidden', boxShadow: '0 40px 120px rgba(0,0,0,0.6), 0 0 0 1px rgba(255,255,255,0.12)' }}>
        <div style={{ width: 1920, height: 1080, transform: `scale(${WIN.w / 1920})`, transformOrigin: '0 0' }}>
          <Desktop progress={Math.max(0, Math.min(1, f / n))} frames={n} />
        </div>
      </div>
    );
  }
  else if (s.shot.id === 'intro') body = <Intro frames={n} />;
  else if (s.shot.id === 'outro') body = <Outro frames={n} />;
  return <AbsoluteFill style={style}>{body}</AbsoluteFill>;
};
