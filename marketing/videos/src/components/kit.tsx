import React from 'react';
import {
  AbsoluteFill,
  interpolate,
  spring,
  useCurrentFrame,
  useVideoConfig,
  Easing,
} from 'remotion';
import {
  T,
  Theme,
  fonts,
  radius,
  brand,
  status as STATUS,
  navActive,
  alpha,
  cinematicBg,
} from '../theme';
import { Icon } from './Icon';
import { OttoIcon, OttoGlyph } from './OttoLogo';

// ════════════════════════════════════════════════════════════════════════════
//  ANIMATION HELPERS
// ════════════════════════════════════════════════════════════════════════════

/** Smooth 0→1 spring, delayable, with sensible defaults. */
export function useSpring(delay = 0, opts?: { damping?: number; stiffness?: number; mass?: number }): number {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  return spring({
    frame: frame - delay,
    fps,
    config: { damping: opts?.damping ?? 200, stiffness: opts?.stiffness ?? 120, mass: opts?.mass ?? 1 },
  });
}

/** Linear-ish eased interpolation over a frame window, clamped both ends. */
export function track(frame: number, range: [number, number], out: [number, number], ease = true): number {
  return interpolate(frame, range, out, {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
    easing: ease ? Easing.inOut(Easing.cubic) : (x) => x,
  });
}

/** Fade + rise on entry (spring). Holds in place after. */
export const Appear: React.FC<{
  delay?: number;
  y?: number;
  x?: number;
  scale?: number;
  children: React.ReactNode;
  style?: React.CSSProperties;
}> = ({ delay = 0, y = 22, x = 0, scale, children, style }) => {
  const s = useSpring(delay);
  return (
    <div
      style={{
        opacity: s,
        transform: `translate(${interpolate(s, [0, 1], [x, 0])}px, ${interpolate(s, [0, 1], [y, 0])}px)` +
          (scale != null ? ` scale(${interpolate(s, [0, 1], [scale, 1])})` : ''),
        ...style,
      }}
    >
      {children}
    </div>
  );
};

/** Stagger a list of nodes by `step` frames from `delay`. */
export const Stagger: React.FC<{
  delay?: number;
  step?: number;
  y?: number;
  children: React.ReactNode;
  style?: React.CSSProperties;
  childStyle?: React.CSSProperties;
}> = ({ delay = 0, step = 5, y = 16, children, style, childStyle }) => (
  <div style={style}>
    {React.Children.toArray(children).map((c, i) => (
      <Appear key={i} delay={delay + i * step} y={y} style={childStyle}>
        {c}
      </Appear>
    ))}
  </div>
);

/** Typewriter string: how much of `text` is revealed at the current frame. */
export function useTyped(text: string, start = 0, cps = 26): string {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const n = Math.max(0, Math.floor(((frame - start) / fps) * cps));
  return text.slice(0, n);
}

/** Blinking caret. */
export const Caret: React.FC<{ color?: string; h?: number }> = ({ color = brand.cyan, h = 18 }) => {
  const frame = useCurrentFrame();
  return (
    <span
      style={{
        display: 'inline-block',
        width: 2,
        height: h,
        background: color,
        marginLeft: 2,
        verticalAlign: 'text-bottom',
        opacity: Math.floor(frame / 8) % 2 === 0 ? 1 : 0.15,
      }}
    />
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  CINEMATIC LAYER (brand identity)
// ════════════════════════════════════════════════════════════════════════════

/** Deep brand void with drifting auroras + faint grid + vignette. Put behind a scene. */
export const Background: React.FC<{ grid?: boolean; drift?: boolean }> = ({ grid = true, drift = true }) => {
  const frame = useCurrentFrame();
  const d = drift ? frame : 0;
  return (
    <AbsoluteFill style={{ background: cinematicBg, overflow: 'hidden' }}>
      <div
        style={{
          position: 'absolute',
          width: 1100,
          height: 1100,
          left: -180 + Math.sin(d / 120) * 40,
          top: -360 + Math.cos(d / 140) * 30,
          borderRadius: '50%',
          background: `radial-gradient(circle, ${alpha(brand.purple, 0.32)} 0%, rgba(0,0,0,0) 62%)`,
          filter: 'blur(20px)',
        }}
      />
      <div
        style={{
          position: 'absolute',
          width: 1000,
          height: 1000,
          right: -260 - Math.sin(d / 150) * 36,
          bottom: -340 + Math.sin(d / 110) * 30,
          borderRadius: '50%',
          background: `radial-gradient(circle, ${alpha(brand.cyan, 0.18)} 0%, rgba(0,0,0,0) 60%)`,
          filter: 'blur(24px)',
        }}
      />
      {grid && (
        <div
          style={{
            position: 'absolute',
            inset: 0,
            backgroundImage:
              `linear-gradient(${alpha('#ffffff', 0.035)} 1px, transparent 1px),` +
              `linear-gradient(90deg, ${alpha('#ffffff', 0.035)} 1px, transparent 1px)`,
            backgroundSize: '64px 64px',
            maskImage: 'radial-gradient(1200px 700px at 50% 42%, #000 0%, transparent 78%)',
            WebkitMaskImage: 'radial-gradient(1200px 700px at 50% 42%, #000 0%, transparent 78%)',
          }}
        />
      )}
      <AbsoluteFill
        style={{ boxShadow: 'inset 0 0 320px rgba(0,0,0,0.7)', pointerEvents: 'none' }}
      />
    </AbsoluteFill>
  );
};

/** Eyebrow / kicker — mono, tracked, brand-cyan. */
export const Kicker: React.FC<{ children: React.ReactNode; delay?: number; color?: string }> = ({
  children,
  delay = 0,
  color = brand.cyan,
}) => (
  <Appear delay={delay} y={10}>
    <div
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 10,
        fontFamily: fonts.mono,
        fontSize: 21,
        letterSpacing: 5,
        textTransform: 'uppercase',
        color,
        fontWeight: 600,
      }}
    >
      <span style={{ width: 26, height: 2, background: color, opacity: 0.7 }} />
      {children}
    </div>
  </Appear>
);

/** Big gradient brand word. */
export const BrandWord: React.FC<{ children: React.ReactNode; size?: number; delay?: number }> = ({
  children,
  size = 92,
  delay = 0,
}) => (
  <Appear delay={delay} y={26}>
    <div
      style={{
        fontFamily: fonts.ui,
        fontSize: size,
        fontWeight: 800,
        letterSpacing: -2,
        lineHeight: 1.02,
        backgroundImage: brand.gradSoft,
        WebkitBackgroundClip: 'text',
        backgroundClip: 'text',
        color: 'transparent',
        WebkitTextFillColor: 'transparent',
      }}
    >
      {children}
    </div>
  </Appear>
);

/** Lower-third caption: step pill + title + subtitle. Brand-styled, glassy. */
export const Caption: React.FC<{
  step?: number | string;
  title: string;
  sub?: string;
  delay?: number;
  align?: 'left' | 'center';
}> = ({ step, title, sub, delay = 0, align = 'left' }) => (
  <Appear
    delay={delay}
    y={26}
    style={{
      position: 'absolute',
      left: align === 'center' ? '50%' : 70,
      bottom: 66,
      transform: align === 'center' ? 'translateX(-50%)' : undefined,
      maxWidth: 1180,
      textAlign: align,
    }}
  >
    <div style={{ display: 'flex', alignItems: 'center', gap: 16, justifyContent: align === 'center' ? 'center' : 'flex-start' }}>
      {step != null && (
        <div
          style={{
            minWidth: 46,
            height: 46,
            padding: '0 14px',
            borderRadius: 14,
            background: brand.grad,
            color: '#fff',
            display: 'grid',
            placeItems: 'center',
            fontFamily: fonts.ui,
            fontWeight: 800,
            fontSize: 22,
            boxShadow: `0 10px 34px ${alpha(brand.purple, 0.5)}`,
          }}
        >
          {step}
        </div>
      )}
      <div
        style={{
          fontFamily: fonts.ui,
          color: '#ffffff',
          fontSize: 42,
          fontWeight: 750 as never,
          letterSpacing: -0.5,
          textShadow: '0 2px 26px rgba(0,0,0,0.85)',
        }}
      >
        {title}
      </div>
    </div>
    {sub && (
      <div
        style={{
          fontFamily: fonts.ui,
          color: alpha('#ffffff', 0.72),
          fontSize: 25,
          marginTop: 12,
          marginLeft: step != null && align === 'left' ? 62 : 0,
          textShadow: '0 2px 18px rgba(0,0,0,0.85)',
          maxWidth: 980,
        }}
      >
        {sub}
      </div>
    )}
  </Appear>
);

/** Full-screen brand title card (kicker + word + subtitle + app icon). */
export const TitleCard: React.FC<{
  kicker?: string;
  title: string;
  subtitle?: string;
  icon?: boolean;
}> = ({ kicker, title, subtitle, icon = true }) => (
  <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center', gap: 0 }}>
    {icon && (
      <Appear delay={2} scale={0.7} y={0} style={{ marginBottom: 30 }}>
        <OttoIcon size={132} />
      </Appear>
    )}
    {kicker && <div style={{ marginBottom: 18 }}><Kicker delay={10}>{kicker}</Kicker></div>}
    <BrandWord delay={16} size={104}>{title}</BrandWord>
    {subtitle && (
      <Appear delay={26} y={16}>
        <div
          style={{
            fontFamily: fonts.ui,
            fontSize: 30,
            color: alpha('#ffffff', 0.66),
            marginTop: 18,
            letterSpacing: 0.2,
            textAlign: 'center',
            maxWidth: 1100,
          }}
        >
          {subtitle}
        </div>
      </Appear>
    )}
  </AbsoluteFill>
);

/** Brand feature pill (dot + label). */
export const FeaturePill: React.FC<{ label: string; color?: string; delay?: number; icon?: string }> = ({
  label,
  color = brand.cyan,
  delay = 0,
  icon,
}) => (
  <Appear delay={delay} y={14} scale={0.9}>
    <div
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 10,
        padding: '11px 20px',
        borderRadius: 40,
        background: alpha(color, 0.12),
        border: `1px solid ${alpha(color, 0.4)}`,
        boxShadow: `0 6px 26px ${alpha(color, 0.16)}`,
        fontFamily: fonts.ui,
        fontSize: 22,
        fontWeight: 650 as never,
        color: '#fff',
        whiteSpace: 'nowrap',
      }}
    >
      {icon ? (
        <Icon name={icon} size={18} color={color} />
      ) : (
        <span style={{ width: 9, height: 9, borderRadius: '50%', background: color, boxShadow: `0 0 10px ${color}` }} />
      )}
      {label}
    </div>
  </Appear>
);

// ════════════════════════════════════════════════════════════════════════════
//  IN-APP ATOMS (themeable; default native dark)
// ════════════════════════════════════════════════════════════════════════════

export const StatusDot: React.FC<{ kind?: keyof typeof STATUS; size?: number; pulse?: boolean }> = ({
  kind = 'working',
  size = 9,
  pulse = true,
}) => {
  const frame = useCurrentFrame();
  const color = STATUS[kind];
  const p = kind === 'working' && pulse ? Math.sin(frame / 8) * 0.35 + 0.65 : 1;
  return (
    <span
      style={{
        width: size,
        height: size,
        borderRadius: '50%',
        background: color,
        opacity: kind === 'idle' ? 0.6 : p,
        boxShadow: kind === 'working' ? `0 0 8px ${color}` : 'none',
        flexShrink: 0,
        display: 'inline-block',
      }}
    />
  );
};

export const Chip: React.FC<{
  children: React.ReactNode;
  tone?: 'default' | 'ok' | 'bad' | 'warn' | 'accent';
  t?: Theme;
  color?: string;
  style?: React.CSSProperties;
}> = ({ children, tone = 'default', t = T, color, style }) => {
  const c =
    color ?? (tone === 'ok' ? STATUS.working : tone === 'bad' ? STATUS.exited : tone === 'warn' ? STATUS.needsYou : tone === 'accent' ? t.accent : t.textDim);
  const tinted = tone !== 'default' || color != null;
  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        height: 22,
        padding: '0 9px',
        borderRadius: 999,
        fontFamily: fonts.ui,
        fontSize: 12.5,
        fontWeight: 600,
        color: tinted ? c : t.textDim,
        background: tinted ? alpha(c, 0.15) : t.surface2,
        border: `1px solid ${tinted ? alpha(c, 0.4) : t.border}`,
        whiteSpace: 'nowrap',
        ...style,
      }}
    >
      {children}
    </span>
  );
};

export const Button: React.FC<{
  children: React.ReactNode;
  variant?: 'primary' | 'default' | 'ghost' | 'danger';
  t?: Theme;
  size?: 'm' | 's';
  icon?: string;
  style?: React.CSSProperties;
}> = ({ children, variant = 'default', t = T, size = 'm', icon, style }) => {
  const h = size === 's' ? 24 : 30;
  const primary = variant === 'primary';
  const danger = variant === 'danger';
  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 7,
        height: h,
        padding: `0 ${size === 's' ? 10 : 13}px`,
        borderRadius: radius.s,
        fontFamily: fonts.ui,
        fontSize: size === 's' ? 12 : 13,
        fontWeight: 600,
        color: primary ? t.accentContrast : danger ? STATUS.exited : t.text,
        background: primary ? t.accent : variant === 'ghost' ? 'transparent' : t.surface,
        border: `1px solid ${primary ? 'transparent' : variant === 'ghost' ? 'transparent' : t.border}`,
        boxShadow: primary ? `0 6px 18px ${alpha(t.accent, 0.4)}` : 'none',
        whiteSpace: 'nowrap',
        ...style,
      }}
    >
      {icon && <Icon name={icon} size={14} />}
      {children}
    </span>
  );
};

export const Card: React.FC<{
  children?: React.ReactNode;
  t?: Theme;
  pad?: number;
  style?: React.CSSProperties;
}> = ({ children, t = T, pad = 16, style }) => (
  <div
    style={{
      background: t.surface,
      border: `1px solid ${t.border}`,
      borderRadius: radius.m,
      padding: pad,
      boxSizing: 'border-box',
      ...style,
    }}
  >
    {children}
  </div>
);

export const Field: React.FC<{
  label?: string;
  value?: string;
  placeholder?: string;
  t?: Theme;
  focused?: boolean;
  mono?: boolean;
  caret?: boolean;
  icon?: string;
  style?: React.CSSProperties;
}> = ({ label, value, placeholder, t = T, focused, mono, caret, icon, style }) => (
  <div style={{ display: 'flex', flexDirection: 'column', gap: 5, ...style }}>
    {label && (
      <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 500, color: t.textDim }}>{label}</span>
    )}
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        minHeight: 32,
        padding: '0 11px',
        borderRadius: radius.s,
        background: t.surface2,
        border: `1px solid ${focused ? t.accent : t.border}`,
        boxShadow: focused ? `0 0 0 3px ${alpha(t.accent, 0.22)}` : 'none',
      }}
    >
      {icon && <Icon name={icon} size={14} color={t.textDim} />}
      <span
        style={{
          flex: 1,
          fontFamily: mono ? fonts.mono : fonts.ui,
          fontSize: 14,
          color: value ? t.text : alpha(t.textDim, 0.8),
        }}
      >
        {value || placeholder}
      </span>
      {caret && <Caret color={t.accent} h={16} />}
    </div>
  </div>
);

export const Toggle: React.FC<{ on?: boolean; t?: Theme }> = ({ on = true, t = T }) => (
  <span
    style={{
      width: 38,
      height: 22,
      borderRadius: 999,
      background: on ? STATUS.working : t.surface2,
      border: `1px solid ${on ? alpha(STATUS.working, 0.5) : t.border}`,
      position: 'relative',
      display: 'inline-block',
    }}
  >
    <span
      style={{
        position: 'absolute',
        top: 2,
        left: on ? 18 : 2,
        width: 16,
        height: 16,
        borderRadius: '50%',
        background: '#fff',
        boxShadow: '0 1px 3px rgba(0,0,0,0.4)',
      }}
    />
  </span>
);

export const Segmented: React.FC<{ options: string[]; active?: number; t?: Theme }> = ({
  options,
  active = 0,
  t = T,
}) => (
  <div
    style={{
      display: 'inline-flex',
      gap: 2,
      padding: 2,
      borderRadius: radius.s,
      background: t.surface2,
      border: `1px solid ${t.border}`,
    }}
  >
    {options.map((o, i) => (
      <span
        key={o}
        style={{
          height: 24,
          padding: '0 12px',
          display: 'grid',
          placeItems: 'center',
          borderRadius: 4,
          fontFamily: fonts.ui,
          fontSize: 12.5,
          fontWeight: i === active ? 600 : 500,
          color: i === active ? t.text : t.textDim,
          background: i === active ? t.surface : 'transparent',
          boxShadow: i === active ? '0 1px 2px rgba(0,0,0,0.18)' : 'none',
        }}
      >
        {o}
      </span>
    ))}
  </div>
);

export const KeyCap: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <span
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      justifyContent: 'center',
      minWidth: 50,
      height: 50,
      padding: '0 13px',
      borderRadius: 11,
      background: 'linear-gradient(180deg,#33333b,#1d1d23)',
      border: '1px solid rgba(255,255,255,0.12)',
      boxShadow: '0 4px 0 #0c0c10, 0 8px 22px rgba(0,0,0,0.45)',
      color: '#fff',
      fontFamily: fonts.ui,
      fontSize: 26,
      fontWeight: 700,
    }}
  >
    {children}
  </span>
);

export const Keys: React.FC<{ keys: string[]; delay?: number }> = ({ keys, delay = 0 }) => (
  <Appear delay={delay} y={12}>
    <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
      {keys.map((k, i) => (
        <React.Fragment key={i}>
          {i > 0 && <span style={{ color: alpha('#fff', 0.5), fontSize: 24 }}>+</span>}
          <KeyCap>{k}</KeyCap>
        </React.Fragment>
      ))}
    </div>
  </Appear>
);

export const Avatar: React.FC<{ name: string; t?: Theme; size?: number; color?: string }> = ({
  name,
  t = T,
  size = 26,
  color,
}) => {
  const c = color ?? t.accent;
  return (
    <span
      style={{
        width: size,
        height: size,
        borderRadius: '50%',
        background: alpha(c, 0.26),
        color: c,
        display: 'grid',
        placeItems: 'center',
        fontFamily: fonts.ui,
        fontSize: size * 0.42,
        fontWeight: 700,
        flexShrink: 0,
      }}
    >
      {name.slice(0, 1).toUpperCase()}
    </span>
  );
};

/** Animated mouse cursor moving from→to over a window, with optional click ring. */
export const Cursor: React.FC<{
  from: [number, number];
  to: [number, number];
  startAt: number;
  duration?: number;
  click?: boolean;
}> = ({ from, to, startAt, duration = 22, click }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const tt = spring({ frame: frame - startAt, fps, durationInFrames: duration, config: { damping: 200 } });
  const x = interpolate(tt, [0, 1], [from[0], to[0]]);
  const y = interpolate(tt, [0, 1], [from[1], to[1]]);
  const clickT = click ? spring({ frame: frame - startAt - duration, fps, durationInFrames: 12 }) : 0;
  const ring = click ? interpolate(clickT, [0, 1], [0, 64]) : 0;
  const ringOp = click ? interpolate(clickT, [0, 1], [0.55, 0]) : 0;
  return (
    <div style={{ position: 'absolute', left: x, top: y, zIndex: 80, pointerEvents: 'none' }}>
      {click && (
        <div
          style={{
            position: 'absolute',
            left: -ring / 2,
            top: -ring / 2,
            width: ring,
            height: ring,
            borderRadius: '50%',
            border: `3px solid ${brand.cyan}`,
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

export const Toast: React.FC<{
  text: string;
  tone?: 'ok' | 'bad' | 'info';
  t?: Theme;
  delay?: number;
  style?: React.CSSProperties;
}> = ({ text, tone = 'ok', t = T, delay = 0, style }) => {
  const c = tone === 'ok' ? STATUS.working : tone === 'bad' ? STATUS.exited : t.accent;
  return (
    <Appear delay={delay} y={-14} style={style}>
      <div
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 10,
          padding: '11px 16px',
          borderRadius: radius.m,
          background: t.surface,
          border: `1px solid ${t.border}`,
          boxShadow: t.shadow,
          fontFamily: fonts.ui,
          fontSize: 14,
          color: t.text,
        }}
      >
        <span style={{ width: 18, height: 18, borderRadius: '50%', background: c, display: 'grid', placeItems: 'center' }}>
          <Icon name={tone === 'bad' ? 'x' : 'check'} size={12} color="#fff" />
        </span>
        {text}
      </div>
    </Appear>
  );
};

// ── data viz ────────────────────────────────────────────────────────────────

export const MetricStat: React.FC<{
  label: string;
  value: string;
  delta?: string;
  deltaTone?: 'ok' | 'bad';
  t?: Theme;
  accent?: string;
  style?: React.CSSProperties;
}> = ({ label, value, delta, deltaTone = 'ok', t = T, accent, style }) => (
  <Card t={t} pad={14} style={{ minWidth: 160, ...style }}>
    <div style={{ fontFamily: fonts.ui, fontSize: 12.5, color: t.textDim, marginBottom: 6 }}>{label}</div>
    <div style={{ fontFamily: fonts.ui, fontSize: 30, fontWeight: 750 as never, color: accent ?? t.text, letterSpacing: -0.5 }}>
      {value}
    </div>
    {delta && (
      <div style={{ fontFamily: fonts.ui, fontSize: 12.5, color: deltaTone === 'ok' ? STATUS.working : STATUS.exited, marginTop: 4 }}>
        {delta}
      </div>
    )}
  </Card>
);

/** Animated bar chart. `grow` 0–1 scales heights in. */
export const BarChart: React.FC<{
  data: number[];
  labels?: string[];
  color?: string;
  t?: Theme;
  height?: number;
  grow?: number;
  width?: number;
}> = ({ data, labels, color, t = T, height = 160, grow = 1, width }) => {
  const max = Math.max(...data, 1);
  const c = color ?? t.accent;
  return (
    <div style={{ width, display: 'flex', flexDirection: 'column', gap: 6 }}>
      <div style={{ display: 'flex', alignItems: 'flex-end', gap: 8, height }}>
        {data.map((v, i) => (
          <div
            key={i}
            style={{
              flex: 1,
              height: `${(v / max) * 100 * grow}%`,
              minHeight: 2,
              borderRadius: 5,
              background: `linear-gradient(180deg, ${c}, ${alpha(c, 0.45)})`,
            }}
          />
        ))}
      </div>
      {labels && (
        <div style={{ display: 'flex', gap: 8 }}>
          {labels.map((l, i) => (
            <div key={i} style={{ flex: 1, textAlign: 'center', fontFamily: fonts.ui, fontSize: 10.5, color: t.textDim }}>
              {l}
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

/** Smooth area sparkline (SVG). `progress` 0–1 draws it in. */
export const Sparkline: React.FC<{
  data: number[];
  color?: string;
  width?: number;
  height?: number;
  progress?: number;
  t?: Theme;
}> = ({ data, color, width = 320, height = 90, progress = 1, t = T }) => {
  const c = color ?? t.accent;
  const max = Math.max(...data);
  const min = Math.min(...data);
  const n = data.length;
  const pts = data.map((v, i) => {
    const x = (i / (n - 1)) * width;
    const y = height - ((v - min) / (max - min || 1)) * (height - 10) - 5;
    return [x, y] as const;
  });
  const visN = Math.max(2, Math.ceil(n * progress));
  const vis = pts.slice(0, visN);
  const line = vis.map((p, i) => `${i === 0 ? 'M' : 'L'}${p[0].toFixed(1)},${p[1].toFixed(1)}`).join(' ');
  const area = `${line} L${vis[vis.length - 1][0].toFixed(1)},${height} L0,${height} Z`;
  const gid = `spark-${Math.round(width)}-${Math.round(c.length * 7)}`;
  return (
    <svg width={width} height={height}>
      <defs>
        <linearGradient id={gid} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor={c} stopOpacity="0.36" />
          <stop offset="1" stopColor={c} stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d={area} fill={`url(#${gid})`} />
      <path d={line} fill="none" stroke={c} strokeWidth={2.5} strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
};

/** Donut / ring progress. `value` 0–1. */
export const Ring: React.FC<{
  value: number;
  size?: number;
  color?: string;
  track?: string;
  label?: string;
  t?: Theme;
}> = ({ value, size = 120, color, track: trackC, label, t = T }) => {
  const c = color ?? t.accent;
  const r = size / 2 - 8;
  const circ = 2 * Math.PI * r;
  return (
    <div style={{ position: 'relative', width: size, height: size }}>
      <svg width={size} height={size} style={{ transform: 'rotate(-90deg)' }}>
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke={trackC ?? t.surface2} strokeWidth={9} />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke={c}
          strokeWidth={9}
          strokeLinecap="round"
          strokeDasharray={circ}
          strokeDashoffset={circ * (1 - Math.max(0, Math.min(1, value)))}
        />
      </svg>
      {label && (
        <div
          style={{
            position: 'absolute',
            inset: 0,
            display: 'grid',
            placeItems: 'center',
            fontFamily: fonts.ui,
            fontWeight: 750 as never,
            fontSize: size * 0.2,
            color: t.text,
          }}
        >
          {label}
        </div>
      )}
    </div>
  );
};

// ── terminal / code / diff ────────────────────────────────────────────────────

export interface TermLine {
  text: string;
  tone?: 'cmd' | 'ok' | 'dim' | 'warn' | 'err' | 'text' | 'accent';
}

/** Terminal body with line-by-line reveal (each line appears at delay + i*step). */
export const Terminal: React.FC<{
  lines: TermLine[];
  t?: Theme;
  fontSize?: number;
  delay?: number;
  step?: number;
  pad?: number;
  style?: React.CSSProperties;
}> = ({ lines, t = T, fontSize = 15, delay = 0, step = 9, pad = 18, style }) => {
  const frame = useCurrentFrame();
  const toneColor = (tone?: TermLine['tone']): string =>
    tone === 'cmd' ? brand.cyan
    : tone === 'ok' ? STATUS.working
    : tone === 'warn' ? STATUS.needsYou
    : tone === 'err' ? STATUS.exited
    : tone === 'accent' ? t.accent
    : tone === 'text' ? t.text
    : t.textDim;
  return (
    <div
      style={{
        background: t.termBg,
        borderRadius: radius.m,
        padding: pad,
        fontFamily: fonts.mono,
        fontSize,
        lineHeight: 1.7,
        overflow: 'hidden',
        ...style,
      }}
    >
      {lines.map((l, i) => {
        const at = delay + i * step;
        const op = track(frame, [at, at + 7], [0, 1]);
        const x = track(frame, [at, at + 7], [-8, 0]);
        return (
          <div key={i} style={{ opacity: op, transform: `translateX(${x}px)`, color: toneColor(l.tone), whiteSpace: 'pre-wrap' }}>
            {l.text}
          </div>
        );
      })}
    </div>
  );
};

export interface DiffLine {
  text: string;
  kind?: 'add' | 'del' | 'ctx' | 'hunk';
}

export const Diff: React.FC<{ lines: DiffLine[]; t?: Theme; delay?: number; step?: number; fontSize?: number; style?: React.CSSProperties }> = ({
  lines,
  t = T,
  delay = 0,
  step = 4,
  fontSize = 14,
  style,
}) => {
  const frame = useCurrentFrame();
  return (
    <div
      style={{
        background: t.termBg,
        borderRadius: radius.m,
        overflow: 'hidden',
        fontFamily: fonts.mono,
        fontSize,
        lineHeight: 1.65,
        border: `1px solid ${t.border}`,
        ...style,
      }}
    >
      {lines.map((l, i) => {
        const at = delay + i * step;
        const op = track(frame, [at, at + 6], [0, 1]);
        const bg =
          l.kind === 'add' ? alpha(STATUS.working, 0.12)
          : l.kind === 'del' ? alpha(STATUS.exited, 0.12)
          : 'transparent';
        const fg =
          l.kind === 'add' ? '#7ee787'
          : l.kind === 'del' ? '#ff9a93'
          : l.kind === 'hunk' ? t.accent
          : t.textDim;
        const gutter = l.kind === 'add' ? '+' : l.kind === 'del' ? '-' : l.kind === 'hunk' ? '@' : ' ';
        return (
          <div key={i} style={{ display: 'flex', opacity: op, background: bg }}>
            <span style={{ width: 26, textAlign: 'center', color: alpha(fg, 0.7), flexShrink: 0 }}>{gutter}</span>
            <span style={{ color: fg, whiteSpace: 'pre', paddingRight: 14 }}>{l.text}</span>
          </div>
        );
      })}
    </div>
  );
};

// ── table ─────────────────────────────────────────────────────────────────────

export const Table: React.FC<{
  columns: string[];
  rows: (string | React.ReactNode)[][];
  t?: Theme;
  delay?: number;
  step?: number;
  widths?: (number | string)[];
  fontSize?: number;
  style?: React.CSSProperties;
}> = ({ columns, rows, t = T, delay = 0, step = 4, widths, fontSize = 13, style }) => {
  const frame = useCurrentFrame();
  const grid = (widths ?? columns.map(() => '1fr')).join(' ');
  return (
    <div style={{ borderRadius: radius.m, overflow: 'hidden', border: `1px solid ${t.border}`, background: t.surface, ...style }}>
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: grid,
          padding: '0 14px',
          height: 34,
          alignItems: 'center',
          background: t.surface2,
          borderBottom: `1px solid ${t.border}`,
          fontFamily: fonts.ui,
          fontSize: 11.5,
          fontWeight: 600,
          letterSpacing: 0.04,
          textTransform: 'uppercase',
          color: t.textDim,
        }}
      >
        {columns.map((c, i) => (
          <div key={i}>{c}</div>
        ))}
      </div>
      {rows.map((r, ri) => {
        const at = delay + ri * step;
        const op = track(frame, [at, at + 6], [0, 1]);
        const x = track(frame, [at, at + 6], [-10, 0]);
        return (
          <div
            key={ri}
            style={{
              display: 'grid',
              gridTemplateColumns: grid,
              padding: '0 14px',
              height: 36,
              alignItems: 'center',
              borderBottom: ri < rows.length - 1 ? `1px solid ${alpha(t.border, 0.6)}` : 'none',
              fontFamily: fonts.mono,
              fontSize,
              color: t.text,
              opacity: op,
              transform: `translateX(${x}px)`,
            }}
          >
            {r.map((cell, ci) => (
              <div key={ci} style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', paddingRight: 10 }}>
                {cell}
              </div>
            ))}
          </div>
        );
      })}
    </div>
  );
};

// Re-export logo helpers for convenience in compositions.
export { OttoIcon, OttoGlyph };
export { Icon };
export { navActive };
