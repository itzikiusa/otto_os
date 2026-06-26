import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, alpha, status } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Stagger,
  Caption,
  TitleCard,
  Chip,
  Button,
  Card,
  Table,
  Terminal,
  TermLine,
  Icon,
  StatusDot,
  track,
} from '../components/kit';

// ─── Scene 1 — Title ──────────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="MCP Control Plane"
    title="MCP Control Plane"
    subtitle="Govern every tool call your agents make — and expose Otto itself as a secure MCP server."
  />
);

// ─── Scene 2 — Outbound Governance ───────────────────────────────────────────

type PipelineStage = {
  label: string;
  icon: string;
  tone: 'ok' | 'warn' | 'default';
};

const PIPELINE: PipelineStage[] = [
  { label: 'Allowlist', icon: 'key',   tone: 'ok'      },
  { label: 'Policy',   icon: 'gear',  tone: 'ok'      },
  { label: 'Approval', icon: 'check', tone: 'warn'    },
  { label: 'Dry-run',  icon: 'play',  tone: 'default' },
  { label: 'Audit',    icon: 'eye',   tone: 'default' },
  { label: 'Stats',    icon: 'chart', tone: 'default' },
];

const PipelineRow: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
      {PIPELINE.map((stage, i) => {
        const op = track(frame, [i * 6, i * 6 + 10], [0, 1]);
        const color =
          stage.tone === 'ok'   ? status.working  :
          stage.tone === 'warn' ? status.needsYou :
          T.textDim;
        const isActive = stage.tone === 'warn';
        return (
          <React.Fragment key={stage.label}>
            {i > 0 && (
              <span style={{ color: T.textDim, fontSize: 14, opacity: op, lineHeight: 1, userSelect: 'none' }}>→</span>
            )}
            <div style={{ opacity: op }}>
              <Chip
                tone={stage.tone}
                style={{
                  fontSize: 12.5,
                  boxShadow: isActive ? `0 0 18px ${alpha(status.needsYou, 0.55)}` : 'none',
                }}
              >
                <Icon name={stage.icon} size={11} color={color} />
                &nbsp;{stage.label}
              </Chip>
            </div>
          </React.Fragment>
        );
      })}
    </div>
  );
};

const OutboundScene: React.FC = () => {
  const frame = useCurrentFrame();
  const cardOp  = track(frame, [50, 68],  [0, 1]);
  const cardY   = track(frame, [50, 68],  [18, 0]);
  const btnsOp  = track(frame, [90, 105], [0, 1]);

  return (
    <>
      <Stage scale={0.88}>
        <OttoWindow
          nav={<Navigator active="mcp" />}
          title="Otto — MCP Control Plane"
        >
          <div style={{
            display: 'flex', flexDirection: 'column', height: '100%',
            padding: '20px 24px', gap: 18, boxSizing: 'border-box',
          }}>
            {/* section header */}
            <Appear delay={2}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <Icon name="plug" size={16} color={brand.cyan} />
                <span style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 700, color: T.text }}>
                  Outbound Gateway
                </span>
                <div style={{ flex: 1 }} />
                <span style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim, marginRight: 4 }}>Gateway</span>
                <Chip tone="ok">ON</Chip>
              </div>
            </Appear>

            {/* governance pipeline */}
            <Card>
              <div style={{
                fontFamily: fonts.ui, fontSize: 11, color: T.textDim,
                textTransform: 'uppercase', letterSpacing: 0.06, marginBottom: 10,
              }}>
                Governance Pipeline
              </div>
              <PipelineRow />
            </Card>

            {/* pending approval card */}
            <div style={{ opacity: cardOp, transform: `translateY(${cardY}px)` }}>
              <Card style={{
                border: `1px solid ${alpha(status.needsYou, 0.5)}`,
                boxShadow: `0 8px 32px ${alpha(status.needsYou, 0.18)}`,
              }}>
                {/* card header */}
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 14 }}>
                  <StatusDot kind="needsYou" size={9} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 12, fontWeight: 600, color: status.needsYou }}>
                    Needs Approval
                  </span>
                  <div style={{ flex: 1 }} />
                  <Chip tone="warn">single-use</Chip>
                </div>

                {/* tool call */}
                <div style={{ fontFamily: fonts.mono, fontSize: 14, fontWeight: 700, color: T.text, marginBottom: 12 }}>
                  github.create_issue
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 5, marginBottom: 16 }}>
                  {[
                    { k: 'owner', v: '"myorg"' },
                    { k: 'repo',  v: '"sinatra-api"' },
                    { k: 'title', v: '"fix: JWT token expiry bug — issue #218"' },
                  ].map(({ k, v }) => (
                    <div key={k} style={{ display: 'flex', gap: 14, fontFamily: fonts.mono, fontSize: 13 }}>
                      <span style={{ color: T.textDim, minWidth: 60 }}>{k}</span>
                      <span style={{ color: brand.cyan }}>{v}</span>
                    </div>
                  ))}
                </div>

                {/* action buttons */}
                <div style={{ opacity: btnsOp, display: 'flex', gap: 9, justifyContent: 'flex-end' }}>
                  <Button variant="danger" icon="x">Deny</Button>
                  <Button variant="primary" icon="check">Approve</Button>
                </div>
              </Card>
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={1}
        title="Govern every MCP tool your agents call"
        sub="Allowlist → policy → single-use approval — every outbound call passes the pipeline"
        delay={55}
      />
    </>
  );
};

// ─── Scene 3 — Dry-run + Fail-closed Audit ────────────────────────────────────

const DRY_RUN_LINES: TermLine[] = [
  { text: '$ [DRY-RUN] github.push_branch', tone: 'cmd' },
  { text: '  args:', tone: 'dim' },
  { text: '    owner : "myorg"', tone: 'dim' },
  { text: '    repo  : "sinatra-api"', tone: 'dim' },
  { text: '    branch: "feat/mcp-gateway"', tone: 'dim' },
  { text: '' },
  { text: '  → would push 3 commits to origin/main', tone: 'text' },
  { text: '  → force: false  — no history rewrite', tone: 'text' },
  { text: '' },
  { text: '  ⚑ dry-run only — zero side effects', tone: 'warn' },
  { text: '  awaiting operator approval to execute', tone: 'dim' },
];

const AUDIT_ROWS: (string | React.ReactNode)[][] = [
  ['09:14:22', 'github.create_issue',  <Chip key="a1" tone="ok">allowed</Chip>],
  ['09:14:55', 'slack.post_message',   <Chip key="a2" tone="ok">allowed</Chip>],
  ['09:15:02', 'jira.delete_issue',    <Chip key="a3" tone="bad">denied</Chip>],
  ['09:15:18', 'github.push_branch',   <Chip key="a4" tone="warn">dry-run</Chip>],
  ['09:15:40', 'github.delete_repo',   <Chip key="a5" tone="bad">denied</Chip>],
  ['09:16:01', 'notion.append_block',  <Chip key="a6" tone="ok">allowed</Chip>],
];

const AuditScene: React.FC = () => {
  const frame = useCurrentFrame();
  const failBadgeOp = track(frame, [90, 104], [0, 1]);

  return (
    <>
      <Stage scale={0.88}>
        <OttoWindow
          nav={<Navigator active="mcp" />}
          title="Otto — MCP Control Plane"
        >
          <div style={{ display: 'flex', height: '100%' }}>
            {/* left — dry-run preview */}
            <div style={{
              flex: 1, padding: '18px 20px', display: 'flex', flexDirection: 'column',
              gap: 14, borderRight: `1px solid ${T.border}`,
            }}>
              <Appear delay={4}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <Icon name="play" size={14} color={brand.cyan} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 700, color: T.text }}>
                    Dry-run Preview
                  </span>
                  <div style={{ flex: 1 }} />
                  <Chip tone="warn">sandbox</Chip>
                </div>
              </Appear>
              <Terminal
                lines={DRY_RUN_LINES}
                delay={10}
                step={7}
                style={{ flex: 1 }}
              />
            </div>

            {/* right — audit log */}
            <div style={{
              flex: 1, padding: '18px 20px', display: 'flex', flexDirection: 'column', gap: 14,
            }}>
              <Appear delay={6}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <Icon name="eye" size={14} color={brand.cyan} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 700, color: T.text }}>
                    Audit Log
                  </span>
                  <div style={{ flex: 1 }} />
                  <div style={{ opacity: failBadgeOp }}>
                    <Chip tone="bad">fail-closed</Chip>
                  </div>
                </div>
              </Appear>
              <Table
                columns={['Time', 'Tool', 'Decision']}
                rows={AUDIT_ROWS}
                widths={['90px', '1fr', '90px']}
                delay={22}
                step={8}
                style={{ flex: 1 }}
              />
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={2}
        title="Dry-run risky calls · fail-closed audit"
        sub="Preview would-be side effects · every decision logged — denied by default on error"
        delay={38}
      />
    </>
  );
};

// ─── Scene 4 — Outward otto.* Server ─────────────────────────────────────────

const OTTO_TOOLS: { name: string; desc: string }[] = [
  { name: 'search_codebase',    desc: 'Semantic + keyword search across the repo' },
  { name: 'get_context_packet', desc: 'Assemble a context bundle for a topic' },
  { name: 'run_goal_loop',      desc: 'Kick off a bounded goal loop' },
  { name: 'create_work_item',   desc: 'Create a tracked work item' },
  { name: 'query_db_readonly',  desc: 'Run a read-only SQL query' },
  { name: 'open_pr_draft',      desc: 'Draft a pull request from a branch' },
  { name: 'get_proof_pack',     desc: "Fetch a task's proof pack & status" },
  { name: 'ask_human_approval', desc: 'Request a human approval, then block' },
];

const OutwardScene: React.FC = () => {
  const frame = useCurrentFrame();
  const clientOp = track(frame, [80, 96], [0, 1]);
  const clientY  = track(frame, [80, 96], [14, 0]);

  return (
    <>
      <Stage scale={0.88}>
        <OttoWindow
          nav={<Navigator active="mcp" />}
          title="Otto — MCP Control Plane"
        >
          <div style={{
            display: 'flex', flexDirection: 'column', height: '100%',
            padding: '20px 24px', gap: 18, boxSizing: 'border-box',
          }}>
            {/* header */}
            <Appear delay={2}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <Icon name="send" size={16} color={brand.purple} />
                <span style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 700, color: T.text }}>
                  ottod mcp-server
                </span>
                <div style={{ flex: 1 }} />
                <Chip color={brand.purple}>opt-in gateway</Chip>
                <Chip tone="ok">RUNNING</Chip>
              </div>
            </Appear>

            {/* otto.* tool rows */}
            <div style={{ flex: 1 }}>
              <Stagger delay={12} step={6}>
                {OTTO_TOOLS.map((tool) => (
                  <div
                    key={tool.name}
                    style={{
                      display: 'flex', alignItems: 'center', gap: 12,
                      padding: '9px 14px', borderRadius: 7,
                      background: T.surface, border: `1px solid ${T.border}`,
                      marginBottom: 6,
                    }}
                  >
                    <Icon name="zap" size={13} color={brand.purple} />
                    <span style={{ fontFamily: fonts.mono, fontSize: 13, fontWeight: 600, color: T.text, minWidth: 230 }}>
                      {tool.name}
                    </span>
                    <span style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>
                      {tool.desc}
                    </span>
                  </div>
                ))}
              </Stagger>
            </div>

            {/* external client banner */}
            <div style={{ opacity: clientOp, transform: `translateY(${clientY}px)` }}>
              <div style={{
                display: 'flex', alignItems: 'center', gap: 14,
                padding: '13px 18px', borderRadius: 10,
                background: alpha(brand.purple, 0.09),
                border: `1px solid ${alpha(brand.purple, 0.38)}`,
              }}>
                <Icon name="external" size={15} color={brand.purple} />
                <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 600, color: T.text }}>
                  External MCP Client
                </span>
                <span style={{ fontFamily: fonts.mono, fontSize: 14, color: T.textDim }}>→</span>
                <Chip color={brand.purple}>token kind=mcp (restricted)</Chip>
                <div style={{ flex: 1 }} />
                <StatusDot kind="working" size={8} />
                <span style={{ fontFamily: fonts.ui, fontSize: 12, color: status.working, marginLeft: 4 }}>
                  connected
                </span>
              </div>
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={3}
        title="Expose Otto itself as an MCP server"
        sub="8 otto.* tools behind a restricted kind=mcp token — opt-in gateway, externally accessible"
        delay={38}
      />
    </>
  );
};

// ─── Scenes ───────────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 80,  node: <TitleScene />,    name: 'Title' },
  { dur: 160, node: <OutboundScene />, name: 'Outbound' },
  { dur: 140, node: <AuditScene />,    name: 'DryRunAudit' },
  { dur: 110, node: <OutwardScene />,  name: 'Outward' },
  {
    dur: 130,
    name: 'Outro',
    node: (
      <WalkOutro
        title="MCP Control Plane"
        tagline="Two-way MCP — governed going out, restricted coming in"
        pills={[
          { label: 'Governed outbound',    icon: 'plug'  },
          { label: 'Single-use approval',  icon: 'check' },
          { label: 'Fail-closed audit',    icon: 'eye'   },
          { label: 'Outward otto.* tools', icon: 'send'  },
        ]}
      />
    ),
  },
];

export const mcpDuration = scenesDuration(SCENES);
export const Mcp: React.FC = () => <Scenes scenes={SCENES} />;
