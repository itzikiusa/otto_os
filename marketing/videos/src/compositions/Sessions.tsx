import React from 'react';
import { AbsoluteFill } from 'remotion';
import { T, brand, fonts, providers, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
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
  Field,
  Toast,
  Segmented,
  Icon,
} from '../components/kit';

// ── Shared session model ──────────────────────────────────────────────────────

const SESSIONS: NavSession[] = [
  { title: 'fix auth middleware', provider: 'claude', status: 'working', tasks: [2, 4] },
  { title: 'refactor db layer',    provider: 'codex',  status: 'working', tasks: [1, 3] },
  { title: 'add e2e coverage',     provider: 'agy',    status: 'working', tasks: [0, 5] },
  { title: 'tail production logs', provider: 'shell',  status: 'idle' },
];

// ── Scene 1 — Title card ──────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Agent Sessions"
    title="Sessions"
    subtitle="Claude Code, Codex, Antigravity & shell — real PTY terminals, all at once"
  />
);

// ── Scene 2 — New Session dialog ──────────────────────────────────────────────

const NewSessionScene: React.FC = () => (
  <>
    <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
      <Appear delay={4} scale={0.92}>
        <div
          style={{
            background: T.surface,
            border: `1px solid ${T.border}`,
            borderRadius: 14,
            padding: 28,
            width: 500,
            boxShadow: T.shadow,
          }}
        >
          {/* header */}
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 10,
              marginBottom: 22,
              fontFamily: fonts.ui,
              fontSize: 17,
              fontWeight: 700,
              color: T.text,
            }}
          >
            <Icon name="terminal" size={18} color={T.accent} />
            New Agent Session
          </div>

          {/* provider */}
          <div style={{ marginBottom: 16 }}>
            <div
              style={{
                fontFamily: fonts.ui,
                fontSize: 12.5,
                fontWeight: 500,
                color: T.textDim,
                marginBottom: 8,
              }}
            >
              Provider
            </div>
            <Segmented options={['claude', 'codex', 'agy', 'shell']} active={0} t={T} />
          </div>

          {/* workspace */}
          <Field
            label="Workspace"
            value="~/projects/sinatra-users-go"
            icon="folder"
            t={T}
            focused
            style={{ marginBottom: 14 }}
          />

          {/* session name */}
          <Field
            label="Session name"
            placeholder="auto-generated from first task"
            t={T}
            style={{ marginBottom: 22 }}
          />

          {/* actions */}
          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 10 }}>
            <Button variant="ghost" t={T}>Cancel</Button>
            <Button variant="primary" icon="play" t={T}>Start Session</Button>
          </div>
        </div>
      </Appear>
    </AbsoluteFill>
    <Caption
      step={1}
      title="Pick a provider and a workspace"
      sub="Otto spawns a real PTY — claude, codex, agy, or a plain shell"
    />
  </>
);

// ── Scene 3 — Tiled multi-session (2 × 2 grid) ───────────────────────────────

interface MiniPaneProps {
  title: string;
  provider: string;
  provColor: string;
  lines: TermLine[];
  live: boolean;
}

const MiniPane: React.FC<MiniPaneProps> = ({ title, provider, provColor, lines, live }) => (
  <div
    style={{
      display: 'flex',
      flexDirection: 'column',
      background: T.termBg,
      border: `1px solid ${live ? alpha(provColor, 0.3) : T.border}`,
      borderRadius: 10,
      overflow: 'hidden',
      minHeight: 0,
    }}
  >
    {/* pane header */}
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: '8px 12px',
        borderBottom: `1px solid ${T.border}`,
        background: alpha('#fff', 0.02),
        flexShrink: 0,
      }}
    >
      <StatusDot kind={live ? 'working' : 'idle'} size={8} />
      <span
        style={{
          flex: 1,
          fontFamily: fonts.mono,
          fontSize: 11.5,
          fontWeight: 600,
          color: T.text,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {title}
      </span>
      <Chip color={provColor}>{provider}</Chip>
    </div>
    <Terminal
      lines={lines}
      delay={20}
      step={9}
      pad={10}
      fontSize={12}
      style={{ flex: 1, background: 'transparent', borderRadius: 0, minHeight: 0, overflow: 'hidden' }}
    />
  </div>
);

const claudeLines: TermLine[] = [
  { text: '$ go test ./auth/...',             tone: 'cmd'  },
  { text: '  reading jwt.go + middleware.go', tone: 'dim'  },
  { text: '  ✗ token expiry off-by-one',      tone: 'warn' },
  { text: '  applying fix → jwt.go:47',       tone: 'text' },
  { text: '  ✓ 89 passed  (1.6s)',            tone: 'ok'   },
];

const codexLines: TermLine[] = [
  { text: '$ codex run plan.md',              tone: 'cmd' },
  { text: '  restructuring db/queries.go',    tone: 'dim' },
  { text: '  extracting interface → repo.go', tone: 'text'},
  { text: '  writing tests/db_repo_test.go',  tone: 'dim' },
  { text: '  ✓ build ok · coverage 81%',     tone: 'ok'  },
];

const agyLines: TermLine[] = [
  { text: '$ agy start task.yaml',            tone: 'cmd'  },
  { text: '  reading openapi.yaml + routes',  tone: 'dim'  },
  { text: '  scaffolding e2e/auth_test.go',   tone: 'text' },
  { text: '  scaffolding e2e/user_test.go',   tone: 'text' },
  { text: '  playwright  12/12 ✓',            tone: 'ok'   },
];

const shellLines: TermLine[] = [
  { text: '$ kubectl logs -f api-6d8f9b',      tone: 'cmd'  },
  { text: '  [INFO] GET  /users/42   200  3ms', tone: 'dim'  },
  { text: '  [INFO] POST /auth/login 200 11ms', tone: 'dim'  },
  { text: '  [WARN] rate-limit: ip 10.0.1.4',  tone: 'warn' },
  { text: '  idle — watching',                  tone: 'dim'  },
];

const TiledScene: React.FC = () => (
  <>
    <Stage scale={0.87}>
      <OttoWindow
        nav={
          <Navigator
            active="agents"
            sessions={SESSIONS}
            activeSessionTitle="fix auth middleware"
            workingCount={3}
          />
        }
        title="Otto — sinatra-users-go"
      >
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: '1fr 1fr',
            gridTemplateRows: '1fr 1fr',
            gap: 10,
            padding: 12,
            height: '100%',
            boxSizing: 'border-box',
          }}
        >
          <MiniPane
            title="fix auth middleware"
            provider="claude"
            provColor={providers.claude}
            lines={claudeLines}
            live
          />
          <MiniPane
            title="refactor db layer"
            provider="codex"
            provColor={providers.codex}
            lines={codexLines}
            live
          />
          <MiniPane
            title="add e2e coverage"
            provider="agy"
            provColor={providers.agy}
            lines={agyLines}
            live
          />
          <MiniPane
            title="tail production logs"
            provider="shell"
            provColor={providers.shell}
            lines={shellLines}
            live={false}
          />
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Run many at once — tiled, watch them work live"
      sub="Every pane is a real PTY · type directly into any agent · Navigator shows live status dots"
    />
  </>
);

// ── Scene 4 — Broadcast ───────────────────────────────────────────────────────

const BroadcastScene: React.FC = () => (
  <>
    <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
      <Appear delay={4} scale={0.92}>
        <div
          style={{
            background: T.surface,
            border: `1px solid ${T.border}`,
            borderRadius: 14,
            padding: 24,
            width: 560,
            boxShadow: T.shadow,
          }}
        >
          {/* header */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 18 }}>
            <Icon name="send" size={18} color={brand.cyan} />
            <span
              style={{
                flex: 1,
                fontFamily: fonts.ui,
                fontSize: 16,
                fontWeight: 700,
                color: T.text,
              }}
            >
              Message all working sessions
            </span>
            <Chip tone="ok">3 sessions</Chip>
          </div>

          {/* message field */}
          <Field
            value="wrap up and commit what you have"
            focused
            caret
            t={T}
            style={{ marginBottom: 16 }}
          />

          {/* session chips + send button */}
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <div style={{ display: 'flex', gap: 6 }}>
              <Chip color={providers.claude}>claude</Chip>
              <Chip color={providers.codex}>codex</Chip>
              <Chip color={providers.agy}>agy</Chip>
            </div>
            <Button variant="primary" icon="send" t={T}>Broadcast</Button>
          </div>
        </div>
      </Appear>

      {/* confirmation toast */}
      <Toast
        text="Sent to 3 sessions — claude · codex · agy"
        tone="ok"
        delay={56}
        style={{ position: 'absolute', top: '26%', right: '10%' }}
      />
    </AbsoluteFill>
    <Caption
      step={3}
      title="Broadcast one message to every live agent"
      sub="No AI in the loop — the literal text lands in every working terminal at once"
    />
  </>
);

// ── Scene 5 — Resumable, idle-suspend, auto-trust ─────────────────────────────

const resumeLines: TermLine[] = [
  { text: '$ otto resume fix-auth-middleware',          tone: 'cmd'  },
  { text: '  ⟳ restoring session state…',               tone: 'dim'  },
  { text: '  ✓ workspace ~/projects/sinatra-users-go — trusted', tone: 'ok' },
  { text: '  ✓ PTY respawned (PID 18423)',              tone: 'ok'   },
  { text: '  ✓ transcript loaded (142 turns)',          tone: 'ok'   },
  { text: '',                                           tone: 'text' },
  { text: '  claude: picking up where we left off…',   tone: 'text' },
  { text: '  reading jwt.go + auth/handler_test.go',   tone: 'dim'  },
  { text: '  continuing — token expiry edge-case',     tone: 'text' },
];

const ResumeTrustScene: React.FC = () => (
  <>
    <Stage scale={0.87}>
      <OttoWindow
        nav={
          <Navigator
            active="agents"
            sessions={SESSIONS}
            activeSessionTitle="fix auth middleware"
            workingCount={3}
          />
        }
        title="Otto — sinatra-users-go"
      >
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            height: '100%',
            padding: 20,
            boxSizing: 'border-box',
          }}
        >
          {/* status badges */}
          <Appear
            delay={4}
            style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 14 }}
          >
            <Chip tone="ok">workspace trusted</Chip>
            <Chip tone="warn">idle-suspended → resuming</Chip>
            <Chip color={providers.claude}>claude</Chip>
          </Appear>

          <Terminal
            lines={resumeLines}
            delay={12}
            step={11}
            fontSize={14}
            style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}
          />
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={4}
      title="Resumable · idle-suspend · auto-trust"
      sub="Sessions survive daemon restarts · workspace auto-trusted on launch · stuck agents get a nudge"
    />
  </>
);

// ── Scenes ────────────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 80,  node: <TitleScene />,       name: 'Title'       },
  { dur: 90,  node: <NewSessionScene />,  name: 'NewSession'  },
  { dur: 180, node: <TiledScene />,       name: 'Tiled'       },
  { dur: 100, node: <BroadcastScene />,   name: 'Broadcast'   },
  { dur: 85,  node: <ResumeTrustScene />, name: 'ResumeTrust' },
  {
    dur: 120,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Agent Sessions"
        tagline="Claude Code, Codex, Antigravity & shell — orchestrated"
        pills={[
          { label: 'Tiled',      icon: 'grid'    },
          { label: 'Broadcast',  icon: 'send'    },
          { label: 'Resumable',  icon: 'refresh' },
          { label: 'Auto-trust', icon: 'check'   },
        ]}
      />
    ),
  },
];

export const sessionsDuration: number = scenesDuration(SCENES);
export const Sessions: React.FC = () => <Scenes scenes={SCENES} />;
