import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, status as STATUS, providers, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  track,
  TitleCard,
  Caption,
  Chip,
  Button,
  Toast,
  StatusDot,
  Icon,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  WORKFLOWS — visual automation graph: typed nodes wired on a canvas,
//  a topological run loop, human approval gates, webhook/event/manual triggers.
//  5 scenes · 620 frames · 30 fps
// ════════════════════════════════════════════════════════════════════════════

type ExecState = 'pending' | 'running' | 'done' | 'paused';

interface NodeDef {
  id: string;
  icon: string;
  label: string;
  sublabel: string;
  color: string;
}

// ── Graph node definitions (left-to-right) ────────────────────────────────────

const NODES: NodeDef[] = [
  {
    id: 'webhook',
    icon: 'link',
    label: 'Webhook trigger',
    sublabel: 'POST /hooks/release-guard',
    color: brand.cyan,
  },
  {
    id: 'review',
    icon: 'eye',
    label: 'Agent: run review',
    sublabel: 'claude · code-review',
    color: providers.claude,
  },
  {
    id: 'ci',
    icon: 'check',
    label: 'HTTP: check CI',
    sublabel: 'GET github/status/main',
    color: STATUS.working,
  },
  {
    id: 'approval',
    icon: 'user',
    label: 'Human approval',
    sublabel: '@ops-team · sign-off',
    color: STATUS.needsYou,
  },
  {
    id: 'notify',
    icon: 'slack',
    label: 'Notify Slack',
    sublabel: '#releases — deploy ready',
    color: '#36c5f0',
  },
];

// ── Node box ──────────────────────────────────────────────────────────────────

const NodeBox: React.FC<{ node: NodeDef; execState?: ExecState }> = ({
  node,
  execState = 'pending',
}) => {
  const isPaused = execState === 'paused';
  const isDone   = execState === 'done';
  const isRunning = execState === 'running';

  const borderColor = isPaused
    ? STATUS.needsYou
    : isDone
    ? alpha(STATUS.working, 0.55)
    : isRunning
    ? alpha(node.color, 0.65)
    : T.border;

  const boxShadow = isPaused
    ? `0 0 22px ${alpha(STATUS.needsYou, 0.38)}, 0 4px 16px rgba(0,0,0,0.3)`
    : isRunning
    ? `0 0 16px ${alpha(node.color, 0.28)}, 0 4px 16px rgba(0,0,0,0.3)`
    : '0 4px 16px rgba(0,0,0,0.25)';

  return (
    <div
      style={{
        width: 158,
        background: T.surface,
        border: `1.5px solid ${borderColor}`,
        borderRadius: 10,
        padding: '11px 12px',
        boxShadow,
        flexShrink: 0,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 7, marginBottom: 6 }}>
        <div
          style={{
            width: 26,
            height: 26,
            borderRadius: 7,
            background: alpha(node.color, 0.15),
            display: 'grid',
            placeItems: 'center',
            flexShrink: 0,
          }}
        >
          <Icon name={node.icon} size={13} color={node.color} />
        </div>
        <span
          style={{
            flex: 1,
            fontFamily: fonts.ui,
            fontSize: 11.5,
            fontWeight: 600,
            color: T.text,
            lineHeight: 1.3,
          }}
        >
          {node.label}
        </span>
        {isDone && (
          <span
            style={{
              width: 16,
              height: 16,
              borderRadius: '50%',
              background: STATUS.working,
              display: 'grid',
              placeItems: 'center',
              flexShrink: 0,
            }}
          >
            <Icon name="check" size={9} color="#fff" />
          </span>
        )}
        {isRunning && <StatusDot kind="working" size={8} />}
        {isPaused && <StatusDot kind="needsYou" size={8} pulse={false} />}
      </div>
      <div
        style={{
          fontFamily: fonts.mono,
          fontSize: 10,
          color: T.textDim,
          paddingLeft: 33,
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {node.sublabel}
      </div>
    </div>
  );
};

// ── Edge connector ────────────────────────────────────────────────────────────

const Connector: React.FC<{ done?: boolean }> = ({ done = false }) => (
  <div style={{ display: 'flex', alignItems: 'center', width: 36, flexShrink: 0 }}>
    <div style={{ flex: 1, height: 1.5, background: done ? STATUS.working : T.border }} />
    <Icon name="chevronRight" size={11} color={done ? STATUS.working : T.textDim} />
  </div>
);

// ── Shared window chrome ──────────────────────────────────────────────────────

const WorkflowCanvas: React.FC<{
  children: React.ReactNode;
  running?: boolean;
}> = ({ children, running = false }) => (
  <OttoWindow
    nav={<Navigator active="workflows" />}
    title="Otto — Release Guard"
  >
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      {/* toolbar */}
      <div
        style={{
          height: 42,
          borderBottom: `1px solid ${T.border}`,
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: '0 16px',
          background: T.surface,
          flexShrink: 0,
        }}
      >
        <Icon name="split" size={14} color={brand.cyan} />
        <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 13, fontWeight: 600, color: T.text }}>
          Release Guard
        </span>
        {running ? (
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <StatusDot kind="working" size={8} />
            <span style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>Running…</span>
          </div>
        ) : (
          <Chip tone="ok">Published</Chip>
        )}
        <Button variant={running ? 'default' : 'primary'} size="s" icon="play">
          {running ? 'Running' : 'Run'}
        </Button>
      </div>
      {/* dot-grid canvas */}
      <div
        style={{
          flex: 1,
          position: 'relative',
          overflow: 'hidden',
          backgroundColor: T.bg,
          backgroundImage: `radial-gradient(circle, ${alpha('#ffffff', 0.055)} 1px, transparent 1px)`,
          backgroundSize: '28px 28px',
        }}
      >
        {children}
      </div>
    </div>
  </OttoWindow>
);

// ── Scene 1 — Title ───────────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Workflows"
    title="Workflows"
    subtitle="Visual graph — chain agents, HTTP, approvals & swarm tasks into a runnable pipeline."
  />
);

// ── Scene 2 — Build the graph ─────────────────────────────────────────────────

const GraphScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <WorkflowCanvas>
        <div
          style={{
            position: 'absolute',
            inset: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          <div style={{ display: 'flex', alignItems: 'center' }}>
            {NODES.map((node, i) => (
              <React.Fragment key={node.id}>
                {i > 0 && (
                  <Appear delay={20 + (i - 1) * 14 + 9} y={0}>
                    <Connector />
                  </Appear>
                )}
                <Appear delay={20 + i * 14} y={20}>
                  <NodeBox node={node} />
                </Appear>
              </React.Fragment>
            ))}
          </div>
        </div>
      </WorkflowCanvas>
    </Stage>
    <Caption
      step={1}
      title="Chain agents, HTTP, DB, brokers, approvals & swarm tasks into a graph"
      sub="Design once — Otto runs nodes in topological order, concurrently where possible."
    />
  </>
);

// ── Scene 3 — Run + human approval ───────────────────────────────────────────

const RunScene: React.FC = () => {
  const frame = useCurrentFrame();

  // CI completes and approval pauses at the same topological moment (frame 22)
  const execStates: ExecState[] = [
    'done',
    'done',
    frame < 22 ? 'running' : 'done',
    frame < 22 ? 'pending' : 'paused',
    'pending',
  ];

  const approvalPanelProgress = track(frame, [38, 58], [0, 1]);

  return (
    <>
      <Stage scale={0.88}>
        <WorkflowCanvas running>
          <div
            style={{
              position: 'absolute',
              inset: 0,
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              gap: 22,
            }}
          >
            {/* graph row */}
            <div style={{ display: 'flex', alignItems: 'center' }}>
              {NODES.map((node, i) => (
                <React.Fragment key={node.id}>
                  {i > 0 && <Connector done={execStates[i - 1] === 'done'} />}
                  <NodeBox node={node} execState={execStates[i]} />
                </React.Fragment>
              ))}
            </div>

            {/* human approval panel */}
            <div
              style={{
                opacity: approvalPanelProgress,
                transform: `translateY(${(1 - approvalPanelProgress) * 14}px)`,
                display: 'flex',
                alignItems: 'center',
                gap: 12,
                background: T.surface,
                border: `1.5px solid ${alpha(STATUS.needsYou, 0.55)}`,
                borderRadius: 10,
                padding: '12px 18px',
                boxShadow: `0 0 28px ${alpha(STATUS.needsYou, 0.22)}, 0 6px 20px rgba(0,0,0,0.35)`,
              }}
            >
              <StatusDot kind="needsYou" size={9} />
              <span
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 13,
                  fontWeight: 600,
                  color: T.text,
                  marginRight: 6,
                }}
              >
                Human approval requested ·{' '}
                <span style={{ color: STATUS.needsYou }}>@ops-team</span>
              </span>
              <Button variant="primary" size="s" icon="check">Approve</Button>
              <Button variant="danger"  size="s" icon="x">Reject</Button>
            </div>
          </div>

          {/* resumed toast — appears at frame 100, stays visible */}
          <Toast
            text="Approved — workflow resumed, running Notify Slack"
            tone="ok"
            delay={100}
            style={{ position: 'absolute', top: 14, right: 14 }}
          />
        </WorkflowCanvas>
      </Stage>
      <Caption
        step={2}
        title="A topological run loop — pause for a human, then resume"
        sub="Human approval nodes block execution until an operator approves or rejects."
      />
    </>
  );
};

// ── Scene 4 — Triggers ────────────────────────────────────────────────────────

interface TriggerDef {
  label: string;
  icon: string;
  desc: string;
  active: boolean;
}

const TRIGGERS: TriggerDef[] = [
  { label: 'Manual',    icon: 'play',  desc: 'Run on demand from UI or API',      active: true  },
  { label: 'Webhook',   icon: 'link',  desc: 'POST /hooks/{workflow-id}',          active: true  },
  { label: 'Event',     icon: 'zap',   desc: 'System or Otto event subscription', active: true  },
  { label: 'Scheduled', icon: 'clock', desc: 'Cron schedule — coming soon',       active: false },
];

const TriggersScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <WorkflowCanvas>
        <div
          style={{
            position: 'absolute',
            inset: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            padding: '0 80px',
          }}
        >
          <div style={{ width: '100%', maxWidth: 880 }}>
            <Appear delay={8} y={10}>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 12,
                  fontWeight: 600,
                  color: T.textDim,
                  letterSpacing: 1.4,
                  textTransform: 'uppercase',
                  marginBottom: 18,
                }}
              >
                Trigger type
              </div>
            </Appear>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr 1fr', gap: 16 }}>
              {TRIGGERS.map((trig, i) => (
                <Appear key={trig.label} delay={16 + i * 14} y={22}>
                  <div
                    style={{
                      background: trig.active ? alpha(brand.cyan, 0.07) : alpha(T.surface, 0.6),
                      border: `1.5px solid ${trig.active ? alpha(brand.cyan, 0.35) : T.border}`,
                      borderRadius: 12,
                      padding: '18px 16px',
                      opacity: trig.active ? 1 : 0.44,
                    }}
                  >
                    <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 10 }}>
                      <div
                        style={{
                          width: 32,
                          height: 32,
                          borderRadius: 8,
                          background: trig.active ? alpha(brand.cyan, 0.15) : alpha(T.textDim, 0.1),
                          display: 'grid',
                          placeItems: 'center',
                        }}
                      >
                        <Icon name={trig.icon} size={15} color={trig.active ? brand.cyan : T.textDim} />
                      </div>
                      <div>
                        <div
                          style={{
                            fontFamily: fonts.ui,
                            fontSize: 14,
                            fontWeight: 700,
                            color: trig.active ? T.text : T.textDim,
                            marginBottom: trig.active ? 0 : 4,
                          }}
                        >
                          {trig.label}
                        </div>
                        {!trig.active && <Chip tone="warn">coming soon</Chip>}
                      </div>
                    </div>
                    <div style={{ fontFamily: fonts.mono, fontSize: 11, color: T.textDim, lineHeight: 1.5 }}>
                      {trig.desc}
                    </div>
                  </div>
                </Appear>
              ))}
            </div>
          </div>
        </div>
      </WorkflowCanvas>
    </Stage>
    <Caption
      step={3}
      title="Fire on manual, webhook, or event triggers"
      sub="Scheduled triggers are being wired — manual, webhook, and event fire today."
    />
  </>
);

// ── Scenes list ───────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 75,  node: <TitleScene />,    name: 'Title'    },
  { dur: 175, node: <GraphScene />,    name: 'Build'    },
  { dur: 145, node: <RunScene />,      name: 'Run'      },
  { dur: 105, node: <TriggersScene />, name: 'Triggers' },
  {
    dur: 120,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Workflows"
        tagline="Automate the repeatable — with a human in the loop when it matters"
        pills={[
          { label: 'Visual graph',    icon: 'split' },
          { label: 'Human approval',  icon: 'check' },
          { label: 'Webhook / event', icon: 'link'  },
          { label: 'Multi-step',      icon: 'box'   },
        ]}
      />
    ),
  },
];

export const workflowsDuration = scenesDuration(SCENES);
export const Workflows: React.FC = () => <Scenes scenes={SCENES} />;
