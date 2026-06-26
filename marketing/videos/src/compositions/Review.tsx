import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, providers, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Caption,
  TitleCard,
  Card,
  Chip,
  Button,
  StatusDot,
  Segmented,
  Sparkline,
  Table,
  track,
  Icon,
} from '../components/kit';

// ── Scene 1 — Title card ──────────────────────────────────────────────────────
const TitleScene: React.FC = () => (
  <TitleCard
    kicker="AI Code Review"
    title="Code Review"
    subtitle="Parallel reviewers. Tracked findings. Real evidence."
  />
);

// ── Scene 2 — Fan-out: one agent per lens × provider ─────────────────────────

interface ReviewAgentCardProps {
  lens: string;
  provider: 'claude' | 'codex';
  statusKind: 'working' | 'idle';
  data: number[];
  color: string;
  delay: number;
  progress: number;
}

const ReviewAgentCard: React.FC<ReviewAgentCardProps> = ({
  lens,
  provider,
  statusKind,
  data,
  color,
  delay,
  progress,
}) => {
  const provColor = provider === 'claude' ? providers.claude : providers.codex;
  return (
    <Appear delay={delay} y={18}>
      <Card pad={18} style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
        {/* Header */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
          <StatusDot kind={statusKind} size={11} />
          <span
            style={{
              flex: 1,
              fontFamily: fonts.ui,
              fontSize: 16,
              fontWeight: 700,
              color: T.text,
              letterSpacing: -0.2,
            }}
          >
            {lens}
          </span>
          <Chip color={provColor}>{provider}</Chip>
          <Chip tone={statusKind === 'working' ? 'ok' : 'default'}>
            {statusKind === 'working' ? 'running' : 'complete'}
          </Chip>
        </div>
        {/* Sparkline */}
        <div
          style={{
            background: T.surface2,
            borderRadius: 8,
            padding: '10px 12px 6px',
            overflow: 'hidden',
          }}
        >
          <Sparkline data={data} color={color} width={560} height={56} progress={progress} />
        </div>
        {/* Footer */}
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <span style={{ fontFamily: fonts.mono, fontSize: 12, color: T.textDim }}>
            {statusKind === 'idle'
              ? '3 findings · analysis complete'
              : `${Math.round(progress * 100)}% analyzed`}
          </span>
          <Icon name="eye" size={13} color={color} />
        </div>
      </Card>
    </Appear>
  );
};

const FanOutScene: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <>
      <Stage scale={0.87}>
        <OttoWindow
          nav={<Navigator active="git" workingCount={3} />}
          title="Otto — review · PR #482 · add-jwt-refresh"
          tabs={[{ label: 'Review Agents', icon: 'eye', active: true, dot: 'working' }]}
        >
          <div
            style={{
              padding: '20px 22px',
              height: '100%',
              boxSizing: 'border-box',
              display: 'grid',
              gridTemplateColumns: '1fr 1fr',
              gridTemplateRows: '1fr 1fr',
              gap: 14,
            }}
          >
            <ReviewAgentCard
              lens="Security"
              provider="claude"
              statusKind="working"
              data={[8, 22, 35, 48, 57, 65, 73, 81, 88]}
              color={brand.violet}
              delay={8}
              progress={track(frame, [28, 136], [0.12, 0.88])}
            />
            <ReviewAgentCard
              lens="Correctness"
              provider="codex"
              statusKind="working"
              data={[5, 15, 27, 40, 53, 65, 74, 82]}
              color={providers.codex}
              delay={20}
              progress={track(frame, [40, 136], [0.10, 0.74])}
            />
            <ReviewAgentCard
              lens="Performance"
              provider="claude"
              statusKind="working"
              data={[10, 26, 40, 54, 66, 76, 84, 90]}
              color={providers.claude}
              delay={32}
              progress={track(frame, [52, 136], [0.10, 0.81])}
            />
            <ReviewAgentCard
              lens="Tests"
              provider="codex"
              statusKind="idle"
              data={[14, 30, 46, 60, 73, 84, 93, 100]}
              color={brand.cyan}
              delay={44}
              progress={1}
            />
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={1}
        title="One agent per lens × provider"
        sub="Fan-out over a PR or your working tree — Security, Correctness, Performance, Tests all run in parallel"
      />
    </>
  );
};

// ── Scene 3 — Findings list ───────────────────────────────────────────────────

const FindingsScene: React.FC = () => {
  const findingRows: (string | React.ReactNode)[][] = [
    [
      <Chip tone="bad">HIGH</Chip>,
      'middleware/jwt.go:42',
      'Security',
      <Chip>open</Chip>,
    ],
    [
      <Chip tone="warn">MED</Chip>,
      'repo/users.go:88',
      'Performance',
      <Chip tone="warn">triaged</Chip>,
    ],
    [
      <Chip tone="warn">MED</Chip>,
      'handlers/auth.go:17',
      'Tests',
      <Chip>open</Chip>,
    ],
    [
      <Chip tone="ok">LOW</Chip>,
      'api/routes.go:31',
      'Correctness',
      <Chip>open</Chip>,
    ],
  ];

  return (
    <>
      <Stage scale={0.87}>
        <OttoWindow
          nav={<Navigator active="git" />}
          title="Otto — review · PR #482 · add-jwt-refresh"
          tabs={[{ label: 'Findings', icon: 'eye', active: true }]}
        >
          <div style={{ padding: '20px 22px' }}>
            {/* Summary banner */}
            <Appear delay={4} y={12}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 10,
                  marginBottom: 18,
                  padding: '12px 16px',
                  background: T.surface,
                  border: `1px solid ${T.border}`,
                  borderRadius: 10,
                }}
              >
                <Icon name="eye" size={16} color={brand.violet} />
                <span
                  style={{
                    flex: 1,
                    fontFamily: fonts.ui,
                    fontSize: 14,
                    fontWeight: 700,
                    color: T.text,
                  }}
                >
                  Review Findings — PR #482
                </span>
                <Chip tone="bad">4 issues</Chip>
                <Chip tone="ok">2 agents done</Chip>
                <Chip>2 running</Chip>
              </div>
            </Appear>
            <Table
              columns={['Severity', 'File : Line', 'Lens', 'State']}
              rows={findingRows}
              widths={['100px', '1fr', '130px', '120px']}
              delay={14}
              step={14}
              fontSize={13}
            />
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={2}
        title="Findings ranked and de-duplicated"
        sub="Merged across all lens agents — severity, file location, lens, and lifecycle state at a glance"
      />
    </>
  );
};

// ── Scene 4 — Tracked finding detail ─────────────────────────────────────────

const LIFECYCLE_STATES = ['open', 'triaged', 'in progress', 'fixed', 'verified', 'resolved'];

const TrackedFindingScene: React.FC = () => (
  <>
    <Stage scale={0.87}>
      <OttoWindow
        nav={<Navigator active="git" />}
        title="Otto — review · PR #482 · Finding"
        tabs={[
          { label: 'Findings', icon: 'eye' },
          { label: 'Missing JWT exp check', icon: 'file', active: true, dot: 'needsYou' },
        ]}
      >
        <div
          style={{
            padding: '22px 28px',
            display: 'flex',
            flexDirection: 'column',
            gap: 18,
            height: '100%',
            boxSizing: 'border-box',
          }}
        >
          {/* Finding header card */}
          <Appear delay={4} y={16}>
            <Card pad={20} style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 14 }}>
                <Chip tone="bad" style={{ marginTop: 3, flexShrink: 0 }}>
                  HIGH
                </Chip>
                <div style={{ flex: 1 }}>
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 18,
                      fontWeight: 700,
                      color: T.text,
                      letterSpacing: -0.3,
                      marginBottom: 6,
                    }}
                  >
                    Missing JWT expiry check
                  </div>
                  <div style={{ fontFamily: fonts.mono, fontSize: 12.5, color: T.textDim }}>
                    middleware/jwt.go:42 · Security · claude
                  </div>
                </div>
              </div>
              <div
                style={{
                  borderTop: `1px solid ${alpha('#ffffff', 0.08)}`,
                  paddingTop: 14,
                  fontFamily: fonts.ui,
                  fontSize: 14,
                  color: T.textDim,
                  lineHeight: 1.72,
                }}
              >
                JWT tokens are issued without an{' '}
                <span
                  style={{
                    fontFamily: fonts.mono,
                    color: T.accent,
                    background: alpha(T.accent, 0.12),
                    padding: '1px 6px',
                    borderRadius: 4,
                  }}
                >
                  exp
                </span>{' '}
                claim — any token minted by this handler never expires.
                Session hijack and privilege escalation risk if a token leaks.
              </div>
            </Card>
          </Appear>

          {/* 6-state lifecycle pipeline */}
          <Appear delay={18} y={14}>
            <Card pad={18} style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
              <span
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 11,
                  fontWeight: 600,
                  color: T.textDim,
                  textTransform: 'uppercase' as const,
                  letterSpacing: 1,
                }}
              >
                Lifecycle
              </span>
              <Segmented options={LIFECYCLE_STATES} active={1} />
              <span style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>
                State changed to{' '}
                <span style={{ color: T.text, fontWeight: 600 }}>triaged</span>
                {' '}— assigned to sprint backlog, awaiting fix.
              </span>
            </Card>
          </Appear>

          {/* Action buttons */}
          <Appear delay={30} y={12}>
            <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap' as const, alignItems: 'center' }}>
              <Button icon="tag">Triage</Button>
              <Button icon="zap" variant="primary">Fix with agent</Button>
              <Button icon="x" variant="ghost">Dismiss</Button>
              <Button icon="check">Ingest → Proof Pack</Button>
              <Button icon="globe" variant="ghost">Save → Vault</Button>
            </div>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="Every finding is tracked"
      sub="Triage, fix with an agent, prove it's done, remember it — a full lifecycle in one view"
    />
  </>
);

// ── Scene list ────────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 80,  node: <TitleScene />,          name: 'Title'          },
  { dur: 150, node: <FanOutScene />,          name: 'FanOut'         },
  { dur: 140, node: <FindingsScene />,        name: 'Findings'       },
  { dur: 120, node: <TrackedFindingScene />,  name: 'TrackedFinding' },
  {
    dur: 130,
    name: 'Outro',
    node: (
      <WalkOutro
        title="AI Code Review"
        tagline="Parallel reviewers, tracked findings, real evidence"
        pills={[
          { label: 'Per-lens agents',    icon: 'eye'    },
          { label: 'PR or working tree', icon: 'branch' },
          { label: 'Tracked findings',   icon: 'check'  },
          { label: 'Proof + Vault',      icon: 'globe'  },
        ]}
      />
    ),
  },
];

export const reviewDuration = scenesDuration(SCENES);
export const Review: React.FC = () => <Scenes scenes={SCENES} />;
