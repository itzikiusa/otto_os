import React from 'react';
import { AbsoluteFill, useCurrentFrame, interpolate } from 'remotion';
import { T, brand, fonts, alpha, providers } from '../theme';
import { Scenes, scenesDuration, Stage, SceneDef } from '../components/scene';
import { OttoIcon } from '../components/OttoLogo';
import { OttoWindow } from '../components/Frame';
import { Navigator, NavSession } from '../components/Nav';
import {
  Appear,
  Kicker,
  BrandWord,
  Caption,
  FeaturePill,
  Terminal,
  StatusDot,
  Keys,
  Chip,
} from '../components/kit';

// ── Scene 1 — brand cold open ────────────────────────────────────────────────
const Open: React.FC = () => {
  const frame = useCurrentFrame();
  const ring = interpolate(frame, [30, 90], [0, 520], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' });
  return (
    <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
      <Appear delay={2} scale={0.62} y={0} style={{ marginBottom: 30 }}>
        <OttoIcon size={150} glowPx={110} />
      </Appear>
      <div style={{ marginBottom: 18 }}>
        <Kicker delay={14}>Agentic Development Environment</Kicker>
      </div>
      <BrandWord delay={20} size={118}>Otto</BrandWord>
      <Appear delay={32} y={14}>
        <div style={{ fontFamily: fonts.ui, fontSize: 30, color: alpha('#fff', 0.66), marginTop: 16 }}>
          Run your coding agents <span style={{ color: brand.cyan, fontWeight: 700 }}>like a pro.</span>
        </div>
      </Appear>
      <div
        style={{
          position: 'absolute',
          bottom: '30%',
          width: ring,
          height: 1,
          background: `linear-gradient(90deg, transparent, ${alpha(brand.cyan, 0.7)}, transparent)`,
        }}
      />
    </AbsoluteFill>
  );
};

// ── Scene 2 — the real window, agents working ────────────────────────────────
const sessions: NavSession[] = [
  { title: 'fix auth tests', provider: 'claude', status: 'working', tasks: [2, 4] },
  { title: 'refactor api/v2', provider: 'codex', status: 'working', tasks: [1, 3] },
  { title: 'add rate-limit', provider: 'claude', status: 'idle', tasks: [3, 3] },
];

const AgentPane: React.FC<{ name: string; color: string; lines: { text: string; tone?: never }[]; live?: boolean }> = ({
  name,
  color,
  lines,
  live = true,
}) => (
  <div style={{ flex: 1, display: 'flex', flexDirection: 'column', background: T.termBg, border: `1px solid ${T.border}`, borderRadius: 10, overflow: 'hidden' }}>
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '9px 12px', borderBottom: `1px solid ${T.border}`, background: alpha('#fff', 0.02) }}>
      <StatusDot kind={live ? 'working' : 'idle'} size={9} />
      <span style={{ flex: 1, fontFamily: fonts.mono, fontSize: 13, fontWeight: 600, color: T.text }}>{name}</span>
      <Chip color={color}>{name.split(' ')[0]}</Chip>
    </div>
    <Terminal lines={lines as never} delay={18} step={9} pad={14} fontSize={14} style={{ flex: 1, background: 'transparent', borderRadius: 0 }} />
  </div>
);

const WindowScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="agents" sessions={sessions} activeSessionTitle="fix auth tests" workingCount={2} />}
        tabs={[
          { label: 'fix auth tests', icon: 'terminal', active: true, dot: 'working' },
          { label: 'refactor api/v2', icon: 'terminal', dot: 'working' },
        ]}
        title="Otto — sinatra-users-go"
      >
        <div style={{ display: 'flex', gap: 12, padding: 16, height: '100%', boxSizing: 'border-box' }}>
          <AgentPane
            name="claude · fix auth tests"
            color={providers.claude}
            lines={[
              { text: '$ go test ./auth/...' },
              { text: '  reading handler.go, jwt.go…' },
              { text: '  ✗ 3 failing — missing JWT validation' },
              { text: '  applying fix → middleware/jwt.go' },
              { text: '  ✓ 142 passed (3.4s)' },
            ] as never}
          />
          <AgentPane
            name="codex · refactor api/v2"
            color={providers.codex}
            lines={[
              { text: '$ codex run task.md' },
              { text: '  editing server.go, routes.go…' },
              { text: '  writing tests/api_test.go' },
              { text: '  ✓ build ok · 0 issues' },
              { text: '  drafting PR…' },
            ] as never}
          />
        </div>
      </OttoWindow>
    </Stage>
    <Caption step={1} title="Claude Code, Codex & shell — as first-class sessions" sub="Watch them work, type in live, run many at once. Resumable on the daemon." />
  </>
);

// ── Scene 3 — one window, the whole workflow (all pillars) ───────────────────
const PILLARS: { label: string; color: string; icon: string }[] = [
  { label: 'Agent Sessions', color: providers.claude, icon: 'terminal' },
  { label: 'Git & Pull Requests', color: '#28c840', icon: 'branch' },
  { label: 'Multi-Agent Review', color: brand.violet, icon: 'eye' },
  { label: 'Jira / Confluence', color: '#2684ff', icon: 'note' },
  { label: 'Database Explorer', color: '#0a84ff', icon: 'db' },
  { label: 'Kafka Brokers', color: '#febc2e', icon: 'box' },
  { label: 'Agent Swarm', color: brand.cyan, icon: 'grid' },
  { label: 'SSH · SQL · Redis · Mongo', color: '#bf7aff', icon: 'plug' },
  { label: 'Slack & Telegram', color: '#36c5f0', icon: 'slack' },
  { label: 'Usage & Budgets', color: '#ff8a65', icon: 'chart' },
  { label: 'Workflows', color: '#9ee039', icon: 'split' },
  { label: 'Custom Plugins', color: '#a78bfa', icon: 'zap' },
  { label: 'Knowledge Vault', color: '#47bfff', icon: 'globe' },
  { label: 'Remote & Mobile', color: '#0a84ff', icon: 'share' },
];

const PillarsScene: React.FC = () => (
  <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center', padding: '0 120px' }}>
    <div style={{ marginBottom: 14 }}>
      <Kicker delay={2}>One window</Kicker>
    </div>
    <Appear delay={8} y={18}>
      <div style={{ fontFamily: fonts.ui, fontSize: 60, fontWeight: 800, letterSpacing: -1.5, color: '#fff', textAlign: 'center', lineHeight: 1.08 }}>
        Your whole engineering workflow,
        <br />
        <span style={{ backgroundImage: brand.gradSoft, WebkitBackgroundClip: 'text', backgroundClip: 'text', color: 'transparent', WebkitTextFillColor: 'transparent' }}>
          wired into one place.
        </span>
      </div>
    </Appear>
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 14, justifyContent: 'center', maxWidth: 1500, marginTop: 44 }}>
      {PILLARS.map((p, i) => (
        <FeaturePill key={p.label} label={p.label} color={p.color} icon={p.icon} delay={26 + i * 4} />
      ))}
    </div>
  </AbsoluteFill>
);

// ── Scene 4 — outro lockup + launch hint ─────────────────────────────────────
const Lockup: React.FC = () => (
  <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
    <Appear delay={2} scale={0.7} y={0} style={{ marginBottom: 26 }}>
      <OttoIcon size={128} glowPx={100} />
    </Appear>
    <Appear delay={10} y={20}>
      <div style={{ fontFamily: fonts.ui, fontSize: 80, fontWeight: 800, letterSpacing: -2, color: '#fff', textAlign: 'center', lineHeight: 1.05 }}>
        Your agents,{' '}
        <span style={{ backgroundImage: brand.gradSoft, WebkitBackgroundClip: 'text', backgroundClip: 'text', color: 'transparent', WebkitTextFillColor: 'transparent' }}>
          orchestrated.
        </span>
      </div>
    </Appear>
    <Appear delay={20} y={14}>
      <div style={{ fontFamily: fonts.ui, fontSize: 27, color: alpha('#fff', 0.62), marginTop: 16 }}>
        Otto — the Agentic Development Environment
      </div>
    </Appear>
    <div style={{ marginTop: 44, display: 'flex', alignItems: 'center', gap: 16 }}>
      <Appear delay={32}><span style={{ fontFamily: fonts.ui, fontSize: 22, color: alpha('#fff', 0.6) }}>Press</span></Appear>
      <Keys keys={['⌘', 'K']} delay={36} />
      <Appear delay={40}><span style={{ fontFamily: fonts.ui, fontSize: 22, color: alpha('#fff', 0.6) }}>to launch anything</span></Appear>
    </div>
  </AbsoluteFill>
);

const SCENES: SceneDef[] = [
  { dur: 120, node: <Open />, name: 'Open' },
  { dur: 196, node: <WindowScene />, name: 'Window' },
  { dur: 200, node: <PillarsScene />, name: 'Pillars' },
  { dur: 124, node: <Lockup />, name: 'Lockup' },
];

export const introDuration = scenesDuration(SCENES);
export const Intro: React.FC = () => <Scenes scenes={SCENES} />;
