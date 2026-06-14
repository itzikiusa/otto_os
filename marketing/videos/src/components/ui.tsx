import React from 'react';
import { theme } from '../theme';
import {
  interpolate,
  spring,
  useCurrentFrame,
  useVideoConfig,
  staticFile,
  Img,
} from 'remotion';

/** Spring fade+rise; `delay` in frames. */
export const Appear: React.FC<{
  delay?: number;
  y?: number;
  children: React.ReactNode;
  style?: React.CSSProperties;
}> = ({ delay = 0, y = 24, children, style }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const s = spring({ frame: frame - delay, fps, config: { damping: 200 } });
  return (
    <div
      style={{
        opacity: s,
        transform: `translateY(${interpolate(s, [0, 1], [y, 0])}px)`,
        ...style,
      }}
    >
      {children}
    </div>
  );
};

/** Lower-third caption: a number + a line of explanation. */
export const Caption: React.FC<{ step?: number; title: string; sub?: string; delay?: number }> = ({
  step,
  title,
  sub,
  delay = 0,
}) => (
  <Appear delay={delay} style={{ position: 'absolute', left: 60, bottom: 54, maxWidth: 1000 }}>
    <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
      {step != null && (
        <div
          style={{
            width: 44,
            height: 44,
            borderRadius: 12,
            background: theme.accent,
            color: '#fff',
            display: 'grid',
            placeItems: 'center',
            fontFamily: theme.font,
            fontWeight: 800,
            fontSize: 22,
            flexShrink: 0,
            boxShadow: `0 8px 30px ${theme.accent}66`,
          }}
        >
          {step}
        </div>
      )}
      <div
        style={{
          fontFamily: theme.font,
          color: theme.text,
          fontSize: 38,
          fontWeight: 700,
          textShadow: '0 2px 24px rgba(0,0,0,0.8)',
        }}
      >
        {title}
      </div>
    </div>
    {sub && (
      <div
        style={{
          fontFamily: theme.font,
          color: theme.textDim,
          fontSize: 24,
          marginTop: 10,
          marginLeft: step != null ? 60 : 0,
          textShadow: '0 2px 16px rgba(0,0,0,0.8)',
        }}
      >
        {sub}
      </div>
    )}
  </Appear>
);

/** A keyboard key chip, e.g. <KeyCap>⌘</KeyCap><KeyCap>K</KeyCap>. */
export const KeyCap: React.FC<{ children: React.ReactNode; wide?: boolean }> = ({
  children,
  wide,
}) => (
  <span
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      justifyContent: 'center',
      minWidth: wide ? 92 : 56,
      height: 56,
      padding: '0 14px',
      borderRadius: 12,
      background: 'linear-gradient(180deg,#2a3340,#1a212b)',
      border: `1px solid ${theme.border}`,
      boxShadow: '0 4px 0 #0c1014, 0 8px 24px rgba(0,0,0,0.4)',
      color: theme.text,
      fontFamily: theme.font,
      fontSize: 30,
      fontWeight: 700,
    }}
  >
    {children}
  </span>
);

export const Shortcut: React.FC<{ keys: string[]; delay?: number }> = ({ keys, delay = 0 }) => (
  <Appear delay={delay} y={14}>
    <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
      {keys.map((k, i) => (
        <React.Fragment key={i}>
          {i > 0 && <span style={{ color: theme.textDim, fontSize: 28 }}>+</span>}
          <KeyCap wide={k.length > 1}>{k}</KeyCap>
        </React.Fragment>
      ))}
    </div>
  </Appear>
);

/** Animated mouse cursor that moves between points over a frame window. */
export const Cursor: React.FC<{
  from: [number, number];
  to: [number, number];
  startAt: number;
  duration?: number;
  click?: boolean;
}> = ({ from, to, startAt, duration = 24, click }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const t = spring({ frame: frame - startAt, fps, durationInFrames: duration, config: { damping: 200 } });
  const x = interpolate(t, [0, 1], [from[0], to[0]]);
  const y = interpolate(t, [0, 1], [from[1], to[1]]);
  const clickT = click ? spring({ frame: frame - startAt - duration, fps, durationInFrames: 10 }) : 0;
  const ring = click ? interpolate(clickT, [0, 1], [0, 60]) : 0;
  const ringOp = click ? interpolate(clickT, [0, 1], [0.5, 0]) : 0;
  return (
    <div style={{ position: 'absolute', left: x, top: y, zIndex: 50, pointerEvents: 'none' }}>
      {click && (
        <div
          style={{
            position: 'absolute',
            left: -ring / 2,
            top: -ring / 2,
            width: ring,
            height: ring,
            borderRadius: '50%',
            border: `3px solid ${theme.accent2}`,
            opacity: ringOp,
          }}
        />
      )}
      <svg width="30" height="34" viewBox="0 0 30 34" style={{ filter: 'drop-shadow(0 3px 6px rgba(0,0,0,.6))' }}>
        <path d="M2 2 L2 26 L8 20 L12 30 L17 28 L13 18 L22 18 Z" fill="#fff" stroke="#1a1a1a" strokeWidth="1.5" />
      </svg>
    </div>
  );
};

/** Full-screen brand title card. */
export const TitleCard: React.FC<{ title: string; subtitle?: string; kicker?: string }> = ({
  title,
  subtitle,
  kicker,
}) => (
  <div
    style={{
      position: 'absolute',
      inset: 0,
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'center',
      justifyContent: 'center',
      gap: 8,
    }}
  >
    <Appear>
      <Img
        src={staticFile('otto-mark.png')}
        style={{ width: 150, height: 150, borderRadius: 34, boxShadow: `0 30px 90px ${theme.accent}55` }}
      />
    </Appear>
    {kicker && (
      <Appear delay={6}>
        <div style={{ color: theme.accent2, fontFamily: theme.mono, fontSize: 24, letterSpacing: 3, marginTop: 22 }}>
          {kicker}
        </div>
      </Appear>
    )}
    <Appear delay={10}>
      <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 84, fontWeight: 800, marginTop: 6 }}>
        {title}
      </div>
    </Appear>
    {subtitle && (
      <Appear delay={16}>
        <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 32 }}>{subtitle}</div>
      </Appear>
    )}
  </div>
);
