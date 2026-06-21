import React from 'react';
import { T, brand, fonts, alpha, status as STATUS } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Stagger,
  TitleCard,
  Caption,
  Chip,
  Button,
  Card,
  Field,
  Table,
  MetricStat,
  Icon,
  StatusDot,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  MESSAGE BROKERS — Kafka viewer walkthrough
//  Connect Kafka clusters (incl. AWS MSK over an SSH bastion), browse topics,
//  peek/produce messages, watch consumer lag & broker health, schema registry,
//  DLQ replay. Native-dark Otto chrome throughout.
// ════════════════════════════════════════════════════════════════════════════

const mono = fonts.mono;

// ── Scene 1 — title ──────────────────────────────────────────────────────────
const TitleScene: React.FC = () => (
  <TitleCard
    kicker="MESSAGE BROKERS"
    title="Kafka, even behind a bastion"
    subtitle="Topics, lag & schemas — MSK over SSH, safely"
  />
);

// ── shared: cluster sidebar list (left rail inside the content area) ─────────
const ClusterRow: React.FC<{
  name: string;
  badge: { label: string; color: string };
  tunnel?: boolean;
  active?: boolean;
}> = ({ name, badge, tunnel, active }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 9,
      padding: '0 11px',
      height: 42,
      borderRadius: 8,
      background: active ? alpha(brand.cyan, 0.12) : 'transparent',
      border: `1px solid ${active ? alpha(brand.cyan, 0.34) : 'transparent'}`,
    }}
  >
    <span
      style={{
        width: 26,
        height: 26,
        borderRadius: 7,
        background: alpha(badge.color, 0.16),
        color: badge.color,
        display: 'grid',
        placeItems: 'center',
        flexShrink: 0,
      }}
    >
      <Icon name="box" size={15} />
    </span>
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{ fontFamily: mono, fontSize: 13, fontWeight: 600, color: T.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {name}
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginTop: 3 }}>
        <span
          style={{
            fontFamily: fonts.ui,
            fontSize: 10,
            fontWeight: 700,
            letterSpacing: 0.4,
            textTransform: 'uppercase',
            color: badge.color,
            background: alpha(badge.color, 0.16),
            border: `1px solid ${alpha(badge.color, 0.4)}`,
            borderRadius: 5,
            padding: '1px 6px',
          }}
        >
          {badge.label}
        </span>
        {tunnel && (
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4, fontFamily: fonts.ui, fontSize: 10.5, color: brand.cyan }}>
            <Icon name="key" size={11} color={brand.cyan} />
            SSH tunnel
          </span>
        )}
      </div>
    </div>
  </div>
);

const ClusterList: React.FC = () => (
  <div
    style={{
      width: 280,
      flexShrink: 0,
      background: T.bgSidebar,
      borderRight: `1px solid ${T.border}`,
      display: 'flex',
      flexDirection: 'column',
      padding: '14px 12px',
      gap: 6,
    }}
  >
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '0 4px 6px' }}>
      <span style={{ fontFamily: fonts.ui, fontSize: 12, fontWeight: 700, letterSpacing: 0.4, textTransform: 'uppercase', color: T.textDim }}>
        Clusters
      </span>
      <Button variant="ghost" size="s" icon="plus" style={{ color: T.textDim }}>
        Add
      </Button>
    </div>
    <Stagger delay={14} step={6} y={10} style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <ClusterRow name="prod-msk" badge={{ label: 'prod', color: STATUS.exited }} tunnel active />
      <ClusterRow name="analytics-msk" badge={{ label: 'staging', color: STATUS.needsYou }} />
      <ClusterRow name="dev-kafka" badge={{ label: 'dev', color: STATUS.working }} />
    </Stagger>
    <div style={{ flex: 1 }} />
    <div style={{ borderTop: `1px solid ${T.border}`, paddingTop: 10, display: 'flex', alignItems: 'center', gap: 8 }}>
      <StatusDot kind="working" size={8} />
      <span style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim }}>
        prod-msk · <span style={{ color: STATUS.exited }}>read-only guarded</span>
      </span>
    </div>
  </div>
);

// ── Scene 2 — clusters + topics ──────────────────────────────────────────────
const ClustersScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="brokers" />}
        tabs={[{ label: 'prod-msk', icon: 'box', active: true }, { label: 'Topics', icon: 'grid' }]}
        title="Otto — Message Brokers"
      >
        <div style={{ display: 'flex', height: '100%' }}>
          <ClusterList />
          <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', padding: 18, gap: 14 }}>
            <Appear delay={6} y={10}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                <span style={{ fontFamily: fonts.ui, fontSize: 19, fontWeight: 750 as never, color: T.text }}>prod-msk</span>
                <Chip color={STATUS.exited}>prod</Chip>
                <Chip color={brand.cyan}>
                  <Icon name="key" size={11} color={brand.cyan} /> SSH bastion
                </Chip>
                <div style={{ flex: 1 }} />
                <Chip tone="default">9 brokers · 248 partitions</Chip>
              </div>
            </Appear>
            <Appear delay={12}>
              <Table
                columns={['Topic', 'Partitions', 'Replicas', 'Msgs/s', 'Size']}
                widths={['2.1fr', '1fr', '1fr', '1fr', '1fr']}
                rows={[
                  ['orders.v2', '24', '3', '4.2k', '128 GB'],
                  ['payments.events', '12', '3', '1.8k', '64 GB'],
                  ['player.activity', '48', '3', '11.4k', '512 GB'],
                  [
                    <span key="d" style={{ color: STATUS.needsYou }}>dlq.payments</span>,
                    '6',
                    '3',
                    '7',
                    '0.9 GB',
                  ],
                  ['inventory.changelog', '12', '3', '320', '22 GB'],
                ]}
                delay={18}
                step={5}
                fontSize={13.5}
              />
            </Appear>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Every cluster — MSK over an SSH bastion"
      sub="Topics, partitions & throughput · prod is read-only-guarded"
    />
  </>
);

// ── Scene 3 — peek / produce ─────────────────────────────────────────────────
const PeekRow: React.FC<{ offset: string; k: string; value: string; delay: number }> = ({ offset, k, value, delay }) => (
  <Appear delay={delay} y={10}>
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: '92px 168px 1fr',
        gap: 12,
        alignItems: 'flex-start',
        padding: '10px 12px',
        borderRadius: 8,
        background: T.surface,
        border: `1px solid ${T.border}`,
        fontFamily: mono,
        fontSize: 13,
      }}
    >
      <span style={{ color: T.textDim }}>
        <span style={{ color: alpha(T.textDim, 0.7), fontSize: 11 }}>off </span>
        {offset}
      </span>
      <span style={{ color: brand.cyan, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{k}</span>
      <span style={{ color: T.text, whiteSpace: 'pre-wrap', lineHeight: 1.5 }}>{value}</span>
    </div>
  </Appear>
);

const PeekProduceScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="brokers" />}
        tabs={[{ label: 'payments.events', icon: 'box', active: true }]}
        title="Otto — payments.events"
      >
        <div style={{ display: 'flex', height: '100%', padding: 18, gap: 16, boxSizing: 'border-box' }}>
          {/* peek panel */}
          <div style={{ flex: 1.55, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 12 }}>
            <Appear delay={4} y={8}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <Icon name="eye" size={16} color={T.textDim} />
                <span style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 700, color: T.text }}>Peek · payments.events</span>
                <div style={{ flex: 1 }} />
                <Chip tone="default">partition 3 · from latest</Chip>
              </div>
            </Appear>
            <PeekRow
              offset="48 213"
              k="pmt_9f3a2c"
              value={'{ "id": "pmt_9f3a2c", "amount": 49.90,\n  "currency": "EUR", "status": "captured" }'}
              delay={12}
            />
            <PeekRow
              offset="48 214"
              k="pmt_71be08"
              value={'{ "id": "pmt_71be08", "amount": 12.00,\n  "currency": "USD", "status": "refunded" }'}
              delay={20}
            />
          </div>
          {/* produce composer */}
          <Card pad={16} style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 12, background: T.bgSidebar }}>
            <Appear delay={10} y={8}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
                <span style={{ width: 26, height: 26, borderRadius: 7, background: alpha('#0a84ff', 0.16), color: '#0a84ff', display: 'grid', placeItems: 'center' }}>
                  <Icon name="send" size={14} />
                </span>
                <span style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 700, color: T.text }}>Produce a message</span>
              </div>
            </Appear>
            <Appear delay={18}>
              <Field label="Key" value="pmt_a04d11" mono icon="key" />
            </Appear>
            <Appear delay={24}>
              <Field
                label="Value (JSON)"
                value={'{ "id": "pmt_a04d11", "amount": 8.50, "currency": "EUR" }'}
                mono
                focused
                caret
              />
            </Appear>
            <Appear delay={30}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginTop: 2 }}>
                <Button variant="primary" icon="send">Produce → partition 3</Button>
                <Chip tone="ok">
                  <Icon name="check" size={11} color={STATUS.working} /> safe on guarded clusters
                </Chip>
              </div>
            </Appear>
          </Card>
        </div>
      </OttoWindow>
    </Stage>
    <Caption step={2} title="Peek and produce messages" sub="Decoded key/value · safe on guarded clusters" />
  </>
);

// ── Scene 4 — consumer lag + broker health ───────────────────────────────────
const lagCell = (n: string, warn?: boolean): React.ReactNode => (
  <span style={{ color: warn ? STATUS.needsYou : T.text, fontWeight: warn ? (700 as never) : (400 as never) }}>{n}</span>
);

const BrokerBar: React.FC<{ id: string; cpu: number; ram: number; delay: number }> = ({ id, cpu, ram, delay }) => (
  <Appear delay={delay} y={8}>
    <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
      <span style={{ width: 56, fontFamily: mono, fontSize: 12, color: T.textDim }}>{id}</span>
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 5 }}>
        <Meter label="CPU" pct={cpu} color={cpu > 80 ? STATUS.needsYou : brand.cyan} />
        <Meter label="RAM" pct={ram} color="#0a84ff" />
      </div>
    </div>
  </Appear>
);

const Meter: React.FC<{ label: string; pct: number; color: string }> = ({ label, pct, color }) => (
  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
    <span style={{ width: 30, fontFamily: fonts.ui, fontSize: 10.5, color: T.textDim }}>{label}</span>
    <div style={{ flex: 1, height: 7, borderRadius: 4, background: alpha(T.textDim, 0.16), overflow: 'hidden' }}>
      <div style={{ width: `${pct}%`, height: '100%', borderRadius: 4, background: `linear-gradient(90deg, ${color}, ${alpha(color, 0.5)})` }} />
    </div>
    <span style={{ width: 36, textAlign: 'right', fontFamily: mono, fontSize: 11, color: T.text }}>{pct}%</span>
  </div>
);

const ConsumerLagScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="brokers" />}
        tabs={[{ label: 'Consumer groups', icon: 'gauge', active: true }, { label: 'Brokers', icon: 'chart' }]}
        title="Otto — prod-msk · health"
      >
        <div style={{ display: 'flex', height: '100%', padding: 18, gap: 16, boxSizing: 'border-box' }}>
          {/* consumer groups */}
          <div style={{ flex: 1.5, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 12 }}>
            <Appear delay={4} y={8}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <Icon name="gauge" size={16} color={T.textDim} />
                <span style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 700, color: T.text }}>Consumer groups</span>
                <div style={{ flex: 1 }} />
                <Chip color={STATUS.needsYou}>
                  <Icon name="bell" size={11} color={STATUS.needsYou} /> Lag alert
                </Chip>
              </div>
            </Appear>
            <Appear delay={10}>
              <Table
                columns={['Group', 'Topic', 'Lag']}
                widths={['1.5fr', '1.6fr', '0.9fr']}
                rows={[
                  ['settlement-svc', 'payments.events', lagCell('142')],
                  ['ledger-writer', 'orders.v2', lagCell('38')],
                  ['risk-scoring', 'player.activity', lagCell('1.2M', true)],
                  ['email-notify', 'payments.events', lagCell('0')],
                ]}
                delay={16}
                step={5}
                fontSize={13.5}
              />
            </Appear>
            <Appear delay={34}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <Button icon="refresh">Reset offsets…</Button>
                <Chip tone="accent">
                  <Icon name="eye" size={11} color={T.accent} /> dry-run preview
                </Chip>
              </div>
            </Appear>
          </div>
          {/* per-broker health */}
          <Card pad={16} style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 12, background: T.bgSidebar }}>
            <Appear delay={8} y={8}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
                <Icon name="chart" size={15} color={T.textDim} />
                <span style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 700, color: T.text }}>Broker health</span>
              </div>
            </Appear>
            <Appear delay={14}>
              <div style={{ display: 'flex', gap: 10 }}>
                <MetricStat label="Throughput" value="17.4k/s" delta="▲ 6%" deltaTone="ok" style={{ flex: 1, minWidth: 0 }} />
                <MetricStat label="Brokers" value="9" accent={brand.cyan} style={{ flex: 1, minWidth: 0 }} />
              </div>
            </Appear>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 12, marginTop: 2 }}>
              <BrokerBar id="b-1" cpu={42} ram={61} delay={22} />
              <BrokerBar id="b-2" cpu={88} ram={74} delay={28} />
              <BrokerBar id="b-3" cpu={37} ram={55} delay={34} />
            </div>
          </Card>
        </div>
      </OttoWindow>
    </Stage>
    <Caption step={3} title="Consumer lag & broker health" sub="Per-partition lag · alerts · dry-run lag reset" />
  </>
);

// ── Scene 5 — schema registry + DLQ replay ───────────────────────────────────
const SchemaDlqScene: React.FC = () => (
  <>
    <Stage scale={0.92}>
      <OttoWindow
        nav={<Navigator active="brokers" />}
        tabs={[{ label: 'Schema Registry', icon: 'file', active: true }, { label: 'dlq.payments', icon: 'archive' }]}
        title="Otto — schemas & DLQ"
      >
        <div style={{ display: 'flex', height: '100%', padding: 22, gap: 18, boxSizing: 'border-box', alignItems: 'center' }}>
          {/* schema registry card */}
          <Appear delay={6} y={12} style={{ flex: 1, minWidth: 0 }}>
            <Card pad={18} style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <span style={{ width: 30, height: 30, borderRadius: 8, background: alpha(brand.violet, 0.18), color: brand.violet, display: 'grid', placeItems: 'center' }}>
                  <Icon name="file" size={16} />
                </span>
                <span style={{ fontFamily: fonts.ui, fontSize: 16, fontWeight: 750 as never, color: T.text }}>Schema Registry</span>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <span style={{ fontFamily: mono, fontSize: 14, color: T.text }}>payments.events</span>
                <Chip color={brand.violet}>v3</Chip>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>
                <Icon name="clock" size={12} color={T.textDim} /> versions v1 · v2 · v3 (Avro)
              </div>
              <Chip tone="ok">
                <Icon name="check" size={11} color={STATUS.working} /> BACKWARD compatible
              </Chip>
            </Card>
          </Appear>
          {/* DLQ replay card */}
          <Appear delay={14} y={12} style={{ flex: 1, minWidth: 0 }}>
            <Card pad={18} style={{ display: 'flex', flexDirection: 'column', gap: 14, background: T.bgSidebar }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <span style={{ width: 30, height: 30, borderRadius: 8, background: alpha(STATUS.needsYou, 0.18), color: STATUS.needsYou, display: 'grid', placeItems: 'center' }}>
                  <Icon name="archive" size={16} />
                </span>
                <span style={{ fontFamily: fonts.ui, fontSize: 16, fontWeight: 750 as never, color: T.text }}>Dead-letter queue</span>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <span style={{ fontFamily: mono, fontSize: 14, color: STATUS.needsYou }}>dlq.payments</span>
                <Chip color={STATUS.needsYou}>7 messages</Chip>
              </div>
              <div style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>
                Replay → <span style={{ color: T.text, fontFamily: mono }}>payments.events</span> with transform
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <Button variant="primary" icon="refresh">Replay DLQ</Button>
                <Chip tone="accent">
                  <Icon name="edit" size={11} color={T.accent} /> transform
                </Chip>
              </div>
            </Card>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption step={4} title="Schema Registry + DLQ replay" />
  </>
);

// ── compose ──────────────────────────────────────────────────────────────────
const SCENES: SceneDef[] = [
  { dur: 80, node: <TitleScene />, name: 'Title' },
  { dur: 220, node: <ClustersScene />, name: 'Clusters' },
  { dur: 200, node: <PeekProduceScene />, name: 'Peek/Produce' },
  { dur: 200, node: <ConsumerLagScene />, name: 'Consumer lag' },
  { dur: 70, node: <SchemaDlqScene />, name: 'Schema/DLQ' },
  {
    dur: 130,
    node: (
      <WalkOutro
        title="Message Brokers"
        tagline="Kafka you can actually see."
        pills={[
          { label: 'MSK over SSH', color: brand.cyan, icon: 'key' },
          { label: 'Peek/Produce', color: '#0a84ff', icon: 'box' },
          { label: 'Consumer lag', color: '#febc2e', icon: 'gauge' },
          { label: 'Schema Registry', color: brand.violet, icon: 'file' },
          { label: 'DLQ replay', color: '#28c840', icon: 'refresh' },
        ]}
      />
    ),
    name: 'Outro',
  },
];

export const brokersDuration = scenesDuration(SCENES);
export const Brokers: React.FC = () => <Scenes scenes={SCENES} />;
