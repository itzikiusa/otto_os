import React from 'react';
import { T, brand, fonts, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Caption,
  TitleCard,
  Terminal,
  TermLine,
  Chip,
  Card,
  Icon,
  StatusDot,
  Button,
  Table,
} from '../components/kit';

// ── Types & static data ───────────────────────────────────────────────────────

interface ConnEntry {
  name: string;
  type: string;
  icon: string;
  color: string;
  host: string;
  tag?: string;
}

const CONNS: ConnEntry[] = [
  { name: 'bastion-prod',      type: 'SSH',        icon: 'terminal', color: brand.cyan, host: 'bastion.example.internal:22' },
  { name: 'users-mysql',       type: 'MySQL',      icon: 'db',       color: '#f29111',  host: 'users-mysql.internal:3306' },
  { name: 'sessions-redis',    type: 'Redis',      icon: 'box',      color: '#d63b31',  host: 'redis-sessions.internal:6379' },
  { name: 'atlas-mongo',       type: 'MongoDB',    icon: 'globe',    color: '#10aa50',  host: 'cluster0.mongodb.net', tag: '+srv' },
  { name: 'events-clickhouse', type: 'ClickHouse', icon: 'chart',    color: '#febc2e',  host: 'ch-events.internal:8123' },
];

const SSH_LINES: TermLine[] = [
  { text: '$ ssh itzik@bastion-prod.example.internal',                           tone: 'cmd' },
  { text: 'Welcome to bastion-prod (Ubuntu 22.04.4 LTS)',                        tone: 'dim' },
  { text: 'Last login: Thu Jun 26 14:28:01 2025 from 10.0.1.42',                tone: 'dim' },
  { text: '$ uname -a',                                                           tone: 'cmd' },
  { text: 'Linux bastion-prod 6.5.0-1018-aws #18~22.04.1-Ubuntu SMP x86_64',    tone: 'text' },
  { text: '$ uptime',                                                             tone: 'cmd' },
  { text: ' 14:32:06 up 42 days,  3:11,  1 user,  load avg: 0.08, 0.05, 0.01', tone: 'ok' },
  { text: '$ df -h /var/www',                                                     tone: 'cmd' },
  { text: 'Filesystem       Size  Used Avail Use%  Mounted on',                  tone: 'dim' },
  { text: '/dev/nvme0n1p3    80G   22G   56G  28%  /var',                        tone: 'text' },
];

interface TunnelEntry {
  mode: string;
  flag: string;
  detail: string;
  note: string;
  color: string;
  delay: number;
}

const TUNNELS: TunnelEntry[] = [
  {
    mode: 'Local Forward',
    flag: '-L 3306:users-mysql.internal:3306',
    detail: 'Binds a local port to a remote host:port via the bastion.',
    note: 'bastion must have AllowTcpForwarding yes',
    color: brand.cyan,
    delay: 16,
  },
  {
    mode: 'SOCKS5 Dynamic',
    flag: '-D 1080',
    detail: 'Dynamic proxy — required for MongoDB +srv and Atlas.',
    note: '+srv resolves SRV DNS records that -L cannot handle',
    color: '#10aa50',
    delay: 32,
  },
];

const fileRow = (
  name: string,
  isDir: boolean,
  size: string,
  modified: string,
): (string | React.ReactNode)[] => [
  <div key={name} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
    <Icon name={isDir ? 'folder' : 'file'} size={13} color={isDir ? brand.cyan : T.textDim} />
    <span style={{ fontFamily: fonts.mono, fontSize: 13, color: T.text }}>{name}</span>
  </div>,
  size,
  modified,
];

const FILE_ROWS: (string | React.ReactNode)[][] = [
  fileRow('nginx',      true,  '—',      'Jun 25  08:14'),
  fileRow('releases',   true,  '—',      'Jun 26  02:41'),
  fileRow('uploads',    true,  '—',      'Jun 24  15:30'),
  fileRow('nginx.conf', false, '4.2 KB', 'Jun 25  08:14'),
  fileRow('server.crt', false, '1.8 KB', 'May 12  09:00'),
  fileRow('deploy.sh',  false, '2.1 KB', 'Jun 26  02:41'),
];

// ── Scene 1 — Title ───────────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Connections · SSH & SFTP"
    title="Connections"
    subtitle="SSH, tunnels, SFTP, and every datastore — one global library."
  />
);

// ── Scene 2 — Connection library ──────────────────────────────────────────────

const ConnRow: React.FC<{ c: ConnEntry; delay: number }> = ({ c, delay }) => (
  <Appear delay={delay} y={12}>
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        padding: '10px 16px',
        borderBottom: `1px solid ${T.border}`,
        background: T.surface,
      }}
    >
      {/* type icon */}
      <div
        style={{
          width: 34,
          height: 34,
          borderRadius: 9,
          background: alpha(c.color, 0.16),
          border: `1px solid ${alpha(c.color, 0.35)}`,
          display: 'grid',
          placeItems: 'center',
          flexShrink: 0,
        }}
      >
        <Icon name={c.icon} size={17} color={c.color} />
      </div>

      {/* name + type chips */}
      <div style={{ flex: '0 0 210px' }}>
        <div style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 600, color: T.text, marginBottom: 4 }}>
          {c.name}
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
          <Chip color={c.color}>{c.type}</Chip>
          {c.tag && <Chip>{c.tag}</Chip>}
        </div>
      </div>

      {/* host */}
      <div
        style={{
          flex: 1,
          fontFamily: fonts.mono,
          fontSize: 12.5,
          color: T.textDim,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {c.host}
      </div>

      {/* status + action */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexShrink: 0 }}>
        <StatusDot kind="idle" size={8} />
        <span style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>Saved</span>
        <Button size="s" variant="default" icon="terminal">Open</Button>
      </div>
    </div>
  </Appear>
);

const LibraryScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow nav={<Navigator active="connections" />} title="Otto — Connections">
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>

          {/* toolbar */}
          <Appear delay={4}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                padding: '8px 16px',
                borderBottom: `1px solid ${T.border}`,
                background: T.bg,
                flexShrink: 0,
              }}
            >
              <div
                style={{
                  flex: 1,
                  display: 'flex',
                  alignItems: 'center',
                  gap: 6,
                  height: 28,
                  padding: '0 10px',
                  borderRadius: 6,
                  background: T.surface2,
                  border: `1px solid ${T.border}`,
                  fontFamily: fonts.ui,
                  fontSize: 13,
                  color: T.textDim,
                }}
              >
                <Icon name="search" size={13} color={T.textDim} />
                Search connections…
              </div>
              <Button variant="primary" icon="plus" size="s">New Connection</Button>
            </div>
          </Appear>

          {/* connection rows */}
          <div style={{ flex: 1, overflow: 'hidden' }}>
            {CONNS.map((c, i) => (
              <ConnRow key={c.name} c={c} delay={12 + i * 14} />
            ))}
          </div>

          {/* keychain footer */}
          <Appear delay={86}>
            <div
              style={{
                padding: '9px 16px',
                borderTop: `1px solid ${T.border}`,
                background: T.bgSidebar,
                display: 'flex',
                alignItems: 'center',
                gap: 8,
              }}
            >
              <Icon name="key" size={13} color={brand.cyan} />
              <span style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>
                Credentials stored in the macOS Keychain — never in the database
              </span>
              <Chip tone="ok" style={{ marginLeft: 'auto' }}>Keychain secured</Chip>
            </div>
          </Appear>

        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="One global library — SSH, MySQL, Redis, Mongo, ClickHouse"
      sub="Secrets in the macOS Keychain. Not the database."
    />
  </>
);

// ── Scene 3 — SSH PTY + tunnel config ────────────────────────────────────────

const TunnelCard: React.FC<{ entry: TunnelEntry }> = ({ entry }) => (
  <Appear delay={entry.delay} y={14}>
    <Card
      pad={14}
      style={{
        marginBottom: 12,
        border: `1px solid ${alpha(entry.color, 0.32)}`,
        background: alpha(entry.color, 0.05),
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 7, marginBottom: 8 }}>
        <Icon name="link" size={13} color={entry.color} />
        <span
          style={{
            fontFamily: fonts.ui,
            fontSize: 11.5,
            fontWeight: 700,
            color: entry.color,
            textTransform: 'uppercase',
            letterSpacing: 0.6,
          }}
        >
          {entry.mode}
        </span>
      </div>
      <div
        style={{
          fontFamily: fonts.mono,
          fontSize: 12.5,
          color: T.text,
          background: T.termBg,
          padding: '5px 10px',
          borderRadius: 5,
          marginBottom: 8,
        }}
      >
        {entry.flag}
      </div>
      <div style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim, lineHeight: 1.5 }}>
        {entry.detail}
      </div>
      <div
        style={{
          fontFamily: fonts.ui,
          fontSize: 11,
          color: alpha(T.textDim, 0.65),
          marginTop: 4,
          fontStyle: 'italic',
        }}
      >
        {entry.note}
      </div>
    </Card>
  </Appear>
);

const SSHTunnelScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={<Navigator active="connections" />}
        tabs={[{ label: 'bastion-prod', icon: 'terminal', active: true, dot: 'working' }]}
        title="Otto — Connections"
        contentStyle={{ display: 'flex' }}
      >
        {/* left: live PTY */}
        <div
          style={{
            flex: '0 0 58%',
            display: 'flex',
            flexDirection: 'column',
            overflow: 'hidden',
            borderRight: `1px solid ${T.border}`,
          }}
        >
          <Terminal
            lines={SSH_LINES}
            delay={6}
            step={10}
            fontSize={13}
            style={{ flex: 1, borderRadius: 0 }}
          />
        </div>

        {/* right: tunnel config */}
        <div
          style={{
            flex: 1,
            background: T.bgSidebar,
            padding: 16,
            overflow: 'hidden',
            display: 'flex',
            flexDirection: 'column',
          }}
        >
          <Appear delay={10}>
            <div
              style={{
                fontFamily: fonts.ui,
                fontSize: 11.5,
                fontWeight: 600,
                color: T.textDim,
                textTransform: 'uppercase',
                letterSpacing: 0.7,
                marginBottom: 14,
              }}
            >
              Tunnels via bastion-prod
            </div>
          </Appear>
          {TUNNELS.map((entry, i) => (
            <TunnelCard key={i} entry={entry} />
          ))}
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="Open a live PTY — or tunnel a private DB through bastion"
      sub="Local forward (-L) for SQL · SOCKS5 (-D) for MongoDB +srv and Atlas"
    />
  </>
);

// ── Scene 4 — SFTP browser ────────────────────────────────────────────────────

const SFTPScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={<Navigator active="connections" />}
        tabs={[{ label: 'bastion-prod · SFTP', icon: 'folder', active: true }]}
        title="Otto — Connections"
      >
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            height: '100%',
            padding: 16,
            gap: 12,
            boxSizing: 'border-box',
          }}
        >
          {/* breadcrumb */}
          <Appear delay={4}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 6,
                fontFamily: fonts.mono,
                fontSize: 12.5,
                color: T.textDim,
              }}
            >
              <Icon name="globe" size={13} color={T.textDim} />
              <span>bastion-prod.example.internal</span>
              <Icon name="chevronRight" size={12} color={T.textDim} />
              <span style={{ color: T.text }}>/var/www</span>
            </div>
          </Appear>

          {/* file table */}
          <Table
            columns={['Name', 'Size', 'Modified']}
            rows={FILE_ROWS}
            widths={['1fr', '90px', '130px']}
            delay={10}
            step={7}
            style={{ flex: 1 }}
          />

          {/* actions */}
          <Appear delay={58}>
            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <Button size="s" icon="arrowDown" variant="default">Download</Button>
              <Button size="s" icon="arrowUp" variant="primary">Upload</Button>
              <span style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim, marginLeft: 6 }}>
                Drag files here to transfer
              </span>
            </div>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={4}
      title="Browse and transfer files over SFTP"
      sub="Full file browser on any SSH connection — download, upload, drag and drop."
    />
  </>
);

// ── Scene list ────────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 80,  node: <TitleScene />,     name: 'Title' },
  { dur: 170, node: <LibraryScene />,   name: 'Library' },
  { dur: 170, node: <SSHTunnelScene />, name: 'SSHTunnel' },
  { dur: 120, node: <SFTPScene />,      name: 'SFTP' },
  {
    dur: 130,
    node: (
      <WalkOutro
        title="Connections"
        tagline="Every server and datastore, one click away"
        pills={[
          { label: 'SSH · SQL · Redis · Mongo · CH', icon: 'plug' },
          { label: 'Tunnels (-L / -D)',               icon: 'link' },
          { label: 'SFTP',                            icon: 'folder' },
          { label: 'Keychain',                        icon: 'key' },
        ]}
      />
    ),
    name: 'Outro',
  },
];

export const connectionsDuration = scenesDuration(SCENES);
export const Connections: React.FC = () => <Scenes scenes={SCENES} />;
