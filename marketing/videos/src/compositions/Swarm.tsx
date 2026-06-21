import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, providers, status, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  TitleCard,
  Caption,
  Appear,
  Avatar,
  StatusDot,
  Chip,
  Icon,
  track,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  AGENT SWARM — teams of role-specialized agents, an org tree & a coordinator
// ════════════════════════════════════════════════════════════════════════════

// ── Scene 1 — title card ──────────────────────────────────────────────────────
const Title: React.FC = () => (
  <TitleCard
    kicker="Agent Swarm"
    title="A whole team of agents"
    subtitle="Roles, an org tree, and a coordinator that schedules the work"
  />
);

// ── shared bits ───────────────────────────────────────────────────────────────
const ProviderChip: React.FC<{ name: string; color: string }> = ({ name, color }) => (
  <span
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 5,
      padding: '0 7px',
      height: 17,
      borderRadius: 999,
      fontFamily: fonts.ui,
      fontSize: 10,
      fontWeight: 600,
      color,
      background: alpha(color, 0.16),
      border: `1px solid ${alpha(color, 0.38)}`,
      whiteSpace: 'nowrap',
    }}
  >
    <span style={{ width: 6, height: 6, borderRadius: '50%', background: color }} />
    {name}
  </span>
);

// ── Scene 2 — org tree ────────────────────────────────────────────────────────
interface OrgNode {
  role: string;
  person: string;
  color: string;
  provider: string;
  provColor: string;
  st: keyof typeof status;
}

const COORD: OrgNode = { role: 'Coordinator', person: 'C', color: brand.cyan, provider: 'claude', provColor: providers.claude, st: 'working' };
const LEADS: OrgNode[] = [
  { role: 'CTO', person: 'T', color: brand.violet, provider: 'claude', provColor: providers.claude, st: 'working' },
  { role: 'VP Eng', person: 'V', color: '#0a84ff', provider: 'codex', provColor: providers.codex, st: 'idle' },
];
const LEAVES: OrgNode[] = [
  { role: 'Backend Eng', person: 'B', color: providers.codex, provider: 'codex', provColor: providers.codex, st: 'working' },
  { role: 'Backend Eng', person: 'B', color: providers.codex, provider: 'codex', provColor: providers.codex, st: 'working' },
  { role: 'Reviewer', person: 'R', color: brand.violet, provider: 'claude', provColor: providers.claude, st: 'idle' },
  { role: 'QA', person: 'Q', color: '#febc2e', provider: 'gemini', provColor: providers.gemini, st: 'working' },
];

const OrgCard: React.FC<{ node: OrgNode; w?: number }> = ({ node, w = 184 }) => (
  <div
    style={{
      width: w,
      display: 'flex',
      alignItems: 'center',
      gap: 10,
      padding: '11px 12px',
      borderRadius: 12,
      background: T.surface,
      border: `1px solid ${alpha(node.color, 0.4)}`,
      boxShadow: `0 8px 26px rgba(0,0,0,0.4), 0 0 0 1px ${alpha(node.color, 0.12)}`,
      boxSizing: 'border-box',
    }}
  >
    <Avatar name={node.person} color={node.color} size={34} />
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        <span style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 700, color: T.text }}>{node.role}</span>
        <StatusDot kind={node.st} size={7} />
      </div>
      <div style={{ marginTop: 5 }}>
        <ProviderChip name={node.provider} color={node.provColor} />
      </div>
    </div>
  </div>
);

// SVG connector layer drawn behind the cards (relative to the content box).
const OrgWires: React.FC = () => {
  const frame = useCurrentFrame();
  const draw = track(frame, [10, 40], [0, 1]);
  const W = 1130;
  const H = 560;
  // coordinate anchors (matched to the flex layout below)
  const coordX = W / 2;
  const coordY = 96;
  const leadY = 250;
  const leadXs = [W / 2 - 220, W / 2 + 220];
  const leafY = 420;
  const leafXs = [W / 2 - 360, W / 2 - 120, W / 2 + 120, W / 2 + 360];
  const stroke = alpha(brand.cyan, 0.5);
  const lines: [number, number, number, number][] = [
    // coordinator → leads
    [coordX, coordY, leadXs[0], leadY],
    [coordX, coordY, leadXs[1], leadY],
    // CTO → backend ×2
    [leadXs[0], leadY, leafXs[0], leafY],
    [leadXs[0], leadY, leafXs[1], leafY],
    // VP Eng → reviewer, QA
    [leadXs[1], leadY, leafXs[2], leafY],
    [leadXs[1], leadY, leafXs[3], leafY],
  ];
  return (
    <svg width={W} height={H} style={{ position: 'absolute', left: '50%', top: 0, transform: 'translateX(-50%)', pointerEvents: 'none' }}>
      {lines.map(([x1, y1, x2, y2], i) => {
        const my = (y1 + y2) / 2;
        const d = `M${x1},${y1} C${x1},${my} ${x2},${my} ${x2},${y2}`;
        return (
          <path
            key={i}
            d={d}
            fill="none"
            stroke={stroke}
            strokeWidth={2}
            strokeDasharray={460}
            strokeDashoffset={460 * (1 - draw)}
          />
        );
      })}
    </svg>
  );
};

const OrgTreeScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="swarm" counts={{ swarm: 7 }} />}
        title="Otto — Swarm · Platform Team"
      >
        <div style={{ position: 'relative', height: '100%', padding: '28px 0', boxSizing: 'border-box' }}>
          {/* board header */}
          <div style={{ position: 'absolute', top: 18, left: 26, display: 'flex', alignItems: 'center', gap: 10 }}>
            <Icon name="grid" size={16} color={brand.cyan} />
            <span style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 700, color: T.text }}>Org Tree</span>
            <Chip color={brand.cyan}>7 agents</Chip>
            <Chip tone="default">3 working</Chip>
          </div>

          <OrgWires />

          <div style={{ position: 'relative', height: '100%' }}>
            {/* Coordinator */}
            <div style={{ position: 'absolute', top: 70, left: '50%', transform: 'translateX(-50%)' }}>
              <Appear delay={4} y={14}>
                <OrgCard node={COORD} w={210} />
              </Appear>
            </div>
            {/* Leads row */}
            <div style={{ position: 'absolute', top: 224, left: '50%', transform: 'translateX(-50%)', display: 'flex', gap: 256 }}>
              {LEADS.map((n, i) => (
                <Appear key={i} delay={14 + i * 4} y={14}>
                  <OrgCard node={n} w={196} />
                </Appear>
              ))}
            </div>
            {/* Leaves row */}
            <div style={{ position: 'absolute', top: 396, left: '50%', transform: 'translateX(-50%)', display: 'flex', gap: 56 }}>
              {LEAVES.map((n, i) => (
                <Appear key={i} delay={26 + i * 5} y={14}>
                  <OrgCard node={n} w={184} />
                </Appear>
              ))}
            </div>
            {/* recruiter strip */}
            <div style={{ position: 'absolute', bottom: 6, left: '50%', transform: 'translateX(-50%)' }}>
              <Appear delay={52} y={12}>
                <div
                  style={{
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: 9,
                    padding: '8px 14px',
                    borderRadius: 999,
                    background: alpha(providers.claude, 0.1),
                    border: `1px solid ${alpha(providers.claude, 0.32)}`,
                    fontFamily: fonts.ui,
                    fontSize: 13,
                    color: T.text,
                  }}
                >
                  <Icon name="user" size={14} color={providers.claude} />
                  Recruiter drafted each agent — role, persona, skills &amp; schedule
                </div>
              </Appear>
            </div>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Role agents in an org hierarchy"
      sub="A recruiter drafts each one · 5 presets ship in the box"
    />
  </>
);

// ── Scene 3 — Kanban board ────────────────────────────────────────────────────
interface KCard {
  title: string;
  who: string;
  whoColor: string;
  st: keyof typeof status;
  stLabel: string;
}

const COLUMNS: { name: string; tone: string; cards: KCard[] }[] = [
  {
    name: 'Backlog',
    tone: T.textDim,
    cards: [
      { title: 'Add pagination to /orders', who: 'B', whoColor: providers.codex, st: 'idle', stLabel: 'queued' },
      { title: 'Cache currency rates', who: 'B', whoColor: providers.codex, st: 'idle', stLabel: 'queued' },
    ],
  },
  {
    name: 'In progress',
    tone: status.working,
    cards: [
      { title: 'JWT refresh middleware', who: 'B', whoColor: providers.codex, st: 'working', stLabel: 'running' },
      { title: 'Rate-limit gateway', who: 'V', whoColor: '#0a84ff', st: 'working', stLabel: 'running' },
    ],
  },
  {
    name: 'Review',
    tone: brand.violet,
    cards: [
      { title: 'Refactor wallet ledger', who: 'R', whoColor: brand.violet, st: 'needsYou', stLabel: 'in review' },
    ],
  },
  {
    name: 'Done',
    tone: status.working,
    cards: [
      { title: 'Fix flaky auth tests', who: 'Q', whoColor: '#febc2e', st: 'exited', stLabel: 'merged' },
      { title: 'Seed staging DB', who: 'B', whoColor: providers.codex, st: 'exited', stLabel: 'merged' },
    ],
  },
];

const TaskCard: React.FC<{ c: KCard; moving?: boolean }> = ({ c, moving }) => {
  const frame = useCurrentFrame();
  const lift = moving ? track(frame, [70, 96], [0, 1]) : 0;
  return (
    <div
      style={{
        padding: '11px 12px',
        borderRadius: 10,
        background: T.surface,
        border: `1px solid ${moving ? alpha(brand.cyan, 0.6) : T.border}`,
        boxShadow: moving
          ? `0 ${12 + lift * 8}px ${22 + lift * 10}px rgba(0,0,0,0.5), 0 0 0 1px ${alpha(brand.cyan, 0.4)}`
          : '0 4px 14px rgba(0,0,0,0.28)',
        transform: moving ? `translate(${lift * 18}px, ${-lift * 6}px) rotate(${lift * 1.4}deg)` : 'none',
      }}
    >
      <div style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 600, color: T.text, lineHeight: 1.3 }}>{c.title}</div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 9 }}>
        <Avatar name={c.who} color={c.whoColor} size={20} />
        <span style={{ flex: 1 }} />
        <StatusDot kind={c.st} size={7} />
        <span style={{ fontFamily: fonts.ui, fontSize: 11, color: T.textDim }}>{c.stLabel}</span>
      </div>
    </div>
  );
};

const KanbanScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="swarm" counts={{ swarm: 7 }} />}
        tabs={[
          { label: 'Kanban', icon: 'grid', active: true },
          { label: 'Run Graph', icon: 'split' },
          { label: 'Org Tree', icon: 'box' },
          { label: 'Runs', icon: 'clock' },
          { label: 'Board', icon: 'comment' },
        ]}
        title="Otto — Swarm · Checkout Revamp"
      >
        <div style={{ display: 'flex', gap: 14, padding: 20, height: '100%', boxSizing: 'border-box' }}>
          {COLUMNS.map((col, ci) => (
            <Appear key={col.name} delay={6 + ci * 6} y={18} style={{ flex: 1, display: 'flex' }}>
              <div
                style={{
                  flex: 1,
                  display: 'flex',
                  flexDirection: 'column',
                  background: alpha('#fff', 0.018),
                  border: `1px solid ${T.border}`,
                  borderRadius: 12,
                  padding: 11,
                  minWidth: 0,
                }}
              >
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 11, padding: '0 2px' }}>
                  <span style={{ width: 8, height: 8, borderRadius: '50%', background: col.tone }} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 700, color: T.text }}>{col.name}</span>
                  <span style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>{col.cards.length}</span>
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
                  {col.cards.map((c, i) => (
                    <Appear key={i} delay={16 + ci * 6 + i * 5} y={12}>
                      <TaskCard c={c} moving={ci === 1 && i === 0} />
                    </Appear>
                  ))}
                </div>
              </div>
            </Appear>
          ))}
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Projects break into tasks on a board"
      sub="The Coordinator assigns work within a parallel cap"
    />
  </>
);

// ── Scene 4 — run graph + board ───────────────────────────────────────────────
interface GNode {
  id: string;
  label: string;
  x: number;
  y: number;
  st: keyof typeof status;
  color: string;
}

const GNODES: GNode[] = [
  { id: 'plan', label: 'Plan', x: 70, y: 150, st: 'exited', color: status.idle },
  { id: 'api', label: 'API layer', x: 280, y: 78, st: 'exited', color: status.idle },
  { id: 'db', label: 'DB schema', x: 280, y: 222, st: 'exited', color: status.idle },
  { id: 'impl', label: 'Implement', x: 500, y: 150, st: 'working', color: status.working },
  { id: 'review', label: 'Review', x: 720, y: 78, st: 'working', color: status.working },
  { id: 'qa', label: 'QA', x: 720, y: 222, st: 'idle', color: status.idle },
  { id: 'merge', label: 'Merge PR', x: 930, y: 150, st: 'idle', color: status.idle },
];
const GEDGES: [string, string][] = [
  ['plan', 'api'],
  ['plan', 'db'],
  ['api', 'impl'],
  ['db', 'impl'],
  ['impl', 'review'],
  ['impl', 'qa'],
  ['review', 'merge'],
  ['qa', 'merge'],
];

const RunGraph: React.FC = () => {
  const frame = useCurrentFrame();
  const draw = track(frame, [8, 40], [0, 1]);
  const byId = (id: string) => GNODES.find((n) => n.id === id)!;
  const NW = 116;
  const NH = 44;
  return (
    <div style={{ position: 'relative', width: 1046, height: 320 }}>
      <svg width={1046} height={320} style={{ position: 'absolute', inset: 0, pointerEvents: 'none' }}>
        {GEDGES.map(([a, b], i) => {
          const na = byId(a);
          const nb = byId(b);
          const x1 = na.x + NW;
          const y1 = na.y + NH / 2;
          const x2 = nb.x;
          const y2 = nb.y + NH / 2;
          const mx = (x1 + x2) / 2;
          const d = `M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}`;
          const active = na.st === 'exited' && (nb.st === 'working' || nb.st === 'exited');
          const col = active ? brand.cyan : alpha('#fff', 0.18);
          return (
            <path
              key={i}
              d={d}
              fill="none"
              stroke={col}
              strokeWidth={active ? 2.4 : 1.6}
              strokeDasharray={300}
              strokeDashoffset={300 * (1 - draw)}
            />
          );
        })}
      </svg>
      {GNODES.map((n, i) => {
        const op = track(frame, [10 + i * 4, 22 + i * 4], [0, 1]);
        const lit = n.st === 'working';
        return (
          <div
            key={n.id}
            style={{
              position: 'absolute',
              left: n.x,
              top: n.y,
              width: NW,
              height: NH,
              opacity: op,
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              padding: '0 12px',
              boxSizing: 'border-box',
              borderRadius: 10,
              background: lit ? alpha(status.working, 0.14) : T.surface,
              border: `1px solid ${lit ? alpha(status.working, 0.6) : T.border}`,
              boxShadow: lit ? `0 0 22px ${alpha(status.working, 0.35)}` : '0 4px 12px rgba(0,0,0,0.3)',
            }}
          >
            <StatusDot kind={n.st} size={8} pulse={lit} />
            <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 600, color: T.text }}>{n.label}</span>
          </div>
        );
      })}
    </div>
  );
};

interface BoardMsg {
  who: string;
  color: string;
  text: string;
  st: keyof typeof status;
}
const BOARD: BoardMsg[] = [
  { who: 'Reviewer', color: brand.violet, text: 'approved #142 — wallet ledger LGTM', st: 'working' },
  { who: 'QA', color: '#febc2e', text: '3 cases passing · 0 failing', st: 'working' },
  { who: 'Backend Eng', color: providers.codex, text: 'pushed impl, requesting review', st: 'idle' },
  { who: 'Coordinator', color: brand.cyan, text: 'handed off Merge PR → VP Eng', st: 'idle' },
];

const SwarmBoard: React.FC = () => (
  <div
    style={{
      width: 360,
      flexShrink: 0,
      display: 'flex',
      flexDirection: 'column',
      background: alpha('#fff', 0.02),
      border: `1px solid ${T.border}`,
      borderRadius: 12,
      padding: 12,
    }}
  >
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
      <Icon name="comment" size={15} color={brand.cyan} />
      <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 700, color: T.text }}>Board</span>
      <StatusDot kind="working" size={7} />
    </div>
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      {BOARD.map((m, i) => (
        <Appear key={i} delay={30 + i * 7} y={10}>
          <div style={{ display: 'flex', gap: 9, alignItems: 'flex-start' }}>
            <Avatar name={m.who} color={m.color} size={24} />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 700, color: m.color }}>{m.who}</span>
                <StatusDot kind={m.st} size={6} />
              </div>
              <div style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim, marginTop: 3, lineHeight: 1.35 }}>{m.text}</div>
            </div>
          </div>
        </Appear>
      ))}
    </div>
  </div>
);

const RunGraphScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="swarm" counts={{ swarm: 7 }} />}
        tabs={[
          { label: 'Kanban', icon: 'grid' },
          { label: 'Run Graph', icon: 'split', active: true, dot: 'working' },
          { label: 'Org Tree', icon: 'box' },
          { label: 'Runs', icon: 'clock' },
          { label: 'Board', icon: 'comment' },
        ]}
        title="Otto — Swarm · Checkout Revamp"
      >
        <div style={{ display: 'flex', gap: 16, padding: 20, height: '100%', boxSizing: 'border-box' }}>
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 8 }}>
              <Icon name="split" size={16} color={brand.cyan} />
              <span style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 700, color: T.text }}>Run Graph</span>
              <Chip color={status.working}>2 active</Chip>
              <span style={{ flex: 1 }} />
              <Chip tone="default">pause</Chip>
              <Chip tone="default">abort</Chip>
              <Chip color={brand.cyan}>resume</Chip>
            </div>
            <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <RunGraph />
            </div>
          </div>
          <SwarmBoard />
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="Watch the run graph live"
      sub="Delegation, hand-offs &amp; a shared board — pause/abort/resume anytime"
    />
  </>
);

// ── Scene 5 — outro ───────────────────────────────────────────────────────────
const Outro: React.FC = () => (
  <WalkOutro
    title="Agent Swarm"
    tagline="Delegate to a team that runs itself."
    pills={[
      { label: 'Org tree', color: brand.cyan, icon: 'grid' },
      { label: 'Recruiter', color: providers.claude, icon: 'user' },
      { label: 'Kanban', color: '#0a84ff', icon: 'grid' },
      { label: 'Run graph', color: brand.violet, icon: 'split' },
      { label: 'Budgets', color: '#febc2e', icon: 'gauge' },
    ]}
  />
);

const SCENES: SceneDef[] = [
  { dur: 80, node: <Title />, name: 'Title' },
  { dur: 230, node: <OrgTreeScene />, name: 'Org Tree' },
  { dur: 230, node: <KanbanScene />, name: 'Kanban' },
  { dur: 210, node: <RunGraphScene />, name: 'Run Graph' },
  { dur: 130, node: <Outro />, name: 'Outro' },
];

export const swarmDuration = scenesDuration(SCENES);
export const Swarm: React.FC = () => <Scenes scenes={SCENES} />;
