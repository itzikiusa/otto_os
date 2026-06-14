import React from 'react';
import {
  AbsoluteFill,
  Sequence,
  useCurrentFrame,
  useVideoConfig,
  interpolate,
  spring,
  staticFile,
  Img,
} from 'remotion';
import { theme } from '../theme';
import { OttoWindow } from '../components/OttoWindow';
import { Navigator } from '../components/Navigator';
import { Appear, Caption, KeyCap, Shortcut, TitleCard } from '../components/ui';

// ─── Timing constants (all in frames @ 30fps) ───────────────────────────────
const T = {
  // Scene 0: Title card  0–75
  TITLE_IN: 0,
  TITLE_DUR: 75,

  // Scene 1: ⌘K palette  75–240
  CMD_K_START: 75,
  CMD_K_PRESS: 80,
  PALETTE_OPEN: 90,
  TYPE_START: 105,
  TYPE_END: 150,
  HIGHLIGHT_RESULT: 155,
  SESSION_SPAWN: 178,
  SCENE1_END: 240,

  // Scene 2: ⌘I Ask Otto  240–420
  CMD_I_START: 240,
  CMD_I_PRESS: 248,
  PROMPT_OPEN: 258,
  PROMPT_TYPE_START: 270,
  PROMPT_TYPE_END: 330,
  PLAN_APPEAR: 340,
  SESSIONS_SPAWN: 360,
  SCENE2_END: 420,

  // Scene 3: ⌘F find-in-page  420–570
  CMD_F_START: 420,
  CMD_F_PRESS: 428,
  FIND_OPEN: 438,
  FIND_TYPE_START: 448,
  FIND_TYPE_END: 478,
  MATCHES_APPEAR: 485,
  SCENE3_END: 570,

  // Scene 4: Shortcut grid  570–720
  GRID_START: 570,
  SCENE4_END: 720,

  // Scene 5: Outro  720–960
  OUTRO_START: 720,
};

// ─── Typewriter helper ───────────────────────────────────────────────────────
function useTypewriter(text: string, startFrame: number, endFrame: number): string {
  const frame = useCurrentFrame();
  if (frame < startFrame) return '';
  if (frame >= endFrame) return text;
  const progress = (frame - startFrame) / (endFrame - startFrame);
  return text.slice(0, Math.floor(progress * text.length));
}

// ─── Keycap "press" animation ─────────────────────────────────────────────────
const PressedKeyCap: React.FC<{ children: React.ReactNode; pressAt: number; wide?: boolean }> = ({
  children,
  pressAt,
  wide,
}) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const elapsed = frame - pressAt;
  const press = elapsed >= 0 && elapsed < 20
    ? spring({ frame: elapsed, fps, durationInFrames: 20, config: { damping: 120, stiffness: 300 } })
    : elapsed >= 20
    ? spring({ frame: elapsed - 20, fps, durationInFrames: 20, config: { damping: 120, stiffness: 300 } })
    : 0;
  // dip down on press, bounce back
  const scaleVal = elapsed >= 0 && elapsed < 20
    ? interpolate(press, [0, 1], [1, 0.88])
    : interpolate(press, [0, 1], [0.88, 1]);
  const translateY = elapsed >= 0 && elapsed < 20
    ? interpolate(press, [0, 1], [0, 4])
    : interpolate(press, [0, 1], [4, 0]);

  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        minWidth: wide ? 92 : 56,
        height: 56,
        padding: '0 14px',
        borderRadius: 12,
        background: elapsed >= 0 && elapsed < 40
          ? 'linear-gradient(180deg,#3d5bff44,#1a212b)'
          : 'linear-gradient(180deg,#2a3340,#1a212b)',
        border: `1px solid ${elapsed >= 0 && elapsed < 40 ? theme.accent : theme.border}`,
        boxShadow: elapsed >= 0 && elapsed < 40
          ? `0 2px 0 #0c1014, 0 4px 20px ${theme.accent}66`
          : '0 4px 0 #0c1014, 0 8px 24px rgba(0,0,0,0.4)',
        color: theme.text,
        fontFamily: theme.font,
        fontSize: 30,
        fontWeight: 700,
        transform: `scale(${scaleVal}) translateY(${translateY}px)`,
        transition: 'background 0.1s',
      }}
    >
      {children}
    </span>
  );
};

// ─── Animated shortcut with pressed keys ─────────────────────────────────────
const PressedShortcut: React.FC<{ keys: string[]; pressAt: number; delay?: number }> = ({
  keys,
  pressAt,
  delay = 0,
}) => (
  <Appear delay={delay} y={14}>
    <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
      {keys.map((k, i) => (
        <React.Fragment key={i}>
          {i > 0 && <span style={{ color: theme.textDim, fontSize: 28 }}>+</span>}
          <PressedKeyCap wide={k.length > 1} pressAt={pressAt}>
            {k}
          </PressedKeyCap>
        </React.Fragment>
      ))}
    </div>
  </Appear>
);

// ─── Command Palette overlay ──────────────────────────────────────────────────
const PALETTE_ITEMS = [
  { label: 'New Session', shortcut: '⌘T', icon: '+' },
  { label: 'Go to Git', shortcut: '⌘⇧G', icon: '⎇' },
  { label: 'Toggle Sidebar', shortcut: '⌘1', icon: '◫' },
  { label: 'Split View', shortcut: '⌘D', icon: '⊞' },
  { label: 'Switch Theme…', shortcut: '', icon: '◑' },
  { label: 'Open Connections', shortcut: '⌘⇧C', icon: '🔌' },
];

const CommandPalette: React.FC<{ openAt: number; typed: string }> = ({ openAt, typed }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const s = spring({ frame: frame - openAt, fps, config: { damping: 180, stiffness: 220 } });
  const opacity = interpolate(s, [0, 1], [0, 1]);
  const scale = interpolate(s, [0, 1], [0.92, 1]);

  const query = typed.toLowerCase();
  const filtered = query
    ? PALETTE_ITEMS.filter((it) => it.label.toLowerCase().includes(query))
    : PALETTE_ITEMS;

  if (frame < openAt) return null;

  return (
    <div
      style={{
        position: 'absolute',
        inset: 0,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 20,
        background: 'rgba(5,8,14,0.65)',
        backdropFilter: 'blur(12px)',
        opacity,
      }}
    >
      <div
        style={{
          width: 640,
          borderRadius: 16,
          background: '#0e1420',
          border: `1px solid ${theme.border}`,
          boxShadow: `0 40px 120px rgba(0,0,0,0.7), 0 0 0 1px rgba(61,91,255,0.18)`,
          overflow: 'hidden',
          transform: `scale(${scale})`,
        }}
      >
        {/* Search bar */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 12,
            padding: '16px 20px',
            borderBottom: `1px solid ${theme.border}`,
          }}
        >
          <span style={{ color: theme.textDim, fontSize: 20 }}>⌕</span>
          <span
            style={{
              flex: 1,
              color: theme.text,
              fontFamily: theme.mono,
              fontSize: 18,
              letterSpacing: 0.3,
            }}
          >
            {typed}
            <span
              style={{
                display: 'inline-block',
                width: 2,
                height: 20,
                background: theme.accent,
                marginLeft: 2,
                verticalAlign: 'middle',
                opacity: Math.floor(frame / 15) % 2 === 0 ? 1 : 0,
              }}
            />
          </span>
          <KeyCap wide>esc</KeyCap>
        </div>
        {/* Results */}
        <div style={{ padding: '8px 0' }}>
          {filtered.map((item, i) => {
            const isHighlighted = query.includes('new sess') || query === 'new session'
              ? item.label === 'New Session'
              : i === 0;
            return (
              <div
                key={item.label}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 12,
                  padding: '10px 20px',
                  background: isHighlighted ? `${theme.accent}22` : 'transparent',
                  borderLeft: isHighlighted ? `3px solid ${theme.accent}` : '3px solid transparent',
                }}
              >
                <span style={{ fontSize: 18, width: 24, textAlign: 'center', color: theme.textDim }}>
                  {item.icon}
                </span>
                <span
                  style={{
                    flex: 1,
                    color: isHighlighted ? theme.text : theme.textDim,
                    fontFamily: theme.font,
                    fontSize: 17,
                    fontWeight: isHighlighted ? 600 : 400,
                  }}
                >
                  {item.label}
                </span>
                {item.shortcut && (
                  <span
                    style={{
                      color: theme.textDim,
                      fontFamily: theme.mono,
                      fontSize: 14,
                      background: theme.surface2,
                      padding: '3px 8px',
                      borderRadius: 6,
                    }}
                  >
                    {item.shortcut}
                  </span>
                )}
              </div>
            );
          })}
        </div>
        <div
          style={{
            padding: '10px 20px',
            borderTop: `1px solid ${theme.border}`,
            display: 'flex',
            gap: 20,
            color: theme.textDim,
            fontFamily: theme.font,
            fontSize: 13,
          }}
        >
          <span>↵ Open</span>
          <span>↑↓ Navigate</span>
          <span>⌘K Command palette</span>
        </div>
      </div>
    </div>
  );
};

// ─── Ask Otto prompt overlay ──────────────────────────────────────────────────
const PLAN_CHIPS = ['Claude session ×2', 'Shell session ×1', 'Auto-layout: 3-panel'];

const AskOttoPrompt: React.FC<{
  openAt: number;
  typed: string;
  showPlan: boolean;
}> = ({ openAt, typed, showPlan }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const s = spring({ frame: frame - openAt, fps, config: { damping: 180, stiffness: 220 } });
  const opacity = interpolate(s, [0, 1], [0, 1]);
  const scale = interpolate(s, [0, 1], [0.94, 1]);

  if (frame < openAt) return null;

  return (
    <div
      style={{
        position: 'absolute',
        inset: 0,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 20,
        background: 'rgba(5,8,14,0.65)',
        backdropFilter: 'blur(12px)',
        opacity,
      }}
    >
      <div
        style={{
          width: 700,
          borderRadius: 18,
          background: '#0e1420',
          border: `1px solid ${theme.accent}44`,
          boxShadow: `0 40px 120px rgba(0,0,0,0.7), 0 0 0 1px ${theme.accent}22`,
          overflow: 'hidden',
          transform: `scale(${scale})`,
        }}
      >
        {/* Header */}
        <div
          style={{
            padding: '14px 22px',
            borderBottom: `1px solid ${theme.border}`,
            display: 'flex',
            alignItems: 'center',
            gap: 10,
          }}
        >
          <span
            style={{
              color: theme.accent,
              fontFamily: theme.font,
              fontSize: 15,
              fontWeight: 700,
              letterSpacing: 0.5,
            }}
          >
            ✦ Ask Otto
          </span>
          <span style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13, marginLeft: 8 }}>
            plain English → actions
          </span>
        </div>
        {/* Input */}
        <div style={{ padding: '18px 22px 14px' }}>
          <div
            style={{
              color: theme.text,
              fontFamily: theme.mono,
              fontSize: 18,
              lineHeight: 1.5,
              minHeight: 36,
            }}
          >
            {typed}
            {typed.length > 0 && (
              <span
                style={{
                  display: 'inline-block',
                  width: 2,
                  height: 20,
                  background: theme.accent2,
                  marginLeft: 2,
                  verticalAlign: 'middle',
                  opacity: Math.floor(frame / 15) % 2 === 0 ? 1 : 0,
                }}
              />
            )}
          </div>
        </div>
        {/* Parsed plan */}
        {showPlan && (
          <div
            style={{
              padding: '0 22px 18px',
              display: 'flex',
              flexDirection: 'column',
              gap: 8,
            }}
          >
            <div
              style={{
                color: theme.textDim,
                fontFamily: theme.font,
                fontSize: 12,
                fontWeight: 600,
                letterSpacing: 1,
                textTransform: 'uppercase',
                marginBottom: 4,
              }}
            >
              Deterministic parser → plan
            </div>
            <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap' }}>
              {PLAN_CHIPS.map((chip, i) => {
                const chipS = spring({
                  frame: frame - T.PLAN_APPEAR - i * 8,
                  fps,
                  config: { damping: 160 },
                });
                return (
                  <div
                    key={chip}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 8,
                      padding: '8px 16px',
                      borderRadius: 10,
                      background: `${theme.accent}22`,
                      border: `1px solid ${theme.accent}44`,
                      color: theme.accent2,
                      fontFamily: theme.mono,
                      fontSize: 15,
                      fontWeight: 600,
                      opacity: chipS,
                      transform: `translateY(${interpolate(chipS, [0, 1], [12, 0])}px)`,
                    }}
                  >
                    <span style={{ color: theme.accent }}>▸</span> {chip}
                  </div>
                );
              })}
            </div>
          </div>
        )}
        {/* Footer hint */}
        <div
          style={{
            padding: '10px 22px',
            borderTop: `1px solid ${theme.border}`,
            display: 'flex',
            justifyContent: 'space-between',
            color: theme.textDim,
            fontFamily: theme.font,
            fontSize: 13,
          }}
        >
          <span>↵ Execute plan</span>
          <span style={{ color: theme.accent2 }}>⌘I Ask Otto</span>
        </div>
      </div>
    </div>
  );
};

// ─── Find-in-page bar + highlighted content ───────────────────────────────────
const FindBar: React.FC<{ openAt: number; typed: string; matchCount: number; current: number }> = ({
  openAt,
  typed,
  matchCount,
  current,
}) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const s = spring({ frame: frame - openAt, fps, config: { damping: 200, stiffness: 280 } });
  if (frame < openAt) return null;

  return (
    <div
      style={{
        position: 'absolute',
        top: 12,
        right: 16,
        display: 'flex',
        alignItems: 'center',
        gap: 10,
        background: '#0e1420',
        border: `1px solid ${theme.border}`,
        borderRadius: 10,
        padding: '8px 14px',
        boxShadow: '0 8px 32px rgba(0,0,0,0.5)',
        zIndex: 15,
        transform: `translateY(${interpolate(s, [0, 1], [-32, 0])}px)`,
        opacity: s,
      }}
    >
      <span style={{ color: theme.textDim, fontSize: 16 }}>⌕</span>
      <span style={{ color: theme.text, fontFamily: theme.mono, fontSize: 16 }}>
        {typed}
        <span
          style={{
            display: 'inline-block',
            width: 2,
            height: 16,
            background: theme.accent2,
            marginLeft: 2,
            verticalAlign: 'middle',
            opacity: Math.floor(frame / 15) % 2 === 0 ? 1 : 0,
          }}
        />
      </span>
      {matchCount > 0 && (
        <span
          style={{
            color: theme.textDim,
            fontFamily: theme.font,
            fontSize: 14,
            padding: '2px 10px',
            background: theme.surface2,
            borderRadius: 6,
          }}
        >
          {current} / {matchCount}
        </span>
      )}
      <span style={{ fontSize: 18 }}>✕</span>
    </div>
  );
};

// ─── Fake editor content with highlighted matches ─────────────────────────────
const EditorContent: React.FC<{ highlight: boolean; query: string }> = ({ highlight, query }) => {
  const lines = [
    '// Otto ADE — keyboard-first development environment',
    'const session = await otto.newSession({ provider: "claude" });',
    'const shell   = await otto.newSession({ provider: "shell" });',
    '',
    '// Broadcast a command to all active sessions',
    'await otto.broadcast("git status");',
    '',
    'session.on("output", (data) => {',
    '  console.log("[claude]", data);',
    '});',
    '',
    'shell.on("output", (data) => {',
    '  console.log("[shell]", data);',
    '});',
  ];

  const renderLine = (line: string, lineIdx: number) => {
    if (!highlight || !query || !line.toLowerCase().includes(query.toLowerCase())) {
      return (
        <span key={lineIdx} style={{ color: lineIdx === 0 ? theme.textDim : theme.text }}>
          {line}
        </span>
      );
    }
    const idx = line.toLowerCase().indexOf(query.toLowerCase());
    const before = line.slice(0, idx);
    const match = line.slice(idx, idx + query.length);
    const after = line.slice(idx + query.length);
    const isFirst = lines.slice(0, lineIdx).every((l) => !l.toLowerCase().includes(query.toLowerCase()));
    return (
      <span key={lineIdx}>
        <span style={{ color: theme.text }}>{before}</span>
        <span
          style={{
            background: isFirst ? theme.accent2 : `${theme.warn}66`,
            color: isFirst ? '#000' : theme.text,
            borderRadius: 3,
            padding: '0 2px',
          }}
        >
          {match}
        </span>
        <span style={{ color: theme.text }}>{after}</span>
      </span>
    );
  };

  return (
    <div
      style={{
        padding: '32px 36px',
        fontFamily: theme.mono,
        fontSize: 18,
        lineHeight: 1.7,
        color: theme.text,
      }}
    >
      {lines.map((line, i) => (
        <div key={i}>
          {line === '' ? ' ' : renderLine(line, i)}
        </div>
      ))}
    </div>
  );
};

// ─── Shortcut grid item ───────────────────────────────────────────────────────
const GridItem: React.FC<{ keys: string[]; label: string; delay: number }> = ({ keys, label, delay }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const s = spring({ frame: frame - delay, fps, config: { damping: 160, stiffness: 160 } });
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: 14,
        opacity: s,
        transform: `translateY(${interpolate(s, [0, 1], [28, 0])}px) scale(${interpolate(s, [0, 1], [0.9, 1])})`,
      }}
    >
      <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
        {keys.map((k, i) => (
          <React.Fragment key={i}>
            {i > 0 && <span style={{ color: theme.textDim, fontSize: 22 }}>+</span>}
            <KeyCap wide={k.length > 1}>{k}</KeyCap>
          </React.Fragment>
        ))}
      </div>
      <span
        style={{
          color: theme.textDim,
          fontFamily: theme.font,
          fontSize: 18,
          fontWeight: 500,
        }}
      >
        {label}
      </span>
    </div>
  );
};

// ─── Session list state helpers ───────────────────────────────────────────────
function useSessionList(
  spawnAt: number,
  initial: { title: string; provider: string; status?: 'working' | 'idle' }[],
  extra: { title: string; provider: string; status?: 'working' | 'idle' }[]
) {
  const frame = useCurrentFrame();
  if (frame >= spawnAt) return [...initial, ...extra];
  return initial;
}

// ─── Main composition ─────────────────────────────────────────────────────────
export const Shortcuts: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  // ── Scene 1 typewriter ──────────────────────────────────────────────────────
  const paletteTyped = useTypewriter('new sess', T.TYPE_START, T.TYPE_END);
  const showHighlight = frame >= T.HIGHLIGHT_RESULT;

  // ── Scene 1 sessions ────────────────────────────────────────────────────────
  const s1Sessions = useSessionList(
    T.SESSION_SPAWN,
    [{ title: 'Claude #1', provider: 'claude', status: 'idle' }],
    [{ title: 'New Session', provider: 'claude', status: 'working' }]
  );

  // ── Scene 2 typewriter ──────────────────────────────────────────────────────
  const askOttoTyped = useTypewriter(
    'open 2 claude sessions and a shell',
    T.PROMPT_TYPE_START,
    T.PROMPT_TYPE_END
  );
  const showPlan = frame >= T.PLAN_APPEAR;

  // ── Scene 2 sessions ────────────────────────────────────────────────────────
  const s2Sessions = useSessionList(
    T.SESSIONS_SPAWN,
    [{ title: 'Claude #1', provider: 'claude', status: 'idle' }],
    [
      { title: 'Claude #2', provider: 'claude', status: 'working' },
      { title: 'Shell', provider: 'bash', status: 'working' },
    ]
  );

  // ── Scene 3 typewriter ──────────────────────────────────────────────────────
  const findTyped = useTypewriter('session', T.FIND_TYPE_START, T.FIND_TYPE_END);
  const showMatches = frame >= T.MATCHES_APPEAR;

  return (
    <AbsoluteFill
      style={{
        background: theme.bgGradient,
        fontFamily: theme.font,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      {/* ── Scene 0: Title ──────────────────────────────────────────────────── */}
      <Sequence from={T.TITLE_IN} durationInFrames={T.TITLE_DUR}>
        <AbsoluteFill>
          <TitleCard kicker="OTTO ADE" title="Shortcuts" subtitle="Command palette & keyboard-first flow" />
        </AbsoluteFill>
      </Sequence>

      {/* ── Scene 1: ⌘K Command Palette ─────────────────────────────────────── */}
      <Sequence from={T.CMD_K_START} durationInFrames={T.SCENE1_END - T.CMD_K_START}>
        <AbsoluteFill
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          {/* Window */}
          <Appear delay={0} y={32}>
            <OttoWindow
              title="Otto"
              sidebar={
                <Navigator
                  active="agents"
                  sessions={s1Sessions}
                />
              }
            >
              {/* Static editor content behind palette */}
              <EditorContent highlight={false} query="" />
              {/* Palette overlay */}
              <CommandPalette
                openAt={T.PALETTE_OPEN - T.CMD_K_START}
                typed={showHighlight ? 'new sess' : paletteTyped}
              />
            </OttoWindow>
          </Appear>

          {/* Floating shortcut badge */}
          <div
            style={{
              position: 'absolute',
              top: 60,
              left: 60,
              display: 'flex',
              alignItems: 'center',
              gap: 20,
            }}
          >
            <PressedShortcut keys={['⌘', 'K']} pressAt={T.CMD_K_PRESS - T.CMD_K_START} delay={0} />
            <Appear delay={8} y={8}>
              <span
                style={{
                  color: theme.textDim,
                  fontFamily: theme.font,
                  fontSize: 26,
                  fontWeight: 500,
                }}
              >
                Command Palette
              </span>
            </Appear>
          </div>

          <Caption
            step={1}
            title="⌘K opens the command palette"
            sub="Fuzzy search commands, sessions, themes — or type a plain command"
            delay={T.HIGHLIGHT_RESULT - T.CMD_K_START}
          />
        </AbsoluteFill>
      </Sequence>

      {/* ── Scene 2: ⌘I Ask Otto ─────────────────────────────────────────────── */}
      <Sequence from={T.CMD_I_START} durationInFrames={T.SCENE2_END - T.CMD_I_START}>
        <AbsoluteFill
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          <Appear delay={0} y={32}>
            <OttoWindow
              title="Otto"
              sidebar={
                <Navigator
                  active="agents"
                  sessions={s2Sessions}
                />
              }
            >
              <EditorContent highlight={false} query="" />
              <AskOttoPrompt
                openAt={T.PROMPT_OPEN - T.CMD_I_START}
                typed={askOttoTyped}
                showPlan={showPlan}
              />
            </OttoWindow>
          </Appear>

          <div
            style={{
              position: 'absolute',
              top: 60,
              left: 60,
              display: 'flex',
              alignItems: 'center',
              gap: 20,
            }}
          >
            <PressedShortcut keys={['⌘', 'I']} pressAt={T.CMD_I_PRESS - T.CMD_I_START} delay={0} />
            <Appear delay={8} y={8}>
              <span style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 26, fontWeight: 500 }}>
                Ask Otto
              </span>
            </Appear>
          </div>

          <Caption
            step={2}
            title="⌘I  Ask Otto in plain English"
            sub="Deterministic parser converts your intent into a concrete action plan"
            delay={T.PLAN_APPEAR - T.CMD_I_START}
          />
        </AbsoluteFill>
      </Sequence>

      {/* ── Scene 3: ⌘F Find-in-page ─────────────────────────────────────────── */}
      <Sequence from={T.CMD_F_START} durationInFrames={T.SCENE3_END - T.CMD_F_START}>
        <AbsoluteFill
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          <Appear delay={0} y={32}>
            <OttoWindow title="Otto">
              <EditorContent
                highlight={showMatches}
                query={findTyped}
              />
              <FindBar
                openAt={T.FIND_OPEN - T.CMD_F_START}
                typed={findTyped}
                matchCount={showMatches ? 27 : 0}
                current={3}
              />
            </OttoWindow>
          </Appear>

          <div
            style={{
              position: 'absolute',
              top: 60,
              left: 60,
              display: 'flex',
              alignItems: 'center',
              gap: 20,
            }}
          >
            <PressedShortcut keys={['⌘', 'F']} pressAt={T.CMD_F_PRESS - T.CMD_F_START} delay={0} />
            <Appear delay={8} y={8}>
              <span style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 26, fontWeight: 500 }}>
                Find in page
              </span>
            </Appear>
          </div>

          <Caption
            step={3}
            title="⌘F  Find & highlight across any session"
            sub="Jump between matches — 3 / 27 shown"
            delay={T.MATCHES_APPEAR - T.CMD_F_START}
          />
        </AbsoluteFill>
      </Sequence>

      {/* ── Scene 4: Shortcut grid ────────────────────────────────────────────── */}
      <Sequence from={T.GRID_START} durationInFrames={T.SCENE4_END - T.GRID_START}>
        <AbsoluteFill
          style={{
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 0,
            padding: '0 60px',
          }}
        >
          <Appear delay={0} y={20}>
            <div
              style={{
                color: theme.textDim,
                fontFamily: theme.font,
                fontSize: 22,
                fontWeight: 600,
                letterSpacing: 2,
                textTransform: 'uppercase',
                marginBottom: 48,
              }}
            >
              More shortcuts
            </div>
          </Appear>

          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(5, 1fr)',
              gap: '48px 80px',
              width: '100%',
              maxWidth: 1400,
            }}
          >
            <GridItem keys={['⌘', 'T']} label="New session" delay={8} />
            <GridItem keys={['⌘', 'J']} label="Right panel" delay={18} />
            <GridItem keys={['⌘', '1']} label="Sidebar" delay={28} />
            <GridItem keys={['⌘', 'D']} label="Split" delay={38} />
            <GridItem keys={['⌘', '⇧', 'B']} label="Broadcast" delay={48} />
          </div>

          <Caption
            step={4}
            title="Every action has a shortcut"
            sub="Navigate, split, broadcast — without touching the mouse"
            delay={50}
          />
        </AbsoluteFill>
      </Sequence>

      {/* ── Scene 5: Outro ────────────────────────────────────────────────────── */}
      <Sequence from={T.OUTRO_START} durationInFrames={960 - T.OUTRO_START}>
        <AbsoluteFill
          style={{
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 20,
          }}
        >
          <Appear delay={0}>
            <Img
              src={staticFile('otto-mark.png')}
              style={{
                width: 120,
                height: 120,
                borderRadius: 28,
                boxShadow: `0 30px 90px ${theme.accent}55`,
              }}
            />
          </Appear>
          <Appear delay={10}>
            <div
              style={{
                color: theme.text,
                fontFamily: theme.font,
                fontSize: 86,
                fontWeight: 800,
                letterSpacing: -2,
                textAlign: 'center',
                marginTop: 10,
              }}
            >
              Keyboard-first.
            </div>
          </Appear>
          <Appear delay={22}>
            <div
              style={{
                color: theme.accent2,
                fontFamily: theme.font,
                fontSize: 86,
                fontWeight: 800,
                letterSpacing: -2,
                textAlign: 'center',
              }}
            >
              Always.
            </div>
          </Appear>
          <Appear delay={38}>
            <div
              style={{
                color: theme.textDim,
                fontFamily: theme.mono,
                fontSize: 24,
                letterSpacing: 2,
                marginTop: 16,
              }}
            >
              otto-ade.com
            </div>
          </Appear>
        </AbsoluteFill>
      </Sequence>
    </AbsoluteFill>
  );
};
