import React from 'react';
import { T, Theme, fonts, radius, alpha, status as STATUS, navActive } from '../theme';
import { Icon } from './Icon';
import { OttoGlyph } from './OttoLogo';
import { StatusDot } from './kit';

export interface NavSession {
  title: string;
  provider: string;
  status?: keyof typeof STATUS;
  tasks?: [number, number]; // [done, total]
  needsYou?: boolean;
}

// The real module list + order from the app's Navigator (ui/src/lib/sidebar.ts),
// excluding the special "Agents" row which is rendered above with nested sessions.
const MODULES: { id: string; icon: string; label: string }[] = [
  { id: 'mission-control', icon: 'radar', label: 'Mission Control' },
  { id: 'connections', icon: 'plug', label: 'Connections' },
  { id: 'swarm', icon: 'grid', label: 'Swarm' },
  { id: 'loops', icon: 'refresh', label: 'Goal Loops' },
  { id: 'proof', icon: 'check', label: 'Proof' },
  { id: 'git', icon: 'branch', label: 'Git' },
  { id: 'product', icon: 'note', label: 'Product' },
  { id: 'vault', icon: 'globe', label: 'Vault' },
  { id: 'canvas', icon: 'shapes', label: 'Canvas' },
  { id: 'api', icon: 'send', label: 'API' },
  { id: 'database', icon: 'db', label: 'Database' },
  { id: 'brokers', icon: 'box', label: 'Message Brokers' },
  { id: 'mcp', icon: 'plug', label: 'MCP Control Plane' },
  { id: 'workflows', icon: 'split', label: 'Workflows' },
  { id: 'scheduled-tasks', icon: 'clock', label: 'Scheduled Tasks' },
  { id: 'skills-eval', icon: 'zap', label: 'Skills Evaluator' },
  { id: 'insights', icon: 'gauge', label: 'Insights' },
  { id: 'usage', icon: 'chart', label: 'Usage' },
];

const Row: React.FC<{
  icon: string;
  label: string;
  active?: boolean;
  t: Theme;
  count?: number | string;
  countTone?: 'working' | 'default';
  twisty?: boolean;
  open?: boolean;
}> = ({ icon, label, active, t, count, countTone, twisty, open }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 8,
      height: 28,
      padding: '0 8px',
      margin: '1px 0',
      borderRadius: radius.s,
      background: active ? navActive.bg : 'transparent',
      color: active ? navActive.fg : t.text,
      boxShadow: active ? `inset 3px 0 0 ${navActive.edge}` : 'none',
      fontFamily: fonts.ui,
      fontSize: 12.5,
      fontWeight: active ? 600 : 500,
    }}
  >
    <Icon name={icon} size={14} color={active ? navActive.fg : t.text} />
    <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{label}</span>
    {count != null && (
      <span
        style={{
          minWidth: 16,
          height: 15,
          padding: '0 4px',
          borderRadius: 999,
          fontSize: 10,
          fontWeight: 700,
          display: 'grid',
          placeItems: 'center',
          color: countTone === 'working' ? STATUS.working : t.textDim,
          background: countTone === 'working' ? alpha(STATUS.working, 0.22) : alpha(t.textDim, 0.16),
        }}
      >
        {count}
      </span>
    )}
    {twisty && <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} color={active ? navActive.fg : t.textDim} />}
  </div>
);

const SessionRow: React.FC<{ s: NavSession; t: Theme; active?: boolean }> = ({ s, t, active }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 8,
      height: 26,
      padding: '0 8px',
      borderRadius: radius.s,
      background: active ? navActive.bg : 'transparent',
      color: active ? navActive.fg : t.text,
      boxShadow: active ? `inset 3px 0 0 ${navActive.edge}` : s.needsYou ? `inset 2px 0 0 ${STATUS.needsYou}` : 'none',
      fontFamily: fonts.ui,
      fontSize: 12,
    }}
  >
    <StatusDot kind={s.status ?? 'working'} size={8} />
    <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: active ? navActive.fg : t.text }}>
      {s.title}
    </span>
    {s.tasks && (
      <span
        style={{
          padding: '0 5px',
          height: 14,
          lineHeight: '14px',
          borderRadius: 999,
          fontSize: 9,
          fontWeight: 700,
          color: s.tasks[0] === s.tasks[1] ? STATUS.working : t.accent,
          background: alpha(s.tasks[0] === s.tasks[1] ? STATUS.working : t.accent, 0.16),
        }}
      >
        {s.tasks[0]}/{s.tasks[1]}
      </span>
    )}
    <span style={{ fontSize: 10, color: active ? alpha(navActive.fg, 0.7) : t.textDim }}>{s.provider}</span>
  </div>
);

/** The full expanded Navigator (real module list + green active row). */
export const Navigator: React.FC<{
  active?: string;
  t?: Theme;
  width?: number;
  sessions?: NavSession[];
  activeSessionTitle?: string;
  workingCount?: number;
  counts?: Record<string, number>;
  user?: { name: string; sub?: string };
}> = ({ active = 'agents', t = T, width = 248, sessions, activeSessionTitle, workingCount, counts = {}, user = { name: 'Alex', sub: 'root' } }) => (
  <div
    style={{
      width,
      flexShrink: 0,
      background: t.bgSidebar,
      borderRight: `1px solid ${t.border}`,
      display: 'flex',
      flexDirection: 'column',
      minHeight: 0,
    }}
  >
    {/* header */}
    <div style={{ display: 'flex', alignItems: 'center', gap: 9, padding: '11px 12px 6px 14px' }}>
      <OttoGlyph size={18} glow={false} />
      <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 13, fontWeight: 700, letterSpacing: -0.2, color: t.text }}>Otto</span>
      <Icon name="chevronLeft" size={14} color={t.textDim} />
      <Icon name="chevronRight" size={14} color={t.textDim} />
      <Icon name="sidebar" size={14} color={t.textDim} />
    </div>
    {/* scroll */}
    <div style={{ flex: 1, overflow: 'hidden', padding: '4px 8px' }}>
      {/* search */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          margin: '2px 0 6px',
          padding: '0 8px',
          height: 26,
          borderRadius: radius.s,
          background: alpha(t.textDim, 0.1),
          color: t.textDim,
          fontFamily: fonts.ui,
          fontSize: 12,
        }}
      >
        <Icon name="search" size={12} />
        Search all sessions…
      </div>

      {/* Agents + nested sessions */}
      <Row icon="terminal" label="Agents" active={active === 'agents'} t={t} count={workingCount} countTone="working" twisty open />
      {sessions && sessions.length > 0 && (
        <div style={{ paddingLeft: 10, marginBottom: 6 }}>
          {sessions.map((s, i) => (
            <SessionRow key={i} s={s} t={t} active={active === 'agents' && s.title === activeSessionTitle} />
          ))}
        </div>
      )}

      {MODULES.map((m) => (
        <Row key={m.id} icon={m.icon} label={m.label} active={active === m.id} t={t} count={counts[m.id]} />
      ))}
    </div>
    {/* foot */}
    <div style={{ borderTop: `1px solid ${t.border}`, padding: '6px 8px 8px' }}>
      <Row icon="info" label="Walkthroughs" active={active === 'walkthroughs'} t={t} />
      <Row icon="gear" label="Settings" active={active === 'settings'} t={t} />
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '6px 8px 2px' }}>
        <span style={{ width: 24, height: 24, borderRadius: '50%', background: alpha(t.accent, 0.28), color: t.accent, display: 'grid', placeItems: 'center', fontFamily: fonts.ui, fontSize: 11, fontWeight: 700 }}>
          {user.name.slice(0, 1)}
        </span>
        <div style={{ flex: 1 }}>
          <div style={{ fontFamily: fonts.ui, fontSize: 12, fontWeight: 500, color: t.text, lineHeight: 1.2 }}>{user.name}</div>
          <div style={{ fontFamily: fonts.ui, fontSize: 10.5, color: t.textDim }}>{user.sub}</div>
        </div>
      </div>
    </div>
  </div>
);

/** Collapsed 44px icon rail. */
export const Rail: React.FC<{ active?: string; t?: Theme; workingCount?: number }> = ({ active = 'agents', t = T, workingCount }) => {
  const items = [{ id: 'agents', icon: 'terminal' }, ...MODULES.map((m) => ({ id: m.id, icon: m.icon }))];
  return (
    <div
      style={{
        width: 44,
        flexShrink: 0,
        background: t.bgSidebar,
        borderRight: `1px solid ${t.border}`,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        padding: '10px 0',
        gap: 4,
      }}
    >
      <div style={{ marginBottom: 8 }}>
        <OttoGlyph size={20} glow={false} />
      </div>
      {items.map((m) => {
        const on = m.id === active;
        return (
          <div
            key={m.id}
            style={{
              width: 30,
              height: 30,
              borderRadius: radius.s,
              display: 'grid',
              placeItems: 'center',
              background: on ? alpha(t.accent, 0.18) : 'transparent',
              color: on ? t.accent : t.textDim,
              position: 'relative',
            }}
          >
            <Icon name={m.icon} size={16} />
            {m.id === 'agents' && workingCount ? (
              <span style={{ position: 'absolute', top: -2, right: -3, minWidth: 14, height: 14, padding: '0 3px', borderRadius: 999, background: STATUS.working, color: '#fff', fontFamily: fonts.ui, fontSize: 8.5, fontWeight: 700, display: 'grid', placeItems: 'center' }}>
                {workingCount}
              </span>
            ) : null}
          </div>
        );
      })}
    </div>
  );
};
