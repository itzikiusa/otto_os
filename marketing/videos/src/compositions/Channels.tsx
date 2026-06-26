import React from 'react';
import { T, brand, fonts, alpha, status as statusColors, providers } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator, NavSession } from '../components/Nav';
import {
  Appear,
  TitleCard,
  Caption,
  Terminal,
  Chip,
  StatusDot,
  Icon,
  Avatar,
} from '../components/kit';

// ── Slack / Telegram brand marks ─────────────────────────────────────────────
const SLACK = '#36c5f0';
const TELEGRAM = '#2aabee';

// ── Shared sub-components ────────────────────────────────────────────────────

/** Realistic Slack-style chat message row. */
const SlackMsg: React.FC<{
  name: string;
  color: string;
  time: string;
  text: string;
  delay?: number;
  file?: string;
}> = ({ name, color, time, text, delay = 0, file }) => (
  <Appear delay={delay} y={10}>
    <div style={{ display: 'flex', gap: 10, padding: '7px 0' }}>
      <Avatar name={name} color={color} size={30} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, marginBottom: 2 }}>
          <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 700, color: T.text }}>{name}</span>
          <span style={{ fontFamily: fonts.ui, fontSize: 11, color: T.textDim }}>{time}</span>
        </div>
        <div style={{ fontFamily: fonts.ui, fontSize: 13, color: T.text, lineHeight: 1.5 }}>{text}</div>
        {file && (
          <div style={{
            marginTop: 5, display: 'inline-flex', alignItems: 'center', gap: 6,
            padding: '5px 10px', borderRadius: 6,
            background: alpha('#fff', 0.04), border: `1px solid ${T.border}`,
          }}>
            <Icon name="file" size={12} color={T.textDim} />
            <span style={{ fontFamily: fonts.mono, fontSize: 11.5, color: T.textDim }}>{file}</span>
          </div>
        )}
      </div>
    </div>
  </Appear>
);

/** File-relay indicator row (inbound or outbound). */
const FileRow: React.FC<{
  name: string;
  size: string;
  direction: 'in' | 'out';
  label: string;
  color: string;
  delay?: number;
}> = ({ name, size, direction, label, color, delay = 0 }) => (
  <Appear delay={delay} y={10}>
    <div style={{
      display: 'flex', alignItems: 'center', gap: 10,
      padding: '9px 14px', marginBottom: 8, borderRadius: 8,
      background: alpha(color, 0.08), border: `1px solid ${alpha(color, 0.25)}`,
    }}>
      <Icon name={direction === 'in' ? 'arrowDown' : 'arrowUp'} size={13} color={color} />
      <Icon name="file" size={13} color={T.textDim} />
      <span style={{ fontFamily: fonts.mono, fontSize: 13, color: T.text, flex: 1 }}>{name}</span>
      <span style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim }}>{size}</span>
      <Chip color={color}>{label}</Chip>
    </div>
  </Appear>
);

/** Session list row (for the archive scene). */
const ChannelRow: React.FC<{
  transportIcon: string;
  transportColor: string;
  channel: string;
  ticket: string;
  age: string;
  chipLabel: string;
  chipTone: 'ok' | 'warn' | 'default';
  delay?: number;
}> = ({ transportIcon, transportColor, channel, ticket, age, chipLabel, chipTone, delay = 0 }) => (
  <Appear delay={delay} y={8}>
    <div style={{
      display: 'flex', alignItems: 'center', gap: 14,
      padding: '11px 20px', borderBottom: `1px solid ${T.border}`,
    }}>
      <Icon name={transportIcon} size={15} color={transportColor} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 600, color: T.text }}>{channel}</div>
        <div style={{ fontFamily: fonts.mono, fontSize: 11.5, color: T.textDim }}>{ticket}</div>
      </div>
      <Chip color={providers.claude}>claude</Chip>
      <span style={{
        fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim,
        minWidth: 72, textAlign: 'right',
      }}>{age}</span>
      <Chip tone={chipTone}>{chipLabel}</Chip>
    </div>
  </Appear>
);

// ── Shared nav sessions ──────────────────────────────────────────────────────

const bridgeSessions: NavSession[] = [
  { title: '#support · SUP-204', provider: 'claude', status: 'working' },
  { title: '#oncall · INC-891',  provider: 'claude', status: 'idle'    },
];

// ── Scene 1 — Title (~80f) ───────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Channels · Slack & Telegram"
    title="Channels"
    subtitle="Bridge any Slack or Telegram thread to a real agent session — messages, files, and replies flow both ways."
  />
);

// ── Scene 2 — Slack thread ↔ agent session (~175f) ───────────────────────────

const SlackBridgeScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={
          <Navigator
            active="agents"
            sessions={bridgeSessions}
            activeSessionTitle="#support · SUP-204"
            workingCount={1}
          />
        }
        title="Otto — #support · SUP-204"
        tabs={[
          { label: '#support · SUP-204', icon: 'terminal', active: true, dot: 'working' },
          { label: '#oncall · INC-891',  icon: 'terminal', dot: 'idle' },
        ]}
      >
        <div style={{ display: 'flex', height: '100%' }}>

          {/* ── Left: Slack thread ── */}
          <div style={{
            width: 460, flexShrink: 0, borderRight: `1px solid ${T.border}`,
            display: 'flex', flexDirection: 'column', overflow: 'hidden',
          }}>
            <div style={{
              padding: '9px 14px', borderBottom: `1px solid ${T.border}`,
              display: 'flex', alignItems: 'center', gap: 8,
              background: alpha('#fff', 0.025), flexShrink: 0,
            }}>
              <Icon name="slack" size={15} color={SLACK} />
              <span style={{ fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 700, color: T.text }}>#support</span>
              <Chip color={SLACK}>SUP-204</Chip>
              <span style={{ marginLeft: 'auto' }}><Chip tone="ok">bridged</Chip></span>
            </div>

            <div style={{ flex: 1, padding: '10px 14px', overflow: 'hidden' }}>
              <SlackMsg
                name="Priya" color="#e8a87c" time="10:14 AM"
                text="the webhook 500s on every retry — payloads aren't reaching the processor"
                delay={8}
              />
              <SlackMsg
                name="Priya" color="#e8a87c" time="10:14 AM"
                text="attached the raw request for context"
                file="payload.json"
                delay={14}
              />
              <SlackMsg
                name="OttoBot" color={providers.claude} time="10:15 AM"
                text="Reading payload.json… Content-Type header is missing — the processor rejects anything that isn't application/json"
                delay={26}
              />
              <SlackMsg
                name="OttoBot" color={providers.claude} time="10:16 AM"
                text="Fix applied in webhook/handler.go → PR #418 opened"
                delay={38}
              />
            </div>
          </div>

          {/* ── Right: bridged agent Terminal ── */}
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
            <div style={{
              padding: '9px 14px', borderBottom: `1px solid ${T.border}`,
              display: 'flex', alignItems: 'center', gap: 8,
              background: alpha('#fff', 0.025), flexShrink: 0,
            }}>
              <StatusDot kind="working" size={8} />
              <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 600, color: T.text }}>
                claude · #support · SUP-204
              </span>
            </div>
            <Terminal
              lines={[
                { text: '↳ [slack/#support] Priya: webhook 500s on retry', tone: 'dim' },
                { text: '↳ [attachment] payload.json (2.1 KB)',             tone: 'dim' },
                { text: '$ cat payload.json | jq .headers',                 tone: 'cmd' },
                { text: '  "Content-Type": (missing)',                       tone: 'err' },
                { text: '  ↳ processor requires application/json',           tone: 'warn' },
                { text: '  reading webhook/handler.go…',                     tone: 'text' },
                { text: '  patching validateContentType()',                   tone: 'text' },
                { text: '  ✓ tests pass — opening PR #418',                  tone: 'ok' },
                { text: '↳ [relay] reply → #support thread',                 tone: 'accent' },
              ]}
              delay={10}
              step={12}
              fontSize={13}
              style={{ flex: 1, borderRadius: 0 }}
            />
          </div>

        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Bridge a Slack or Telegram thread to a real agent session"
      sub="Inbound messages relay in; the agent's replies relay back into the thread."
    />
  </>
);

// ── Scene 3 — Files both ways + Telegram (~155f) ─────────────────────────────

const FilesScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={
          <Navigator
            active="agents"
            sessions={bridgeSessions}
            activeSessionTitle="#support · SUP-204"
            workingCount={1}
          />
        }
        title="Otto — #support · SUP-204"
      >
        <div style={{ display: 'flex', height: '100%' }}>

          {/* ── Left: transport strip + file relay ── */}
          <div style={{
            width: 460, flexShrink: 0, borderRight: `1px solid ${T.border}`,
            display: 'flex', flexDirection: 'column',
          }}>
            <div style={{
              padding: '9px 14px', borderBottom: `1px solid ${T.border}`,
              display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0,
            }}>
              <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 600, color: T.textDim }}>
                Transport
              </span>
              <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
                <Chip color={SLACK}>
                  <Icon name="slack" size={11} color={SLACK} />{' '}Slack
                </Chip>
                <Chip color={TELEGRAM}>
                  <Icon name="send" size={11} color={TELEGRAM} />{' '}Telegram
                </Chip>
              </div>
            </div>

            <div style={{ flex: 1, padding: 14, overflow: 'hidden' }}>
              <Appear delay={4} y={6}>
                <div style={{
                  fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim,
                  marginBottom: 10, letterSpacing: 0.5, textTransform: 'uppercase',
                }}>
                  Files relayed in
                </div>
              </Appear>
              <FileRow
                name="payload.json" size="2.1 KB"
                direction="in" label="from Slack"
                color={brand.cyan} delay={8}
              />

              <Appear delay={16} y={6}>
                <div style={{
                  fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim,
                  margin: '12px 0 10px', letterSpacing: 0.5, textTransform: 'uppercase',
                }}>
                  Files relayed out
                </div>
              </Appear>
              <FileRow
                name="fix.patch" size="3.4 KB"
                direction="out" label="to thread"
                color={statusColors.working} delay={22}
              />
            </div>
          </div>

          {/* ── Right: agent output ── */}
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
            <Terminal
              lines={[
                { text: '↳ [attachment] payload.json (2.1 KB)',        tone: 'dim'    },
                { text: '  → saved to session workspace',               tone: 'text'   },
                { text: '  analysing webhook failure path…',            tone: 'text'   },
                { text: '  root cause: missing Content-Type header',    tone: 'warn'   },
                { text: '  generating patch…',                          tone: 'text'   },
                { text: '  ✓ fix.patch ready (3.4 KB)',                 tone: 'ok'     },
                { text: '↳ [file-out] fix.patch → Slack thread',        tone: 'accent' },
                { text: '  (same relay works for Telegram)',             tone: 'dim'    },
              ]}
              delay={6}
              step={13}
              fontSize={13}
              style={{ flex: 1, borderRadius: 0 }}
            />
          </div>

        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Messages and files relayed both ways"
      sub="Slack (Socket Mode) & Telegram (BotFather) — attach files in, receive files out."
    />
  </>
);

// ── Scene 4 — One-per-ticket + auto-archive (~125f) ──────────────────────────

const ArchiveScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow nav={<Navigator active="agents" />} title="Otto — Channels">
        <div style={{ display: 'flex', height: '100%', flexDirection: 'column', overflow: 'hidden' }}>

          {/* header */}
          <div style={{
            padding: '10px 20px', borderBottom: `1px solid ${T.border}`,
            display: 'flex', alignItems: 'center', gap: 12, flexShrink: 0,
          }}>
            <Appear delay={2} y={6}>
              <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 600, color: T.text }}>
                Channel Sessions
              </span>
            </Appear>
            <Appear delay={6} y={6}>
              <div style={{
                marginLeft: 'auto',
                display: 'inline-flex', alignItems: 'center', gap: 8,
                padding: '6px 16px', borderRadius: 8,
                background: alpha(brand.cyan, 0.08),
                border: `1px solid ${alpha(brand.cyan, 0.22)}`,
                fontFamily: fonts.ui, fontSize: 13, fontWeight: 700, color: brand.cyan,
              }}>
                1 agent ⇄ 1 thread
              </div>
            </Appear>
          </div>

          {/* session rows */}
          <ChannelRow
            transportIcon="slack" transportColor={SLACK}
            channel="#support" ticket="SUP-204"
            age="just now" chipLabel="working" chipTone="ok"
            delay={10}
          />
          <ChannelRow
            transportIcon="send" transportColor={TELEGRAM}
            channel="@ops-alerts" ticket="TG-1039"
            age="12 min" chipLabel="idle" chipTone="warn"
            delay={17}
          />
          <ChannelRow
            transportIcon="slack" transportColor={SLACK}
            channel="#oncall" ticket="INC-891"
            age="65 min" chipLabel="archived" chipTone="default"
            delay={24}
          />

          {/* auto-archive callout */}
          <Appear delay={34} y={10}>
            <div style={{
              margin: '16px 20px 0',
              display: 'flex', alignItems: 'center', gap: 10,
              padding: '10px 16px', borderRadius: 8,
              background: alpha(statusColors.needsYou, 0.08),
              border: `1px solid ${alpha(statusColors.needsYou, 0.22)}`,
            }}>
              <Icon name="clock" size={14} color={statusColors.needsYou} />
              <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.text }}>
                Sessions auto-archive after 60 min idle
              </span>
              <span style={{ marginLeft: 'auto' }}>
                <Chip color={brand.cyan}>revive by messaging the thread</Chip>
              </span>
            </div>
          </Appear>

        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="One agent per ticket — auto-archived when the thread goes quiet"
      sub="Message the thread again to revive. Sessions are fully isolated per ticket."
    />
  </>
);

// ── Scene list ───────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 80,  node: <TitleScene />,       name: 'Title'       },
  { dur: 175, node: <SlackBridgeScene />, name: 'SlackBridge' },
  { dur: 155, node: <FilesScene />,       name: 'Files'       },
  { dur: 125, node: <ArchiveScene />,     name: 'Archive'     },
  {
    dur: 130,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Channels"
        tagline="Work a ticket from the chat thread it lives in"
        pills={[
          { label: 'Slack',         icon: 'slack'   },
          { label: 'Telegram',      icon: 'send'    },
          { label: 'Files both ways', icon: 'file'  },
          { label: 'Webhook',       icon: 'link'    },
        ]}
      />
    ),
  },
];

export const channelsDuration = scenesDuration(SCENES);
export const Channels: React.FC = () => <Scenes scenes={SCENES} />;
