import React from 'react';
import { AbsoluteFill, Img, interpolate, staticFile, useCurrentFrame } from 'remotion';
import { Logo } from './Logo';
import { C, FONT, GROUP_TINT, rgba } from '../theme';
import { GLIDE, POP, SNAP, beats, clamp, sp } from '../motion';

const TINT = GROUP_TINT.Everywhere;

const Card: React.FC<{ src: string; label: string; k: number; x: number; y: number; w: number; rot: number; out: number }> = ({ src, label, k, x, y, w, rot, out }) => (
  <div
    style={{
      position: 'absolute',
      left: x,
      top: y,
      width: w,
      opacity: Math.min(1, k * 1.4) * (1 - out),
      transform: `translateY(${(1 - k) * 90 - out * 40}px) scale(${(0.8 + 0.2 * k) * (1 - 0.1 * out)}) rotate(${rot * k}deg)`,
    }}
  >
    <div style={{ borderRadius: 16, overflow: 'hidden', boxShadow: `0 30px 80px rgba(0,0,0,0.55), 0 0 0 1px ${rgba(TINT, 0.4)}, 0 0 50px ${rgba(TINT, 0.2)}` }}>
      <Img src={staticFile(src)} style={{ width: '100%', display: 'block' }} />
    </div>
    <div style={{ font: `750 32px ${FONT}`, color: C.text, marginTop: 16, letterSpacing: -0.5 }}>{label}</div>
  </div>
);

/**
 * Everywhere + call to action, on the bar grid (6 bars): Snip, the Slack and
 * Telegram bridges and the phone view land on successive downbeats under an
 * "Everywhere." headline; on bar 4 (the score's tonic hit) everything clears
 * for the logo lockup, the line and the download call.
 */
export const Outro: React.FC<{ frames: number }> = ({ frames }) => {
  const f = useCurrentFrame();
  const lockAt = beats(12);
  const clear = interpolate(f, [lockAt - 6, lockAt + 2], [0, 1], clamp);
  const headK = sp(f, 0, POP);
  const lk = sp(f, lockAt, POP);
  const lineK = sp(f, lockAt + beats(1), SNAP);
  const ctaK = sp(f, lockAt + beats(2), SNAP);
  const urlK = sp(f, lockAt + beats(3), GLIDE);
  const fadeOut = interpolate(f, [frames - 24, frames], [1, 0], clamp);
  return (
    <AbsoluteFill style={{ opacity: fadeOut }}>
      <AbsoluteFill style={{ opacity: 1 - clear }}>
        <div style={{ position: 'absolute', left: 120, top: 80, opacity: Math.min(1, headK * 1.5), transform: `translateY(${(1 - headK) * 40}px)` }}>
          <div style={{ font: `750 26px ${FONT}`, letterSpacing: 4, color: TINT, textTransform: 'uppercase' }}>Everywhere</div>
          <div style={{ font: `800 104px ${FONT}`, color: C.text, letterSpacing: -4, marginTop: 6 }}>Reach Otto anywhere.</div>
        </div>
        <Card src="capture/snip.jpg" label="Snip and annotate" k={sp(f, beats(1), SNAP)} x={120} y={330} w={660} rot={-3} out={clear} />
        <Card src="capture/channels.jpg" label="Slack & Telegram" k={sp(f, beats(3), SNAP)} x={680} y={560} w={600} rot={2.5} out={clear} />
        {/* phone */}
        <div
          style={{
            position: 'absolute',
            right: 160,
            top: 120,
            width: 400,
            height: 840,
            borderRadius: 62,
            background: '#0d0d10',
            padding: 15,
            boxShadow: `0 40px 100px rgba(0,0,0,0.6), 0 0 0 2px #3a3a42, inset 0 0 0 2px #1d1d22, 0 0 70px ${rgba(TINT, 0.25)}`,
            opacity: Math.min(1, sp(f, beats(5), SNAP) * 1.4) * (1 - clear),
            transform: `translateY(${(1 - sp(f, beats(5), SNAP)) * 120}px) rotate(${3 * sp(f, beats(5), SNAP)}deg)`,
          }}
        >
          <div style={{ width: '100%', height: '100%', borderRadius: 48, overflow: 'hidden', position: 'relative' }}>
            <Img src={staticFile('capture/phone.jpg')} style={{ width: '100%', height: '100%', objectFit: 'cover', objectPosition: 'top' }} />
            <div style={{ position: 'absolute', top: 12, left: '50%', transform: 'translateX(-50%)', width: 116, height: 32, borderRadius: 20, background: '#000' }} />
          </div>
          <div style={{ font: `750 32px ${FONT}`, color: C.text, marginTop: 22, textAlign: 'center', letterSpacing: -0.5 }}>On your phone</div>
        </div>
      </AbsoluteFill>
      {/* lockup */}
      {f >= lockAt - 1 ? (
        <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
          <div style={{ position: 'absolute', width: 1200, height: 800, background: `radial-gradient(closest-side, ${rgba(C.accent, 0.22 * lk)}, transparent)` }} />
          <div style={{ transform: `scale(${1.8 - 0.8 * lk})`, opacity: Math.min(1, lk * 2) }}>
            <Logo size={156} glow={30 * lk} />
          </div>
          <div style={{ font: `800 112px ${FONT}`, color: C.text, letterSpacing: -4.5, marginTop: 12, opacity: lk }}>Otto</div>
          <div style={{ font: `550 40px ${FONT}`, color: '#d2d4de', marginTop: 2, opacity: lineK, transform: `translateY(${(1 - lineK) * 16}px)` }}>All your agents, one home.</div>
          <div
            style={{
              marginTop: 36,
              padding: '17px 36px',
              borderRadius: 999,
              background: C.accent,
              color: '#fff',
              font: `750 32px ${FONT}`,
              boxShadow: `0 16px 44px ${rgba(C.accent, 0.5)}`,
              opacity: Math.min(1, ctaK * 1.5),
              transform: `translateY(${(1 - ctaK) * 24}px) scale(${0.9 + 0.1 * ctaK})`,
            }}
          >
            Download Otto for macOS
          </div>
          <div style={{ font: `500 24px ${FONT}`, color: C.textDim, marginTop: 20, opacity: urlK }}>
            github.com/itzikiusa/otto_os · Help → Walkthroughs for every chapter
          </div>
        </AbsoluteFill>
      ) : null}
    </AbsoluteFill>
  );
};
