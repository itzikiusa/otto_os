import React from 'react';
import {
  AbsoluteFill,
  Sequence,
  useCurrentFrame,
  useVideoConfig,
  interpolate,
  spring,
} from 'remotion';
import { theme } from '../theme';
import { OttoWindow } from '../components/OttoWindow';
import { Appear, Caption, TitleCard } from '../components/ui';

// ─── Agent Swarm walkthrough — ~36s ──────────────────────────────────────────
// Teams of role-specialized agents: org tree → projects → tasks,
// coordinator, shared Kanban board, run-graph view.
// ─────────────────────────────────────────────────────────────────────────────

const TITLE_DUR  = 75;
const S1_DUR     = 195;  // org tree + create swarm
const S2_DUR     = 210;  // kanban board (projects → tasks)
const S3_DUR     = 165;  // run-graph / transcript
const OUTRO_DUR  = 90;

const S1_START   = TITLE_DUR;
const S2_START   = S1_START + S1_DUR;
const S3_START   = S2_START + S2_DUR;
const OUTRO_START = S3_START + S3_DUR;

// ─── Role colors ──────────────────────────────────────────────────────────────
const ROLE_COLOR: Record<string, string> = {
  coordinator: theme.accent,
  engineer:    theme.accent2,
  reviewer:    '#bf7aff',
  qa:          theme.warn,
  product:     '#63e6be',
};

const RoleBadge: React.FC<{ role: string }> = ({ role }) => {
  const c = ROLE_COLOR[role] ?? theme.textDim;
  return <span style={{ fontFamily: theme.mono, fontSize: 11, fontWeight: 700, color: c, background: `${c}22`, border: `1px solid ${c}44`, borderRadius: 6, padding: '2px 8px', letterSpacing: 0.4 }}>{role}</span>;
};

// ─── Scene 1 – Org tree + new swarm ──────────────────────────────────────────
const AGENTS = [
  { name: 'Otto Coord',   role: 'coordinator', status: 'working', depth: 0 },
  { name: 'Alex',         role: 'engineer',    status: 'working', depth: 1 },
  { name: 'Sam',          role: 'engineer',    status: 'idle',    depth: 1 },
  { name: 'Maya',         role: 'reviewer',    status: 'idle',    depth: 1 },
  { name: 'Taylor',       role: 'qa',          status: 'idle',    depth: 1 },
];

const Scene1OrgTree: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const showWizard = frame >= 80;
  const wizardS = spring({ frame: frame - 80, fps, config: { damping: 180 } });

  return (
    <div style={{ display: 'flex', height: '100%' }}>
      {/* org tree sidebar */}
      <div style={{ width: 280, background: theme.surface, borderRight: `1px solid ${theme.border}`, display: 'flex', flexDirection: 'column', height: '100%', flexShrink: 0 }}>
        <div style={{ padding: '14px 16px 10px', borderBottom: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <span style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 11, fontWeight: 700, letterSpacing: 1.2, textTransform: 'uppercase' }}>Swarms</span>
          <div style={{ width: 22, height: 22, borderRadius: 6, background: `${theme.accent}22`, border: `1px solid ${theme.accent}44`, display: 'grid', placeItems: 'center', color: theme.accent, fontSize: 16, fontWeight: 700 }}>+</div>
        </div>
        <div style={{ padding: '8px 0', flex: 1 }}>
          <div style={{ padding: '6px 16px 4px', color: theme.textDim, fontFamily: theme.mono, fontSize: 12 }}>feat/rbac-multiuser</div>
          {AGENTS.map((agent, i) => {
            const s = spring({ frame: frame - i * 10, fps, config: { damping: 200 } });
            const isCoord = agent.role === 'coordinator';
            return (
              <div key={agent.name} style={{ opacity: s, transform: `translateX(${interpolate(s, [0, 1], [-10, 0])}px)`, display: 'flex', alignItems: 'center', gap: 10, padding: `7px 16px 7px ${16 + agent.depth * 18}px`, background: isCoord ? `${theme.accent}14` : 'transparent', borderLeft: isCoord ? `2px solid ${theme.accent}` : '2px solid transparent' }}>
                <div style={{ width: 8, height: 8, borderRadius: '50%', background: agent.status === 'working' ? theme.accent2 : theme.textDim, flexShrink: 0 }} />
                <span style={{ color: isCoord ? theme.text : theme.textDim, fontFamily: theme.font, fontSize: 13, flex: 1, fontWeight: isCoord ? 700 : 400 }}>{agent.name}</span>
                <RoleBadge role={agent.role} />
              </div>
            );
          })}
        </div>
      </div>

      {/* main panel */}
      <div style={{ flex: 1, position: 'relative', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        {!showWizard && (
          <Appear delay={8}>
            <div style={{ textAlign: 'center', maxWidth: 480 }}>
              <div style={{ fontSize: 56, marginBottom: 20 }}>🐝</div>
              <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 26, fontWeight: 800, marginBottom: 12 }}>Agent Swarm</div>
              <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 16, lineHeight: 1.65 }}>
                Assemble a team of role-specialized agents. A coordinator breaks down the work; engineers, reviewers, and QA agents run in parallel — sharing a Kanban board and run-graph.
              </div>
            </div>
          </Appear>
        )}

        {showWizard && (
          <div style={{ opacity: wizardS, transform: `scale(${interpolate(wizardS, [0, 1], [0.9, 1])})`, width: 580, background: theme.surface, border: `1px solid ${theme.border}`, borderRadius: 18, boxShadow: '0 40px 100px rgba(0,0,0,0.7)', padding: '28px 32px' }}>
            <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 20, fontWeight: 800, marginBottom: 6 }}>New Swarm</div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13, marginBottom: 24 }}>Choose a role template — the coordinator recruits the right agents automatically</div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
              {[
                { template: 'Full-stack feature', desc: 'Coordinator · 2× Engineer · Reviewer · QA', selected: true },
                { template: 'Code review team', desc: 'Coordinator · Architecture · Security · Performance', selected: false },
                { template: 'Product sprint',    desc: 'Product agent · Engineer · QA · Docs', selected: false },
              ].map(({ template, desc, selected }) => (
                <div key={template} style={{ padding: '14px 18px', borderRadius: 12, background: selected ? `${theme.accent}14` : theme.surface2, border: `1px solid ${selected ? theme.accent : theme.border}`, display: 'flex', alignItems: 'center', gap: 14 }}>
                  <div style={{ width: 14, height: 14, borderRadius: '50%', border: `2px solid ${selected ? theme.accent : theme.border}`, display: 'grid', placeItems: 'center', flexShrink: 0 }}>
                    {selected && <div style={{ width: 6, height: 6, borderRadius: '50%', background: theme.accent }} />}
                  </div>
                  <div>
                    <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 15, fontWeight: selected ? 700 : 400 }}>{template}</div>
                    <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 12, marginTop: 2 }}>{desc}</div>
                  </div>
                </div>
              ))}
            </div>
            <div style={{ display: 'flex', gap: 12, marginTop: 24, justifyContent: 'flex-end' }}>
              <div style={{ padding: '9px 22px', border: `1px solid ${theme.border}`, borderRadius: 10, color: theme.textDim, fontFamily: theme.font, fontSize: 14 }}>Cancel</div>
              <div style={{ padding: '9px 26px', background: theme.accent, borderRadius: 10, color: '#fff', fontFamily: theme.font, fontSize: 14, fontWeight: 700, boxShadow: `0 6px 20px ${theme.accent}44` }}>Launch Swarm</div>
            </div>
          </div>
        )}
      </div>

      <Caption step={1} title="Assemble a swarm" sub="Pick a role template — coordinator recruits the team" delay={70} />
    </div>
  );
};

// ─── Scene 2 – Kanban board ────────────────────────────────────────────────────
const KANBAN_COLS = ['Backlog', 'In progress', 'In review', 'Done'] as const;
type KanbanCol = typeof KANBAN_COLS[number];

const TASKS: { title: string; col: KanbanCol; agent: string; role: string }[] = [
  { title: 'Design RBAC migration',           col: 'Done',        agent: 'Otto Coord', role: 'coordinator' },
  { title: 'Implement user_feature_grants',   col: 'Done',        agent: 'Alex',       role: 'engineer' },
  { title: 'Write component tests',           col: 'In review',   agent: 'Sam',        role: 'engineer' },
  { title: 'Architecture review',             col: 'In review',   agent: 'Maya',       role: 'reviewer' },
  { title: 'E2E session isolation test',      col: 'In progress', agent: 'Taylor',     role: 'qa' },
  { title: 'Update OpenAPI spec',             col: 'In progress', agent: 'Alex',       role: 'engineer' },
  { title: 'Impersonation audit log',         col: 'Backlog',     agent: '-',          role: 'engineer' },
];

const Scene2Kanban: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const colTasks = (col: KanbanCol) => TASKS.filter((t) => t.col === col);
  const COL_ORDER = { 'Backlog': 0, 'In progress': 1, 'In review': 2, 'Done': 3 };
  const COL_COLOR: Record<KanbanCol, string> = {
    Backlog:      theme.textDim,
    'In progress': theme.accent,
    'In review':  '#bf7aff',
    Done:         theme.accent2,
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', overflow: 'hidden' }}>
      {/* toolbar */}
      <Appear delay={4}>
        <div style={{ padding: '16px 24px', borderBottom: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', gap: 12, flexShrink: 0 }}>
          <div style={{ width: 8, height: 8, borderRadius: '50%', background: theme.accent2 }} />
          <span style={{ color: theme.text, fontFamily: theme.font, fontSize: 16, fontWeight: 700 }}>feat/rbac-multiuser</span>
          <span style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>2 active agents · 3 tasks remaining</span>
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
            <div style={{ padding: '6px 14px', borderRadius: 8, background: `${theme.accent}14`, border: `1px solid ${theme.accent}44`, color: theme.accent, fontFamily: theme.font, fontSize: 13, fontWeight: 700 }}>Board</div>
            <div style={{ padding: '6px 14px', borderRadius: 8, border: `1px solid ${theme.border}`, color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>Run graph</div>
          </div>
        </div>
      </Appear>

      {/* columns */}
      <div style={{ flex: 1, display: 'grid', gridTemplateColumns: '1fr 1fr 1fr 1fr', gap: 0, overflow: 'hidden' }}>
        {KANBAN_COLS.map((col) => {
          const tasks = colTasks(col);
          const colColor = COL_COLOR[col];
          return (
            <div key={col} style={{ borderRight: `1px solid ${theme.border}`, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
              {/* column header */}
              <div style={{ padding: '12px 16px', borderBottom: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0 }}>
                <div style={{ width: 8, height: 8, borderRadius: '50%', background: colColor }} />
                <span style={{ color: colColor, fontFamily: theme.font, fontSize: 13, fontWeight: 700 }}>{col}</span>
                <span style={{ marginLeft: 'auto', color: theme.textDim, fontFamily: theme.mono, fontSize: 12, background: `${colColor}18`, padding: '2px 8px', borderRadius: 6 }}>{tasks.length}</span>
              </div>
              {/* task cards */}
              <div style={{ flex: 1, overflow: 'hidden', padding: '12px 10px', display: 'flex', flexDirection: 'column', gap: 8 }}>
                {tasks.map((task, i) => {
                  const globalIdx = TASKS.indexOf(task);
                  const s = spring({ frame: frame - (globalIdx * 10 + 12), fps, config: { damping: 200 } });
                  return (
                    <div key={task.title} style={{ opacity: s, transform: `translateY(${interpolate(s, [0, 1], [12, 0])}px)`, background: theme.surface2, borderRadius: 10, border: `1px solid ${theme.border}`, padding: '12px 14px' }}>
                      <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 13, fontWeight: 600, marginBottom: 8, lineHeight: 1.4 }}>{task.title}</div>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                        <div style={{ width: 6, height: 6, borderRadius: '50%', background: ROLE_COLOR[task.role] ?? theme.textDim }} />
                        <span style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 11 }}>{task.agent}</span>
                        <RoleBadge role={task.role} />
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>

      <Caption step={2} title="Shared Kanban board" sub="Tasks flow across agents — progress visible to all" delay={55} />
    </div>
  );
};

// ─── Scene 3 – Run-graph (coordinator + agent nodes) ──────────────────────────
type NodeStatus = 'done' | 'running' | 'waiting';

const RUN_NODES: { id: string; label: string; role: string; status: NodeStatus; x: number; y: number }[] = [
  { id: 'coord', label: 'Coordinator',           role: 'coordinator', status: 'done',    x: 540, y: 120 },
  { id: 'alex',  label: 'Alex: implement RBAC',  role: 'engineer',    status: 'done',    x: 280, y: 260 },
  { id: 'maya',  label: 'Maya: review PR',        role: 'reviewer',    status: 'running', x: 800, y: 260 },
  { id: 'sam',   label: 'Sam: write tests',       role: 'engineer',    status: 'running', x: 280, y: 400 },
  { id: 'taylor', label: 'Taylor: E2E tests',     role: 'qa',          status: 'waiting', x: 800, y: 400 },
];
const RUN_EDGES = [['coord', 'alex'], ['coord', 'maya'], ['alex', 'sam'], ['maya', 'taylor']];

const STATUS_NODE_COLOR: Record<NodeStatus, string> = {
  done:    theme.accent2,
  running: theme.accent,
  waiting: theme.textDim,
};

const Scene3RunGraph: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const dotPulse = Math.abs(Math.sin((frame / 30) * Math.PI * 1.5)) * 0.4 + 0.6;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', overflow: 'hidden' }}>
      <div style={{ padding: '14px 24px', borderBottom: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', gap: 12, flexShrink: 0 }}>
        <div style={{ width: 8, height: 8, borderRadius: '50%', background: theme.accent2 }} />
        <span style={{ color: theme.text, fontFamily: theme.font, fontSize: 16, fontWeight: 700 }}>Run graph</span>
        <span style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>feat/rbac-multiuser · 2 of 5 agents still running</span>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
          <div style={{ padding: '6px 14px', borderRadius: 8, border: `1px solid ${theme.border}`, color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>Board</div>
          <div style={{ padding: '6px 14px', borderRadius: 8, background: `${theme.accent}14`, border: `1px solid ${theme.accent}44`, color: theme.accent, fontFamily: theme.font, fontSize: 13, fontWeight: 700 }}>Run graph</div>
        </div>
      </div>

      <div style={{ flex: 1, position: 'relative', overflow: 'hidden' }}>
        <svg width="100%" height="100%" viewBox="0 0 1080 520" style={{ position: 'absolute', inset: 0 }}>
          {/* edges */}
          {RUN_EDGES.map((edge, i) => {
            const from = RUN_NODES.find((n) => n.id === edge[0])!;
            const to   = RUN_NODES.find((n) => n.id === edge[1])!;
            const s = spring({ frame: frame - i * 10, fps, config: { damping: 200 } });
            return (
              <line
                key={i}
                x1={from.x}
                y1={from.y}
                x2={interpolate(s, [0, 1], [from.x, to.x])}
                y2={interpolate(s, [0, 1], [from.y, to.y])}
                stroke={theme.border}
                strokeWidth={1.5}
                opacity={s}
              />
            );
          })}

          {/* nodes */}
          {RUN_NODES.map((node, i) => {
            const s = spring({ frame: frame - i * 12, fps, config: { damping: 180 } });
            const c = STATUS_NODE_COLOR[node.status];
            const isRunning = node.status === 'running';
            const r = node.id === 'coord' ? 18 : 14;
            return (
              <g key={node.id} opacity={s} transform={`translate(${node.x}, ${node.y})`}>
                {/* pulse ring for running */}
                {isRunning && (
                  <circle r={r + 8} fill="none" stroke={c} strokeWidth={1.5} opacity={dotPulse * 0.5} />
                )}
                <circle r={r} fill={`${c}22`} stroke={c} strokeWidth={2} />
                {node.status === 'done' && (
                  <text textAnchor="middle" y={6} fill={c} fontSize={14} fontWeight={700}>✓</text>
                )}
                {isRunning && (
                  <circle r={5} fill={c} opacity={dotPulse} />
                )}
                {node.status === 'waiting' && (
                  <text textAnchor="middle" y={5} fill={c} fontSize={13}>⋯</text>
                )}
                <text textAnchor="middle" y={r + 18} fill={ROLE_COLOR[node.role] ?? theme.textDim} fontFamily={theme.font} fontSize={12} fontWeight={700}>
                  {node.label.length > 22 ? node.label.slice(0, 20) + '…' : node.label}
                </text>
                <text textAnchor="middle" y={r + 32} fill={theme.textDim} fontFamily={theme.mono} fontSize={10}>
                  {node.role}
                </text>
              </g>
            );
          })}
        </svg>

        {/* legend */}
        <Appear delay={30} style={{ position: 'absolute', bottom: 20, right: 24 }}>
          <div style={{ background: theme.surface2, borderRadius: 12, padding: '12px 18px', border: `1px solid ${theme.border}`, display: 'flex', gap: 16 }}>
            {(['done', 'running', 'waiting'] as NodeStatus[]).map((st) => (
              <div key={st} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <div style={{ width: 8, height: 8, borderRadius: '50%', background: STATUS_NODE_COLOR[st] }} />
                <span style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 12 }}>{st}</span>
              </div>
            ))}
          </div>
        </Appear>
      </div>

      <Caption step={3} title="Run graph" sub="Trace how the coordinator delegated — done, running, waiting" delay={45} />
    </div>
  );
};

// ─── Outro ─────────────────────────────────────────────────────────────────────
const Outro: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const t1 = spring({ frame,              fps, config: { damping: 160 } });
  const t2 = spring({ frame: frame - 18, fps, config: { damping: 160 } });
  const t3 = spring({ frame: frame - 32, fps, config: { damping: 160 } });

  return (
    <div style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 12 }}>
      <div style={{ opacity: t1, transform: `scale(${interpolate(t1, [0, 1], [0.5, 1])})`, fontSize: 80 }}>🐝</div>
      <div style={{ opacity: t2, transform: `translateY(${interpolate(t2, [0, 1], [24, 0])}px)`, color: theme.text, fontFamily: theme.font, fontSize: 64, fontWeight: 800, textAlign: 'center' }}>
        Your whole team, automated.
      </div>
      <div style={{ opacity: t3, transform: `translateY(${interpolate(t3, [0, 1], [16, 0])}px)`, color: theme.textDim, fontFamily: theme.font, fontSize: 24, textAlign: 'center' }}>
        Coordinator · Engineers · Reviewer · QA · Shared board
      </div>
    </div>
  );
};

// ─── Root composition ─────────────────────────────────────────────────────────
export const Swarm: React.FC = () => {
  return (
    <AbsoluteFill style={{ background: theme.bgGradient, fontFamily: theme.font }}>

      <Sequence durationInFrames={TITLE_DUR}>
        <TitleCard kicker="OTTO ADE" title="Agent Swarm" subtitle="Teams of specialized agents, in sync" />
      </Sequence>

      <Sequence from={S1_START} durationInFrames={S1_DUR + S2_DUR + S3_DUR}>
        <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <OttoWindow title="Otto — Agent Swarm">
            <Sequence durationInFrames={S1_DUR}>
              <Scene1OrgTree />
            </Sequence>
            <Sequence from={S1_DUR} durationInFrames={S2_DUR}>
              <Scene2Kanban />
            </Sequence>
            <Sequence from={S1_DUR + S2_DUR} durationInFrames={S3_DUR}>
              <Scene3RunGraph />
            </Sequence>
          </OttoWindow>
        </AbsoluteFill>
      </Sequence>

      <Sequence from={OUTRO_START} durationInFrames={OUTRO_DUR}>
        <Outro />
      </Sequence>

    </AbsoluteFill>
  );
};
