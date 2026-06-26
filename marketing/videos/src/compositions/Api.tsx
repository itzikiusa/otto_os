import React from 'react';
import { T, brand, fonts, alpha, status } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow, RightPanel } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Stagger,
  Caption,
  TitleCard,
  Chip,
  Button,
  Card,
  Field,
  Segmented,
  Terminal,
  TermLine,
  Toast,
  Icon,
} from '../components/kit';

// ── Shared sub-components ────────────────────────────────────────────────────

const FolderItem: React.FC<{ label: string; count: number; open?: boolean }> = ({
  label,
  count,
  open,
}) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 6,
      height: 28,
      padding: '0 10px',
      fontFamily: fonts.ui,
      fontSize: 12.5,
      color: T.text,
    }}
  >
    <Icon name={open ? 'chevronDown' : 'chevronRight'} size={11} color={T.textDim} />
    <Icon name="folder" size={13} color={brand.cyan} />
    <span style={{ flex: 1 }}>{label}</span>
    <span style={{ fontFamily: fonts.mono, fontSize: 10.5, color: T.textDim }}>{count}</span>
  </div>
);

const RequestItem: React.FC<{
  method: string;
  label: string;
  color: string;
  active?: boolean;
}> = ({ method, label, color, active }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 6,
      height: 24,
      padding: '0 10px 0 28px',
      background: active ? alpha(T.accent, 0.12) : 'transparent',
      borderRadius: 4,
      fontFamily: fonts.ui,
      fontSize: 12,
      color: active ? T.text : T.textDim,
    }}
  >
    <span
      style={{ fontFamily: fonts.mono, fontSize: 10, color, fontWeight: 700, minWidth: 28 }}
    >
      {method}
    </span>
    <span
      style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
    >
      {label}
    </span>
  </div>
);

const HRow: React.FC<{ k: string; v: string }> = ({ k, v }) => (
  <div style={{ display: 'flex', gap: 12, alignItems: 'center', minHeight: 24 }}>
    <span style={{ fontFamily: fonts.mono, fontSize: 12.5, color: T.textDim, minWidth: 160 }}>
      {k}
    </span>
    <span
      style={{
        fontFamily: fonts.mono,
        fontSize: 12.5,
        color: v.includes('{{') ? brand.cyan : T.text,
      }}
    >
      {v}
    </span>
  </div>
);

const HistoryRow: React.FC<{
  method: string;
  path: string;
  code: string;
  ms: string;
}> = ({ method, path, code, ms }) => {
  const methodColor =
    method === 'GET'
      ? status.working
      : method === 'POST'
      ? brand.purple
      : method === 'DELETE'
      ? status.exited
      : T.textDim;
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 6,
        padding: '5px 8px',
        borderRadius: 5,
        fontFamily: fonts.mono,
        fontSize: 11.5,
      }}
    >
      <span style={{ color: methodColor, fontWeight: 700, minWidth: 42, flexShrink: 0 }}>
        {method}
      </span>
      <span
        style={{
          flex: 1,
          color: T.textDim,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {path}
      </span>
      <span
        style={{
          color: code.startsWith('2') ? status.working : status.exited,
          minWidth: 28,
          flexShrink: 0,
        }}
      >
        {code}
      </span>
      <span
        style={{
          color: T.textDim,
          minWidth: 38,
          textAlign: 'right',
          flexShrink: 0,
        }}
      >
        {ms}
      </span>
    </div>
  );
};

// ── Scene data ────────────────────────────────────────────────────────────────

const responseLines: TermLine[] = [
  { text: 'HTTP/1.1 200 OK  (142 ms)', tone: 'ok' },
  { text: 'Content-Type: application/json; charset=utf-8', tone: 'dim' },
  { text: 'X-Request-Id: req_7f3a2b1d9c', tone: 'dim' },
  { text: '', tone: 'text' },
  { text: '{', tone: 'text' },
  { text: '  "data": [', tone: 'text' },
  { text: '    { "id": "plr_8f3a2", "email": "alice@acme.dev", "status": "active" },', tone: 'text' },
  { text: '    { "id": "plr_9c1b4", "email": "bob@acme.dev",   "status": "active" }', tone: 'text' },
  { text: '  ],', tone: 'text' },
  { text: '  "total": 2,', tone: 'text' },
  { text: '  "page": 1,', tone: 'text' },
  { text: '  "elapsed_ms": 142', tone: 'dim' },
  { text: '}', tone: 'text' },
];

const historyRows: [string, string, string, string][] = [
  ['GET',    '/v1/players?status=active', '200', '142ms'],
  ['POST',   '/v1/auth/token',            '200',  '88ms'],
  ['GET',    '/v1/wallet/balance',        '200',  '67ms'],
  ['DELETE', '/v1/sessions/7a9b',         '204',  '31ms'],
  ['GET',    '/v1/players/plr_8f3a2',     '200',  '54ms'],
];

// ── Scene 1 — Title (80f) ─────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="API Client"
    title="REST Workbench"
    subtitle="A Postman-class request builder, wired right into Otto"
  />
);

// ── Scene 2 — Request Builder (160f) ─────────────────────────────────────────

const RequestBuilderScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow nav={<Navigator active="api" />} title="Otto — API">
        <div style={{ display: 'flex', height: '100%', overflow: 'hidden' }}>
          {/* Collections sidebar */}
          <div
            style={{
              width: 224,
              flexShrink: 0,
              background: T.bgSidebar,
              borderRight: `1px solid ${T.border}`,
              display: 'flex',
              flexDirection: 'column',
              overflow: 'hidden',
            }}
          >
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                padding: '10px 10px 8px',
                borderBottom: `1px solid ${T.border}`,
                flexShrink: 0,
              }}
            >
              <span
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 11,
                  fontWeight: 600,
                  textTransform: 'uppercase',
                  letterSpacing: 0.6,
                  color: T.textDim,
                }}
              >
                Collections
              </span>
              <Icon name="plus" size={13} color={T.textDim} />
            </div>

            <div style={{ flex: 1, padding: '6px 0', overflow: 'hidden' }}>
              <Stagger delay={8} step={6} y={8}>
                <FolderItem label="Auth" count={4} />
                <FolderItem label="Players" count={7} open />
                <RequestItem method="GET" label="List players" color={status.working} active />
                <RequestItem method="POST" label="Create player" color={brand.purple} />
                <RequestItem method="GET" label="Get player" color={status.working} />
                <FolderItem label="Wallet" count={5} />
              </Stagger>
            </div>
          </div>

          {/* Request editor */}
          <div
            style={{
              flex: 1,
              display: 'flex',
              flexDirection: 'column',
              padding: 20,
              gap: 16,
              overflow: 'hidden',
            }}
          >
            {/* URL bar */}
            <Appear delay={16} y={14}>
              <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                <Chip
                  tone="ok"
                  style={{
                    fontFamily: fonts.mono,
                    fontSize: 12,
                    fontWeight: 700,
                    letterSpacing: 0.2,
                  }}
                >
                  GET
                </Chip>
                <Field
                  value="https://api.acme.dev/v1/players?status=active"
                  mono
                  focused
                  style={{ flex: 1 }}
                />
                <Button variant="primary" icon="send">
                  Send
                </Button>
              </div>
            </Appear>

            {/* Environment selector */}
            <Appear delay={28} y={10}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                <span style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>
                  Environment
                </span>
                <Segmented options={['dev', 'staging', 'prod']} active={0} />
              </div>
            </Appear>

            {/* Headers */}
            <Appear delay={40} y={10}>
              <Card>
                <div
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 10.5,
                    fontWeight: 600,
                    textTransform: 'uppercase',
                    letterSpacing: 0.6,
                    color: T.textDim,
                    marginBottom: 12,
                  }}
                >
                  Headers
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 9 }}>
                  <HRow k="Authorization" v="Bearer {{token}}" />
                  <HRow k="Content-Type" v="application/json" />
                  <HRow k="X-Tenant-Id" v="{{tenant_id}}" />
                </div>
              </Card>
            </Appear>

            {/* Query params */}
            <Appear delay={54} y={10}>
              <Card style={{ background: alpha(T.accent, 0.05) }}>
                <div
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 10.5,
                    fontWeight: 600,
                    textTransform: 'uppercase',
                    letterSpacing: 0.6,
                    color: T.textDim,
                    marginBottom: 10,
                  }}
                >
                  Query Params
                </div>
                <HRow k="status" v="active" />
              </Card>
            </Appear>
          </div>
        </div>
      </OttoWindow>
    </Stage>

    <Caption
      step={1}
      title="A Postman-class workbench — collections, environments, {{vars}}"
      sub="Organize requests in named collections. Switch envs in one click. Variables resolve at send time."
    />
  </>
);

// ── Scene 3 — Response + History (130f) ──────────────────────────────────────

const ResponseScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="api" />}
        title="Otto — API"
        right={
          <RightPanel title="History" icon="clock" width={264}>
            <div style={{ padding: '6px 4px' }}>
              <Stagger delay={8} step={9} y={8}>
                {historyRows.map(([method, path, code, ms]) => (
                  <HistoryRow key={path} method={method} path={path} code={code} ms={ms} />
                ))}
              </Stagger>
            </div>
          </RightPanel>
        }
      >
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            height: '100%',
            padding: 16,
            gap: 12,
            boxSizing: 'border-box',
            overflow: 'hidden',
          }}
        >
          {/* Status + protocol bar */}
          <Appear delay={6} y={10}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <Chip tone="ok">200 OK</Chip>
              <Chip>142 ms</Chip>
              <Chip>1.4 KB</Chip>
              <div style={{ flex: 1 }} />
              <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                {(['HTTP', 'SSE', 'WS', 'gRPC'] as const).map((p, i) => (
                  <Chip key={p} color={i === 0 ? T.accent : T.textDim}>
                    {p}
                  </Chip>
                ))}
              </div>
            </div>
          </Appear>

          {/* Response body */}
          <Appear delay={14} y={12} style={{ flex: 1, minHeight: 0 }}>
            <Terminal
              lines={responseLines}
              delay={18}
              step={8}
              fontSize={13}
              style={{ height: '100%', overflow: 'hidden' }}
            />
          </Appear>
        </div>
      </OttoWindow>
    </Stage>

    <Caption
      step={2}
      title="HTTP · SSE · WS · gRPC · response viewer · request history"
      sub="Every protocol in one place. Full response inspection, timing breakdown, and request history."
    />
  </>
);

// ── Scene 4 — Import + SSRF Guard (110f) ─────────────────────────────────────

const ImportSsrfScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow nav={<Navigator active="api" />} title="Otto — API">
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            height: '100%',
            gap: 28,
            padding: '0 60px',
            boxSizing: 'border-box',
            overflow: 'hidden',
          }}
        >
          {/* Import card */}
          <Appear delay={8} y={18}>
            <Card
              style={{
                width: 680,
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'center',
                gap: 20,
                padding: 32,
              }}
            >
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 20,
                  fontWeight: 700,
                  color: T.text,
                }}
              >
                Import Collection
              </div>

              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 13.5,
                  color: T.textDim,
                  textAlign: 'center',
                  lineHeight: 1.6,
                  maxWidth: 520,
                }}
              >
                Bring in a Postman collection, an OpenAPI spec, or a recorded browser HAR
                file.
              </div>

              {/* Drop zone */}
              <div
                style={{
                  width: '100%',
                  height: 96,
                  border: `2px dashed ${alpha(T.accent, 0.38)}`,
                  borderRadius: 8,
                  display: 'grid',
                  placeItems: 'center',
                  background: alpha(T.accent, 0.04),
                  fontFamily: fonts.ui,
                  fontSize: 13,
                  color: T.textDim,
                }}
              >
                Drop a file here, or choose a format below
              </div>

              {/* Format buttons */}
              <Appear delay={22} y={8}>
                <div style={{ display: 'flex', gap: 12 }}>
                  <Button variant="default" icon="external">
                    Postman v2.1
                  </Button>
                  <Button variant="default" icon="file">
                    OpenAPI 3.x
                  </Button>
                  <Button variant="default" icon="archive">
                    HAR
                  </Button>
                </div>
              </Appear>
            </Card>
          </Appear>

          {/* SSRF guard toast */}
          <Toast
            tone="bad"
            text="blocked: 169.254.169.254 (AWS metadata) — SSRF guard"
            delay={32}
          />
        </div>
      </OttoWindow>
    </Stage>

    <Caption
      step={3}
      title="Import Postman / OpenAPI / HAR · outbound is SSRF-guarded"
      sub="Migrate existing Postman collections in one click. SSRF guard blocks localhost, RFC1918, and cloud metadata."
    />
  </>
);

// ── Outro node (extracted for clean array literal) ───────────────────────────

const OutroNode = (
  <WalkOutro
    title="API Client"
    tagline="Hit any endpoint — safely — without leaving Otto"
    pills={[
      { label: 'HTTP / SSE / WS / gRPC', icon: 'send',     color: '#0a84ff'         },
      { label: 'Environments',            icon: 'box',      color: brand.cyan         },
      { label: 'Import / Export',         icon: 'external', color: brand.purple       },
      { label: 'SSRF-guarded',            icon: 'key',      color: status.needsYou    },
    ]}
  />
);

// ── Composition ───────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur:  80, node: <TitleScene />,          name: 'Title'    },
  { dur: 160, node: <RequestBuilderScene />, name: 'Builder'  },
  { dur: 130, node: <ResponseScene />,       name: 'Response' },
  { dur: 110, node: <ImportSsrfScene />,     name: 'Import'   },
  { dur: 120, node: OutroNode,               name: 'Outro'    },
];

export const apiDuration = scenesDuration(SCENES);
export const Api: React.FC = () => <Scenes scenes={SCENES} />;
