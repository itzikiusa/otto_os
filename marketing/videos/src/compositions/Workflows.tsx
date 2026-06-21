import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, status as STATUS, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  TitleCard,
  Caption,
  Appear,
  Stagger,
  Chip,
  Button,
  Card,
  Segmented,
  StatusDot,
  Icon,
  track,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  WORKFLOWS — visual automation: typed nodes wired on a canvas, triggers
//  (webhook / cron / manual / event), runs that pause at human-approval gates,
//  and a run inspector with per-node results, logs & saved views. Native dark.
// ════════════════════════════════════════════════════════════════════════════

const mono = fonts.mono;

// node-type accent palette (matches the brand pills in the outro)
const C = {
  trigger: brand.cyan,
  review: brand.violet,
  approval: '#febc2e',
  notify: '#36c5f0',
  product: '#9ee039',
  swarm: brand.cyan,
  db: '#0a84ff',
  api: '#bf7aff',
} as const;

// ── Scene 1 — title card ──────────────────────────────────────────────────────
const TitleScene: React.FC = () => (
  <TitleCard
    kicker="WORKFLOWS"
    title="Wire your agents into pipelines"
    subtitle="Typed nodes, triggers & approval gates — drawn on a canvas"
  />
);

// ════════════════════════════════════════════════════════════════════════════
//  Scene 2 — the canvas: a node graph wired left→right + a node-type palette
// ════════════════════════════════════════════════════════════════════════════

interface FlowNode {
  id: string;
  title: string;
  type: string;
  icon: string;
  color: string;
  x: number;
  y: number;
  st: keyof typeof STATUS;
  pending?: boolean;
}

const NW = 196;
const NH = 78;

const FLOW_NODES: FlowNode[] = [
  { id: 'trigger', title: 'Webhook', type: 'on PR opened', icon: 'zap', color: C.trigger, x: 18, y: 150, st: 'exited' },
  { id: 'review', title: 'review_run', type: 'multi-agent review', icon: 'eye', color: C.review, x: 300, y: 150, st: 'exited' },
  { id: 'approval', title: 'human_approval', type: 'approve deploy?', icon: 'check', color: C.approval, x: 582, y: 150, st: 'needsYou', pending: true },
  { id: 'notify', title: 'channel_notify', type: 'Slack #releases', icon: 'slack', color: C.notify, x: 864, y: 150, st: 'idle' },
];

const FLOW_EDGES: [string, string][] = [
  ['trigger', 'review'],
  ['review', 'approval'],
  ['approval', 'notify'],
];

// node card on the canvas
const CanvasNode: React.FC<{ n: FlowNode; delay: number }> = ({ n, delay }) => {
  const frame = useCurrentFrame();
  const op = track(frame, [delay, delay + 10], [0, 1]);
  const y = track(frame, [delay, delay + 10], [12, 0]);
  // approval node pulses to read as "pending / waiting on you"
  const pulse = n.pending ? Math.sin(frame / 9) * 0.5 + 0.5 : 0;
  const glow = n.pending
    ? `0 0 ${22 + pulse * 18}px ${alpha(n.color, 0.42)}`
    : '0 10px 26px rgba(0,0,0,0.42)';
  return (
    <Card
      pad={0}
      style={{
        position: 'absolute',
        left: n.x,
        top: n.y,
        width: NW,
        height: NH,
        opacity: op,
        transform: `translateY(${y}px)`,
        border: `1px solid ${alpha(n.color, n.pending ? 0.7 : 0.42)}`,
        boxShadow: glow,
        overflow: 'hidden',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', height: '100%', padding: '0 13px', gap: 11 }}>
        <span
          style={{
            width: 38,
            height: 38,
            borderRadius: 10,
            flexShrink: 0,
            background: alpha(n.color, 0.16),
            color: n.color,
            display: 'grid',
            placeItems: 'center',
          }}
        >
          <Icon name={n.icon} size={19} />
        </span>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
            <span style={{ fontFamily: mono, fontSize: 13.5, fontWeight: 700, color: T.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {n.title}
            </span>
            <StatusDot kind={n.st} size={7} pulse={n.pending} />
          </div>
          <div style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim, marginTop: 4, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {n.type}
          </div>
          <div style={{ marginTop: 5 }}>
            <span
              style={{
                fontFamily: mono,
                fontSize: 9.5,
                fontWeight: 700,
                letterSpacing: 0.4,
                textTransform: 'uppercase',
                color: n.color,
                background: alpha(n.color, 0.14),
                border: `1px solid ${alpha(n.color, 0.34)}`,
                borderRadius: 4,
                padding: '1px 6px',
              }}
            >
              {n.id === 'trigger' ? 'trigger' : n.title.split(' ')[0]}
            </span>
          </div>
        </div>
      </div>
    </Card>
  );
};

// SVG edges with a draw-in + one animated moving dot (trigger→review)
const CanvasEdges: React.FC<{ w: number; h: number }> = ({ w, h }) => {
  const frame = useCurrentFrame();
  const draw = track(frame, [10, 42], [0, 1]);
  const byId = (id: string) => FLOW_NODES.find((n) => n.id === id)!;
  // a looping 0→1 progress for the moving dot on the first edge
  const dotT = (frame % 70) / 70;

  const edgePath = (a: string, b: string) => {
    const na = byId(a);
    const nb = byId(b);
    const x1 = na.x + NW;
    const y1 = na.y + NH / 2;
    const x2 = nb.x;
    const y2 = nb.y + NH / 2;
    const mx = (x1 + x2) / 2;
    return { d: `M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}`, x1, y1, x2, y2 };
  };

  // moving dot position along the first edge (cubic with flat control y → straight)
  const e0 = edgePath('trigger', 'review');
  const dotX = e0.x1 + (e0.x2 - e0.x1) * dotT;
  const dotY = e0.y1; // both endpoints share y for this segment

  return (
    <svg width={w} height={h} style={{ position: 'absolute', inset: 0, pointerEvents: 'none' }}>
      {FLOW_EDGES.map(([a, b], i) => {
        const { d } = edgePath(a, b);
        // the approval→notify edge is "pending" (the run hasn't passed the gate yet)
        const pending = a === 'approval';
        const col = pending ? alpha(C.approval, 0.5) : alpha(brand.cyan, 0.6);
        return (
          <path
            key={i}
            d={d}
            fill="none"
            stroke={col}
            strokeWidth={2.4}
            strokeDasharray={pending ? '7 7' : 320}
            strokeDashoffset={pending ? 0 : 320 * (1 - draw)}
            opacity={pending ? draw : 1}
          />
        );
      })}
      {/* arrowheads at each target */}
      {FLOW_EDGES.map(([a, b], i) => {
        const { x2, y2 } = edgePath(a, b);
        const pending = a === 'approval';
        const col = pending ? C.approval : brand.cyan;
        return (
          <path
            key={`h${i}`}
            d={`M${x2 - 8},${y2 - 5} L${x2},${y2} L${x2 - 8},${y2 + 5}`}
            fill="none"
            stroke={col}
            strokeWidth={2.2}
            opacity={draw}
          />
        );
      })}
      {/* the animated traveling dot on the active first edge */}
      <circle cx={dotX} cy={dotY} r={5} fill={brand.cyan} opacity={draw} />
      <circle cx={dotX} cy={dotY} r={10} fill="none" stroke={brand.cyan} strokeWidth={1.5} opacity={draw * 0.4} />
    </svg>
  );
};

// the left palette of node types you drag onto the canvas
const PALETTE: { label: string; icon: string; color: string }[] = [
  { label: 'product_analyze', icon: 'note', color: C.product },
  { label: 'product_rewrite', icon: 'edit', color: C.product },
  { label: 'product_plan', icon: 'split', color: C.product },
  { label: 'review_run', icon: 'eye', color: C.review },
  { label: 'swarm_task', icon: 'grid', color: C.swarm },
  { label: 'api_run', icon: 'send', color: C.api },
  { label: 'db_query', icon: 'db', color: C.db },
  { label: 'broker_peek', icon: 'box', color: '#0a84ff' },
  { label: 'channel_notify', icon: 'slack', color: C.notify },
  { label: 'budget_gate', icon: 'gauge', color: '#ff8a65' },
  { label: 'human_approval', icon: 'check', color: C.approval },
];

const Palette: React.FC = () => (
  <div
    style={{
      width: 230,
      flexShrink: 0,
      background: T.bgSidebar,
      borderRight: `1px solid ${T.border}`,
      display: 'flex',
      flexDirection: 'column',
      padding: '13px 11px',
      gap: 7,
    }}
  >
    <div style={{ display: 'flex', alignItems: 'center', gap: 7, padding: '0 4px 4px' }}>
      <Icon name="grid" size={14} color={brand.cyan} />
      <span style={{ fontFamily: fonts.ui, fontSize: 11.5, fontWeight: 700, letterSpacing: 0.4, textTransform: 'uppercase', color: T.textDim }}>
        Node types
      </span>
    </div>
    <Stagger delay={14} step={3.5} y={8} style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
      {PALETTE.map((p) => (
        <div
          key={p.label}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 9,
            height: 32,
            padding: '0 9px',
            borderRadius: 7,
            background: T.surface,
            border: `1px solid ${T.border}`,
          }}
        >
          <span style={{ width: 22, height: 22, borderRadius: 6, flexShrink: 0, background: alpha(p.color, 0.16), color: p.color, display: 'grid', placeItems: 'center' }}>
            <Icon name={p.icon} size={13} />
          </span>
          <span style={{ flex: 1, fontFamily: mono, fontSize: 12, color: T.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {p.label}
          </span>
        </div>
      ))}
    </Stagger>
  </div>
);

const CanvasScene: React.FC = () => {
  const CW = 1086; // canvas inner width
  const CH = 380;
  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow
          nav={<Navigator active="workflows" counts={{ workflows: 3 }} />}
          tabs={[
            { label: 'release-guard', icon: 'split', active: true },
            { label: 'Runs', icon: 'clock' },
            { label: 'Triggers', icon: 'zap' },
          ]}
          title="Otto — Workflows · release-guard"
        >
          <div style={{ display: 'flex', height: '100%' }}>
            <Palette />
            <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', padding: '14px 18px 18px' }}>
              {/* canvas toolbar */}
              <Appear delay={4} y={8}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 11, marginBottom: 10 }}>
                  <Icon name="split" size={16} color={brand.cyan} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 750 as never, color: T.text }}>release-guard</span>
                  <Chip color={C.trigger}>
                    <Icon name="zap" size={11} color={C.trigger} /> webhook
                  </Chip>
                  <div style={{ flex: 1 }} />
                  <Chip tone="default">4 nodes</Chip>
                  <Button variant="primary" size="s" icon="play">Run</Button>
                </div>
              </Appear>
              {/* the canvas itself */}
              <div
                style={{
                  position: 'relative',
                  flex: 1,
                  borderRadius: 12,
                  background:
                    `radial-gradient(${alpha('#ffffff', 0.05)} 1px, transparent 1px)`,
                  backgroundSize: '22px 22px',
                  border: `1px solid ${T.border}`,
                  overflow: 'hidden',
                }}
              >
                <div style={{ position: 'absolute', left: 0, top: 0, width: CW, height: CH }}>
                  <CanvasEdges w={CW} h={CH} />
                  {FLOW_NODES.map((n, i) => (
                    <CanvasNode key={n.id} n={n} delay={16 + i * 7} />
                  ))}
                </div>
              </div>
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={1}
        title="Drag typed nodes onto a canvas"
        sub="product · review · swarm · db · broker · api · gates"
      />
    </>
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  Scene 3 — triggers panel + an approval gate paused on you
// ════════════════════════════════════════════════════════════════════════════

const TriggerRow: React.FC<{
  icon: string;
  color: string;
  title: string;
  detail: React.ReactNode;
  on?: boolean;
}> = ({ icon, color, title, detail, on = true }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 12,
      padding: '13px 14px',
      borderRadius: 10,
      background: T.surface,
      border: `1px solid ${on ? alpha(color, 0.36) : T.border}`,
    }}
  >
    <span style={{ width: 34, height: 34, borderRadius: 9, flexShrink: 0, background: alpha(color, 0.16), color, display: 'grid', placeItems: 'center' }}>
      <Icon name={icon} size={17} />
    </span>
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 700, color: T.text }}>{title}</div>
      <div style={{ fontFamily: mono, fontSize: 12, color: T.textDim, marginTop: 4 }}>{detail}</div>
    </div>
    {on && <StatusDot kind="working" size={8} />}
  </div>
);

const TriggersScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="workflows" counts={{ workflows: 3 }} />}
        tabs={[
          { label: 'release-guard', icon: 'split' },
          { label: 'Triggers', icon: 'zap', active: true },
        ]}
        title="Otto — Workflows · triggers"
      >
        <div style={{ display: 'flex', height: '100%', padding: 18, gap: 16, boxSizing: 'border-box' }}>
          {/* triggers column */}
          <div style={{ flex: 1.25, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 12 }}>
            <Appear delay={4} y={8}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <Icon name="zap" size={16} color={brand.cyan} />
                <span style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 750 as never, color: T.text }}>Triggers</span>
                <div style={{ flex: 1 }} />
                <Chip tone="ok">3 active</Chip>
              </div>
            </Appear>
            <Stagger delay={10} step={7} y={12} style={{ display: 'flex', flexDirection: 'column', gap: 11 }}>
              <TriggerRow
                icon="globe"
                color={C.trigger}
                title="Webhook"
                detail={
                  <span>
                    <span style={{ color: T.textDim }}>POST </span>
                    <span style={{ color: brand.cyan }}>/hooks/wf_8c1a…</span>
                    <span style={{ color: T.textDim }}> · public-by-token</span>
                  </span>
                }
              />
              <TriggerRow icon="clock" color="#0a84ff" title="Schedule" detail="cron · 0 9 * * 1  (Mon 09:00)" />
              <TriggerRow icon="zap" color={C.product} title="On event" detail="product.story.updated → run" />
              <TriggerRow icon="play" color={T.textDim} title="Manual" detail="Run from the UI · ⌘↵" on={false} />
            </Stagger>
          </div>
          {/* approval gate card */}
          <Appear delay={14} y={12} style={{ flex: 1, minWidth: 0 }}>
            <Card pad={0} style={{ background: T.bgSidebar, border: `1px solid ${alpha(C.approval, 0.5)}`, overflow: 'hidden', height: '100%' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '14px 16px', borderBottom: `1px solid ${T.border}` }}>
                <span style={{ width: 30, height: 30, borderRadius: 8, background: alpha(C.approval, 0.18), color: C.approval, display: 'grid', placeItems: 'center' }}>
                  <Icon name="check" size={16} />
                </span>
                <span style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 750 as never, color: T.text }}>human_approval</span>
                <div style={{ flex: 1 }} />
                <Chip color={C.approval}>
                  <StatusDot kind="needsYou" size={7} /> paused
                </Chip>
              </div>
              <div style={{ padding: '18px 16px', display: 'flex', flexDirection: 'column', gap: 14 }}>
                <div style={{ fontFamily: fonts.ui, fontSize: 20, fontWeight: 750 as never, color: T.text }}>Approve deploy?</div>
                <div style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim, lineHeight: 1.5 }}>
                  <span style={{ color: T.text, fontFamily: mono }}>sinatra-users-go</span> · review passed · 1 PR ready to ship to prod.
                </div>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 9,
                    padding: '10px 12px',
                    borderRadius: 9,
                    background: alpha(C.approval, 0.1),
                    border: `1px solid ${alpha(C.approval, 0.34)}`,
                  }}
                >
                  <StatusDot kind="needsYou" size={9} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 13, color: C.approval, fontWeight: 600 }}>
                    Paused — waiting on you
                  </span>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginTop: 2 }}>
                  <Button variant="primary" icon="check">Approve</Button>
                  <Button variant="danger" icon="x">Deny</Button>
                </div>
              </div>
            </Card>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Trigger by webhook, schedule or hand"
      sub="Runs pause at approval gates — resume or deny from the UI"
    />
  </>
);

// ════════════════════════════════════════════════════════════════════════════
//  Scene 4 — run inspector: per-node timeline with status, durations & logs
// ════════════════════════════════════════════════════════════════════════════

interface RunRow {
  node: string;
  type: string;
  icon: string;
  color: string;
  st: keyof typeof STATUS;
  stLabel: string;
  dur: string;
}

const RUN_ROWS: RunRow[] = [
  { node: 'on PR opened', type: 'webhook', icon: 'zap', color: C.trigger, st: 'exited', stLabel: 'done', dur: '0.1s' },
  { node: 'review_run', type: 'multi-agent review', icon: 'eye', color: C.review, st: 'exited', stLabel: 'done', dur: '48s' },
  { node: 'human_approval', type: 'approve deploy?', icon: 'check', color: C.approval, st: 'needsYou', stLabel: 'paused', dur: '— ' },
  { node: 'channel_notify', type: 'Slack #releases', icon: 'slack', color: C.notify, st: 'idle', stLabel: 'queued', dur: '—' },
];

const RunNodeRow: React.FC<{ r: RunRow; delay: number }> = ({ r, delay }) => {
  const frame = useCurrentFrame();
  const op = track(frame, [delay, delay + 8], [0, 1]);
  const x = track(frame, [delay, delay + 8], [-10, 0]);
  const running = r.st === 'needsYou';
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        padding: '11px 13px',
        borderRadius: 10,
        background: running ? alpha(r.color, 0.08) : T.surface,
        border: `1px solid ${running ? alpha(r.color, 0.45) : T.border}`,
        opacity: op,
        transform: `translateX(${x}px)`,
      }}
    >
      <span style={{ width: 30, height: 30, borderRadius: 8, flexShrink: 0, background: alpha(r.color, 0.16), color: r.color, display: 'grid', placeItems: 'center' }}>
        <Icon name={r.icon} size={15} />
      </span>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontFamily: mono, fontSize: 13.5, fontWeight: 700, color: T.text }}>{r.node}</div>
        <div style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim, marginTop: 3 }}>{r.type}</div>
      </div>
      <span style={{ fontFamily: mono, fontSize: 12, color: T.textDim, width: 44, textAlign: 'right' }}>{r.dur}</span>
      <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6, width: 86, justifyContent: 'flex-end' }}>
        {r.st === 'exited' ? (
          <Icon name="check" size={14} color={STATUS.working} />
        ) : (
          <StatusDot kind={r.st} size={8} pulse={running} />
        )}
        <span
          style={{
            fontFamily: fonts.ui,
            fontSize: 12,
            fontWeight: 600,
            color: r.st === 'exited' ? STATUS.working : r.st === 'needsYou' ? C.approval : T.textDim,
          }}
        >
          {r.stLabel}
        </span>
      </span>
    </div>
  );
};

const LOG_LINES: { text: string; color: string }[] = [
  { text: '12:04:02  review_run  ✓ 3 lenses · 0 blocking comments', color: STATUS.working },
  { text: '12:04:50  human_approval  ⏸ paused — awaiting operator', color: C.approval },
];

const InspectorScene: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <>
      <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="workflows" counts={{ workflows: 3 }} />}
        tabs={[
          { label: 'release-guard', icon: 'split' },
          { label: 'Runs', icon: 'clock', active: true, dot: 'needsYou' },
        ]}
        title="Otto — Workflows · run #312"
      >
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%', padding: 18, gap: 13, boxSizing: 'border-box' }}>
          {/* header: run id + saved views */}
          <Appear delay={4} y={8}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 11 }}>
              <Icon name="clock" size={16} color={brand.cyan} />
              <span style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 750 as never, color: T.text }}>Run #312</span>
              <Chip color={C.approval}>
                <StatusDot kind="needsYou" size={7} /> paused
              </Chip>
              <Chip tone="default">via webhook</Chip>
              <div style={{ flex: 1 }} />
              <Segmented options={['All runs', 'Paused', 'Failed', 'Mine']} active={1} />
            </div>
          </Appear>

          {/* per-node timeline */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 9 }}>
            {RUN_ROWS.map((r, i) => (
              <RunNodeRow key={r.node} r={r} delay={12 + i * 7} />
            ))}
          </div>

          {/* logs */}
          <Appear delay={44} y={10} style={{ flex: 1, minHeight: 0 }}>
            <div
              style={{
                height: '100%',
                background: T.termBg,
                borderRadius: 10,
                border: `1px solid ${T.border}`,
                padding: '12px 14px',
                display: 'flex',
                flexDirection: 'column',
                gap: 7,
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 2 }}>
                <Icon name="terminal" size={13} color={T.textDim} />
                <span style={{ fontFamily: fonts.ui, fontSize: 12, fontWeight: 600, color: T.textDim }}>Node logs</span>
              </div>
              {LOG_LINES.map((l, i) => {
                const op = track(frame, [48 + i * 8, 56 + i * 8], [0, 1]);
                return (
                  <div key={i} style={{ opacity: op, fontFamily: mono, fontSize: 12.5, color: l.color, whiteSpace: 'pre-wrap' }}>
                    {l.text}
                  </div>
                );
              })}
            </div>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption step={3} title="Inspect every run" sub="Per-node results & logs · saved views" />
    </>
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  Scene 5 — outro
// ════════════════════════════════════════════════════════════════════════════
const Outro: React.FC = () => (
  <WalkOutro
    title="Workflows"
    tagline="Automation, with a human in the loop."
    pills={[
      { label: 'Typed nodes', color: '#9ee039', icon: 'split' },
      { label: 'Webhook/Cron', color: brand.cyan, icon: 'clock' },
      { label: 'Approval gates', color: '#febc2e', icon: 'check' },
      { label: 'Run inspector', color: '#0a84ff', icon: 'eye' },
      { label: 'Event triggers', color: brand.violet, icon: 'zap' },
    ]}
  />
);

// ── compose ──────────────────────────────────────────────────────────────────
const SCENES: SceneDef[] = [
  { dur: 80, node: <TitleScene />, name: 'Title' },
  { dur: 240, node: <CanvasScene />, name: 'Canvas' },
  { dur: 200, node: <TriggersScene />, name: 'Triggers' },
  { dur: 190, node: <InspectorScene />, name: 'Inspector' },
  { dur: 130, node: <Outro />, name: 'Outro' },
];

export const workflowsDuration = scenesDuration(SCENES);
export const Workflows: React.FC = () => <Scenes scenes={SCENES} />;
