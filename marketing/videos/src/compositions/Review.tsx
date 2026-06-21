import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, providers, status, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Stagger,
  Caption,
  TitleCard,
  Card,
  Chip,
  StatusDot,
  Icon,
  track,
} from '../components/kit';

// ── Scene 1 — title card ─────────────────────────────────────────────────────
const Title: React.FC = () => (
  <TitleCard
    kicker="Multi-Agent Code Review"
    title="A panel of reviewers, on every change"
    subtitle="Fan out agents per lens — findings you can act on"
  />
);

// ── Scene 2 — fan-out: a reviewer per lens ───────────────────────────────────
interface Lens {
  name: string;
  provider: keyof typeof providers;
  scan: string[];
  prog: number; // 0–1 target progress
}

const LENSES: Lens[] = [
  {
    name: 'Correctness',
    provider: 'claude',
    scan: ['middleware/jwt.go', 'handlers/auth.go'],
    prog: 0.82,
  },
  {
    name: 'Security',
    provider: 'agy',
    scan: ['db/players.go', 'querybuilder.go'],
    prog: 0.64,
  },
  {
    name: 'Performance',
    provider: 'codex',
    scan: ['loader/player.go', 'cache/redis.go'],
    prog: 0.71,
  },
  {
    name: 'Tests',
    provider: 'gemini',
    scan: ['auth/jwt_test.go', 'coverage report'],
    prog: 0.55,
  },
];

const ReviewerCard: React.FC<{ lens: Lens; delay: number }> = ({ lens, delay }) => {
  const frame = useCurrentFrame();
  const color = providers[lens.provider];
  const w = track(frame, [delay + 8, delay + 70], [0.06, lens.prog]);
  return (
    <Card
      pad={0}
      style={{
        flex: '1 1 calc(50% - 9px)',
        minWidth: 0,
        display: 'flex',
        flexDirection: 'column',
        overflow: 'hidden',
      }}
    >
      {/* header */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: '12px 14px',
          borderBottom: `1px solid ${T.border}`,
          background: alpha('#fff', 0.02),
        }}
      >
        <StatusDot kind="working" size={9} />
        <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 16, fontWeight: 700, color: T.text }}>
          {lens.name}
        </span>
        <Chip color={color}>{lens.provider}</Chip>
        <Chip tone="accent">working</Chip>
      </div>
      {/* body */}
      <div style={{ padding: 14, display: 'flex', flexDirection: 'column', gap: 11, flex: 1 }}>
        {/* progress bar */}
        <div
          style={{
            height: 7,
            borderRadius: 999,
            background: T.surface2,
            overflow: 'hidden',
          }}
        >
          <div
            style={{
              width: `${w * 100}%`,
              height: '100%',
              borderRadius: 999,
              background: `linear-gradient(90deg, ${alpha(color, 0.6)}, ${color})`,
              boxShadow: `0 0 10px ${alpha(color, 0.6)}`,
            }}
          />
        </div>
        {/* scan lines */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
          {lens.scan.map((s, i) => (
            <div
              key={s}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                fontFamily: fonts.mono,
                fontSize: 13,
                color: i === 0 ? T.text : T.textDim,
              }}
            >
              <Icon name={i === 0 ? 'search' : 'file'} size={13} color={i === 0 ? color : T.textDim} />
              <span style={{ whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                {i === 0 ? `scanning ${s}` : s}
              </span>
            </div>
          ))}
        </div>
      </div>
    </Card>
  );
};

const FanOut: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="git" counts={{ git: 4 }} />}
        title="Otto — Multi-Agent Review · sinatra-wallet-go"
      >
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%', boxSizing: 'border-box', padding: 22 }}>
          <Appear delay={4} y={14}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 18 }}>
              <Icon name="eye" size={20} color={brand.violet} />
              <span style={{ fontFamily: fonts.ui, fontSize: 22, fontWeight: 750 as never, color: T.text }}>
                Reviewing PR #482 · feat/wallet-rate-limit
              </span>
              <Chip tone="accent" style={{ marginLeft: 'auto' }}>
                4 agents live
              </Chip>
            </div>
          </Appear>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 18, flex: 1, alignContent: 'flex-start' }}>
            {LENSES.map((lens, i) => (
              <ReviewerCard key={lens.name} lens={lens} delay={16 + i * 12} />
            ))}
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Fan out a reviewer per lens"
      sub="Correctness · Security · Performance · Tests — each a live session"
    />
  </>
);

// ── Scene 3 — findings list ──────────────────────────────────────────────────
interface Finding {
  sev: 'High' | 'Med' | 'Low';
  sevTone: 'bad' | 'warn' | 'default';
  loc: string;
  msg: string;
  state: 'new' | 'approved' | 'fixed';
}

const FINDINGS: Finding[] = [
  {
    sev: 'High',
    sevTone: 'bad',
    loc: 'db/players.go:118',
    msg: 'SQL built via string concat — injection risk',
    state: 'new',
  },
  {
    sev: 'Med',
    sevTone: 'warn',
    loc: 'loader/player.go:54',
    msg: 'N+1 query in player loader',
    state: 'approved',
  },
  {
    sev: 'Med',
    sevTone: 'warn',
    loc: 'middleware/jwt.go:31',
    msg: 'missing nil-check on token',
    state: 'fixed',
  },
  {
    sev: 'Low',
    sevTone: 'default',
    loc: 'auth/jwt_test.go',
    msg: 'no test for expired JWT',
    state: 'new',
  },
  {
    sev: 'Low',
    sevTone: 'default',
    loc: 'cache/redis.go:77',
    msg: 'unbounded key scan on warmup',
    state: 'new',
  },
];

const stateChip = (state: Finding['state']) => {
  if (state === 'fixed') return <Chip tone="ok">fixed</Chip>;
  if (state === 'approved') return <Chip color={brand.cyan}>approved</Chip>;
  return <Chip tone="accent">new</Chip>;
};

const FindingRow: React.FC<{ f: Finding }> = ({ f }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 14,
      padding: '13px 16px',
      borderRadius: 10,
      background: T.surface2,
      border: `1px solid ${T.border}`,
    }}
  >
    <Chip tone={f.sevTone} style={{ minWidth: 52, justifyContent: 'center' }}>
      {f.sev}
    </Chip>
    <span
      style={{
        fontFamily: fonts.mono,
        fontSize: 13.5,
        color: T.textDim,
        minWidth: 168,
        whiteSpace: 'nowrap',
      }}
    >
      {f.loc}
    </span>
    <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 16, color: T.text, whiteSpace: 'nowrap' }}>
      {f.msg}
    </span>
    {stateChip(f.state)}
  </div>
);

const Findings: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="git" counts={{ git: 4 }} />}
        title="Otto — Multi-Agent Review · sinatra-wallet-go"
      >
        <div style={{ padding: 24, height: '100%', boxSizing: 'border-box' }}>
          <Card pad={20} style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 18 }}>
              <Icon name="eye" size={19} color={brand.violet} />
              <span style={{ fontFamily: fonts.ui, fontSize: 21, fontWeight: 750 as never, color: T.text }}>
                Findings (5)
              </span>
              <span style={{ fontFamily: fonts.ui, fontSize: 14, color: T.textDim, marginLeft: 4 }}>
                fingerprinted · stable across re-runs
              </span>
              <Chip tone="bad" style={{ marginLeft: 'auto' }}>
                1 blocker
              </Chip>
            </div>
            <Stagger delay={8} step={7} y={14} style={{ display: 'flex', flexDirection: 'column', gap: 11 }}>
              {FINDINGS.map((f) => (
                <FindingRow key={f.loc} f={f} />
              ))}
            </Stagger>
          </Card>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Fingerprinted findings with a lifecycle"
      sub="new → approved → fixed — stable across re-runs"
    />
  </>
);

// ── Scene 4 — merge-readiness + handoff ──────────────────────────────────────
const CheckRow: React.FC<{ label: string; ok?: boolean }> = ({ label, ok = true }) => (
  <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
    <span
      style={{
        width: 19,
        height: 19,
        borderRadius: '50%',
        background: ok ? alpha(status.working, 0.18) : alpha(status.needsYou, 0.18),
        display: 'grid',
        placeItems: 'center',
        flexShrink: 0,
      }}
    >
      <Icon name="check" size={12} color={ok ? status.working : status.needsYou} />
    </span>
    <span style={{ fontFamily: fonts.mono, fontSize: 14.5, color: T.text }}>{label}</span>
  </div>
);

const MergeReady: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="git" counts={{ git: 4 }} />}
        title="Otto — Multi-Agent Review · sinatra-wallet-go"
      >
        <div
          style={{
            display: 'flex',
            gap: 22,
            padding: 24,
            height: '100%',
            boxSizing: 'border-box',
            alignItems: 'stretch',
          }}
        >
          {/* merge-readiness dashboard */}
          <Appear delay={4} y={16} style={{ flex: 1.25, display: 'flex' }}>
            <Card pad={22} style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 18 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 11 }}>
                <Icon name="merge" size={20} color={brand.violet} />
                <span style={{ fontFamily: fonts.ui, fontSize: 21, fontWeight: 750 as never, color: T.text }}>
                  Merge readiness
                </span>
              </div>
              <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap' }}>
                <Chip tone="bad">1 blocker</Chip>
                <Chip tone="warn">Not mergeable</Chip>
                <Chip color={brand.cyan}>
                  <Icon name="arrowUp" size={11} /> 6 ahead · <Icon name="arrowDown" size={11} /> 0 behind
                </Chip>
              </div>
              <div
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 12,
                  padding: 16,
                  borderRadius: 10,
                  background: T.surface2,
                  border: `1px solid ${T.border}`,
                }}
              >
                <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 600, color: T.textDim, letterSpacing: 0.4 }}>
                  CI CHECKS
                </span>
                <CheckRow label="lint" />
                <CheckRow label="tests · 412 passed" />
                <CheckRow label="build" />
              </div>
            </Card>
          </Appear>

          {/* handoff + retry column */}
          <Appear delay={16} y={16} style={{ flex: 1, display: 'flex' }}>
            <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 18 }}>
              {/* handoff session card */}
              <Card pad={16} style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                  <StatusDot kind="working" size={10} />
                  <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 16, fontWeight: 700, color: T.text }}>
                    Handed to claude · fixing…
                  </span>
                  <Chip color={providers.claude}>claude</Chip>
                </div>
                <div
                  style={{
                    fontFamily: fonts.mono,
                    fontSize: 13,
                    color: T.textDim,
                    background: T.termBg,
                    borderRadius: 8,
                    padding: '11px 13px',
                    lineHeight: 1.7,
                    border: `1px solid ${T.border}`,
                  }}
                >
                  <div style={{ color: brand.cyan }}>$ fix db/players.go:118</div>
                  <div>parameterizing query…</div>
                  <div style={{ color: status.working }}>✓ patch staged · re-running Security</div>
                </div>
              </Card>
              {/* retry a reviewer */}
              <Card pad={16} style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                <Icon name="refresh" size={17} color={providers.codex} />
                <div style={{ flex: 1 }}>
                  <div style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 600, color: T.text }}>
                    Performance reviewer
                  </div>
                  <div style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>
                    retry this agent independently
                  </div>
                </div>
                <Chip color={providers.codex}>retry</Chip>
              </Card>
            </div>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="Merge-readiness at a glance"
      sub="Hand a finding to an agent to fix — or retry a reviewer"
    />
  </>
);

// ── Scene 5 — outro ──────────────────────────────────────────────────────────
const Outro: React.FC = () => (
  <WalkOutro
    title="Multi-Agent Review"
    tagline="Catch it before it merges."
    pills={[
      { label: 'Local & PR', color: '#0a84ff', icon: 'eye' },
      { label: 'Per-lens agents', color: brand.cyan, icon: 'grid' },
      { label: 'Lifecycle findings', color: '#28c840', icon: 'check' },
      { label: 'Merge-ready', color: brand.violet, icon: 'merge' },
      { label: 'Ultrareview', color: providers.claude, icon: 'zap' },
    ]}
  />
);

const SCENES: SceneDef[] = [
  { dur: 80, node: <Title />, name: 'Title' },
  { dur: 220, node: <FanOut />, name: 'Fan-out' },
  { dur: 220, node: <Findings />, name: 'Findings' },
  { dur: 190, node: <MergeReady />, name: 'Merge-readiness' },
  { dur: 130, node: <Outro />, name: 'Outro' },
];

export const reviewDuration = scenesDuration(SCENES);
export const Review: React.FC = () => <Scenes scenes={SCENES} />;
