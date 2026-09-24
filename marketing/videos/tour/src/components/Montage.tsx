import React from 'react';
import { AbsoluteFill, Easing, Img, interpolate, staticFile } from 'remotion';
import type { MontageShot } from '../types';
import { C, FONT } from '../theme';

const clamp = { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' } as const;

/**
 * Quick-beat montage: each item gets an equal slice; the current one is a
 * large card (with a slow push-in onto its focus point), the previous one
 * slides away and shrinks into a strip of thumbnails along the bottom.
 */
export const Montage: React.FC<{ shot: MontageShot; progress: number; frames: number }> = ({ shot, progress, frames }) => {
  const n = shot.items.length;
  const slot = 1 / n;
  const idx = Math.min(n - 1, Math.floor(progress / slot));
  const local = (progress - idx * slot) / slot;
  const localFrames = frames * slot;
  const W = 1480;
  const H = W * (9 / 16);
  return (
    <AbsoluteFill>
      {shot.items.map((it, i) => {
        if (i < idx - 1 || i > idx) return null;
        const isCur = i === idx;
        const k = isCur
          ? interpolate(local * localFrames, [0, 16], [0, 1], { ...clamp, easing: Easing.out(Easing.cubic) })
          : 1;
        const leaving = !isCur ? interpolate(local * localFrames, [0, 16], [0, 1], { ...clamp, easing: Easing.in(Easing.cubic) }) : 0;
        if (!isCur && leaving >= 1) return null;
        const z0 = it.z ?? 1.12;
        const pz = isCur ? 1 + (z0 - 1) * Easing.inOut(Easing.quad)(local) : z0;
        const fx = it.x ?? 0.5;
        const fy = it.y ?? 0.5;
        const half = 0.5 / pz;
        const cx = Math.min(1 - half, Math.max(half, 0.5 + (fx - 0.5) * (pz - 1) / Math.max(0.001, pz - 1 + 1e-9)));
        const cy = Math.min(1 - half, Math.max(half, 0.5 + (fy - 0.5) * (pz - 1) / Math.max(0.001, pz - 1 + 1e-9)));
        const tx = (0.5 - cx) * W * pz;
        const ty = (0.5 - cy) * H * pz;
        const x = isCur ? (1 - k) * 380 : -leaving * 520;
        const s = isCur ? 0.9 + 0.1 * k : 1 - 0.25 * leaving;
        const o = isCur ? k : 1 - leaving;
        return (
          <AbsoluteFill key={i} style={{ alignItems: 'center', justifyContent: 'center' }}>
            <div
              style={{
                width: W,
                height: H,
                borderRadius: 16,
                overflow: 'hidden',
                position: 'relative',
                transform: `translateX(${x}px) scale(${s}) rotateY(${isCur ? (1 - k) * -12 : leaving * 10}deg)`,
                opacity: o,
                boxShadow: '0 40px 120px rgba(0,0,0,0.55), 0 0 0 1px rgba(255,255,255,0.12)',
                background: C.surface,
              }}
            >
              <Img
                src={staticFile(it.src)}
                style={{ width: W, height: H, transform: `translate(${tx}px, ${ty}px) scale(${pz})`, transformOrigin: '50% 50%' }}
              />
              <div
                style={{
                  position: 'absolute',
                  left: 28,
                  bottom: 26,
                  padding: '12px 20px',
                  borderRadius: 14,
                  background: 'rgba(14,14,18,0.84)',
                  backdropFilter: 'blur(14px)',
                  border: '1px solid rgba(255,255,255,0.14)',
                  opacity: isCur ? interpolate(local * localFrames, [8, 20], [0, 1], clamp) : 1,
                }}
              >
                <div style={{ font: `700 32px ${FONT}`, color: C.text, letterSpacing: -0.6 }}>{it.label}</div>
                {it.sub ? <div style={{ font: `500 20px ${FONT}`, color: '#c4c4cc', marginTop: 2 }}>{it.sub}</div> : null}
              </div>
            </div>
          </AbsoluteFill>
        );
      })}
      {/* progress dots */}
      <div style={{ position: 'absolute', bottom: 28, left: 0, right: 0, display: 'flex', justifyContent: 'center', gap: 10 }}>
        {shot.items.map((_, i) => (
          <div
            key={i}
            style={{
              width: i === idx ? 28 : 9,
              height: 9,
              borderRadius: 5,
              background: i === idx ? C.accent : 'rgba(255,255,255,0.3)',
            }}
          />
        ))}
      </div>
    </AbsoluteFill>
  );
};
