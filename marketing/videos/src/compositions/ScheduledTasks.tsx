import React from 'react';
import { T, brand, fonts, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Stagger,
  Caption,
  TitleCard,
  Toast,
  Chip,
  Field,
  Button,
  Segmented,
  Table,
  Terminal,
  TermLine,
  Icon,
} from '../components/kit';

// ── Scene 1 — Title card ─────────────────────────────────────────────────────
const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Scheduled Tasks"
    title="Agent Jobs on Autopilot"
    subtitle="Define once. Run on a schedule. Wake up to the report."
  />
);

// ── Scene 2 — Define a recurring job ────────────────────────────────────────
const DefineScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={<Navigator active="scheduled-tasks" />}
        title="Otto — Scheduled Tasks"
        width={1200}
        height={740}
      >
        <div style={{ padding: '22px 28px', display: 'flex', flexDirection: 'column', gap: 18 }}>

          {/* Section header */}
          <Appear delay={4} y={14}>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontFamily: fonts.ui, fontSize: 16, fontWeight: 700, color: T.text }}>
                <Icon name="plus" size={15} color={T.accent} />
                New Scheduled Job
              </div>
              <Button variant="primary" icon="clock">Save & Activate</Button>
            </div>
          </Appear>

          {/* Job name */}
          <Appear delay={14} y={14}>
            <Field label="Job name" value="Daily PR & incident digest" icon="file" />
          </Appear>

          {/* Schedule row */}
          <Appear delay={24} y={14}>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 500, color: T.textDim }}>Schedule</span>
              <div style={{ display: 'flex', gap: 14, alignItems: 'center' }}>
                <Segmented options={['Interval', 'Daily', 'Weekly']} active={1} />
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 7,
                    padding: '0 12px',
                    height: 28,
                    borderRadius: 6,
                    background: T.surface2,
                    border: `1px solid ${T.border}`,
                    fontFamily: fonts.mono,
                    fontSize: 13.5,
                    color: T.text,
                  }}
                >
                  <Icon name="clock" size={13} color={T.textDim} />
                  09:00 UTC
                </div>
                <span style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>every day</span>
              </div>
            </div>
          </Appear>

          {/* Agent prompt textarea */}
          <Appear delay={36} y={14}>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 500, color: T.textDim }}>Agent prompt</span>
              <div
                style={{
                  background: T.surface2,
                  border: `1px solid ${T.accent}`,
                  borderRadius: 8,
                  padding: '11px 14px',
                  minHeight: 90,
                  fontFamily: fonts.ui,
                  fontSize: 14,
                  color: T.text,
                  lineHeight: 1.65,
                  boxShadow: `0 0 0 3px ${alpha(T.accent, 0.22)}`,
                }}
              >
                Summarise all PRs merged in the last 24 h and any open incidents from PagerDuty.
                Group by team, highlight blockers, list action items.
              </div>
            </div>
          </Appear>

          {/* Delivery targets */}
          <Appear delay={50} y={14}>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 500, color: T.textDim }}>Deliver to</span>
              <Stagger delay={54} step={6} style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'center' }}>
                <Chip color="#36c5f0">
                  <Icon name="slack" size={11} color="#36c5f0" /> Slack #eng
                </Chip>
                <Chip color={brand.cyan}>
                  <Icon name="send" size={11} color={brand.cyan} /> lead@acme.dev
                </Chip>
                <Chip>
                  <Icon name="plus" size={11} /> Add channel
                </Chip>
              </Stagger>
            </div>
          </Appear>

          {/* Redaction notice */}
          <Appear delay={72} y={12}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 9,
                padding: '9px 13px',
                borderRadius: 8,
                background: alpha(T.accent, 0.08),
                border: `1px solid ${alpha(T.accent, 0.22)}`,
              }}
            >
              <Icon name="eye" size={13} color={T.accent} />
              <span style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>
                Secrets and API keys are automatically redacted before delivery.
              </span>
            </div>
          </Appear>

        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Recurring agent jobs — interval, daily, or weekly"
      sub="Name the job, pick the cadence, write the prompt, add delivery targets."
    />
  </>
);

// ── Scene 3 — Markdown report + delivery confirmation ────────────────────────
const REPORT_LINES: TermLine[] = [
  { text: '## Daily Digest — 2026-06-26', tone: 'cmd' },
  { text: '' },
  { text: '### Pull Requests Merged (last 24 h)', tone: 'text' },
  { text: '  • feat/scheduled-tasks      3 approvals · merged 08:14', tone: 'ok' },
  { text: '  • fix/auth-jwt-expiry       closed INC-8812 (P1) ✓', tone: 'ok' },
  { text: '  • chore/bump-deps           12 packages updated', tone: 'dim' },
  { text: '' },
  { text: '### Open Incidents', tone: 'text' },
  { text: '  • INC-8814 — API latency spike (P2)   ETA 14:00 UTC', tone: 'warn' },
  { text: '' },
  { text: '### Action Items', tone: 'text' },
  { text: '  1. #eng sign-off on latency runbook  (owner: @dana)', tone: 'dim' },
  { text: '  2. Tag INC-8814 resolved once p99 < 200 ms', tone: 'dim' },
];

const ReportScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={<Navigator active="scheduled-tasks" />}
        title="Otto — Scheduled Tasks"
        width={1200}
        height={740}
      >
        <div style={{ padding: '22px 28px', display: 'flex', flexDirection: 'column', gap: 14 }}>

          {/* Run header */}
          <Appear delay={4} y={12}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              <span style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 700, color: T.text }}>
                Daily PR &amp; incident digest
              </span>
              <Chip tone="ok">✓ Completed</Chip>
              <span style={{ fontFamily: fonts.mono, fontSize: 12, color: T.textDim }}>
                2026-06-26 · 09:00 UTC · 14 s
              </span>
            </div>
          </Appear>

          {/* Markdown report */}
          <Appear delay={10} y={14}>
            <Terminal
              lines={REPORT_LINES}
              delay={14}
              step={8}
              fontSize={13.5}
              style={{ minHeight: 400 }}
            />
          </Appear>

          {/* Delivery toast — absolutely positioned so it floats over the content */}
          <Toast
            text="Delivered to Slack #eng & email · secrets redacted"
            tone="ok"
            delay={86}
            style={{ position: 'absolute', bottom: 22, right: 26, zIndex: 10 }}
          />

        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Each run writes a Markdown report"
      sub="Delivered to Slack, Telegram, email or webhook — secrets redacted on delivery."
    />
  </>
);

// ── Scene 4 — Runs history + MCP tools note ──────────────────────────────────
const RunsScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={<Navigator active="scheduled-tasks" />}
        title="Otto — Scheduled Tasks"
        width={1200}
        height={740}
      >
        <div style={{ padding: '22px 28px', display: 'flex', flexDirection: 'column', gap: 16 }}>

          {/* Job header + controls */}
          <Appear delay={4} y={12}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <span style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 700, color: T.text }}>
                Daily PR &amp; incident digest
              </span>
              <Chip tone="ok">Active</Chip>
              <span style={{ fontFamily: fonts.mono, fontSize: 12, color: T.textDim }}>
                Daily · 09:00 UTC
              </span>
              <div style={{ flex: 1 }} />
              <Button variant="ghost" icon="play">Run now</Button>
              <Button variant="ghost" icon="edit">Edit</Button>
            </div>
          </Appear>

          {/* Runs history table */}
          <Appear delay={14} y={14}>
            <Table
              columns={['Ran at', 'Status', 'Duration', 'Delivered to']}
              rows={[
                [
                  '2026-06-26 · 09:00',
                  <Chip tone="ok">✓ Success</Chip>,
                  '14 s',
                  'Slack #eng · lead@acme.dev',
                ],
                [
                  '2026-06-25 · 09:00',
                  <Chip tone="ok">✓ Success</Chip>,
                  '11 s',
                  'Slack #eng · lead@acme.dev',
                ],
                [
                  '2026-06-24 · 09:00',
                  <Chip tone="bad">✗ Error</Chip>,
                  '—',
                  'Delivery failed — retried 3×',
                ],
              ]}
              widths={['2fr', '1.2fr', '1fr', '2.5fr']}
              delay={20}
              step={16}
              fontSize={13}
            />
          </Appear>

          {/* MCP tools callout */}
          <Appear delay={70} y={12}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                padding: '11px 15px',
                borderRadius: 9,
                background: alpha(brand.violet, 0.1),
                border: `1px solid ${alpha(brand.violet, 0.3)}`,
              }}
            >
              <Icon name="zap" size={14} color={brand.violet} />
              <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim, flex: 1 }}>
                <span style={{ color: brand.violet, fontFamily: fonts.mono, fontWeight: 600 }}>7 otto.*</span>
                {' '}MCP tools — create, run, pause, list, and inspect jobs from any agent or workflow.
              </span>
            </div>
          </Appear>

        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="Browse run history · manage jobs from MCP tools"
      sub="Re-run, pause, or delete any job. Seven otto.* MCP tools for programmatic control."
    />
  </>
);

// ── Scene list ────────────────────────────────────────────────────────────────
const SCENES: SceneDef[] = [
  { dur: 80,  node: <TitleScene />,  name: 'Title' },
  { dur: 180, node: <DefineScene />, name: 'Define Job' },
  { dur: 160, node: <ReportScene />, name: 'Report & Deliver' },
  { dur: 130, node: <RunsScene />,   name: 'Runs & MCP' },
  {
    dur: 130,
    node: (
      <WalkOutro
        title="Scheduled Tasks"
        tagline="Set an agent loose on a schedule — wake up to the report"
        pills={[
          { label: 'Interval / Daily / Weekly', icon: 'clock' },
          { label: 'Markdown report',           icon: 'file'  },
          { label: 'Slack · TG · email · webhook', icon: 'send' },
          { label: 'Redacted',                  icon: 'eye'   },
        ]}
      />
    ),
    name: 'Outro',
  },
];

export const scheduledTasksDuration = scenesDuration(SCENES);
export const ScheduledTasks: React.FC = () => <Scenes scenes={SCENES} />;
