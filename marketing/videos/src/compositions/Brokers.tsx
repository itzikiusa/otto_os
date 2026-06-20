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

// ─── Message Brokers (Kafka) walkthrough — ~38s ───────────────────────────────
// Scenes: connect cluster (incl. AWS MSK over SSH bastion) → topics table →
// peek/produce messages → consumer groups → schema registry overview tiles.
// ─────────────────────────────────────────────────────────────────────────────

const TITLE_DUR  = 75;
const S1_DUR     = 195;  // cluster list + form
const S2_DUR     = 195;  // topics table
const S3_DUR     = 150;  // peek / produce
const S4_DUR     = 120;  // consumer groups
const S5_DUR     = 120;  // overview tiles
const OUTRO_DUR  = 90;

const S1_START   = TITLE_DUR;
const S2_START   = S1_START + S1_DUR;
const S3_START   = S2_START + S2_DUR;
const S4_START   = S3_START + S3_DUR;
const S5_START   = S4_START + S4_DUR;
const OUTRO_START = S5_START + S5_DUR;

// ─── Helpers ─────────────────────────────────────────────────────────────────

function typewriter(text: string, frame: number, cps = 20): string {
  const chars = Math.floor((frame / 30) * cps);
  return text.slice(0, Math.min(chars, text.length));
}

const HR: React.FC = () => (
  <div style={{ height: 1, background: theme.border }} />
);

const Badge: React.FC<{ color: string; children: React.ReactNode }> = ({ color, children }) => (
  <span style={{ fontFamily: theme.mono, fontSize: 12, fontWeight: 700, color, background: `${color}22`, border: `1px solid ${color}44`, borderRadius: 6, padding: '2px 8px', letterSpacing: 0.5 }}>
    {children}
  </span>
);

// ─── Cluster sidebar ─────────────────────────────────────────────────────────
const CLUSTERS = [
  { name: 'prod-msk',      env: 'prod', color: theme.danger },
  { name: 'analytics-msk', env: 'stg',  color: theme.warn },
  { name: 'dev-kafka',     env: 'dev',  color: theme.accent2 },
];

const ClusterSidebar: React.FC<{ active?: string }> = ({ active = 'prod-msk' }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  return (
    <div style={{ width: 240, background: theme.surface, borderRight: `1px solid ${theme.border}`, display: 'flex', flexDirection: 'column', height: '100%', flexShrink: 0 }}>
      <div style={{ padding: '14px 16px 10px', borderBottom: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <span style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 11, fontWeight: 700, letterSpacing: 1.2, textTransform: 'uppercase' }}>Clusters</span>
        <div style={{ width: 22, height: 22, borderRadius: 6, background: `${theme.accent}22`, border: `1px solid ${theme.accent}44`, display: 'grid', placeItems: 'center', color: theme.accent, fontSize: 16, fontWeight: 700 }}>+</div>
      </div>
      <div style={{ padding: '8px 0', flex: 1 }}>
        {CLUSTERS.map((c, i) => {
          const s = spring({ frame: frame - i * 10, fps, config: { damping: 200 } });
          const isActive = c.name === active;
          return (
            <div key={c.name} style={{ opacity: s, transform: `translateX(${interpolate(s, [0, 1], [-10, 0])}px)`, display: 'flex', alignItems: 'center', gap: 10, padding: '9px 16px', background: isActive ? `${theme.accent}18` : 'transparent', borderLeft: isActive ? `2px solid ${theme.accent}` : '2px solid transparent' }}>
              <div style={{ width: 8, height: 8, borderRadius: '50%', background: c.color, flexShrink: 0 }} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ color: isActive ? theme.text : theme.textDim, fontFamily: theme.mono, fontSize: 13, fontWeight: isActive ? 600 : 400 }}>{c.name}</div>
              </div>
              <Badge color={c.color}>{c.env}</Badge>
            </div>
          );
        })}
      </div>
    </div>
  );
};

// ─── Scene 1 – Cluster list + form showing SSH bastion option ─────────────────
const Scene1ClusterForm: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const showForm = frame >= 60;
  const formS = spring({ frame: frame - 60, fps, config: { damping: 180 } });

  const brokerVal = frame >= 70 ? typewriter('b-0123abcd.kafka.us-east-1.amazonaws.com:9094', frame - 70, 26) : '';
  const bastionVal = frame >= 120 ? typewriter('bastion.prod.internal', frame - 120, 20) : '';

  return (
    <div style={{ display: 'flex', height: '100%' }}>
      <ClusterSidebar active="prod-msk" />
      <div style={{ flex: 1, position: 'relative', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        {showForm && (
          <div style={{ opacity: formS, transform: `translateY(${interpolate(formS, [0, 1], [20, 0])}px)`, width: 640, background: theme.surface, border: `1px solid ${theme.border}`, borderRadius: 18, boxShadow: '0 40px 100px rgba(0,0,0,0.7)', padding: '32px 36px' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginBottom: 28 }}>
              <div style={{ width: 44, height: 44, borderRadius: 12, background: `${theme.warn}22`, border: `1px solid ${theme.warn}44`, display: 'grid', placeItems: 'center', fontSize: 22 }}>📨</div>
              <div>
                <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 20, fontWeight: 800 }}>New Kafka Cluster</div>
                <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13, marginTop: 2 }}>Connect via direct or SSH-tunneled bootstrap</div>
              </div>
            </div>

            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              {/* Name */}
              <div>
                <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 11, fontWeight: 700, letterSpacing: 0.8, textTransform: 'uppercase', marginBottom: 6 }}>Name</div>
                <div style={{ background: theme.surface2, border: `1px solid ${theme.border}`, borderRadius: 8, padding: '10px 14px', fontFamily: theme.font, fontSize: 16, color: theme.text }}>prod-msk</div>
              </div>

              {/* Broker */}
              <div>
                <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 11, fontWeight: 700, letterSpacing: 0.8, textTransform: 'uppercase', marginBottom: 6 }}>Bootstrap brokers</div>
                <div style={{ background: theme.surface2, border: `1px solid ${theme.accent}`, borderRadius: 8, padding: '10px 14px', fontFamily: theme.mono, fontSize: 14, color: theme.text, boxShadow: `0 0 0 3px ${theme.accent}22`, minHeight: 44 }}>
                  {brokerVal}
                </div>
              </div>

              {/* SSH Bastion toggle */}
              <div style={{ background: `${theme.accent2}11`, border: `1px solid ${theme.accent2}33`, borderRadius: 12, padding: '16px 18px' }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 12 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                    <span style={{ fontSize: 18 }}>🔒</span>
                    <span style={{ color: theme.accent2, fontFamily: theme.font, fontSize: 15, fontWeight: 700 }}>AWS MSK via SSH bastion</span>
                  </div>
                  <div style={{ width: 48, height: 26, borderRadius: 13, background: theme.accent2, position: 'relative', boxShadow: `0 0 12px ${theme.accent2}55` }}>
                    <div style={{ position: 'absolute', top: 4, left: 26, width: 18, height: 18, borderRadius: '50%', background: '#fff' }} />
                  </div>
                </div>
                <div>
                  <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 11, fontWeight: 700, letterSpacing: 0.8, textTransform: 'uppercase', marginBottom: 6 }}>Bastion host</div>
                  <div style={{ background: theme.surface, border: `1px solid ${theme.border}`, borderRadius: 8, padding: '9px 14px', fontFamily: theme.mono, fontSize: 14, color: theme.text, minHeight: 40 }}>
                    {bastionVal}
                  </div>
                </div>
              </div>
            </div>

            <div style={{ display: 'flex', gap: 12, marginTop: 24, justifyContent: 'flex-end' }}>
              <div style={{ padding: '10px 22px', border: `1px solid ${theme.border}`, borderRadius: 10, color: theme.textDim, fontFamily: theme.font, fontSize: 15, fontWeight: 600 }}>Cancel</div>
              <div style={{ padding: '10px 24px', background: theme.accent, borderRadius: 10, color: '#fff', fontFamily: theme.font, fontSize: 15, fontWeight: 700, boxShadow: `0 6px 20px ${theme.accent}55` }}>Save & Test</div>
            </div>
          </div>
        )}
      </div>

      <Caption step={1} title="Connect a Kafka cluster" sub="Direct or through an SSH bastion — AWS MSK supported" delay={70} />
    </div>
  );
};

// ─── Scene 2 – Topics table ─────────────────────────────────────────────────────
const TOPICS = [
  { name: 'player.events',        partitions: 12, msgs: '1.2M', lag: '0', retention: '7d' },
  { name: 'transaction.created',  partitions: 24, msgs: '4.8M', lag: '14', retention: '14d' },
  { name: 'bonus.granted',        partitions: 6,  msgs: '220K', lag: '0', retention: '3d' },
  { name: 'session.heartbeat',    partitions: 8,  msgs: '9.1M', lag: '320', retention: '1d' },
  { name: 'audit.log',            partitions: 4,  msgs: '82K',  lag: '0', retention: '90d' },
];

const Scene2Topics: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  return (
    <div style={{ display: 'flex', height: '100%' }}>
      <ClusterSidebar active="prod-msk" />
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {/* tab bar */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 0, borderBottom: `1px solid ${theme.border}`, padding: '0 20px', height: 40, flexShrink: 0 }}>
          {(['Topics', 'Groups', 'Schema'] as const).map((t) => (
            <div key={t} style={{ display: 'flex', alignItems: 'center', padding: '0 18px', height: '100%', borderBottom: t === 'Topics' ? `2px solid ${theme.accent}` : '2px solid transparent', color: t === 'Topics' ? theme.text : theme.textDim, fontFamily: theme.font, fontSize: 14, fontWeight: t === 'Topics' ? 700 : 400 }}>
              {t}
            </div>
          ))}
        </div>

        {/* search + header */}
        <Appear delay={4}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 16, padding: '12px 20px', borderBottom: `1px solid ${theme.border}` }}>
            <div style={{ flex: 1, background: theme.surface2, borderRadius: 8, padding: '7px 14px', display: 'flex', alignItems: 'center', gap: 8, border: `1px solid ${theme.border}` }}>
              <span style={{ color: theme.textDim, fontSize: 14 }}>⌕</span>
              <span style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 13 }}>filter topics…</span>
            </div>
            <div style={{ padding: '7px 18px', background: theme.accent, borderRadius: 8, color: '#fff', fontFamily: theme.font, fontSize: 13, fontWeight: 700, boxShadow: `0 4px 14px ${theme.accent}44` }}>+ New topic</div>
          </div>
        </Appear>

        {/* column headers */}
        <Appear delay={8}>
          <div style={{ display: 'grid', gridTemplateColumns: '280px 100px 120px 80px 100px', padding: '0 20px', background: theme.surface2, borderBottom: `1px solid ${theme.border}` }}>
            {['Topic', 'Partitions', 'Messages', 'Lag', 'Retention'].map((h) => (
              <div key={h} style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 12, fontWeight: 700, padding: '9px 8px 9px 0', textTransform: 'uppercase', letterSpacing: 0.5 }}>{h}</div>
            ))}
          </div>
        </Appear>

        {/* rows */}
        <div style={{ flex: 1, overflow: 'hidden' }}>
          {TOPICS.map((t, i) => {
            const s = spring({ frame: frame - (i * 14 + 18), fps, config: { damping: 200 } });
            const hasLag = parseInt(t.lag) > 0;
            return (
              <div key={t.name} style={{ opacity: s, transform: `translateX(${interpolate(s, [0, 1], [12, 0])}px)`, display: 'grid', gridTemplateColumns: '280px 100px 120px 80px 100px', padding: '0 20px', borderBottom: `1px solid ${theme.border}22` }}>
                <div style={{ padding: '11px 8px 11px 0', color: theme.accent, fontFamily: theme.mono, fontSize: 13 }}>{t.name}</div>
                <div style={{ padding: '11px 8px 11px 0', color: theme.text, fontFamily: theme.mono, fontSize: 13 }}>{t.partitions}</div>
                <div style={{ padding: '11px 8px 11px 0', color: theme.text, fontFamily: theme.mono, fontSize: 13 }}>{t.msgs}</div>
                <div style={{ padding: '11px 8px 11px 0' }}>
                  {hasLag
                    ? <span style={{ color: theme.warn, fontFamily: theme.mono, fontSize: 13, fontWeight: 700 }}>{t.lag}</span>
                    : <span style={{ color: theme.accent2, fontFamily: theme.mono, fontSize: 13 }}>0</span>}
                </div>
                <div style={{ padding: '11px 8px 11px 0', color: theme.textDim, fontFamily: theme.mono, fontSize: 13 }}>{t.retention}</div>
              </div>
            );
          })}
        </div>
      </div>

      <Caption step={2} title="Conduktor-style topics table" sub="Lag, partitions, retention — scan your cluster at a glance" delay={55} />
    </div>
  );
};

// ─── Scene 3 – Peek / produce messages ────────────────────────────────────────
const PEEK_MSGS = [
  { offset: '9184230', key: 'player:482901', value: '{"event":"login","ts":"2024-01-20T10:14:32Z"}' },
  { offset: '9184231', key: 'player:482902', value: '{"event":"bet","amount":50.0,"gameId":"slots-x99"}' },
  { offset: '9184232', key: 'player:482903', value: '{"event":"win","amount":250.0,"gameId":"slots-x99"}' },
];

const Scene3PeekProduce: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const PRODUCE_START = 80;
  const showProduce = frame >= PRODUCE_START;
  const proS = spring({ frame: frame - PRODUCE_START, fps, config: { damping: 180 } });
  const msgVal = frame >= PRODUCE_START + 14 ? typewriter('{"event":"bonus_granted","playerId":482901,"amount":100}', frame - (PRODUCE_START + 14), 22) : '';

  return (
    <div style={{ display: 'flex', height: '100%' }}>
      <ClusterSidebar active="prod-msk" />
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        <div style={{ padding: '14px 20px', borderBottom: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', gap: 10, flexShrink: 0 }}>
          <span style={{ color: theme.accent, fontFamily: theme.mono, fontSize: 13, fontWeight: 700 }}>player.events</span>
          <Badge color={theme.textDim}>partition 0</Badge>
          <Badge color={theme.textDim}>offset 9184230…</Badge>
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
            <div style={{ padding: '6px 14px', borderRadius: 8, border: `1px solid ${theme.border}`, color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>⟳ Peek latest</div>
            <div style={{ padding: '6px 14px', borderRadius: 8, background: theme.accent, color: '#fff', fontFamily: theme.font, fontSize: 13, fontWeight: 700, boxShadow: `0 4px 12px ${theme.accent}44` }}>Produce</div>
          </div>
        </div>

        {/* peek messages */}
        <div style={{ flex: 1, overflow: 'hidden', padding: '12px 0' }}>
          {PEEK_MSGS.map((msg, i) => {
            const s = spring({ frame: frame - i * 14, fps, config: { damping: 200 } });
            return (
              <div key={msg.offset} style={{ opacity: s, transform: `translateX(${interpolate(s, [0, 1], [10, 0])}px)`, padding: '10px 20px', borderBottom: `1px solid ${theme.border}22`, display: 'flex', flexDirection: 'column', gap: 4 }}>
                <div style={{ display: 'flex', gap: 20 }}>
                  <span style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 12, width: 90 }}>#{msg.offset}</span>
                  <span style={{ color: theme.accent, fontFamily: theme.mono, fontSize: 12 }}>key: {msg.key}</span>
                </div>
                <div style={{ color: theme.accent2, fontFamily: theme.mono, fontSize: 13, paddingLeft: 110 }}>
                  {msg.value}
                </div>
              </div>
            );
          })}
        </div>

        {/* produce overlay */}
        {showProduce && (
          <div style={{ opacity: proS, transform: `translateY(${interpolate(proS, [0, 1], [24, 0])}px)`, background: theme.surface, borderTop: `1px solid ${theme.border}`, padding: '20px 24px', boxShadow: '0 -20px 60px rgba(0,0,0,0.5)' }}>
            <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 15, fontWeight: 700, marginBottom: 12 }}>Produce message</div>
            <div style={{ background: theme.surface2, border: `1px solid ${theme.accent}`, borderRadius: 10, padding: '12px 16px', fontFamily: theme.mono, fontSize: 14, color: theme.accent2, boxShadow: `0 0 0 3px ${theme.accent}22`, minHeight: 52 }}>
              {msgVal}
            </div>
            <div style={{ marginTop: 12, display: 'flex', justifyContent: 'flex-end' }}>
              <div style={{ padding: '9px 24px', background: theme.accent, borderRadius: 10, color: '#fff', fontFamily: theme.font, fontSize: 14, fontWeight: 700, boxShadow: `0 4px 16px ${theme.accent}44` }}>Send</div>
            </div>
          </div>
        )}
      </div>

      <Caption step={3} title="Peek + produce messages" sub="Inspect any offset or send a test message" delay={50} />
    </div>
  );
};

// ─── Scene 4 – Consumer groups ────────────────────────────────────────────────
const GROUPS = [
  { name: 'player-service-cg',   members: 3, lag: 0,   state: 'Stable' },
  { name: 'analytics-pipeline',  members: 6, lag: 14,  state: 'Stable' },
  { name: 'audit-consumer',      members: 1, lag: 320, state: 'Rebalancing' },
];

const Scene4Groups: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  return (
    <div style={{ display: 'flex', height: '100%' }}>
      <ClusterSidebar active="prod-msk" />
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 0, borderBottom: `1px solid ${theme.border}`, padding: '0 20px', height: 40, flexShrink: 0 }}>
          {(['Topics', 'Groups', 'Schema'] as const).map((t) => (
            <div key={t} style={{ display: 'flex', alignItems: 'center', padding: '0 18px', height: '100%', borderBottom: t === 'Groups' ? `2px solid ${theme.accent}` : '2px solid transparent', color: t === 'Groups' ? theme.text : theme.textDim, fontFamily: theme.font, fontSize: 14, fontWeight: t === 'Groups' ? 700 : 400 }}>
              {t}
            </div>
          ))}
        </div>
        <Appear delay={4}>
          <div style={{ display: 'grid', gridTemplateColumns: '280px 100px 100px 140px', padding: '0 20px', background: theme.surface2, borderBottom: `1px solid ${theme.border}` }}>
            {['Group', 'Members', 'Total lag', 'State'].map((h) => (
              <div key={h} style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 12, fontWeight: 700, padding: '9px 8px 9px 0', textTransform: 'uppercase', letterSpacing: 0.5 }}>{h}</div>
            ))}
          </div>
        </Appear>
        <div style={{ flex: 1, overflow: 'hidden' }}>
          {GROUPS.map((g, i) => {
            const s = spring({ frame: frame - (i * 14 + 18), fps, config: { damping: 200 } });
            const lagColor = g.lag > 100 ? theme.danger : g.lag > 0 ? theme.warn : theme.accent2;
            const stateColor = g.state === 'Rebalancing' ? theme.warn : theme.accent2;
            return (
              <div key={g.name} style={{ opacity: s, transform: `translateX(${interpolate(s, [0, 1], [12, 0])}px)`, display: 'grid', gridTemplateColumns: '280px 100px 100px 140px', padding: '0 20px', borderBottom: `1px solid ${theme.border}22` }}>
                <div style={{ padding: '11px 8px 11px 0', color: theme.accent, fontFamily: theme.mono, fontSize: 13 }}>{g.name}</div>
                <div style={{ padding: '11px 8px 11px 0', color: theme.text, fontFamily: theme.mono, fontSize: 13 }}>{g.members}</div>
                <div style={{ padding: '11px 8px 11px 0' }}>
                  <span style={{ color: lagColor, fontFamily: theme.mono, fontSize: 13, fontWeight: g.lag > 0 ? 700 : 400 }}>{g.lag}</span>
                </div>
                <div style={{ padding: '11px 8px 11px 0' }}>
                  <Badge color={stateColor}>{g.state}</Badge>
                </div>
              </div>
            );
          })}
        </div>
      </div>

      <Caption step={4} title="Consumer groups + lag" sub="Spot rebalancing and growing lag before it's a problem" delay={50} />
    </div>
  );
};

// ─── Scene 5 – Overview tiles ─────────────────────────────────────────────────
const Scene5Overview: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const tiles = [
    { icon: '📨', label: 'Topics',   val: '47',    color: theme.accent },
    { icon: '👥', label: 'Groups',   val: '12',    color: theme.accent2 },
    { icon: '⚠️', label: 'Total lag', val: '334',  color: theme.warn },
    { icon: '🗂', label: 'Schema IDs', val: '203', color: '#bf7aff' },
  ];

  return (
    <div style={{ display: 'flex', height: '100%' }}>
      <ClusterSidebar active="prod-msk" />
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', padding: 28, gap: 24, overflow: 'hidden' }}>
        <Appear delay={0}>
          <div>
            <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 22, fontWeight: 800 }}>Cluster overview</div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 14, marginTop: 4 }}>prod-msk · AWS MSK · us-east-1</div>
          </div>
        </Appear>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr 1fr', gap: 16 }}>
          {tiles.map((tile, i) => {
            const s = spring({ frame: frame - (i * 12 + 8), fps, config: { damping: 180 } });
            return (
              <div key={tile.label} style={{ opacity: s, transform: `scale(${interpolate(s, [0, 1], [0.88, 1])})`, background: theme.surface2, borderRadius: 14, padding: '24px 22px', border: `1px solid ${theme.border}` }}>
                <div style={{ fontSize: 26, marginBottom: 10 }}>{tile.icon}</div>
                <div style={{ color: tile.color, fontFamily: theme.mono, fontSize: 38, fontWeight: 800 }}>{tile.val}</div>
                <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13, marginTop: 4 }}>{tile.label}</div>
              </div>
            );
          })}
        </div>

        <Appear delay={50}>
          <div style={{ background: `#bf7aff11`, border: `1px solid #bf7aff33`, borderRadius: 12, padding: '18px 24px', display: 'flex', alignItems: 'center', gap: 16 }}>
            <span style={{ fontSize: 22 }}>🗂</span>
            <div>
              <div style={{ color: '#bf7aff', fontFamily: theme.font, fontSize: 15, fontWeight: 700 }}>Schema Registry</div>
              <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13, marginTop: 3 }}>203 schemas · Avro + JSON · compatibility BACKWARD</div>
            </div>
            <div style={{ marginLeft: 'auto', padding: '8px 18px', border: `1px solid #bf7aff55`, borderRadius: 8, color: '#bf7aff', fontFamily: theme.font, fontSize: 13, fontWeight: 600 }}>Browse schemas</div>
          </div>
        </Appear>
      </div>

      <Caption step={5} title="Overview + Schema Registry" sub="CPU, partitions, schema IDs — all in one panel" delay={40} />
    </div>
  );
};

// ─── Outro ────────────────────────────────────────────────────────────────────
const Outro: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const t1 = spring({ frame,              fps, config: { damping: 160 } });
  const t2 = spring({ frame: frame - 18, fps, config: { damping: 160 } });
  const t3 = spring({ frame: frame - 32, fps, config: { damping: 160 } });

  return (
    <div style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 12 }}>
      <div style={{ opacity: t1, transform: `scale(${interpolate(t1, [0, 1], [0.5, 1])})`, fontSize: 80 }}>📨</div>
      <div style={{ opacity: t2, transform: `translateY(${interpolate(t2, [0, 1], [24, 0])}px)`, color: theme.text, fontFamily: theme.font, fontSize: 60, fontWeight: 800, textAlign: 'center', lineHeight: 1.15 }}>
        Kafka, without the complexity.
      </div>
      <div style={{ opacity: t3, transform: `translateY(${interpolate(t3, [0, 1], [16, 0])}px)`, color: theme.textDim, fontFamily: theme.font, fontSize: 24, textAlign: 'center' }}>
        Topics · Groups · Peek · Produce · Schema Registry · SSH bastion
      </div>
    </div>
  );
};

// ─── Root composition ─────────────────────────────────────────────────────────
export const Brokers: React.FC = () => {
  return (
    <AbsoluteFill style={{ background: theme.bgGradient, fontFamily: theme.font }}>

      <Sequence durationInFrames={TITLE_DUR}>
        <TitleCard kicker="OTTO ADE" title="Message Brokers" subtitle="Kafka at your fingertips" />
      </Sequence>

      <Sequence from={S1_START} durationInFrames={S1_DUR + S2_DUR + S3_DUR + S4_DUR + S5_DUR}>
        <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <OttoWindow title="Otto — Message Brokers">
            <Sequence durationInFrames={S1_DUR}>
              <Scene1ClusterForm />
            </Sequence>
            <Sequence from={S1_DUR} durationInFrames={S2_DUR}>
              <Scene2Topics />
            </Sequence>
            <Sequence from={S1_DUR + S2_DUR} durationInFrames={S3_DUR}>
              <Scene3PeekProduce />
            </Sequence>
            <Sequence from={S1_DUR + S2_DUR + S3_DUR} durationInFrames={S4_DUR}>
              <Scene4Groups />
            </Sequence>
            <Sequence from={S1_DUR + S2_DUR + S3_DUR + S4_DUR} durationInFrames={S5_DUR}>
              <Scene5Overview />
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
