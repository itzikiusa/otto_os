import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, providers, alpha, status } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Caption,
  TitleCard,
  Field,
  Button,
  Card,
  Chip,
  BarChart,
  StatusDot,
  Icon,
  track,
  Terminal,
  TermLine,
} from '../components/kit';

// ── Scene 1 — title card (~75f) ───────────────────────────────────────────────
const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Skills Evaluator"
    title="Skills Evaluator"
    subtitle="Benchmark any skill — across iterations and providers"
  />
);

// ── Scene 2 — configure a run (~145f) ─────────────────────────────────────────
const PROVIDER_OPTIONS: { label: string; color: string }[] = [
  { label: 'claude', color: providers.claude },
  { label: 'codex',  color: providers.codex  },
];

const ConfigScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={<Navigator active="skills-eval" />}
        title="Otto — Skills Evaluator"
      >
        <div
          style={{
            display: 'flex',
            justifyContent: 'center',
            alignItems: 'flex-start',
            padding: '44px 40px',
            height: '100%',
            boxSizing: 'border-box',
          }}
        >
          <Appear delay={8} y={24}>
            <Card t={T} pad={28} style={{ width: 500 }}>
              {/* Card header */}
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 12,
                  marginBottom: 26,
                  paddingBottom: 20,
                  borderBottom: `1px solid ${T.border}`,
                }}
              >
                <div
                  style={{
                    width: 34,
                    height: 34,
                    borderRadius: 9,
                    background: alpha(brand.purple, 0.18),
                    border: `1px solid ${alpha(brand.purple, 0.38)}`,
                    display: 'grid',
                    placeItems: 'center',
                  }}
                >
                  <Icon name="gauge" size={16} color={brand.purple} />
                </div>
                <span
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 16,
                    fontWeight: 700,
                    color: T.text,
                  }}
                >
                  New Benchmark Run
                </span>
              </div>

              {/* Skill */}
              <Appear delay={18} y={10}>
                <div style={{ marginBottom: 18 }}>
                  <Field label="Skill" value="review/security" icon="zap" />
                </div>
              </Appear>

              {/* Providers */}
              <Appear delay={30} y={10}>
                <div style={{ marginBottom: 18 }}>
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 12.5,
                      fontWeight: 500,
                      color: T.textDim,
                      marginBottom: 10,
                    }}
                  >
                    Providers
                  </div>
                  <div style={{ display: 'flex', gap: 8 }}>
                    {PROVIDER_OPTIONS.map((p) => (
                      <div
                        key={p.label}
                        style={{
                          display: 'flex',
                          alignItems: 'center',
                          gap: 8,
                          padding: '7px 14px',
                          borderRadius: 7,
                          background: alpha(p.color, 0.16),
                          border: `1px solid ${alpha(p.color, 0.5)}`,
                          fontFamily: fonts.ui,
                          fontSize: 13.5,
                          fontWeight: 600,
                          color: p.color,
                        }}
                      >
                        <Icon name="check" size={12} color={p.color} />
                        {p.label}
                      </div>
                    ))}
                  </div>
                </div>
              </Appear>

              {/* Iterations */}
              <Appear delay={42} y={10}>
                <div style={{ marginBottom: 26 }}>
                  <Field label="Iterations" value="5" />
                </div>
              </Appear>

              {/* Run button */}
              <Appear delay={58} y={10}>
                <div style={{ display: 'flex' }}>
                  <Button
                    variant="primary"
                    icon="play"
                    style={{ flex: 1, justifyContent: 'center' }}
                  >
                    Run benchmark
                  </Button>
                </div>
              </Appear>
            </Card>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Benchmark a skill — across iterations and providers"
      sub="Pick a skill, choose providers, set iteration count, then run."
      delay={62}
    />
  </>
);

// ── Scene 3 — the eval loop (~135f) ───────────────────────────────────────────
interface PipelineStep {
  label: string;
  icon: string;
  done: boolean;
  active: boolean;
}

const PIPELINE: PipelineStep[] = [
  { label: 'IMPLEMENT', icon: 'edit',    done: true,  active: false },
  { label: 'VALIDATE',  icon: 'check',   done: false, active: true  },
  { label: 'SCORE',     icon: 'gauge',   done: false, active: false },
  { label: 'IMPROVE',   icon: 'refresh', done: false, active: false },
];

const ITER_SCORES = [62, 71, 78];
const ITER_LABELS = ['iter 1', 'iter 2', 'iter 3'];

const evalLines: TermLine[] = [
  { text: '▶  applying skill: review/security',       tone: 'cmd'    },
  { text: '   reading 14 files in pr/api-gateway-v2', tone: 'dim'    },
  { text: '   running 3 security review heuristics…', tone: 'text'   },
  { text: '   · JWT expiry not enforced on /refresh',  tone: 'warn'   },
  { text: '   · SSRF guard absent on webhook handler', tone: 'warn'   },
  { text: '   ✓ validation pass — 0 regressions',     tone: 'ok'     },
  { text: '   score: 78 / 100  (+7 vs iteration 2)',  tone: 'accent' },
];

const EvalLoopScene: React.FC = () => {
  const frame = useCurrentFrame();
  const barGrow = track(frame, [32, 105], [0, 1]);

  return (
    <>
      <Stage scale={0.88}>
        <OttoWindow
          nav={<Navigator active="skills-eval" />}
          title="Otto — Skills Evaluator"
          tabs={[
            { label: 'Run #9 · in progress', icon: 'gauge', active: true, dot: 'working' },
          ]}
        >
          <div
            style={{
              padding: '18px 22px',
              height: '100%',
              boxSizing: 'border-box',
              display: 'flex',
              flexDirection: 'column',
              gap: 16,
              overflow: 'hidden',
            }}
          >
            {/* Iteration status bar */}
            <Appear delay={5}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 14,
                  padding: '9px 16px',
                  borderRadius: 8,
                  background: T.surface,
                  border: `1px solid ${T.border}`,
                }}
              >
                <StatusDot kind="working" size={10} />
                <span
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 13.5,
                    fontWeight: 600,
                    color: T.text,
                  }}
                >
                  Iteration
                </span>
                <span
                  style={{
                    fontFamily: fonts.mono,
                    fontSize: 15,
                    fontWeight: 700,
                    color: T.text,
                  }}
                >
                  3 / 5
                </span>
                <Chip color={providers.claude} style={{ marginLeft: 2 }}>claude</Chip>
                <div style={{ flex: 1 }} />
                <span
                  style={{
                    fontFamily: fonts.mono,
                    fontSize: 12.5,
                    color: T.textDim,
                  }}
                >
                  review/security
                </span>
              </div>
            </Appear>

            {/* Pipeline steps */}
            <Appear delay={12}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                {PIPELINE.map((step, i) => (
                  <React.Fragment key={step.label}>
                    <div
                      style={{
                        flex: 1,
                        display: 'flex',
                        flexDirection: 'column',
                        alignItems: 'center',
                        gap: 8,
                        padding: '12px 10px',
                        borderRadius: 8,
                        background: step.active
                          ? alpha(brand.purple, 0.1)
                          : step.done
                          ? alpha(status.working, 0.07)
                          : T.surface,
                        border: `1px solid ${
                          step.active
                            ? alpha(brand.purple, 0.4)
                            : step.done
                            ? alpha(status.working, 0.3)
                            : T.border
                        }`,
                      }}
                    >
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                        <StatusDot
                          kind={step.active ? 'working' : 'idle'}
                          size={step.active ? 9 : 7}
                          pulse={step.active}
                        />
                        <span
                          style={{
                            fontFamily: fonts.mono,
                            fontSize: 10.5,
                            fontWeight: 700,
                            letterSpacing: 0.7,
                            textTransform: 'uppercase',
                            color: step.active
                              ? brand.purple
                              : step.done
                              ? status.working
                              : T.textDim,
                          }}
                        >
                          {step.label}
                        </span>
                      </div>
                      <Icon
                        name={step.icon}
                        size={15}
                        color={
                          step.active
                            ? brand.purple
                            : step.done
                            ? status.working
                            : T.textDim
                        }
                      />
                    </div>
                    {i < PIPELINE.length - 1 && (
                      <div
                        style={{
                          width: 18,
                          height: 1,
                          background: T.border,
                          flexShrink: 0,
                        }}
                      />
                    )}
                  </React.Fragment>
                ))}
              </div>
            </Appear>

            {/* Score chart + terminal */}
            <div style={{ display: 'flex', gap: 18, flex: 1, minHeight: 0 }}>
              {/* Score chart */}
              <Appear delay={22}>
                <div
                  style={{
                    background: T.surface,
                    border: `1px solid ${T.border}`,
                    borderRadius: 10,
                    padding: '16px 20px',
                    width: 268,
                    display: 'flex',
                    flexDirection: 'column',
                    gap: 14,
                  }}
                >
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 12.5,
                      fontWeight: 600,
                      color: T.textDim,
                    }}
                  >
                    Score by iteration
                  </div>
                  <BarChart
                    data={ITER_SCORES}
                    labels={ITER_LABELS}
                    color={brand.purple}
                    height={145}
                    grow={barGrow}
                    t={T}
                  />
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Icon name="arrowUp" size={12} color={status.working} />
                    <span
                      style={{
                        fontFamily: fonts.ui,
                        fontSize: 12,
                        fontWeight: 600,
                        color: status.working,
                      }}
                    >
                      +16 pts over 3 iters
                    </span>
                  </div>
                </div>
              </Appear>

              {/* Terminal output */}
              <Appear delay={32} style={{ flex: 1, minWidth: 0 }}>
                <Terminal
                  lines={evalLines}
                  delay={38}
                  step={12}
                  fontSize={13}
                  t={T}
                  style={{ borderRadius: 10 }}
                />
              </Appear>
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={2}
        title="Implement → validate → score → improve, each iteration"
        sub="Otto runs the full loop per iteration, compounding improvements across the run."
        delay={68}
      />
    </>
  );
};

// ── Scene 4 — compare runs (~100f) ─────────────────────────────────────────────
const RunCard: React.FC<{
  runNum: number;
  providerLabel: string;
  providerColor: string;
  score: number;
  delta: string;
  deltaPositive: boolean;
  winner: boolean;
}> = ({ runNum, providerLabel, providerColor, score, delta, deltaPositive, winner }) => (
  <Card
    t={T}
    pad={24}
    style={{
      width: 390,
      border: winner
        ? `1px solid ${alpha(status.working, 0.55)}`
        : `1px solid ${T.border}`,
      boxShadow: winner
        ? `0 0 0 3px ${alpha(status.working, 0.1)}, 0 14px 44px rgba(0,0,0,0.42)`
        : '0 12px 38px rgba(0,0,0,0.32)',
      position: 'relative',
    }}
  >
    {/* "Best" badge */}
    {winner && (
      <div
        style={{
          position: 'absolute',
          top: -1,
          right: 18,
          background: status.working,
          color: '#000',
          fontFamily: fonts.ui,
          fontSize: 10.5,
          fontWeight: 800,
          letterSpacing: 0.5,
          padding: '3px 10px',
          borderRadius: '0 0 7px 7px',
        }}
      >
        BEST RUN
      </div>
    )}

    {/* Header: run id + provider */}
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        marginBottom: 22,
      }}
    >
      <span
        style={{
          fontFamily: fonts.mono,
          fontSize: 14,
          fontWeight: 700,
          color: T.textDim,
        }}
      >
        run #{runNum}
      </span>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '4px 10px',
          borderRadius: 6,
          background: alpha(providerColor, 0.16),
          border: `1px solid ${alpha(providerColor, 0.45)}`,
          fontFamily: fonts.ui,
          fontSize: 12.5,
          fontWeight: 600,
          color: providerColor,
        }}
      >
        {providerLabel}
      </div>
    </div>

    {/* Score */}
    <div
      style={{
        fontFamily: fonts.ui,
        fontSize: 72,
        fontWeight: 800,
        letterSpacing: -2,
        lineHeight: 1.0,
        color: winner ? status.working : T.text,
      }}
    >
      {score}
    </div>
    <div
      style={{
        fontFamily: fonts.ui,
        fontSize: 14,
        color: T.textDim,
        marginBottom: 20,
      }}
    >
      / 100
    </div>

    {/* Details */}
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
        marginBottom: 18,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <Icon name="zap" size={13} color={T.textDim} />
        <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.text }}>
          review/security
        </span>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <Icon name="refresh" size={13} color={T.textDim} />
        <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.text }}>
          5 iterations
        </span>
      </div>
    </div>

    {/* Delta */}
    <div
      style={{
        padding: '7px 12px',
        borderRadius: 6,
        background: deltaPositive
          ? alpha(status.working, 0.1)
          : alpha('#ff5f57', 0.1),
        border: `1px solid ${
          deltaPositive
            ? alpha(status.working, 0.35)
            : alpha('#ff5f57', 0.35)
        }`,
        display: 'flex',
        alignItems: 'center',
        gap: 6,
      }}
    >
      <Icon
        name={deltaPositive ? 'arrowUp' : 'arrowDown'}
        size={12}
        color={deltaPositive ? status.working : '#ff5f57'}
      />
      <span
        style={{
          fontFamily: fonts.mono,
          fontSize: 12.5,
          fontWeight: 700,
          color: deltaPositive ? status.working : '#ff5f57',
        }}
      >
        {delta}
      </span>
    </div>
  </Card>
);

const CompareScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={<Navigator active="skills-eval" />}
        title="Otto — Skills Evaluator"
        tabs={[{ label: 'Compare Runs', icon: 'split', active: true }]}
      >
        <div
          style={{
            display: 'flex',
            justifyContent: 'center',
            alignItems: 'flex-start',
            padding: '40px 48px',
            gap: 40,
            height: '100%',
            boxSizing: 'border-box',
          }}
        >
          {/* Run #7 — winner */}
          <Appear delay={8} y={22}>
            <RunCard
              runNum={7}
              providerLabel="claude"
              providerColor={providers.claude}
              score={88}
              delta="+4 vs run #8"
              deltaPositive={true}
              winner={true}
            />
          </Appear>

          {/* Center divider with delta */}
          <Appear delay={30} y={0}>
            <div
              style={{
                alignSelf: 'center',
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'center',
                gap: 10,
                marginTop: 64,
              }}
            >
              <div
                style={{
                  width: 48,
                  height: 48,
                  borderRadius: '50%',
                  background: alpha(brand.purple, 0.18),
                  border: `1px solid ${alpha(brand.purple, 0.4)}`,
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                }}
              >
                <Icon name="split" size={18} color={brand.purple} />
              </div>
              <div
                style={{
                  fontFamily: fonts.mono,
                  fontSize: 30,
                  fontWeight: 800,
                  color: status.working,
                  letterSpacing: -0.5,
                }}
              >
                +4
              </div>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 11.5,
                  color: T.textDim,
                  textAlign: 'center',
                  maxWidth: 70,
                  lineHeight: 1.4,
                }}
              >
                pts better
              </div>
            </div>
          </Appear>

          {/* Run #8 */}
          <Appear delay={16} y={22}>
            <RunCard
              runNum={8}
              providerLabel="codex"
              providerColor={providers.codex}
              score={84}
              delta="-4 vs run #7"
              deltaPositive={false}
              winner={false}
            />
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="Compare runs side-by-side to see what actually improved"
      sub="Score history, delta, and per-iteration breakdown — at a glance."
      delay={56}
    />
  </>
);

// ── Composition ───────────────────────────────────────────────────────────────
const SCENES: SceneDef[] = [
  { dur: 75,  node: <TitleScene />,    name: 'Title'     },
  { dur: 145, node: <ConfigScene />,   name: 'Configure'  },
  { dur: 135, node: <EvalLoopScene />, name: 'EvalLoop'   },
  { dur: 100, node: <CompareScene />,  name: 'Compare'    },
  {
    dur: 125,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Skills Evaluator"
        tagline="Measure a skill, then make it measurably better"
        pills={[
          { label: 'Implement→Validate→Score→Improve', icon: 'refresh' },
          { label: 'Multi-provider',                   icon: 'zap'     },
          { label: 'Run report',                       icon: 'chart'   },
          { label: 'Side-by-side compare',             icon: 'split'   },
        ]}
      />
    ),
  },
];

export const skillsEvalDuration = scenesDuration(SCENES);
export const SkillsEval: React.FC = () => <Scenes scenes={SCENES} />;
