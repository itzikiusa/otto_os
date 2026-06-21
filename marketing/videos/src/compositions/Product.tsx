import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, providers, status, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Stagger,
  track,
  TitleCard,
  Caption,
  Button,
  Chip,
  Card,
  StatusDot,
  Terminal,
  Diff,
  Toast,
  Cursor,
  Icon,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  PRODUCT — Jira / Confluence multi-agent story workflows
// ════════════════════════════════════════════════════════════════════════════

// Small section header used inside panels.
const PanelHead: React.FC<{ icon: string; title: string; right?: React.ReactNode }> = ({ icon, title, right }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 9,
      padding: '0 14px',
      height: 40,
      borderBottom: `1px solid ${T.border}`,
      flexShrink: 0,
    }}
  >
    <Icon name={icon} size={15} color={T.textDim} />
    <span style={{ fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 700, color: T.text, flex: 1 }}>{title}</span>
    {right}
  </div>
);

// A source chip (Jira / Confluence / Draft) tinted to match the source.
const SourceChip: React.FC<{ kind: 'Jira' | 'Confluence' | 'Draft' }> = ({ kind }) => {
  const color = kind === 'Jira' ? '#2684ff' : kind === 'Confluence' ? '#1c8ad6' : T.textDim;
  return (
    <Chip color={kind === 'Draft' ? undefined : color}>
      <Icon name={kind === 'Draft' ? 'edit' : 'note'} size={11} />
      {kind}
    </Chip>
  );
};

// ── Scene 1 — title card ──────────────────────────────────────────────────────
const TitleScene: React.FC = () => (
  <TitleCard
    kicker="PRODUCT · JIRA / CONFLUENCE"
    title="Turn a ticket into a plan"
    subtitle="Import a story → multi-agent analysis → tasks your agents pick up"
  />
);

// ── Scene 2 — story import ────────────────────────────────────────────────────
type Story = { key: string; title: string; source: 'Jira' | 'Confluence' | 'Draft'; active?: boolean };
const STORIES: Story[] = [
  { key: 'SIN-4821', title: 'Add JWT validation to protected routes', source: 'Jira', active: true },
  { key: 'SIN-4790', title: 'Rate-limit the login endpoint', source: 'Jira' },
  { key: 'CONF-212', title: 'Auth RFC — session strategy', source: 'Confluence' },
  { key: 'DRAFT-3', title: 'Idea: passkey sign-in flow', source: 'Draft' },
];

const StoryRow: React.FC<{ s: Story }> = ({ s }) => (
  <div
    style={{
      display: 'flex',
      flexDirection: 'column',
      gap: 6,
      padding: '11px 12px',
      borderRadius: 9,
      background: s.active ? alpha(brand.purple, 0.12) : 'transparent',
      border: `1px solid ${s.active ? alpha(brand.purple, 0.4) : 'transparent'}`,
    }}
  >
    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
      <span style={{ fontFamily: fonts.mono, fontSize: 12.5, fontWeight: 700, color: s.active ? brand.cyan : T.textDim }}>
        {s.key}
      </span>
      <span style={{ flex: 1 }} />
      <SourceChip kind={s.source} />
    </div>
    <span
      style={{
        fontFamily: fonts.ui,
        fontSize: 13,
        fontWeight: s.active ? 650 : 500,
        color: s.active ? T.text : alpha(T.text, 0.82),
        lineHeight: 1.3,
      }}
    >
      {s.title}
    </span>
  </div>
);

const ImportScene: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow nav={<Navigator active="product" />} title="Otto — Product">
          <div style={{ display: 'flex', height: '100%', boxSizing: 'border-box' }}>
            {/* stories sidebar */}
            <div
              style={{
                width: 392,
                flexShrink: 0,
                borderRight: `1px solid ${T.border}`,
                display: 'flex',
                flexDirection: 'column',
                background: T.bgSidebar,
              }}
            >
              <PanelHead
                icon="note"
                title="Stories"
                right={<Chip color={brand.cyan}>4</Chip>}
              />
              <Stagger delay={10} step={6} y={12} style={{ padding: 10, display: 'flex', flexDirection: 'column', gap: 6 }}>
                {STORIES.map((s) => (
                  <StoryRow key={s.key} s={s} />
                ))}
              </Stagger>
            </div>

            {/* opened story card */}
            <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}>
              <PanelHead
                icon="ticket"
                title="SIN-4821"
                right={
                  <div style={{ position: 'relative' }}>
                    <Button variant="primary" icon="refresh">Import / Refresh</Button>
                  </div>
                }
              />
              <div style={{ flex: 1, padding: 22, display: 'flex', flexDirection: 'column', gap: 16, minHeight: 0 }}>
                <Appear delay={16} y={14}>
                  <div style={{ fontFamily: fonts.ui, fontSize: 24, fontWeight: 750 as never, color: T.text, letterSpacing: -0.4 }}>
                    Add JWT validation to protected routes
                  </div>
                </Appear>
                <Appear delay={22}>
                  <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                    <SourceChip kind="Jira" />
                    <Chip tone="warn">In Progress</Chip>
                    <Chip color={brand.violet}>auth</Chip>
                    <Chip color={brand.violet}>security</Chip>
                    <Chip>P1</Chip>
                  </div>
                </Appear>
                <Appear delay={28}>
                  <Card pad={18} style={{ background: T.surface }}>
                    <div style={{ fontFamily: fonts.ui, fontSize: 12, fontWeight: 600, color: T.textDim, marginBottom: 8, textTransform: 'uppercase', letterSpacing: 0.4 }}>
                      Summary
                    </div>
                    <div style={{ fontFamily: fonts.ui, fontSize: 15, lineHeight: 1.6, color: alpha(T.text, 0.9) }}>
                      Every route under <span style={{ fontFamily: fonts.mono, color: brand.cyan }}>/api/v2/*</span> must
                      verify a signed JWT before handling the request. Reject expired or tampered tokens with{' '}
                      <span style={{ fontFamily: fonts.mono, color: brand.cyan }}>401</span>, and surface the failing claim
                      in structured logs.
                    </div>
                  </Card>
                </Appear>
                <Appear delay={34}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 10, color: T.textDim, fontFamily: fonts.ui, fontSize: 13 }}>
                    <Icon name="link" size={14} color={T.textDim} />
                    Linked: CONF-212 · Auth RFC — session strategy
                  </div>
                </Appear>
              </div>
            </div>
          </div>
          {/* cursor drives the Import button, then a toast confirms */}
          <Cursor from={[820, 560]} to={[1392, 86]} startAt={42} duration={26} click />
          {track(frame, [74, 75], [0, 1]) > 0 && (
            <div style={{ position: 'absolute', top: 100, right: 28 }}>
              <Toast text="Imported from Jira — synced 4s ago" tone="ok" delay={74} />
            </div>
          )}
        </OttoWindow>
      </Stage>
      <Caption
        step={1}
        title="Import from Jira or Confluence"
        sub="Stories, RFCs & drafts in one place — context stays in sync"
      />
    </>
  );
};

// ── Scene 3 — multi-agent analysis ────────────────────────────────────────────
const AgentTile: React.FC<{
  name: string;
  provider: keyof typeof providers;
  lines: { text: string; tone?: 'cmd' | 'ok' | 'dim' | 'text' | 'accent' }[];
  delay: number;
}> = ({ name, provider, lines, delay }) => (
  <Appear delay={delay} y={18} style={{ flex: 1, minWidth: 0, display: 'flex' }}>
    <div
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        background: T.termBg,
        border: `1px solid ${T.border}`,
        borderRadius: 11,
        overflow: 'hidden',
        boxShadow: `0 1px 0 ${alpha(providers[provider], 0.4)} inset`,
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '9px 12px',
          borderBottom: `1px solid ${T.border}`,
          background: alpha(providers[provider], 0.07),
        }}
      >
        <StatusDot kind="working" size={8} />
        <span style={{ flex: 1, fontFamily: fonts.mono, fontSize: 12.5, fontWeight: 600, color: T.text }}>{name}</span>
        <Chip color={providers[provider]}>{provider}</Chip>
      </div>
      <Terminal
        lines={lines as never}
        delay={delay + 10}
        step={8}
        pad={12}
        fontSize={12.5}
        style={{ flex: 1, background: 'transparent', borderRadius: 0, lineHeight: 1.6 }}
      />
    </div>
  </Appear>
);

const AnalysisScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="product" />}
        tabs={[{ label: 'SIN-4821 · Analysis', icon: 'note', active: true, dot: 'working' }]}
        title="Otto — Product · multi-agent analysis"
      >
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%', padding: 16, gap: 12, boxSizing: 'border-box' }}>
          {/* three tiled provider sessions */}
          <div style={{ display: 'flex', gap: 12, flex: 1, minHeight: 0 }}>
            <AgentTile
              name="claude · analyze"
              provider="claude"
              delay={8}
              lines={[
                { text: '$ analyze SIN-4821', tone: 'cmd' },
                { text: 'reading ticket + CONF-212…', tone: 'dim' },
                { text: 'risk: no exp/iss check', tone: 'text' },
                { text: 'risk: error leaks claim', tone: 'text' },
                { text: '✓ 4 findings', tone: 'ok' },
              ]}
            />
            <AgentTile
              name="codex · analyze"
              provider="codex"
              delay={14}
              lines={[
                { text: '$ analyze SIN-4821', tone: 'cmd' },
                { text: 'scanning api/v2 routes…', tone: 'dim' },
                { text: '12 routes unguarded', tone: 'text' },
                { text: 'suggest: shared middleware', tone: 'text' },
                { text: '✓ 3 findings', tone: 'ok' },
              ]}
            />
            <AgentTile
              name="gemini · analyze"
              provider="gemini"
              delay={20}
              lines={[
                { text: '$ analyze SIN-4821', tone: 'cmd' },
                { text: 'checking test coverage…', tone: 'dim' },
                { text: 'no negative-path tests', tone: 'text' },
                { text: 'open Q: refresh tokens?', tone: 'accent' },
                { text: '✓ 2 findings', tone: 'ok' },
              ]}
            />
          </div>

          {/* summarizer consolidation */}
          <Appear delay={44} y={16} style={{ flexShrink: 0 }}>
            <Card pad={0} style={{ background: T.surface, border: `1px solid ${alpha(brand.cyan, 0.4)}` }}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 9,
                  padding: '10px 14px',
                  borderBottom: `1px solid ${T.border}`,
                }}
              >
                <Icon name="zap" size={15} color={brand.cyan} />
                <span style={{ fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 700, color: T.text, flex: 1 }}>
                  Summarizer · consolidated findings
                </span>
                <Chip color={brand.cyan}>9 → 5 deduped</Chip>
              </div>
              <Stagger delay={54} step={6} y={8} style={{ padding: '12px 16px', display: 'flex', flexDirection: 'column', gap: 7 }}>
                {[
                  'Add shared JWT middleware across all 12 /api/v2 routes',
                  'Verify exp + iss claims; reject tampered tokens with 401',
                  'Stop leaking the failing claim in error responses',
                  'Add negative-path tests (expired, wrong issuer, no token)',
                  'Open question: do we rotate refresh tokens here?',
                ].map((b, i) => (
                  <div key={i} style={{ display: 'flex', alignItems: 'flex-start', gap: 9 }}>
                    <span style={{ marginTop: 6, width: 6, height: 6, borderRadius: '50%', background: brand.cyan, flexShrink: 0 }} />
                    <span style={{ fontFamily: fonts.ui, fontSize: 13.5, color: alpha(T.text, 0.92), lineHeight: 1.4 }}>{b}</span>
                  </div>
                ))}
              </Stagger>
            </Card>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Analyzed by a panel of agents"
      sub="Fan out across providers · a summarizer consolidates"
    />
  </>
);

// ── Scene 4 — plan → tasks ────────────────────────────────────────────────────
type Task = { label: string; provider: keyof typeof providers; status: 'done' | 'working' | 'queued' };
const TASKS: Task[] = [
  { label: 'Add JWT middleware to /api/v2', provider: 'claude', status: 'done' },
  { label: 'Validate exp / iss claims', provider: 'codex', status: 'working' },
  { label: 'Return 401 on invalid token', provider: 'claude', status: 'working' },
  { label: 'Unit + integration tests', provider: 'gemini', status: 'queued' },
  { label: 'Update OpenAPI spec', provider: 'codex', status: 'queued' },
];

const TaskRow: React.FC<{ t: Task }> = ({ t }) => {
  const checked = t.status === 'done';
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        padding: '13px 16px',
        borderRadius: 10,
        background: T.surface,
        border: `1px solid ${T.border}`,
      }}
    >
      <span
        style={{
          width: 20,
          height: 20,
          borderRadius: 6,
          flexShrink: 0,
          display: 'grid',
          placeItems: 'center',
          background: checked ? status.working : 'transparent',
          border: `1.5px solid ${checked ? status.working : T.border}`,
        }}
      >
        {checked && <Icon name="check" size={13} color="#fff" />}
      </span>
      <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 15, fontWeight: 550 as never, color: T.text }}>{t.label}</span>
      {t.status === 'working' && <StatusDot kind="working" size={9} />}
      {t.status === 'queued' && <Chip>queued</Chip>}
      <Chip color={providers[t.provider]}>{t.provider}</Chip>
    </div>
  );
};

const PlanScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="product" />}
        tabs={[{ label: 'SIN-4821 · Plan', icon: 'split', active: true, dot: 'working' }]}
        title="Otto — Product · plan"
      >
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
          <PanelHead
            icon="split"
            title="Plan · 5 tasks"
            right={
              <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
                <Chip color={brand.violet}>
                  <Icon name="zap" size={11} />
                  Autonomous
                </Chip>
                <Button variant="primary" icon="grid">Create Swarm project</Button>
              </div>
            }
          />
          <div style={{ flex: 1, padding: 22, display: 'flex', flexDirection: 'column', gap: 18, minHeight: 0 }}>
            <Appear delay={10}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, fontFamily: fonts.ui, fontSize: 13, color: T.textDim }}>
                <StatusDot kind="working" size={8} />
                Multi-agent planning · broke the story into tasks and assigned providers
              </div>
            </Appear>
            <Stagger delay={18} step={9} y={16} style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
              {TASKS.map((t) => (
                <TaskRow key={t.label} t={t} />
              ))}
            </Stagger>
            <Appear delay={70} style={{ marginTop: 'auto' }}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 12,
                  padding: '14px 18px',
                  borderRadius: 12,
                  background: alpha(brand.violet, 0.1),
                  border: `1px solid ${alpha(brand.violet, 0.4)}`,
                }}
              >
                <Icon name="grid" size={18} color={brand.violet} />
                <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 14, color: T.text }}>
                  Push straight into a <b>Swarm</b> — role agents pick up tasks and run them.
                </span>
                <Button variant="default" icon="send">Send to agent</Button>
              </div>
            </Appear>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="Break it into tasks — agents pick them up"
      sub="Multi-agent planning · push straight into a Swarm"
    />
  </>
);

// ── Scene 5 — rewrite + test cases + learnings ────────────────────────────────
const REWRITE: { text: string; kind?: 'add' | 'del' | 'ctx' | 'hunk' }[] = [
  { text: 'Acceptance criteria', kind: 'hunk' },
  { text: 'Protected routes require a token.', kind: 'del' },
  { text: 'Reject bad tokens.', kind: 'del' },
  { text: 'All /api/v2/* routes verify a signed JWT.', kind: 'add' },
  { text: 'exp + iss claims validated; clock-skew 60s.', kind: 'add' },
  { text: 'Invalid/expired token → 401, no claim leak.', kind: 'add' },
  { text: 'Failing claim recorded in structured logs.', kind: 'add' },
];

const TEST_CASES: { g: string; w: string; t: string }[] = [
  { g: 'a request with a valid JWT', w: 'it hits /api/v2/orders', t: 'the handler runs, 200' },
  { g: 'a request with an expired JWT', w: 'it hits any protected route', t: 'the server returns 401' },
  { g: 'a tampered signature', w: 'the token is verified', t: '401, claim not echoed back' },
  { g: 'no Authorization header', w: 'the route is protected', t: '401 with WWW-Authenticate' },
];

const TestCaseRow: React.FC<{ c: { g: string; w: string; t: string } }> = ({ c }) => (
  <div
    style={{
      display: 'flex',
      flexDirection: 'column',
      gap: 3,
      padding: '10px 13px',
      borderRadius: 9,
      background: T.surface,
      border: `1px solid ${T.border}`,
    }}
  >
    <Gwt k="Given" v={c.g} color={T.textDim} />
    <Gwt k="When" v={c.w} color={brand.cyan} />
    <Gwt k="Then" v={c.t} color={status.working} />
  </div>
);

const Gwt: React.FC<{ k: string; v: string; color: string }> = ({ k, v, color }) => (
  <div style={{ display: 'flex', gap: 8, alignItems: 'baseline' }}>
    <span style={{ fontFamily: fonts.mono, fontSize: 11, fontWeight: 700, color, width: 44, flexShrink: 0 }}>{k}</span>
    <span style={{ fontFamily: fonts.ui, fontSize: 12.5, color: alpha(T.text, 0.9), lineHeight: 1.35 }}>{v}</span>
  </div>
);

const RewriteScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="product" />}
        tabs={[{ label: 'SIN-4821 · Rewrite & tests', icon: 'edit', active: true }]}
        title="Otto — Product · rewrite"
      >
        <div style={{ display: 'flex', height: '100%', minHeight: 0 }}>
          {/* suggested rewrite */}
          <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', borderRight: `1px solid ${T.border}` }}>
            <PanelHead icon="edit" title="Suggested rewrite" right={<Chip color={providers.claude}>AI polish</Chip>} />
            <div style={{ padding: 18, flex: 1, minHeight: 0 }}>
              <Appear delay={12}>
                <Diff lines={REWRITE} delay={18} step={5} fontSize={13} style={{ height: '100%' }} />
              </Appear>
            </div>
          </div>

          {/* generated test cases */}
          <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}>
            <PanelHead
              icon="check"
              title="Generated test cases"
              right={<Button variant="primary" icon="note">Approve & publish to Confluence</Button>}
            />
            <Stagger delay={26} step={8} y={12} style={{ padding: 18, display: 'flex', flexDirection: 'column', gap: 9, flex: 1, minHeight: 0 }}>
              {TEST_CASES.map((c, i) => (
                <TestCaseRow key={i} c={c} />
              ))}
            </Stagger>
          </div>
        </div>
        {/* learnings chip row across the bottom */}
        <Appear delay={64} style={{ position: 'absolute', left: 18, right: 18, bottom: 14 }}>
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 10,
              padding: '9px 14px',
              borderRadius: 10,
              background: T.surface,
              border: `1px solid ${T.border}`,
              boxShadow: T.shadow,
            }}
          >
            <Icon name="tag" size={14} color={brand.violet} />
            <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 600, color: T.textDim, marginRight: 4 }}>
              Learnings
            </span>
            <Chip color={brand.violet}>always verify iss</Chip>
            <Chip color={brand.violet}>never echo claims</Chip>
            <Chip color={brand.violet}>shared auth middleware</Chip>
            <span style={{ flex: 1 }} />
            <Chip tone="warn">
              <Icon name="comment" size={11} />2 open questions → comments
            </Chip>
          </div>
        </Appear>
      </OttoWindow>
    </Stage>
    <Caption
      step={4}
      title="Rewrites, test cases & learnings — written for you"
      sub="Approve and publish back to Confluence; open questions post as comments"
    />
  </>
);

// ── Scene 6 — outro ───────────────────────────────────────────────────────────
const OutroScene: React.FC = () => (
  <WalkOutro
    title="Product Workflows"
    tagline="From ticket to tasks, with agents."
    pills={[
      { label: 'Jira & Confluence', color: '#2684ff', icon: 'note' },
      { label: 'Multi-agent analysis', color: providers.claude, icon: 'grid' },
      { label: 'Plan → tasks', color: brand.cyan, icon: 'split' },
      { label: 'Test cases', color: '#28c840', icon: 'check' },
      { label: '→ Swarm', color: brand.violet, icon: 'grid' },
    ]}
  />
);

// ── sequence ──────────────────────────────────────────────────────────────────
const SCENES: SceneDef[] = [
  { dur: 80, node: <TitleScene />, name: 'Title' },
  { dur: 210, node: <ImportScene />, name: 'Import' },
  { dur: 220, node: <AnalysisScene />, name: 'Analysis' },
  { dur: 210, node: <PlanScene />, name: 'Plan' },
  { dur: 230, node: <RewriteScene />, name: 'Rewrite' },
  { dur: 130, node: <OutroScene />, name: 'Outro' },
];

export const productDuration = scenesDuration(SCENES);
export const Product: React.FC = () => <Scenes scenes={SCENES} />;
