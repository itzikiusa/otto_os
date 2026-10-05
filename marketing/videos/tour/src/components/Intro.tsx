import React from 'react';
import { AbsoluteFill, Img, interpolate, staticFile, useCurrentFrame } from 'remotion';
import { Logo } from './Logo';
import { BEAT } from '../layout';
import { C, FONT, rgba } from '../theme';
import { GLIDE, POP, SNAP, beats, clamp, easeIn, sp } from '../motion';

/** The four claims of the cold open, one every two beats, each with its screen. */
const CLAIMS = [
  { word: 'agents.', tint: C.accent, src: 'capture/agents-tiled.jpg', x: 110, y: 110, rot: -6 },
  { word: 'repos.', tint: C.green, src: 'capture/git.jpg', x: 1190, y: 130, rot: 5 },
  { word: 'data.', tint: C.amber, src: 'capture/db-results.jpg', x: 150, y: 640, rot: 4 },
  { word: 'infrastructure.', tint: C.violet, src: 'capture/kubernetes.jpg', x: 1150, y: 620, rot: -5 },
];

/** Stills that rush past behind the logo — a wall of the real app. */
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
 * Cold open on the bar grid (4 bars): "Your agents. Your repos. Your data.
 * Your infrastructure." lands a word every two beats while its screen pops in
 * around it; on bar 3 the screens fly into a rushing wall and "One home."; on
 * bar 4 the logo slams in with the name and promise; on the last beat the
 * camera zooms through the logo into the first chapter, on the drop.
 */
export const Intro: React.FC<{ frames: number }> = ({ frames }) => {
  const f = useCurrentFrame();
  const wallAt = beats(8);
  const logoAt = beats(12);
  const outAt = frames - beats(1);
  const scatter = sp(f, wallAt, GLIDE); // the four cards clear for the wall
  const wallK = interpolate(f, [wallAt - 4, frames], [0, 1], clamp);
  const through = interpolate(f, [outAt, frames], [0, 1], { ...clamp, easing: easeIn });
  const claimIdx = Math.min(3, Math.floor(f / beats(2)));
  return (
    <AbsoluteFill style={{ overflow: 'hidden' }}>
      {/* the wall */}
      {f >= wallAt - 4 ? (
        <AbsoluteFill style={{ perspective: 1500, opacity: interpolate(f, [wallAt - 4, wallAt + 6], [0, 0.85], clamp) * (1 - through) }}>
          <div
            style={{
              position: 'absolute',
              left: '50%',
              top: '50%',
              width: 3 * 760 + 2 * 40,
              display: 'grid',
              gridTemplateColumns: 'repeat(3, 760px)',
              gap: 40,
              transform: `translate(-50%, -50%) translateZ(${-1700 + 1150 * Math.pow(wallK, 0.8)}px) rotateX(26deg) rotateZ(-7deg) translateY(${interpolate(wallK, [0, 1], [180, -220])}px)`,
            }}
          >
            {WALL.map((src, i) => (
              <div key={i} style={{ width: 760, height: 428, borderRadius: 14, overflow: 'hidden', boxShadow: '0 30px 80px rgba(0,0,0,0.6)', border: '1px solid rgba(255,255,255,0.14)' }}>
                <Img src={staticFile(src)} style={{ width: '100%', height: '100%', objectFit: 'cover' }} />
              </div>
            ))}
          </div>
          <AbsoluteFill style={{ background: 'radial-gradient(55% 55% at 50% 50%, rgba(8,8,12,0.55), rgba(8,8,12,0.92))' }} />
        </AbsoluteFill>
      ) : null}
      {/* the four claim cards */}
      {CLAIMS.map((c, i) => {
        const at = beats(2 * i);
        if (f < at) return null;
        const k = sp(f, at, SNAP);
        const out = scatter;
        const dx = (c.x + 310 - 960) * out * 1.4;
        const dy = (c.y + 175 - 540) * out * 1.4;
        return (
          <div
            key={i}
            style={{
              position: 'absolute',
              left: c.x,
              top: c.y,
              width: 620,
              height: 349,
              borderRadius: 16,
              overflow: 'hidden',
              border: `1px solid ${rgba(c.tint, 0.55)}`,
              boxShadow: `0 30px 80px rgba(0,0,0,0.6), 0 0 60px ${rgba(c.tint, 0.3)}`,
              opacity: Math.min(1, k * 1.3) * (1 - out),
              transform: `translate(${dx}px, ${dy}px) scale(${0.55 + 0.45 * k}) rotate(${c.rot * k}deg)`,
            }}
          >
            <Img src={staticFile(c.src)} style={{ width: '100%', height: '100%', objectFit: 'cover' }} />
          </div>
        );
      })}
      {/* "Your ___." */}
      {f < wallAt ? (
        <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
          <div style={{ position: 'absolute', width: 1300, height: 420, background: 'radial-gradient(closest-side, rgba(6,6,10,0.85), rgba(6,6,10,0))' }} />
          {CLAIMS.map((c, i) => {
            if (i !== claimIdx) return null;
            const at = beats(2 * i);
            const k = sp(f, at, POP);
            const leave = interpolate(f, [at + beats(2) - 4, at + beats(2)], [0, 1], clamp);
            return (
              <div
                key={i}
                style={{
                  font: `800 132px ${FONT}`,
                  letterSpacing: -5,
                  color: C.text,
                  whiteSpace: 'nowrap',
                  opacity: Math.min(1, k * 1.5) * (1 - leave),
                  transform: `translateY(${(1 - k) * 50 - leave * 40}px) scale(${1.25 - 0.25 * k})`,
                }}
              >
                <span style={{ color: '#cfd0da' }}>Your </span>
                <span style={{ color: c.tint, textShadow: `0 0 40px ${rgba(c.tint, 0.55)}` }}>{c.word}</span>
              </div>
            );
          })}
        </AbsoluteFill>
      ) : null}
      {/* "One home." */}
      {f >= wallAt && f < logoAt ? (
        <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
          <div
            style={{
              font: `800 150px ${FONT}`,
              letterSpacing: -6,
              color: C.text,
              opacity: Math.min(1, sp(f, wallAt, POP) * 1.5) * interpolate(f, [logoAt - 4, logoAt], [1, 0], clamp),
              transform: `scale(${1.3 - 0.3 * sp(f, wallAt, POP)})`,
              textShadow: '0 10px 60px rgba(0,0,0,0.8)',
            }}
          >
            One home.
          </div>
        </AbsoluteFill>
      ) : null}
      {/* the lockup, then the zoom through it */}
      {f >= logoAt ? <Lockup f={f - logoAt} through={through} /> : null}
      {/* a tinted bloom as we pass through the logo (no white flash) */}
      <AbsoluteFill style={{ background: `radial-gradient(circle at 50% 46%, ${rgba(C.accent, 0.45)}, transparent 60%)`, opacity: Math.sin(through * Math.PI) * 0.8 }} />
    </AbsoluteFill>
  );
};

const Lockup: React.FC<{ f: number; through: number }> = ({ f, through }) => {
  const logoK = sp(f, 0, POP);
  const nameK = sp(f, Math.round(BEAT * 0.5), POP);
  const tagK = sp(f, BEAT, SNAP);
  return (
    <AbsoluteFill
      style={{
        alignItems: 'center',
        justifyContent: 'center',
        transform: `scale(${1 + 2.6 * through * through})`,
        transformOrigin: '50% 40%',
        opacity: 1 - interpolate(through, [0.55, 1], [0, 1], clamp),
      }}
    >
      <div style={{ transform: `scale(${2.2 - 1.2 * logoK})`, opacity: Math.min(1, logoK * 2) }}>
        <Logo size={180} glow={34 * logoK} />
      </div>
      <div style={{ font: `800 140px ${FONT}`, color: C.text, letterSpacing: -6, marginTop: 16, opacity: nameK, transform: `translateY(${(1 - nameK) * 30}px)` }}>Otto</div>
      <div style={{ font: `550 42px ${FONT}`, color: '#d2d4de', letterSpacing: -0.6, marginTop: 4, opacity: tagK, transform: `translateY(${(1 - tagK) * 18}px)` }}>
        Every coding agent. One native Mac app.
      </div>
    </AbsoluteFill>
  );
};
