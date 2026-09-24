import React from 'react';
import { AbsoluteFill, Audio, Easing, Img, Sequence, interpolate, staticFile, useCurrentFrame } from 'remotion';
import { Logo } from './Logo';
import { C, FONT } from '../theme';

const clamp = { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' } as const;

/** Stills that fly past behind the title — a wall of the real app. */
export const WALL = [
  'capture/home.jpg',
  'capture/agents-tiled.jpg',
  'capture/git.jpg',
  'capture/design.jpg',
  'capture/db-builder.jpg',
  'capture/workflows.jpg',
  'capture/swarm.jpg',
  'capture/vault-graph.jpg',
  'capture/mission-control.jpg',
];

/**
 * Cold open: a tilted wall of real screens drifts in the dark, the logo lands
 * with a hit, the name and the promise type on, then the wall rushes forward
 * into the first chapter.
 */
export const Intro: React.FC<{ frames: number }> = ({ frames }) => {
  const f = useCurrentFrame();
  const logoK = interpolate(f, [34, 58], [0, 1], { ...clamp, easing: Easing.out(Easing.back(1.6)) });
  const titleK = interpolate(f, [58, 78], [0, 1], { ...clamp, easing: Easing.out(Easing.cubic) });
  const tagK = interpolate(f, [80, 100], [0, 1], { ...clamp, easing: Easing.out(Easing.cubic) });
  const outK = interpolate(f, [frames - 26, frames], [0, 1], { ...clamp, easing: Easing.in(Easing.cubic) });
  const wallZ = interpolate(f, [0, frames], [-900, -520]) + outK * 900;
  const wallO = interpolate(f, [0, 24], [0, 0.55], clamp) * (1 - outK * 0.6);
  return (
    <AbsoluteFill>
      <Sequence from={10} durationInFrames={70}>
        <Audio src={staticFile('audio/sfx-riser.wav')} volume={0.35} />
      </Sequence>
      <Sequence from={52} durationInFrames={80}>
        <Audio src={staticFile('audio/sfx-impact.wav')} volume={0.55} />
      </Sequence>
      {/* the wall */}
      <AbsoluteFill style={{ perspective: 1600, overflow: 'hidden' }}>
        <div
          style={{
            position: 'absolute',
            left: '50%',
            top: '50%',
            width: 3 * 760 + 2 * 40,
            transform: `translate(-50%, -50%) translateZ(${wallZ}px) rotateX(28deg) rotateZ(-8deg) translateY(${interpolate(f, [0, frames], [60, -140])}px)`,
            display: 'grid',
            gridTemplateColumns: 'repeat(3, 760px)',
            gap: 40,
            opacity: wallO,
          }}
        >
          {WALL.map((src, i) => (
            <div key={i} style={{ width: 760, height: 428, borderRadius: 14, overflow: 'hidden', boxShadow: '0 30px 80px rgba(0,0,0,0.6)', border: '1px solid rgba(255,255,255,0.12)' }}>
              <Img src={staticFile(src)} style={{ width: '100%', height: '100%', objectFit: 'cover' }} />
            </div>
          ))}
        </div>
        <AbsoluteFill style={{ background: 'radial-gradient(60% 60% at 50% 50%, rgba(11,11,14,0.2), rgba(11,11,14,0.92))' }} />
      </AbsoluteFill>
      {/* the lockup */}
      <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center', opacity: 1 - outK }}>
        <div style={{ transform: `scale(${0.6 + 0.4 * logoK}) translateY(${(1 - logoK) * 30}px)`, opacity: logoK }}>
          <Logo size={176} glow={30 * logoK} />
        </div>
        <div
          style={{
            font: `800 132px ${FONT}`,
            color: C.text,
            letterSpacing: -5,
            marginTop: 18,
            opacity: titleK,
            transform: `translateY(${(1 - titleK) * 24}px)`,
          }}
        >
          Otto
        </div>
        <div
          style={{
            font: `500 40px ${FONT}`,
            color: '#c9cbd6',
            letterSpacing: -0.4,
            marginTop: 6,
            opacity: tagK,
            transform: `translateY(${(1 - tagK) * 18}px)`,
          }}
        >
          Every coding agent. One native Mac home.
        </div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
};
