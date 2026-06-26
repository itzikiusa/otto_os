import React from 'react';
import { AbsoluteFill, useCurrentFrame } from 'remotion';
import { T, brand, fonts, providers, status as STATUS, alpha, radius } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Avatar,
  Caption,
  Chip,
  Button,
  StatusDot,
  TitleCard,
  track,
  Icon,
} from '../components/kit';

// ─────────────────────────────────────────────────────────────────────────────
//  Shared: provider-to-color assignments for each role-agent
// ─────────────────────────────────────────────────────────────────────────────

const COLORS = {
  ada:      providers.claude,   // CEO
  linus:    providers.codex,    // CTO
  grace:    brand.cyan,         // Team Lead
  dijkstra: providers.claude,   // Dev
  hopper:   providers.codex,    // Dev
  turing:   brand.violet,       // Reviewer
  lovelace: providers.shell,    // QA
} as const;

// Consistent tab bars for the three Swarm sub-views
const SWARM_TABS_ORG = [
  { label: 'Overview',  icon: 'gauge' },
  { label: 'Org Tree',  icon: 'user',  active: true },
  { label: 'Board',     icon: 'grid' },
  { label: 'Run Graph', icon: 'split' },
  { label: 'Scheduled', icon: 'clock' },
] as const;

const SWARM_TABS_BOARD = [
  { label: 'Overview',  icon: 'gauge' },
  { label: 'Org Tree',  icon: 'user' },
  { label: 'Board',     icon: 'grid',  active: true, dot: 'working' as const },
  { label: 'Run Graph', icon: 'split' },
  { label: 'Scheduled', icon: 'clock' },
] as const;

const SWARM_TABS_RUN = [
  { label: 'Overview',  icon: 'gauge' },
  { label: 'Org Tree',  icon: 'user' },
  { label: 'Board',     icon: 'grid' },
  { label: 'Run Graph', icon: 'split', active: true },
  { label: 'Scheduled', icon: 'clock' },
] as const;

// ─────────────────────────────────────────────────────────────────────────────
//  Scene 1 — Title card
// ─────────────────────────────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Agent Swarm"
    title="A Company of Agents"
    subtitle="Role-specialized workers, drafted by a Recruiter, coordinated by a Scheduler — running your projects autonomously."
  />
);

// ─────────────────────────────────────────────────────────────────────────────
//  Scene 2 — Org tree
//  Content area: 1312 × 802 px (after Navigator 248 + Titlebar 44 + TabBar 38)
// ─────────────────────────────────────────────────────────────────────────────

// Approximate card dimensions for connector-line math
const CARD_W = 128;
const CARD_H = 124;

// Vertical connector line with dashoffset draw-in
const OrgLine: React.FC<{
  x1: number; y1: number; x2: number; y2: number; delay?: number;
}> = ({ x1, y1, x2, y2, delay = 0 }) => {
  const frame = useCurrentFrame();
  const dx = x2 - x1;
  const dy = y2 - y1;
  const len = Math.ceil(Math.sqrt(dx * dx + dy * dy));
  const t = track(frame, [delay, delay + 16], [0, 1]);
  return (
    <line
      x1={x1} y1={y1} x2={x2} y2={y2}
      stroke={alpha(brand.cyan, 0.55)}
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeDasharray={len}
      strokeDashoffset={len * (1 - t)}
    />
  );
};

// Grace → 4 reports: spine-style elbow connector
const L4_XS   = [340, 550, 762, 972] as const;
const L3_CX   = 656;
const L3_BOT  = 380 + CARD_H;  // = 504, Grace bottom y
const ELBOW_Y = 534;            // horizontal bar y
const L4_TOP  = 555;            // L4 card top y

const ElbowConnector: React.FC<{ delay?: number }> = ({ delay = 0 }) => {
  const frame = useCurrentFrame();
  const t = track(frame, [delay, delay + 24], [0, 1]);
  const stroke = alpha(brand.cyan, 0.55);
  return (
    <g opacity={t}>
      {/* Stem from Grace down to elbow bar */}
      <line x1={L3_CX} y1={L3_BOT} x2={L3_CX} y2={ELBOW_Y}
        stroke={stroke} strokeWidth={1.5} strokeLinecap="round" />
      {/* Horizontal spine */}
      <line x1={L4_XS[0]} y1={ELBOW_Y} x2={L4_XS[L4_XS.length - 1]} y2={ELBOW_Y}
        stroke={stroke} strokeWidth={1.5} strokeLinecap="round" />
      {/* Drops to each child */}
      {L4_XS.map((x) => (
        <line key={x} x1={x} y1={ELBOW_Y} x2={x} y2={L4_TOP}
          stroke={stroke} strokeWidth={1.5} strokeLinecap="round" />
      ))}
    </g>
  );
};

interface AgentDef {
  name: string;
  role: string;
  pColor: string;
  pLabel: string;
  cx: number;    // horizontal center
  ty: number;    // top y
  delay: number;
}

const ORG: AgentDef[] = [
  // L1
  { name: 'Ada',      role: 'CEO',       pColor: COLORS.ada,      pLabel: 'claude', cx: 656,      ty: 20,    delay: 6  },
  // L2
  { name: 'Linus',    role: 'CTO',       pColor: COLORS.linus,    pLabel: 'codex',  cx: 656,      ty: 198,   delay: 20 },
  // L3
  { name: 'Grace',    role: 'Team Lead', pColor: COLORS.grace,    pLabel: 'claude', cx: 656,      ty: 380,   delay: 36 },
  // L4
  { name: 'Dijkstra', role: 'Dev',       pColor: COLORS.dijkstra, pLabel: 'claude', cx: L4_XS[0], ty: L4_TOP, delay: 55 },
  { name: 'Hopper',   role: 'Dev',       pColor: COLORS.hopper,   pLabel: 'codex',  cx: L4_XS[1], ty: L4_TOP, delay: 63 },
  { name: 'Turing',   role: 'Reviewer',  pColor: COLORS.turing,   pLabel: 'codex',  cx: L4_XS[2], ty: L4_TOP, delay: 71 },
  { name: 'Lovelace', role: 'QA',        pColor: COLORS.lovelace, pLabel: 'shell',  cx: L4_XS[3], ty: L4_TOP, delay: 79 },
];

const AgentNode: React.FC<AgentDef> = ({ name, role, pColor, pLabel, cx, ty, delay }) => (
  <div style={{ position: 'absolute', left: cx, top: ty, transform: 'translateX(-50%)' }}>
    <Appear delay={delay} y={10}>
      <div style={{
        display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 5,
        padding: '10px 16px', borderRadius: radius.l, minWidth: CARD_W,
        background: T.surface, border: `1px solid ${alpha(pColor, 0.45)}`,
        boxShadow: `0 6px 28px ${alpha(pColor, 0.2)}, ${T.shadow}`,
      }}>
        <Avatar name={name} size={34} color={pColor} />
        <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 700, color: T.text }}>{name}</span>
        <span style={{ fontFamily: fonts.ui, fontSize: 11, color: T.textDim }}>{role}</span>
        <Chip color={pColor}>{pLabel}</Chip>
      </div>
    </Appear>
  </div>
);

const OrgTreeScene: React.FC = () => (
  <>
    <Stage scale={0.87}>
      <OttoWindow
        nav={<Navigator active="swarm" />}
        title="Otto — Swarm · Payments Squad"
        tabs={[...SWARM_TABS_ORG]}
        width={1560}
        height={884}
      >
        <AbsoluteFill>
          {/* SVG connector lines — drawn behind agent cards */}
          <svg style={{ position: 'absolute', top: 0, left: 0, width: '100%', height: '100%', pointerEvents: 'none' }}>
            {/* L1 → L2 */}
            <OrgLine x1={L3_CX} y1={20 + CARD_H}   x2={L3_CX} y2={198}  delay={14} />
            {/* L2 → L3 */}
            <OrgLine x1={L3_CX} y1={198 + CARD_H}  x2={L3_CX} y2={380}  delay={28} />
            {/* L3 → L4 elbow spine */}
            <ElbowConnector delay={48} />
          </svg>
          {/* Agent nodes */}
          {ORG.map((a) => <AgentNode key={a.name} {...a} />)}
        </AbsoluteFill>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Org hierarchy — drafted by a Recruiter"
      sub="Each agent gets a role, skills & soul. CEO delegates to CTO → Team Lead → Dev / Reviewer / QA."
    />
  </>
);

// ─────────────────────────────────────────────────────────────────────────────
//  Scene 3 — Coordinator banner + Kanban board
// ─────────────────────────────────────────────────────────────────────────────

type CardStatus = 'idle' | 'working' | 'done' | 'review';

const CARD_DOT_TONE: Record<CardStatus, 'working' | 'idle' | 'needsYou'> = {
  idle:    'idle',
  working: 'working',
  done:    'working',
  review:  'needsYou',
};

const CARD_CHIP_CLR: Record<CardStatus, string> = {
  idle:    STATUS.idle,
  working: STATUS.working,
  done:    STATUS.working,
  review:  STATUS.needsYou,
};

const CARD_CHIP_LBL: Record<CardStatus, string> = {
  idle:    'queued',
  working: 'running',
  done:    'done',
  review:  'needs review',
};

interface TaskCardDef {
  title: string;
  owner: string;
  ownerColor: string;
  cardStatus: CardStatus;
  delay: number;
}

const TaskCard: React.FC<TaskCardDef> = ({ title, owner, ownerColor, cardStatus, delay }) => {
  const chipColor = CARD_CHIP_CLR[cardStatus];
  const isActive  = cardStatus === 'working';
  return (
    <Appear delay={delay} y={8}>
      <div style={{
        background: T.surface, borderRadius: radius.m, padding: '10px 12px', marginBottom: 8,
        border: `1px solid ${isActive ? alpha(chipColor, 0.4) : T.border}`,
        boxShadow: isActive ? `0 4px 18px ${alpha(chipColor, 0.18)}` : T.shadow,
      }}>
        <div style={{
          fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 600,
          color: T.text, marginBottom: 8, lineHeight: 1.4,
        }}>
          {title}
        </div>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
            <Avatar name={owner} size={17} color={ownerColor} />
            <span style={{ fontFamily: fonts.ui, fontSize: 11, color: T.textDim }}>{owner}</span>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
            <StatusDot kind={CARD_DOT_TONE[cardStatus]} size={7} pulse={cardStatus === 'working'} />
            <Chip color={chipColor}>{CARD_CHIP_LBL[cardStatus]}</Chip>
          </div>
        </div>
      </div>
    </Appear>
  );
};

interface ColumnDef {
  title: string;
  dotColor: string;
  tasks: TaskCardDef[];
  colDelay: number;
}

const KanbanCol: React.FC<ColumnDef> = ({ title, dotColor, tasks, colDelay }) => (
  <Appear
    delay={colDelay}
    y={18}
    style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}
  >
    <div style={{
      flex: 1, display: 'flex', flexDirection: 'column',
      background: alpha('#ffffff', 0.025),
      border: `1px solid ${T.border}`, borderRadius: radius.m,
    }}>
      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 8,
        padding: '10px 14px', borderBottom: `1px solid ${T.border}`, flexShrink: 0,
      }}>
        <span style={{ width: 8, height: 8, borderRadius: '50%', background: dotColor, flexShrink: 0 }} />
        <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 700, color: T.text, flex: 1 }}>{title}</span>
        <span style={{
          minWidth: 18, height: 18, padding: '0 5px', borderRadius: 999,
          background: T.surface2, fontFamily: fonts.ui, fontSize: 11,
          fontWeight: 600, color: T.textDim, display: 'grid', placeItems: 'center',
        }}>{tasks.length}</span>
      </div>
      {/* Cards */}
      <div style={{ padding: '10px 10px 0', flex: 1 }}>
        {tasks.map((task, i) => <TaskCard key={i} {...task} />)}
      </div>
    </div>
  </Appear>
);

const COLUMNS: ColumnDef[] = [
  {
    title: 'Backlog', dotColor: T.textDim, colDelay: 10,
    tasks: [
      { title: 'Add rate-limiting middleware', owner: 'Lovelace', ownerColor: COLORS.lovelace, cardStatus: 'idle',    delay: 22 },
      { title: 'Extend OAuth token scopes',    owner: 'Dijkstra', ownerColor: COLORS.dijkstra, cardStatus: 'idle',    delay: 28 },
    ],
  },
  {
    title: 'In Progress', dotColor: brand.cyan, colDelay: 14,
    tasks: [
      { title: 'Implement JWT refresh flow',   owner: 'Hopper',   ownerColor: COLORS.hopper,   cardStatus: 'working', delay: 30 },
      { title: 'Build user search endpoint',   owner: 'Dijkstra', ownerColor: COLORS.dijkstra, cardStatus: 'working', delay: 36 },
    ],
  },
  {
    title: 'Review', dotColor: STATUS.needsYou, colDelay: 18,
    tasks: [
      { title: 'DB migration v3',              owner: 'Turing',   ownerColor: COLORS.turing,   cardStatus: 'review',  delay: 40 },
      { title: 'API error-response format',    owner: 'Grace',    ownerColor: COLORS.grace,    cardStatus: 'review',  delay: 46 },
    ],
  },
  {
    title: 'Done', dotColor: STATUS.working, colDelay: 22,
    tasks: [
      { title: 'Initialise monorepo',          owner: 'Ada',      ownerColor: COLORS.ada,      cardStatus: 'done',    delay: 50 },
      { title: 'Design schema v1',             owner: 'Linus',    ownerColor: COLORS.linus,    cardStatus: 'done',    delay: 56 },
      { title: 'Auth service (basic)',          owner: 'Grace',    ownerColor: COLORS.grace,    cardStatus: 'done',    delay: 62 },
    ],
  },
];

const COORD_AGENTS: Array<{ name: string; color: string; active: boolean }> = [
  { name: 'Dijkstra', color: COLORS.dijkstra, active: true  },
  { name: 'Hopper',   color: COLORS.hopper,   active: true  },
  { name: 'Turing',   color: COLORS.turing,   active: true  },
  { name: 'Grace',    color: COLORS.grace,    active: false },  // scheduled, not yet dispatched
];

const KanbanScene: React.FC = () => (
  <>
    <Stage scale={0.87}>
      <OttoWindow
        nav={<Navigator active="swarm" />}
        title="Otto — Swarm · Payments Squad"
        tabs={[...SWARM_TABS_BOARD]}
        width={1560}
        height={884}
      >
        <div style={{
          display: 'flex', flexDirection: 'column', height: '100%',
          padding: 16, gap: 12, boxSizing: 'border-box',
        }}>
          {/* Coordinator status banner */}
          <Appear delay={4}>
            <div style={{
              display: 'flex', alignItems: 'center', gap: 14,
              padding: '0 18px', height: 48, flexShrink: 0,
              background: alpha(brand.cyan, 0.07), borderRadius: radius.m,
              border: `1px solid ${alpha(brand.cyan, 0.3)}`,
            }}>
              <Icon name="grid" size={15} color={brand.cyan} />
              <span style={{ fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 700, color: T.text }}>
                Coordinator
              </span>
              <div style={{ width: 1, height: 22, background: T.border, margin: '0 2px' }} />
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <StatusDot kind="working" size={8} />
                <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 600, color: T.text }}>
                  3 / 4 sessions active
                </span>
                <Chip color={brand.cyan}>cap 4</Chip>
              </div>
              <span style={{ flex: 1 }} />
              {/* Per-agent indicators */}
              <div style={{ display: 'flex', gap: 10, alignItems: 'center' }}>
                {COORD_AGENTS.map(({ name, color, active }) => (
                  <div key={name} style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                    <StatusDot kind={active ? 'working' : 'idle'} size={7} pulse={active} />
                    <Avatar name={name} size={20} color={color} />
                    {!active && (
                      <span style={{ fontFamily: fonts.ui, fontSize: 10, color: T.textDim }}>scheduled</span>
                    )}
                  </div>
                ))}
              </div>
            </div>
          </Appear>

          {/* Four Kanban columns */}
          <div style={{ flex: 1, display: 'flex', gap: 12, minHeight: 0 }}>
            {COLUMNS.map((col) => <KanbanCol key={col.title} {...col} />)}
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Coordinator schedules work within the session cap"
      sub="Tasks flow Backlog → In Progress → Review → Done. Agents auto-assigned; coordinator respects the parallel cap."
    />
  </>
);

// ─────────────────────────────────────────────────────────────────────────────
//  Scene 4 — Run-graph (DAG) + swarm controls
//  Row-1 at cy=115, row-2 at cy=285. Controls banner at content bottom.
// ─────────────────────────────────────────────────────────────────────────────

type RunStatus = 'done' | 'working' | 'idle';

const RUN_DOT_KIND: Record<RunStatus, 'working' | 'idle'> = {
  done: 'working', working: 'working', idle: 'idle',
};
const RUN_CHIP_CLR: Record<RunStatus, string> = {
  done: STATUS.working, working: brand.cyan, idle: STATUS.idle,
};
const RUN_CHIP_LBL: Record<RunStatus, string> = {
  done: 'done', working: 'running', idle: 'queued',
};

// Node geometry
const NW  = 188;        // node width
const NH  = 60;         // node height
const HNW = NW / 2;     // 94
const HNH = NH / 2;     // 30

interface RunNodeDef {
  label: string;
  owner: string;
  ownerColor: string;
  runStatus: RunStatus;
  cx: number;    // center x
  cy: number;    // center y
  delay: number;
}

const RUN_NODES: RunNodeDef[] = [
  // Row 1 — main track
  { label: 'Init repo',         owner: 'Ada',      ownerColor: COLORS.ada,      runStatus: 'done',    cx: 155,  cy: 115, delay: 6  },
  { label: 'Design schema',     owner: 'Linus',    ownerColor: COLORS.linus,    runStatus: 'done',    cx: 385,  cy: 115, delay: 12 },
  { label: 'Auth service',      owner: 'Grace',    ownerColor: COLORS.grace,    runStatus: 'done',    cx: 615,  cy: 115, delay: 18 },
  { label: 'REST endpoints',    owner: 'Dijkstra', ownerColor: COLORS.dijkstra, runStatus: 'working', cx: 850,  cy: 115, delay: 24 },
  { label: 'Integration tests', owner: 'Hopper',   ownerColor: COLORS.hopper,   runStatus: 'idle',    cx: 1100, cy: 115, delay: 30 },
  // Row 2 — parallel frontend track
  { label: 'Frontend setup',    owner: 'Hopper',   ownerColor: COLORS.hopper,   runStatus: 'working', cx: 850,  cy: 285, delay: 36 },
  { label: 'E2E test suite',    owner: 'Lovelace', ownerColor: COLORS.lovelace, runStatus: 'idle',    cx: 1100, cy: 285, delay: 42 },
];

// DAG edges: [x1, y1, x2, y2, animDelay]
const DAG_EDGES: Array<[number, number, number, number, number]> = [
  [155 + HNW, 115,        385 - HNW, 115,        4  ],  // Init → Schema
  [385 + HNW, 115,        615 - HNW, 115,        10 ],  // Schema → Auth
  [615 + HNW, 115,        850 - HNW, 115,        16 ],  // Auth → REST
  [850 + HNW, 115,       1100 - HNW, 115,        22 ],  // REST → Integration
  [850,       115 + HNH,  850,       285 - HNH,  28 ],  // REST → Frontend (vertical)
  [850 + HNW, 285,       1100 - HNW, 285,        36 ],  // Frontend → E2E
];

const DagEdge: React.FC<{ x1: number; y1: number; x2: number; y2: number; delay?: number }> = ({
  x1, y1, x2, y2, delay = 0,
}) => {
  const frame = useCurrentFrame();
  const len = Math.ceil(Math.sqrt((x2 - x1) ** 2 + (y2 - y1) ** 2));
  const t = track(frame, [delay, delay + 16], [0, 1]);
  return (
    <line
      x1={x1} y1={y1} x2={x2} y2={y2}
      stroke={alpha(T.textDim, 0.5)}
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeDasharray={len}
      strokeDashoffset={len * (1 - t)}
    />
  );
};

const RunNode: React.FC<RunNodeDef> = ({ label, owner, ownerColor, runStatus, cx, cy, delay }) => {
  const chipColor = RUN_CHIP_CLR[runStatus];
  const isRunning = runStatus === 'working';
  return (
    <div style={{ position: 'absolute', left: cx - HNW, top: cy - HNH }}>
      <Appear delay={delay} y={8}>
        <div style={{
          width: NW, height: NH,
          display: 'flex', alignItems: 'center', gap: 9,
          padding: '0 12px', borderRadius: radius.m, boxSizing: 'border-box',
          background: isRunning ? alpha(chipColor, 0.09) : T.surface,
          border: `1px solid ${isRunning ? alpha(chipColor, 0.5) : T.border}`,
          boxShadow: isRunning ? `0 4px 22px ${alpha(chipColor, 0.24)}` : T.shadow,
        }}>
          <StatusDot kind={RUN_DOT_KIND[runStatus]} size={9} pulse={isRunning} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{
              fontFamily: fonts.ui, fontSize: 12, fontWeight: 700, color: T.text,
              overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
            }}>
              {label}
            </div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 4, marginTop: 3 }}>
              <Avatar name={owner} size={14} color={ownerColor} />
              <span style={{ fontFamily: fonts.ui, fontSize: 10, color: T.textDim }}>{owner}</span>
            </div>
          </div>
          <Chip color={chipColor}>{RUN_CHIP_LBL[runStatus]}</Chip>
        </div>
      </Appear>
    </div>
  );
};

const RunGraphScene: React.FC = () => (
  <>
    <Stage scale={0.87}>
      <OttoWindow
        nav={<Navigator active="swarm" />}
        title="Otto — Swarm · Run Graph"
        tabs={[...SWARM_TABS_RUN]}
        width={1560}
        height={884}
      >
        <AbsoluteFill>
          {/* SVG edge layer */}
          <svg style={{ position: 'absolute', top: 0, left: 0, width: '100%', height: '100%', pointerEvents: 'none' }}>
            {DAG_EDGES.map(([x1, y1, x2, y2, d], i) => (
              <DagEdge key={i} x1={x1} y1={y1} x2={x2} y2={y2} delay={d} />
            ))}
          </svg>

          {/* Task nodes */}
          {RUN_NODES.map((n) => <RunNode key={n.label} {...n} />)}

          {/* Swarm controls banner */}
          <div style={{ position: 'absolute', left: 32, right: 32, bottom: 42 }}>
            <Appear delay={52}>
              <div style={{
                display: 'flex', alignItems: 'center', gap: 10,
                padding: '12px 20px', borderRadius: radius.m,
                background: T.surface, border: `1px solid ${T.border}`, boxShadow: T.shadow,
              }}>
                <span style={{
                  fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 600,
                  color: T.textDim, marginRight: 4,
                }}>
                  Swarm controls
                </span>
                <Button variant="ghost" icon="square">Pause</Button>
                <Button variant="danger" icon="x">Abort all</Button>
                <Button variant="primary" icon="play">Resume</Button>
                <span style={{ flex: 1 }} />
                <Chip color={brand.cyan}>worktree-isolated</Chip>
                <Chip color={brand.violet}>merged back</Chip>
              </div>
            </Appear>
          </div>
        </AbsoluteFill>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="Org tree · run-graph · board — pause / abort / resume anytime"
      sub="Per-agent worktree isolation. Leader goal-verify loop. Integration branch merged back on task completion."
    />
  </>
);

// ─────────────────────────────────────────────────────────────────────────────
//  Scene list + exports
// ─────────────────────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 75,  name: 'Title',    node: <TitleScene />    },
  { dur: 190, name: 'OrgTree',  node: <OrgTreeScene />  },
  { dur: 190, name: 'Kanban',   node: <KanbanScene />   },
  { dur: 170, name: 'RunGraph', node: <RunGraphScene />  },
  {
    dur: 130,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Agent Swarm"
        tagline="Stand up a whole team of agents — and stay in control"
        pills={[
          { label: 'Org hierarchy', icon: 'grid'    },
          { label: 'Coordinator',   icon: 'refresh' },
          { label: 'Kanban + DAG',  icon: 'split'   },
          { label: '5 presets',     icon: 'box'     },
        ]}
      />
    ),
  },
];

export const swarmDuration = scenesDuration(SCENES);
export const Swarm: React.FC = () => <Scenes scenes={SCENES} />;
