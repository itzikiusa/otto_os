import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, providers, status, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
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
  Diff,
  Toast,
  StatusDot,
  Avatar,
  Caret,
  Icon,
  track,
  useTyped,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  GIT & PULL REQUESTS — diff → PR, drafted by agents
// ════════════════════════════════════════════════════════════════════════════

const REPO_TABS = [
  { label: 'sinatra-users-go', icon: 'branch', active: true },
  { label: 'admission', icon: 'branch' },
  { label: 'otto', icon: 'branch' },
] as const;

const GIT_NAV = <Navigator active="git" />;

// ── shared bits ──────────────────────────────────────────────────────────────

const SectionLabel: React.FC<{ children: React.ReactNode; icon?: string }> = ({ children, icon }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 8,
      fontFamily: fonts.ui,
      fontSize: 12,
      fontWeight: 700,
      letterSpacing: 0.6,
      textTransform: 'uppercase',
      color: T.textDim,
    }}
  >
    {icon && <Icon name={icon} size={13} color={T.textDim} />}
    {children}
  </div>
);

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 1 — title card
// ════════════════════════════════════════════════════════════════════════════

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="GIT & PULL REQUESTS"
    title="From diff to PR, drafted by agents"
    subtitle="Repo tabs · commit graph · AI commits · one-click PRs"
  />
);

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 2 — repo tabs + live commit graph
// ════════════════════════════════════════════════════════════════════════════

interface GraphCommit {
  hash: string;
  msg: string;
  author: string;
  color: string;
  when: string;
  // lane positions for the SVG (0-based column index)
  lane: number;
  // a "branch out" parent on another lane (for the merge/fork curve)
  forkTo?: number;
  head?: boolean;
}

const LANE_COLORS = [brand.cyan, '#bf7aff', status.needsYou];

const GRAPH: GraphCommit[] = [
  { hash: 'a3f9c21', msg: 'feat(auth): validate JWT on protected routes', author: 'Claude', color: providers.claude, when: 'just now', lane: 1, head: true },
  { hash: '7b1e0d4', msg: 'test(auth): cover expired & malformed tokens', author: 'Claude', color: providers.claude, when: '4 min ago', lane: 1 },
  { hash: 'e0c4a87', msg: 'chore: bump jwt-go to v5.2.1', author: 'Alex', color: T.accent, when: '22 min ago', lane: 1, forkTo: 0 },
  { hash: 'c9d2f10', msg: 'release: cut v1.4.0', author: 'Mara', color: '#28c840', when: '1 h ago', lane: 0 },
  { hash: '5a8e6b3', msg: 'fix(api): paginate /users list endpoint', author: 'Alex', color: T.accent, when: '3 h ago', lane: 0 },
  { hash: '1f7c992', msg: 'feat(api): add /users search', author: 'Mara', color: '#28c840', when: 'yesterday', lane: 0 },
];

const ROW_H = 60;
const LANE_X = (lane: number) => 26 + lane * 30;

const CommitGraph: React.FC = () => {
  const frame = useCurrentFrame();
  const totalH = GRAPH.length * ROW_H;
  const draw = track(frame, [10, 46], [0, 1]); // 0→1 lane-line reveal

  return (
    <div style={{ position: 'relative', flex: 1, minHeight: 0 }}>
      {/* SVG lanes behind the rows */}
      <svg
        width={120}
        height={totalH}
        style={{ position: 'absolute', left: 0, top: 0 }}
        viewBox={`0 0 120 ${totalH}`}
      >
        {/* lane 0 — main (full height) */}
        <line
          x1={LANE_X(0)}
          y1={ROW_H * 2.5}
          x2={LANE_X(0)}
          y2={totalH}
          stroke={LANE_COLORS[0]}
          strokeWidth={2.4}
          strokeOpacity={0.85}
          strokeDasharray={totalH}
          strokeDashoffset={totalH * (1 - draw)}
        />
        {/* lane 1 — feature/jwt-validation (rows 0..2) */}
        <line
          x1={LANE_X(1)}
          y1={ROW_H * 0.5}
          x2={LANE_X(1)}
          y2={ROW_H * 2.5}
          stroke={LANE_COLORS[1]}
          strokeWidth={2.4}
          strokeOpacity={0.9}
          strokeDasharray={totalH}
          strokeDashoffset={totalH * (1 - draw)}
        />
        {/* fork curve: feature lane merges down into main at the v1.4.0 base */}
        <path
          d={`M ${LANE_X(1)} ${ROW_H * 2.5} C ${LANE_X(1)} ${ROW_H * 3.1}, ${LANE_X(0)} ${ROW_H * 2.9}, ${LANE_X(0)} ${ROW_H * 3.5}`}
          fill="none"
          stroke={LANE_COLORS[1]}
          strokeWidth={2.4}
          strokeOpacity={0.7 * draw}
        />
        {/* commit dots */}
        {GRAPH.map((c, i) => {
          const cy = i * ROW_H + ROW_H / 2;
          const pop = track(frame, [16 + i * 5, 24 + i * 5], [0, 1]);
          return (
            <g key={c.hash} opacity={pop}>
              {c.head && (
                <circle cx={LANE_X(c.lane)} cy={cy} r={9} fill="none" stroke={LANE_COLORS[c.lane]} strokeWidth={2} opacity={0.5} />
              )}
              <circle cx={LANE_X(c.lane)} cy={cy} r={5.5} fill={T.bg} stroke={LANE_COLORS[c.lane]} strokeWidth={2.6} />
              <circle cx={LANE_X(c.lane)} cy={cy} r={2.2} fill={LANE_COLORS[c.lane]} />
            </g>
          );
        })}
      </svg>

      {/* commit rows */}
      <div style={{ position: 'absolute', left: 0, top: 0, right: 0 }}>
        {GRAPH.map((c, i) => {
          const at = 18 + i * 5;
          const op = track(frame, [at, at + 8], [0, 1]);
          const x = track(frame, [at, at + 8], [10, 0]);
          return (
            <div
              key={c.hash}
              style={{
                height: ROW_H,
                display: 'flex',
                alignItems: 'center',
                gap: 12,
                paddingLeft: 116,
                paddingRight: 18,
                opacity: op,
                transform: `translateX(${x}px)`,
                borderBottom: i < GRAPH.length - 1 ? `1px solid ${alpha(T.border, 0.5)}` : 'none',
              }}
            >
              <span style={{ fontFamily: fonts.mono, fontSize: 13.5, color: T.textDim, width: 70, flexShrink: 0 }}>{c.hash}</span>
              <div style={{ flex: 1, minWidth: 0, display: 'flex', alignItems: 'center', gap: 9 }}>
                {/* HEAD + branch/tag chips on the head row */}
                {c.head && (
                  <>
                    <Chip color={LANE_COLORS[c.lane]} style={{ fontFamily: fonts.mono }}>HEAD</Chip>
                    <Chip tone="accent" style={{ fontFamily: fonts.mono }}>feature/jwt-validation</Chip>
                  </>
                )}
                {c.msg === 'release: cut v1.4.0' && (
                  <>
                    <Chip color="#28c840" style={{ fontFamily: fonts.mono }}>main</Chip>
                    <Chip color={status.needsYou} style={{ fontFamily: fonts.mono }}>
                      <Icon name="tag" size={11} color={status.needsYou} /> v1.4.0
                    </Chip>
                  </>
                )}
                <span
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 15,
                    color: T.text,
                    fontWeight: c.head ? 600 : 500,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {c.msg}
                </span>
              </div>
              <Avatar name={c.author} color={c.color} size={24} />
              <span style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim, width: 78, textAlign: 'right', flexShrink: 0 }}>
                {c.when}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
};

const GraphScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow nav={GIT_NAV} tabs={REPO_TABS as never} title="Otto — sinatra-users-go">
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%', boxSizing: 'border-box' }}>
          {/* graph toolbar */}
          <Appear delay={4} y={-8}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 12,
                padding: '12px 18px',
                borderBottom: `1px solid ${T.border}`,
                flexShrink: 0,
              }}
            >
              <SectionLabel icon="branch">Commit graph</SectionLabel>
              <span style={{ flex: 1 }} />
              <Chip tone="accent" style={{ fontFamily: fonts.mono }}>
                <Icon name="commit" size={12} color={T.accent} /> feature/jwt-validation
              </Chip>
              <Chip color="#28c840" style={{ fontFamily: fonts.mono }}>↑2 ↓0</Chip>
              <Button size="s" variant="default" icon="fetch">Fetch</Button>
            </div>
          </Appear>
          <CommitGraph />
          {/* right-click hint footer */}
          <Appear delay={50} y={8}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                padding: '10px 18px',
                borderTop: `1px solid ${T.border}`,
                background: alpha(T.surface, 0.4),
                flexShrink: 0,
                fontFamily: fonts.ui,
                fontSize: 12.5,
                color: T.textDim,
              }}
            >
              <Icon name="commit" size={13} color={T.textDim} />
              Right-click a commit:
              {['cherry-pick', 'revert', 'checkout', 'branch', 'tag'].map((a) => (
                <span key={a} style={{ fontFamily: fonts.mono, color: T.text, padding: '2px 8px', borderRadius: 6, background: T.surface2, border: `1px solid ${T.border}` }}>
                  {a}
                </span>
              ))}
            </div>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption step={1} title="Repo tabs + a live commit graph" sub="Branches, tags & HEAD as chips — right-click any commit to act" />
  </>
);

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 3 — stage files, then let an agent draft the commit
// ════════════════════════════════════════════════════════════════════════════

interface ChangeFile {
  path: string;
  add: number;
  del: number;
  st: 'M' | 'A';
}

const CHANGES: ChangeFile[] = [
  { path: 'auth/middleware/jwt.go', add: 12, del: 4, st: 'M' },
  { path: 'auth/middleware/jwt_test.go', add: 38, del: 0, st: 'A' },
  { path: 'router/routes.go', add: 3, del: 1, st: 'M' },
];

const StFlag: React.FC<{ st: 'M' | 'A' }> = ({ st }) => (
  <span
    style={{
      width: 18,
      height: 18,
      borderRadius: 4,
      display: 'grid',
      placeItems: 'center',
      fontFamily: fonts.mono,
      fontSize: 11,
      fontWeight: 700,
      flexShrink: 0,
      color: st === 'A' ? status.working : status.needsYou,
      background: alpha(st === 'A' ? status.working : status.needsYou, 0.16),
      border: `1px solid ${alpha(st === 'A' ? status.working : status.needsYou, 0.4)}`,
    }}
  >
    {st}
  </span>
);

const CommitScene: React.FC = () => {
  const typed = useTyped('feat(auth): validate JWT on protected routes', 64, 30);
  const full = typed.length >= 44;
  return (
    <>
      <Stage scale={0.88}>
        <OttoWindow nav={GIT_NAV} tabs={REPO_TABS as never} title="Otto — sinatra-users-go">
          <div style={{ display: 'flex', flexDirection: 'column', height: '100%', padding: 22, gap: 18, boxSizing: 'border-box' }}>
            {/* Changes list */}
            <div>
              <Appear delay={4} y={-6}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 12 }}>
                  <SectionLabel icon="edit">Changes</SectionLabel>
                  <Chip color={T.accent}>3 staged</Chip>
                </div>
              </Appear>
              <Stagger delay={10} step={6} y={10} style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                {CHANGES.map((f) => (
                  <Card key={f.path} pad={0} style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '11px 14px' }}>
                    <span style={{ width: 16, height: 16, borderRadius: 4, background: status.working, display: 'grid', placeItems: 'center', flexShrink: 0 }}>
                      <Icon name="check" size={11} color="#fff" />
                    </span>
                    <StFlag st={f.st} />
                    <Icon name="file" size={15} color={T.textDim} />
                    <span style={{ flex: 1, fontFamily: fonts.mono, fontSize: 14.5, color: T.text }}>{f.path}</span>
                    <span style={{ fontFamily: fonts.mono, fontSize: 13.5, color: status.working, fontWeight: 600 }}>+{f.add}</span>
                    <span style={{ fontFamily: fonts.mono, fontSize: 13.5, color: status.exited, fontWeight: 600 }}>−{f.del}</span>
                  </Card>
                ))}
              </Stagger>
            </div>

            {/* AI-drafted commit message */}
            <Appear delay={40} y={14} style={{ marginTop: 4 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 12 }}>
                <SectionLabel icon="commit">Commit message</SectionLabel>
                <Chip color={providers.claude}>✨ drafted by claude</Chip>
              </div>
              <Card pad={0} style={{ padding: '14px 16px', background: T.surface2 }}>
                <div style={{ fontFamily: fonts.mono, fontSize: 18, color: T.text, fontWeight: 600, lineHeight: 1.5 }}>
                  {typed}
                  {!full && <Caret color={T.accent} h={20} />}
                </div>
                <div style={{ fontFamily: fonts.mono, fontSize: 13.5, color: T.textDim, marginTop: 10, lineHeight: 1.6 }}>
                  Reject unsigned & expired tokens before handler dispatch;
                  <br />
                  add 401 path + coverage for malformed claims.
                </div>
              </Card>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginTop: 16 }}>
                <Button variant="primary" icon="commit">Commit</Button>
                <Button variant="default" icon="edit">Regenerate</Button>
                <span style={{ flex: 1 }} />
                <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim }}>
                  Conventional Commits · written from your staged diff
                </span>
              </div>
            </Appear>
          </div>
        </OttoWindow>
      </Stage>
      <Caption step={2} title="Stage, then let an agent draft the commit" sub="Conventional Commits, written from your staged diff" />
    </>
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 4 — full, syntax-aware diff
// ════════════════════════════════════════════════════════════════════════════

const DIFF_LINES = [
  { text: 'func RequireAuth(next http.Handler) http.Handler {', kind: 'ctx' as const },
  { text: '@@ -18,7 +18,15 @@ func RequireAuth(next http.Handler)', kind: 'hunk' as const },
  { text: '  tok := bearer(r.Header.Get("Authorization"))', kind: 'ctx' as const },
  { text: '  if tok == "" {', kind: 'del' as const },
  { text: '    next.ServeHTTP(w, r) // TODO: verify', kind: 'del' as const },
  { text: '  claims, err := jwt.Parse(tok, keyFor)', kind: 'add' as const },
  { text: '  if err != nil || !claims.Valid {', kind: 'add' as const },
  { text: '    http.Error(w, "unauthorized", 401)', kind: 'add' as const },
  { text: '    return', kind: 'add' as const },
  { text: '  }', kind: 'add' as const },
  { text: '  if claims.ExpiresAt.Before(time.Now()) {', kind: 'add' as const },
  { text: '    http.Error(w, "token expired", 401)', kind: 'add' as const },
  { text: '    return', kind: 'add' as const },
  { text: '  ctx := withUser(r.Context(), claims.Sub)', kind: 'ctx' as const },
  { text: '  next.ServeHTTP(w, r.WithContext(ctx))', kind: 'ctx' as const },
];

const DiffScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow nav={GIT_NAV} tabs={REPO_TABS as never} title="Otto — sinatra-users-go">
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%', padding: 22, boxSizing: 'border-box' }}>
          {/* file header */}
          <Appear delay={4} y={-6}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 12,
                padding: '12px 16px',
                borderRadius: '8px 8px 0 0',
                background: T.surface2,
                border: `1px solid ${T.border}`,
                borderBottom: 'none',
              }}
            >
              <Icon name="file" size={16} color={T.textDim} />
              <span style={{ fontFamily: fonts.mono, fontSize: 15, color: T.text, fontWeight: 600 }}>auth/middleware/jwt.go</span>
              <Chip color={T.accent} style={{ fontFamily: fonts.mono }}>M</Chip>
              <span style={{ flex: 1 }} />
              <span style={{ fontFamily: fonts.mono, fontSize: 14, color: status.working, fontWeight: 700 }}>+12</span>
              <span style={{ fontFamily: fonts.mono, fontSize: 14, color: status.exited, fontWeight: 700 }}>−4</span>
              {/* diff stat bar */}
              <span style={{ display: 'flex', gap: 2, marginLeft: 4 }}>
                {Array.from({ length: 8 }).map((_, i) => (
                  <span key={i} style={{ width: 8, height: 8, borderRadius: 2, background: i < 6 ? status.working : status.exited }} />
                ))}
              </span>
            </div>
          </Appear>
          <Diff
            lines={DIFF_LINES}
            delay={14}
            step={5}
            fontSize={15.5}
            style={{ flex: 1, borderRadius: '0 0 8px 8px', minHeight: 0, padding: '8px 0' }}
          />
        </div>
      </OttoWindow>
    </Stage>
    <Caption step={3} title="Full, syntax-aware diffs" sub="Per-hunk add / remove with exact line counts" />
  </>
);

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 5 — open a PR + merge-readiness
// ════════════════════════════════════════════════════════════════════════════

const CI_CHECKS: { label: string; meta: string }[] = [
  { label: 'lint', meta: 'golangci-lint' },
  { label: 'unit tests', meta: '142 passed' },
  { label: 'build', meta: 'go build ./…' },
];

const PRScene: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <>
      <Stage scale={0.88}>
        <OttoWindow nav={GIT_NAV} tabs={REPO_TABS as never} title="Otto — sinatra-users-go">
          <div style={{ display: 'flex', height: '100%', padding: 22, gap: 18, boxSizing: 'border-box' }}>
            {/* LEFT — the PR draft */}
            <div style={{ flex: 1.35, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 14 }}>
              <Appear delay={4} y={-6}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                  <SectionLabel icon="pr">New pull request</SectionLabel>
                  <Chip color={providers.claude}>✨ drafted by claude</Chip>
                  <span style={{ flex: 1 }} />
                  <span style={{ fontFamily: fonts.mono, fontSize: 12.5, color: T.textDim }}>feature/jwt-validation → main</span>
                </div>
              </Appear>
              <Appear delay={12} y={12} style={{ flex: 1, minHeight: 0 }}>
                <Card pad={0} style={{ height: '100%', display: 'flex', flexDirection: 'column', padding: 18, gap: 14 }}>
                  <Field
                    label="Title"
                    value="feat(auth): validate JWT on protected routes"
                    mono
                    style={{ flexShrink: 0 }}
                  />
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 6, flex: 1, minHeight: 0 }}>
                    <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 500, color: T.textDim }}>Body</span>
                    <div
                      style={{
                        flex: 1,
                        borderRadius: 6,
                        background: T.surface2,
                        border: `1px solid ${T.border}`,
                        padding: '14px 16px',
                        display: 'flex',
                        flexDirection: 'column',
                        gap: 11,
                      }}
                    >
                      <span style={{ fontFamily: fonts.ui, fontSize: 14, color: T.textDim }}>
                        Adds JWT verification to the auth middleware so every protected route enforces a valid, unexpired token.
                      </span>
                      {[
                        'Reject unsigned / malformed tokens with a 401',
                        'Short-circuit expired tokens before handler dispatch',
                        'Cover happy-path, expiry & tampering in jwt_test.go',
                      ].map((b, i) => (
                        <div key={i} style={{ display: 'flex', alignItems: 'flex-start', gap: 9 }}>
                          <span style={{ marginTop: 6, width: 6, height: 6, borderRadius: '50%', background: T.accent, flexShrink: 0 }} />
                          <span style={{ fontFamily: fonts.ui, fontSize: 14, color: T.text, lineHeight: 1.45 }}>{b}</span>
                        </div>
                      ))}
                    </div>
                  </div>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                    <Button variant="primary" icon="pr">Create Pull Request</Button>
                    <Button variant="default" icon="edit">Edit draft</Button>
                  </div>
                </Card>
              </Appear>
            </div>

            {/* RIGHT — merge readiness */}
            <Appear delay={26} x={18} style={{ flex: 1, minWidth: 0 }}>
              <Card pad={0} style={{ height: '100%', display: 'flex', flexDirection: 'column', padding: 18, gap: 14 }}>
                <SectionLabel icon="merge">Merge readiness</SectionLabel>

                {/* CI checks */}
                <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                  {CI_CHECKS.map((c, i) => {
                    const at = 40 + i * 9;
                    const op = track(frame, [at, at + 8], [0, 1]);
                    return (
                      <div
                        key={c.label}
                        style={{
                          display: 'flex',
                          alignItems: 'center',
                          gap: 11,
                          padding: '10px 13px',
                          borderRadius: 7,
                          background: T.surface2,
                          border: `1px solid ${T.border}`,
                          opacity: op,
                          transform: `translateY(${(1 - op) * 8}px)`,
                        }}
                      >
                        <StatusDot kind="working" size={10} pulse={false} />
                        <span style={{ fontFamily: fonts.ui, fontSize: 14.5, color: T.text, fontWeight: 600 }}>{c.label}</span>
                        <span style={{ fontFamily: fonts.mono, fontSize: 12.5, color: T.textDim }}>{c.meta}</span>
                        <span style={{ flex: 1 }} />
                        <Icon name="check" size={16} color={status.working} />
                      </div>
                    );
                  })}
                </div>

                {/* mergeability summary */}
                <Appear delay={72} y={10}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 10, paddingTop: 4 }}>
                    <Chip tone="ok"><Icon name="merge" size={12} color={status.working} /> Mergeable</Chip>
                    <Chip color="#28c840" style={{ fontFamily: fonts.mono }}>↑2 ↓0</Chip>
                    <Chip color={T.accent}>3 checks ✓</Chip>
                  </div>
                </Appear>
                <span style={{ flex: 1 }} />
                <Appear delay={80} y={10}>
                  <div style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim, lineHeight: 1.5 }}>
                    All checks green · no conflicts with <span style={{ fontFamily: fonts.mono, color: T.text }}>main</span> · ready to merge
                  </div>
                </Appear>
              </Card>
            </Appear>
          </div>

          {/* auto-push toast */}
          <Toast
            text="git push → origin feature/jwt-validation"
            tone="ok"
            delay={56}
            style={{ position: 'absolute', right: 26, bottom: 22 }}
          />
        </OttoWindow>
      </Stage>
      <Caption step={4} title="Open a PR — title & body written for you" sub="Branch auto-pushed · CI + mergeability at a glance" />
    </>
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  SCENES
// ════════════════════════════════════════════════════════════════════════════

const SCENES: SceneDef[] = [
  { dur: 80, node: <TitleScene />, name: 'Title' },
  { dur: 220, node: <GraphScene />, name: 'Graph' },
  { dur: 190, node: <CommitScene />, name: 'Commit' },
  { dur: 180, node: <DiffScene />, name: 'Diff' },
  { dur: 210, node: <PRScene />, name: 'PR' },
  {
    dur: 130,
    node: (
      <WalkOutro
        title="Git & Pull Requests"
        tagline="Diff to merge, without leaving Otto."
        pills={[
          { label: 'Repo tabs', color: '#28c840', icon: 'branch' },
          { label: 'Commit graph', color: brand.cyan, icon: 'commit' },
          { label: 'AI commits', color: providers.claude, icon: 'edit' },
          { label: 'PR drafts', color: brand.violet, icon: 'pr' },
          { label: 'SFTP', color: '#0a84ff', icon: 'folder' },
        ]}
      />
    ),
    name: 'Outro',
  },
];

export const gitDuration = scenesDuration(SCENES);
export const Git: React.FC = () => <Scenes scenes={SCENES} />;
