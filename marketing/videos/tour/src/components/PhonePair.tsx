import React from 'react';
import { AbsoluteFill, Img, staticFile, useCurrentFrame } from 'remotion';
import { C, FONT, GROUP_TINT, rgba } from '../theme';
import { SNAP, beats, sp } from '../motion';

const TINT = GROUP_TINT.Everywhere;

const Phone: React.FC<{ src: string; label: string; step: string; k: number; x: number; rot: number }> = ({ src, label, step, k, x, rot }) => (
  <div
    style={{
      position: 'absolute',
      left: x,
      top: 150,
      width: 360,
      opacity: Math.min(1, k * 1.4),
      transform: `translateY(${(1 - k) * 120}px) rotate(${rot * k}deg)`,
    }}
  >
    <div
      style={{
        height: 760,
        borderRadius: 58,
        background: '#0d0d10',
        padding: 14,
        boxShadow: `0 40px 100px rgba(0,0,0,0.6), 0 0 0 2px #3a3a42, inset 0 0 0 2px #1d1d22, 0 0 70px ${rgba(TINT, 0.25)}`,
      }}
    >
      <div style={{ width: '100%', height: '100%', borderRadius: 45, overflow: 'hidden', position: 'relative' }}>
        <Img src={staticFile(src)} style={{ width: '100%', height: '100%', objectFit: 'cover', objectPosition: 'top' }} />
        <div style={{ position: 'absolute', top: 11, left: '50%', transform: 'translateX(-50%)', width: 104, height: 30, borderRadius: 18, background: '#000' }} />
      </div>
    </div>
    <div style={{ display: 'flex', alignItems: 'baseline', gap: 12, justifyContent: 'center', marginTop: 22 }}>
      <span style={{ font: `800 26px ${FONT}`, color: TINT }}>{step}</span>
      <span style={{ font: `700 28px ${FONT}`, color: C.text, letterSpacing: -0.4 }}>{label}</span>
    </div>
  </div>
);

/**
 * Phone access, the phone's side of pairing (captured through the fictional
 * tailnet in scripts/lib/stack.mjs): the comparison code, then the same
 * thread opened on the phone, signed in. Lands on successive downbeats.
 */
export const PhonePair: React.FC<{ frames: number }> = () => {
  const f = useCurrentFrame();
  const chip = sp(f, beats(4), SNAP);
  return (
    <AbsoluteFill>
      <Phone src="capture/phone-pair-code.jpg" step="1" label="The same code on both screens" k={sp(f, beats(0.25), SNAP)} x={430} rot={-2.5} />
      <Phone src="capture/phone-thread.jpg" step="2" label="Your thread, signed in" k={sp(f, beats(2), SNAP)} x={1130} rot={2.5} />
      <div
        style={{
          position: 'absolute',
          left: '50%',
          top: 470,
          transform: `translateX(-50%) scale(${0.85 + 0.15 * chip})`,
          opacity: Math.min(1, chip * 1.5),
          padding: '14px 26px',
          borderRadius: 999,
          background: rgba(C.bg, 0.85),
          border: `1px solid ${rgba(TINT, 0.55)}`,
          boxShadow: `0 0 40px ${rgba(TINT, 0.3)}`,
          font: `700 24px ${FONT}`,
          color: C.text,
          whiteSpace: 'nowrap',
        }}
      >
        Tailnet only · never the public internet
      </div>
    </AbsoluteFill>
  );
};

/**
 * The opt-in light version on a phone (captured in a phone viewport): the
 * five-tab Otto, then one of its tabs. Lands on successive downbeats.
 */
export const PhoneLight: React.FC<{ frames: number }> = () => {
  const f = useCurrentFrame();
  const chip = sp(f, beats(2.5), SNAP);
  return (
    <AbsoluteFill>
      <Phone src="capture/phone-light.jpg" step="1" label="Five tabs, opening on Threads" k={sp(f, beats(0.25), SNAP)} x={430} rot={-2.5} />
      <Phone src="capture/phone-light-usage.jpg" step="2" label="Usage, a tap away" k={sp(f, beats(1.25), SNAP)} x={1130} rot={2.5} />
      <div
        style={{
          position: 'absolute',
          left: '50%',
          top: 470,
          transform: `translateX(-50%) scale(${0.85 + 0.15 * chip})`,
          opacity: Math.min(1, chip * 1.5),
          padding: '14px 26px',
          borderRadius: 999,
          background: rgba(C.bg, 0.85),
          border: `1px solid ${rgba(TINT, 0.55)}`,
          boxShadow: `0 0 40px ${rgba(TINT, 0.3)}`,
          font: `700 24px ${FONT}`,
          color: C.text,
          whiteSpace: 'nowrap',
        }}
      >
        Opt-in · one tap back to the full app
      </div>
    </AbsoluteFill>
  );
};
