import React from 'react';
import { AbsoluteFill, useCurrentFrame, interpolate } from 'remotion';
import { brand, fonts, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, FloorGlow } from '../components/scene';
import {
  Appear,
  Kicker,
  BrandWord,
  FeaturePill,
  Keys,
  OttoIcon,
} from '../components/kit';

// ── Scene 1 — Recap (~150f) ───────────────────────────────────────────────────
//
// Brand closer recap: kicker + gradient headline + staggered pill grid.
// No app window — pure cinematic. The 12-pill stagger (delay 30 + i×9)
// means the final pill springs in at frame ~129; the scene then holds
// every element visible through frame 150, staying alive to the cut.

const MARQUEE: { label: string; color: string; icon: string }[] = [
  { label: 'Agent Sessions',     color: '#d97757',  icon: 'terminal' },
  { label: 'Mission Control',    color: '#47bfff',  icon: 'gauge'    },
  { label: 'Git & Review',       color: '#28c840',  icon: 'branch'   },
  { label: 'Product & Canvas',   color: '#a78bfa',  icon: 'note'     },
  { label: 'Swarm & Goal Loops', color: '#863bff',  icon: 'grid'     },
  { label: 'Database & Brokers', color: '#0a84ff',  icon: 'db'       },
  { label: 'Scheduled Tasks',    color: '#bf7aff',  icon: 'clock'    },
  { label: 'MCP & Plugins',      color: '#ff8a65',  icon: 'plug'     },
  { label: 'Proof Packs',        color: '#28c840',  icon: 'check'    },
  { label: 'Usage & Insights',   color: '#febc2e',  icon: 'chart'    },
  { label: 'Vault',              color: '#8ab4f8',  icon: 'globe'    },
  { label: 'Remote & Mobile',    color: '#2684ff',  icon: 'share'    },
];

const Recap: React.FC = () => (
  <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center', padding: '0 120px' }}>
    {/* Kicker eyebrow */}
    <div style={{ marginBottom: 18 }}>
      <Kicker delay={2}>Everything, in one window</Kicker>
    </div>

    {/* Headline — white + gradient second line */}
    <Appear delay={12} y={24}>
      <div
        style={{
          fontFamily: fonts.ui,
          fontSize: 62,
          fontWeight: 800,
          letterSpacing: -1.6,
          color: '#ffffff',
          textAlign: 'center',
          lineHeight: 1.1,
          marginBottom: 46,
        }}
      >
        Your whole engineering workflow,
        <br />
        <span
          style={{
            backgroundImage: brand.gradSoft,
            WebkitBackgroundClip: 'text',
            backgroundClip: 'text',
            color: 'transparent',
            WebkitTextFillColor: 'transparent',
          }}
        >
          orchestrated.
        </span>
      </div>
    </Appear>

    {/* Pill grid — staggered so the last pill arrives near frame 129, then holds */}
    <div
      style={{
        display: 'flex',
        flexWrap: 'wrap',
        gap: 16,
        justifyContent: 'center',
        maxWidth: 1440,
      }}
    >
      {MARQUEE.map((p, i) => (
        <FeaturePill
          key={p.label}
          label={p.label}
          color={p.color}
          icon={p.icon}
          delay={30 + i * 9}
        />
      ))}
    </div>

    <FloorGlow color={brand.purple} w={800} />
  </AbsoluteFill>
);

// ── Scene 2 — Brand Lockup (~140f) ────────────────────────────────────────────
//
// Cinematic closing frame: icon + two slow-expanding rings, BrandWord, subtitle,
// closing tagline, and ⌘K hint. The last scene — never fades out (holds to end).

const Lockup: React.FC = () => {
  const frame = useCurrentFrame();

  // Two rings emerge from center and expand outward, fading as they spread —
  // keeps the scene alive without competing with the wordmark.
  const r1 = interpolate(frame, [4, 130], [0, 680], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' });
  const op1 = interpolate(frame, [4, 70, 139], [0, 0.45, 0.0], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' });
  const r2 = interpolate(frame, [22, 139], [0, 900], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' });
  const op2 = interpolate(frame, [22, 90, 139], [0, 0.20, 0.0], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' });

  return (
    <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
      {/* Purple expanding ring */}
      <div
        style={{
          position: 'absolute',
          top: '50%',
          left: '50%',
          width: r1,
          height: r1,
          transform: 'translate(-50%, -50%)',
          borderRadius: '50%',
          border: `1.5px solid ${alpha(brand.purple, op1)}`,
          boxShadow: `0 0 48px ${alpha(brand.purple, op1 * 0.5)}`,
          pointerEvents: 'none',
        }}
      />
      {/* Cyan outer ring */}
      <div
        style={{
          position: 'absolute',
          top: '50%',
          left: '50%',
          width: r2,
          height: r2,
          transform: 'translate(-50%, -50%)',
          borderRadius: '50%',
          border: `1px solid ${alpha(brand.cyan, op2)}`,
          pointerEvents: 'none',
        }}
      />

      {/* Logo */}
      <Appear delay={2} scale={0.52} y={0} style={{ marginBottom: 32 }}>
        <OttoIcon size={148} glowPx={120} />
      </Appear>

      {/* "Otto" wordmark */}
      <BrandWord delay={16} size={120}>Otto</BrandWord>

      {/* Subtitle */}
      <Appear delay={28} y={18}>
        <div
          style={{
            fontFamily: fonts.ui,
            fontSize: 32,
            color: alpha('#ffffff', 0.62),
            marginTop: 18,
            textAlign: 'center',
            letterSpacing: 0.1,
          }}
        >
          The Agentic Development Environment
        </div>
      </Appear>

      {/* Closing line */}
      <Appear delay={42} y={14}>
        <div
          style={{
            fontFamily: fonts.ui,
            fontSize: 27,
            color: alpha('#ffffff', 0.78),
            marginTop: 14,
            textAlign: 'center',
            letterSpacing: 0.1,
          }}
        >
          Run your coding agents{' '}
          <span style={{ color: brand.cyan, fontWeight: 700 }}>like a pro.</span>
        </div>
      </Appear>

      {/* ⌘K hint */}
      <div style={{ marginTop: 52, display: 'flex', alignItems: 'center', gap: 16 }}>
        <Appear delay={56}>
          <span style={{ fontFamily: fonts.ui, fontSize: 21, color: alpha('#ffffff', 0.52) }}>
            Press
          </span>
        </Appear>
        <Keys keys={['⌘', 'K']} delay={62} />
        <Appear delay={68}>
          <span style={{ fontFamily: fonts.ui, fontSize: 21, color: alpha('#ffffff', 0.52) }}>
            to launch anything
          </span>
        </Appear>
      </div>

      <FloorGlow color={brand.cyan} w={580} />
    </AbsoluteFill>
  );
};

// ── Composition ───────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 150, node: <Recap />,  name: 'Recap'  },
  { dur: 140, node: <Lockup />, name: 'Lockup' },
];

export const outroDuration = scenesDuration(SCENES);
export const Outro: React.FC = () => <Scenes scenes={SCENES} />;
