import React from 'react';
import { AbsoluteFill, useCurrentFrame } from 'remotion';
import { C } from '../theme';

/** Slow-drifting ambient gradient (the app's own ambient art, dark scheme). */
export const Backdrop: React.FC<{ hue?: number; intensity?: number }> = ({ hue = 0, intensity = 1 }) => {
  const f = useCurrentFrame();
  const a = f / 300;
  const x1 = 30 + Math.sin(a) * 12;
  const y1 = 25 + Math.cos(a * 0.8) * 10;
  const x2 = 75 + Math.cos(a * 0.7) * 12;
  const y2 = 80 + Math.sin(a * 0.9) * 8;
  return (
    <AbsoluteFill style={{ background: C.bgDeep, overflow: 'hidden' }}>
      <AbsoluteFill
        style={{
          filter: `hue-rotate(${hue}deg)`,
          opacity: intensity,
          background: [
            `radial-gradient(60% 70% at ${x1}% ${y1}%, rgba(10,132,255,0.34), transparent 70%)`,
            `radial-gradient(55% 60% at ${x2}% ${y2}%, rgba(124,58,237,0.30), transparent 70%)`,
            `radial-gradient(40% 50% at ${100 - x1}% ${100 - y2 + 40}%, rgba(13,148,136,0.22), transparent 70%)`,
          ].join(','),
        }}
      />
      {/* fine grain so the gradients never band in H.264 */}
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
