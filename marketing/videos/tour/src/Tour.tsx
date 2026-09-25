import React from 'react';
import { AbsoluteFill, Audio, Easing, Sequence, interpolate, staticFile, useCurrentFrame } from 'remotion';
import { Backdrop } from './components/Backdrop';
import { Screen } from './components/Screen';
import { Montage } from './components/Montage';
import { ChapterCard, FeatureChip } from './components/Titles';
import { Desktop } from './components/Desktop';
import { Intro } from './components/Intro';
import { Outro } from './components/Outro';
import { FPS, NUMBERED, T, layout, type PlacedChapter, type PlacedShot } from './layout';

const CHAPTERS = layout();
const XFADE = 12; // frames of overlap between shots inside a chapter
const clamp = { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' } as const;

/** Instrumental edition: a continuous score with gentle opening/closing fades. */
function musicGain(f: number): number {
  const fadeIn = interpolate(f, [0, 20], [0, 1], clamp);
  const fadeOut = interpolate(f, [T.totalFrames - 60, T.totalFrames - 1], [1, 0], clamp);
  return 0.85 * fadeIn * fadeOut;
}

export const Tour: React.FC = () => (
  <AbsoluteFill style={{ background: '#0b0b0e' }}>
    <Backdrop />
    {CHAPTERS.map((ch) => (
      <Sequence key={ch.id} from={ch.from} durationInFrames={ch.frames} name={`ch:${ch.id}`}>
        <ChapterView ch={ch} />
      </Sequence>
    ))}
    {/* ── sound ── */}
    <Audio src={staticFile('audio/music.wav')} volume={musicGain} />
    {CHAPTERS.slice(1).map((c) => (
      <Sequence key={`wh-${c.id}`} from={Math.max(0, c.from - 8)} durationInFrames={24} name="sfx:whoosh">
        <Audio src={staticFile('audio/sfx-whoosh.wav')} volume={0.32} />
      </Sequence>
    ))}
    {CHAPTERS.flatMap((c) =>
      c.shots.flatMap((s) =>
        s.shot.kind === 'screen'
          ? (s.shot.callouts ?? []).map((co, i) => (
              <Sequence key={`tk-${c.id}-${s.index}-${i}`} from={s.from + Math.round(co.t0 * s.frames)} durationInFrames={15} name="sfx:tick">
                <Audio src={staticFile('audio/sfx-tick.wav')} volume={0.2} />
              </Sequence>
            ))
          : [],
      ),
    )}
  </AbsoluteFill>
);

const ChapterView: React.FC<{ ch: PlacedChapter }> = ({ ch }) => {
  if (ch.id === 'intro') return <Intro frames={ch.frames} />;
  if (ch.id === 'outro') return <Outro ch={ch} />;
  return (
    <AbsoluteFill>
      {ch.shots.map((s) => (
        <Sequence
          key={s.index}
          from={s.from - ch.from - (s.index > 0 ? XFADE : 0)}
          durationInFrames={s.frames + (s.index > 0 ? XFADE : 0)}
          name={`shot:${s.index}`}
        >
          <ShotView s={s} chapterFrames={ch.frames} chapterFrom={ch.from} />
        </Sequence>
      ))}
      <ChapterCard group={ch.spec.group} title={ch.spec.title} kicker={ch.spec.kicker} index={ch.number} total={NUMBERED.length} />
    </AbsoluteFill>
  );
};

const ShotView: React.FC<{ s: PlacedShot; chapterFrames: number; chapterFrom: number }> = ({ s, chapterFrames, chapterFrom }) => {
  const f = useCurrentFrame();
  const lead = s.index > 0 ? XFADE : 0;
  const local = f - lead; // frames since the shot's nominal start
  const progress = Math.max(0, Math.min(1, local / s.frames));
  const isFirst = s.index === 0;
  const isLast = s.index === s.count - 1;
  // Chapter entrance (first shot) vs. crossfade (later shots).
  const enter = isFirst ? interpolate(f, [0, 20], [0, 1], { ...clamp, easing: Easing.out(Easing.cubic) }) : 1;
  const fadeIn = isFirst ? 1 : interpolate(f, [0, XFADE], [0, 1], clamp);
  // Chapter exit (last shot): shrink + fade over the final 10 frames.
  const endLocal = s.from - chapterFrom + s.frames; // chapter-relative end
  void endLocal;
  const out = isLast ? interpolate(local, [s.frames - 10, s.frames], [1, 0], { ...clamp, easing: Easing.in(Easing.cubic) }) : 1;
  const scale = isLast ? 0.965 + 0.035 * out : 1;
  let body: React.ReactNode = null;
  if (s.shot.kind === 'screen') body = <Screen shot={s.shot} progress={progress} frames={s.frames} enter={enter} />;
  else if (s.shot.kind === 'montage') body = <Montage shot={s.shot} progress={progress} frames={s.frames} />;
  else if (s.shot.kind === 'custom' && s.shot.id === 'desktop') body = <Desktop progress={progress} frames={s.frames} />;
  void chapterFrames;
  return (
    <AbsoluteFill style={{ opacity: fadeIn * out, transform: `scale(${scale})` }}>
      {body}
      {s.shot.kind === 'screen' && s.shot.chip ? (
        <Sequence from={lead + 4} durationInFrames={Math.max(1, s.frames - 8)}>
          <FeatureChip label={s.shot.chip} frames={Math.max(1, s.frames - 8)} />
        </Sequence>
      ) : null}
    </AbsoluteFill>
  );
};

export { FPS };
