import React from 'react';
import { AbsoluteFill, Easing, Img, OffthreadVideo, interpolate, staticFile, useCurrentFrame, useVideoConfig } from 'remotion';
import type { CamKey, Callout, ScreenShot, Spot } from '../types';
import { C, FONT } from '../theme';

/** Window geometry inside the 1920×1080 frame (16:9 content + title bar). */
export const WIN = { w: 1664, h: 936, bar: 30 };

const ease = Easing.bezier(0.45, 0, 0.2, 1);

/** Camera at progress p: piecewise eased interpolation between keys. */
export function camAt(keys: CamKey[] | undefined, p: number): CamKey {
  const ks = keys?.length ? keys : [{ t: 0, x: 0.5, y: 0.5, z: 1 }];
  if (p <= ks[0].t) return ks[0];
  for (let i = 0; i < ks.length - 1; i++) {
    const a = ks[i];
    const b = ks[i + 1];
    if (p <= b.t) {
      const k = ease((p - a.t) / Math.max(1e-6, b.t - a.t));
      return { t: p, x: a.x + (b.x - a.x) * k, y: a.y + (b.y - a.y) * k, z: a.z + (b.z - a.z) * k };
    }
  }
  return ks[ks.length - 1];
}

/** Clamp the focus so a zoomed view never shows past the footage edge. */
function clampCam(c: CamKey): CamKey {
  const half = 0.5 / c.z;
  return { ...c, x: Math.min(1 - half, Math.max(half, c.x)), y: Math.min(1 - half, Math.max(half, c.y)) };
}

const isVideo = (src: string) => src.endsWith('.mp4') || src.endsWith('.webm');

/**
 * A macOS window showing footage with a moving camera, spotlights and pinned
 * callouts. `progress` is the shot's 0..1 timeline; `enter` 0..1 drives the
 * window's arrival.
 */
export const Screen: React.FC<{ shot: ScreenShot; progress: number; frames: number; enter?: number }> = ({
  shot,
  progress,
  frames,
  enter = 1,
}) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const cam = clampCam(camAt(shot.cam, progress));
  const cw = WIN.w;
  const ch = WIN.h;
  // content transform: focus point → window center, then zoom.
  const tx = (0.5 - cam.x) * cw * cam.z;
  const ty = (0.5 - cam.y) * ch * cam.z;
  const toScreen = (x: number, y: number) => ({
    left: cw / 2 + (x - cam.x) * cw * cam.z,
    top: ch / 2 + (y - cam.y) * ch * cam.z,
  });
  const lift = interpolate(enter, [0, 1], [60, 0]);
  const tilt = interpolate(enter, [0, 1], [10, 0]);
  const wipe =
    shot.wipeTo != null ? interpolate(progress, [0.42, 0.62], [0, 1], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp', easing: ease }) : 0;

  const media = (src: string) =>
    isVideo(src) ? (
      <OffthreadVideo
        src={staticFile(src)}
        muted
        startFrom={Math.round((shot.from ?? 0) * fps)}
        playbackRate={shot.rate ?? 1}
        style={{ width: cw, height: ch, display: 'block' }}
      />
    ) : (
      <Img src={staticFile(src)} style={{ width: cw, height: ch, display: 'block' }} />
    );

  return (
    <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center', perspective: 2400 }}>
      <div
        style={{
          width: cw,
          height: ch + WIN.bar,
          borderRadius: 14,
          overflow: 'hidden',
          background: C.surface,
          boxShadow: '0 40px 120px rgba(0,0,0,0.55), 0 0 0 1px rgba(255,255,255,0.10)',
          transform: `translateY(${lift}px) rotateX(${tilt}deg)`,
          opacity: Math.min(1, enter * 1.4),
          position: 'relative',
        }}
      >
        <TitleBar />
        <div style={{ position: 'relative', width: cw, height: ch, overflow: 'hidden' }}>
          <div
            style={{
              position: 'absolute',
              width: cw,
              height: ch,
              transformOrigin: '50% 50%',
              transform: `translate(${tx}px, ${ty}px) scale(${cam.z})`,
            }}
          >
            {media(shot.src)}
            {shot.wipeTo ? (
              <div style={{ position: 'absolute', inset: 0, clipPath: `inset(0 0 0 ${(1 - wipe) * 100}%)` }}>{media(shot.wipeTo)}</div>
            ) : null}
          </div>
          {shot.wipeTo && wipe > 0 && wipe < 1 ? (
            <div
              style={{
                position: 'absolute',
                top: 0,
                bottom: 0,
                left: (1 - wipe) * cw - 1,
                width: 2,
                background: 'rgba(255,255,255,0.9)',
                boxShadow: '0 0 24px rgba(10,132,255,0.9)',
              }}
            />
          ) : null}
          {(shot.spots ?? []).map((s, i) => (
            <Spotlight key={i} spot={s} progress={progress} toScreen={toScreen} zoom={cam.z} />
          ))}
          {(shot.callouts ?? []).map((c, i) => (
            <CalloutPin key={i} c={c} progress={progress} frames={frames} toScreen={toScreen} frame={frame} />
          ))}
        </div>
      </div>
    </AbsoluteFill>
  );
};

const TitleBar: React.FC = () => (
  <div
    style={{
      height: WIN.bar,
      background: 'linear-gradient(#2c2c31, #26262b)',
      borderBottom: '1px solid rgba(0,0,0,0.5)',
      display: 'flex',
      alignItems: 'center',
      paddingLeft: 13,
      gap: 8,
      position: 'relative',
    }}
  >
    {['#ff5f57', '#febc2e', '#28c840'].map((c) => (
      <div key={c} style={{ width: 12, height: 12, borderRadius: 6, background: c, boxShadow: 'inset 0 0 0 0.5px rgba(0,0,0,0.25)' }} />
    ))}
    <div style={{ position: 'absolute', left: 0, right: 0, textAlign: 'center', font: `600 13px ${FONT}`, color: '#b5b5bd' }}>Otto</div>
  </div>
);

const fade = (p: number, t0: number, t1: number, edge = 0.05) =>
  interpolate(p, [t0, t0 + edge, t1 - edge, t1], [0, 1, 1, 0], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' });

const Spotlight: React.FC<{
  spot: Spot;
  progress: number;
  zoom: number;
  toScreen: (x: number, y: number) => { left: number; top: number };
}> = ({ spot, progress, toScreen, zoom }) => {
  const o = fade(progress, spot.t0, spot.t1, 0.06);
  if (o <= 0) return null;
  const a = toScreen(spot.x, spot.y);
  const w = spot.w * WIN.w * zoom;
  const h = spot.h * WIN.h * zoom;
  return (
    <div
      style={{
        position: 'absolute',
        left: a.left - 6,
        top: a.top - 6,
        width: w + 12,
        height: h + 12,
        borderRadius: 12,
        boxShadow: `0 0 0 3px rgba(10,132,255,${0.95 * o}), 0 0 40px rgba(10,132,255,${0.55 * o}), 0 0 0 4000px rgba(5,5,10,${0.5 * o})`,
        pointerEvents: 'none',
      }}
    />
  );
};

const CalloutPin: React.FC<{
  c: Callout;
  progress: number;
  frames: number;
  frame: number;
  toScreen: (x: number, y: number) => { left: number; top: number };
}> = ({ c, progress, frames, toScreen }) => {
  const t1 = c.t1 ?? 0.98;
  if (progress < c.t0 || progress > t1) return null;
  const inP = Math.min(1, ((progress - c.t0) * frames) / 12);
  const outP = Math.min(1, ((t1 - progress) * frames) / 8);
  const k = Math.min(inP, outP);
  const pop = Easing.out(Easing.back(1.8))(inP);
  const p = toScreen(c.x, c.y);
  // Keep the pill inside the window: estimate its width, flip or slide it.
  const estW = c.label.length * 12.5 + 44 + (c.keys ? c.keys.split(' ').length * 44 : 0);
  let side = c.side ?? 'right';
  if (side === 'right' && p.left + 26 + estW > WIN.w - 12) side = 'left';
  else if (side === 'left' && p.left - 26 - estW < 12) side = 'right';
  const slide =
    side === 'top' || side === 'bottom'
      ? Math.max(12 + estW / 2 - p.left, Math.min(0, WIN.w - 12 - estW / 2 - p.left))
      : 0;
  const off = 26;
  const pill: React.CSSProperties = {
    position: 'absolute',
    whiteSpace: 'nowrap',
    padding: '10px 16px',
    borderRadius: 999,
    background: 'rgba(22,22,26,0.88)',
    backdropFilter: 'blur(12px)',
    border: '1px solid rgba(255,255,255,0.16)',
    boxShadow: '0 12px 30px rgba(0,0,0,0.45)',
    color: C.text,
    font: `600 22px ${FONT}`,
    letterSpacing: -0.2,
    display: 'flex',
    alignItems: 'center',
    gap: 10,
    opacity: k,
  };
  const place: Record<string, React.CSSProperties> = {
    right: { left: off, top: 0, transform: `translateY(-50%) scale(${0.8 + 0.2 * pop})`, transformOrigin: 'left center' },
    left: { right: off, top: 0, transform: `translateY(-50%) scale(${0.8 + 0.2 * pop})`, transformOrigin: 'right center' },
    top: { left: slide, bottom: off, transform: `translateX(-50%) scale(${0.8 + 0.2 * pop})`, transformOrigin: 'center bottom' },
    bottom: { left: slide, top: off, transform: `translateX(-50%) scale(${0.8 + 0.2 * pop})`, transformOrigin: 'center top' },
  };
  return (
    <div style={{ position: 'absolute', left: p.left, top: p.top, width: 0, height: 0 }}>
      <div
        style={{
          position: 'absolute',
          left: -9,
          top: -9,
          width: 18,
          height: 18,
          borderRadius: 9,
          background: C.accent,
          opacity: k,
          boxShadow: `0 0 0 ${4 + 6 * (1 - inP)}px rgba(10,132,255,0.35), 0 0 18px rgba(10,132,255,0.8)`,
        }}
      />
      <div style={{ ...pill, ...place[side] }}>
        {c.label}
        {c.keys ? <Keys keys={c.keys} /> : null}
      </div>
    </div>
  );
};

export const Keys: React.FC<{ keys: string; size?: number }> = ({ keys, size = 18 }) => (
  <span style={{ display: 'inline-flex', gap: 4 }}>
    {keys.split(' ').map((k) => (
      <span
        key={k}
        style={{
          font: `600 ${size}px ${FONT}`,
          padding: '2px 8px',
          borderRadius: 6,
          background: 'rgba(255,255,255,0.12)',
          border: '1px solid rgba(255,255,255,0.22)',
          borderBottomWidth: 2,
          color: '#fff',
        }}
      >
        {k}
      </span>
    ))}
  </span>
);
