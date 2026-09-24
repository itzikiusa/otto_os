import React from 'react';
import { AbsoluteFill, Audio, Easing, Img, Sequence, interpolate, staticFile, useCurrentFrame } from 'remotion';
import { Logo } from './Logo';
import { C, FONT } from '../theme';
import type { PlacedChapter } from '../layout';

const clamp = { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' } as const;

const Card: React.FC<{ src: string; label: string; k: number; x: number; y: number; w: number; rot: number }> = ({ src, label, k, x, y, w, rot }) => (
  <div
    style={{
      position: 'absolute',
      left: x,
      top: y,
      width: w,
      opacity: k,
      transform: `translateY(${(1 - k) * 60}px) rotate(${rot * k}deg)`,
    }}
  >
    <div style={{ borderRadius: 16, overflow: 'hidden', boxShadow: '0 30px 80px rgba(0,0,0,0.55), 0 0 0 1px rgba(255,255,255,0.14)' }}>
      <Img src={staticFile(src)} style={{ width: '100%', display: 'block' }} />
    </div>
    <div style={{ font: `700 30px ${FONT}`, color: C.text, marginTop: 16, letterSpacing: -0.5 }}>{label}</div>
  </div>
);

/**
 * Everywhere + call to action: Snip, Slack/Telegram channels and the phone
 * view fan in, then everything clears for the logo lockup.
 */
export const Outro: React.FC<{ ch: PlacedChapter }> = ({ ch }) => {
  const f = useCurrentFrame();
  const n = ch.frames;
  const lockAt = n - 4.2 * 30;
  const k1 = interpolate(f, [6, 26], [0, 1], { ...clamp, easing: Easing.out(Easing.cubic) });
  const k2 = interpolate(f, [40, 60], [0, 1], { ...clamp, easing: Easing.out(Easing.cubic) });
  const k3 = interpolate(f, [80, 100], [0, 1], { ...clamp, easing: Easing.out(Easing.cubic) });
  const clear = interpolate(f, [lockAt - 16, lockAt], [1, 0], clamp);
  const lk = interpolate(f, [lockAt, lockAt + 22], [0, 1], { ...clamp, easing: Easing.out(Easing.back(1.5)) });
  const ctaK = interpolate(f, [lockAt + 22, lockAt + 40], [0, 1], clamp);
  const fadeOut = interpolate(f, [n - 24, n], [1, 0], clamp);
  return (
    <AbsoluteFill style={{ opacity: fadeOut }}>
      <Sequence from={Math.round(lockAt) - 4} durationInFrames={80}>
        <Audio src={staticFile('audio/sfx-impact.wav')} volume={0.45} />
      </Sequence>
      <AbsoluteFill style={{ opacity: clear }}>
        <Card src="capture/snip.jpg" label="Snip" k={k1} x={90} y={160} w={640} rot={-2.5} />
        <Card src="capture/channels.jpg" label="Slack & Telegram" k={k2} x={520} y={520} w={640} rot={1.5} />
        {/* phone */}
        <div
          style={{
            position: 'absolute',
            right: 170,
            top: 90,
            width: 420,
            height: 880,
            borderRadius: 64,
            background: '#0d0d10',
            padding: 16,
            boxShadow: '0 40px 100px rgba(0,0,0,0.6), 0 0 0 2px #3a3a42, inset 0 0 0 2px #1d1d22',
            opacity: k3,
            transform: `translateY(${(1 - k3) * 80}px) rotate(${3 * k3}deg)`,
          }}
        >
          <div style={{ width: '100%', height: '100%', borderRadius: 50, overflow: 'hidden', position: 'relative' }}>
            <Img src={staticFile('capture/phone.jpg')} style={{ width: '100%', height: '100%', objectFit: 'cover', objectPosition: 'top' }} />
            <div style={{ position: 'absolute', top: 12, left: '50%', transform: 'translateX(-50%)', width: 120, height: 34, borderRadius: 20, background: '#000' }} />
          </div>
          <div style={{ font: `700 30px ${FONT}`, color: C.text, marginTop: 22, textAlign: 'center', letterSpacing: -0.5 }}>On your phone</div>
        </div>
      </AbsoluteFill>
      {/* lockup */}
      <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center', opacity: lk > 0 ? 1 : 0 }}>
        <div style={{ transform: `scale(${0.7 + 0.3 * lk})`, opacity: lk }}>
          <Logo size={150} glow={26 * lk} />
        </div>
        <div style={{ font: `800 104px ${FONT}`, color: C.text, letterSpacing: -4, marginTop: 14, opacity: lk }}>Otto</div>
        <div style={{ font: `500 36px ${FONT}`, color: '#c9cbd6', marginTop: 4, opacity: ctaK }}>All your agents, one home.</div>
        <div
          style={{
            marginTop: 34,
            padding: '16px 34px',
            borderRadius: 999,
            background: C.accent,
            color: '#fff',
            font: `700 30px ${FONT}`,
            boxShadow: '0 16px 40px rgba(10,132,255,0.45)',
            opacity: ctaK,
            transform: `translateY(${(1 - ctaK) * 14}px)`,
          }}
        >
          Download Otto for macOS
        </div>
        <div style={{ font: `500 22px ${FONT}`, color: C.textDim, marginTop: 18, opacity: ctaK }}>
          github.com/itzikiusa/otto_os · Help → Walkthroughs for every chapter
        </div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
};
