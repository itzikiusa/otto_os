import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Stagger,
  Caption,
  TitleCard,
  Terminal,
  TermLine,
  Table,
  MetricStat,
  BarChart,
  Sparkline,
  Field,
  Card,
  Chip,
  track,
  useTyped,
  Icon,
} from '../components/kit';

// ── Scene 1 — title card ──────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Database Explorer"
    title="Database Explorer"
    subtitle="MySQL · Redis · MongoDB · ClickHouse — with an agent inside."
  />
);

// ── Scene 2 — schema tree + query tab + results grid ─────────────────────────

interface TreeItem {
  depth: number;
  label: string;
  icon: string;
  active?: boolean;
}

const TREE: TreeItem[] = [
  { depth: 0, label: 'users-mysql',             icon: 'db' },
  { depth: 1, label: 'Tables (3)',               icon: 'folder' },
  { depth: 2, label: 'players',                  icon: 'square', active: true },
  { depth: 2, label: 'transactions',             icon: 'square' },
  { depth: 2, label: 'sessions',                 icon: 'square' },
  { depth: 1, label: 'Views',                    icon: 'folder' },
  { depth: 1, label: 'Indexes',                  icon: 'folder' },
];

const SQL_LINES: TermLine[] = [
  { text: "SELECT id, login, status, balance", tone: 'cmd' },
  { text: "FROM   players",                    tone: 'cmd' },
  { text: "WHERE  status = 'active'",          tone: 'cmd' },
  { text: "LIMIT  100;",                       tone: 'cmd' },
  { text: '── 4 rows · 2 ms ─────────────────────────', tone: 'ok' },
];

const TABLE_ROWS: (string | React.ReactNode)[][] = [
  ['1001', 'jsmith',   <Chip tone="ok">active</Chip>, '$4,821.00'],
  ['1002', 'emiller',  <Chip tone="ok">active</Chip>, '$12,300.50'],
  ['1003', 'kwang',    <Chip tone="ok">active</Chip>, '$890.20'],
  ['1004', 'alopez',   <Chip tone="ok">active</Chip>, '$6,102.75'],
];

const SchemaQueryScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={<Navigator active="database" />}
        tabs={[
          { label: 'players — SELECT', icon: 'db', active: true },
          { label: 'transactions',     icon: 'db' },
        ]}
        title="Otto — users-mysql"
      >
        <div style={{ display: 'flex', height: '100%' }}>
          {/* schema tree */}
          <div
            style={{
              width: 210,
              flexShrink: 0,
              background: T.bgSidebar,
              borderRight: `1px solid ${T.border}`,
              padding: '10px 0',
              overflow: 'hidden',
            }}
          >
            <div
              style={{
                padding: '0 12px 8px',
                fontFamily: fonts.ui,
                fontSize: 10.5,
                fontWeight: 600,
                letterSpacing: 0.6,
                textTransform: 'uppercase',
                color: T.textDim,
              }}
            >
              Schema
            </div>
            {TREE.map((item, i) => (
              <Appear key={i} delay={8 + i * 7} y={6}>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 6,
                    height: 26,
                    paddingLeft: 12 + item.depth * 14,
                    paddingRight: 8,
                    fontFamily: fonts.ui,
                    fontSize: 12.5,
                    fontWeight: item.active ? 600 : 500,
                    color: item.active ? T.text : T.textDim,
                    background: item.active ? alpha(T.accent, 0.12) : 'transparent',
                    borderLeft: item.active
                      ? `2px solid ${T.accent}`
                      : '2px solid transparent',
                  }}
                >
                  <Icon
                    name={item.icon}
                    size={13}
                    color={
                      item.depth === 0 ? T.accent
                        : item.active ? T.text
                        : T.textDim
                    }
                  />
                  {item.label}
                </div>
              </Appear>
            ))}
          </div>

          {/* query editor + results */}
          <div
            style={{
              flex: 1,
              display: 'flex',
              flexDirection: 'column',
              gap: 14,
              padding: '16px 18px',
              overflow: 'hidden',
            }}
          >
            <Appear delay={20} y={12}>
              <Terminal lines={SQL_LINES} delay={22} step={8} fontSize={14} />
            </Appear>
            <Appear delay={60} y={10}>
              <Table
                columns={['id', 'login', 'status', 'balance']}
                rows={TABLE_ROWS}
                widths={['60px', '1fr', '100px', '110px']}
                delay={64}
                step={10}
                fontSize={13}
              />
            </Appear>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Schema tree, query tabs, virtualized grid"
      sub="Auto LIMIT on every read · cancel any running query · client-side filter & sort"
      delay={26}
    />
  </>
);

// ── Scene 3 — NL → SQL + JOIN builder + inline-edit note ─────────────────────

const GEN_SQL =
  'SELECT p.login, SUM(t.amount) AS deposits\n' +
  'FROM players p\n' +
  'JOIN transactions t ON t.player_id = p.id\n' +
  "WHERE t.created_at >= NOW() - INTERVAL 7 DAY\n" +
  "  AND t.type = 'deposit'\n" +
  'GROUP BY p.id\n' +
  'ORDER BY deposits DESC LIMIT 10;';

const NlSqlScene: React.FC = () => {
  const typed = useTyped(GEN_SQL, 45, 40);
  return (
    <>
      <Stage scale={0.88}>
        <OttoWindow
          nav={<Navigator active="database" />}
          tabs={[{ label: 'NL → SQL', icon: 'command', active: true }]}
          title="Otto — users-mysql"
        >
          <div
            style={{
              padding: '24px 28px',
              display: 'flex',
              flexDirection: 'column',
              gap: 18,
              height: '100%',
              boxSizing: 'border-box',
              overflow: 'hidden',
            }}
          >
            {/* natural-language input */}
            <Appear delay={8} y={14}>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 10.5,
                  fontWeight: 600,
                  color: T.textDim,
                  letterSpacing: 0.6,
                  textTransform: 'uppercase',
                  marginBottom: 6,
                }}
              >
                Natural Language
              </div>
              <Field
                value="top 10 players by total deposits this week"
                icon="command"
                focused
              />
            </Appear>

            {/* divider arrow */}
            <Appear delay={35} y={0}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  fontFamily: fonts.ui,
                  fontSize: 12.5,
                  fontWeight: 600,
                  color: brand.cyan,
                }}
              >
                <Icon name="arrowDown" size={15} color={brand.cyan} />
                Generated SQL
              </div>
            </Appear>

            {/* typewriter SQL output */}
            <Appear delay={42} y={14}>
              <div
                style={{
                  background: T.termBg,
                  borderRadius: 8,
                  padding: '14px 18px',
                  fontFamily: fonts.mono,
                  fontSize: 14,
                  lineHeight: 1.75,
                  color: brand.cyan,
                  border: `1px solid ${alpha(brand.cyan, 0.22)}`,
                  whiteSpace: 'pre',
                  minHeight: 178,
                }}
              >
                {typed}
              </div>
            </Appear>

            {/* JOIN builder + inline-edit note cards */}
            <Appear delay={100} y={14}>
              <div style={{ display: 'flex', gap: 14 }}>
                <Card
                  pad={13}
                  style={{ flex: 1, display: 'flex', alignItems: 'center', gap: 10 }}
                >
                  <Icon name="branch" size={15} color={brand.violet} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.text, fontWeight: 500 }}>
                    <strong>Visual JOIN builder</strong> — drag tables to wire relations
                  </span>
                </Card>
                <Card
                  pad={13}
                  style={{ flex: 1, display: 'flex', alignItems: 'center', gap: 10 }}
                >
                  <Icon name="edit" size={15} color={brand.cyan} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.text, fontWeight: 500 }}>
                    Inline edits are <strong>approval-gated</strong> — staged before they run
                  </span>
                </Card>
              </div>
            </Appear>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={2}
        title="Natural-language → SQL · visual JOIN builder"
        sub="Approval-gated inline edits · FK navigation in-grid · import a file into a table"
        delay={22}
      />
    </>
  );
};

// ── Scene 4 — DB Assistant + ClickHouse dashboards ───────────────────────────

const AGENT_LINES: TermLine[] = [
  { text: '$ otto db examine --conn users-mysql',      tone: 'cmd'  },
  { text: '  reading schema for players, transactions…', tone: 'dim'  },
  { text: '  found 14 tables · 3 FK relationships',   tone: 'ok'   },
  { text: '  generating SCHEMA.md (read-only)…',      tone: 'dim'  },
  { text: '  ✓ SCHEMA.md written · context loaded',   tone: 'ok'   },
  { text: '',                                          tone: 'dim'  },
  { text: '  Agent ready — ask about indexes,',       tone: 'text' },
  { text: '  N+1 patterns, or schema improvements.',  tone: 'text' },
];

const DEPOSITS_WEEK = [28, 41, 36, 55, 62, 47, 71];
const DAU_WEEK      = [1240, 1380, 1290, 1510, 1640, 1420, 1790];

const ExamineDashScene: React.FC = () => {
  const frame = useCurrentFrame();
  const barGrow       = track(frame, [55, 92],  [0, 1]);
  const sparkProgress = track(frame, [72, 112], [0, 1]);

  return (
    <>
      <Stage scale={0.88}>
        <OttoWindow
          nav={<Navigator active="database" />}
          tabs={[
            { label: 'DB Assistant',         icon: 'terminal', active: true },
            { label: 'ClickHouse Dashboard', icon: 'chart' },
          ]}
          title="Otto — clickhouse-prod"
        >
          <div style={{ display: 'flex', height: '100%' }}>
            {/* left: agent examine terminal */}
            <div
              style={{
                width: '46%',
                flexShrink: 0,
                borderRight: `1px solid ${T.border}`,
                padding: 16,
                display: 'flex',
                flexDirection: 'column',
                gap: 10,
                overflow: 'hidden',
              }}
            >
              <Appear delay={6} y={10}>
                <div
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 10.5,
                    fontWeight: 600,
                    color: T.textDim,
                    letterSpacing: 0.5,
                    textTransform: 'uppercase',
                    marginBottom: 6,
                  }}
                >
                  DB Assistant
                </div>
                <Terminal lines={AGENT_LINES} delay={10} step={11} fontSize={13} />
              </Appear>
            </div>

            {/* right: ClickHouse dashboard */}
            <div
              style={{
                flex: 1,
                padding: '16px 18px',
                display: 'flex',
                flexDirection: 'column',
                gap: 14,
                overflow: 'hidden',
              }}
            >
              <Appear delay={10} y={8}>
                <div
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 10.5,
                    fontWeight: 600,
                    color: T.textDim,
                    letterSpacing: 0.5,
                    textTransform: 'uppercase',
                    marginBottom: 2,
                  }}
                >
                  ClickHouse Dashboard
                </div>
              </Appear>

              {/* metric stat row */}
              <Stagger
                delay={14}
                step={8}
                y={10}
                style={{ display: 'flex', gap: 12 }}
              >
                <MetricStat
                  label="Total Deposits (7d)"
                  value="$1.24M"
                  delta="↑ 18% vs prev week"
                  deltaTone="ok"
                  accent={brand.cyan}
                />
                <MetricStat
                  label="Active Players"
                  value="9,841"
                  delta="↑ 342 today"
                  deltaTone="ok"
                />
                <MetricStat
                  label="Avg Session"
                  value="23m"
                  delta="↓ 2m"
                  deltaTone="bad"
                />
              </Stagger>

              {/* bar chart: daily deposits */}
              <Appear delay={40} y={12}>
                <Card pad={14}>
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 12,
                      color: T.textDim,
                      marginBottom: 10,
                    }}
                  >
                    Daily Deposits — Last 7 Days
                  </div>
                  <BarChart
                    data={DEPOSITS_WEEK}
                    labels={['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']}
                    color={brand.cyan}
                    height={100}
                    grow={barGrow}
                  />
                </Card>
              </Appear>

              {/* sparkline: DAU */}
              <Appear delay={56} y={12}>
                <Card pad={14}>
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 12,
                      color: T.textDim,
                      marginBottom: 10,
                    }}
                  >
                    Daily Active Users
                  </div>
                  <Sparkline
                    data={DAU_WEEK}
                    color={brand.violet}
                    width={400}
                    height={70}
                    progress={sparkProgress}
                  />
                </Card>
              </Appear>
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={3}
        title="Examine a schema with an agent · ClickHouse dashboards"
        sub="Agent writes SCHEMA.md + answers queries read-only · Superset-style widgets · streaming CSV export"
        delay={22}
      />
    </>
  );
};

// ── Scene list ────────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 75,  node: <TitleScene />,       name: 'Title' },
  { dur: 185, node: <SchemaQueryScene />, name: 'SchemaQuery' },
  { dur: 140, node: <NlSqlScene />,       name: 'NlSql' },
  { dur: 120, node: <ExamineDashScene />, name: 'ExamineDash' },
  {
    dur: 130,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Database Explorer"
        tagline="A real database client, with an agent inside it"
        pills={[
          { label: 'MySQL · Redis · Mongo · CH', icon: 'db' },
          { label: 'NL → SQL',                  icon: 'command' },
          { label: 'Inline edits',               icon: 'edit' },
          { label: 'Dashboards & export',        icon: 'chart' },
        ]}
      />
    ),
  },
];

export const databaseDuration = scenesDuration(SCENES);
export const Database: React.FC = () => <Scenes scenes={SCENES} />;
