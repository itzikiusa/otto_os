import React from 'react';
import {
  AbsoluteFill,
  Sequence,
  useCurrentFrame,
  useVideoConfig,
  interpolate,
  spring,
  staticFile,
  Img,
} from 'remotion';
import { theme } from '../theme';
import { OttoWindow } from '../components/OttoWindow';
import { Navigator } from '../components/Navigator';
import { Appear, Caption, Cursor, TitleCard } from '../components/ui';

// ─── Timing constants (frames @ 30 fps) ───────────────────────────────────────
const TITLE_DUR   = 75;   // ~2.5 s title card
const LIST_DUR    = 210;  // ~7 s  scene 1 – connections list
const FORM_DUR    = 300;  // ~10 s scene 2 – new connection form
const TEST_DUR    = 120;  // ~4 s  scene 3 – test spinner → result
const TERM_DUR    = 195;  // ~6.5 s scene 4 – terminal session
const OUTRO_DUR   = 100;  // ~3.3 s outro

// Scene start offsets
const S1_START = TITLE_DUR;
const S2_START = S1_START + LIST_DUR;
const S3_START = S2_START + FORM_DUR;
const S4_START = S3_START + TEST_DUR;
const OUTRO_START = S4_START + TERM_DUR;

// ─── Helpers ──────────────────────────────────────────────────────────────────

/** Typewriter effect – returns substring of `text` for the current local frame */
function typewriter(text: string, frame: number, cps = 18): string {
  const chars = Math.floor((frame / 30) * cps);
  return text.slice(0, Math.min(chars, text.length));
}

/** Thin horizontal rule */
const HR: React.FC = () => (
  <div style={{ height: 1, background: theme.border, margin: '0 0 0 0' }} />
);

// ─── Kind chip ────────────────────────────────────────────────────────────────
const KIND_COLORS: Record<string, string> = {
  mysql:      '#4a8fff',
  ssh:        '#9ee039',
  redis:      '#e85050',
  mongodb:    '#4cbb87',
  clickhouse: '#febc2e',
};

const KindChip: React.FC<{ kind: string }> = ({ kind }) => (
  <span
    style={{
      fontFamily: theme.mono,
      fontSize: 13,
      fontWeight: 700,
      color: KIND_COLORS[kind] ?? theme.textDim,
      background: `${KIND_COLORS[kind] ?? theme.textDim}22`,
      border: `1px solid ${KIND_COLORS[kind] ?? theme.textDim}44`,
      borderRadius: 6,
      padding: '2px 8px',
      letterSpacing: 0.5,
      textTransform: 'uppercase',
    }}
  >
    {kind}
  </span>
);

// ─── Connection card ──────────────────────────────────────────────────────────
const KIND_ICONS: Record<string, string> = {
  mysql:      '🗄️',
  ssh:        '🔒',
  redis:      '⚡',
  mongodb:    '🍃',
  clickhouse: '🖥️',
};

const ConnCard: React.FC<{
  name: string;
  kind: string;
  host: string;
  user: string;
  highlighted?: boolean;
  delay?: number;
}> = ({ name, kind, host, user, highlighted, delay = 0 }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const s = spring({ frame: frame - delay, fps, config: { damping: 180 } });
  return (
    <div
      style={{
        opacity: s,
        transform: `translateY(${interpolate(s, [0, 1], [20, 0])}px)`,
        display: 'flex',
        alignItems: 'center',
        gap: 20,
        padding: '18px 24px',
        background: highlighted ? `rgba(61,91,255,0.12)` : 'rgba(255,255,255,0.025)',
        border: `1px solid ${highlighted ? theme.accent : theme.border}`,
        borderRadius: 14,
        cursor: 'pointer',
      }}
    >
      <div
        style={{
          width: 48,
          height: 48,
          borderRadius: 12,
          background: `${KIND_COLORS[kind] ?? theme.textDim}22`,
          border: `1px solid ${KIND_COLORS[kind] ?? theme.textDim}33`,
          display: 'grid',
          placeItems: 'center',
          fontSize: 24,
          flexShrink: 0,
        }}
      >
        {KIND_ICONS[kind] ?? '🔌'}
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 20, fontWeight: 700 }}>
          {name}
        </div>
        <div style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 14, marginTop: 3 }}>
          {user}@{host}
        </div>
      </div>
      <KindChip kind={kind} />
      <div
        style={{
          width: 8,
          height: 8,
          borderRadius: '50%',
          background: theme.textDim,
        }}
      />
    </div>
  );
};

// ─── Scene 1 – Connections list ───────────────────────────────────────────────
const Scene1List: React.FC<{ showCaption?: boolean }> = ({ showCaption = true }) => {
  const connections = [
    { name: 'prod-db',  kind: 'mysql',   host: 'db.prod.internal',  user: 'app' },
    { name: 'bastion',  kind: 'ssh',     host: 'bastion.prod.int',  user: 'ubuntu' },
    { name: 'cache',    kind: 'redis',   host: 'redis.svc.local',   user: 'default' },
  ];

  return (
    <div style={{ height: '100%', display: 'flex', flexDirection: 'column', padding: 32, gap: 0 }}>
      {/* header row */}
      <Appear delay={4}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 28 }}>
          <div>
            <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 32, fontWeight: 800 }}>
              Connections
            </div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 18, marginTop: 4 }}>
              Saved profiles — secrets stored in Keychain
            </div>
          </div>
          <Appear delay={18}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                background: theme.accent,
                color: '#fff',
                fontFamily: theme.font,
                fontSize: 18,
                fontWeight: 700,
                padding: '12px 24px',
                borderRadius: 12,
                boxShadow: `0 8px 28px ${theme.accent}55`,
              }}
            >
              <span style={{ fontSize: 20 }}>+</span> New Connection
            </div>
          </Appear>
        </div>
      </Appear>

      <HR />

      {/* cards */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 14, marginTop: 24 }}>
        {connections.map((c, i) => (
          <ConnCard key={c.name} delay={24 + i * 16} {...c} />
        ))}
      </div>

      {showCaption && <Caption step={1} title="Saved connections" sub="SSH, MySQL, Redis — all in one place" delay={60} />}
    </div>
  );
};

// ─── Scene 2 – New connection form ────────────────────────────────────────────
const KINDS = ['mysql', 'redis', 'ssh', 'mongodb', 'clickhouse'];

const FormField: React.FC<{
  label: string;
  value: string;
  delay?: number;
  mono?: boolean;
  placeholder?: string;
  focused?: boolean;
}> = ({ label, value, delay = 0, mono, placeholder, focused }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const s = spring({ frame: frame - delay, fps, config: { damping: 200 } });
  return (
    <div style={{ opacity: s, transform: `translateY(${interpolate(s, [0, 1], [10, 0])}px)` }}>
      <div
        style={{
          color: theme.textDim,
          fontFamily: theme.font,
          fontSize: 13,
          fontWeight: 600,
          letterSpacing: 0.5,
          textTransform: 'uppercase',
          marginBottom: 6,
        }}
      >
        {label}
      </div>
      <div
        style={{
          background: theme.surface2,
          border: `1px solid ${focused ? theme.accent : theme.border}`,
          borderRadius: 10,
          padding: '10px 16px',
          fontFamily: mono ? theme.mono : theme.font,
          fontSize: 18,
          color: value ? theme.text : theme.textDim,
          boxShadow: focused ? `0 0 0 3px ${theme.accent}33` : 'none',
          minHeight: 44,
        }}
      >
        {value || (
          <span style={{ color: theme.textDim, opacity: 0.5 }}>{placeholder}</span>
        )}
      </div>
    </div>
  );
};

const Scene2Form: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  // Local frame within scene
  const lf = frame;

  // Typewriter fields – staggered
  const HOST_START   = 30;
  const USER_START   = 90;
  const PASS_START   = 150;
  const CMD_START    = 190;
  const DROP_START   = 60; // dropdown appears
  const DROP_OPEN    = 80; // dropdown opens to show options

  const hostVal  = lf >= HOST_START  ? typewriter('db.analytics.prod.internal', lf - HOST_START,  16) : '';
  const userVal  = lf >= USER_START  ? typewriter('analytics_ro', lf - USER_START,  20) : '';
  const passVal  = lf >= PASS_START  ? typewriter('••••••••••••', lf - PASS_START,  16) : '';
  const cmdVal   = lf >= CMD_START   ? typewriter('USE casino;', lf - CMD_START,    18) : '';

  // Dropdown open animation
  const dropOpen = lf >= DROP_OPEN;
  const dropS    = spring({ frame: lf - DROP_OPEN, fps, config: { damping: 200 } });

  // Selected kind animates through options when dropdown opens
  const selectedKind = lf >= DROP_OPEN + 30 ? 'mysql' : 'mysql';

  return (
    <div style={{ position: 'absolute', inset: 0 }}>
      {/* dim background list */}
      <div style={{ position: 'absolute', inset: 0, background: 'rgba(10,12,16,0.6)', zIndex: 1 }} />

      {/* centering layer */}
      <div
        style={{
          position: 'absolute',
          inset: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          padding: '32px 48px',
          zIndex: 10,
        }}
      >
      {/* modal */}
      <div
        style={{
          width: '100%',
          maxWidth: 660,
          background: theme.surface,
          border: `1px solid ${theme.border}`,
          borderRadius: 18,
          boxShadow: '0 40px 120px rgba(0,0,0,0.7)',
          padding: '32px 36px 28px',
        }}
      >
        {/* modal header */}
        <Appear delay={0}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginBottom: 28 }}>
            <div
              style={{
                width: 44,
                height: 44,
                borderRadius: 12,
                background: `${theme.accent}22`,
                border: `1px solid ${theme.accent}44`,
                display: 'grid',
                placeItems: 'center',
                fontSize: 22,
              }}
            >
              🔌
            </div>
            <div>
              <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 22, fontWeight: 800 }}>
                New Connection
              </div>
              <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 14, marginTop: 2 }}>
                Fill in the details — password saved to Keychain
              </div>
            </div>
          </div>
        </Appear>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
          {/* Name field */}
          <FormField label="Name" value="analytics-db" delay={4} />

          {/* Kind dropdown */}
          <div
            style={{
              opacity: spring({ frame: lf - DROP_START, fps, config: { damping: 200 } }),
              position: 'relative',
            }}
          >
            <div
              style={{
                color: theme.textDim,
                fontFamily: theme.font,
                fontSize: 13,
                fontWeight: 600,
                letterSpacing: 0.5,
                textTransform: 'uppercase',
                marginBottom: 6,
              }}
            >
              Kind
            </div>
            <div
              style={{
                background: theme.surface2,
                border: `1px solid ${dropOpen ? theme.accent : theme.border}`,
                borderRadius: 10,
                padding: '10px 16px',
                fontFamily: theme.font,
                fontSize: 18,
                color: theme.text,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                boxShadow: dropOpen ? `0 0 0 3px ${theme.accent}33` : 'none',
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <span>{KIND_ICONS[selectedKind]}</span>
                <span>{selectedKind}</span>
              </div>
              <span style={{ color: theme.textDim, fontSize: 14 }}>▾</span>
            </div>

            {/* Dropdown options */}
            {dropOpen && (
              <div
                style={{
                  position: 'absolute',
                  top: '100%',
                  left: 0,
                  right: 0,
                  marginTop: 4,
                  background: theme.surface,
                  border: `1px solid ${theme.border}`,
                  borderRadius: 12,
                  overflow: 'hidden',
                  zIndex: 20,
                  boxShadow: '0 20px 60px rgba(0,0,0,0.5)',
                  opacity: dropS,
                  transform: `scaleY(${interpolate(dropS, [0, 1], [0.7, 1])})`,
                  transformOrigin: 'top',
                }}
              >
                {KINDS.map((k, idx) => (
                  <div
                    key={k}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 12,
                      padding: '10px 16px',
                      background: k === selectedKind ? `rgba(61,91,255,0.12)` : 'transparent',
                      borderBottom: idx < KINDS.length - 1 ? `1px solid ${theme.border}` : 'none',
                    }}
                  >
                    <span>{KIND_ICONS[k]}</span>
                    <span style={{ fontFamily: theme.font, fontSize: 17, color: theme.text }}>{k}</span>
                    <KindChip kind={k} />
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* Host */}
          <FormField
            label="Host"
            value={hostVal}
            mono
            delay={HOST_START}
            focused={lf >= HOST_START && lf < USER_START}
          />

          {/* User + Password side by side */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
            <FormField
              label="User"
              value={userVal}
              mono
              delay={USER_START}
              focused={lf >= USER_START && lf < PASS_START}
            />
            <FormField
              label="Password"
              value={passVal}
              mono
              delay={PASS_START}
              focused={lf >= PASS_START && lf < CMD_START}
            />
          </div>

          {/* First command */}
          <FormField
            label="First command (optional)"
            value={cmdVal}
            mono
            placeholder="e.g. USE mydb;"
            delay={CMD_START}
            focused={lf >= CMD_START}
          />
        </div>

        {/* Buttons */}
        <Appear delay={20}>
          <div style={{ display: 'flex', gap: 12, marginTop: 28, justifyContent: 'flex-end' }}>
            <div
              style={{
                padding: '11px 22px',
                border: `1px solid ${theme.border}`,
                borderRadius: 10,
                color: theme.textDim,
                fontFamily: theme.font,
                fontSize: 17,
                fontWeight: 600,
              }}
            >
              Cancel
            </div>
            <div
              style={{
                padding: '11px 28px',
                background: theme.accent,
                borderRadius: 10,
                color: '#fff',
                fontFamily: theme.font,
                fontSize: 17,
                fontWeight: 700,
                boxShadow: `0 6px 20px ${theme.accent}55`,
              }}
            >
              Save & Test
            </div>
          </div>
        </Appear>
      </div>
      </div>{/* end centering layer */}

      {/* Cursor clicks "Save & Test" late in scene */}
      {lf >= 260 && (
        <Cursor
          from={[660, 540]}
          to={[700, 540]}
          startAt={260}
          duration={20}
          click
        />
      )}

      <Caption
        step={2}
        title="New connection form"
        sub="Kind, host, credentials, optional first command"
        delay={40}
      />
    </div>
  );
};

// ─── Scene 3 – Test result ─────────────────────────────────────────────────────
const Scene3Test: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const SPINNER_END = 48;
  const RESULT_START = 50;

  const spinnerAngle = frame < SPINNER_END ? (frame / 30) * 360 * 2 : 0;
  const resultS = spring({ frame: frame - RESULT_START, fps, config: { damping: 180 } });

  return (
    <div style={{ position: 'absolute', inset: 0 }}>
      {/* dim bg */}
      <div style={{ position: 'absolute', inset: 0, background: 'rgba(10,12,16,0.6)', zIndex: 1 }} />

      {/* centering layer */}
      <div
        style={{
          position: 'absolute',
          inset: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          padding: '32px 48px',
          zIndex: 10,
        }}
      >
      {/* modal */}
      <div
        style={{
          width: '100%',
          maxWidth: 520,
          background: theme.surface,
          border: `1px solid ${theme.border}`,
          borderRadius: 18,
          boxShadow: '0 40px 120px rgba(0,0,0,0.7)',
          padding: '44px 44px',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          gap: 20,
        }}
      >
        {frame < RESULT_START ? (
          /* spinner */
          <>
            <div
              style={{
                width: 64,
                height: 64,
                borderRadius: '50%',
                border: `4px solid ${theme.border}`,
                borderTopColor: theme.accent,
                transform: `rotate(${spinnerAngle}deg)`,
              }}
            />
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 20 }}>
              Testing connection…
            </div>
          </>
        ) : (
          /* success */
          <>
            <div
              style={{
                opacity: resultS,
                transform: `scale(${interpolate(resultS, [0, 1], [0.5, 1])})`,
                width: 72,
                height: 72,
                borderRadius: '50%',
                background: `${theme.accent2}22`,
                border: `2px solid ${theme.accent2}`,
                display: 'grid',
                placeItems: 'center',
                fontSize: 36,
              }}
            >
              ✓
            </div>
            <div
              style={{
                opacity: resultS,
                transform: `translateY(${interpolate(resultS, [0, 1], [10, 0])}px)`,
                textAlign: 'center',
              }}
            >
              <div
                style={{
                  color: theme.accent2,
                  fontFamily: theme.font,
                  fontSize: 26,
                  fontWeight: 800,
                }}
              >
                Connected
              </div>
              <div style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 18, marginTop: 8 }}>
                analytics-db · mysql · 24 ms
              </div>
            </div>
            <div
              style={{
                opacity: resultS,
                display: 'flex',
                gap: 12,
                marginTop: 8,
              }}
            >
              <div
                style={{
                  padding: '10px 20px',
                  border: `1px solid ${theme.border}`,
                  borderRadius: 10,
                  color: theme.textDim,
                  fontFamily: theme.font,
                  fontSize: 16,
                  fontWeight: 600,
                }}
              >
                Close
              </div>
              <div
                style={{
                  padding: '10px 24px',
                  background: theme.accent,
                  borderRadius: 10,
                  color: '#fff',
                  fontFamily: theme.font,
                  fontSize: 16,
                  fontWeight: 700,
                }}
              >
                Save
              </div>
            </div>
          </>
        )}
      </div>
      </div>{/* end centering layer */}

      <Caption
        step={3}
        title="Test connection"
        sub="✓ connected · 24 ms"
        delay={RESULT_START}
      />
    </div>
  );
};

// ─── Scene 4 – Terminal session ───────────────────────────────────────────────
const TERM_LINES = [
  { text: 'Connecting to db.prod.internal:3306…',    delay: 10,  color: '#8b97a8' },
  { text: 'SSL handshake OK',                         delay: 28,  color: '#8b97a8' },
  { text: 'Authenticated as app@db.prod.internal',   delay: 42,  color: '#9ee039' },
  { text: '',                                          delay: 48,  color: '' },
  { text: 'mysql> USE casino;',                       delay: 52,  color: '#e8edf4' },
  { text: 'Database changed',                         delay: 72,  color: '#8b97a8' },
  { text: '',                                          delay: 76,  color: '' },
  { text: 'mysql> SELECT COUNT(*) FROM MdlGm_tblPlayers;', delay: 80, color: '#e8edf4' },
  { text: '+----------+',                              delay: 112, color: '#4a8fff' },
  { text: '| COUNT(*) |',                              delay: 116, color: '#4a8fff' },
  { text: '+----------+',                              delay: 118, color: '#4a8fff' },
  { text: '|   482901 |',                              delay: 122, color: '#e8edf4' },
  { text: '+----------+',                              delay: 126, color: '#4a8fff' },
  { text: '1 row in set (0.04 sec)',                   delay: 132, color: '#8b97a8' },
  { text: '',                                          delay: 138, color: '' },
  { text: 'mysql> _',                                  delay: 140, color: '#e8edf4' },
];

const Scene4Terminal: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const TILE_START  = 8;
  const tileS = spring({ frame: frame - TILE_START, fps, config: { damping: 160 } });

  return (
    <div style={{ height: '100%', display: 'flex', flexDirection: 'column', padding: 32, gap: 0 }}>
      {/* header row matching list */}
      <Appear delay={0}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 24 }}>
          <div>
            <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 32, fontWeight: 800 }}>
              Connections
            </div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 18, marginTop: 4 }}>
              Saved profiles — secrets stored in Keychain
            </div>
          </div>
        </div>
      </Appear>

      <HR />

      {/* cards behind terminal */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 12, marginTop: 20 }}>
        <ConnCard name="prod-db"  kind="mysql" host="db.prod.internal" user="app" highlighted delay={0} />
        <ConnCard name="bastion"  kind="ssh"   host="bastion.prod.int" user="ubuntu" delay={0} />
        <ConnCard name="cache"    kind="redis" host="redis.svc.local"  user="default" delay={0} />
      </div>

      {/* terminal tile springs up */}
      <div
        style={{
          position: 'absolute',
          left: 48,
          right: 48,
          bottom: 48,
          opacity: tileS,
          transform: `translateY(${interpolate(tileS, [0, 1], [60, 0])}px) scale(${interpolate(tileS, [0, 1], [0.96, 1])})`,
          background: '#0d1117',
          border: `1px solid ${theme.border}`,
          borderRadius: 16,
          boxShadow: '0 30px 80px rgba(0,0,0,0.7)',
          overflow: 'hidden',
        }}
      >
        {/* terminal titlebar */}
        <div
          style={{
            height: 36,
            background: '#161b22',
            borderBottom: `1px solid ${theme.border}`,
            display: 'flex',
            alignItems: 'center',
            padding: '0 16px',
            gap: 8,
          }}
        >
          <div style={{ width: 11, height: 11, borderRadius: '50%', background: '#ff5f57' }} />
          <div style={{ width: 11, height: 11, borderRadius: '50%', background: '#febc2e' }} />
          <div style={{ width: 11, height: 11, borderRadius: '50%', background: '#28c840' }} />
          <div style={{ flex: 1 }} />
          <span style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 13 }}>
            prod-db — mysql · db.prod.internal
          </span>
          <div style={{ flex: 1 }} />
          <KindChip kind="mysql" />
        </div>

        {/* terminal body */}
        <div
          style={{
            padding: '16px 24px 20px',
            minHeight: 260,
            display: 'flex',
            flexDirection: 'column',
            gap: 2,
          }}
        >
          {TERM_LINES.map((line, i) => {
            const lineS = spring({ frame: frame - line.delay, fps, config: { damping: 200 } });
            if (!line.text) return <div key={i} style={{ height: 8 }} />;
            return (
              <div
                key={i}
                style={{
                  opacity: lineS,
                  transform: `translateX(${interpolate(lineS, [0, 1], [-6, 0])}px)`,
                  fontFamily: theme.mono,
                  fontSize: 18,
                  color: line.color || theme.text,
                  lineHeight: 1.55,
                  whiteSpace: 'pre',
                }}
              >
                {line.text}
              </div>
            );
          })}
        </div>
      </div>

      {/* cursor clicks prod-db card */}
      {frame >= 4 && frame < 32 && (
        <Cursor
          from={[900, 200]}
          to={[700, 250]}
          startAt={4}
          duration={18}
          click
        />
      )}

      <Caption step={4} title="Open connection → live terminal" sub="First command runs automatically" delay={80} />
    </div>
  );
};

// ─── Outro ────────────────────────────────────────────────────────────────────
const Outro: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const s  = spring({ frame, fps, config: { damping: 180 } });
  const s2 = spring({ frame: frame - 18, fps, config: { damping: 180 } });
  const s3 = spring({ frame: frame - 32, fps, config: { damping: 180 } });

  return (
    <div
      style={{
        position: 'absolute',
        inset: 0,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 12,
      }}
    >
      <div
        style={{
          opacity: s,
          transform: `scale(${interpolate(s, [0, 1], [0.7, 1])})`,
        }}
      >
        <Img
          src={staticFile('otto-mark.png')}
          style={{ width: 120, height: 120, borderRadius: 28, boxShadow: `0 24px 72px ${theme.accent}55` }}
        />
      </div>
      <div
        style={{
          opacity: s2,
          transform: `translateY(${interpolate(s2, [0, 1], [20, 0])}px)`,
          color: theme.text,
          fontFamily: theme.font,
          fontSize: 72,
          fontWeight: 800,
          marginTop: 16,
          textAlign: 'center',
        }}
      >
        Your stack, one click away.
      </div>
      <div
        style={{
          opacity: s3,
          transform: `translateY(${interpolate(s3, [0, 1], [16, 0])}px)`,
          color: theme.textDim,
          fontFamily: theme.font,
          fontSize: 28,
          textAlign: 'center',
          maxWidth: 700,
        }}
      >
        SSH · MySQL · Redis · MongoDB · ClickHouse
      </div>
    </div>
  );
};

// ─── Root composition ─────────────────────────────────────────────────────────
export const Connections: React.FC = () => {
  return (
    <AbsoluteFill style={{ background: theme.bgGradient, fontFamily: theme.font }}>

      {/* Scene 0 – Title card */}
      <Sequence durationInFrames={TITLE_DUR}>
        <TitleCard kicker="OTTO ADE" title="Connections" subtitle="Your stack, one click away" />
      </Sequence>

      {/* Window shell present for scenes 1–4 */}
      <Sequence from={S1_START} durationInFrames={LIST_DUR + FORM_DUR + TEST_DUR + TERM_DUR}>
        <AbsoluteFill
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          <OttoWindow sidebar={<Navigator active="connections" />}>
            {/* Scene 1 – list */}
            <Sequence durationInFrames={LIST_DUR}>
              <Scene1List />
            </Sequence>

            {/* Scene 2 – form */}
            <Sequence from={LIST_DUR} durationInFrames={FORM_DUR}>
              {/* dim base layer showing the list (no caption to avoid overlap) */}
              <div style={{ position: 'absolute', inset: 0 }}>
                <Scene1List showCaption={false} />
              </div>
              <Scene2Form />
            </Sequence>

            {/* Scene 3 – test */}
            <Sequence from={LIST_DUR + FORM_DUR} durationInFrames={TEST_DUR}>
              {/* dim base layer (no caption to avoid overlap) */}
              <div style={{ position: 'absolute', inset: 0 }}>
                <Scene1List showCaption={false} />
              </div>
              <Scene3Test />
            </Sequence>

            {/* Scene 4 – terminal */}
            <Sequence from={LIST_DUR + FORM_DUR + TEST_DUR} durationInFrames={TERM_DUR}>
              <Scene4Terminal />
            </Sequence>
          </OttoWindow>
        </AbsoluteFill>
      </Sequence>

      {/* Outro */}
      <Sequence from={OUTRO_START} durationInFrames={OUTRO_DUR}>
        <Outro />
      </Sequence>

    </AbsoluteFill>
  );
};
