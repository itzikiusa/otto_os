import React from 'react';
import { T, brand, fonts, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Caption,
  TitleCard,
  Chip,
  Card,
  Field,
  Button,
  Table,
  Terminal,
  TermLine,
  BarChart,
  MetricStat,
  StatusDot,
  useSpring,
  Icon,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  MESSAGE BROKERS — Kafka viewer walkthrough
//  5 scenes, ~620 frames @ 30 fps
//
//  1. Title card
//  2. Cluster header + topics table (connect Kafka, SSH bastion)
//  3. Peek panel + produce composer
//  4. Consumer-group lag + throughput BarChart + Schema Registry
//  5. WalkOutro
// ════════════════════════════════════════════════════════════════════════════

// ── Scene 1 — Title ──────────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Message Brokers · Kafka"
    title="Kafka Brokers"
    subtitle="Connect clusters — peek messages, monitor lag, and tunnel through SSH bastions."
  />
);

// ── Scene 2 — Cluster header + topics ────────────────────────────────────────

const ClustersScene: React.FC = () => (
  <>
    <Stage scale={0.85}>
      <OttoWindow
        nav={<Navigator active="brokers" />}
        title="Otto — msk-prod"
      >
        <div
          style={{
            padding: 20,
            display: 'flex',
            flexDirection: 'column',
            gap: 16,
            height: '100%',
            boxSizing: 'border-box',
            overflow: 'hidden',
          }}
        >
          {/* Cluster header card */}
          <Appear delay={6}>
            <Card
              t={T}
              pad={16}
              style={{ display: 'flex', alignItems: 'center', gap: 16, flexShrink: 0 }}
            >
              <div
                style={{
                  width: 40,
                  height: 40,
                  borderRadius: 10,
                  background: alpha('#febc2e', 0.14),
                  border: `1px solid ${alpha('#febc2e', 0.32)}`,
                  display: 'grid',
                  placeItems: 'center',
                  flexShrink: 0,
                }}
              >
                <Icon name="box" size={20} color="#febc2e" />
              </div>
              <div style={{ flex: 1 }}>
                <div
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 18,
                    fontWeight: 700,
                    color: T.text,
                    letterSpacing: -0.3,
                  }}
                >
                  msk-prod
                </div>
                <div
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 12.5,
                    color: T.textDim,
                    marginTop: 3,
                  }}
                >
                  3 brokers · b-1.msk-prod.kafka.us-east-1.amazonaws.com:9092
                </div>
              </div>
              <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                <StatusDot kind="working" size={9} />
                <Chip t={T}>TLS</Chip>
                <Chip t={T}>SASL/SCRAM</Chip>
                <Chip t={T}>SSH bastion</Chip>
                <Chip tone="warn" t={T}>read-only</Chip>
              </div>
            </Card>
          </Appear>

          {/* Topics section label */}
          <Appear delay={14}>
            <div
              style={{
                fontFamily: fonts.ui,
                fontSize: 12.5,
                fontWeight: 600,
                color: T.textDim,
                textTransform: 'uppercase',
                letterSpacing: 0.05,
              }}
            >
              Topics
            </div>
          </Appear>

          {/* Topics table */}
          <Table
            t={T}
            delay={18}
            step={7}
            columns={['Topic', 'Partitions', 'Msgs/s', 'Retention']}
            widths={['2fr', '1fr', '1fr', '1fr']}
            rows={[
              ['wallet.tx', '12', '1.4k', '7d'],
              ['game.events', '24', '8.2k', '3d'],
              ['audit.log', '6', '120', '30d'],
              ['bonus.claims', '12', '340', '14d'],
              ['user.events', '8', '2.1k', '3d'],
            ]}
          />
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Connect Kafka — even AWS MSK over an SSH bastion"
      sub="Browse all topics · see partitions, throughput & retention at a glance"
    />
  </>
);

// ── Scene 3 — Peek + Produce ──────────────────────────────────────────────────

const PEEK_LINES: TermLine[] = [
  { text: '  Topic:     wallet.tx',                tone: 'dim' },
  { text: '  Partition: 3    Offset: 8,142,937',   tone: 'dim' },
  { text: '  Key:       "txn-8b3f2a19"',           tone: 'text' },
  { text: '  Timestamp: 2026-06-26T14:32:01.482Z', tone: 'dim' },
  { text: '' },
  { text: '  {',                                   tone: 'text' },
  { text: '    "playerId": "p-881029",',            tone: 'accent' },
  { text: '    "amount":   -250.00,',               tone: 'warn' },
  { text: '    "currency": "USD",',                 tone: 'text' },
  { text: '    "type":     "bet",',                 tone: 'text' },
  { text: '    "status":   "confirmed"',            tone: 'ok' },
  { text: '  }' },
];

const PeekScene: React.FC = () => (
  <>
    <Stage scale={0.85}>
      <OttoWindow
        nav={<Navigator active="brokers" />}
        title="Otto — msk-prod · wallet.tx · Partition 3"
      >
        <div style={{ display: 'flex', height: '100%' }}>
          {/* Message peek panel */}
          <div
            style={{
              flex: 3,
              display: 'flex',
              flexDirection: 'column',
              padding: 20,
              gap: 14,
              borderRight: `1px solid ${T.border}`,
            }}
          >
            <Appear delay={4}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <span
                  style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 700, color: T.text }}
                >
                  Message Peek
                </span>
                <Chip t={T}>offset 8,142,937</Chip>
                <div style={{ flex: 1 }} />
                <Button variant="ghost" t={T} size="s" icon="chevronLeft">Prev</Button>
                <Button variant="ghost" t={T} size="s" icon="chevronRight">Next</Button>
              </div>
            </Appear>
            <Terminal
              lines={PEEK_LINES}
              t={T}
              delay={10}
              step={5}
              fontSize={14}
              style={{ flex: 1 }}
            />
          </div>

          {/* Produce composer */}
          <div
            style={{
              flex: 2,
              padding: 20,
              display: 'flex',
              flexDirection: 'column',
              gap: 14,
            }}
          >
            <Appear delay={8}>
              <div style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 700, color: T.text }}>
                Produce Message
              </div>
            </Appear>
            <Appear delay={16}>
              <Field label="Topic" value="wallet.tx" t={T} icon="box" />
            </Appear>
            <Appear delay={22}>
              <Field label="Key (optional)" value="txn-new-8c9a" mono t={T} />
            </Appear>
            <Appear delay={28}>
              <Field
                label="Value (JSON)"
                value='{"playerId":"p-002211","amount":100,"type":"deposit"}'
                mono
                focused
                caret
                t={T}
              />
            </Appear>
            <Appear delay={34}>
              <div style={{ display: 'flex', gap: 10, alignItems: 'center', marginTop: 4 }}>
                <Chip t={T}>PLAINTEXT</Chip>
                <Chip t={T}>SASL PLAIN</Chip>
                <div style={{ flex: 1 }} />
                <Button variant="primary" t={T} icon="send">Produce</Button>
              </div>
            </Appear>
            <Appear delay={52} y={-10}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 10,
                  padding: '10px 14px',
                  borderRadius: 8,
                  marginTop: 4,
                  background: alpha('#28c840', 0.12),
                  border: `1px solid ${alpha('#28c840', 0.35)}`,
                  fontFamily: fonts.ui,
                  fontSize: 13,
                  color: '#28c840',
                }}
              >
                <Icon name="check" size={14} color="#28c840" />
                Delivered · partition 3 · offset 8,142,938
              </div>
            </Appear>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Peek & produce messages · PLAINTEXT/TLS · SASL PLAIN/SCRAM"
      sub="Explore offsets, inspect JSON payloads, and publish test messages — with prod/read-only guards."
    />
  </>
);

// ── Scene 4 — Consumer-group lag + Overview + Schema Registry ─────────────────

const THROUGHPUT_DATA = [420, 610, 980, 1240, 870, 1420, 1380, 1560, 1190, 1410, 1650, 1380];
const THROUGHPUT_LABELS = ['14:20', '', '14:24', '', '14:28', '', '14:32', '', '14:36', '', '14:40', '14:42'];

const OverviewScene: React.FC = () => {
  const grow = useSpring(14, { damping: 22, stiffness: 80 });
  return (
    <>
      <Stage scale={0.85}>
        <OttoWindow
          nav={<Navigator active="brokers" />}
          title="Otto — msk-prod · Overview"
        >
          <div style={{ display: 'flex', height: '100%', overflow: 'hidden' }}>
            {/* Left: consumer-group lag + schema registry */}
            <div
              style={{
                flex: 1,
                display: 'flex',
                flexDirection: 'column',
                padding: 20,
                gap: 14,
                borderRight: `1px solid ${T.border}`,
                overflow: 'hidden',
              }}
            >
              <Appear delay={4}>
                <div
                  style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 700, color: T.text }}
                >
                  Consumer-Group Lag
                </div>
              </Appear>
              <Table
                t={T}
                delay={8}
                step={8}
                columns={['Group', 'Topic', 'Lag']}
                widths={['2fr', '2fr', '1fr']}
                rows={[
                  ['payments-svc', 'wallet.tx', (
                    <span style={{ color: '#28c840', fontFamily: fonts.mono, fontSize: 13 }}>0</span>
                  )],
                  ['analytics-etl', 'game.events', (
                    <span style={{ color: '#febc2e', fontFamily: fonts.mono, fontSize: 13 }}>1,247</span>
                  )],
                  ['audit-archiver', 'audit.log', (
                    <span style={{ color: '#28c840', fontFamily: fonts.mono, fontSize: 13 }}>12</span>
                  )],
                  ['ml-feats', 'user.events', (
                    <span style={{ color: '#ff5f57', fontFamily: fonts.mono, fontSize: 13 }}>48,391</span>
                  )],
                ]}
              />

              <Appear delay={44}>
                <div
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 13,
                    fontWeight: 700,
                    color: T.text,
                    marginTop: 8,
                  }}
                >
                  Schema Registry
                </div>
              </Appear>
              <Appear delay={50}>
                <Card t={T} pad={14}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                    <div
                      style={{
                        width: 32,
                        height: 32,
                        borderRadius: 8,
                        background: alpha(brand.violet, 0.15),
                        display: 'grid',
                        placeItems: 'center',
                        flexShrink: 0,
                      }}
                    >
                      <Icon name="file" size={16} color={brand.violet} />
                    </div>
                    <div style={{ flex: 1 }}>
                      <div style={{ fontFamily: fonts.mono, fontSize: 13, color: T.text }}>
                        wallet.tx-value
                      </div>
                      <div
                        style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim, marginTop: 2 }}
                      >
                        Avro · v4 · 1,204 bytes
                      </div>
                    </div>
                    <Chip tone="accent" t={T}>AVRO</Chip>
                  </div>
                  <div
                    style={{
                      marginTop: 10,
                      fontFamily: fonts.mono,
                      fontSize: 11.5,
                      color: T.textDim,
                      lineHeight: 1.55,
                    }}
                  >
                    {'{"type":"record","name":"WalletTx","fields":['}<br />
                    {'  {"name":"playerId","type":"string"},'}<br />
                    {'  {"name":"amount","type":"double"},'}<br />
                    {'  {"name":"type","type":"string"}, …]}'}
                  </div>
                </Card>
              </Appear>
            </div>

            {/* Right: throughput chart + cluster metrics */}
            <div
              style={{
                flex: 1,
                padding: 20,
                display: 'flex',
                flexDirection: 'column',
                gap: 14,
                overflow: 'hidden',
              }}
            >
              <Appear delay={6}>
                <div
                  style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 700, color: T.text }}
                >
                  Throughput — msgs/s
                </div>
              </Appear>
              <Appear delay={10}>
                <BarChart
                  t={T}
                  data={THROUGHPUT_DATA}
                  labels={THROUGHPUT_LABELS}
                  color={brand.cyan}
                  height={180}
                  grow={grow}
                />
              </Appear>
              <div style={{ display: 'flex', gap: 12, marginTop: 4, flexWrap: 'wrap' }}>
                <Appear delay={60}>
                  <MetricStat
                    t={T}
                    label="In (msgs/s)"
                    value="1,650"
                    delta="+18% vs 1h"
                    deltaTone="ok"
                    accent={brand.cyan}
                  />
                </Appear>
                <Appear delay={66}>
                  <MetricStat t={T} label="Total msgs" value="2.4B" />
                </Appear>
                <Appear delay={72}>
                  <MetricStat t={T} label="Partitions" value="62" />
                </Appear>
              </div>
              <Appear delay={80}>
                <div
                  style={{
                    display: 'flex',
                    gap: 10,
                    alignItems: 'center',
                    flexWrap: 'wrap',
                    marginTop: 4,
                  }}
                >
                  <Chip tone="ok" t={T}>3 / 3 brokers up</Chip>
                  <Chip t={T}>ISR: 3</Chip>
                  <Chip tone="warn" t={T}>1 high-lag group</Chip>
                </div>
              </Appear>
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={3}
        title="Consumer-group lag, throughput overview & Schema Registry"
        sub="Spot lag spikes, track cluster health, and browse Avro schemas — all in one tab."
      />
    </>
  );
};

// ── Scene list ────────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 70,  node: <TitleScene />,    name: 'Title' },
  { dur: 160, node: <ClustersScene />, name: 'Clusters' },
  { dur: 140, node: <PeekScene />,     name: 'Peek' },
  { dur: 120, node: <OverviewScene />, name: 'Overview' },
  {
    dur: 130,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Message Brokers"
        tagline="Kafka, end to end — even behind a bastion"
        pills={[
          { label: 'Kafka · MSK', icon: 'box' },
          { label: 'Peek / Produce', icon: 'send' },
          { label: 'Consumer lag', icon: 'chart' },
          { label: 'Schema Registry', icon: 'file' },
        ]}
      />
    ),
  },
];

export const brokersDuration = scenesDuration(SCENES);
export const Brokers: React.FC = () => <Scenes scenes={SCENES} />;
