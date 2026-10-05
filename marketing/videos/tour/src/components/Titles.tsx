import React from 'react';
import { AbsoluteFill, interpolate, useCurrentFrame } from 'remotion';
import { CHAPTERS, NUMBERED, type PlacedChapter } from '../layout';
import { C, FONT, GROUP_TINT, rgba } from '../theme';
import { GLIDE, POP, beatPulse, clamp, easeOut, sp } from '../motion';
import { WIN, WIN_LEFT } from './Screen';

/** Frames the hero title holds before it docks into the header band. */
const HERO = 22;

/**
 * Kinetic chapter title. On the downbeat the group and title slam in big over
 * a scrim (each word rises out of a mask, staggered); after ~1.5 beats it
 * docks into the header band above the window, where it stays with a second
 * line that follows the shot on screen.
 */
export const ChapterHeader: React.FC<{ ch: PlacedChapter }> = ({ ch }) => {
  const f = useCurrentFrame(); // frames since the chapter's start
  const tint = GROUP_TINT[ch.spec.group] ?? C.accent;
  const dock = sp(f, HERO, GLIDE);
  const exit = interpolate(f, [ch.frames - 5, ch.frames], [1, 0], clamp);
  const words = ch.spec.title.split(' ');
  // The hero block flies toward the band and hands over to the docked header.
  const heroO = interpolate(dock, [0, 0.55, 0.8], [1, 1, 0], clamp);
  const heroX = interpolate(dock, [0, 1], [0, WIN_LEFT - 128]);
  const heroY = interpolate(dock, [0, 1], [0, -390]);
  const heroS = interpolate(dock, [0, 1], [1, 0.32]);
  const dockO = interpolate(dock, [0.55, 0.9], [0, 1], clamp) * exit;
  // The line under the docked title: the current shot's label, else the kicker.
  const abs = ch.from + f;
  const shot = ch.shots.find((s) => abs >= s.from && abs < s.from + s.frames) ?? ch.shots[0];
  const label = shot.shot.label ?? ch.spec.kicker ?? '';
  const labelF = abs - (shot.shot.label ? shot.from : ch.from);
  const lk = shot.index === 0 ? dockO : sp(labelF, 0, POP) * exit;
  return (
    <AbsoluteFill style={{ pointerEvents: 'none' }}>
      {/* scrim behind the hero so the title reads over any footage */}
      <AbsoluteFill
        style={{
          background: `linear-gradient(90deg, rgba(6,6,10,0.9) 0%, rgba(6,6,10,0.72) 45%, rgba(6,6,10,0) 80%)`,
          opacity: interpolate(f, [0, 2], [0, 1], clamp) * interpolate(dock, [0, 0.7], [1, 0], clamp),
        }}
      />
      {heroO > 0 ? (
        <div
          style={{
            position: 'absolute',
            left: 128,
            top: 360,
            width: 1400,
            transformOrigin: '0 0',
            transform: `translate(${heroX}px, ${heroY}px) scale(${heroS})`,
            opacity: heroO,
          }}
        >
          <GroupTag group={ch.spec.group} tint={tint} size={28} k={sp(f, 0, POP)} number={ch.number} />
          <div style={{ display: 'flex', flexWrap: 'wrap', columnGap: 30, rowGap: 0, marginTop: 14 }}>
            {words.map((w, i) => {
              const k = sp(f, 1 + i * 2, POP);
              return (
                <span key={i} style={{ display: 'inline-block', overflow: 'hidden', paddingBottom: 10 }}>
                  <span
                    style={{
                      display: 'inline-block',
                      font: `800 118px ${FONT}`,
                      letterSpacing: -4.5,
                      lineHeight: 1.02,
                      color: C.text,
                      transform: `translateY(${(1 - k) * 110}%)`,
                    }}
                  >
                    {w}
                  </span>
                </span>
              );
            })}
          </div>
          {ch.spec.kicker ? (
            <div
              style={{
                font: `550 38px ${FONT}`,
                color: '#d4d4de',
                marginTop: 18,
                letterSpacing: -0.4,
                opacity: interpolate(f, [6, 12], [0, 1], clamp),
                transform: `translateY(${(1 - easeOut(interpolate(f, [6, 14], [0, 1], clamp))) * 20}px)`,
              }}
            >
              {ch.spec.kicker}
            </div>
          ) : null}
        </div>
      ) : null}
      {/* docked header, above the window */}
      <div style={{ position: 'absolute', left: WIN_LEFT, top: 22, height: WIN.top - 30, display: 'flex', flexDirection: 'column', justifyContent: 'center', opacity: dockO }}>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 16 }}>
          <GroupTag group={ch.spec.group} tint={tint} size={15} k={1} />
          <span style={{ font: `750 34px ${FONT}`, color: C.text, letterSpacing: -0.9 }}>{ch.spec.title}</span>
        </div>
        <div style={{ height: 30, overflow: 'hidden', marginTop: 2 }}>
          <div
            key={shot.index}
            style={{ font: `500 22px ${FONT}`, color: '#c4c4ce', letterSpacing: -0.1, transform: `translateY(${(1 - lk) * 26}px)`, opacity: lk }}
          >
            {label}
          </div>
        </div>
      </div>
    </AbsoluteFill>
  );
};

const GroupTag: React.FC<{ group: string; tint: string; size: number; k: number; number?: number }> = ({ group, tint, size, k, number }) => (
  <span style={{ display: 'inline-flex', alignItems: 'center', gap: size * 0.6, opacity: k, transform: `translateX(${(1 - k) * -30}px)` }}>
    <span style={{ width: size * 0.62, height: size * 0.62, borderRadius: size, background: tint, boxShadow: `0 0 ${size}px ${rgba(tint, 0.9)}` }} />
    <span style={{ font: `750 ${size}px ${FONT}`, letterSpacing: size * 0.16, textTransform: 'uppercase', color: tint }}>{group}</span>
    {number ? (
      <span style={{ font: `600 ${size}px ${FONT}`, letterSpacing: size * 0.08, color: C.textDim }}>
        {String(number).padStart(2, '0')} / {String(NUMBERED.length).padStart(2, '0')}
      </span>
    ) : null}
  </span>
);

/**
 * The chapter rail, top right: one segment per numbered chapter (sized by its
 * length), done ones in their group colour, the current one filling in time
 * with a beat-synced glow.
 */
export const ProgressRail: React.FC = () => {
  const f = useCurrentFrame(); // absolute
  const numbered = CHAPTERS.filter((c) => c.number > 0);
  const first = numbered[0];
  const last = numbered[numbered.length - 1];
  const o = Math.min(
    interpolate(f, [first.from + HERO, first.from + HERO + 12], [0, 1], clamp),
    interpolate(f, [last.from + last.frames - 8, last.from + last.frames], [1, 0], clamp),
  );
  if (o <= 0) return null;
  const total = numbered.reduce((s, c) => s + c.frames, 0);
  const W = 470;
  const gap = 5;
  const cur = numbered.find((c) => f >= c.from && f < c.from + c.frames);
  return (
    <div style={{ position: 'absolute', right: WIN_LEFT, top: 50, width: W, opacity: o }}>
      <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: 10, font: `650 16px ${FONT}`, letterSpacing: 1.5, color: C.textDim }}>
        {cur ? (
          <span>
            <span style={{ color: C.text }}>{String(cur.number).padStart(2, '0')}</span> / {String(numbered.length).padStart(2, '0')}
          </span>
        ) : null}
      </div>
      <div style={{ display: 'flex', gap }}>
        {numbered.map((c) => {
          const tint = GROUP_TINT[c.spec.group] ?? C.accent;
          const w = ((W - gap * (numbered.length - 1)) * c.frames) / total;
          const p = interpolate(f, [c.from, c.from + c.frames], [0, 1], clamp);
          const active = c === cur;
          const glow = active ? 0.45 + 0.4 * beatPulse(f) : 0;
          return (
            <div key={c.id} style={{ width: w, height: 6, borderRadius: 3, background: 'rgba(255,255,255,0.14)', overflow: 'hidden', boxShadow: active ? `0 0 14px ${rgba(tint, glow)}` : undefined }}>
              <div style={{ width: `${p * 100}%`, height: '100%', background: tint, opacity: active ? 1 : 0.85 }} />
            </div>
          );
        })}
      </div>
    </div>
  );
};
