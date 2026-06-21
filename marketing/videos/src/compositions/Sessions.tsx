import React from 'react';
import { useCurrentFrame, useVideoConfig, spring } from 'remotion';
import { T, brand, fonts, alpha, providers, status } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow, PhoneFrame } from '../components/Frame';
import { Navigator, NavSession } from '../components/Nav';
import {
  Appear,
  Caption,
  TitleCard,
  Terminal,
  TermLine,
  StatusDot,
  Chip,
  Button,
  Cursor,
  Caret,
  Icon,
  track,
  useTyped,
} from '../components/kit';

// ─────────────────────────────────────────────────────────────────────────────
//  Shared session model used across the desktop scenes.
// ─────────────────────────────────────────────────────────────────────────────
const SESSIONS: NavSession[] = [
  { title: 'fix auth tests', provider: 'claude', status: 'working', tasks: [2, 4] },
  { title: 'refactor api/v2', provider: 'codex', status: 'working', tasks: [1, 3] },
  { title: 'migrate db schema', provider: 'agy', status: 'working', tasks: [0, 2] },
  { title: 'tail prod logs', provider: 'shell', status: 'idle', tasks: [1, 1] },
];

const PROVIDER_COLOR: Record<string, string> = {
  claude: providers.claude,
  codex: providers.codex,
  agy: providers.agy,
  shell: providers.shell,
};

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 1 — Title card
// ════════════════════════════════════════════════════════════════════════════
const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Agent Sessions"
    title="Run agents, in parallel"
    subtitle="Claude Code · Codex · shell — watchable, resumable PTY sessions"
  />
);

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 2 — New-Session provider picker (modal sheet over the agents window)
// ════════════════════════════════════════════════════════════════════════════

interface ProviderRow {
  id: string;
  name: string;
  desc: string;
  color: string;
}

const PROVIDER_ROWS: ProviderRow[] = [
  { id: 'claude', name: 'Claude Code', desc: 'Anthropic · agentic coding CLI', color: providers.claude },
  { id: 'codex', name: 'Codex', desc: 'OpenAI · code generation agent', color: providers.codex },
  { id: 'agy', name: 'Antigravity', desc: 'Google · agentic IDE CLI', color: providers.agy },
  { id: 'shell', name: 'shell', desc: 'Plain PTY · your own tools', color: providers.shell },
];

const PickerRow: React.FC<{ row: ProviderRow; active: boolean; delay: number }> = ({ row, active, delay }) => {
  const frame = useCurrentFrame();
  const glow = active ? track(frame, [38, 52], [0, 1]) : 0;
  return (
    <Appear delay={delay} y={14} x={0}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 14,
          padding: '13px 15px',
          borderRadius: 12,
          background: active ? alpha(row.color, 0.1 + glow * 0.08) : T.surface,
          border: `1px solid ${active ? alpha(row.color, 0.45 + glow * 0.35) : T.border}`,
          boxShadow: active ? `0 0 0 ${1 + glow * 3}px ${alpha(row.color, 0.18 * glow)}, 0 10px 30px ${alpha(row.color, 0.18 * glow)}` : 'none',
        }}
      >
        <div
          style={{
            width: 40,
            height: 40,
            borderRadius: 10,
            flexShrink: 0,
            display: 'grid',
            placeItems: 'center',
            background: alpha(row.color, 0.16),
            border: `1px solid ${alpha(row.color, 0.4)}`,
            color: row.color,
          }}
        >
          <Icon name={row.id === 'shell' ? 'command' : 'terminal'} size={20} color={row.color} />
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontFamily: fonts.ui, fontSize: 17, fontWeight: 700, color: T.text }}>{row.name}</div>
          <div style={{ fontFamily: fonts.ui, fontSize: 13.5, color: T.textDim, marginTop: 2 }}>{row.desc}</div>
        </div>
        {active && <Chip color={row.color}>selected</Chip>}
      </div>
    </Appear>
  );
};

const PickerScene: React.FC = () => {
  const frame = useCurrentFrame();
  // Scrim + sheet fade-in.
  const scrim = track(frame, [6, 22], [0, 0.55]);
  const sheetY = track(frame, [8, 26], [40, 0]);
  const sheetOp = track(frame, [8, 24], [0, 1]);
  return (
    <>
      <Stage scale={0.9}>
        <div style={{ position: 'relative' }}>
          <OttoWindow
            nav={<Navigator active="agents" sessions={SESSIONS} activeSessionTitle="fix auth tests" workingCount={3} />}
            title="Otto — sinatra-users-go"
          >
            <div style={{ position: 'relative', width: '100%', height: '100%' }}>
              {/* dimmed underlying content */}
              <div style={{ padding: 18, opacity: 0.5 }}>
                <Terminal
                  lines={[
                    { text: '$ otto session new', tone: 'cmd' },
                    { text: '  press ⌘T to pick a provider…', tone: 'dim' },
                  ] as TermLine[]}
                  delay={0}
                  step={6}
                  pad={14}
                  fontSize={14}
                  style={{ background: 'transparent' }}
                />
              </div>
              {/* scrim */}
              <div style={{ position: 'absolute', inset: 0, background: alpha('#000', scrim) }} />
              {/* modal sheet */}
              <div
                style={{
                  position: 'absolute',
                  top: '50%',
                  left: '50%',
                  transform: `translate(-50%, calc(-50% + ${sheetY}px))`,
                  opacity: sheetOp,
                  width: 560,
                  background: T.surface,
                  border: `1px solid ${T.border}`,
                  borderRadius: 16,
                  boxShadow: `0 40px 100px rgba(0,0,0,0.65)`,
                  overflow: 'hidden',
                }}
              >
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    padding: '16px 18px 12px',
                    borderBottom: `1px solid ${T.border}`,
                  }}
                >
                  <Icon name="plus" size={18} color={T.accent} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 19, fontWeight: 750 as never, color: T.text }}>New Session</span>
                  <span style={{ flex: 1 }} />
                  <Chip>⌘T</Chip>
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 9, padding: 16 }}>
                  {PROVIDER_ROWS.map((r, i) => (
                    <PickerRow key={r.id} row={r} active={r.id === 'claude'} delay={14 + i * 5} />
                  ))}
                </div>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'flex-end',
                    gap: 10,
                    padding: '0 16px 16px',
                  }}
                >
                  <Button variant="ghost">Cancel</Button>
                  <Button variant="primary" icon="play">Start Session ⌘T</Button>
                </div>
              </div>
              {/* cursor moves onto the highlighted Claude row and clicks */}
              <Cursor from={[760, 120]} to={[560, 250]} startAt={34} duration={26} click />
            </div>
          </OttoWindow>
        </div>
      </Stage>
      <Caption step={1} title="New session — pick a provider" sub="⌘T · claude · codex · agy · shell" />
    </>
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 3 — Tiled grid of terminals (2×2), agents working in parallel
// ════════════════════════════════════════════════════════════════════════════

interface Tile {
  name: string;
  provider: string;
  working: boolean;
  tasks: [number, number];
  lines: TermLine[];
}

const TILES: Tile[] = [
  {
    name: 'fix auth tests',
    provider: 'claude',
    working: true,
    tasks: [2, 4],
    lines: [
      { text: '$ go test ./auth/...', tone: 'cmd' },
      { text: '  reading handler.go, jwt.go…', tone: 'dim' },
      { text: '  ✗ 3 failing — missing JWT exp check', tone: 'err' },
      { text: '  patch → middleware/jwt.go', tone: 'text' },
      { text: '  ✓ 142 passed (3.4s)', tone: 'ok' },
    ],
  },
  {
    name: 'refactor api/v2',
    provider: 'codex',
    working: true,
    tasks: [1, 3],
    lines: [
      { text: '$ codex run task.md', tone: 'cmd' },
      { text: '  editing server.go, routes.go…', tone: 'dim' },
      { text: '  go build ./... ', tone: 'text' },
      { text: '  ✓ build ok · 0 issues', tone: 'ok' },
      { text: '  drafting PR…', tone: 'accent' },
    ],
  },
  {
    name: 'migrate db schema',
    provider: 'agy',
    working: true,
    tasks: [0, 2],
    lines: [
      { text: '$ agy apply 0061_users.sql', tone: 'cmd' },
      { text: '  diffing live schema…', tone: 'dim' },
      { text: '  + add column last_seen_at', tone: 'text' },
      { text: '  running migration…', tone: 'warn' },
    ],
  },
  {
    name: 'tail prod logs',
    provider: 'shell',
    working: false,
    tasks: [1, 1],
    lines: [
      { text: '$ kubectl logs -f api-7f9', tone: 'cmd' },
      { text: '  200 GET /healthz', tone: 'dim' },
      { text: '  idle — suspended to save memory', tone: 'dim' },
    ],
  },
];

const TerminalTile: React.FC<{ tile: Tile; delay: number }> = ({ tile, delay }) => {
  const c = PROVIDER_COLOR[tile.provider];
  return (
    <Appear delay={delay} y={20} scale={0.97} style={{ minHeight: 0 }}>
      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          height: '100%',
          background: T.termBg,
          border: `1px solid ${tile.working ? alpha(c, 0.35) : T.border}`,
          borderRadius: 12,
          overflow: 'hidden',
          opacity: tile.working ? 1 : 0.62,
          boxShadow: tile.working ? `0 0 0 1px ${alpha(c, 0.12)}` : 'none',
        }}
      >
        {/* tile header */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            padding: '9px 12px',
            borderBottom: `1px solid ${T.border}`,
            background: alpha('#fff', 0.02),
          }}
        >
          <StatusDot kind={tile.working ? 'working' : 'idle'} size={9} />
          <span style={{ flex: 1, fontFamily: fonts.mono, fontSize: 13, fontWeight: 600, color: T.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {tile.name}
          </span>
          <span
            style={{
              fontFamily: fonts.ui,
              fontSize: 10.5,
              fontWeight: 700,
              color: tile.tasks[0] === tile.tasks[1] ? status.working : T.accent,
              background: alpha(tile.tasks[0] === tile.tasks[1] ? status.working : T.accent, 0.16),
              borderRadius: 999,
              padding: '2px 7px',
            }}
          >
            {tile.tasks[0]}/{tile.tasks[1]}
          </span>
          <Chip color={c}>{tile.provider}</Chip>
        </div>
        <Terminal
          lines={tile.lines}
          delay={delay + 8}
          step={9}
          pad={13}
          fontSize={13.5}
          style={{ flex: 1, background: 'transparent', borderRadius: 0 }}
        />
      </div>
    </Appear>
  );
};

const TiledScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="agents" sessions={SESSIONS} activeSessionTitle="fix auth tests" workingCount={3} />}
        tabs={[
          { label: 'fix auth tests', icon: 'terminal', active: true, dot: 'working' },
          { label: 'refactor api/v2', icon: 'terminal', dot: 'working' },
          { label: 'migrate db schema', icon: 'terminal', dot: 'working' },
          { label: 'tail prod logs', icon: 'terminal', dot: 'idle' },
        ]}
        title="Otto — sinatra-users-go · tiled"
      >
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: '1fr 1fr',
            gridTemplateRows: '1fr 1fr',
            gap: 14,
            padding: 16,
            height: '100%',
            boxSizing: 'border-box',
          }}
        >
          {TILES.map((tile, i) => (
            <TerminalTile key={tile.name} tile={tile} delay={10 + i * 8} />
          ))}
        </div>
      </OttoWindow>
    </Stage>
    <Caption step={2} title="Tiled view — every agent at once" sub="Status dots pulse while agents work · resumable on the daemon" />
  </>
);

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 4 — Broadcast (⌘⇧B): one literal message to every session
// ════════════════════════════════════════════════════════════════════════════

const BROADCAST_MSG = 'run the full test suite and report back';

const BroadcastTile: React.FC<{ tile: Tile; delay: number; flash: number; show: boolean }> = ({ tile, delay, flash, show }) => {
  const c = PROVIDER_COLOR[tile.provider];
  return (
    <Appear delay={delay} y={16} scale={0.97} style={{ minHeight: 0 }}>
      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          height: '100%',
          background: T.termBg,
          border: `1px solid ${alpha(brand.violet, 0.25 + flash * 0.5)}`,
          borderRadius: 12,
          overflow: 'hidden',
          boxShadow: `0 0 0 ${flash * 3}px ${alpha(brand.violet, 0.22 * flash)}, 0 0 ${flash * 40}px ${alpha(brand.violet, 0.3 * flash)}`,
        }}
      >
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            padding: '8px 12px',
            borderBottom: `1px solid ${T.border}`,
            background: alpha('#fff', 0.02),
          }}
        >
          <StatusDot kind="working" size={8} />
          <span style={{ flex: 1, fontFamily: fonts.mono, fontSize: 12.5, fontWeight: 600, color: T.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {tile.name}
          </span>
          <Chip color={c}>{tile.provider}</Chip>
        </div>
        <div style={{ flex: 1, padding: 13, fontFamily: fonts.mono, fontSize: 13, lineHeight: 1.7, color: T.textDim }}>
          {show && (
            <>
              <div style={{ display: 'flex', alignItems: 'center', gap: 7, color: brand.violet }}>
                <Icon name="send" size={12} color={brand.violet} />
                <span style={{ fontWeight: 600 }}>broadcast</span>
              </div>
              <div style={{ color: T.text, marginTop: 3 }}>$ {BROADCAST_MSG}</div>
              <div style={{ color: status.working, marginTop: 3 }}>✓ queued — running…</div>
            </>
          )}
        </div>
      </div>
    </Appear>
  );
};

const BroadcastScene: React.FC = () => {
  const frame = useCurrentFrame();
  const typed = useTyped(BROADCAST_MSG, 16, 24);
  const done = typed.length >= BROADCAST_MSG.length;
  // Flash pulse on send (~frame 78), fades back so tiles stay lit (alive) till end.
  const sendAt = 78;
  const flashRaw = track(frame, [sendAt, sendAt + 8], [0, 1]) * (1 - track(frame, [sendAt + 8, sendAt + 34], [0, 0.5]));
  const flash = Math.max(0, flashRaw);
  const showLines = frame > sendAt + 4;
  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow
          nav={<Navigator active="agents" sessions={SESSIONS} activeSessionTitle="fix auth tests" workingCount={3} />}
          title="Otto — broadcast to all sessions"
        >
          <div style={{ display: 'flex', flexDirection: 'column', height: '100%', boxSizing: 'border-box' }}>
            {/* BROADCAST bar */}
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 12,
                padding: '12px 16px',
                margin: 16,
                marginBottom: 0,
                borderRadius: 12,
                background: alpha(brand.violet, 0.1),
                border: `1px solid ${alpha(brand.violet, 0.45)}`,
                boxShadow: `0 10px 30px ${alpha(brand.violet, 0.16)}`,
              }}
            >
              <div
                style={{
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: 7,
                  padding: '5px 11px',
                  borderRadius: 8,
                  background: alpha(brand.violet, 0.2),
                  color: brand.violet,
                  fontFamily: fonts.ui,
                  fontSize: 12.5,
                  fontWeight: 700,
                  letterSpacing: 0.5,
                }}
              >
                <Icon name="send" size={14} color={brand.violet} />
                BROADCAST
              </div>
              <div
                style={{
                  flex: 1,
                  display: 'flex',
                  alignItems: 'center',
                  height: 38,
                  padding: '0 13px',
                  borderRadius: 9,
                  background: T.surface2,
                  border: `1px solid ${T.border}`,
                  fontFamily: fonts.mono,
                  fontSize: 15,
                  color: T.text,
                }}
              >
                <span>{typed}</span>
                {!done && <Caret color={brand.violet} h={17} />}
              </div>
              <span
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 12,
                  color: T.textDim,
                }}
              >
                4 sessions
              </span>
              <Button variant="primary" icon="send" style={{ background: brand.violet, boxShadow: `0 6px 18px ${alpha(brand.violet, 0.45)}` }}>
                Send ↵
              </Button>
            </div>
            {/* the 4 tiles receiving the broadcast */}
            <div
              style={{
                flex: 1,
                display: 'grid',
                gridTemplateColumns: '1fr 1fr',
                gridTemplateRows: '1fr 1fr',
                gap: 14,
                padding: 16,
                minHeight: 0,
                boxSizing: 'border-box',
              }}
            >
              {TILES.map((tile, i) => (
                <BroadcastTile key={tile.name} tile={tile} delay={8 + i * 4} flash={flash} show={showLines} />
              ))}
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption step={3} title="Broadcast ⌘⇧B" sub="One message, sent to every session at once" />
    </>
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 5 — Mobile beat: a single session on the phone, type-to-agent row
// ════════════════════════════════════════════════════════════════════════════

const PhoneSession: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const inputType = useTyped('add a regression test for the exp claim', 42, 22);
  const focus = spring({ frame: frame - 30, fps, config: { damping: 200 } });
  return (
    <PhoneFrame title="claude · fix auth" active="agents" workingBadge={2}>
      <div style={{ display: 'flex', flexDirection: 'column', height: '100%', boxSizing: 'border-box' }}>
        {/* session header */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            padding: '10px 14px',
            borderBottom: `1px solid ${T.border}`,
            background: T.bgSidebar,
          }}
        >
        <StatusDot kind="working" size={9} />
        <span style={{ flex: 1, fontFamily: fonts.mono, fontSize: 14, fontWeight: 600, color: T.text }}>fix auth tests</span>
        <Chip color={providers.claude}>claude</Chip>
      </div>
      {/* terminal body */}
      <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
        <Terminal
          lines={[
            { text: '$ go test ./auth/...', tone: 'cmd' },
            { text: '  ✗ exp claim not validated', tone: 'err' },
            { text: '  patch → middleware/jwt.go', tone: 'text' },
            { text: '  ✓ 142 passed (3.4s)', tone: 'ok' },
            { text: '  awaiting next instruction…', tone: 'dim' },
          ] as TermLine[]}
          delay={10}
          step={12}
          pad={14}
          fontSize={14}
          style={{ background: 'transparent', borderRadius: 0, height: '100%' }}
        />
      </div>
      {/* type-to-agent input row */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 9,
          padding: '10px 12px',
          borderTop: `1px solid ${T.border}`,
          background: T.bgSidebar,
        }}
      >
        <div
          style={{
            flex: 1,
            display: 'flex',
            alignItems: 'center',
            minHeight: 38,
            padding: '0 13px',
            borderRadius: 12,
            background: T.surface2,
            border: `1px solid ${focus > 0.5 ? T.accent : T.border}`,
            boxShadow: focus > 0.5 ? `0 0 0 3px ${alpha(T.accent, 0.22 * focus)}` : 'none',
            fontFamily: fonts.ui,
            fontSize: 14,
            color: inputType ? T.text : alpha(T.textDim, 0.8),
          }}
        >
          <span>{inputType || 'Type to your agent…'}</span>
          {inputType.length > 0 && inputType.length < 39 && <Caret color={T.accent} h={16} />}
        </div>
        <div
          style={{
            width: 40,
            height: 40,
            borderRadius: 12,
            flexShrink: 0,
            display: 'grid',
            placeItems: 'center',
            background: T.accent,
            color: '#fff',
            boxShadow: `0 6px 16px ${alpha(T.accent, 0.45)}`,
          }}
        >
            <Icon name="arrowUp" size={18} color="#fff" />
          </div>
        </div>
      </div>
    </PhoneFrame>
  );
};

const MobileScene: React.FC = () => (
  <>
    <Stage scale={0.92}>
      <PhoneSession />
    </Stage>
    <Caption
      step={4}
      title="…from your desk or your phone"
      sub="Fully responsive — touch terminal, per-device session isolation"
    />
  </>
);

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 6 — WalkOutro
// ════════════════════════════════════════════════════════════════════════════
const OutroScene: React.FC = () => (
  <WalkOutro
    title="Agent Sessions"
    tagline="Watch them work. Resume anytime."
    pills={[
      { label: 'Claude Code', color: providers.claude, icon: 'terminal' },
      { label: 'Codex', color: providers.codex, icon: 'terminal' },
      { label: 'shell', color: providers.shell, icon: 'command' },
      { label: 'Resumable', color: brand.cyan, icon: 'refresh' },
      { label: 'Broadcast', color: brand.violet, icon: 'send' },
    ]}
  />
);

// ════════════════════════════════════════════════════════════════════════════
//  COMPOSITION
// ════════════════════════════════════════════════════════════════════════════
const SCENES: SceneDef[] = [
  { dur: 80, node: <TitleScene />, name: 'Title' },
  { dur: 200, node: <PickerScene />, name: 'New Session' },
  { dur: 210, node: <TiledScene />, name: 'Tiled' },
  { dur: 200, node: <BroadcastScene />, name: 'Broadcast' },
  { dur: 140, node: <MobileScene />, name: 'Mobile' },
  { dur: 130, node: <OutroScene />, name: 'Outro' },
];

export const sessionsDuration: number = scenesDuration(SCENES);
export const Sessions: React.FC = () => <Scenes scenes={SCENES} />;
