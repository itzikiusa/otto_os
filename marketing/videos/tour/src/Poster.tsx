import React from 'react';
import { AbsoluteFill, Img, staticFile } from 'remotion';
import { Backdrop } from './components/Backdrop';
import { Logo } from './components/Logo';
import { C, FONT } from './theme';
import timing from './generated/timing.json';

/** Poster frame for the player (out/otto-tour-poster.jpg). */
export const Poster: React.FC = () => (
  <AbsoluteFill>
    <Backdrop />
    <div
      style={{
        position: 'absolute',
        right: -120,
        top: 150,
        width: 1300,
        height: 731,
        borderRadius: 18,
        overflow: 'hidden',
        transform: 'perspective(2000px) rotateY(-14deg) rotateX(4deg)',
        boxShadow: '0 50px 140px rgba(0,0,0,0.6), 0 0 0 1px rgba(255,255,255,0.14)',
      }}
    >
      <Img src={staticFile('capture/home.jpg')} style={{ width: '100%', height: '100%', objectFit: 'cover' }} />
    </div>
    <div style={{ position: 'absolute', left: 120, top: 300, width: 720 }}>
      <Logo size={120} glow={24} />
      <div style={{ font: `800 120px ${FONT}`, color: C.text, letterSpacing: -5, marginTop: 20 }}>Otto</div>
      <div style={{ font: `600 44px ${FONT}`, color: '#d6d8e2', letterSpacing: -0.8, marginTop: 4 }}>The product tour</div>
      <div style={{ font: `500 28px ${FONT}`, color: C.textDim, marginTop: 18, lineHeight: 1.4 }}>
        Agents, automation, git, design, data and infrastructure — in one native Mac app.
      </div>
      <div
        style={{
          marginTop: 36,
          display: 'inline-flex',
          alignItems: 'center',
          gap: 14,
          padding: '14px 28px',
          borderRadius: 999,
          background: C.accent,
          color: '#fff',
          font: `700 28px ${FONT}`,
        }}
      >
        ▶ Watch · {Math.floor(timing.totalSeconds / 60)}:{String(Math.round(timing.totalSeconds % 60)).padStart(2, '0')}
      </div>
    </div>
  </AbsoluteFill>
);
