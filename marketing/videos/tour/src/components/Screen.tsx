import React from 'react';
import { Img, OffthreadVideo, interpolate, staticFile, useVideoConfig } from 'remotion';
import type { CamKey, Callout, Highlight, ScreenShot } from '../types';
import { BEAT } from '../layout';
import { C, FONT, rgba } from '../theme';
import { GLIDE, POP, beatPulse, clamp, easeInOut, sp } from '../motion';

/** Window geometry inside the 1920×1080 frame: a top band for the chapter header. */
export const WIN = { w: 1536, h: 864, bar: 28, top: 118 };
export const WIN_LEFT = (1920 - WIN.w) / 2;

/** Camera at `f` frames into the shot: each key springs in from wherever the camera is. */
export function camAt(keys: CamKey[] | undefined, f: number): CamKey {
  const ks = keys?.length ? keys : [{ b: 0, x: 0.5, y: 0.5, z: 1 }];
  let cur = { ...ks[0] };
  for (let i = 1; i < ks.length; i++) {
    const at = ks[i].b * BEAT;
    if (f < at) break;
    const k = sp(f, at, GLIDE);
    cur = { b: ks[i].b, x: cur.x + (ks[i].x - cur.x) * k, y: cur.y + (ks[i].y - cur.y) * k, z: cur.z + (ks[i].z - cur.z) * k };
  }
  return cur;
}

/** Clamp the focus so a zoomed view never shows past the footage edge. */
function clampCam(c: CamKey): CamKey {
  const half = 0.5 / c.z;
  return { ...c, x: Math.min(1 - half, Math.max(half, c.x)), y: Math.min(1 - half, Math.max(half, c.y)) };
}

const isVideo = (src: string) => src.endsWith('.mp4') || src.endsWith('.webm');

/**
 * The window's content: footage under a moving camera, highlight rings, pinned
 * callouts and an optional glass card. `f` is frames since the shot's nominal
 * start (negative while it is still arriving), `lead` how early its sequence began.
 */
export const ScreenContent: React.FC<{ shot: ScreenShot; f: number; frames: number; lead: number; tint: string }> = ({ shot, f, frames, lead, tint }) => {
  const { fps } = useVideoConfig();
  // A slow push across the whole shot keeps holds alive between the moves.
  const drift = 1 + 0.025 * interpolate(f, [0, frames], [0, 1], clamp);
  const base = camAt(shot.cam, f);
  const cam = clampCam({ ...base, z: base.z * drift });
  const cw = WIN.w;
  const ch = WIN.h;
  const tx = (0.5 - cam.x) * cw * cam.z;
  const ty = (0.5 - cam.y) * ch * cam.z;
  const toScreen = (x: number, y: number) => ({ left: cw / 2 + (x - cam.x) * cw * cam.z, top: ch / 2 + (y - cam.y) * ch * cam.z });
  const wipe = shot.wipeTo ? interpolate(f, [shot.wipeTo.b0 * BEAT, shot.wipeTo.b1 * BEAT], [0, 1], { ...clamp, easing: easeInOut }) : 0;
  const rate = shot.rate ?? 1;
  const media = (src: string) =>
    isVideo(src) ? (
      <OffthreadVideo
        src={staticFile(src)}
        muted
        trimBefore={Math.max(0, Math.round((shot.from ?? 0) * fps - lead * rate))}
        playbackRate={rate}
        style={{ width: cw, height: ch, display: 'block' }}
      />
    ) : (
      <Img src={staticFile(src)} style={{ width: cw, height: ch, display: 'block' }} />
    );
  return (
    <div style={{ position: 'relative', width: cw, height: ch, overflow: 'hidden', background: C.bg }}>
      <div style={{ position: 'absolute', width: cw, height: ch, transformOrigin: '50% 50%', transform: `translate(${tx}px, ${ty}px) scale(${cam.z})` }}>
        {media(shot.src)}
        {shot.wipeTo ? <div style={{ position: 'absolute', inset: 0, clipPath: `inset(0 0 0 ${(1 - wipe) * 100}%)` }}>{media(shot.wipeTo.src)}</div> : null}
      </div>
      {shot.wipeTo && wipe > 0 && wipe < 1 ? (
        <div style={{ position: 'absolute', top: 0, bottom: 0, left: (1 - wipe) * cw - 2, width: 4, background: '#fff', boxShadow: `0 0 30px 6px ${rgba(tint, 0.9)}` }} />
      ) : null}
      {(shot.highlights ?? []).map((h, i) => (
        <Ring key={i} h={h} f={f} frames={frames} zoom={cam.z} toScreen={toScreen} tint={tint} />
      ))}
      {(shot.callouts ?? []).map((c, i) => (
        <CalloutPin key={i} c={c} f={f} frames={frames} toScreen={toScreen} tint={tint} />
      ))}
      {shot.card ? <Card title={shot.card.title} sub={shot.card.sub} f={f} tint={tint} /> : null}
    </div>
  );
};

/** Visibility 0..1 for an element shown from beat b0 to b1 (springs in, eases out). */
function life(f: number, b0: number, b1: number | undefined, frames: number) {
  const start = b0 * BEAT;
  const end = b1 != null ? b1 * BEAT : frames - 2;
  const inK = sp(f, start, POP);
  const outK = interpolate(f, [end - 6, end], [1, 0], clamp);
  return { inK, k: Math.min(Math.min(1, inK * 1.4), outK), shown: f >= start && f <= end };
}

const Ring: React.FC<{
  h: Highlight;
  f: number;
  frames: number;
  zoom: number;
  tint: string;
  toScreen: (x: number, y: number) => { left: number; top: number };
}> = ({ h, f, frames, zoom, toScreen, tint }) => {
  const { inK, k, shown } = life(f, h.b, h.until, frames);
  if (!shown || k <= 0) return null;
  const a = toScreen(h.x, h.y);
  const w = h.w * WIN.w * zoom;
  const hh = h.h * WIN.h * zoom;
  const grow = 1.12 - 0.12 * inK; // the ring lands onto its target
  const glow = 0.55 + 0.35 * beatPulse(f);
  return (
    <div
      style={{
        position: 'absolute',
        left: a.left - 8,
        top: a.top - 8,
        width: w + 16,
        height: hh + 16,
        borderRadius: 14,
        transform: `scale(${grow})`,
        boxShadow: `0 0 0 3px ${rgba(tint, 0.95 * k)}, 0 0 46px 6px ${rgba(tint, glow * k)}, 0 0 0 4000px rgba(4,4,10,${0.5 * k})`,
      }}
    />
  );
};

const CalloutPin: React.FC<{
  c: Callout;
  f: number;
  frames: number;
  tint: string;
  toScreen: (x: number, y: number) => { left: number; top: number };
}> = ({ c, f, frames, toScreen, tint }) => {
  const { inK, k, shown } = life(f, c.b, c.until, frames);
  if (!shown || k <= 0) return null;
  const p = toScreen(c.x, c.y);
  // Keep the pill inside the window: estimate its width, flip or slide it.
  const estW = c.label.length * 12.5 + 44 + (c.keys ? c.keys.split(' ').length * 44 : 0);
  let side = c.side ?? 'right';
  if (side === 'right' && p.left + 30 + estW > WIN.w - 12) side = 'left';
  else if (side === 'left' && p.left - 30 - estW < 12) side = 'right';
  const slide = side === 'top' || side === 'bottom' ? Math.max(12 + estW / 2 - p.left, Math.min(0, WIN.w - 12 - estW / 2 - p.left)) : 0;
  const off = 30;
  const s = 0.6 + 0.4 * inK;
  const pill: React.CSSProperties = {
    position: 'absolute',
    whiteSpace: 'nowrap',
    padding: '11px 18px',
    borderRadius: 999,
    background: 'rgba(14,14,20,0.9)',
    border: `1px solid ${rgba(tint, 0.55)}`,
    boxShadow: `0 14px 34px rgba(0,0,0,0.5), 0 0 24px ${rgba(tint, 0.25)}`,
    color: C.text,
    font: `650 23px ${FONT}`,
    letterSpacing: -0.2,
    display: 'flex',
    alignItems: 'center',
    gap: 10,
    opacity: k,
  };
  const place: Record<string, React.CSSProperties> = {
    right: { left: off, top: 0, transform: `translateY(-50%) scale(${s})`, transformOrigin: 'left center' },
    left: { right: off, top: 0, transform: `translateY(-50%) scale(${s})`, transformOrigin: 'right center' },
    top: { left: slide, bottom: off, transform: `translateX(-50%) scale(${s})`, transformOrigin: 'center bottom' },
    bottom: { left: slide, top: off, transform: `translateX(-50%) scale(${s})`, transformOrigin: 'center top' },
  };
  const pulse = beatPulse(f, 4);
  return (
    <div style={{ position: 'absolute', left: p.left, top: p.top, width: 0, height: 0 }}>
      {/* the target: a solid dot with a ring that ripples out on the beat */}
      <div
        style={{
          position: 'absolute',
          left: -26,
          top: -26,
          width: 52,
          height: 52,
          borderRadius: 26,
          border: `2px solid ${rgba(tint, 0.7 * k * pulse)}`,
          transform: `scale(${0.5 + 0.7 * (1 - pulse)})`,
        }}
      />
      <div
        style={{
          position: 'absolute',
          left: -9,
          top: -9,
          width: 18,
          height: 18,
          borderRadius: 9,
          background: tint,
          opacity: k,
          boxShadow: `0 0 0 ${4 + 8 * (1 - inK)}px ${rgba(tint, 0.35)}, 0 0 22px ${rgba(tint, 0.9)}`,
        }}
      />
      <div style={{ ...pill, ...place[side] }}>
        {c.label}
        {c.keys ? <Keys keys={c.keys} /> : null}
      </div>
    </div>
  );
};

/** A large glass card naming what's on screen (quick-cut montages). */
const Card: React.FC<{ title: string; sub?: string; f: number; tint: string }> = ({ title, sub, f, tint }) => {
  const k = sp(f, 1, POP);
  const k2 = sp(f, 4, POP);
  return (
    <div
      style={{
        position: 'absolute',
        left: 36,
        bottom: 34,
        padding: '18px 26px 20px',
        borderRadius: 18,
        background: 'rgba(12,12,18,0.88)',
        border: `1px solid ${rgba(tint, 0.45)}`,
        boxShadow: `0 20px 50px rgba(0,0,0,0.55), 0 0 40px ${rgba(tint, 0.18)}`,
        transform: `translateY(${(1 - k) * 40}px)`,
        opacity: Math.min(1, k * 1.5),
      }}
    >
      <div style={{ font: `750 44px ${FONT}`, color: C.text, letterSpacing: -1 }}>{title}</div>
      {sub ? (
        <div style={{ font: `500 24px ${FONT}`, color: '#c8c8d2', marginTop: 4, opacity: k2, transform: `translateY(${(1 - k2) * 10}px)` }}>{sub}</div>
      ) : null}
    </div>
  );
};

/** The macOS title bar above the content. */
export const TitleBar: React.FC = () => (
  <div
    style={{
      height: WIN.bar,
      background: 'linear-gradient(#2c2c31, #26262b)',
      borderBottom: '1px solid rgba(0,0,0,0.5)',
      display: 'flex',
      alignItems: 'center',
      paddingLeft: 12,
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

/** A whole window: title bar + content, glowing faintly in the chapter's tint. */
export const Window: React.FC<{ children: React.ReactNode; tint: string; style?: React.CSSProperties }> = ({ children, tint, style }) => (
  <div
    style={{
      position: 'absolute',
      left: WIN_LEFT,
      top: WIN.top,
      width: WIN.w,
      height: WIN.h + WIN.bar,
      borderRadius: 14,
      overflow: 'hidden',
      background: C.surface,
      boxShadow: `0 40px 120px rgba(0,0,0,0.6), 0 0 0 1px rgba(255,255,255,0.12), 0 0 90px ${rgba(tint, 0.22)}`,
      ...style,
    }}
  >
    <TitleBar />
    {children}
  </div>
);

export const Keys: React.FC<{ keys: string; size?: number }> = ({ keys, size = 19 }) => (
  <span style={{ display: 'inline-flex', gap: 4 }}>
    {keys.split(' ').map((k) => (
      <span
        key={k}
        style={{
          font: `650 ${size}px ${FONT}`,
          padding: '2px 8px',
          borderRadius: 6,
          background: 'rgba(255,255,255,0.12)',
          border: '1px solid rgba(255,255,255,0.24)',
          borderBottomWidth: 2,
          color: '#fff',
        }}
      >
        {k}
      </span>
    ))}
  </span>
);
