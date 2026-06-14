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
  Easing,
} from 'remotion';
import { theme, VIDEO } from '../theme';
import { OttoWindow } from '../components/OttoWindow';
import { Navigator } from '../components/Navigator';
import { Appear, KeyCap } from '../components/ui';

// ─── Scene 1: Logo + Tagline build (~0–90f, 3s) ────────────────────────────

const Scene1Logo: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const logoSpring = spring({ frame, fps, config: { damping: 18, stiffness: 120 } });
  const logoScale = interpolate(logoSpring, [0, 1], [0.55, 1]);
  const logoOpacity = interpolate(frame, [0, 15], [0, 1], { extrapolateRight: 'clamp' });

  const taglineProgress = spring({ frame: frame - 22, fps, config: { damping: 200 } });
  const taglineOpacity = interpolate(taglineProgress, [0, 1], [0, 1]);
  const taglineY = interpolate(taglineProgress, [0, 1], [28, 0]);

  const subProgress = spring({ frame: frame - 36, fps, config: { damping: 200 } });
  const subOpacity = interpolate(subProgress, [0, 1], [0, 1]);
  const subY = interpolate(subProgress, [0, 1], [20, 0]);

  return (
    <AbsoluteFill
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 0,
      }}
    >
      {/* Otto mark */}
      <div
        style={{
          opacity: logoOpacity,
          transform: `scale(${logoScale})`,
          marginBottom: 24,
        }}
      >
        <Img
          src={staticFile('otto-mark.png')}
          style={{
            width: 136,
            height: 136,
            borderRadius: 32,
            boxShadow: `0 0 0 1px ${theme.accent}44, 0 20px 80px ${theme.accent}66, 0 0 120px ${theme.accent}22`,
          }}
        />
      </div>

      {/* OTTO wordmark */}
      <div
        style={{
          opacity: taglineOpacity,
          transform: `translateY(${taglineY}px)`,
          fontFamily: theme.font,
          fontSize: 100,
          fontWeight: 900,
          color: theme.text,
          letterSpacing: -3,
          lineHeight: 1,
        }}
      >
        Otto
      </div>

      {/* Tagline */}
      <div
        style={{
          opacity: subOpacity,
          transform: `translateY(${subY}px)`,
          fontFamily: theme.font,
          fontSize: 30,
          fontWeight: 500,
          color: theme.textDim,
          letterSpacing: 0.5,
          marginTop: 16,
        }}
      >
        Run your AI agents{' '}
        <span style={{ color: theme.accent2, fontWeight: 700 }}>like a pro.</span>
      </div>

      {/* Accent glow line */}
      <div
        style={{
          position: 'absolute',
          bottom: '35%',
          left: '50%',
          transform: 'translateX(-50%)',
          width: interpolate(frame, [40, 90], [0, 400], { extrapolateRight: 'clamp' }),
          height: 1,
          background: `linear-gradient(90deg, transparent, ${theme.accent}88, transparent)`,
          opacity: interpolate(frame, [40, 60], [0, 1], { extrapolateRight: 'clamp' }),
        }}
      />
    </AbsoluteFill>
  );
};

// ─── Scene 2: OttoWindow glimpse + feature pills (~90–330f, ~8s) ────────────

const PILLS = [
  { label: 'Agent Mode', color: theme.accent },
  { label: 'Git & PRs', color: theme.accent2 },
  { label: 'Connections', color: '#bf7aff' },
  { label: 'Channels', color: '#ff9a3d' },
  { label: 'Review Agents', color: theme.accent },
];

const FeaturePill: React.FC<{ label: string; color: string; delay: number }> = ({
  label,
  color,
  delay,
}) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const s = spring({ frame: frame - delay, fps, config: { damping: 180, stiffness: 140 } });
  const opacity = interpolate(s, [0, 1], [0, 1]);
  const y = interpolate(s, [0, 1], [18, 0]);
  const scale = interpolate(s, [0, 1], [0.88, 1]);

  return (
    <div
      style={{
        opacity,
        transform: `translateY(${y}px) scale(${scale})`,
        display: 'inline-flex',
        alignItems: 'center',
        gap: 10,
        padding: '10px 22px',
        borderRadius: 40,
        background: `${color}18`,
        border: `1px solid ${color}55`,
        boxShadow: `0 6px 28px ${color}22`,
        fontFamily: theme.font,
        fontSize: 22,
        fontWeight: 700,
        color: theme.text,
        whiteSpace: 'nowrap' as const,
      }}
    >
      <span
        style={{
          width: 8,
          height: 8,
          borderRadius: '50%',
          background: color,
          boxShadow: `0 0 8px ${color}`,
          flexShrink: 0,
        }}
      />
      {label}
    </div>
  );
};

// Mock terminal-style content for the main area
const AgentTerminal: React.FC<{ frame: number; fps: number }> = ({ frame, fps }) => {
  const lines = [
    { t: 0, text: '$ otto run --agent claude --repo sinatra-users-go', color: theme.textDim },
    { t: 12, text: '✓ Session started · claude-opus-4', color: theme.accent2 },
    { t: 22, text: '  Analyzing codebase…', color: theme.textDim },
    { t: 34, text: '  Found 3 failing tests in /auth', color: theme.warn },
    { t: 46, text: '  Applying fix: add missing JWT validation', color: theme.text },
    { t: 58, text: '  Running test suite…', color: theme.textDim },
    { t: 72, text: '✓ All tests passing · 0.8s', color: theme.accent2 },
    { t: 84, text: '  Opening PR draft…', color: theme.textDim },
  ];

  return (
    <div
      style={{
        padding: '28px 36px',
        fontFamily: theme.mono,
        fontSize: 18,
        lineHeight: 1.85,
      }}
    >
      {lines.map((line, i) => {
        const lineStart = line.t;
        const opacity = interpolate(frame, [lineStart, lineStart + 10], [0, 1], {
          extrapolateRight: 'clamp',
          extrapolateLeft: 'clamp',
        });
        const x = interpolate(frame, [lineStart, lineStart + 10], [-10, 0], {
          extrapolateRight: 'clamp',
          extrapolateLeft: 'clamp',
        });
        return (
          <div
            key={i}
            style={{
              opacity,
              transform: `translateX(${x}px)`,
              color: line.color,
            }}
          >
            {line.text}
          </div>
        );
      })}
    </div>
  );
};

const Scene2Window: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  // Window slides up from below
  const windowSpring = spring({ frame, fps, config: { damping: 22, stiffness: 100 } });
  const windowY = interpolate(windowSpring, [0, 1], [120, 0]);
  const windowOpacity = interpolate(frame, [0, 20], [0, 1], { extrapolateRight: 'clamp' });
  const windowScale = interpolate(windowSpring, [0, 1], [0.94, 1]);

  const sessions = [
    { title: 'fix auth tests', provider: 'claude', status: 'working' as const },
    { title: 'refactor api/v2', provider: 'codex', status: 'working' as const },
    { title: 'add rate limit', provider: 'claude', status: 'idle' as const },
  ];

  // Pills stagger across the scene
  const pillStartOffset = 60;

  return (
    <AbsoluteFill
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 36,
      }}
    >
      {/* Window */}
      <div
        style={{
          opacity: windowOpacity,
          transform: `translateY(${windowY}px) scale(${windowScale})`,
          transformOrigin: 'center bottom',
        }}
      >
        <OttoWindow
          sidebar={<Navigator active="agents" sessions={sessions} />}
          title="Otto"
          style={{ width: 1300, height: 720 }}
        >
          <AgentTerminal frame={frame} fps={fps} />
        </OttoWindow>
      </div>

      {/* Feature pills row — floats below window */}
      <div
        style={{
          display: 'flex',
          gap: 16,
          flexWrap: 'wrap' as const,
          justifyContent: 'center',
          maxWidth: 1300,
        }}
      >
        {PILLS.map((pill, i) => (
          <FeaturePill
            key={pill.label}
            label={pill.label}
            color={pill.color}
            delay={pillStartOffset + i * 14}
          />
        ))}
      </div>
    </AbsoluteFill>
  );
};

// ─── Scene 3: "claude · codex · antigravity — together." (~330–450f, ~4s) ──

const Scene3Montage: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const agents = ['claude', 'codex', 'antigravity'];

  return (
    <AbsoluteFill
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 0,
      }}
    >
      {/* Agents line */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 0,
          fontFamily: theme.mono,
        }}
      >
        {agents.map((agent, i) => {
          const s = spring({ frame: frame - i * 14, fps, config: { damping: 180 } });
          const opacity = interpolate(s, [0, 1], [0, 1]);
          const y = interpolate(s, [0, 1], [24, 0]);
          const agentColors = [theme.accent, theme.accent2, '#bf7aff'];
          return (
            <React.Fragment key={agent}>
              <span
                style={{
                  opacity,
                  transform: `translateY(${y}px)`,
                  display: 'inline-block',
                  fontSize: 72,
                  fontWeight: 700,
                  color: agentColors[i],
                }}
              >
                {agent}
              </span>
              {i < agents.length - 1 && (
                <span
                  style={{
                    opacity: interpolate(frame, [i * 14 + 10, i * 14 + 24], [0, 1], {
                      extrapolateRight: 'clamp',
                      extrapolateLeft: 'clamp',
                    }),
                    display: 'inline-block',
                    fontSize: 48,
                    color: theme.border,
                    margin: '0 18px',
                    fontWeight: 300,
                  }}
                >
                  ·
                </span>
              )}
            </React.Fragment>
          );
        })}
      </div>

      {/* "— together." tagline */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 14,
          marginTop: 24,
        }}
      >
        {(() => {
          const s = spring({ frame: frame - 52, fps, config: { damping: 200 } });
          const opacity = interpolate(s, [0, 1], [0, 1]);
          const x = interpolate(s, [0, 1], [-20, 0]);
          return (
            <div
              style={{
                opacity,
                transform: `translateX(${x}px)`,
                fontFamily: theme.font,
                fontSize: 42,
                fontWeight: 600,
                color: theme.text,
                letterSpacing: -0.5,
              }}
            >
              — together, in one window.
            </div>
          );
        })()}
      </div>

      {/* Horizontal accent divider */}
      <div
        style={{
          marginTop: 40,
          width: interpolate(frame, [70, 100], [0, 560], {
            extrapolateRight: 'clamp',
            extrapolateLeft: 'clamp',
          }),
          height: 2,
          background: `linear-gradient(90deg, ${theme.accent}, ${theme.accent2}, #bf7aff)`,
          borderRadius: 2,
          opacity: interpolate(frame, [70, 85], [0, 1], { extrapolateRight: 'clamp' }),
        }}
      />
    </AbsoluteFill>
  );
};

// ─── Scene 4: Outro — Otto mark + name + ⌘K hint (~450–600f, ~5s) ──────────

const Scene4Outro: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  // Soft pulse on the logo glow
  const pulse = Math.sin(frame / 18) * 0.5 + 0.5;
  const glowSize = interpolate(pulse, [0, 1], [60, 100]);

  const logoS = spring({ frame, fps, config: { damping: 18, stiffness: 100 } });
  const logoScale = interpolate(logoS, [0, 1], [0.6, 1]);
  const logoOpacity = interpolate(frame, [0, 18], [0, 1], { extrapolateRight: 'clamp' });

  const nameS = spring({ frame: frame - 16, fps, config: { damping: 200 } });
  const nameOpacity = interpolate(nameS, [0, 1], [0, 1]);
  const nameY = interpolate(nameS, [0, 1], [22, 0]);

  const subtitleS = spring({ frame: frame - 30, fps, config: { damping: 200 } });
  const subtitleOpacity = interpolate(subtitleS, [0, 1], [0, 1]);
  const subtitleY = interpolate(subtitleS, [0, 1], [16, 0]);

  const cmdkS = spring({ frame: frame - 60, fps, config: { damping: 200 } });
  const cmdkOpacity = interpolate(cmdkS, [0, 1], [0, 1]);

  return (
    <AbsoluteFill
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 0,
      }}
    >
      {/* Otto mark */}
      <div
        style={{
          opacity: logoOpacity,
          transform: `scale(${logoScale})`,
          marginBottom: 28,
        }}
      >
        <Img
          src={staticFile('otto-mark.png')}
          style={{
            width: 160,
            height: 160,
            borderRadius: 38,
            boxShadow: `0 0 0 1.5px ${theme.accent}55, 0 20px ${glowSize}px ${theme.accent}55`,
          }}
        />
      </div>

      {/* Otto — Agentic Development Environment */}
      <div
        style={{
          opacity: nameOpacity,
          transform: `translateY(${nameY}px)`,
          fontFamily: theme.font,
          fontSize: 88,
          fontWeight: 900,
          color: theme.text,
          letterSpacing: -3,
          lineHeight: 1,
        }}
      >
        Otto
      </div>

      <div
        style={{
          opacity: subtitleOpacity,
          transform: `translateY(${subtitleY}px)`,
          fontFamily: theme.font,
          fontSize: 28,
          fontWeight: 500,
          color: theme.textDim,
          letterSpacing: 0.4,
          marginTop: 18,
        }}
      >
        Agentic Development Environment
      </div>

      {/* ⌘K hint */}
      <div
        style={{
          opacity: cmdkOpacity,
          position: 'absolute',
          bottom: 72,
          display: 'flex',
          alignItems: 'center',
          gap: 12,
          fontFamily: theme.font,
          fontSize: 20,
          color: theme.textDim,
        }}
      >
        <span>Press</span>
        <KeyCap>⌘</KeyCap>
        <KeyCap>K</KeyCap>
        <span>to launch</span>
      </div>
    </AbsoluteFill>
  );
};

// ─── Cross-fade helper ────────────────────────────────────────────────────────

const FadeIn: React.FC<{ children: React.ReactNode; durationFrames?: number }> = ({
  children,
  durationFrames = 18,
}) => {
  const frame = useCurrentFrame();
  const opacity = interpolate(frame, [0, durationFrames], [0, 1], {
    extrapolateRight: 'clamp',
    extrapolateLeft: 'clamp',
    easing: Easing.ease,
  });
  return <div style={{ opacity, width: '100%', height: '100%' }}>{children}</div>;
};

// ─── Root composition ─────────────────────────────────────────────────────────

export const Intro: React.FC = () => {
  // Scene timing (frames @ 30fps, total = 600f = 20s)
  // Scene 1: 0–100   (3.3s)  — Logo + tagline
  // Scene 2: 85–340  (8.5s)  — OttoWindow + pills  [overlaps scene1 exit]
  // Scene 3: 325–460 (4.5s)  — claude · codex · antigravity
  // Scene 4: 445–600 (5.2s)  — Outro

  return (
    <AbsoluteFill style={{ background: theme.bgGradient, fontFamily: theme.font }}>
      {/* Scene 1 */}
      <Sequence from={0} durationInFrames={115}>
        <FadeIn durationFrames={12}>
          <AbsoluteFill>
            <Scene1Logo />
          </AbsoluteFill>
        </FadeIn>
      </Sequence>

      {/* Scene 2 */}
      <Sequence from={90} durationInFrames={255}>
        <FadeIn durationFrames={20}>
          <AbsoluteFill>
            <Scene2Window />
          </AbsoluteFill>
        </FadeIn>
      </Sequence>

      {/* Scene 3 */}
      <Sequence from={330} durationInFrames={130}>
        <FadeIn durationFrames={18}>
          <AbsoluteFill>
            <Scene3Montage />
          </AbsoluteFill>
        </FadeIn>
      </Sequence>

      {/* Scene 4 */}
      <Sequence from={445} durationInFrames={155}>
        <FadeIn durationFrames={18}>
          <AbsoluteFill>
            <Scene4Outro />
          </AbsoluteFill>
        </FadeIn>
      </Sequence>
    </AbsoluteFill>
  );
};
