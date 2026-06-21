import React from 'react';
import { AbsoluteFill, useCurrentFrame, interpolate } from 'remotion';
import { brand, fonts, alpha, providers } from '../theme';
import { Scenes, SceneDef, scenesDuration } from '../components/scene';
import { OttoIcon } from '../components/OttoLogo';
import { Appear, Kicker, FeaturePill, Keys, Icon } from '../components/kit';

// ── Scene 1 — kinetic recap of the whole surface area ────────────────────────
const RECAP: { label: string; color: string; icon: string }[] = [
  { label: 'Agent Sessions', color: providers.claude, icon: 'terminal' },
  { label: 'Git & PRs', color: '#28c840', icon: 'branch' },
  { label: 'Code Review', color: brand.violet, icon: 'eye' },
  { label: 'Product', color: '#2684ff', icon: 'note' },
  { label: 'Database', color: '#0a84ff', icon: 'db' },
  { label: 'Kafka', color: '#febc2e', icon: 'box' },
  { label: 'Swarm', color: brand.cyan, icon: 'grid' },
  { label: 'Connections', color: '#bf7aff', icon: 'plug' },
  { label: 'Channels', color: '#36c5f0', icon: 'slack' },
  { label: 'Usage & Budgets', color: '#ff8a65', icon: 'chart' },
  { label: 'Workflows', color: '#9ee039', icon: 'split' },
  { label: 'Plugins', color: '#a78bfa', icon: 'zap' },
  { label: 'Vault', color: '#47bfff', icon: 'globe' },
  { label: 'Remote & Mobile', color: '#0a84ff', icon: 'share' },
];

const Recap: React.FC = () => (
  <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center', padding: '0 120px' }}>
    <div style={{ marginBottom: 16 }}>
      <Kicker delay={2}>One window for all of it</Kicker>
    </div>
    <Appear delay={8} y={18}>
      <div style={{ fontFamily: fonts.ui, fontSize: 62, fontWeight: 800, letterSpacing: -1.6, color: '#fff', textAlign: 'center', lineHeight: 1.06 }}>
        Your agents. Your stack.
        <br />
        <span style={{ backgroundImage: brand.gradSoft, WebkitBackgroundClip: 'text', backgroundClip: 'text', color: 'transparent', WebkitTextFillColor: 'transparent' }}>
          One Otto.
        </span>
      </div>
    </Appear>
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 13, justifyContent: 'center', maxWidth: 1480, marginTop: 42 }}>
      {RECAP.map((p, i) => (
        <FeaturePill key={p.label} label={p.label} color={p.color} icon={p.icon} delay={24 + i * 3} />
      ))}
    </div>
  </AbsoluteFill>
);

// ── Scene 2 — final CTA lockup ───────────────────────────────────────────────
const CTA: React.FC = () => {
  const frame = useCurrentFrame();
  const line = interpolate(frame, [40, 90], [0, 460], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' });
  return (
    <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
      <Appear delay={2} scale={0.66} y={0} style={{ marginBottom: 30 }}>
        <OttoIcon size={158} glowPx={120} />
      </Appear>
      <Appear delay={12} y={20}>
        <div style={{ fontFamily: fonts.ui, fontSize: 112, fontWeight: 800, letterSpacing: -3, lineHeight: 1, backgroundImage: brand.gradSoft, WebkitBackgroundClip: 'text', backgroundClip: 'text', color: 'transparent', WebkitTextFillColor: 'transparent' }}>
          Otto
        </div>
      </Appear>
      <Appear delay={22} y={14}>
        <div style={{ fontFamily: fonts.ui, fontSize: 29, color: alpha('#fff', 0.66), marginTop: 16 }}>
          The Agentic Development Environment
        </div>
      </Appear>
      <div style={{ height: 1, width: line, marginTop: 30, background: `linear-gradient(90deg, transparent, ${alpha(brand.cyan, 0.7)}, transparent)` }} />
      <div style={{ marginTop: 30, display: 'flex', alignItems: 'center', gap: 18 }}>
        <Appear delay={40}>
          <div style={{ display: 'inline-flex', alignItems: 'center', gap: 9, padding: '12px 20px', borderRadius: 12, background: alpha('#fff', 0.06), border: `1px solid ${alpha('#fff', 0.14)}`, fontFamily: fonts.ui, fontSize: 20, color: '#fff' }}>
            <Icon name="command" size={18} color={brand.cyan} /> macOS desktop app
          </div>
        </Appear>
        <Appear delay={46}><span style={{ fontFamily: fonts.ui, fontSize: 20, color: alpha('#fff', 0.55) }}>press</span></Appear>
        <Keys keys={['⌘', 'K']} delay={50} />
        <Appear delay={56}><span style={{ fontFamily: fonts.ui, fontSize: 20, color: alpha('#fff', 0.55) }}>to begin</span></Appear>
      </div>
    </AbsoluteFill>
  );
};

const SCENES: SceneDef[] = [
  { dur: 168, node: <Recap />, name: 'Recap' },
  { dur: 192, node: <CTA />, name: 'CTA' },
];

export const outroDuration = scenesDuration(SCENES);
export const Outro: React.FC = () => <Scenes scenes={SCENES} />;
