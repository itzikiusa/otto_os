import React from 'react';
import {
  AbsoluteFill,
  Sequence,
  useCurrentFrame,
  useVideoConfig,
  interpolate,
  spring,
  random,
  staticFile,
  Img,
} from 'remotion';
import { theme } from '../theme';
import { OttoWindow } from '../components/OttoWindow';
import { Navigator } from '../components/Navigator';
import { Appear, Caption, KeyCap, Shortcut, TitleCard } from '../components/ui';

// ─── Timing constants (frames @ 30 fps) ────────────────────────────────────
const T = {
  titleStart: 0,
  titleEnd: 75,          // ~2.5s title card
  s1Start: 75,           // Scene 1: New Session sheet
  s1End: 225,            // ~5s
  s2Start: 225,          // Scene 2: Tiled grid
  s2End: 480,            // ~8.5s (long — lots of parallel animation)
  s3Start: 480,          // Scene 3: Maximize tile
  s3End: 660,            // ~6s
  s4Start: 660,          // Scene 4: Broadcast
  s4End: 870,            // ~7s
  s5Start: 870,          // Scene 5: Resumable
  s5End: 1110,           // ~8s
  s6Start: 1110,         // Scene 6: Outro
  s6End: 1260,
};

// ─── Terminal lines (deterministic via random()) ────────────────────────────
const LINES: Record<string, string[]> = {
  claude: [
    '$ go test ./...',
    '  Reading handlers.go…',
    '  Analyzing routes.go…',
    '  ✓ 142 passed (3.4s)',
    '$ git diff --stat',
    '  handlers.go | 12 ++---',
    '  routes.go   |  8 ++--',
    '  Refactoring complete.',
  ],
  codex: [
    '$ codex run task.md',
    '  Loading workspace…',
    '  Writing tests/api_test.go…',
    '  Editing server.go…',
    '  ✓ task complete',
    '$ go build ./...',
    '  Build successful.',
    '  No issues found.',
  ],
  agy: [
    '$ agy search "auth handler"',
    '  Found 4 matches in 3 files',
    '  auth/handler.go:22',
    '  middleware/jwt.go:11',
    '  Proposing refactor…',
    '  auth/handler.go updated',
    '  ✓ Changes applied',
    '  Running linter…',
  ],
  shell: [
    '$ make lint',
    '  golangci-lint run ./...',
    '  No issues found.',
    '$ make build',
    '  → go build -o dist/app',
    '  Build OK (1.2s)',
    '$ ./dist/app --version',
    '  v0.9.4-dev',
  ],
};

const PROVIDERS = [
  { id: 'claude', label: 'claude #1', chip: 'claude', color: '#d97757' },
  { id: 'codex',  label: 'codex #1',  chip: 'codex',  color: '#10b981' },
  { id: 'agy',    label: 'agy #1',    chip: 'agy',    color: '#8b5cf6' },
  { id: 'shell',  label: 'shell #1',  chip: 'shell',  color: '#3d5bff' },
];

// ─── TerminalTile ────────────────────────────────────────────────────────────
const TerminalTile: React.FC<{
  provider: typeof PROVIDERS[number];
  frame: number;
  fps: number;
  lineOffset?: number;   // which frame to start typing lines
  status?: 'working' | 'idle';
  broadcastLine?: string;
  showBroadcast?: boolean;
  style?: React.CSSProperties;
  scale?: number;
}> = ({
  provider,
  frame,
  fps,
  lineOffset = 0,
  status = 'working',
  broadcastLine,
  showBroadcast = false,
  style,
  scale = 1,
}) => {
  const lines = LINES[provider.id];

  // How many chars to reveal per line (typewriter)
  const charsPerLine = 32;
  const framesPerLine = 28;
  const totalChars = (f: number) => Math.max(0, f - lineOffset) / framesPerLine * charsPerLine;
  const tc = totalChars(frame);

  const visibleLines = lines.map((line, i) => {
    const start = i * charsPerLine;
    const revealed = Math.floor(tc - start);
    return revealed > 0 ? line.slice(0, Math.min(revealed, line.length)) : null;
  });

  // Pulsing dot for "working"
  const pulse = Math.sin(frame / 8) * 0.4 + 0.6;
  const dotOpacity = status === 'working' ? pulse : 0.35;

  // Idle dim
  const tileOpacity = status === 'idle' ? 0.45 : 1;

  return (
    <div
      style={{
        background: '#0d111a',
        borderRadius: 12,
        border: `1px solid ${status === 'idle' ? 'rgba(34,42,54,0.5)' : theme.border}`,
        overflow: 'hidden',
        display: 'flex',
        flexDirection: 'column',
        opacity: tileOpacity,
        transform: `scale(${scale})`,
        transformOrigin: 'center center',
        ...style,
      }}
    >
      {/* Header */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '8px 12px',
          borderBottom: `1px solid ${theme.border}`,
          background: 'rgba(255,255,255,0.02)',
          flexShrink: 0,
        }}
      >
        <div
          style={{
            width: 9,
            height: 9,
            borderRadius: '50%',
            background: status === 'working' ? theme.working : theme.idle,
            opacity: dotOpacity,
            boxShadow: status === 'working' ? `0 0 6px ${theme.working}` : 'none',
          }}
        />
        <span
          style={{
            color: theme.text,
            fontFamily: theme.mono,
            fontSize: 13,
            fontWeight: 600,
            flex: 1,
          }}
        >
          {provider.label}
        </span>
        <span
          style={{
            background: provider.color + '22',
            color: provider.color,
            border: `1px solid ${provider.color}55`,
            borderRadius: 6,
            padding: '2px 7px',
            fontFamily: theme.mono,
            fontSize: 11,
            fontWeight: 600,
            letterSpacing: 0.5,
          }}
        >
          {provider.chip}
        </span>
      </div>
      {/* Body */}
      <div
        style={{
          flex: 1,
          padding: '10px 14px',
          overflow: 'hidden',
          fontFamily: theme.mono,
          fontSize: 14,
          lineHeight: 1.7,
          color: theme.textDim,
        }}
      >
        {visibleLines.map((line, i) =>
          line ? (
            <div
              key={i}
              style={{
                color: line.startsWith('$') ? theme.accent2 : line.startsWith('  ✓') ? theme.working : theme.textDim,
                whiteSpace: 'pre',
              }}
            >
              {line}
            </div>
          ) : null,
        )}
        {showBroadcast && broadcastLine && (
          <div
            style={{
              marginTop: 4,
              color: '#facc15',
              fontFamily: theme.mono,
              fontSize: 14,
              borderLeft: '3px solid #facc15',
              paddingLeft: 8,
            }}
          >
            {broadcastLine}
          </div>
        )}
      </div>
    </div>
  );
};

// ─── Scene 1: New Session sheet ─────────────────────────────────────────────
const Scene1NewSession: React.FC<{ frame: number }> = ({ frame }) => {
  const { fps } = useVideoConfig();
  const localF = frame - T.s1Start;

  // Sheet slides up
  const sheetSpring = spring({ frame: localF - 10, fps, config: { damping: 180 } });
  const sheetY = interpolate(sheetSpring, [0, 1], [160, 0]);
  const sheetOp = interpolate(sheetSpring, [0, 1], [0, 1]);

  // Highlight each provider row in turn
  const selectedIdx = Math.floor(interpolate(localF, [40, 110], [0, 3], { extrapolateRight: 'clamp' }));

  // Tile appears at end
  const tileSpring = spring({ frame: localF - 115, fps, config: { damping: 160 } });
  const tileScale = interpolate(tileSpring, [0, 1], [0.6, 1]);
  const tileOp = interpolate(tileSpring, [0, 1], [0, 1]);

  const providers = [
    { label: 'claude', desc: 'Anthropic Claude', color: '#d97757', icon: '◆' },
    { label: 'codex',  desc: 'OpenAI Codex CLI', color: '#10b981', icon: '⬡' },
    { label: 'agy',    desc: 'Antigravity / agy', color: '#8b5cf6', icon: '⬢' },
    { label: 'shell',  desc: 'Plain shell (bash)', color: '#3d5bff', icon: '❯' },
  ];

  const sessions = [{ title: 'claude #1', provider: 'claude', status: 'working' as const }];

  return (
    <AbsoluteFill style={{ background: theme.bgGradient }}>
      <div
        style={{
          position: 'absolute',
          inset: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <div style={{ transform: 'scale(0.88)', transformOrigin: 'center center' }}>
          <OttoWindow sidebar={<Navigator active="agents" sessions={sessions} />}>
            {/* dim background content */}
            <div
              style={{
                position: 'absolute',
                inset: 0,
                background: 'rgba(0,0,0,0.55)',
                zIndex: 5,
                backdropFilter: 'blur(3px)',
              }}
            />
            {/* Sheet modal */}
            <div
              style={{
                position: 'absolute',
                top: '50%',
                left: '50%',
                transform: `translate(-50%, calc(-50% + ${sheetY}px))`,
                opacity: sheetOp,
                zIndex: 10,
                background: theme.surface2,
                border: `1px solid ${theme.border}`,
                borderRadius: 16,
                width: 520,
                padding: '28px 28px 20px',
                boxShadow: '0 40px 100px rgba(0,0,0,0.7)',
              }}
            >
              <div
                style={{
                  color: theme.text,
                  fontFamily: theme.font,
                  fontSize: 22,
                  fontWeight: 700,
                  marginBottom: 6,
                }}
              >
                New Session
              </div>
              <div
                style={{
                  color: theme.textDim,
                  fontFamily: theme.font,
                  fontSize: 15,
                  marginBottom: 20,
                }}
              >
                Choose a provider to start a new agent session
              </div>
              {providers.map((p, i) => (
                <div
                  key={p.label}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 14,
                    padding: '12px 14px',
                    borderRadius: 10,
                    marginBottom: 6,
                    background: i === selectedIdx ? p.color + '18' : 'transparent',
                    border: `1px solid ${i === selectedIdx ? p.color + '55' : 'transparent'}`,
                    transition: 'all 0.15s',
                  }}
                >
                  <span
                    style={{
                      width: 36,
                      height: 36,
                      borderRadius: 9,
                      background: p.color + '22',
                      color: p.color,
                      display: 'grid',
                      placeItems: 'center',
                      fontSize: 18,
                      fontWeight: 700,
                    }}
                  >
                    {p.icon}
                  </span>
                  <div style={{ flex: 1 }}>
                    <div
                      style={{
                        color: theme.text,
                        fontFamily: theme.mono,
                        fontSize: 15,
                        fontWeight: 600,
                      }}
                    >
                      {p.label}
                    </div>
                    <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>
                      {p.desc}
                    </div>
                  </div>
                  {i === selectedIdx && (
                    <span style={{ color: p.color, fontSize: 18 }}>→</span>
                  )}
                </div>
              ))}
              <div
                style={{
                  display: 'flex',
                  justifyContent: 'flex-end',
                  gap: 10,
                  marginTop: 16,
                  paddingTop: 16,
                  borderTop: `1px solid ${theme.border}`,
                }}
              >
                <div
                  style={{
                    padding: '8px 20px',
                    borderRadius: 9,
                    background: 'rgba(255,255,255,0.06)',
                    color: theme.textDim,
                    fontFamily: theme.font,
                    fontSize: 14,
                    fontWeight: 600,
                  }}
                >
                  Cancel
                </div>
                <div
                  style={{
                    padding: '8px 24px',
                    borderRadius: 9,
                    background: theme.accent,
                    color: '#fff',
                    fontFamily: theme.font,
                    fontSize: 14,
                    fontWeight: 700,
                    boxShadow: `0 6px 20px ${theme.accent}55`,
                  }}
                >
                  Start Session
                </div>
              </div>
            </div>
            {/* Spawned tile preview */}
            {tileOp > 0.05 && (
              <div
                style={{
                  position: 'absolute',
                  right: 40,
                  top: 40,
                  width: 280,
                  height: 160,
                  opacity: tileOp,
                  transform: `scale(${tileScale})`,
                  transformOrigin: 'top right',
                  zIndex: 4,
                }}
              >
                <TerminalTile
                  provider={PROVIDERS[0]}
                  frame={frame}
                  fps={fps}
                  lineOffset={localF > 115 ? T.s1Start + 115 : 9999}
                  status="working"
                  style={{ width: '100%', height: '100%' }}
                />
              </div>
            )}
          </OttoWindow>
        </div>
      </div>
      <Caption step={1} title="New Session  ⌘T" sub="Pick provider: claude · codex · agy · shell" delay={20} />
    </AbsoluteFill>
  );
};

// ─── Scene 2: Tiled grid ─────────────────────────────────────────────────────
const Scene2TiledGrid: React.FC<{ frame: number }> = ({ frame }) => {
  const { fps } = useVideoConfig();
  const localF = frame - T.s2Start;

  const sessions = PROVIDERS.map((p) => ({ title: p.label, provider: p.chip, status: 'working' as const }));

  return (
    <AbsoluteFill style={{ background: theme.bgGradient }}>
      <div
        style={{
          position: 'absolute',
          inset: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <div style={{ transform: 'scale(0.88)', transformOrigin: 'center center' }}>
          <OttoWindow sidebar={<Navigator active="agents" sessions={sessions} />}>
            {/* 2×2 grid */}
            <div
              style={{
                display: 'grid',
                gridTemplateColumns: '1fr 1fr',
                gridTemplateRows: '1fr 1fr',
                gap: 10,
                padding: 14,
                height: '100%',
                boxSizing: 'border-box',
              }}
            >
              {PROVIDERS.map((p, i) => {
                const tileSpring = spring({
                  frame: localF - i * 14,
                  fps,
                  config: { damping: 160 },
                });
                const tileOp = interpolate(tileSpring, [0, 1], [0, 1]);
                const tileY = interpolate(tileSpring, [0, 1], [30, 0]);
                return (
                  <div
                    key={p.id}
                    style={{
                      opacity: tileOp,
                      transform: `translateY(${tileY}px)`,
                      minHeight: 0,
                    }}
                  >
                    <TerminalTile
                      provider={p}
                      frame={frame}
                      fps={fps}
                      lineOffset={T.s2Start + i * 20}
                      status="working"
                      style={{ width: '100%', height: '100%' }}
                    />
                  </div>
                );
              })}
            </div>
          </OttoWindow>
        </div>
      </div>
      <Caption step={2} title="Tiled view — all agents in parallel" sub="Status dots pulse while agents are working" delay={20} />
    </AbsoluteFill>
  );
};

// ─── Scene 3: Maximize one tile ──────────────────────────────────────────────
const Scene3Maximize: React.FC<{ frame: number }> = ({ frame }) => {
  const { fps } = useVideoConfig();
  const localF = frame - T.s3Start;

  const duration = T.s3End - T.s3Start; // 180 frames

  // zoom in at 20, zoom back out at 100
  const zoomIn = spring({ frame: localF - 10, fps, config: { damping: 180 } });
  const zoomOut = spring({ frame: localF - 100, fps, config: { damping: 180 } });

  const zoomedScale = interpolate(zoomIn, [0, 1], [1, 1]) - interpolate(zoomOut, [0, 1], [0, 0]);
  const focusedScale = Math.max(0.88, interpolate(zoomIn, [0, 1], [0.88, 1.22]) - interpolate(zoomOut, [0, 1], [0, 0.34]));

  // overlay opacity
  const overlayOp = interpolate(zoomIn, [0, 1], [0, 0.8]) - interpolate(zoomOut, [0, 1], [0, 0.8]);

  const sessions = PROVIDERS.map((p, i) => ({
    title: p.label,
    provider: p.chip,
    status: i === 0 ? ('working' as const) : ('idle' as const),
  }));

  return (
    <AbsoluteFill style={{ background: theme.bgGradient }}>
      <div
        style={{
          position: 'absolute',
          inset: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <div style={{ transform: 'scale(0.88)', transformOrigin: 'center center' }}>
          <OttoWindow sidebar={<Navigator active="agents" sessions={sessions} />}>
            {/* dim overlay for unfocused tiles */}
            <div
              style={{
                position: 'absolute',
                inset: 0,
                background: `rgba(0,0,0,${Math.max(0, Math.min(0.8, overlayOp))})`,
                zIndex: 5,
                pointerEvents: 'none',
              }}
            />
            <div
              style={{
                display: 'grid',
                gridTemplateColumns: '1fr 1fr',
                gridTemplateRows: '1fr 1fr',
                gap: 10,
                padding: 14,
                height: '100%',
                boxSizing: 'border-box',
              }}
            >
              {PROVIDERS.map((p, i) => (
                <div key={p.id} style={{ minHeight: 0, position: 'relative', zIndex: i === 0 ? 10 : 1 }}>
                  <TerminalTile
                    provider={p}
                    frame={frame}
                    fps={fps}
                    lineOffset={T.s2Start + i * 20}
                    status={i === 0 ? 'working' : 'idle'}
                    style={{ width: '100%', height: '100%' }}
                    scale={i === 0 ? focusedScale : 1}
                  />
                </div>
              ))}
            </div>
          </OttoWindow>
        </div>
      </div>
      <Caption step={3} title="Zoom / maximize a single session" sub="Focus one agent without losing the others" delay={20} />
    </AbsoluteFill>
  );
};

// ─── Scene 4: Broadcast ──────────────────────────────────────────────────────
const Scene4Broadcast: React.FC<{ frame: number }> = ({ frame }) => {
  const { fps } = useVideoConfig();
  const localF = frame - T.s4Start;

  // Bar slides in
  const barSpring = spring({ frame: localF - 10, fps, config: { damping: 200 } });
  const barY = interpolate(barSpring, [0, 1], [-60, 0]);
  const barOp = barSpring;

  // Broadcast fires at frame 80
  const broadcastFired = localF > 80;
  const broadcastSpring = broadcastFired
    ? spring({ frame: localF - 80, fps, config: { damping: 160 } })
    : 0;

  // How much of the typed message to show
  const broadcastMsg = '⟡  run all tests and report back';
  const typedChars = Math.floor(interpolate(localF, [20, 75], [0, broadcastMsg.length], { extrapolateRight: 'clamp' }));
  const typedMsg = broadcastMsg.slice(0, typedChars);

  const sessions = PROVIDERS.map((p) => ({ title: p.label, provider: p.chip, status: 'working' as const }));

  return (
    <AbsoluteFill style={{ background: theme.bgGradient }}>
      <div
        style={{
          position: 'absolute',
          inset: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <div style={{ transform: 'scale(0.88)', transformOrigin: 'center center' }}>
          <OttoWindow sidebar={<Navigator active="agents" sessions={sessions} />}>
            {/* Broadcast bar */}
            <div
              style={{
                position: 'absolute',
                top: 0,
                left: 0,
                right: 0,
                zIndex: 20,
                opacity: barOp,
                transform: `translateY(${barY}px)`,
                background: 'rgba(61,91,255,0.12)',
                borderBottom: `2px solid ${theme.accent}`,
                padding: '10px 20px',
                display: 'flex',
                alignItems: 'center',
                gap: 14,
                backdropFilter: 'blur(6px)',
              }}
            >
              <span
                style={{
                  color: theme.accent,
                  fontFamily: theme.mono,
                  fontSize: 14,
                  fontWeight: 700,
                  letterSpacing: 1,
                  whiteSpace: 'nowrap',
                }}
              >
                BROADCAST
              </span>
              <div
                style={{
                  flex: 1,
                  background: 'rgba(255,255,255,0.05)',
                  borderRadius: 8,
                  padding: '8px 14px',
                  fontFamily: theme.mono,
                  fontSize: 16,
                  color: theme.text,
                  border: `1px solid ${theme.accent}44`,
                  minHeight: 36,
                }}
              >
                {typedMsg}
                {localF < 76 && (
                  <span
                    style={{
                      display: 'inline-block',
                      width: 2,
                      height: 16,
                      background: theme.accent,
                      marginLeft: 2,
                      verticalAlign: 'text-bottom',
                      opacity: Math.sin(localF / 5) > 0 ? 1 : 0,
                    }}
                  />
                )}
              </div>
              <div
                style={{
                  padding: '7px 18px',
                  borderRadius: 8,
                  background: theme.accent,
                  color: '#fff',
                  fontFamily: theme.font,
                  fontSize: 14,
                  fontWeight: 700,
                  boxShadow: `0 4px 16px ${theme.accent}55`,
                  opacity: broadcastFired ? 1 : 0.4,
                }}
              >
                Send ↵
              </div>
            </div>
            {/* Grid with broadcast lines */}
            <div
              style={{
                display: 'grid',
                gridTemplateColumns: '1fr 1fr',
                gridTemplateRows: '1fr 1fr',
                gap: 10,
                padding: 14,
                paddingTop: 68,
                height: '100%',
                boxSizing: 'border-box',
              }}
            >
              {PROVIDERS.map((p, i) => {
                const tileFlash = broadcastFired
                  ? spring({ frame: localF - 80 - i * 4, fps, config: { damping: 200 } })
                  : 0;
                const borderGlow = interpolate(tileFlash, [0, 0.4, 1], [0, 1, 0]);
                return (
                  <div
                    key={p.id}
                    style={{
                      minHeight: 0,
                      borderRadius: 12,
                      boxShadow: broadcastFired
                        ? `0 0 ${borderGlow * 28}px ${theme.accent}${Math.round(borderGlow * 200).toString(16).padStart(2, '0')}`
                        : 'none',
                    }}
                  >
                    <TerminalTile
                      provider={p}
                      frame={frame}
                      fps={fps}
                      lineOffset={T.s2Start + i * 20}
                      status="working"
                      broadcastLine={broadcastMsg}
                      showBroadcast={broadcastFired && localF > 80 + i * 4}
                      style={{ width: '100%', height: '100%' }}
                    />
                  </div>
                );
              })}
            </div>
          </OttoWindow>
        </div>
      </div>
      <Caption step={4} title="Broadcast ⌘⇧B" sub="Same message sent to all sessions at once" delay={20} />
    </AbsoluteFill>
  );
};

// ─── Scene 5: Resumable sessions ─────────────────────────────────────────────
const Scene5Resumable: React.FC<{ frame: number }> = ({ frame }) => {
  const { fps } = useVideoConfig();
  const localF = frame - T.s5Start;

  // Phase 1 (0–90): sessions go idle one by one
  // Phase 2 (90–180): session 0 reconnects / springs back
  const idlePhase = localF < 90;

  const getStatus = (i: number): 'working' | 'idle' => {
    const idleAt = 20 + i * 18;
    if (idlePhase && localF > idleAt) return 'idle';
    if (!idlePhase && i === 0) return 'working';
    if (!idlePhase && localF > 150 + i * 15 && i > 0) return 'working';
    return idlePhase ? 'idle' : 'idle';
  };

  // Resume glow for tile 0
  const resumeSpring = spring({ frame: localF - 95, fps, config: { damping: 160 } });
  const resumeGlow = interpolate(resumeSpring, [0, 1], [0, 1]);

  const sessions = PROVIDERS.map((p, i) => ({
    title: p.label,
    provider: p.chip,
    status: getStatus(i),
  }));

  // Daemon status badge
  const badgeOp = interpolate(localF, [30, 50], [0, 1], { extrapolateRight: 'clamp' });

  return (
    <AbsoluteFill style={{ background: theme.bgGradient }}>
      <div
        style={{
          position: 'absolute',
          inset: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <div style={{ transform: 'scale(0.88)', transformOrigin: 'center center' }}>
          <OttoWindow sidebar={<Navigator active="agents" sessions={sessions} />}>
            <div
              style={{
                display: 'grid',
                gridTemplateColumns: '1fr 1fr',
                gridTemplateRows: '1fr 1fr',
                gap: 10,
                padding: 14,
                height: '100%',
                boxSizing: 'border-box',
              }}
            >
              {PROVIDERS.map((p, i) => {
                const st = getStatus(i);
                const isResuming = !idlePhase && i === 0;
                return (
                  <div
                    key={p.id}
                    style={{
                      minHeight: 0,
                      borderRadius: 12,
                      boxShadow: isResuming
                        ? `0 0 ${resumeGlow * 24}px ${theme.working}66`
                        : 'none',
                      transition: 'box-shadow 0.3s',
                    }}
                  >
                    <TerminalTile
                      provider={p}
                      frame={frame}
                      fps={fps}
                      lineOffset={isResuming ? T.s5Start + 90 : T.s2Start + i * 20}
                      status={st}
                      style={{ width: '100%', height: '100%' }}
                    />
                  </div>
                );
              })}
            </div>
            {/* Daemon badge */}
            <div
              style={{
                position: 'absolute',
                top: 14,
                right: 14,
                opacity: badgeOp,
                background: 'rgba(13,17,26,0.95)',
                border: `1px solid ${theme.border}`,
                borderRadius: 10,
                padding: '8px 14px',
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                fontFamily: theme.mono,
                fontSize: 13,
                color: theme.textDim,
                zIndex: 10,
                boxShadow: '0 8px 30px rgba(0,0,0,0.5)',
              }}
            >
              <span
                style={{
                  width: 7,
                  height: 7,
                  borderRadius: '50%',
                  background: theme.working,
                  boxShadow: `0 0 6px ${theme.working}`,
                }}
              />
              otto daemon running
            </div>
          </OttoWindow>
        </div>
      </div>
      <Caption
        step={5}
        title="Resumable — persist on the daemon"
        sub="Close the window; agents keep state. Re-open to resume."
        delay={20}
      />
    </AbsoluteFill>
  );
};

// ─── Scene 6: Outro ──────────────────────────────────────────────────────────
const Scene6Outro: React.FC<{ frame: number }> = ({ frame }) => {
  const { fps } = useVideoConfig();
  const localF = frame - T.s6Start;

  const taglineSpring = spring({ frame: localF - 20, fps, config: { damping: 180 } });
  const taglineOp = taglineSpring;

  const subSpring = spring({ frame: localF - 40, fps, config: { damping: 180 } });
  const subOp = subSpring;

  const markSpring = spring({ frame: localF - 5, fps, config: { damping: 200 } });

  // Background dots for visual depth — deterministic
  const dots = Array.from({ length: 18 }, (_, i) => ({
    x: random(`dx${i}`) * 1920,
    y: random(`dy${i}`) * 1080,
    size: 3 + random(`ds${i}`) * 6,
    op: 0.04 + random(`do${i}`) * 0.1,
  }));

  return (
    <AbsoluteFill style={{ background: theme.bgGradient }}>
      {/* Decorative dots */}
      {dots.map((d, i) => (
        <div
          key={i}
          style={{
            position: 'absolute',
            left: d.x,
            top: d.y,
            width: d.size,
            height: d.size,
            borderRadius: '50%',
            background: theme.accent,
            opacity: d.op,
          }}
        />
      ))}

      <div
        style={{
          position: 'absolute',
          inset: 0,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          gap: 12,
        }}
      >
        <div
          style={{
            opacity: markSpring,
            transform: `scale(${interpolate(markSpring, [0, 1], [0.8, 1])})`,
          }}
        >
          <Img
            src={staticFile('otto-mark.png')}
            style={{
              width: 120,
              height: 120,
              borderRadius: 28,
              boxShadow: `0 24px 80px ${theme.accent}55`,
            }}
          />
        </div>

        <div
          style={{
            opacity: taglineOp,
            transform: `translateY(${interpolate(taglineOp, [0, 1], [20, 0])}px)`,
            marginTop: 20,
          }}
        >
          <div
            style={{
              fontFamily: theme.font,
              fontSize: 80,
              fontWeight: 800,
              color: theme.text,
              textAlign: 'center',
              lineHeight: 1.1,
              letterSpacing: -1,
            }}
          >
            Your agents,
            <br />
            <span
              style={{
                background: `linear-gradient(90deg, ${theme.accent}, ${theme.accent2})`,
                WebkitBackgroundClip: 'text',
                WebkitTextFillColor: 'transparent',
              }}
            >
              in parallel.
            </span>
          </div>
        </div>

        <div
          style={{
            opacity: subOp,
            transform: `translateY(${interpolate(subOp, [0, 1], [16, 0])}px)`,
            marginTop: 6,
          }}
        >
          <div
            style={{
              fontFamily: theme.font,
              fontSize: 28,
              color: theme.textDim,
              textAlign: 'center',
            }}
          >
            Otto ADE — Agentic Development Environment
          </div>
        </div>

        {/* Provider chips */}
        <div
          style={{
            display: 'flex',
            gap: 12,
            marginTop: 28,
            opacity: interpolate(localF, [55, 75], [0, 1], { extrapolateRight: 'clamp' }),
          }}
        >
          {PROVIDERS.map((p) => (
            <div
              key={p.id}
              style={{
                background: p.color + '18',
                color: p.color,
                border: `1px solid ${p.color}44`,
                borderRadius: 10,
                padding: '8px 18px',
                fontFamily: theme.mono,
                fontSize: 16,
                fontWeight: 600,
                letterSpacing: 0.5,
              }}
            >
              {p.chip}
            </div>
          ))}
        </div>
      </div>
    </AbsoluteFill>
  );
};

// ─── Root composition ────────────────────────────────────────────────────────
export const AgentMode: React.FC = () => {
  const frame = useCurrentFrame();

  return (
    <AbsoluteFill style={{ background: theme.bgGradient, fontFamily: theme.font }}>
      {/* Title card */}
      <Sequence from={T.titleStart} durationInFrames={T.titleEnd - T.titleStart + 20}>
        <AbsoluteFill style={{ background: theme.bgGradient }}>
          <TitleCard kicker="OTTO ADE" title="Agent Mode" subtitle="Many sessions at once" />
        </AbsoluteFill>
      </Sequence>

      {/* Scene 1: New Session */}
      <Sequence from={T.s1Start} durationInFrames={T.s1End - T.s1Start + 20}>
        <Scene1NewSession frame={frame} />
      </Sequence>

      {/* Scene 2: Tiled Grid */}
      <Sequence from={T.s2Start} durationInFrames={T.s2End - T.s2Start + 20}>
        <Scene2TiledGrid frame={frame} />
      </Sequence>

      {/* Scene 3: Maximize */}
      <Sequence from={T.s3Start} durationInFrames={T.s3End - T.s3Start + 20}>
        <Scene3Maximize frame={frame} />
      </Sequence>

      {/* Scene 4: Broadcast */}
      <Sequence from={T.s4Start} durationInFrames={T.s4End - T.s4Start + 20}>
        <Scene4Broadcast frame={frame} />
      </Sequence>

      {/* Scene 5: Resumable */}
      <Sequence from={T.s5Start} durationInFrames={T.s5End - T.s5Start + 20}>
        <Scene5Resumable frame={frame} />
      </Sequence>

      {/* Scene 6: Outro */}
      <Sequence from={T.s6Start} durationInFrames={T.s6End - T.s6Start}>
        <Scene6Outro frame={frame} />
      </Sequence>
    </AbsoluteFill>
  );
};
