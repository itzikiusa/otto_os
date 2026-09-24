import React from 'react';
import { AbsoluteFill, Easing, Img, interpolate, staticFile, useCurrentFrame } from 'remotion';
import { Logo } from './Logo';
import { Keys } from './Screen';
import { C, FONT } from '../theme';

const clamp = { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' } as const;

/**
 * The desktop-app beat: a macOS desktop with Otto in the menu bar. The tray
 * popover drops from the menu bar, ⌥Space summons the floating bar over any
 * app, and a pop-out window slides in. The popover / bar / window contents are
 * real captures of the app's `#/tray`, `#/bar` and pop-out routes.
 */
export const Desktop: React.FC<{ progress: number; frames: number }> = ({ progress, frames }) => {
  const f = useCurrentFrame();
  void f;
  const p = progress;
  // ⌥Space summons the bar → the menu-bar popover → a pop-out window.
  const keysK = interpolate(p, [0.02, 0.08, 0.26, 0.32], [0, 1, 1, 0], clamp);
  const bar = interpolate(p, [0.1, 0.2], [0, 1], { ...clamp, easing: Easing.out(Easing.back(1.4)) });
  const barOut = interpolate(p, [0.46, 0.54], [1, 0], clamp);
  const tray = interpolate(p, [0.46, 0.56, 0.92, 1], [0, 1, 1, 1], clamp);
  const pop = interpolate(p, [0.66, 0.78], [0, 1], { ...clamp, easing: Easing.out(Easing.cubic) });
  void frames;
  return (
    <AbsoluteFill>
      {/* wallpaper */}
      <AbsoluteFill
        style={{
          background:
            'radial-gradient(70% 90% at 20% 20%, #3a2d7a 0%, transparent 60%), radial-gradient(70% 90% at 85% 80%, #0e5d6e 0%, transparent 60%), linear-gradient(135deg, #1b1f3b, #0d1020)',
        }}
      />
      {/* menu bar */}
      <div
        style={{
          position: 'absolute',
          left: 0,
          right: 0,
          top: 0,
          height: 38,
          background: 'rgba(30,30,40,0.55)',
          backdropFilter: 'blur(20px)',
          display: 'flex',
          alignItems: 'center',
          padding: '0 22px',
          gap: 26,
          font: `600 17px ${FONT}`,
          color: '#fff',
        }}
      >
        <span>Finder</span>
        <span style={{ fontWeight: 400, opacity: 0.85 }}>File</span>
        <span style={{ fontWeight: 400, opacity: 0.85 }}>Edit</span>
        <span style={{ fontWeight: 400, opacity: 0.85 }}>View</span>
        <span style={{ flex: 1 }} />
        <span
          style={{
            display: 'flex',
            alignItems: 'center',
            padding: '3px 8px',
            borderRadius: 6,
            background: tray > 0.05 ? 'rgba(255,255,255,0.22)' : 'transparent',
          }}
        >
          <Logo size={22} />
        </span>
        <span style={{ fontWeight: 400 }}>Thu 24 Sep  14:05</span>
      </div>
      {/* tray popover */}
      <div
        style={{
          position: 'absolute',
          right: 120,
          top: 48,
          width: 700,
          height: 505,
          borderRadius: 16,
          overflow: 'hidden',
          boxShadow: '0 30px 80px rgba(0,0,0,0.55), 0 0 0 1px rgba(255,255,255,0.14)',
          opacity: tray,
          transform: `translateY(${(1 - tray) * -14}px) scale(${0.97 + 0.03 * tray})`,
          transformOrigin: 'top right',
        }}
      >
        <Img src={staticFile('capture/desktop-tray.jpg')} style={{ width: '100%', height: '100%', objectFit: 'cover', objectPosition: 'top' }} />
      </div>
      {/* pop-out window */}
      <div
        style={{
          position: 'absolute',
          left: 120,
          top: 150,
          width: 900,
          height: 560,
          borderRadius: 14,
          overflow: 'hidden',
          boxShadow: '0 40px 100px rgba(0,0,0,0.6), 0 0 0 1px rgba(255,255,255,0.14)',
          opacity: pop,
          transform: `translateX(${(1 - pop) * -80}px)`,
          background: C.surface,
        }}
      >
        <div style={{ height: 28, background: '#2a2a30', display: 'flex', alignItems: 'center', gap: 8, paddingLeft: 12 }}>
          {['#ff5f57', '#febc2e', '#28c840'].map((c) => (
            <div key={c} style={{ width: 12, height: 12, borderRadius: 6, background: c }} />
          ))}
          <span style={{ font: `600 13px ${FONT}`, color: '#b5b5bd', marginLeft: 12 }}>Add rate limiting to checkout — Otto</span>
        </div>
        <Img src={staticFile('capture/desktop-popout.jpg')} style={{ width: '100%', height: 532, objectFit: 'cover', objectPosition: 'top left' }} />
      </div>
      {/* ⌥Space bar */}
      <div
        style={{
          position: 'absolute',
          left: '50%',
          top: 470,
          width: 1000,
          transform: `translateX(-50%) translateY(${(1 - bar) * 40}px) scale(${0.92 + 0.08 * bar})`,
          opacity: bar * barOut,
          borderRadius: 22,
          overflow: 'hidden',
          boxShadow: '0 30px 90px rgba(0,0,0,0.6), 0 0 0 1px rgba(255,255,255,0.18)',
        }}
      >
        <Img src={staticFile('capture/desktop-bar.png')} style={{ width: '100%', display: 'block' }} />
      </div>
      {/* keycaps */}
      <div
        style={{
          position: 'absolute',
          left: '50%',
          top: 330,
          transform: `translateX(-50%) scale(${0.9 + 0.1 * keysK})`,
          opacity: keysK,
          padding: '14px 26px',
          borderRadius: 18,
          background: 'rgba(15,15,20,0.8)',
          border: '1px solid rgba(255,255,255,0.16)',
        }}
      >
        <Keys keys="⌥ Space" size={44} />
      </div>
    </AbsoluteFill>
  );
};
