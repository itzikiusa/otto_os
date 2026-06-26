import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, alpha, status } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  TitleCard,
  Caption,
  Field,
  Chip,
  StatusDot,
  Toast,
  Terminal,
  BarChart,
  track,
  Icon,
} from '../components/kit';

// ── Scene 1 — brand title card ────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Goal Loops"
    title="Goal Loops"
    subtitle="Set a goal. Agents iterate until criteria pass — or the budget runs out."
  />
);

// ── Scene 2 — define goal + criteria + budget ─────────────────────────────────

const DefineScene: React.FC = () => (
  <>
    <Stage scale={0.84}>
      <OttoWindow
        nav={<Navigator active="loops" />}
        title="Otto — Goal Loops"
      >
        <div
          style={{
            height: '100%',
            padding: '28px 36px',
            display: 'flex',
            flexDirection: 'column',
            gap: 22,
            boxSizing: 'border-box',
          }}
        >
          {/* Goal field */}
          <Appear delay={6}>
            <Field
              label="Goal"
              value="Make the flaky auth E2E suite pass 20× in a row"
              focused
              icon="zap"
            />
          </Appear>

          {/* Machine-checked criteria */}
          <Appear delay={22}>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 9 }}>
              <span
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 12.5,
                  fontWeight: 600,
                  letterSpacing: 0.5,
                  textTransform: 'uppercase',
                  color: T.textDim,
                }}
              >
                Machine-checked acceptance criteria
              </span>
              <Terminal
                lines={[
                  { text: '$ npm run e2e:auth -- --repeat 20', tone: 'cmd' },
                  { text: '  exit 0  →  goal achieved', tone: 'dim' },
                  { text: '  exit non-0  →  iterate again', tone: 'dim' },
                ]}
                delay={28}
                step={11}
                fontSize={13}
              />
            </div>
          </Appear>

          {/* Budget */}
          <Appear delay={60}>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
              <span
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 12.5,
                  fontWeight: 600,
                  letterSpacing: 0.5,
                  textTransform: 'uppercase',
                  color: T.textDim,
                }}
              >
                Budget
              </span>
              <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap' }}>
                <Chip tone="accent">
                  <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}>
                    <Icon name="refresh" size={11} color={T.accent} />
                    max 8 iterations
                  </span>
                </Chip>
                <Chip tone="accent">
                  <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}>
                    <Icon name="clock" size={11} color={T.accent} />
                    45 min active time
                  </span>
                </Chip>
                <Chip color={brand.violet}>
                  <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}>
                    <Icon name="branch" size={11} color={brand.violet} />
                    isolated branch
                  </span>
                </Chip>
              </div>
            </div>
          </Appear>

          {/* Launch button */}
          <Appear delay={84}>
            <div
              style={{
                alignSelf: 'flex-start',
                display: 'inline-flex',
                alignItems: 'center',
                gap: 8,
                padding: '10px 22px',
                borderRadius: 8,
                background: brand.purple,
                color: '#fff',
                fontFamily: fonts.ui,
                fontSize: 14,
                fontWeight: 700,
                boxShadow: `0 8px 28px ${alpha(brand.purple, 0.45)}`,
              }}
            >
              <Icon name="play" size={13} color="#fff" />
              Launch Goal Loop
            </div>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Set a goal, machine-checked criteria, and a budget"
      sub="Otto validates the criteria command before the first iteration runs"
    />
  </>
);

// ── Scene 3 — live loop: phase pipeline + executor sessions ───────────────────

const PHASES = ['Plan', 'Execute', 'Evaluate', 'Digest'] as const;

const AGENTS = [
  { name: 'planner',    task: 'Analyzing flaky test patterns in auth.spec.ts',           color: '#d97757', s: 'working' as const },
  { name: 'executor-1', task: 'Patching waitForNetworkIdle in AuthMiddleware',            color: '#10a37f', s: 'working' as const },
  { name: 'executor-2', task: 'Queued — waiting for planner output',                      color: '#0a84ff', s: 'idle'    as const },
  { name: 'evaluator',  task: 'Will run acceptance check on iteration completion',        color: '#bf7aff', s: 'idle'    as const },
];

const LoopScene: React.FC = () => {
  const frame = useCurrentFrame();
  // Walk through the four phases once over the scene (one per ~38 f), then hold
  const activePhase = Math.min(3, Math.floor(frame / 38));

  return (
    <>
      <Stage scale={0.84}>
        <OttoWindow
          nav={<Navigator active="loops" workingCount={2} />}
          title="Otto — Goal Loops"
        >
          <div
            style={{
              height: '100%',
              padding: '24px 32px',
              display: 'flex',
              flexDirection: 'column',
              gap: 18,
              boxSizing: 'border-box',
            }}
          >
            {/* Iteration header */}
            <Appear delay={4}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
                <span
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 18,
                    fontWeight: 700,
                    color: T.text,
                  }}
                >
                  Iteration 3 / 8
                </span>
                <Chip color={brand.violet}>
                  <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}>
                    <Icon name="branch" size={11} color={brand.violet} />
                    goal-loop/a1b2c3
                  </span>
                </Chip>
                <StatusDot kind="working" size={9} />
              </div>
            </Appear>

            {/* Phase pipeline */}
            <Appear delay={12}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
                {PHASES.map((phase, i) => {
                  const active = i === activePhase;
                  const done = i < activePhase;
                  return (
                    <React.Fragment key={phase}>
                      {i > 0 && (
                        <Icon
                          name="chevronRight"
                          size={14}
                          color={alpha(T.textDim, 0.4)}
                        />
                      )}
                      <div
                        style={{
                          padding: '8px 18px',
                          borderRadius: 8,
                          background: active
                            ? alpha(brand.cyan, 0.14)
                            : done
                            ? alpha(status.working, 0.1)
                            : T.surface2,
                          border: `1px solid ${
                            active
                              ? alpha(brand.cyan, 0.55)
                              : done
                              ? alpha(status.working, 0.35)
                              : T.border
                          }`,
                          fontFamily: fonts.ui,
                          fontSize: 13.5,
                          fontWeight: active ? 700 : 500,
                          color: active
                            ? brand.cyan
                            : done
                            ? status.working
                            : T.textDim,
                          display: 'flex',
                          alignItems: 'center',
                          gap: 7,
                        }}
                      >
                        {active && <StatusDot kind="working" size={8} />}
                        {done && (
                          <Icon name="check" size={12} color={status.working} />
                        )}
                        {phase}
                      </div>
                    </React.Fragment>
                  );
                })}
              </div>
            </Appear>

            {/* Executor agent sessions */}
            <Appear delay={20} style={{ flex: 1 }}>
              <div
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 8,
                }}
              >
                <span
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 12.5,
                    fontWeight: 600,
                    letterSpacing: 0.5,
                    textTransform: 'uppercase',
                    color: T.textDim,
                    marginBottom: 2,
                  }}
                >
                  Agent sessions
                </span>
                {AGENTS.map((agent, idx) => (
                  <Appear key={agent.name} delay={26 + idx * 9}>
                    <div
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        gap: 12,
                        padding: '10px 14px',
                        borderRadius: 8,
                        background: T.surface,
                        border: `1px solid ${T.border}`,
                      }}
                    >
                      <StatusDot kind={agent.s} size={9} />
                      <Chip color={agent.color}>{agent.name}</Chip>
                      <span
                        style={{
                          flex: 1,
                          fontFamily: fonts.mono,
                          fontSize: 12.5,
                          color: T.textDim,
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        {agent.task}
                      </span>
                      <Icon
                        name="external"
                        size={13}
                        color={alpha(T.textDim, 0.45)}
                      />
                    </div>
                  </Appear>
                ))}
              </div>
            </Appear>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={2}
        title="A team iterates Plan → Execute → Evaluate → Digest on an isolated branch"
        sub="Each executor is a live session — click to open and watch it work"
      />
    </>
  );
};

// ── Scene 4 — convergence: pass-rate chart + success toast ────────────────────

const ConvergeScene: React.FC = () => {
  const frame = useCurrentFrame();
  const grow = track(frame, [14, 72], [0, 1]);

  return (
    <>
      <Stage scale={0.84}>
        <OttoWindow
          nav={<Navigator active="loops" />}
          title="Otto — Goal Loops"
        >
          <div
            style={{
              height: '100%',
              padding: '28px 40px',
              display: 'flex',
              flexDirection: 'column',
              gap: 20,
              boxSizing: 'border-box',
            }}
          >
            <Appear delay={4}>
              <div style={{ display: 'flex', alignItems: 'baseline', gap: 12 }}>
                <span
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 15,
                    fontWeight: 700,
                    color: T.text,
                  }}
                >
                  Auth E2E — consecutive passes per iteration
                </span>
                <span
                  style={{
                    fontFamily: fonts.mono,
                    fontSize: 12.5,
                    color: T.textDim,
                  }}
                >
                  target: 20 passes
                </span>
              </div>
            </Appear>

            <Appear delay={12}>
              <BarChart
                data={[3, 8, 14, 18, 20]}
                labels={['iter 1', 'iter 2', 'iter 3', 'iter 4', 'iter 5 ✓']}
                color={status.working}
                height={210}
                grow={grow}
              />
            </Appear>

            {/* Spacer then toast */}
            <div style={{ marginTop: 'auto' }}>
              <Toast
                text="✓ criteria met at iteration 5 — branch goal-loop/a1b2c3 ready to merge"
                tone="ok"
                delay={84}
              />
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={3}
        title="Stops when the criteria pass — or the budget runs out"
        sub="The isolated branch is yours to merge, extend, or discard"
      />
    </>
  );
};

// ── Scene list ────────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 80,  node: <TitleScene />,   name: 'Title'    },
  { dur: 160, node: <DefineScene />,  name: 'Define'   },
  { dur: 160, node: <LoopScene />,    name: 'Loop'     },
  { dur: 120, node: <ConvergeScene />, name: 'Converge' },
  {
    dur: 130,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Goal Loops"
        tagline="Point agents at a goal — they iterate until it's provably done"
        pills={[
          { label: 'Plan→Execute→Evaluate→Digest', icon: 'refresh' },
          { label: 'Machine-checked',              icon: 'check'   },
          { label: 'Isolated branch',              icon: 'branch'  },
          { label: 'Budgeted',                     icon: 'clock'   },
        ]}
      />
    ),
  },
];

export const goalLoopsDuration = scenesDuration(SCENES);
export const GoalLoops: React.FC = () => <Scenes scenes={SCENES} />;
