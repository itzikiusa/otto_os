import React from 'react';
import { AbsoluteFill, useCurrentFrame } from 'remotion';
import { T, brand, fonts, status as statusColors, alpha } from '../theme';
import {
  Scenes,
  SceneDef,
  scenesDuration,
  Stage,
  WalkOutro,
  FloorGlow,
} from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  track,
  Caption,
  TitleCard,
  Chip,
  Card,
  Toggle,
  Toast,
  Icon,
  StatusDot,
  Kicker,
} from '../components/kit';

// ── Scene 1 — Title (~80f) ────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Proof Packs"
    title="Proof Packs"
    subtitle="No 'done' without evidence — every task backs its claim with a verifiable pack of artifacts."
  />
);

// ── Scene 2 — The Proof Pack (~180f) ─────────────────────────────────────

type ArtifactKind = 'tests' | 'diff' | 'pr' | 'screenshot';

interface Artifact {
  icon: string;
  label: string;
  detail: string;
  kind: ArtifactKind;
  kindColor: string;
  iconColor: string;
}

const ARTIFACTS: Artifact[] = [
  {
    icon: 'check',
    label: '142 tests passed',
    detail: 'go test ./... (3.2s)',
    kind: 'tests',
    kindColor: statusColors.working,
    iconColor: statusColors.working,
  },
  {
    icon: 'branch',
    label: 'diff  +63 −12',
    detail: 'middleware/jwt_validate.go',
    kind: 'diff',
    kindColor: T.accent,
    iconColor: T.textDim,
  },
  {
    icon: 'pr',
    label: 'PR #482 · feat/auth-middleware',
    detail: 'github.com/acme/sinatra-go',
    kind: 'pr',
    kindColor: brand.purple,
    iconColor: brand.purple,
  },
  {
    icon: 'eye',
    label: 'screenshot  login.png',
    detail: 'login flow verified',
    kind: 'screenshot',
    kindColor: brand.cyan,
    iconColor: brand.cyan,
  },
];

const ProofPackScene: React.FC = () => (
  <>
    <Stage scale={0.87} y={-28}>
      <OttoWindow
        nav={<Navigator active="proof" />}
        title="Otto — Proof · fix auth tests"
      >
        <div
          style={{
            padding: '18px 22px',
            display: 'flex',
            flexDirection: 'column',
            gap: 14,
            height: '100%',
            boxSizing: 'border-box',
          }}
        >
          {/* Pack header */}
          <Appear delay={6}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 12,
                paddingBottom: 14,
                borderBottom: `1px solid ${T.border}`,
              }}
            >
              <StatusDot kind="working" size={10} />
              <span
                style={{
                  fontFamily: fonts.ui,
                  fontWeight: 700,
                  fontSize: 16,
                  color: T.text,
                  flex: 1,
                }}
              >
                fix-auth-tests · Proof Pack
              </span>
              <Chip
                tone="ok"
                style={{
                  fontSize: 13,
                  height: 26,
                  padding: '0 14px',
                  letterSpacing: 1.2,
                  fontWeight: 700,
                }}
              >
                PROVEN
              </Chip>
              <Chip tone="warn">risk: low</Chip>
            </div>
          </Appear>

          {/* Artifact rows */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            {ARTIFACTS.map((a, i) => (
              <Appear key={a.kind} delay={20 + i * 24} y={14}>
                <Card
                  pad={14}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 12,
                  }}
                >
                  <Icon name={a.icon} size={16} color={a.iconColor} />
                  <span
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 14,
                      fontWeight: 600,
                      color: T.text,
                      flex: 1,
                    }}
                  >
                    {a.label}
                  </span>
                  <span
                    style={{
                      fontFamily: fonts.mono,
                      fontSize: 12,
                      color: T.textDim,
                      maxWidth: 300,
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {a.detail}
                  </span>
                  <Chip color={a.kindColor}>{a.kind}</Chip>
                </Card>
              </Appear>
            ))}
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="No 'done' without evidence"
      sub="Artifacts accumulate into a Proof Pack — test output, diffs, PR links & screenshots."
    />
  </>
);

// ── Scene 3 — Derive + Redact (~150f) ─────────────────────────────────────

const GATES: { label: string; passed: boolean }[] = [
  { label: 'Tests',     passed: true  },
  { label: 'Diff',      passed: true  },
  { label: 'PR',        passed: true  },
  { label: 'Review',    passed: true  },
  { label: 'Goal-loop', passed: false },
];

const DeriveScene: React.FC = () => {
  const frame = useCurrentFrame();

  return (
    <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center', padding: '0 140px' }}>
      <div style={{ marginBottom: 20 }}>
        <Kicker delay={4}>Pure derivation</Kicker>
      </div>

      <Appear delay={12} y={24}>
        <div
          style={{
            fontFamily: fonts.ui,
            fontSize: 52,
            fontWeight: 800,
            letterSpacing: -1.5,
            color: '#fff',
            textAlign: 'center',
            lineHeight: 1.1,
          }}
        >
          Status, risk & badges{' '}
          <span
            style={{
              backgroundImage: brand.gradSoft,
              WebkitBackgroundClip: 'text',
              backgroundClip: 'text',
              color: 'transparent',
              WebkitTextFillColor: 'transparent',
            }}
          >
            derived automatically
          </span>
        </div>
      </Appear>

      <Appear delay={22} y={16}>
        <div
          style={{
            fontFamily: fonts.ui,
            fontSize: 22,
            color: alpha('#fff', 0.6),
            marginTop: 16,
            textAlign: 'center',
          }}
        >
          Pure functions — no LLM, no guesswork. Only the evidence decides.
        </div>
      </Appear>

      {/* 5 gates */}
      <div
        style={{
          display: 'flex',
          gap: 20,
          marginTop: 40,
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        {GATES.map((g, i) => {
          const gateOp = track(frame, [32 + i * 10, 42 + i * 10], [0, 1]);
          const gateY = track(frame, [32 + i * 10, 42 + i * 10], [18, 0]);
          return (
            <div
              key={g.label}
              style={{
                opacity: gateOp,
                transform: `translateY(${gateY}px)`,
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'center',
                gap: 10,
              }}
            >
              <div
                style={{
                  width: 58,
                  height: 58,
                  borderRadius: 14,
                  display: 'grid',
                  placeItems: 'center',
                  background: g.passed
                    ? alpha(statusColors.working, 0.15)
                    : alpha(statusColors.needsYou, 0.12),
                  border: `1px solid ${
                    g.passed
                      ? alpha(statusColors.working, 0.45)
                      : alpha(statusColors.needsYou, 0.35)
                  }`,
                }}
              >
                <Icon
                  name={g.passed ? 'check' : 'clock'}
                  size={24}
                  color={g.passed ? statusColors.working : statusColors.needsYou}
                />
              </div>
              <span
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 13,
                  fontWeight: 600,
                  color: g.passed ? statusColors.working : statusColors.needsYou,
                  letterSpacing: 0.2,
                }}
              >
                {g.label}
              </span>
            </div>
          );
        })}
      </div>

      {/* Redaction notice */}
      <Appear delay={90} y={14}>
        <div
          style={{
            marginTop: 40,
            display: 'flex',
            alignItems: 'center',
            gap: 12,
            padding: '12px 22px',
            borderRadius: 10,
            background: alpha(brand.violet, 0.12),
            border: `1px solid ${alpha(brand.violet, 0.32)}`,
          }}
        >
          <Icon name="eye" size={16} color={brand.violet} />
          <span
            style={{
              fontFamily: fonts.mono,
              fontSize: 14,
              color: alpha('#fff', 0.85),
              letterSpacing: 0.5,
            }}
          >
            secrets redacted · 2 MiB cap
          </span>
        </div>
      </Appear>

      <Caption
        step={2}
        title="Pure rules derive status, risk & badges"
        sub="Artifacts redacted & capped — no secret leaks, no bloated packs."
        delay={18}
      />

      <FloorGlow color={brand.violet} w={700} />
    </AbsoluteFill>
  );
};

// ── Scene 4 — Require config (~130f) ──────────────────────────────────────

interface EnvSetting {
  name: string;
  description: string;
  on: boolean;
}

const SETTINGS: EnvSetting[] = [
  {
    name: 'OTTO_PROOF_REQUIRE_PR',
    description: 'Block task completion until a PR is linked',
    on: true,
  },
  {
    name: 'OTTO_PROOF_REQUIRE_GOAL_LOOP',
    description: 'Require a verified goal loop before closing',
    on: false,
  },
  {
    name: 'OTTO_PROOF_AUTO_TEST',
    description: 'Automatically run tests and attach output to the pack',
    on: false,
  },
];

const RequireScene: React.FC = () => (
  <>
    <Stage scale={0.87} y={-28}>
      <OttoWindow
        nav={<Navigator active="proof" />}
        title="Otto — Proof · Settings"
      >
        <div
          style={{
            padding: '24px 26px',
            display: 'flex',
            flexDirection: 'column',
            gap: 12,
            height: '100%',
            boxSizing: 'border-box',
            position: 'relative',
          }}
        >
          {/* Section label */}
          <Appear delay={4}>
            <div
              style={{
                fontFamily: fonts.ui,
                fontSize: 12,
                fontWeight: 600,
                letterSpacing: 0.8,
                textTransform: 'uppercase',
                color: T.textDim,
                marginBottom: 6,
              }}
            >
              Completion gates
            </div>
          </Appear>

          {/* Toggle rows */}
          {SETTINGS.map((s, i) => (
            <Appear key={s.name} delay={14 + i * 22} y={14}>
              <Card
                pad={16}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 16,
                }}
              >
                <div style={{ flex: 1 }}>
                  <div
                    style={{
                      fontFamily: fonts.mono,
                      fontSize: 13,
                      fontWeight: 600,
                      color: s.on ? statusColors.working : T.text,
                      marginBottom: 4,
                    }}
                  >
                    {s.name}
                  </div>
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 13,
                      color: T.textDim,
                    }}
                  >
                    {s.description}
                  </div>
                </div>
                <Toggle on={s.on} />
              </Card>
            </Appear>
          ))}

          {/* Toast: task blocked */}
          <div
            style={{
              position: 'absolute',
              bottom: 60,
              right: 26,
            }}
          >
            <Toast
              text="task blocked: PR required before close"
              tone="bad"
              delay={78}
            />
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="Require a PR, a goal loop, or auto-tests before a task can close"
      sub="Env knobs gate completion — tasks that skip evidence stay open."
    />
  </>
);

// ── Composition ───────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 80,  node: <TitleScene />,     name: 'Title'    },
  { dur: 180, node: <ProofPackScene />, name: 'ProofPack' },
  { dur: 150, node: <DeriveScene />,    name: 'Derive'   },
  { dur: 130, node: <RequireScene />,   name: 'Require'  },
  {
    dur: 130,
    node: (
      <WalkOutro
        title="Proof Packs"
        tagline="Make 'done' mean proven — with evidence attached"
        pills={[
          { label: 'Evidence-first',       icon: 'check' },
          { label: 'Derived status & risk', icon: 'gauge' },
          { label: 'Redacted artifacts',    icon: 'eye'   },
          { label: 'Gated completion',      icon: 'key'   },
        ]}
      />
    ),
    name: 'Outro',
  },
];

export const proofPacksDuration: number = scenesDuration(SCENES);
export const ProofPacks: React.FC = () => <Scenes scenes={SCENES} />;
