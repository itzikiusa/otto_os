import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, alpha, status as STATUS, series } from '../theme';
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
  MetricStat,
  BarChart,
  Sparkline,
  StatusDot,
  Toast,
  Ring,
  Icon,
  useTyped,
  Caret,
  track,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  DATABASE EXPLORER — a TablePlus-class, multi-engine SQL/NoSQL client built
//  into Otto. MySQL · Redis · MongoDB · ClickHouse, over plaintext / TLS / SSH.
// ════════════════════════════════════════════════════════════════════════════

// ── Scene 1 — title card ─────────────────────────────────────────────────────
const Title: React.FC = () => (
  <TitleCard
    kicker="Database Explorer"
    title="Query anything, beautifully"
    subtitle="MySQL · Redis · MongoDB · ClickHouse — over TLS or SSH"
  />
);

// ── Schema-tree rows (lazy: databases → tables → columns) ────────────────────
const TreeNode: React.FC<{
  label: string;
  icon: string;
  depth?: number;
  open?: boolean;
  active?: boolean;
  twisty?: boolean;
  dim?: boolean;
  color?: string;
}> = ({ label, icon, depth = 0, open, active, twisty, dim, color }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 7,
      height: 26,
      paddingLeft: 8 + depth * 16,
      paddingRight: 8,
      borderRadius: 5,
      background: active ? alpha(T.accent, 0.16) : 'transparent',
      color: active ? T.text : dim ? T.textDim : T.text,
      fontFamily: fonts.ui,
      fontSize: 12.5,
      fontWeight: active ? 600 : 500,
    }}
  >
    {twisty ? (
      <Icon name={open ? 'chevronDown' : 'chevronRight'} size={11} color={T.textDim} />
    ) : (
      <span style={{ width: 11 }} />
    )}
    <Icon name={icon} size={13} color={color ?? (active ? T.accent : T.textDim)} />
    <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{label}</span>
  </div>
);

// ── Scene 2 — schema tree + SQL editor + results grid ────────────────────────
const QUERY = "SELECT id, login, status FROM MdlGm_tblPlayers WHERE status = 'active' LIMIT 100";

const SchemaQueryScene: React.FC = () => {
  const frame = useCurrentFrame();
  const typed = useTyped(QUERY, 30, 34);
  const typing = typed.length < QUERY.length;
  const ranAt = 92; // results appear after the query "runs"
  const ran = frame >= ranAt;

  const activeChip = (
    <Chip tone="ok" style={{ height: 20 }}>
      active
    </Chip>
  );

  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow
          nav={<Navigator active="database" counts={{ database: 4 }} />}
          tabs={[
            { label: 'pr_bo · players', icon: 'db', active: true },
            { label: 'analytics.bets', icon: 'db' },
            { label: 'sessions:redis', icon: 'db' },
          ]}
          title="Otto — Database Explorer · MySQL @ pr_bo (SSH tunnel)"
        >
          <div style={{ display: 'flex', height: '100%', minHeight: 0 }}>
            {/* ── schema tree ── */}
            <div
              style={{
                width: 268,
                flexShrink: 0,
                borderRight: `1px solid ${T.border}`,
                background: T.bgSidebar,
                display: 'flex',
                flexDirection: 'column',
                minHeight: 0,
              }}
            >
              <div
                style={{
                  height: 34,
                  display: 'flex',
                  alignItems: 'center',
                  gap: 7,
                  padding: '0 12px',
                  borderBottom: `1px solid ${T.border}`,
                  fontFamily: fonts.ui,
                  fontSize: 11.5,
                  fontWeight: 600,
                  letterSpacing: 0.4,
                  textTransform: 'uppercase',
                  color: T.textDim,
                }}
              >
                <Icon name="db" size={13} color="#00758f" />
                Schema
              </div>
              <div style={{ padding: '8px 6px', overflow: 'hidden' }}>
                <Stagger delay={6} step={3} y={6}>
                  <TreeNode label="pr_bo" icon="folder" twisty open color={T.accent} />
                  <TreeNode label="MdlGm_tblPlayers" icon="grid" depth={1} twisty open active />
                  <TreeNode label="id · bigint" icon="key" depth={2} dim />
                  <TreeNode label="login · varchar" icon="dot" depth={2} dim />
                  <TreeNode label="email · varchar" icon="dot" depth={2} dim />
                  <TreeNode label="status · enum" icon="dot" depth={2} dim />
                  <TreeNode label="MdlEnv_tblPlayerBalance" icon="grid" depth={1} twisty />
                  <TreeNode label="GSS_activities" icon="grid" depth={1} twisty />
                  <TreeNode label="MdlCsh_tblTransactions" icon="grid" depth={1} twisty />
                  <TreeNode label="MdlGm_tblGameRounds" icon="grid" depth={1} twisty />
                </Stagger>
              </div>
            </div>

            {/* ── editor + results ── */}
            <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
              {/* SQL editor */}
              <Appear delay={4} y={0}>
                <div
                  style={{
                    background: T.termBg,
                    borderBottom: `1px solid ${T.border}`,
                    padding: '14px 18px 16px',
                  }}
                >
                  <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 12 }}>
                    <Button variant="primary" icon="play" size="s">
                      Run
                    </Button>
                    <Chip color={brand.violet} style={{ height: 22 }}>
                      <Icon name="key" size={11} color={brand.violet} />
                      SSH tunnel · bastion
                    </Chip>
                    <Chip color={STATUS.working} style={{ height: 22 }}>
                      TLS
                    </Chip>
                    <div style={{ flex: 1 }} />
                    <Chip style={{ height: 22 }}>auto LIMIT 1000</Chip>
                    <Chip style={{ height: 22 }}>timeout 30s</Chip>
                  </div>
                  {/* syntax-highlighted query line */}
                  <div style={{ fontFamily: fonts.mono, fontSize: 16, lineHeight: 1.55, color: T.text }}>
                    <SqlLine text={typed} />
                    {typing && <Caret color={brand.cyan} h={18} />}
                  </div>
                </div>
              </Appear>

              {/* results grid */}
              <div style={{ flex: 1, minHeight: 0, padding: '14px 18px', overflow: 'hidden' }}>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    marginBottom: 10,
                    fontFamily: fonts.ui,
                    fontSize: 12.5,
                    color: T.textDim,
                  }}
                >
                  <StatusDot kind={ran ? 'working' : 'idle'} size={8} pulse={false} />
                  {ran ? '6 rows · 14 ms · masked: email' : 'running…'}
                  <div style={{ flex: 1 }} />
                  <Chip style={{ height: 20 }}>
                    <Icon name="eye" size={11} color={T.textDim} />
                    read-only — edits approval-gated
                  </Chip>
                </div>
                {ran && (
                  <Table
                    delay={ranAt - 2}
                    step={4}
                    fontSize={13}
                    columns={['id', 'login', 'email', 'status']}
                    widths={['90px', '1.2fr', '1.6fr', '120px']}
                    rows={[
                      ['840219', 'a.morgan', 'a•••@mail.com', activeChip],
                      ['840220', 'l.fischer', 'l•••@mail.com', activeChip],
                      ['840224', 'm.delacroix', 'm•••@mail.com', activeChip],
                      ['840231', 's.ohara', 's•••@mail.com', activeChip],
                      ['840238', 'k.nakamura', 'k•••@mail.com', activeChip],
                      ['840240', 'r.bianchi', 'r•••@mail.com', activeChip],
                    ]}
                  />
                )}
              </div>
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={1}
        title="Schema tree, query tabs, fast grid"
        sub="Per-engine autocomplete · automatic LIMIT · masked columns"
      />
    </>
  );
};

// Minimal SQL token highlighter for the editor line.
const SqlLine: React.FC<{ text: string }> = ({ text }) => {
  const KEYWORDS = new Set(['SELECT', 'FROM', 'WHERE', 'LIMIT', 'AND', 'OR', 'JOIN', 'ON']);
  const parts = text.split(/(\s+)/);
  return (
    <span>
      {parts.map((p, i) => {
        const up = p.toUpperCase();
        let color = T.text;
        if (KEYWORDS.has(up)) color = brand.cyan;
        else if (/^'.*'?$/.test(p)) color = STATUS.working; // string literal
        else if (/^\d+$/.test(p)) color = STATUS.needsYou; // number
        return (
          <span key={i} style={{ color, fontWeight: KEYWORDS.has(up) ? 600 : 400 }}>
            {p}
          </span>
        );
      })}
    </span>
  );
};

// ── Scene 3 — visual JOIN builder (ERD canvas) ───────────────────────────────
const ErdTable: React.FC<{
  name: string;
  cols: { name: string; pk?: boolean; fk?: boolean; hl?: boolean }[];
  accent: string;
  delay: number;
}> = ({ name, cols, accent, delay }) => (
  <Appear delay={delay} y={18} scale={0.94}>
    <Card pad={0} style={{ width: 264, overflow: 'hidden', boxShadow: T.shadow }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '9px 13px',
          background: alpha(accent, 0.16),
          borderBottom: `1px solid ${T.border}`,
        }}
      >
        <Icon name="grid" size={14} color={accent} />
        <span style={{ fontFamily: fonts.mono, fontSize: 13.5, fontWeight: 600, color: T.text }}>{name}</span>
      </div>
      <div style={{ padding: '6px 0' }}>
        {cols.map((c) => (
          <div
            key={c.name}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              height: 28,
              padding: '0 13px',
              background: c.hl ? alpha(brand.cyan, 0.1) : 'transparent',
              fontFamily: fonts.mono,
              fontSize: 13,
              color: c.hl ? T.text : T.textDim,
            }}
          >
            <Icon name={c.pk ? 'key' : c.fk ? 'link' : 'dot'} size={12} color={c.pk ? STATUS.needsYou : c.fk ? brand.cyan : T.textDim} />
            <span style={{ flex: 1, fontWeight: c.pk || c.fk ? 600 : 400, color: c.pk || c.fk ? T.text : T.textDim }}>
              {c.name}
            </span>
          </div>
        ))}
      </div>
    </Card>
  </Appear>
);

const JoinBuilderScene: React.FC = () => {
  const frame = useCurrentFrame();
  const draw = track(frame, [40, 70], [0, 1]); // join line draws in
  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow
          nav={<Navigator active="database" counts={{ database: 4 }} />}
          tabs={[{ label: 'Visual JOIN builder', icon: 'split', active: true }]}
          title="Otto — Database Explorer · ERD"
        >
          <div style={{ height: '100%', display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            {/* ERD canvas */}
            <div
              style={{
                flex: 1,
                minHeight: 0,
                position: 'relative',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                gap: 150,
                backgroundImage:
                  `linear-gradient(${alpha('#fff', 0.025)} 1px, transparent 1px),` +
                  `linear-gradient(90deg, ${alpha('#fff', 0.025)} 1px, transparent 1px)`,
                backgroundSize: '28px 28px',
              }}
            >
              {/* connecting line on player_id (players.id → balances.player_id) */}
              <svg style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', pointerEvents: 'none' }}>
                <defs>
                  <linearGradient id="joinln" x1="0" y1="0" x2="1" y2="0">
                    <stop offset="0" stopColor={brand.cyan} />
                    <stop offset="1" stopColor={brand.purple} />
                  </linearGradient>
                </defs>
                <path
                  d="M 700 372 C 800 372, 880 372, 980 372"
                  fill="none"
                  stroke="url(#joinln)"
                  strokeWidth={3}
                  strokeLinecap="round"
                  strokeDasharray={220}
                  strokeDashoffset={220 * (1 - draw)}
                />
                <circle cx={700} cy={372} r={5} fill={brand.cyan} opacity={draw} />
                <circle cx={980} cy={372} r={5} fill={brand.purple} opacity={draw} />
              </svg>

              <ErdTable
                name="MdlGm_tblPlayers"
                accent="#00758f"
                delay={6}
                cols={[
                  { name: 'id', pk: true },
                  { name: 'login' },
                  { name: 'player_id', fk: true, hl: true },
                  { name: 'status' },
                ]}
              />
              <ErdTable
                name="MdlEnv_tblPlayerBalance"
                accent={brand.violet}
                delay={14}
                cols={[
                  { name: 'player_id', fk: true, hl: true },
                  { name: 'latestBalance' },
                  { name: 'bonusBalance' },
                  { name: 'currency' },
                ]}
              />

              <Appear delay={50} y={10} style={{ position: 'absolute', top: '50%', left: '50%', transform: 'translate(-50%,-50%)' }}>
                <Chip color={brand.cyan} style={{ height: 26, fontSize: 13 }}>
                  <Icon name="link" size={12} color={brand.cyan} />
                  INNER JOIN · ON player_id
                </Chip>
              </Appear>
            </div>

            {/* generated SQL preview */}
            <Appear delay={58} y={14}>
              <div
                style={{
                  borderTop: `1px solid ${T.border}`,
                  background: T.termBg,
                  padding: '16px 22px',
                  fontFamily: fonts.mono,
                  fontSize: 15,
                  lineHeight: 1.65,
                }}
              >
                <div style={{ display: 'flex', alignItems: 'center', gap: 9, marginBottom: 10 }}>
                  <Icon name="zap" size={13} color={brand.cyan} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 12, fontWeight: 600, color: T.textDim, letterSpacing: 0.4 }}>
                    GENERATED SQL
                  </span>
                </div>
                <div>
                  <span style={{ color: brand.cyan, fontWeight: 600 }}>SELECT</span>{' '}
                  <span style={{ color: T.text }}>p.login, b.latestBalance, b.currency</span>
                </div>
                <div>
                  <span style={{ color: brand.cyan, fontWeight: 600 }}>FROM</span>{' '}
                  <span style={{ color: T.text }}>MdlGm_tblPlayers p</span>{' '}
                  <span style={{ color: brand.cyan, fontWeight: 600 }}>JOIN</span>{' '}
                  <span style={{ color: T.text }}>MdlEnv_tblPlayerBalance b</span>{' '}
                  <span style={{ color: brand.cyan, fontWeight: 600 }}>ON</span>{' '}
                  <span style={{ color: T.text }}>p.player_id = b.player_id</span>
                </div>
              </div>
            </Appear>
          </div>
        </OttoWindow>
      </Stage>
      <Caption step={2} title="Build joins visually" sub="Navicat-style — Otto writes the SQL" />
    </>
  );
};

// ── Scene 4 — ClickHouse dashboard of widgets ────────────────────────────────
const Widget: React.FC<{ title: string; delay: number; children: React.ReactNode; style?: React.CSSProperties }> = ({
  title,
  delay,
  children,
  style,
}) => (
  <Appear delay={delay} y={16} scale={0.96} style={style}>
    <Card pad={0} style={{ height: '100%', overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '9px 14px',
          borderBottom: `1px solid ${T.border}`,
          fontFamily: fonts.ui,
          fontSize: 12.5,
          fontWeight: 600,
          color: T.text,
        }}
      >
        <Icon name="chart" size={13} color={brand.cyan} />
        {title}
      </div>
      <div style={{ flex: 1, padding: 14, display: 'flex', flexDirection: 'column', justifyContent: 'center' }}>{children}</div>
    </Card>
  </Appear>
);

const DashboardScene: React.FC = () => {
  const frame = useCurrentFrame();
  const grow = track(frame, [40, 72], [0, 1]);
  const spark = track(frame, [52, 88], [0, 1]);
  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow
          nav={<Navigator active="database" counts={{ database: 4 }} />}
          tabs={[{ label: 'analytics · Live Ops', icon: 'gauge', active: true, dot: 'working' }]}
          title="Otto — Database Explorer · ClickHouse dashboard"
        >
          <div style={{ height: '100%', padding: 18, display: 'flex', flexDirection: 'column', gap: 14, minHeight: 0 }}>
            {/* header strip */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              <Icon name="db" size={16} color="#febc2e" />
              <span style={{ fontFamily: fonts.ui, fontSize: 17, fontWeight: 700, color: T.text }}>Live Ops — last 24h</span>
              <Chip color="#febc2e" style={{ height: 22 }}>
                ClickHouse
              </Chip>
              <div style={{ flex: 1 }} />
              <Chip tone="ok" style={{ height: 22 }}>
                <StatusDot kind="working" size={7} />
                streaming
              </Chip>
              <Button variant="default" icon="refresh" size="s">
                Saved
              </Button>
            </div>

            {/* metric stats row */}
            <div style={{ display: 'flex', gap: 14 }}>
              <Appear delay={8} y={14} style={{ flex: 1 }}>
                <MetricStat label="Bets · 24h" value="€4.2M" delta="▲ 8.4% vs yest" deltaTone="ok" style={{ minWidth: 0 }} accent={T.text} />
              </Appear>
              <Appear delay={13} y={14} style={{ flex: 1 }}>
                <MetricStat label="Active players" value="18,294" delta="▲ 1,206 online now" deltaTone="ok" style={{ minWidth: 0 }} accent={T.text} />
              </Appear>
              <Appear delay={18} y={14} style={{ flex: 1 }}>
                <MetricStat label="Pending withdrawals" value="132" delta="▲ 12 in queue" deltaTone="bad" style={{ minWidth: 0 }} accent={T.text} />
              </Appear>
            </div>

            {/* charts row */}
            <div style={{ flex: 1, minHeight: 0, display: 'flex', gap: 14 }}>
              <Widget title="Bets per hour" delay={26} style={{ flex: 1.5 }}>
                <BarChart
                  data={[42, 55, 48, 63, 71, 58, 82, 76, 90, 84, 97, 88]}
                  labels={['0h', '', '4h', '', '8h', '', '12h', '', '16h', '', '20h', '']}
                  color={series[2]}
                  grow={grow}
                  height={150}
                />
              </Widget>
              <Widget title="Revenue trend · 14d" delay={34} style={{ flex: 1 }}>
                <Sparkline
                  data={[30, 34, 31, 40, 44, 41, 52, 49, 58, 63, 60, 71, 76, 82]}
                  color={series[1]}
                  width={320}
                  height={150}
                  progress={spark}
                />
              </Widget>
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={3}
        title="ClickHouse dashboards & widgets"
        sub="Superset-style charts on your analytics tables"
      />
    </>
  );
};

// ── Scene 5 — streaming export to a local file ───────────────────────────────
const ExportScene: React.FC = () => {
  const frame = useCurrentFrame();
  const prog = track(frame, [24, 96], [0, 0.92]); // streaming progress
  const pct = Math.round(prog * 100);
  const rows = Math.round(prog * 1_396_633); // streamed so far
  const fmtRows = (n: number) => n.toLocaleString('en-US');
  const formats = ['CSV', 'JSON', 'TSV', 'NDJSON', 'Parquet', 'SQL'];

  return (
    <>
      <Stage scale={0.92}>
        <OttoWindow
          nav={<Navigator active="database" counts={{ database: 4 }} />}
          tabs={[{ label: 'pr_bo · players', icon: 'db', active: true }]}
          title="Otto — Database Explorer · Export"
        >
          <div style={{ height: '100%', position: 'relative' }}>
            {/* dimmed results behind the menu */}
            <div style={{ position: 'absolute', inset: 0, padding: 18, opacity: 0.35 }}>
              <Table
                delay={0}
                step={0}
                fontSize={13}
                columns={['id', 'login', 'email', 'status']}
                widths={['90px', '1.2fr', '1.6fr', '120px']}
                rows={[
                  ['840219', 'a.morgan', 'a•••@mail.com', 'active'],
                  ['840220', 'l.fischer', 'l•••@mail.com', 'active'],
                  ['840224', 'm.delacroix', 'm•••@mail.com', 'active'],
                  ['840231', 's.ohara', 's•••@mail.com', 'active'],
                ]}
              />
            </div>

            {/* export panel */}
            <Appear delay={4} y={18} style={{ position: 'absolute', top: 70, right: 70 }}>
              <Card pad={0} style={{ width: 380, overflow: 'hidden', boxShadow: '0 40px 100px rgba(0,0,0,0.6)' }}>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 9,
                    padding: '13px 16px',
                    borderBottom: `1px solid ${T.border}`,
                  }}
                >
                  <Icon name="arrowDown" size={15} color={brand.cyan} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 14.5, fontWeight: 700, color: T.text }}>Export query</span>
                  <div style={{ flex: 1 }} />
                  <Icon name="x" size={14} color={T.textDim} />
                </div>
                {/* format chips */}
                <div style={{ padding: '14px 16px 6px', display: 'flex', flexWrap: 'wrap', gap: 8 }}>
                  {formats.map((f, i) => (
                    <Chip key={f} tone={i === 0 ? 'accent' : 'default'} color={i === 0 ? brand.cyan : undefined} style={{ height: 26 }}>
                      {f}
                    </Chip>
                  ))}
                </div>
                <div style={{ padding: '8px 16px 14px' }}>
                  <div
                    style={{
                      fontFamily: fonts.mono,
                      fontSize: 12.5,
                      color: T.textDim,
                      marginBottom: 14,
                      display: 'flex',
                      alignItems: 'center',
                      gap: 7,
                    }}
                  >
                    <Icon name="file" size={12} color={T.textDim} />
                    ~/Downloads/players.csv
                  </div>

                  {/* progress: ring + bar */}
                  <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
                    <Ring value={prog} size={72} color={brand.cyan} label={`${pct}%`} />
                    <div style={{ flex: 1 }}>
                      <div
                        style={{
                          fontFamily: fonts.ui,
                          fontSize: 12.5,
                          color: T.text,
                          marginBottom: 8,
                          display: 'flex',
                          justifyContent: 'space-between',
                        }}
                      >
                        <span>Streaming…</span>
                        <span style={{ color: T.textDim }}>bounded RAM · 24 MB</span>
                      </div>
                      <div style={{ height: 8, borderRadius: 999, background: T.surface2, overflow: 'hidden' }}>
                        <div
                          style={{
                            width: `${prog * 100}%`,
                            height: '100%',
                            borderRadius: 999,
                            background: `linear-gradient(90deg, ${brand.purple}, ${brand.cyan})`,
                          }}
                        />
                      </div>
                      <div style={{ fontFamily: fonts.mono, fontSize: 11.5, color: T.textDim, marginTop: 8 }}>
                        {fmtRows(rows)} rows · 1.4 GB
                      </div>
                    </div>
                  </div>
                </div>
              </Card>
            </Appear>

            {/* completion toast */}
            {frame >= 100 && (
              <Toast
                delay={100}
                tone="ok"
                text="players.csv · 1,284,902 rows streamed"
                style={{ position: 'absolute', bottom: 26, right: 70 }}
              />
            )}
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={4}
        title="Stream millions of rows to a local file"
        sub="6 formats · bounded memory · no server-side OUTFILE"
      />
    </>
  );
};

// ── Scenes ────────────────────────────────────────────────────────────────────
const SCENES: SceneDef[] = [
  { dur: 80, node: <Title />, name: 'Title' },
  { dur: 230, node: <SchemaQueryScene />, name: 'Schema + Query' },
  { dur: 200, node: <JoinBuilderScene />, name: 'JOIN builder' },
  { dur: 210, node: <DashboardScene />, name: 'ClickHouse dashboard' },
  { dur: 130, node: <ExportScene />, name: 'Export' },
  {
    dur: 130,
    node: (
      <WalkOutro
        title="Database Explorer"
        tagline="Four engines, one beautiful client."
        pills={[
          { label: 'MySQL', color: '#00758f', icon: 'db' },
          { label: 'Redis', color: '#ff5f57', icon: 'db' },
          { label: 'MongoDB', color: '#28c840', icon: 'db' },
          { label: 'ClickHouse', color: '#febc2e', icon: 'chart' },
          { label: 'CSV export', color: brand.cyan, icon: 'arrowDown' },
        ]}
      />
    ),
    name: 'Outro',
  },
];

export const databaseDuration = scenesDuration(SCENES);
export const Database: React.FC = () => <Scenes scenes={SCENES} />;
