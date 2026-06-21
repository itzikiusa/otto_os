import React from 'react';
import { AbsoluteFill, useCurrentFrame } from 'remotion';
import { T, brand, fonts, alpha, providers, status } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { PhoneFrame } from '../components/Frame';
import { Navigator, NavSession } from '../components/Nav';
import {
  Appear,
  TitleCard,
  Caption,
  Avatar,
  StatusDot,
  Chip,
  Button,
  Card,
  Icon,
  track,
} from '../components/kit';

// Channel brand colors (real Slack/Telegram marks).
const SLACK = '#36c5f0';
const TELEGRAM = '#0a84ff';

// ── A single chat bubble (teammate left / agent right) ───────────────────────
const Bubble: React.FC<{
  who: string;
  color: string;
  delay: number;
  side?: 'left' | 'right';
  agent?: boolean;
  children: React.ReactNode;
  file?: string;
}> = ({ who, color, delay, side = 'left', agent, children, file }) => (
  <Appear delay={delay} y={18} style={{ display: 'flex', gap: 12, alignItems: 'flex-start', flexDirection: side === 'right' ? 'row-reverse' : 'row' }}>
    <Avatar name={who} color={color} size={36} />
    <div style={{ maxWidth: 560, display: 'flex', flexDirection: 'column', alignItems: side === 'right' ? 'flex-end' : 'flex-start', gap: 6 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexDirection: side === 'right' ? 'row-reverse' : 'row' }}>
        <span style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 700, color: T.text }}>{who}</span>
        {agent && <Chip color={color}>otto · agent</Chip>}
      </div>
      <div
        style={{
          background: agent ? alpha(color, 0.12) : T.surface,
          border: `1px solid ${agent ? alpha(color, 0.4) : T.border}`,
          borderRadius: 14,
          borderTopLeftRadius: side === 'left' ? 4 : 14,
          borderTopRightRadius: side === 'right' ? 4 : 14,
          padding: '12px 16px',
          fontFamily: fonts.ui,
          fontSize: 16,
          lineHeight: 1.5,
          color: T.text,
        }}
      >
        {children}
        {file && (
          <div
            style={{
              marginTop: 12,
              display: 'inline-flex',
              alignItems: 'center',
              gap: 9,
              padding: '8px 12px',
              borderRadius: 10,
              background: T.surface2,
              border: `1px solid ${T.border}`,
            }}
          >
            <span style={{ width: 28, height: 28, borderRadius: 7, background: alpha(brand.cyan, 0.18), display: 'grid', placeItems: 'center' }}>
              <Icon name="file" size={15} color={brand.cyan} />
            </span>
            <div style={{ display: 'flex', flexDirection: 'column' }}>
              <span style={{ fontFamily: fonts.mono, fontSize: 13, fontWeight: 600, color: T.text }}>{file}</span>
              <span style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim }}>attached · 3 files changed</span>
            </div>
          </div>
        )}
      </div>
    </div>
  </Appear>
);

// "agent is working" typing indicator.
const Working: React.FC<{ delay: number; color: string }> = ({ delay, color }) => {
  const frame = useCurrentFrame();
  return (
    <Appear delay={delay} y={14} style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
      <Avatar name="otto" color={color} size={36} />
      <div
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 10,
          padding: '11px 16px',
          borderRadius: 14,
          borderTopLeftRadius: 4,
          background: alpha(color, 0.1),
          border: `1px solid ${alpha(color, 0.35)}`,
        }}
      >
        <StatusDot kind="working" size={9} />
        <span style={{ fontFamily: fonts.ui, fontSize: 14.5, color: T.textDim }}>otto is working — reading routes, running tests…</span>
        <span style={{ display: 'inline-flex', gap: 4, marginLeft: 2 }}>
          {[0, 1, 2].map((i) => (
            <span
              key={i}
              style={{
                width: 5,
                height: 5,
                borderRadius: '50%',
                background: color,
                opacity: 0.4 + 0.6 * (Math.sin(frame / 5 - i) * 0.5 + 0.5),
              }}
            />
          ))}
        </span>
      </div>
    </Appear>
  );
};

// ── Scene 1 — title ──────────────────────────────────────────────────────────
const TitleScene: React.FC = () => (
  <TitleCard
    kicker="SLACK & TELEGRAM"
    title="Your agents, in the channels you already use"
    subtitle="Bridge a thread to an agent — replies come straight back"
  />
);

// ── Scene 2 — bridge a thread to an agent (desktop) ──────────────────────────
const agentSessions: NavSession[] = [
  { title: 'fix CI on payments', provider: 'claude', status: 'working', tasks: [2, 3] },
  { title: 'flaky e2e on checkout', provider: 'codex', status: 'idle', tasks: [3, 3] },
];

// A tiny channel-group header inside the content (Slack / Telegram groups with counts).
const ChannelGroup: React.FC<{ icon: string; label: string; color: string; count: number; delay: number; open?: boolean }> = ({
  icon,
  label,
  color,
  count,
  delay,
  open,
}) => (
  <Appear delay={delay} y={8} style={{ display: 'flex', alignItems: 'center', gap: 9, padding: '7px 10px', borderRadius: 8, background: open ? alpha(color, 0.1) : 'transparent', border: `1px solid ${open ? alpha(color, 0.32) : 'transparent'}` }}>
    <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} color={T.textDim} />
    <Icon name={icon} size={15} color={color} />
    <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 600, color: T.text }}>{label}</span>
    <span
      style={{
        minWidth: 18,
        height: 17,
        padding: '0 5px',
        borderRadius: 999,
        fontFamily: fonts.ui,
        fontSize: 10.5,
        fontWeight: 700,
        display: 'grid',
        placeItems: 'center',
        color,
        background: alpha(color, 0.18),
      }}
    >
      {count}
    </span>
  </Appear>
);

const BridgeScene: React.FC = () => (
  <>
    <Stage scale={0.84}>
      <OttoWindow
        nav={<Navigator active="agents" sessions={agentSessions} activeSessionTitle="fix CI on payments" workingCount={1} />}
        tabs={[{ label: '#payments · Slack', icon: 'slack', active: true, dot: 'working' }]}
        title="Otto — sinatra-payments-go"
      >
        <div style={{ display: 'flex', height: '100%' }}>
          {/* channel groups column */}
          <div style={{ width: 270, flexShrink: 0, borderRight: `1px solid ${T.border}`, background: T.bgSidebar, padding: 12, boxSizing: 'border-box', display: 'flex', flexDirection: 'column', gap: 4 }}>
            <Appear delay={2} y={6} style={{ fontFamily: fonts.ui, fontSize: 11.5, fontWeight: 700, letterSpacing: 1, textTransform: 'uppercase', color: T.textDim, padding: '4px 10px 8px' }}>
              Bridged threads
            </Appear>
            <ChannelGroup icon="slack" label="Slack" color={SLACK} count={3} delay={6} open />
            <div style={{ paddingLeft: 14, display: 'flex', flexDirection: 'column', gap: 2 }}>
              {[
                { ch: '#payments', s: 'working' as const, active: true },
                { ch: '#checkout-bugs', s: 'idle' as const },
                { ch: '@ops-oncall', s: 'idle' as const },
              ].map((r, i) => (
                <Appear key={r.ch} delay={10 + i * 3} y={6} style={{ display: 'flex', alignItems: 'center', gap: 8, height: 26, padding: '0 10px', borderRadius: 7, background: r.active ? alpha(SLACK, 0.14) : 'transparent' }}>
                  <StatusDot kind={r.s} size={7} />
                  <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 12.5, color: r.active ? T.text : T.textDim, fontWeight: r.active ? 600 : 500 }}>{r.ch}</span>
                </Appear>
              ))}
            </div>
            <div style={{ height: 6 }} />
            <ChannelGroup icon="send" label="Telegram" color={TELEGRAM} count={1} delay={22} />
          </div>

          {/* the bridged thread */}
          <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '12px 18px', borderBottom: `1px solid ${T.border}` }}>
              <Icon name="slack" size={18} color={SLACK} />
              <span style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 700, color: T.text }}>#payments</span>
              <Chip color={SLACK}>bridged → fix CI on payments</Chip>
              <span style={{ flex: 1 }} />
              <Chip tone="ok">1 agent · this ticket</Chip>
            </div>
            <div style={{ flex: 1, minHeight: 0, padding: '20px 22px', display: 'flex', flexDirection: 'column', gap: 18, boxSizing: 'border-box' }}>
              <Bubble who="Priya" color="#bf7aff" delay={26} side="left">
                @otto can you fix the failing CI on <b>payments</b>? The JWT middleware test started flaking after the v2 merge.
              </Bubble>
              <Working delay={70} color={providers.claude} />
              <Bubble who="otto" color={providers.claude} delay={120} side="right" agent file="patch.diff">
                Found it — race in <code style={{ fontFamily: fonts.mono, fontSize: 14 }}>middleware/jwt.go</code>. Added a mutex around the token cache, re-ran the suite: <b style={{ color: status.working }}>142 passed</b>. Opening a PR.
              </Bubble>
            </div>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Bridge a Slack or Telegram thread to an agent"
      sub="Messages + files in, agent replies + files back · one agent per ticket"
    />
  </>
);

// ── Scene 3 — drive an agent from your phone (mobile beat) ───────────────────
const PhoneBubble: React.FC<{ text: React.ReactNode; mine?: boolean; delay: number; who?: string; color?: string }> = ({
  text,
  mine,
  delay,
  who,
  color = SLACK,
}) => (
  <Appear delay={delay} y={14} style={{ display: 'flex', flexDirection: 'column', alignItems: mine ? 'flex-end' : 'flex-start', gap: 4 }}>
    {who && (
      <span style={{ fontFamily: fonts.ui, fontSize: 11, fontWeight: 700, color: mine ? color : T.textDim, padding: '0 6px' }}>{who}</span>
    )}
    <div
      style={{
        maxWidth: 250,
        padding: '10px 13px',
        borderRadius: 16,
        borderBottomRightRadius: mine ? 4 : 16,
        borderBottomLeftRadius: mine ? 16 : 4,
        background: mine ? alpha(color, 0.16) : T.surface,
        border: `1px solid ${mine ? alpha(color, 0.4) : T.border}`,
        fontFamily: fonts.ui,
        fontSize: 14,
        lineHeight: 1.45,
        color: T.text,
      }}
    >
      {text}
    </div>
  </Appear>
);

const PhoneScene: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <>
      <Stage scale={0.86} float>
        <PhoneFrame title="#payments" active="agents" workingBadge={1} showPanelBtn>
          <div style={{ height: '100%', display: 'flex', flexDirection: 'column', background: T.bg }}>
            {/* channel sub-header */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '8px 14px', borderBottom: `1px solid ${T.border}` }}>
              <Icon name="slack" size={15} color={SLACK} />
              <span style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>bridged to</span>
              <Chip color={SLACK} style={{ height: 19, fontSize: 11 }}>fix CI on payments</Chip>
            </div>
            {/* messages */}
            <div style={{ flex: 1, minHeight: 0, padding: '16px 14px', display: 'flex', flexDirection: 'column', gap: 14, justifyContent: 'flex-end' }}>
              <PhoneBubble who="You" mine delay={20} color={SLACK} text={<>@otto rerun the payments CI and ship the fix 🙏</>} />
              <PhoneBubble who="otto · agent" delay={50} color={providers.claude} text={<>On it — patched <span style={{ fontFamily: fonts.mono }}>jwt.go</span>, suite is green.</>} />
              <PhoneBubble who="otto · agent" delay={86} color={providers.claude} text={<><span style={{ color: status.working, fontWeight: 700 }}>✓ 142 passed</span> · PR #418 opened → <span style={{ color: T.accent }}>review</span></>} />
            </div>
            {/* input bar */}
            <Appear delay={120} y={10} style={{ padding: '10px 12px', borderTop: `1px solid ${T.border}` }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, height: 40, padding: '0 12px', borderRadius: 20, background: T.surface2, border: `1px solid ${T.border}` }}>
                <Icon name="plus" size={16} color={T.textDim} />
                <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 13.5, color: T.textDim }}>
                  Message #payments
                  <span style={{ display: 'inline-block', width: 2, height: 15, background: T.accent, marginLeft: 2, verticalAlign: 'middle', opacity: Math.floor(frame / 8) % 2 ? 1 : 0.2 }} />
                </span>
                <span style={{ width: 28, height: 28, borderRadius: '50%', background: SLACK, display: 'grid', placeItems: 'center' }}>
                  <Icon name="send" size={15} color="#fff" />
                </span>
              </div>
            </Appear>
          </div>
        </PhoneFrame>
      </Stage>
      <Caption
        step={2}
        title="Drive an agent from your phone"
        sub="It's just a chat — Slack or Telegram, from anywhere"
      />
    </>
  );
};

// ── Scene 4 — self-improvement notifier pushed to a channel ──────────────────
const NotifierScene: React.FC = () => {
  const frame = useCurrentFrame();
  const glow = track(frame, [10, 40], [0, 1]);
  return (
    <>
      <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
        <Appear delay={2} y={26}>
          <Card pad={0} style={{ width: 720, overflow: 'hidden', boxShadow: `0 30px 90px rgba(0,0,0,0.5), 0 0 ${40 * glow}px ${alpha(SLACK, 0.3 * glow)}` }}>
            {/* slack-style header */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 11, padding: '14px 20px', borderBottom: `1px solid ${T.border}`, background: T.bgSidebar }}>
              <span style={{ width: 34, height: 34, borderRadius: 9, background: alpha(SLACK, 0.16), display: 'grid', placeItems: 'center' }}>
                <Icon name="slack" size={19} color={SLACK} />
              </span>
              <div style={{ flex: 1 }}>
                <div style={{ fontFamily: fonts.ui, fontSize: 14.5, fontWeight: 700, color: T.text }}>#otto-improvements</div>
                <div style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>Slack · pushed by Otto self-improvement</div>
              </div>
              <Chip color={SLACK} style={{ height: 22 }}>
                <Icon name="bell" size={12} color={SLACK} /> notify
              </Chip>
            </div>
            {/* message body */}
            <div style={{ padding: 22, display: 'flex', gap: 14, alignItems: 'flex-start' }}>
              <Avatar name="otto" color={brand.purple} size={40} />
              <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 12 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <span style={{ fontFamily: fonts.ui, fontSize: 14.5, fontWeight: 700, color: T.text }}>Otto</span>
                  <Chip color={brand.violet}>
                    <Icon name="zap" size={12} color={brand.violet} /> suggestion
                  </Chip>
                  <span style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>just now</span>
                </div>
                <div style={{ fontFamily: fonts.ui, fontSize: 17, lineHeight: 1.55, color: T.text }}>
                  <b>Otto suggests:</b> extract the auth middleware into a shared package — it's duplicated across <span style={{ fontFamily: fonts.mono, fontSize: 15 }}>payments</span>, <span style={{ fontFamily: fonts.mono, fontSize: 15 }}>checkout</span> and <span style={{ fontFamily: fonts.mono, fontSize: 15 }}>admin</span>. Apply?
                </div>
                <Appear delay={30} y={10} style={{ display: 'flex', gap: 10, marginTop: 2 }}>
                  <Button variant="primary" icon="check">Apply</Button>
                  <Button variant="ghost" icon="x">Dismiss</Button>
                  <span style={{ flex: 1 }} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim, alignSelf: 'center' }}>opt-in · self-improvement</span>
                </Appear>
              </div>
            </div>
          </Card>
        </Appear>
      </AbsoluteFill>
      <Caption
        step={3}
        title="Otto pushes improvements to your channel"
        sub="Opt-in self-improvement notifications"
      />
    </>
  );
};

// ── Scenes ───────────────────────────────────────────────────────────────────
const SCENES: SceneDef[] = [
  { dur: 80, node: <TitleScene />, name: 'Title' },
  { dur: 220, node: <BridgeScene />, name: 'Bridge' },
  { dur: 220, node: <PhoneScene />, name: 'Mobile' },
  { dur: 130, node: <NotifierScene />, name: 'Notifier' },
  {
    dur: 130,
    node: (
      <WalkOutro
        title="Channels"
        tagline="Meet your agents where you already talk."
        pills={[
          { label: 'Slack', color: SLACK, icon: 'slack' },
          { label: 'Telegram', color: TELEGRAM, icon: 'send' },
          { label: 'Files in/out', color: brand.cyan, icon: 'file' },
          { label: 'Broadcast', color: brand.violet, icon: 'send' },
          { label: 'Auto-archive', color: '#28c840', icon: 'archive' },
        ]}
      />
    ),
    name: 'Outro',
  },
];

export const channelsDuration = scenesDuration(SCENES);
export const Channels: React.FC = () => <Scenes scenes={SCENES} />;
