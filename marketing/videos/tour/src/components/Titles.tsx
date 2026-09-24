import React from 'react';
import { AbsoluteFill, Easing, interpolate, useCurrentFrame, useVideoConfig } from 'remotion';
import { C, FONT, GROUP_TINT } from '../theme';

const clamp = { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' } as const;

/**
 * Chapter card: a glass lower-third that slides in over the footage at the
 * start of a chapter — sidebar group chip, big title, a short kicker line.
 */
export const ChapterCard: React.FC<{ group: string; title: string; kicker?: string; hold?: number; index: number; total: number }> = ({
  group,
  title,
  kicker,
  hold = 3.4,
  index,
  total,
}) => {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const inK = interpolate(f, [4, 22], [0, 1], { ...clamp, easing: Easing.out(Easing.cubic) });
  const outK = interpolate(f, [hold * fps, hold * fps + 14], [1, 0], { ...clamp, easing: Easing.in(Easing.cubic) });
  const k = Math.min(inK, outK);
  if (k <= 0) return null;
  const tint = GROUP_TINT[group] ?? C.accent;
  const words = title.split(' ');
  return (
    <AbsoluteFill style={{ pointerEvents: 'none' }}>
      <div
        style={{
          position: 'absolute',
          left: 80,
          bottom: 70,
          padding: '22px 30px 24px 26px',
          borderRadius: 22,
          background: 'rgba(16,16,20,0.78)',
          backdropFilter: 'blur(22px) saturate(1.4)',
          border: '1px solid rgba(255,255,255,0.14)',
          boxShadow: '0 30px 80px rgba(0,0,0,0.5)',
          transform: `translateX(${(1 - inK) * -60}px)`,
          opacity: k,
          display: 'flex',
          gap: 22,
          alignItems: 'stretch',
        }}
      >
        <div style={{ width: 5, borderRadius: 3, background: tint, boxShadow: `0 0 18px ${tint}` }} />
        <div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 6 }}>
            <span
              style={{
                font: `700 16px ${FONT}`,
                letterSpacing: 2.2,
                textTransform: 'uppercase',
                color: tint,
              }}
            >
              {group}
            </span>
            <span style={{ font: `500 16px ${FONT}`, color: C.textDim, letterSpacing: 1 }}>
              {String(index).padStart(2, '0')} / {String(total).padStart(2, '0')}
            </span>
          </div>
          <div style={{ font: `700 50px ${FONT}`, color: C.text, letterSpacing: -1.2, lineHeight: 1.05, display: 'flex', gap: 14 }}>
            {words.map((w, i) => {
              const wk = interpolate(f, [8 + i * 4, 24 + i * 4], [0, 1], { ...clamp, easing: Easing.out(Easing.cubic) });
              return (
                <span key={i} style={{ opacity: wk, transform: `translateY(${(1 - wk) * 18}px)`, display: 'inline-block' }}>
                  {w}
                </span>
              );
            })}
          </div>
          {kicker ? (
            <div
              style={{
                font: `500 22px ${FONT}`,
                color: '#c9c9d1',
                marginTop: 8,
                opacity: interpolate(f, [18, 34], [0, 1], clamp),
              }}
            >
              {kicker}
            </div>
          ) : null}
        </div>
      </div>
    </AbsoluteFill>
  );
};

/** A small bottom-center chip naming the feature on screen right now. */
export const FeatureChip: React.FC<{ label: string; frames: number }> = ({ label, frames }) => {
  const f = useCurrentFrame();
  const k = Math.min(
    interpolate(f, [2, 14], [0, 1], clamp),
    interpolate(f, [frames - 10, frames - 1], [1, 0], clamp),
  );
  if (k <= 0) return null;
  return (
    <AbsoluteFill style={{ pointerEvents: 'none', alignItems: 'center' }}>
      <div
        style={{
          position: 'absolute',
          bottom: 30,
          padding: '9px 20px',
          borderRadius: 999,
          background: 'rgba(10,132,255,0.92)',
          color: '#fff',
          font: `650 21px ${FONT}`,
          letterSpacing: 0.2,
          boxShadow: '0 10px 30px rgba(10,132,255,0.45)',
          opacity: k,
          transform: `translateY(${(1 - k) * 16}px)`,
        }}
      >
        {label}
      </div>
    </AbsoluteFill>
  );
};
