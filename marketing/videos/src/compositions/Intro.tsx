import React from 'react';
import { AbsoluteFill, useCurrentFrame, interpolate } from 'remotion';
import { T, brand, fonts, providers, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, FloorGlow } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator, NavSession } from '../components/Nav';
import {
  Appear,
  Kicker,
  BrandWord,
  Caption,
  FeaturePill,
  Terminal,
  TermLine,
  StatusDot,
  Keys,
  Chip,
  OttoIcon,
} from '../components/kit';

// ── Scene 1 — brand cold open (~115f) ─────────────────────────────────────────
const Open: React.FC = () => {
  const frame = useCurrentFrame();

  // Two expanding rings at different radii + opacities for a "portal" feel.
  const ring1r = interpolate(frame, [8, 96], [0, 640], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  const ring1Op = interpolate(frame, [8, 80, 114], [0, 0.5, 0], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  const ring2r = interpolate(frame, [28, 112], [0, 840], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  const ring2Op = interpolate(frame, [28, 100, 114], [0, 0.22, 0], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });

  return (
    <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
      {/* cyan ring */}
      <div
        style={{
          position: 'absolute',
          top: '50%',
          left: '50%',
          width: ring1r,
          height: ring1r,
          transform: 'translate(-50%, -50%)',
          borderRadius: '50%',
          border: `1.5px solid ${alpha(brand.cyan, ring1Op)}`,
          boxShadow: `0 0 60px ${alpha(brand.purple, ring1Op * 0.6)}`,
          pointerEvents: 'none',
        }}
      />
      {/* purple outer ring */}
      <div
        style={{
          position: 'absolute',
          top: '50%',
          left: '50%',
          width: ring2r,
          height: ring2r,
          transform: 'translate(-50%, -50%)',
          borderRadius: '50%',
          border: `1px solid ${alpha(brand.purple, ring2Op)}`,
          pointerEvents: 'none',
        }}
      />

      <Appear delay={2} scale={0.52} y={0} style={{ marginBottom: 34 }}>
        <OttoIcon size={164} glowPx={130} />
      </Appear>

      <div style={{ marginBottom: 18 }}>
        <Kicker delay={18}>Agentic Development Environment</Kicker>
      </div>

      <BrandWord delay={28} size={128}>Otto</BrandWord>

      <Appear delay={40} y={18}>
        <div
          style={{
            fontFamily: fonts.ui,
            fontSize: 31,
            color: alpha('#ffffff', 0.65),
            marginTop: 20,
            textAlign: 'center',
            letterSpacing: 0.1,
          }}
        >
          Run your coding agents{' '}
          <span style={{ color: brand.cyan, fontWeight: 700 }}>like a pro.</span>
        </div>
      </Appear>

      <FloorGlow color={brand.purple} w={740} />
    </AbsoluteFill>
  );
};

// ── Scene 2 — real window, agents working (~195f) ─────────────────────────────
const sessions: NavSession[] = [
  { title: 'fix: auth middleware', provider: 'claude', status: 'working', tasks: [2, 5] },
  { title: 'refactor api/v2 routes', provider: 'codex', status: 'working', tasks: [1, 4] },
  { title: 'add rate-limit tests', provider: 'claude', status: 'idle', tasks: [3, 3] },
];

const claudeLines: TermLine[] = [
  { text: '$ go test ./internal/auth/...', tone: 'cmd' },
  { text: '  → reading: middleware.go, jwt.go, handlers.go', tone: 'dim' },
  { text: '  FAIL TestTokenValidation  (missing exp claim check)', tone: 'err' },
  { text: '  FAIL TestRefreshFlow  (nil ptr at middleware:84)', tone: 'err' },
  { text: '  ↳ applying patch → middleware/jwt_validate.go', tone: 'accent' },
  { text: '  re-running 142 tests…', tone: 'dim' },
  { text: '  ✓ PASS — 142 tests, 0 failures  (3.2s)', tone: 'ok' },
];

const codexLines: TermLine[] = [
  { text: '$ codex run refactor-api-v2.md', tone: 'cmd' },
  { text: '  reading: server.go, routes.go, handlers/', tone: 'dim' },
  { text: '  splitting monolithic router → 6 sub-routers', tone: 'text' },
  { text: '  writing: routes/auth.go, routes/users.go, …', tone: 'accent' },
  { text: '  ✓ build ok · go vet clean · 0 issues', tone: 'ok' },
  { text: '  drafting PR: "refactor(api): modular v2 routing"', tone: 'dim' },
  { text: '  ✓ PR #147 opened → github.com/acme/sinatra-go', tone: 'ok' },
];

const AgentPane: React.FC<{
  name: string;
  color: string;
  lines: TermLine[];
  live?: boolean;
}> = ({ name, color, lines, live = true }) => (
  <div
    style={{
      flex: 1,
      display: 'flex',
      flexDirection: 'column',
      background: T.termBg,
      border: `1px solid ${T.border}`,
      borderRadius: 10,
      overflow: 'hidden',
    }}
  >
    {/* pane header */}
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: '9px 12px',
        borderBottom: `1px solid ${T.border}`,
        background: alpha('#ffffff', 0.025),
        flexShrink: 0,
      }}
    >
      <StatusDot kind={live ? 'working' : 'idle'} size={9} />
      <span
        style={{
          flex: 1,
          fontFamily: fonts.ui,
          fontSize: 13,
          fontWeight: 600,
          color: T.text,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {name}
      </span>
      <Chip color={color}>{name.split(' ')[0]}</Chip>
    </div>
    <Terminal
      lines={lines}
      delay={20}
      step={10}
      pad={14}
      fontSize={13.5}
      style={{ flex: 1, background: 'transparent', borderRadius: 0 }}
    />
  </div>
);

const WindowScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={
          <Navigator
            active="agents"
            sessions={sessions}
            activeSessionTitle="fix: auth middleware"
            workingCount={2}
          />
        }
        tabs={[
          { label: 'fix: auth middleware',   icon: 'terminal', active: true, dot: 'working' },
          { label: 'refactor api/v2 routes', icon: 'terminal', dot: 'working' },
          { label: 'add rate-limit tests',   icon: 'terminal' },
        ]}
        title="Otto — sinatra-go"
      >
        <div
          style={{
            display: 'flex',
            gap: 12,
            padding: 16,
            height: '100%',
            boxSizing: 'border-box',
          }}
        >
          <AgentPane
            name="claude · fix: auth middleware"
            color={providers.claude}
            lines={claudeLines}
          />
          <AgentPane
            name="codex · refactor api/v2 routes"
            color={providers.codex}
            lines={codexLines}
          />
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Claude Code, Codex, Antigravity & shell — as first-class sessions"
      sub="Watch them work in real time. Type in, interrupt, resume. All sessions survive on the daemon."
    />
  </>
);

// ── Scene 3 — one window, the whole workflow (~210f) ──────────────────────────
const PILLARS: { label: string; color: string; icon: string }[] = [
  { label: 'Agent Sessions',      color: providers.claude,  icon: 'terminal' },
  { label: 'Mission Control',     color: '#47bfff',         icon: 'gauge'    },
  { label: 'Git & Pull Requests', color: '#28c840',         icon: 'branch'   },
  { label: 'AI Code Review',      color: brand.violet,      icon: 'eye'      },
  { label: 'Jira / Confluence',   color: '#2684ff',         icon: 'note'     },
  { label: 'Product Canvas',      color: '#a78bfa',         icon: 'square'   },
  { label: 'Database Explorer',   color: '#0a84ff',         icon: 'db'       },
  { label: 'Kafka Brokers',       color: '#febc2e',         icon: 'box'      },
  { label: 'Agent Swarm',         color: brand.cyan,        icon: 'grid'     },
  { label: 'Goal Loops',          color: '#28c840',         icon: 'refresh'  },
  { label: 'Channels',            color: '#36c5f0',         icon: 'slack'    },
  { label: 'Workflows',           color: '#9ee039',         icon: 'split'    },
  { label: 'Scheduled Tasks',     color: '#bf7aff',         icon: 'clock'    },
  { label: 'MCP Control Plane',   color: '#ff8a65',         icon: 'plug'     },
  { label: 'Proof Packs',         color: '#28c840',         icon: 'check'    },
  { label: 'Skills',              color: brand.purple,      icon: 'zap'      },
  { label: 'Knowledge Vault',     color: providers.agy,     icon: 'globe'    },
  { label: 'Usage & Insights',    color: '#ff8a65',         icon: 'chart'    },
  { label: 'Custom Plugins',      color: '#a78bfa',         icon: 'zap'      },
  { label: 'API Client',          color: '#0a84ff',         icon: 'send'     },
  { label: 'Remote & Mobile',     color: '#2684ff',         icon: 'share'    },
  { label: 'RBAC & Sharing',      color: '#febc2e',         icon: 'key'      },
];

const PillarsScene: React.FC = () => (
  <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center', padding: '0 100px' }}>
    <div style={{ marginBottom: 16 }}>
      <Kicker delay={2}>One window</Kicker>
    </div>

    <Appear delay={10} y={22}>
      <div
        style={{
          fontFamily: fonts.ui,
          fontSize: 58,
          fontWeight: 800,
          letterSpacing: -1.5,
          color: '#ffffff',
          textAlign: 'center',
          lineHeight: 1.1,
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
          wired into one place.
        </span>
      </div>
    </Appear>

    <div
      style={{
        display: 'flex',
        flexWrap: 'wrap',
        gap: 14,
        justifyContent: 'center',
        maxWidth: 1540,
        marginTop: 44,
      }}
    >
      {PILLARS.map((p, i) => (
        <FeaturePill
          key={p.label}
          label={p.label}
          color={p.color}
          icon={p.icon}
          delay={24 + i * 5}
        />
      ))}
    </div>
  </AbsoluteFill>
);

// ── Scene 4 — lockup outro (~120f) ────────────────────────────────────────────
const Lockup: React.FC = () => (
  <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
    <Appear delay={2} scale={0.6} y={0} style={{ marginBottom: 32 }}>
      <OttoIcon size={140} glowPx={118} />
    </Appear>

    <Appear delay={14} y={26}>
      <div
        style={{
          fontFamily: fonts.ui,
          fontSize: 86,
          fontWeight: 800,
          letterSpacing: -2.5,
          color: '#ffffff',
          textAlign: 'center',
          lineHeight: 1.04,
        }}
      >
        Your agents,{' '}
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

    <Appear delay={24} y={16}>
      <div
        style={{
          fontFamily: fonts.ui,
          fontSize: 27,
          color: alpha('#ffffff', 0.62),
          marginTop: 20,
          textAlign: 'center',
          letterSpacing: 0.2,
        }}
      >
        Otto — the Agentic Development Environment
      </div>
    </Appear>

    <div style={{ marginTop: 52, display: 'flex', alignItems: 'center', gap: 18 }}>
      <Appear delay={36}>
        <span style={{ fontFamily: fonts.ui, fontSize: 22, color: alpha('#ffffff', 0.58) }}>
          Press
        </span>
      </Appear>
      <Keys keys={['⌘', 'K']} delay={40} />
      <Appear delay={44}>
        <span style={{ fontFamily: fonts.ui, fontSize: 22, color: alpha('#ffffff', 0.58) }}>
          to launch anything
        </span>
      </Appear>
    </div>

    <FloorGlow color={brand.cyan} w={620} />
  </AbsoluteFill>
);

// ── Composition ───────────────────────────────────────────────────────────────
const SCENES: SceneDef[] = [
  { dur: 115, node: <Open />,         name: 'Open'    },
  { dur: 195, node: <WindowScene />,  name: 'Window'  },
  { dur: 210, node: <PillarsScene />, name: 'Pillars' },
  { dur: 120, node: <Lockup />,       name: 'Lockup'  },
];

export const introDuration = scenesDuration(SCENES);
export const Intro: React.FC = () => <Scenes scenes={SCENES} />;
