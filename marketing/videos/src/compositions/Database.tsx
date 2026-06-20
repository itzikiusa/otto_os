import React from 'react';
import {
  AbsoluteFill,
  Sequence,
  useCurrentFrame,
  useVideoConfig,
  interpolate,
  spring,
} from 'remotion';
import { theme } from '../theme';
import { OttoWindow } from '../components/OttoWindow';
import { Appear, Caption, Cursor, TitleCard } from '../components/ui';

// ─── Database Explorer — ~36s walkthrough ────────────────────────────────────
// TablePlus-style browser: schema tree, query tabs, results grid,
// JOIN builder, dashboards, SSH tunnels.
// ─────────────────────────────────────────────────────────────────────────────

// ─── Timing constants (frames @ 30 fps) ───────────────────────────────────────
const TITLE_DUR  = 75;   // 0–75  title card
const S1_DUR     = 180;  // schema tree + table browse
const S2_DUR     = 210;  // query editor
const S3_DUR     = 150;  // JOIN builder / results
const S4_DUR     = 120;  // SSH tunnel indicator
const OUTRO_DUR  = 105;

const S1_START   = TITLE_DUR;
const S2_START   = S1_START + S1_DUR;
const S3_START   = S2_START + S2_DUR;
const S4_START   = S3_START + S3_DUR;
const OUTRO_START = S4_START + S4_DUR;

// ─── Helpers ─────────────────────────────────────────────────────────────────

function typewriter(text: string, frame: number, cps = 20): string {
  const chars = Math.floor((frame / 30) * cps);
  return text.slice(0, Math.min(chars, text.length));
}

const HR: React.FC = () => (
  <div style={{ height: 1, background: theme.border }} />
);

// ─── Shared side panel: schema tree ──────────────────────────────────────────
const TABLES = ['MdlGm_tblPlayers', 'MdlEnv_tblPlayerBalance', 'GSS_activities', 'MdlCsh_tblTransactions'];

const SchemaTree: React.FC<{ active?: string }> = ({ active = 'MdlGm_tblPlayers' }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  return (
    <div
      style={{
        width: 280,
        background: theme.surface,
        borderRight: `1px solid ${theme.border}`,
        height: '100%',
        flexShrink: 0,
        display: 'flex',
        flexDirection: 'column',
      }}
    >
      <div style={{ padding: '14px 16px', borderBottom: `1px solid ${theme.border}` }}>
        <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 11, fontWeight: 700, letterSpacing: 1.2, textTransform: 'uppercase', marginBottom: 8 }}>
          Schema
        </div>
        <div style={{ background: theme.surface2, borderRadius: 8, padding: '7px 12px', display: 'flex', alignItems: 'center', gap: 8, border: `1px solid ${theme.border}` }}>
          <span style={{ color: theme.textDim, fontSize: 14 }}>⌕</span>
          <span style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 13 }}>filter…</span>
        </div>
      </div>
      <div style={{ padding: '8px 0', flex: 1, overflow: 'hidden' }}>
        {/* db group header */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '6px 16px 4px' }}>
          <span style={{ color: '#febc2e', fontSize: 14 }}>▾</span>
          <span style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 12 }}>casino_db</span>
        </div>
        {TABLES.map((t, i) => {
          const s = spring({ frame: frame - i * 10, fps, config: { damping: 200 } });
          const isActive = t === active;
          return (
            <div
              key={t}
              style={{
                opacity: s,
                transform: `translateX(${interpolate(s, [0, 1], [-10, 0])}px)`,
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                padding: '7px 16px 7px 32px',
                background: isActive ? `${theme.accent}18` : 'transparent',
                borderLeft: isActive ? `2px solid ${theme.accent}` : '2px solid transparent',
              }}
            >
              <span style={{ fontSize: 13, color: '#4a8fff' }}>⊞</span>
              <span style={{ fontFamily: theme.mono, fontSize: 12, color: isActive ? theme.text : theme.textDim }}>
                {t}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
};

// ─── Scene 1 – Table browse / results grid ────────────────────────────────────
const COLUMNS = ['ID', 'Login', 'Email', 'PlayerStatus', 'CreatedAt'];
const ROWS = [
  ['482901', 'alice',  'alice@example.com',   'Active',   '2024-01-14'],
  ['482902', 'bob',    'bob@example.com',      'Active',   '2024-01-15'],
  ['482903', 'carol',  'carol@example.com',    'Inactive', '2024-01-16'],
  ['482904', 'dave',   'dave@example.com',     'Active',   '2024-01-17'],
  ['482905', 'eve',    'eve@example.com',      'Pending',  '2024-01-18'],
];

const STATUS_COLOR: Record<string, string> = {
  Active:   '#9ee039',
  Inactive: '#8b97a8',
  Pending:  '#febc2e',
};

const Scene1Browse: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  return (
    <div style={{ display: 'flex', height: '100%' }}>
      <SchemaTree active="MdlGm_tblPlayers" />
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {/* tab bar */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 0, borderBottom: `1px solid ${theme.border}`, padding: '0 20px', height: 40, flexShrink: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '0 16px', height: '100%', borderBottom: `2px solid ${theme.accent}`, color: theme.text, fontFamily: theme.mono, fontSize: 13 }}>
            <span style={{ color: '#4a8fff' }}>⊞</span> MdlGm_tblPlayers
          </div>
        </div>
        {/* grid header */}
        <Appear delay={8}>
          <div style={{ display: 'grid', gridTemplateColumns: '80px 120px 220px 120px 140px', background: theme.surface2, borderBottom: `1px solid ${theme.border}`, padding: '0 20px' }}>
            {COLUMNS.map((col) => (
              <div key={col} style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 12, fontWeight: 700, letterSpacing: 0.5, padding: '10px 8px 10px 0', textTransform: 'uppercase' }}>
                {col}
              </div>
            ))}
          </div>
        </Appear>
        {/* rows */}
        <div style={{ flex: 1, overflow: 'hidden' }}>
          {ROWS.map((row, i) => {
            const s = spring({ frame: frame - (i * 12 + 20), fps, config: { damping: 200 } });
            return (
              <div
                key={i}
                style={{
                  opacity: s,
                  transform: `translateX(${interpolate(s, [0, 1], [10, 0])}px)`,
                  display: 'grid',
                  gridTemplateColumns: '80px 120px 220px 120px 140px',
                  padding: '0 20px',
                  borderBottom: `1px solid ${theme.border}22`,
                  background: i % 2 === 0 ? 'transparent' : 'rgba(255,255,255,0.012)',
                }}
              >
                {row.map((cell, j) => (
                  <div key={j} style={{ padding: '10px 8px 10px 0', fontFamily: theme.mono, fontSize: 13 }}>
                    {j === 3 ? (
                      <span style={{ color: STATUS_COLOR[cell] ?? theme.textDim, background: `${STATUS_COLOR[cell] ?? theme.textDim}22`, borderRadius: 6, padding: '2px 10px', fontSize: 12, fontWeight: 700 }}>
                        {cell}
                      </span>
                    ) : (
                      <span style={{ color: j === 0 ? theme.accent : theme.text }}>{cell}</span>
                    )}
                  </div>
                ))}
              </div>
            );
          })}
        </div>
        {/* row count bar */}
        <div style={{ padding: '8px 20px', borderTop: `1px solid ${theme.border}`, background: theme.surface2, display: 'flex', alignItems: 'center', gap: 16 }}>
          <span style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 12 }}>482 901 rows</span>
          <span style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 12 }}>· page 1 / 9658</span>
        </div>
      </div>

      <Caption step={1} title="Schema tree + results grid" sub="Browse tables like TablePlus — click to explore" delay={50} />
    </div>
  );
};

// ─── Scene 2 – SQL query editor ────────────────────────────────────────────────
const SQL_QUERY = `SELECT p.Login, p.Email,
       b.latestBalance, b.bonusBalance
FROM   MdlGm_tblPlayers p
JOIN   MdlEnv_tblPlayerBalance b
         ON b.playerId = p.ID
WHERE  p.PlayerStatus = 'Active'
LIMIT  50;`;

const RESULT_ROWS = [
  ['alice', 'alice@example.com', '1 250.00', '50.00'],
  ['bob',   'bob@example.com',   '8 740.50', '0.00'],
  ['dave',  'dave@example.com',  '312.80',   '100.00'],
];

const Scene2Query: React.FC = () => {
  const frame = useCurrentFrame();
  const sql = frame < 120 ? typewriter(SQL_QUERY, frame, 28) : SQL_QUERY;
  const showResults = frame >= 130;
  const { fps } = useVideoConfig();
  const resS = spring({ frame: frame - 130, fps, config: { damping: 180 } });

  return (
    <div style={{ display: 'flex', height: '100%' }}>
      <SchemaTree active="MdlGm_tblPlayers" />
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {/* tab bar */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 0, borderBottom: `1px solid ${theme.border}`, padding: '0 20px', height: 40, flexShrink: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '0 16px', height: '100%', borderBottom: `2px solid ${theme.accent}`, color: theme.text, fontFamily: theme.mono, fontSize: 13 }}>
            <span style={{ color: theme.textDim }}>SQL</span> Query 1
          </div>
        </div>
        {/* editor pane */}
        <div style={{ padding: '20px 24px', background: '#0d1117', flex: '0 0 auto', borderBottom: `1px solid ${theme.border}` }}>
          <pre style={{ margin: 0, fontFamily: theme.mono, fontSize: 15, lineHeight: 1.65, color: theme.text, whiteSpace: 'pre' }}>
            {sql.split('\n').map((line, i) => (
              <span key={i} style={{ display: 'block' }}>{line}</span>
            ))}
          </pre>
        </div>
        {/* run button row */}
        <div style={{ padding: '8px 24px', borderBottom: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '7px 18px', background: theme.accent, borderRadius: 8, color: '#fff', fontFamily: theme.font, fontSize: 14, fontWeight: 700, boxShadow: `0 4px 16px ${theme.accent}44` }}>
            ▶ Run
          </div>
          {showResults && (
            <span style={{ color: theme.accent2, fontFamily: theme.mono, fontSize: 13, opacity: resS }}>
              3 rows · 4 ms
            </span>
          )}
        </div>
        {/* results */}
        {showResults && (
          <div style={{ opacity: resS, transform: `translateY(${interpolate(resS, [0, 1], [14, 0])}px)`, flex: 1, overflow: 'hidden' }}>
            <div style={{ display: 'grid', gridTemplateColumns: '160px 220px 120px 120px', background: theme.surface2, padding: '0 20px', borderBottom: `1px solid ${theme.border}` }}>
              {['Login', 'Email', 'latestBalance', 'bonusBalance'].map((h) => (
                <div key={h} style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 12, fontWeight: 700, padding: '9px 8px 9px 0', textTransform: 'uppercase' }}>
                  {h}
                </div>
              ))}
            </div>
            {RESULT_ROWS.map((row, i) => (
              <div key={i} style={{ display: 'grid', gridTemplateColumns: '160px 220px 120px 120px', padding: '0 20px', borderBottom: `1px solid ${theme.border}22` }}>
                {row.map((cell, j) => (
                  <div key={j} style={{ padding: '10px 8px 10px 0', fontFamily: theme.mono, fontSize: 13, color: theme.text }}>
                    {cell}
                  </div>
                ))}
              </div>
            ))}
          </div>
        )}
      </div>

      <Caption step={2} title="SQL query editor" sub="Multi-tab queries, instant results" delay={60} />
    </div>
  );
};

// ─── Scene 3 – JOIN builder / dashboard tile ────────────────────────────────────
const Scene3Join: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const tileS1 = spring({ frame: frame - 8, fps, config: { damping: 180 } });
  const tileS2 = spring({ frame: frame - 22, fps, config: { damping: 180 } });
  const tileS3 = spring({ frame: frame - 36, fps, config: { damping: 180 } });

  return (
    <div style={{ display: 'flex', height: '100%' }}>
      <SchemaTree active="GSS_activities" />
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', padding: 28, gap: 20, overflow: 'hidden' }}>
        <Appear delay={0}>
          <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 22, fontWeight: 800 }}>Dashboards</div>
          <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 14, marginTop: 4 }}>Live tiles built from saved queries</div>
        </Appear>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 18 }}>
          {/* tile 1 */}
          <div style={{ opacity: tileS1, transform: `scale(${interpolate(tileS1, [0, 1], [0.9, 1])})`, background: theme.surface2, borderRadius: 14, padding: '22px 24px', border: `1px solid ${theme.border}` }}>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 12, fontWeight: 700, letterSpacing: 1, textTransform: 'uppercase', marginBottom: 8 }}>Total Bets (24h)</div>
            <div style={{ color: theme.accent2, fontFamily: theme.mono, fontSize: 36, fontWeight: 800 }}>€ 4.2M</div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13, marginTop: 4 }}>↑ 8.3% vs yesterday</div>
          </div>
          {/* tile 2 */}
          <div style={{ opacity: tileS2, transform: `scale(${interpolate(tileS2, [0, 1], [0.9, 1])})`, background: theme.surface2, borderRadius: 14, padding: '22px 24px', border: `1px solid ${theme.border}` }}>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 12, fontWeight: 700, letterSpacing: 1, textTransform: 'uppercase', marginBottom: 8 }}>Active Players</div>
            <div style={{ color: theme.accent, fontFamily: theme.mono, fontSize: 36, fontWeight: 800 }}>18 294</div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13, marginTop: 4 }}>Last 1h concurrent peak</div>
          </div>
          {/* tile 3 */}
          <div style={{ opacity: tileS3, transform: `scale(${interpolate(tileS3, [0, 1], [0.9, 1])})`, background: theme.surface2, borderRadius: 14, padding: '22px 24px', border: `1px solid ${theme.border}` }}>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 12, fontWeight: 700, letterSpacing: 1, textTransform: 'uppercase', marginBottom: 8 }}>Pending Withdrawals</div>
            <div style={{ color: theme.warn, fontFamily: theme.mono, fontSize: 36, fontWeight: 800 }}>132</div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13, marginTop: 4 }}>avg wait 2m 14s</div>
          </div>
        </div>

        {/* join builder banner */}
        <Appear delay={60}>
          <div style={{ background: `${theme.accent}11`, borderRadius: 14, padding: '20px 24px', border: `1px solid ${theme.accent}33`, display: 'flex', alignItems: 'center', gap: 18 }}>
            <span style={{ fontSize: 28 }}>⟵⟶</span>
            <div>
              <div style={{ color: theme.accent, fontFamily: theme.font, fontSize: 16, fontWeight: 700 }}>JOIN Builder</div>
              <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13, marginTop: 3 }}>
                Drag tables to canvas — Otto writes the JOIN conditions automatically
              </div>
            </div>
            <div style={{ marginLeft: 'auto', padding: '8px 20px', background: theme.accent, borderRadius: 10, color: '#fff', fontFamily: theme.font, fontSize: 14, fontWeight: 700, boxShadow: `0 4px 16px ${theme.accent}44` }}>
              Open builder
            </div>
          </div>
        </Appear>
      </div>

      <Caption step={3} title="Dashboards + JOIN builder" sub="Saved-query tiles — drag tables to build JOINs visually" delay={50} />
    </div>
  );
};

// ─── Scene 4 – SSH tunnel indicator ───────────────────────────────────────────
const Scene4Tunnel: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const tunnelS = spring({ frame: frame - 16, fps, config: { damping: 160 } });
  const dotPulse = Math.abs(Math.sin((frame / 30) * Math.PI)) * 0.6 + 0.4;

  return (
    <div style={{ display: 'flex', height: '100%', alignItems: 'center', justifyContent: 'center' }}>
      <div
        style={{
          opacity: tunnelS,
          transform: `scale(${interpolate(tunnelS, [0, 1], [0.9, 1])})`,
          background: theme.surface2,
          borderRadius: 20,
          padding: '40px 52px',
          border: `1px solid ${theme.border}`,
          boxShadow: '0 40px 80px rgba(0,0,0,0.5)',
          maxWidth: 700,
          width: '80%',
          display: 'flex',
          flexDirection: 'column',
          gap: 24,
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
          <div style={{ width: 52, height: 52, borderRadius: 14, background: `${theme.accent2}22`, border: `1px solid ${theme.accent2}44`, display: 'grid', placeItems: 'center', fontSize: 26 }}>
            🔒
          </div>
          <div>
            <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 20, fontWeight: 800 }}>SSH Tunnel active</div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 14, marginTop: 3 }}>
              Connections route through <span style={{ color: theme.accent2, fontFamily: theme.mono }}>bastion.prod.int</span>
            </div>
          </div>
          <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 8 }}>
            <div style={{ width: 10, height: 10, borderRadius: '50%', background: theme.accent2, opacity: dotPulse, boxShadow: `0 0 10px ${theme.accent2}` }} />
            <span style={{ color: theme.accent2, fontFamily: theme.mono, fontSize: 13, fontWeight: 700 }}>Tunneled</span>
          </div>
        </div>
        <HR />
        {[
          { label: 'Bastion host', val: 'bastion.prod.int:22' },
          { label: 'Target DB',    val: 'db.prod.internal:3306' },
          { label: 'Local port',   val: 'localhost:13306' },
        ].map(({ label, val }) => (
          <div key={label} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <span style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 15 }}>{label}</span>
            <span style={{ color: theme.text, fontFamily: theme.mono, fontSize: 15 }}>{val}</span>
          </div>
        ))}
      </div>

      <Caption step={4} title="SSH tunnels" sub="Reach private DBs through a bastion — no VPN required" delay={40} />
    </div>
  );
};

// ─── Outro ───────────────────────────────────────────────────────────────────
const Outro: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const t1 = spring({ frame,              fps, config: { damping: 160 } });
  const t2 = spring({ frame: frame - 18, fps, config: { damping: 160 } });
  const t3 = spring({ frame: frame - 32, fps, config: { damping: 160 } });

  return (
    <div style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 12 }}>
      <div style={{ opacity: t1, transform: `scale(${interpolate(t1, [0, 1], [0.6, 1])})`, fontSize: 80 }}>🗄️</div>
      <div style={{ opacity: t2, transform: `translateY(${interpolate(t2, [0, 1], [24, 0])}px)`, color: theme.text, fontFamily: theme.font, fontSize: 64, fontWeight: 800, textAlign: 'center' }}>
        Your database, at your fingertips.
      </div>
      <div style={{ opacity: t3, transform: `translateY(${interpolate(t3, [0, 1], [16, 0])}px)`, color: theme.textDim, fontFamily: theme.font, fontSize: 26, textAlign: 'center' }}>
        MySQL · Redis · MongoDB · ClickHouse · SSH Tunnels
      </div>
    </div>
  );
};

// ─── Root composition ────────────────────────────────────────────────────────
export const Database: React.FC = () => {
  return (
    <AbsoluteFill style={{ background: theme.bgGradient, fontFamily: theme.font }}>

      <Sequence durationInFrames={TITLE_DUR}>
        <TitleCard kicker="OTTO ADE" title="Database Explorer" subtitle="TablePlus-class browser, built in" />
      </Sequence>

      {/* Scenes inside the OttoWindow */}
      <Sequence from={S1_START} durationInFrames={S1_DUR + S2_DUR + S3_DUR + S4_DUR}>
        <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <OttoWindow title="Otto — Database Explorer">
            <Sequence durationInFrames={S1_DUR}>
              <Scene1Browse />
            </Sequence>
            <Sequence from={S1_DUR} durationInFrames={S2_DUR}>
              <Scene2Query />
            </Sequence>
            <Sequence from={S1_DUR + S2_DUR} durationInFrames={S3_DUR}>
              <Scene3Join />
            </Sequence>
            <Sequence from={S1_DUR + S2_DUR + S3_DUR} durationInFrames={S4_DUR}>
              <Scene4Tunnel />
            </Sequence>
          </OttoWindow>
        </AbsoluteFill>
      </Sequence>

      <Sequence from={OUTRO_START} durationInFrames={OUTRO_DUR}>
        <Outro />
      </Sequence>

    </AbsoluteFill>
  );
};
