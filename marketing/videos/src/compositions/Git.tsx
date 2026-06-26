import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, providers, status, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Stagger,
  TitleCard,
  Caption,
  Diff,
  DiffLine,
  Button,
  Card,
  Field,
  Toast,
  Chip,
  Icon,
  StatusDot,
  Avatar,
  track,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  GIT & PULL REQUESTS
//  5 scenes · ~740f · 30fps
//  Title → Repo/graph → Stage/commit → Agent PR → WalkOutro
// ════════════════════════════════════════════════════════════════════════════

const GIT_NAV = <Navigator active="git" />;

const REPO_TABS = [
  { label: 'sinatra-users-go', icon: 'branch', active: true },
  { label: 'koala-wallet', icon: 'branch' },
] as const;

// ── shared atom ──────────────────────────────────────────────────────────────

const SectionLabel: React.FC<{ icon?: string; children: React.ReactNode }> = ({ icon, children }) => (
  <div style={{ display: 'flex', alignItems: 'center', gap: 7, fontFamily: fonts.ui, fontSize: 11.5, fontWeight: 700, letterSpacing: 0.7, textTransform: 'uppercase', color: T.textDim }}>
    {icon && <Icon name={icon} size={13} color={T.textDim} />}
    {children}
  </div>
);

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 1 — Title card
// ════════════════════════════════════════════════════════════════════════════

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Git & Pull Requests"
    title="Branches to Merged PR"
    subtitle="GitKraken-style graph · AI-drafted PRs · GitHub, Bitbucket & GitLab"
  />
);

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 2 — Repo tabs + branch list + commit graph
// ════════════════════════════════════════════════════════════════════════════

interface GCommit {
  hash: string;
  msg: string;
  author: string;
  authorColor: string;
  when: string;
  lane: 0 | 1;
  head?: boolean;
  tag?: string;
}

const LANE_PAL = [brand.cyan, '#bf7aff'] as const;
const LANE_X = (l: 0 | 1) => 18 + l * 28;
const ROW_H = 52;

const COMMITS: GCommit[] = [
  { hash: '3f9a1c', msg: 'fix(auth): validate JWT exp claim', author: 'itzik', authorColor: providers.claude, when: '2 min', lane: 1, head: true },
  { hash: 'c82b04', msg: 'feat(auth): add JWT refresh endpoint', author: 'itzik', authorColor: providers.claude, when: '18 min', lane: 1 },
  { hash: 'a1d76e', msg: 'chore: update go.sum', author: 'sara', authorColor: T.accent, when: '1 h', lane: 0 },
  { hash: '9e40b2', msg: 'feat(users): paginated list endpoint', author: 'sara', authorColor: T.accent, when: '3 h', lane: 0, tag: 'v1.5.0' },
  { hash: '5c23f1', msg: 'fix(db): connection pool timeout', author: 'dan', authorColor: '#28c840', when: '5 h', lane: 0 },
  { hash: 'd84a60', msg: 'chore(deps): bump gin v1.9.1', author: 'dan', authorColor: '#28c840', when: '1 d', lane: 0 },
];

const GraphLanes: React.FC<{ frame: number }> = ({ frame }) => {
  const totalH = COMMITS.length * ROW_H;
  const draw = track(frame, [8, 44], [0, 1]);
  return (
    <svg width={84} height={totalH} style={{ position: 'absolute', left: 0, top: 0, flexShrink: 0 }}>
      {/* main lane — full height */}
      <line x1={LANE_X(0)} y1={0} x2={LANE_X(0)} y2={totalH}
        stroke={LANE_PAL[0]} strokeWidth={2} strokeOpacity={0.6}
        strokeDasharray={totalH} strokeDashoffset={totalH * (1 - draw)} />
      {/* feat/jwt-refresh lane — rows 0..1 only, then curves into main */}
      <line x1={LANE_X(1)} y1={ROW_H * 0.5} x2={LANE_X(1)} y2={ROW_H * 1.5}
        stroke={LANE_PAL[1]} strokeWidth={2} strokeOpacity={0.8}
        strokeDasharray={totalH} strokeDashoffset={totalH * (1 - draw)} />
      {/* fork-out curve: feat branch diverges from main after row 2 */}
      <path d={`M ${LANE_X(0)} ${ROW_H * 2.5} C ${LANE_X(0)} ${ROW_H * 2.1} ${LANE_X(1)} ${ROW_H * 2.1} ${LANE_X(1)} ${ROW_H * 1.5}`}
        fill="none" stroke={LANE_PAL[1]} strokeWidth={2} strokeOpacity={0.55 * draw} />
      {/* dots */}
      {COMMITS.map((c, i) => {
        const cy = i * ROW_H + ROW_H / 2;
        const pop = track(frame, [14 + i * 5, 22 + i * 5], [0, 1]);
        const lx = LANE_X(c.lane);
        return (
          <g key={c.hash} opacity={pop}>
            {c.head && <circle cx={lx} cy={cy} r={10} fill="none" stroke={LANE_PAL[c.lane]} strokeWidth={1.8} opacity={0.45} />}
            <circle cx={lx} cy={cy} r={5} fill={T.bg} stroke={LANE_PAL[c.lane]} strokeWidth={2.4} />
            <circle cx={lx} cy={cy} r={2} fill={LANE_PAL[c.lane]} />
          </g>
        );
      })}
    </svg>
  );
};

const GraphScene: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <>
      <Stage scale={0.88}>
        <OttoWindow nav={GIT_NAV} tabs={REPO_TABS as never} title="Otto — sinatra-users-go">
          <div style={{ display: 'flex', height: '100%' }}>

            {/* left: branch sidebar */}
            <div style={{ width: 210, borderRight: `1px solid ${T.border}`, background: T.bgSidebar, display: 'flex', flexDirection: 'column', padding: '10px 6px', flexShrink: 0 }}>
              <Appear delay={6}>
                <div style={{ fontFamily: fonts.ui, fontSize: 10.5, fontWeight: 700, color: T.textDim, letterSpacing: 1, textTransform: 'uppercase', padding: '2px 8px 6px' }}>
                  Branches
                </div>
              </Appear>
              {/* folder group: local */}
              {[
                { name: 'main', remote: false, current: false, delay: 10 },
                { name: 'feat/jwt-refresh', remote: false, current: true, delay: 15 },
                { name: 'feat/rate-limit', remote: false, current: false, delay: 20 },
                { name: 'fix/session-leak', remote: false, current: false, delay: 25 },
              ].map(({ name, remote, current, delay }) => {
                const at = delay;
                const op = track(frame, [at, at + 8], [0, 1]);
                return (
                  <div key={name} style={{ display: 'flex', alignItems: 'center', gap: 7, height: 26, padding: '0 8px', borderRadius: 5, background: current ? alpha(brand.cyan, 0.13) : 'transparent', opacity: op, fontFamily: fonts.mono, fontSize: 12, color: current ? brand.cyan : T.text, fontWeight: current ? 700 : 400, margin: '1px 0' }}>
                    <Icon name={remote ? 'globe' : 'branch'} size={12} color={current ? brand.cyan : T.textDim} />
                    <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{name}</span>
                    {current && <Chip tone="accent" style={{ fontSize: 9 }}>HEAD</Chip>}
                  </div>
                );
              })}
              <Appear delay={30}>
                <div style={{ fontFamily: fonts.ui, fontSize: 10.5, fontWeight: 700, color: T.textDim, letterSpacing: 1, textTransform: 'uppercase', padding: '8px 8px 6px', marginTop: 4, borderTop: `1px solid ${alpha(T.border, 0.6)}` }}>
                  Origin
                </div>
              </Appear>
              {[
                { name: 'origin/main', delay: 34 },
                { name: 'origin/feat/jwt-refresh', delay: 38 },
              ].map(({ name, delay }) => {
                const op = track(frame, [delay, delay + 8], [0, 1]);
                return (
                  <div key={name} style={{ display: 'flex', alignItems: 'center', gap: 7, height: 24, padding: '0 8px', borderRadius: 5, opacity: op, fontFamily: fonts.mono, fontSize: 11.5, color: T.textDim, margin: '1px 0' }}>
                    <Icon name="globe" size={11} color={T.textDim} />
                    <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{name}</span>
                  </div>
                );
              })}
              <div style={{ flex: 1 }} />
              <Appear delay={44}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 8px', fontFamily: fonts.mono, fontSize: 10.5, color: status.working }}>
                  <StatusDot kind="working" size={7} />
                  Auto-fetch active
                </div>
              </Appear>
            </div>

            {/* right: commit graph */}
            <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
              {/* toolbar */}
              <Appear delay={5} y={-6}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '10px 16px', borderBottom: `1px solid ${T.border}`, flexShrink: 0 }}>
                  <SectionLabel icon="commit">Commit graph</SectionLabel>
                  <Chip tone="accent" style={{ fontFamily: fonts.mono }}>feat/jwt-refresh</Chip>
                  <Chip color="#28c840" style={{ fontFamily: fonts.mono }}>↑2 ↓0</Chip>
                  <div style={{ flex: 1 }} />
                  <Button size="s" variant="default" icon="fetch">Fetch</Button>
                  <Button size="s" variant="primary" icon="arrowUp">Push</Button>
                </div>
              </Appear>

              {/* graph rows */}
              <div style={{ flex: 1, position: 'relative', overflow: 'hidden' }}>
                <GraphLanes frame={frame} />
                <div style={{ position: 'absolute', left: 0, top: 0, right: 0 }}>
                  {COMMITS.map((c, i) => {
                    const at = 16 + i * 5;
                    const op = track(frame, [at, at + 9], [0, 1]);
                    const tx = track(frame, [at, at + 9], [10, 0]);
                    return (
                      <div key={c.hash} style={{ height: ROW_H, display: 'flex', alignItems: 'center', gap: 10, paddingLeft: 90, paddingRight: 18, opacity: op, transform: `translateX(${tx}px)`, borderBottom: i < COMMITS.length - 1 ? `1px solid ${alpha(T.border, 0.45)}` : 'none' }}>
                        <span style={{ fontFamily: fonts.mono, fontSize: 12, color: T.accent, width: 54, flexShrink: 0 }}>{c.hash}</span>
                        <div style={{ flex: 1, minWidth: 0, display: 'flex', alignItems: 'center', gap: 8 }}>
                          {c.head && <Chip color={LANE_PAL[1]} style={{ fontFamily: fonts.mono }}>HEAD</Chip>}
                          {c.tag && <Chip color={status.needsYou} style={{ fontFamily: fonts.mono }}><Icon name="tag" size={10} color={status.needsYou} /> {c.tag}</Chip>}
                          <span style={{ fontFamily: fonts.ui, fontSize: 14.5, color: T.text, fontWeight: c.head ? 600 : 500, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{c.msg}</span>
                        </div>
                        <Avatar name={c.author} color={c.authorColor} size={22} />
                        <span style={{ fontFamily: fonts.mono, fontSize: 11.5, color: T.textDim, width: 44, textAlign: 'right', flexShrink: 0 }}>{c.when}</span>
                      </div>
                    );
                  })}
                </div>
              </div>
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption step={1} title="Repo tabs, branches & a live commit graph" sub="GitKraken-style graph, branch list with globe/folder grouping, open-repo tabs auto-fetch." />
    </>
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 3 — Stage files + commit with diff preview
// ════════════════════════════════════════════════════════════════════════════

const DIFF_LINES: DiffLine[] = [
  { text: '// middleware/jwt.go', kind: 'hunk' },
  { text: '@@ -42,8 +42,17 @@ func JWTMiddleware(cfg *Config) gin.HandlerFunc {', kind: 'hunk' },
  { text: ' func validateToken(tokenStr string, cfg *Config) (*Claims, error) {' },
  { text: '   token, err := jwt.ParseWithClaims(tokenStr, &Claims{}, keyFn(cfg))' },
  { text: '   if err != nil { return nil, err }' },
  { text: '-  return token.Claims.(*Claims), nil', kind: 'del' },
  { text: '+  claims, ok := token.Claims.(*Claims)', kind: 'add' },
  { text: '+  if !ok || !token.Valid {', kind: 'add' },
  { text: '+    return nil, ErrInvalidToken', kind: 'add' },
  { text: '+  }', kind: 'add' },
  { text: '+  if time.Now().After(claims.ExpiresAt.Time) {', kind: 'add' },
  { text: '+    return nil, ErrTokenExpired', kind: 'add' },
  { text: '+  }', kind: 'add' },
  { text: '+  return claims, nil', kind: 'add' },
  { text: ' }' },
];

const STAGED = [
  { path: 'middleware/jwt.go', add: 34, del: 6 },
  { path: 'auth/handler.go', add: 8, del: 2 },
  { path: 'auth/handler_test.go', add: 5, del: 0 },
];

const CommitScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow nav={GIT_NAV} tabs={REPO_TABS as never} title="Otto — sinatra-users-go">
        <div style={{ display: 'flex', height: '100%' }}>
          {/* left: diff */}
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', padding: 14, minWidth: 0 }}>
            <Appear delay={5}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10 }}>
                <Icon name="file" size={13} color={T.textDim} />
                <span style={{ fontFamily: fonts.mono, fontSize: 13.5, color: T.text, fontWeight: 600 }}>middleware/jwt.go</span>
                <Chip tone="ok" style={{ fontSize: 10 }}>staged</Chip>
                <div style={{ flex: 1 }} />
                <span style={{ fontFamily: fonts.mono, fontSize: 12, color: '#7ee787', fontWeight: 700 }}>+34</span>
                <span style={{ fontFamily: fonts.mono, fontSize: 12, color: '#ff9a93', fontWeight: 700 }}>-6</span>
              </div>
            </Appear>
            <Diff lines={DIFF_LINES} delay={10} step={5} fontSize={12.5} style={{ flex: 1 }} />
          </div>

          {/* right: commit panel */}
          <div style={{ width: 294, borderLeft: `1px solid ${T.border}`, background: T.bgSidebar, display: 'flex', flexDirection: 'column', padding: 14, gap: 12, flexShrink: 0 }}>
            <Appear delay={8}>
              <span style={{ fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 700, color: T.text }}>Commit changes</span>
            </Appear>

            <Stagger delay={14} step={8} y={10}>
              <Field label="Branch" value="feat/jwt-refresh" icon="branch" />
              <Field label="Message" value="fix(auth): validate JWT exp claim" focused caret mono />
            </Stagger>

            <Appear delay={34}>
              <div style={{ borderTop: `1px solid ${alpha(T.border, 0.7)}`, paddingTop: 10, display: 'flex', flexDirection: 'column', gap: 5 }}>
                <div style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim, marginBottom: 4 }}>3 files staged</div>
                {STAGED.map((f) => (
                  <div key={f.path} style={{ display: 'flex', alignItems: 'center', gap: 7, padding: '4px 8px', borderRadius: 5, background: alpha(T.accent, 0.07), border: `1px solid ${alpha(T.border, 0.5)}` }}>
                    <span style={{ width: 7, height: 7, borderRadius: '50%', background: '#7ee787', flexShrink: 0 }} />
                    <span style={{ fontFamily: fonts.mono, fontSize: 11, color: T.text, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{f.path}</span>
                    <span style={{ fontFamily: fonts.mono, fontSize: 10.5, color: '#7ee787' }}>+{f.add}</span>
                    <span style={{ fontFamily: fonts.mono, fontSize: 10.5, color: '#ff9a93' }}>-{f.del}</span>
                  </div>
                ))}
              </div>
            </Appear>

            <div style={{ flex: 1 }} />

            <Appear delay={58}>
              <Button variant="primary" icon="commit" style={{ justifyContent: 'center', width: '100%' }}>
                Commit to feat/jwt-refresh
              </Button>
            </Appear>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption step={2} title="Stage, see the diff, commit — Conventional messages" sub="Inline diff, staged file list, pre-filled Conventional Commit message." />
  </>
);

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 4 — Agent-drafted PR
// ════════════════════════════════════════════════════════════════════════════

const PR_BULLETS = [
  'Reject unsigned / malformed tokens with a 401 before handler dispatch',
  'Short-circuit expired tokens; propagate ErrTokenExpired → 401',
  'Cover happy-path, expiry & tampering in auth/handler_test.go',
];

const AgentPRScene: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <>
      <Stage scale={0.88}>
        <OttoWindow nav={GIT_NAV} tabs={REPO_TABS as never} title="Otto — sinatra-users-go">
          <div style={{ display: 'flex', height: '100%', padding: 20, gap: 16, boxSizing: 'border-box' }}>

            {/* left: diff summary */}
            <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, gap: 10 }}>
              <Appear delay={5}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <Icon name="pr" size={14} color={brand.violet} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 700, color: T.text }}>Draft Pull Request</span>
                  <Chip color={brand.violet} style={{ fontSize: 10 }}>feat/jwt-refresh → main</Chip>
                </div>
              </Appear>
              <Diff lines={DIFF_LINES.slice(0, 9)} delay={10} step={4} fontSize={12} style={{ flex: 1 }} />
              <Appear delay={50}>
                <div style={{ fontFamily: fonts.mono, fontSize: 11, color: T.textDim }}>3 files · +47 −8 · GitHub</div>
              </Appear>
            </div>

            {/* right: PR form */}
            <div style={{ width: 410, borderLeft: `1px solid ${T.border}`, background: T.bgSidebar, display: 'flex', flexDirection: 'column', padding: 16, gap: 12, flexShrink: 0 }}>
              <Appear delay={6}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
                  <StatusDot kind="working" size={8} />
                  <span style={{ fontFamily: fonts.mono, fontSize: 11, color: status.working }}>Otto drafted from branch diff</span>
                </div>
              </Appear>

              <Appear delay={14}>
                <Field label="Title" value="fix(auth): validate JWT exp claim [PROJ-1421]" focused mono />
              </Appear>

              <Appear delay={20}>
                <span style={{ fontFamily: fonts.ui, fontSize: 11.5, fontWeight: 600, color: T.textDim }}>Description</span>
              </Appear>

              <Card pad={12} style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 0 }}>
                {[
                  { text: '## Summary', bold: true, indent: false, delay: 26 },
                  { text: 'Adds JWT expiry validation to the auth middleware so protected routes reject invalid and expired tokens.', bold: false, indent: false, delay: 32 },
                  { text: '', bold: false, indent: false, delay: 34 },
                  { text: '## Changes', bold: true, indent: false, delay: 38 },
                  ...PR_BULLETS.map((b, i) => ({ text: `• ${b}`, bold: false, indent: true, delay: 44 + i * 6 })),
                  { text: '', bold: false, indent: false, delay: 58 },
                  { text: '## Testing', bold: true, indent: false, delay: 60 },
                  { text: '• go test ./auth/... — 142 passed, 0 failed', bold: false, indent: true, delay: 64 },
                  { text: '• Verified with expired token in staging', bold: false, indent: true, delay: 68 },
                ].map(({ text, bold, indent, delay }, idx) => {
                  const op = track(frame, [delay, delay + 7], [0, 1]);
                  return (
                    <div key={idx} style={{ opacity: op, fontFamily: fonts.ui, fontSize: 12, color: bold ? T.text : T.textDim, fontWeight: bold ? 700 : 400, paddingLeft: indent ? 12 : 0, lineHeight: 1.6 }}>
                      {text}
                    </div>
                  );
                })}
              </Card>

              <Appear delay={72}>
                <div style={{ display: 'flex', gap: 8 }}>
                  <Button variant="default" size="s" icon="tag">Reviewers</Button>
                  <Button variant="default" size="s" icon="folder">Labels</Button>
                </div>
              </Appear>

              <Appear delay={80}>
                <Button variant="primary" icon="pr" style={{ justifyContent: 'center' }}>
                  Create Pull Request
                </Button>
              </Appear>

              <Toast
                text="Branch pushed · PR #482 opened"
                tone="ok"
                delay={106}
                style={{ position: 'static', marginTop: 2 }}
              />
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption step={3} title="Otto drafts the PR from your diff, then pushes" sub="Conventional Commits title · Jira key in title only · no AI attribution · GitHub, Bitbucket, GitLab." />
    </>
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  SCENES
// ════════════════════════════════════════════════════════════════════════════

const SCENES: SceneDef[] = [
  {
    dur: 80,
    node: <TitleScene />,
    name: 'Title',
  },
  {
    dur: 200,
    node: <GraphScene />,
    name: 'RepoGraph',
  },
  {
    dur: 160,
    node: <CommitScene />,
    name: 'StageCommit',
  },
  {
    dur: 170,
    node: <AgentPRScene />,
    name: 'AgentPR',
  },
  {
    dur: 130,
    node: (
      <WalkOutro
        title="Git & Pull Requests"
        tagline="From working tree to merged PR — without leaving Otto"
        pills={[
          { label: 'Commit graph', icon: 'commit' },
          { label: 'Conflict resolution', icon: 'merge' },
          { label: 'AI-drafted PRs', icon: 'pr' },
          { label: 'GitHub · Bitbucket · GitLab', icon: 'branch' },
        ]}
      />
    ),
    name: 'Outro',
  },
];

export const gitDuration = scenesDuration(SCENES);
export const Git: React.FC = () => <Scenes scenes={SCENES} />;
