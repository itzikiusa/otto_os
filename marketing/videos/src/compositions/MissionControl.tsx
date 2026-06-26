import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, alpha, providers, status as statusColors } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Stagger,
  TitleCard,
  Caption,
  Terminal,
  TermLine,
  StatusDot,
  Chip,
  Toast,
  Cursor,
  track,
} from '../components/kit';

// ── Shared: node kind palette ────────────────────────────────────────────────

type DotKind = 'working' | 'idle' | 'exited' | 'needsYou' | 'reconnectable';

const KIND_COLORS: Record<string, string> = {
  swarm: brand.cyan,
  session: providers.claude,
  loop: brand.violet,
  review: statusColors.working,
  scheduled: statusColors.needsYou,
  product: '#bf7aff',
  workflow: '#9ee039',
  channel: '#36c5f0',
};

// ── Work-graph node card ─────────────────────────────────────────────────────

const GraphNode: React.FC<{
  kind: string;
  label: string;
  dotKind?: DotKind;
  x: number;
  y: number;
  delay?: number;
  highlight?: boolean;
  highlightOp?: number;
}> = ({ kind, label, dotKind = 'working', x, y, delay = 0, highlight, highlightOp = 1 }) => {
  const color = KIND_COLORS[kind] ?? T.textDim;
  const active = highlight && highlightOp > 0;
  return (
    <Appear
      delay={delay}
      y={14}
      style={{ position: 'absolute', left: x, top: y, zIndex: active ? 10 : 1 }}
    >
      <div
        style={{
          width: 220,
          padding: '10px 14px',
          borderRadius: 10,
          background: active ? alpha(color, 0.18 * highlightOp) : T.surface,
          border: `1.5px solid ${active ? color : T.border}`,
          boxShadow: active
            ? `0 0 ${28 * highlightOp}px ${alpha(color, 0.55 * highlightOp)}, ${T.shadow}`
            : T.shadow,
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          transition: 'none',
        }}
      >
        <StatusDot kind={dotKind} size={9} pulse={dotKind === 'working'} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div
            style={{
              fontFamily: fonts.ui,
              fontSize: 12.5,
              fontWeight: 600,
              color: T.text,
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }}
          >
            {label}
          </div>
          <div style={{ fontFamily: fonts.ui, fontSize: 10.5, color: T.textDim, marginTop: 2 }}>
            {kind}
          </div>
        </div>
        <Chip color={color} style={{ fontSize: 10.5, flexShrink: 0 }}>
          {kind}
        </Chip>
      </div>
    </Appear>
  );
};

// ── Bezier connector lines between node right-edges and left-edges ───────────
//
//  Node geometry (inside the canvas, position: relative):
//    Swarm hub     x=80,  y=262  → center-right=(300,290), w=220, h≈56
//    Session 1     x=340, y=140  → left=(340,168)
//    Session 2     x=340, y=262  → left=(340,290)
//    Session 3     x=340, y=384  → left=(340,412)
//    Goal Loop     x=610, y=90   → left=(610,118)
//    Review        x=610, y=262  → left=(610,290)
//    Scheduled     x=610, y=422  → left=(610,450)

type EdgeCoord = [number, number, number, number]; // x1 y1 x2 y2

const EDGES: EdgeCoord[] = [
  [300, 290, 340, 168],  // hub → session 1
  [300, 290, 340, 290],  // hub → session 2
  [300, 290, 340, 412],  // hub → session 3
  [560, 168, 610, 118],  // session 1 → goal loop
  [560, 290, 610, 290],  // session 2 → review
  [560, 412, 610, 450],  // session 3 → scheduled
];

const GraphEdges: React.FC = () => (
  <svg
    style={{ position: 'absolute', inset: 0, pointerEvents: 'none', overflow: 'visible' }}
    width="100%"
    height="100%"
  >
    {EDGES.map(([x1, y1, x2, y2], i) => {
      const mx = (x1 + x2) / 2;
      return (
        <path
          key={i}
          d={`M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}`}
          fill="none"
          stroke={alpha('#ffffff', 0.13)}
          strokeWidth={1.5}
          strokeDasharray="5 4"
        />
      );
    })}
  </svg>
);

// ── Scene 1 — Title ──────────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Mission Control"
    title="Work Graph"
    subtitle="One live map over everything your agents are doing — sessions, swarms, loops, reviews, and more."
  />
);

// ── Scene 2 — The graph ──────────────────────────────────────────────────────

const GraphScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={<Navigator active="mission-control" workingCount={5} />}
        title="Otto — Mission Control"
        width={1560}
        height={884}
      >
        <div
          style={{
            height: '100%',
            padding: '16px 20px',
            boxSizing: 'border-box',
            display: 'flex',
            flexDirection: 'column',
            overflow: 'hidden',
          }}
        >
          {/* toolbar */}
          <Appear delay={4}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                marginBottom: 16,
                flexShrink: 0,
              }}
            >
              <span
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 14,
                  fontWeight: 700,
                  color: T.text,
                }}
              >
                Work Graph
              </span>
              <Chip tone="ok">5 active</Chip>
              <Chip>8 kinds</Chip>
              <div style={{ flex: 1 }} />
              <Chip style={{ fontSize: 11 }}>live</Chip>
            </div>
          </Appear>

          {/* canvas */}
          <div style={{ position: 'relative', flex: 1, minHeight: 0 }}>
            <GraphEdges />

            {/* col 1 — swarm hub */}
            <GraphNode kind="swarm"  label="Payments Swarm"       dotKind="working"   x={80}  y={240} delay={10} />

            {/* col 2 — sessions spawned by swarm */}
            <GraphNode kind="session" label="fix-payments-handler" dotKind="working"   x={340} y={118} delay={18} />
            <GraphNode kind="session" label="add-retry-logic"      dotKind="working"   x={340} y={240} delay={22} />
            <GraphNode kind="session" label="write-pmt-tests"      dotKind="idle"      x={340} y={362} delay={26} />

            {/* col 3 — downstream work */}
            <GraphNode kind="loop"      label="Goal Loop · flaky-e2e"    dotKind="working"   x={610} y={68}  delay={30} />
            <GraphNode kind="review"    label="Review · PR #482"          dotKind="needsYou"  x={610} y={240} delay={34} />
            <GraphNode kind="scheduled" label="Scheduled · daily-digest"  dotKind="idle"      x={610} y={400} delay={38} />
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="One graph over everything your agents are doing"
      sub="Each unit of work is a node — status dot + kind chip. Edges show spawn and parent relationships."
    />
  </>
);

// ── Scene 3 — Kinds + live activity feed ─────────────────────────────────────

const KINDS = [
  { label: 'Sessions',   color: providers.claude },
  { label: 'Swarm',      color: brand.cyan },
  { label: 'Goal Loops', color: brand.violet },
  { label: 'Reviews',    color: statusColors.working },
  { label: 'Product',    color: '#bf7aff' },
  { label: 'Scheduled',  color: statusColors.needsYou },
  { label: 'Channels',   color: '#36c5f0' },
  { label: 'Workflows',  color: '#9ee039' },
];

const FEED_LINES: TermLine[] = [
  { text: '09:14:02  swarm     Hopper finished task #12 · fix-payments-handler',   tone: 'ok' },
  { text: '09:14:07  review    3 new findings on PR #482 · auth/middleware.go',     tone: 'text' },
  { text: '09:14:19  loop      flaky-e2e  iteration 4/8  →  evaluate',              tone: 'accent' },
  { text: '09:14:31  session   add-retry-logic  needsYou  "approve db migration?"', tone: 'warn' },
  { text: '09:14:44  sched     daily-digest  starting  →  agents/reporter.ts',      tone: 'dim' },
  { text: '09:14:52  swarm     Coordinator spawning  write-pmt-tests',              tone: 'ok' },
  { text: '09:15:01  loop      flaky-e2e  iteration 5/8  →  execute',              tone: 'accent' },
  { text: '09:15:09  review    Finding resolved  auth token null-check  ✓',         tone: 'ok' },
];

const KindsFeedScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={<Navigator active="mission-control" workingCount={5} />}
        title="Otto — Mission Control"
        width={1560}
        height={884}
      >
        <div style={{ display: 'flex', height: '100%' }}>
          {/* left legend */}
          <div
            style={{
              width: 300,
              flexShrink: 0,
              borderRight: `1px solid ${T.border}`,
              padding: 20,
              display: 'flex',
              flexDirection: 'column',
              gap: 0,
            }}
          >
            <Appear delay={4}>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 11,
                  fontWeight: 600,
                  color: T.textDim,
                  textTransform: 'uppercase',
                  letterSpacing: 1,
                  marginBottom: 16,
                }}
              >
                Agentic kinds
              </div>
            </Appear>
            <Stagger delay={8} step={8} y={10}>
              {KINDS.map((k) => (
                <div
                  key={k.label}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 12,
                    padding: '7px 0',
                    borderBottom: `1px solid ${alpha(T.border, 0.5)}`,
                  }}
                >
                  <span
                    style={{
                      width: 10,
                      height: 10,
                      borderRadius: '50%',
                      background: k.color,
                      boxShadow: `0 0 8px ${alpha(k.color, 0.7)}`,
                      flexShrink: 0,
                    }}
                  />
                  <span
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 13,
                      fontWeight: 500,
                      color: T.text,
                    }}
                  >
                    {k.label}
                  </span>
                </div>
              ))}
            </Stagger>
          </div>

          {/* right activity feed */}
          <div
            style={{
              flex: 1,
              padding: 20,
              display: 'flex',
              flexDirection: 'column',
              minWidth: 0,
            }}
          >
            <Appear delay={4}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 10,
                  marginBottom: 14,
                  flexShrink: 0,
                }}
              >
                <span
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 11,
                    fontWeight: 600,
                    color: T.textDim,
                    textTransform: 'uppercase',
                    letterSpacing: 1,
                  }}
                >
                  Live activity
                </span>
                <StatusDot kind="working" size={7} pulse />
              </div>
            </Appear>
            <Terminal
              lines={FEED_LINES}
              delay={10}
              step={15}
              fontSize={13}
              pad={16}
              style={{ flex: 1 }}
            />
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Sessions, swarms, goal loops, reviews, product, scheduled — unified"
      sub="Every event from every agentic kind flows into one live feed. Nothing goes unnoticed."
    />
  </>
);

// ── Scene 4 — Drill-in: click a node to open the work behind it ──────────────
//
//  Cursor screen-space target calculation (scale=0.88, window 1560×884):
//    Window top-left  ≈ (274, 151)  [= (960-686, 540-389)]
//    Nav 248px, titlebar 44px → content origin ≈ (274+218, 151+39) = (492, 190)
//    Canvas padding 16px → canvas origin ≈ (492+14, 190+14) = (506, 204)
//    Header block ≈ 46px → graph origin ≈ (506, 204+40) = (506, 244)
//    Node "fix-payments-handler" at left=340, top=118 in graph:
//      screen ≈ (506+340×0.88, 244+118×0.88) = (506+299, 244+104) = (805, 348)
//    Node center (220/2=110, 56/2=28):
//      screen ≈ (805+110×0.88, 348+28×0.88) = (805+97, 348+25) = (902, 373)

const DrillScene: React.FC = () => {
  const frame = useCurrentFrame();
  // Glow pulses in as the cursor arrives (cursor arrives at frame 12+28=40)
  const highlightOp = track(frame, [38, 54], [0, 1]);

  return (
    <>
      <Stage scale={0.88}>
        <OttoWindow
          nav={<Navigator active="mission-control" workingCount={5} />}
          title="Otto — Mission Control"
          width={1560}
          height={884}
        >
          <div
            style={{
              height: '100%',
              padding: '16px 20px',
              boxSizing: 'border-box',
              display: 'flex',
              flexDirection: 'column',
              overflow: 'hidden',
            }}
          >
            <Appear delay={2}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 10,
                  marginBottom: 16,
                  flexShrink: 0,
                }}
              >
                <span style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 700, color: T.text }}>
                  Work Graph
                </span>
                <Chip tone="ok">5 active</Chip>
                <Chip>8 kinds</Chip>
              </div>
            </Appear>

            <div style={{ position: 'relative', flex: 1, minHeight: 0 }}>
              <GraphEdges />

              {/* swarm hub */}
              <GraphNode kind="swarm" label="Payments Swarm" dotKind="working" x={80} y={240} delay={3} />

              {/* session 1 — the click target, highlighted */}
              <GraphNode
                kind="session"
                label="fix-payments-handler"
                dotKind="working"
                x={340}
                y={118}
                delay={3}
                highlight
                highlightOp={highlightOp}
              />

              {/* remaining nodes — no highlight */}
              <GraphNode kind="session"   label="add-retry-logic"         dotKind="working"   x={340} y={240} delay={3} />
              <GraphNode kind="session"   label="write-pmt-tests"         dotKind="idle"      x={340} y={362} delay={3} />
              <GraphNode kind="loop"      label="Goal Loop · flaky-e2e"   dotKind="working"   x={610} y={68}  delay={3} />
              <GraphNode kind="review"    label="Review · PR #482"        dotKind="needsYou"  x={610} y={240} delay={3} />
              <GraphNode kind="scheduled" label="Scheduled · daily-digest" dotKind="idle"     x={610} y={400} delay={3} />
            </div>
          </div>
        </OttoWindow>
      </Stage>

      {/* Animated cursor travels from content area to the target node */}
      <Cursor from={[1280, 640]} to={[900, 370]} startAt={12} duration={28} click />

      {/* Toast appears after the click */}
      <Toast
        text="Opened: fix-payments-handler"
        tone="ok"
        delay={58}
        style={{ position: 'absolute', top: 72, right: 110 }}
      />

      <Caption
        step={3}
        title="Click any node to jump straight to the work behind it"
        sub="One click opens the live session, swarm run, review thread, or scheduled job — wherever it lives."
      />
    </>
  );
};

// ── Scene array + exports ────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 80,  node: <TitleScene />,   name: 'Title' },
  { dur: 200, node: <GraphScene />,   name: 'Graph' },
  { dur: 165, node: <KindsFeedScene />, name: 'KindsFeed' },
  { dur: 120, node: <DrillScene />,   name: 'DrillIn' },
  {
    dur: 130,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Mission Control"
        tagline="Every agent, every workstream — on one live map"
        pills={[
          { label: 'Unified work graph', icon: 'radar' },
          { label: 'All agentic kinds',  icon: 'grid' },
          { label: 'Live feed',          icon: 'bell' },
          { label: 'Click to open',      icon: 'external' },
        ]}
      />
    ),
  },
];

export const missionControlDuration = scenesDuration(SCENES);
export const MissionControl: React.FC = () => <Scenes scenes={SCENES} />;
