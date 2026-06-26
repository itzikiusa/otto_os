import React from 'react';
import { T, brand, fonts, providers } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Stagger,
  TitleCard,
  Caption,
  Card,
  Chip,
  Button,
  StatusDot,
  Avatar,
  Icon,
  Toast,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  PRODUCT — Jira & Confluence: import → analyze → rewrite → plan → discover
// ════════════════════════════════════════════════════════════════════════════

// ── Shared data ───────────────────────────────────────────────────────────────

interface LensData {
  label: string;
  finding: string;
  dot: 'working' | 'idle' | 'needsYou';
}

const LENSES: LensData[] = [
  {
    label: 'Risk',
    finding: 'Static 7-day window enables token replay — must rotate on each successful use',
    dot: 'working',
  },
  {
    label: 'Edge Cases',
    finding: 'Concurrent refresh calls may race and produce two simultaneously-valid tokens',
    dot: 'working',
  },
  {
    label: 'Dependencies',
    finding: 'AuthService · SessionStore · AuditLog all require coordinated schema migrations',
    dot: 'needsYou',
  },
  {
    label: 'Acceptance',
    finding: 'Missing: revoke-on-logout scope, device-binding criteria, rotation rate-limit',
    dot: 'idle',
  },
];

const SPEC_SECTIONS = [
  {
    title: 'Background',
    body: 'Refresh tokens are currently static for 7 days. Any intercepted token is replayable within that window. Rotating on each successful use closes the exposure to the last-use interval.',
  },
  {
    title: 'Requirements',
    body: 'FR-1: Rotate token on every successful /auth/refresh call.\nFR-2: Invalidate the previous token within 100 ms of issue.\nFR-3: 48 h dual-validity grace for concurrent clients during rollout.',
  },
  {
    title: 'Acceptance Criteria',
    body: 'AC-1: Login → refresh → old token returns 401 Unauthorized.\nAC-2: Concurrent refresh returns 409 + Retry-After header.\nAC-3: Revoke-on-logout invalidates the entire token family.',
  },
  {
    title: 'Edge Cases',
    body: 'Network failure mid-rotation must never leave two valid tokens active. Device-bound tokens require device fingerprint in the rotation key.',
  },
];

interface TaskData {
  label: string;
  done: boolean;
  owner: string;
  color: string;
}

const TASKS: TaskData[] = [
  { label: 'Research token rotation RFCs + prior art', done: true,  owner: 'Alice', color: brand.cyan   },
  { label: 'Implement rotation in auth/token.go',       done: false, owner: 'Ben',   color: brand.purple },
  { label: 'Invalidate previous token within 100 ms',   done: false, owner: 'Ben',   color: brand.purple },
  { label: 'Add E2E tests for concurrent-refresh race', done: false, owner: 'Carol', color: brand.violet },
  { label: 'Update API docs + CHANGELOG entry',         done: false, owner: 'Alice', color: brand.cyan   },
];

const DISCOVERY_BODY =
  'We need to pick a rate-limit strategy for auth endpoints before PROJ-1421 ships.\n\n' +
  'Options on the table:\n' +
  '• Sliding window — resets TTL on each hit; simple but exploitable under burst traffic\n' +
  '• Token bucket — smooth burst handling; better fit for OAuth refresh flows\n' +
  '• Fixed window + exponential backoff — easiest to reason about in distributed deployments\n\n' +
  'Recommendation: token bucket at 10 req / min per device with a 5× burst allowance.';

// ── Scene 1 — Title ───────────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Product · Jira & Confluence"
    title="Product"
    subtitle="Import any ticket or Confluence page — refine it into a build-ready spec, then hand it to your agents"
  />
);

// ── Scene 2 — Import + Analyze ────────────────────────────────────────────────

const LensCard: React.FC<LensData & { delay: number }> = ({ label, finding, dot, delay }) => (
  <Appear delay={delay} y={16} style={{ flex: 1, minWidth: 0 }}>
    <Card t={T} pad={12}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 7, marginBottom: 7 }}>
        <StatusDot kind={dot} size={9} />
        <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 700, color: T.text }}>
          {label}
        </span>
      </div>
      <div style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim, lineHeight: 1.55 }}>
        {finding}
      </div>
    </Card>
  </Appear>
);

const AnalyzeScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow nav={<Navigator active="product" />} title="Otto — Product">
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            height: '100%',
            padding: 20,
            gap: 12,
            boxSizing: 'border-box',
            overflow: 'hidden',
          }}
        >
          {/* Story header */}
          <Appear delay={6} y={18}>
            <Card t={T} pad={14}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 7 }}>
                <Icon name="ticket" size={15} color={T.textDim} />
                <span style={{ fontFamily: fonts.mono, fontSize: 13, fontWeight: 700, color: T.accent }}>
                  PROJ-1421
                </span>
                <span
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 15,
                    fontWeight: 700,
                    color: T.text,
                    flex: 1,
                  }}
                >
                  Refresh-token rotation
                </span>
                <Chip tone="warn">In Progress</Chip>
                <Chip color={providers.claude}>claude</Chip>
                <Chip color={providers.codex}>codex</Chip>
              </div>
              <div style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim, lineHeight: 1.62 }}>
                Implement automatic rotation of refresh tokens on each successful use to prevent token replay
                attacks. Existing sessions must remain valid during the 48-hour migration grace period.
              </div>
            </Card>
          </Appear>

          {/* Lenses heading */}
          <Appear delay={18} y={10}>
            <div
              style={{
                fontFamily: fonts.ui,
                fontSize: 11.5,
                fontWeight: 600,
                letterSpacing: 1.5,
                textTransform: 'uppercase' as const,
                color: T.textDim,
              }}
            >
              Analysis Lenses · 4 providers
            </div>
          </Appear>

          {/* Lens cards row */}
          <div style={{ display: 'flex', gap: 10, flexShrink: 0 }}>
            {LENSES.map((l, i) => (
              <LensCard key={l.label} {...l} delay={24 + i * 9} />
            ))}
          </div>

          {/* Summary */}
          <Appear delay={66} y={14}>
            <Card t={T} pad={14}>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 11.5,
                  fontWeight: 600,
                  letterSpacing: 1.5,
                  textTransform: 'uppercase' as const,
                  color: T.textDim,
                  marginBottom: 8,
                }}
              >
                Summary
              </div>
              <div style={{ fontFamily: fonts.ui, fontSize: 13, color: T.text, lineHeight: 1.65 }}>
                Token rotation must invalidate the previous refresh token within 100 ms of issue. The static
                7-day expiry is a critical vulnerability window. Session continuity during migration requires a
                coordinated dual-validity grace period. AuthService and SessionStore need simultaneous schema
                updates.
              </div>
            </Card>
          </Appear>

          {/* Open questions */}
          <Appear delay={90} y={14}>
            <Card t={T} pad={14}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  marginBottom: 10,
                }}
              >
                <div
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 11.5,
                    fontWeight: 600,
                    letterSpacing: 1.5,
                    textTransform: 'uppercase' as const,
                    color: T.textDim,
                  }}
                >
                  Open Questions · 3
                </div>
                <Button variant="primary" size="s" icon="comment">
                  Post as comments
                </Button>
              </div>
              {[
                'How long should each rotated token remain valid during the dual-validity grace window?',
                'Should rotation be sliding (reset TTL on use) or fixed (inherit original expiry time)?',
                'What is the correct response for concurrent refresh calls — 409 + retry, or rotate-last-wins?',
              ].map((q, i) => (
                <div
                  key={i}
                  style={{
                    display: 'flex',
                    gap: 8,
                    alignItems: 'flex-start',
                    padding: '6px 0',
                    borderTop: `1px solid ${T.border}`,
                  }}
                >
                  <Icon name="dot" size={12} color={T.textDim} style={{ marginTop: 3, flexShrink: 0 }} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.text, lineHeight: 1.55 }}>
                    {q}
                  </span>
                </div>
              ))}
            </Card>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Import a ticket — analyze across lenses & providers, surface open questions"
      sub="Risk · Edge Cases · Dependencies · Acceptance — summarized, ready to post back as Jira comments"
      delay={16}
    />
  </>
);

// ── Scene 3 — Rewrite + Plan ──────────────────────────────────────────────────

const TaskRow: React.FC<TaskData> = ({ label, done, owner, color }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 10,
      padding: '8px 12px',
      borderRadius: 8,
      background: T.surface,
      border: `1px solid ${T.border}`,
    }}
  >
    <span
      style={{
        width: 18,
        height: 18,
        borderRadius: '50%',
        background: done ? '#28c840' : T.surface2,
        border: `1px solid ${done ? '#28c840' : T.border}`,
        display: 'grid',
        placeItems: 'center',
        flexShrink: 0,
      }}
    >
      {done && <Icon name="check" size={11} color="#fff" />}
    </span>
    <span
      style={{
        flex: 1,
        fontFamily: fonts.ui,
        fontSize: 12.5,
        color: done ? T.textDim : T.text,
        lineHeight: 1.4,
        textDecoration: done ? 'line-through' : 'none',
      }}
    >
      {label}
    </span>
    <Avatar name={owner} size={22} color={color} />
  </div>
);

const RewriteScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow nav={<Navigator active="product" />} title="Otto — Product">
        <div style={{ display: 'flex', height: '100%', overflow: 'hidden' }}>
          {/* Left: Spec rewrite */}
          <div
            style={{
              flex: 1,
              padding: 18,
              display: 'flex',
              flexDirection: 'column',
              gap: 8,
              overflow: 'hidden',
              borderRight: `1px solid ${T.border}`,
            }}
          >
            <Appear delay={6} y={10}>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 11.5,
                  fontWeight: 600,
                  letterSpacing: 1.5,
                  textTransform: 'uppercase' as const,
                  color: T.textDim,
                  marginBottom: 4,
                }}
              >
                Rewritten Spec
              </div>
            </Appear>
            <Stagger
              delay={12}
              step={12}
              y={14}
              style={{ display: 'flex', flexDirection: 'column', gap: 8 }}
            >
              {SPEC_SECTIONS.map((s) => (
                <Card key={s.title} t={T} pad={12}>
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 11.5,
                      fontWeight: 700,
                      color: T.accent,
                      marginBottom: 5,
                    }}
                  >
                    {s.title}
                  </div>
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 12,
                      color: T.text,
                      lineHeight: 1.6,
                      whiteSpace: 'pre-line',
                    }}
                  >
                    {s.body}
                  </div>
                </Card>
              ))}
            </Stagger>
          </div>

          {/* Right: Task plan */}
          <div
            style={{
              width: 370,
              flexShrink: 0,
              padding: 18,
              display: 'flex',
              flexDirection: 'column',
              gap: 8,
              overflow: 'hidden',
            }}
          >
            <Appear delay={8} y={10}>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 11.5,
                  fontWeight: 600,
                  letterSpacing: 1.5,
                  textTransform: 'uppercase' as const,
                  color: T.textDim,
                  marginBottom: 4,
                }}
              >
                Task Plan · 5 tasks
              </div>
            </Appear>
            <Stagger
              delay={18}
              step={10}
              y={14}
              style={{ display: 'flex', flexDirection: 'column', gap: 8 }}
            >
              {TASKS.map((task) => (
                <TaskRow key={task.label} {...task} />
              ))}
            </Stagger>
            <Appear delay={82} y={10} style={{ marginTop: 4 }}>
              <Button variant="primary" icon="grid">
                Send to Swarm
              </Button>
            </Appear>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="A build-ready rewrite and a task plan — hand it to a swarm"
      sub="Every section is editable · tasks get owners · one click sends them to an Agent Swarm"
      delay={16}
    />
  </>
);

// ── Scene 4 — Discovery + Inject ──────────────────────────────────────────────

const DiscoveryScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow nav={<Navigator active="product" />} title="Otto — Product">
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            height: '100%',
            padding: 20,
            gap: 14,
            boxSizing: 'border-box',
            overflow: 'hidden',
          }}
        >
          {/* Discovery draft card */}
          <Appear delay={6} y={18}>
            <Card t={T} pad={16}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 12 }}>
                <Icon name="note" size={16} color={brand.violet} />
                <span
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 15,
                    fontWeight: 700,
                    color: T.text,
                    flex: 1,
                  }}
                >
                  Discovery: rate-limit strategy
                </span>
                <Chip tone="default">Draft</Chip>
                <Button size="s" icon="ticket">
                  Publish as Jira story
                </Button>
                <Button variant="primary" size="s" icon="send">
                  Publish as RFC
                </Button>
              </div>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 13,
                  color: T.text,
                  lineHeight: 1.7,
                  whiteSpace: 'pre-line',
                }}
              >
                {DISCOVERY_BODY}
              </div>
            </Card>
          </Appear>

          {/* Inject into agent */}
          <Appear delay={52} y={14}>
            <Card t={T} pad={14}>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 11.5,
                  fontWeight: 600,
                  letterSpacing: 1.5,
                  textTransform: 'uppercase' as const,
                  color: T.textDim,
                  marginBottom: 10,
                }}
              >
                Inject into Agent
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <div
                  style={{
                    flex: 1,
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    padding: '8px 12px',
                    borderRadius: 8,
                    background: T.surface2,
                    border: `1px solid ${T.border}`,
                  }}
                >
                  <StatusDot kind="working" size={8} />
                  <span
                    style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 600, color: T.text }}
                  >
                    fix auth tests
                  </span>
                  <span style={{ fontFamily: fonts.mono, fontSize: 11, color: T.textDim }}>
                    claude · working
                  </span>
                </div>
                <Button variant="primary" icon="send">
                  Inject story context ▸
                </Button>
              </div>
            </Card>
          </Appear>

          {/* Toast confirmation */}
          <Toast
            text="Story context injected into fix auth tests"
            tone="ok"
            delay={84}
            style={{ alignSelf: 'flex-end' }}
          />
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="Discovery → RFC or story · inject refined context into any agent"
      sub="Start blank or paste call transcripts · publish to Jira or Confluence · inject into a running session"
      delay={16}
    />
  </>
);

// ── Scene list ────────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 75,  node: <TitleScene />,     name: 'Title'              },
  { dur: 180, node: <AnalyzeScene />,   name: 'Import & Analyze'   },
  { dur: 155, node: <RewriteScene />,   name: 'Rewrite & Plan'     },
  { dur: 115, node: <DiscoveryScene />, name: 'Discovery & Inject' },
  {
    dur: 125,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Product — Jira & Confluence"
        tagline="From ticket to build-ready spec, then into a coding agent"
        pills={[
          { label: 'Analyze · Ask · Rewrite', icon: 'note'    },
          { label: 'Test cases → Confluence', icon: 'check'   },
          { label: 'Plan → Swarm',            icon: 'grid'    },
          { label: 'Discovery → RFC/story',   icon: 'comment' },
        ]}
      />
    ),
  },
];

export const productDuration = scenesDuration(SCENES);
export const Product: React.FC = () => <Scenes scenes={SCENES} />;
