import React from 'react';
import { AbsoluteFill, interpolate, useCurrentFrame } from 'remotion';
import { CHAPTERS } from '../layout';
import { C, GROUP_GLOW, GROUP_TINT, mix, rgba } from '../theme';
import { beatPulse, clamp } from '../motion';

const ENERGETIC = new Set(['chorus', 'final']);

/**
 * The animated brand backdrop: three drifting light fields in the current
 * chapter's group colours (cross-fading at each chapter start), a slow aurora,
 * a soft shock-ring on every chapter downbeat and a gentle on-beat lift during
 * the choruses. Fine grain keeps the gradients from banding in H.264.
 */
export const Backdrop: React.FC<{ frame?: number; group?: string }> = ({ frame, group }) => {
  const cur = useCurrentFrame();
  const f = frame ?? cur;
  const idx = Math.max(0, CHAPTERS.findIndex((c) => f >= c.from && f < c.from + c.frames));
  const ch = CHAPTERS[idx];
  const prev = CHAPTERS[Math.max(0, idx - 1)];
  const g = group ?? ch.spec.group;
  const k = group ? 1 : interpolate(f - ch.from, [0, 16], [0, 1], clamp);
  const pick = (grp: string) => [GROUP_TINT[grp] ?? C.accent, ...(GROUP_GLOW[grp] ?? [C.violet, C.cyan])];
  const [a0, b0, c0] = pick(group ?? prev.spec.group);
  const [a1, b1, c1] = pick(g);
  const ca = mix(a0, a1, k);
  const cb = mix(b0, b1, k);
  const cc = mix(c0, c1, k);
  // Music section → energy: the backdrop lifts with the beat only in choruses.
  const section = [...CHAPTERS].reverse().find((c) => c.music && c.from <= f)?.music ?? 'intro';
  const lift = ENERGETIC.has(section) ? 0.07 * beatPulse(f, 6) : 0;
  const t = f / 30;
  const x1 = 28 + Math.sin(t * 0.21) * 14;
  const y1 = 30 + Math.cos(t * 0.17) * 12;
  const x2 = 74 + Math.cos(t * 0.15) * 14;
  const y2 = 72 + Math.sin(t * 0.19) * 10;
  const x3 = 55 + Math.sin(t * 0.11 + 2) * 22;
  const ringP = interpolate(f - ch.from, [0, 22], [0, 1], clamp);
  const ringO = ch.from > 0 ? (1 - ringP) * 0.5 : 0;
  return (
    <AbsoluteFill style={{ background: C.bgDeep, overflow: 'hidden' }}>
      <AbsoluteFill
        style={{
          opacity: 0.9 + lift,
          background: [
            `radial-gradient(55% 65% at ${x1}% ${y1}%, ${rgba(ca, 0.58 + lift)}, transparent 70%)`,
            `radial-gradient(50% 60% at ${x2}% ${y2}%, ${rgba(cb, 0.5 + lift)}, transparent 70%)`,
            `radial-gradient(40% 45% at ${x3}% 100%, ${rgba(cc, 0.4)}, transparent 72%)`,
          ].join(','),
        }}
      />
      {/* aurora: a slow conic sweep of the same colours */}
      <AbsoluteFill
        style={{
          opacity: 0.32,
          mixBlendMode: 'screen',
          background: `conic-gradient(from ${t * 9}deg at 50% 120%, transparent 0deg, ${rgba(ca, 0.5)} 40deg, transparent 90deg, ${rgba(cb, 0.45)} 150deg, transparent 210deg, ${rgba(cc, 0.4)} 270deg, transparent 330deg)`,
          filter: 'blur(40px)',
        }}
      />
      {/* the chapter downbeat: a soft ring of light expanding from the centre */}
      {ringO > 0.01 ? (
        <AbsoluteFill
          style={{
            background: `radial-gradient(circle at 50% 55%, transparent ${ringP * 60}%, ${rgba(ca, ringO)} ${ringP * 60 + 6}%, transparent ${ringP * 60 + 16}%)`,
          }}
        />
      ) : null}
      {/* vignette */}
      <AbsoluteFill style={{ background: 'radial-gradient(120% 90% at 50% 50%, transparent 60%, rgba(0,0,0,0.45) 100%)' }} />
      <AbsoluteFill
        style={{
          opacity: 0.07,
          mixBlendMode: 'overlay',
          backgroundImage:
            "url(\"data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='160' height='160'><filter id='n'><feTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='2' stitchTiles='stitch'/></filter><rect width='100%' height='100%' filter='url(%23n)'/></svg>\")",
        }}
      />
    </AbsoluteFill>
  );
};
