import React from 'react';
import { AbsoluteFill, Sequence, useCurrentFrame, useVideoConfig, interpolate, spring, Easing } from 'remotion';
import { brand, fonts, alpha } from '../theme';
import { Background, Appear, FeaturePill } from './kit';
import { OttoIcon } from './OttoLogo';

// ════════════════════════════════════════════════════════════════════════════
//  SCENE SEQUENCER
//
//  Compositions are a list of timed scenes. The registered durationInFrames is
//  ALWAYS exactly the sum of scene durations — so there is never a blank tail.
//  Scenes crossfade; the last scene holds to the final frame (never fades out).
// ════════════════════════════════════════════════════════════════════════════

export interface SceneDef {
  dur: number; // frames
  node: React.ReactNode;
  name?: string;
}

export const scenesDuration = (scenes: SceneDef[]): number => scenes.reduce((a, s) => a + s.dur, 0);

const SceneWrap: React.FC<{ children: React.ReactNode; dur: number; xfade: number; last: boolean }> = ({
  children,
  dur,
  xfade,
  last,
}) => {
  const frame = useCurrentFrame();
  const fin = interpolate(frame, [0, xfade], [0, 1], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp', easing: Easing.out(Easing.cubic) });
  const fout = last ? 1 : interpolate(frame, [dur, dur + xfade], [1, 0], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp', easing: Easing.in(Easing.cubic) });
  const scale = interpolate(frame, [0, xfade], [0.992, 1], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp', easing: Easing.out(Easing.cubic) });
  return (
    <AbsoluteFill style={{ opacity: Math.min(fin, fout), transform: `scale(${scale})` }}>{children}</AbsoluteFill>
  );
};

export const Scenes: React.FC<{ scenes: SceneDef[]; xfade?: number; bg?: React.ReactNode; grid?: boolean }> = ({
  scenes,
  xfade = 14,
  bg,
  grid = true,
}) => {
  let cum = 0;
  const starts = scenes.map((s) => {
    const v = cum;
    cum += s.dur;
    return v;
  });
  return (
    <AbsoluteFill style={{ fontFamily: fonts.ui }}>
      {bg ?? <Background grid={grid} />}
      {scenes.map((s, i) => {
        const last = i === scenes.length - 1;
        return (
          <Sequence key={i} from={starts[i]} durationInFrames={s.dur + (last ? 0 : xfade)} name={s.name}>
            <SceneWrap dur={s.dur} xfade={xfade} last={last}>
              {s.node}
            </SceneWrap>
          </Sequence>
        );
      })}
    </AbsoluteFill>
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  STAGE — centers a device mockup on the cinematic bg with a gentle lift/float,
//  so even a "holding" frame stays subtly alive.
// ════════════════════════════════════════════════════════════════════════════

export const Stage: React.FC<{
  children: React.ReactNode;
  scale?: number;
  enter?: 'up' | 'fade' | 'none';
  float?: boolean;
  y?: number;
  style?: React.CSSProperties;
}> = ({ children, scale = 1, enter = 'up', float = true, y = 0, style }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const s = spring({ frame, fps, config: { damping: 26, stiffness: 90 } });
  const inY = enter === 'up' ? interpolate(s, [0, 1], [70, 0]) : 0;
  const inScale = enter === 'none' ? 1 : interpolate(s, [0, 1], [0.965, 1]);
  const op = enter === 'none' ? 1 : interpolate(frame, [0, 16], [0, 1], { extrapolateRight: 'clamp' });
  const driftY = float ? Math.sin(frame / 50) * 5 : 0;
  return (
    <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center', ...style }}>
      <div style={{ opacity: op, transform: `translateY(${inY + driftY + y}px) scale(${scale * inScale})` }}>{children}</div>
    </AbsoluteFill>
  );
};

// A soft brand glow puck under a centered subject.
export const FloorGlow: React.FC<{ color?: string; w?: number }> = ({ color = brand.purple, w = 900 }) => (
  <div
    style={{
      position: 'absolute',
      bottom: '12%',
      left: '50%',
      transform: 'translateX(-50%)',
      width: w,
      height: 220,
      borderRadius: '50%',
      background: `radial-gradient(closest-side, ${alpha(color, 0.28)}, transparent)`,
      filter: 'blur(8px)',
    }}
  />
);

// ════════════════════════════════════════════════════════════════════════════
//  PER-FEATURE OUTRO — a clean closing card (never blank). Pairs with a Caption
//  earlier; reinforces the feature + brand at the tail of each walkthrough.
// ════════════════════════════════════════════════════════════════════════════

export const WalkOutro: React.FC<{
  title: string;
  tagline?: string;
  pills?: { label: string; color?: string; icon?: string }[];
  accent?: string;
}> = ({ title, tagline, pills, accent = brand.cyan }) => (
  <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
    <Appear delay={2} scale={0.72} y={0} style={{ marginBottom: 26 }}>
      <OttoIcon size={112} />
    </Appear>
    <Appear delay={10} y={20}>
      <div style={{ fontFamily: fonts.ui, fontSize: 64, fontWeight: 800, letterSpacing: -1.5, color: '#fff', textAlign: 'center' }}>{title}</div>
    </Appear>
    {tagline && (
      <Appear delay={18} y={16}>
        <div style={{ fontFamily: fonts.ui, fontSize: 27, color: alpha('#fff', 0.66), marginTop: 14, textAlign: 'center', maxWidth: 1000 }}>{tagline}</div>
      </Appear>
    )}
    {pills && pills.length > 0 && (
      <div style={{ display: 'flex', gap: 14, marginTop: 34, flexWrap: 'wrap', justifyContent: 'center', maxWidth: 1200 }}>
        {pills.map((p, i) => (
          <FeaturePill key={i} label={p.label} color={p.color ?? accent} icon={p.icon} delay={26 + i * 5} />
        ))}
      </div>
    )}
  </AbsoluteFill>
);

// Convenience: build a composition component + its exact duration from scenes.
export function defineWalk(scenes: SceneDef[], opts?: { xfade?: number; grid?: boolean; bg?: React.ReactNode }) {
  const durationInFrames = scenesDuration(scenes);
  const Comp: React.FC = () => <Scenes scenes={scenes} xfade={opts?.xfade} grid={opts?.grid} bg={opts?.bg} />;
  return { Comp, durationInFrames };
}
