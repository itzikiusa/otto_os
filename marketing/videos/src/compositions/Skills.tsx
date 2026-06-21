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
  Card,
  Chip,
  Button,
  Segmented,
  StatusDot,
  Diff,
  Icon,
  track,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  SKILLS & CONTEXT — skill library · skill evaluation · souls & context
// ════════════════════════════════════════════════════════════════════════════

// ── Scene 1 — title ──────────────────────────────────────────────────────────
const TitleScene: React.FC = () => (
  <TitleCard
    kicker="SKILLS & CONTEXT"
    title="Skills your agents actually use"
    subtitle="Bundled, versioned, evaluated — and refined from your work"
  />
);

// ── Scene 2 — the versioned skill library ────────────────────────────────────
interface SkillRow {
  name: string;
  version: string;
  desc: string;
  tags: { label: string; color: string }[];
  installed: boolean;
}

const TAG = {
  review: '#bf7aff',
  feature: '#0a84ff',
  testing: '#28c840',
  product: '#febc2e',
  insights: brand.cyan,
};

const SKILLS: SkillRow[] = [
  {
    name: 'golang-feature-implementation',
    version: 'v3',
    desc: 'SOLID services, DAO + worker templates, multi-tenant SQL',
    tags: [{ label: 'feature', color: TAG.feature }],
    installed: true,
  },
  {
    name: 'code-review',
    version: 'v2',
    desc: 'Security · performance · architecture lens for review runs',
    tags: [{ label: 'review', color: TAG.review }],
    installed: true,
  },
  {
    name: 'golang-testing',
    version: 'v2',
    desc: 'Unit / component / integration pyramid + mock reuse',
    tags: [{ label: 'testing', color: TAG.testing }],
    installed: true,
  },
  {
    name: 'product-prd',
    version: 'v4',
    desc: 'Jira-ready PRDs with acceptance criteria from a repo scan',
    tags: [{ label: 'product', color: TAG.product }],
    installed: false,
  },
  {
    name: 'insights',
    version: 'v1',
    desc: 'Usage-signal summaries that power the daily report',
    tags: [{ label: 'insights', color: TAG.insights }],
    installed: false,
  },
];

const SkillCard: React.FC<{ s: SkillRow }> = ({ s }) => (
  <Card pad={14} style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
    <span
      style={{
        width: 38,
        height: 38,
        borderRadius: 10,
        background: alpha(s.tags[0].color, 0.16),
        border: `1px solid ${alpha(s.tags[0].color, 0.4)}`,
        display: 'grid',
        placeItems: 'center',
        color: s.tags[0].color,
        flexShrink: 0,
      }}
    >
      <Icon name="zap" size={19} color={s.tags[0].color} />
    </span>
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
        <span style={{ fontFamily: fonts.mono, fontSize: 16, fontWeight: 700, color: T.text }}>{s.name}</span>
        <Chip color={brand.cyan} style={{ height: 19, fontSize: 11 }}>
          {s.version}
        </Chip>
        {s.tags.map((tg) => (
          <Chip key={tg.label} color={tg.color} style={{ height: 19, fontSize: 11 }}>
            {tg.label}
          </Chip>
        ))}
      </div>
      <div style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim, marginTop: 5 }}>{s.desc}</div>
    </div>
    {s.installed ? (
      <span style={{ display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0 }}>
        <span style={{ display: 'flex', alignItems: 'center', gap: 6, fontFamily: fonts.ui, fontSize: 12.5, color: status.working }}>
          <StatusDot kind="working" size={8} pulse={false} />
          Installed
        </span>
        <Button size="s" icon="refresh">
          Update
        </Button>
      </span>
    ) : (
      <Button variant="primary" size="s" icon="arrowDown" style={{ flexShrink: 0 }}>
        Install
      </Button>
    )}
  </Card>
);

const LibraryScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="skills-eval" counts={{ 'skills-eval': 12 }} />}
        title="Otto — Skills"
      >
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%', boxSizing: 'border-box', padding: 22 }}>
          {/* header row */}
          <Appear delay={4} y={12}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 18 }}>
              <Icon name="zap" size={20} color={brand.cyan} />
              <span style={{ fontFamily: fonts.ui, fontSize: 21, fontWeight: 750 as never, color: T.text }}>Skill Library</span>
              <Chip color={T.textDim} style={{ height: 21 }}>
                12 skills
              </Chip>
              <div style={{ flex: 1 }} />
              <Segmented options={['All', 'review', 'feature', 'testing', 'product']} active={0} />
            </div>
          </Appear>
          {/* skill cards */}
          <Stagger delay={14} step={8} y={16} style={{ display: 'flex', flexDirection: 'column', gap: 11 }}>
            {SKILLS.map((s) => (
              <SkillCard key={s.name} s={s} />
            ))}
          </Stagger>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="A versioned skill library"
      sub="Install, update & filter by tag — skills power review, product & insights"
    />
  </>
);

// ── Scene 3 — skill evaluation (variants → diff → promote winner) ────────────
const VariantCard: React.FC<{
  label: string;
  color: string;
  score: number;
  best?: boolean;
  notes: string;
  delay: number;
}> = ({ label, color, score, best, notes, delay }) => {
  const frame = useCurrentFrame();
  const grow = track(frame, [delay + 6, delay + 26], [0, 1]);
  return (
    <Appear delay={delay} y={14} style={{ flex: 1 }}>
      <Card
        pad={13}
        style={{
          border: `1px solid ${best ? alpha(status.working, 0.55) : T.border}`,
          background: best ? alpha(status.working, 0.06) : T.surface,
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 9 }}>
          <StatusDot kind={best ? 'working' : 'idle'} size={9} pulse={best} />
          <span style={{ fontFamily: fonts.mono, fontSize: 13.5, fontWeight: 700, color: T.text, flex: 1 }}>{label}</span>
          {best && (
            <Chip tone="ok" style={{ height: 19, fontSize: 11 }}>
              winner
            </Chip>
          )}
        </div>
        {/* score bar */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
          <div style={{ flex: 1, height: 8, borderRadius: 999, background: T.surface2, overflow: 'hidden' }}>
            <div
              style={{
                width: `${score * grow}%`,
                height: '100%',
                borderRadius: 999,
                background: `linear-gradient(90deg, ${color}, ${alpha(color, 0.55)})`,
              }}
            />
          </div>
          <span style={{ fontFamily: fonts.mono, fontSize: 14, fontWeight: 700, color: best ? status.working : T.text, width: 38, textAlign: 'right' }}>
            {Math.round(score * grow)}
          </span>
        </div>
        <div style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim, marginTop: 8 }}>{notes}</div>
      </Card>
    </Appear>
  );
};

const EvalScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="skills-eval" counts={{ 'skills-eval': 12 }} />}
        title="Otto — Skill Evaluator"
      >
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%', boxSizing: 'border-box', padding: 22, gap: 14 }}>
          {/* run header */}
          <Appear delay={3} y={12}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 11 }}>
              <Icon name="play" size={18} color={brand.cyan} />
              <span style={{ fontFamily: fonts.ui, fontSize: 19, fontWeight: 750 as never, color: T.text }}>
                Evaluating <span style={{ fontFamily: fonts.mono, color: brand.cyan }}>golang-feature-implementation</span>
              </span>
              <div style={{ flex: 1 }} />
              <Chip color={T.textDim} style={{ height: 22 }}>
                sources: pr_bo · docs/
              </Chip>
            </div>
          </Appear>

          <div style={{ display: 'flex', gap: 16, flex: 1, minHeight: 0 }}>
            {/* left: variants + promote */}
            <div style={{ width: 430, display: 'flex', flexDirection: 'column', gap: 12 }}>
              <div style={{ display: 'flex', gap: 12 }}>
                <VariantCard
                  label="variant A · claude"
                  color={providers.claude}
                  score={91}
                  best
                  notes="Adds worker/ETL template + table-schema check"
                  delay={14}
                />
                <VariantCard
                  label="variant B · codex"
                  color={providers.codex}
                  score={78}
                  notes="Good SOLID pass, missed mock-reuse rule"
                  delay={22}
                />
              </div>
              <Appear delay={64} y={14}>
                <Card pad={14} style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                  <span
                    style={{
                      width: 34,
                      height: 34,
                      borderRadius: 9,
                      background: alpha(status.working, 0.16),
                      display: 'grid',
                      placeItems: 'center',
                      color: status.working,
                    }}
                  >
                    <Icon name="check" size={18} color={status.working} />
                  </span>
                  <div style={{ flex: 1 }}>
                    <div style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 700, color: T.text }}>variant A wins · +13 pts</div>
                    <div style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim, marginTop: 2 }}>
                      Promotes to v4 · v3 kept for rollback
                    </div>
                  </div>
                  <Button variant="primary" size="m" icon="arrowUp">
                    Promote winner
                  </Button>
                </Card>
              </Appear>
            </div>

            {/* right: iteration diff */}
            <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 9 }}>
              <Appear delay={30} y={10}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <Icon name="split" size={15} color={T.textDim} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 600, color: T.text }}>
                    Iteration diff — SKILL.md
                  </span>
                  <Chip color={brand.cyan} style={{ height: 18, fontSize: 10.5 }}>
                    v3 → v4
                  </Chip>
                </div>
              </Appear>
              <Diff
                delay={36}
                step={5}
                fontSize={13}
                style={{ flex: 1 }}
                lines={[
                  { text: '## Mandatory checks', kind: 'hunk' },
                  { text: 'Verify table schemas before any query', kind: 'ctx' },
                  { text: 'Guess column names from entity structs', kind: 'del' },
                  { text: 'Cross-check ~/go_tests_utils/component/tables', kind: 'add' },
                  { text: 'Reuse mocks from go_casino_kit/clients', kind: 'add' },
                  { text: '', kind: 'ctx' },
                  { text: '## Worker / ETL pattern', kind: 'hunk' },
                  { text: 'Use GeneralWorkerJobService + Conductor', kind: 'add' },
                  { text: 'Enrich with hourly aggregate + currency', kind: 'add' },
                ]}
              />
            </div>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Evaluate & promote the best version"
      sub="Agents iterate a skill against your codebase — keep the winner"
    />
  </>
);

// ── Scene 4 — souls & context, materialized on spawn ─────────────────────────
const CtxItem: React.FC<{ icon: string; color: string; name: string; meta: string }> = ({ icon, color, name, meta }) => (
  <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '7px 4px' }}>
    <span
      style={{
        width: 26,
        height: 26,
        borderRadius: 7,
        background: alpha(color, 0.16),
        display: 'grid',
        placeItems: 'center',
        color,
        flexShrink: 0,
      }}
    >
      <Icon name={icon} size={14} color={color} />
    </span>
    <span style={{ flex: 1, fontFamily: fonts.mono, fontSize: 13, color: T.text }}>{name}</span>
    <span style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim }}>{meta}</span>
  </div>
);

const PreviewLine: React.FC<{ icon: string; text: string; tone?: string }> = ({ icon, text, tone }) => (
  <div style={{ display: 'flex', alignItems: 'center', gap: 9, fontFamily: fonts.mono, fontSize: 12.5, color: tone ?? T.textDim }}>
    <Icon name={icon} size={13} color={tone ?? T.textDim} />
    {text}
  </div>
);

const ContextScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow nav={<Navigator active="skills-eval" />} title="Otto — Context Assembly">
        <div style={{ display: 'flex', gap: 18, height: '100%', boxSizing: 'border-box', padding: 22 }}>
          {/* left: selection */}
          <div style={{ width: 470, display: 'flex', flexDirection: 'column', gap: 13 }}>
            <Appear delay={4} y={10}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <Icon name="folder" size={18} color={brand.cyan} />
                <span style={{ fontFamily: fonts.ui, fontSize: 17, fontWeight: 750 as never, color: T.text }}>
                  Context · workspace sinatra-users-go
                </span>
              </div>
            </Appear>
            <Appear delay={12} y={12}>
              <Card pad={12}>
                <div style={{ fontFamily: fonts.ui, fontSize: 12, fontWeight: 600, color: T.textDim, textTransform: 'uppercase', letterSpacing: 0.5, marginBottom: 4 }}>
                  Skills
                </div>
                <CtxItem icon="zap" color={TAG.feature} name="golang-feature-implementation v4" meta="selected" />
                <CtxItem icon="zap" color={TAG.review} name="code-review v2" meta="selected" />
                <CtxItem icon="zap" color={TAG.testing} name="golang-testing v2" meta="selected" />
              </Card>
            </Appear>
            <Appear delay={20} y={12}>
              <Card pad={12}>
                <div style={{ fontFamily: fonts.ui, fontSize: 12, fontWeight: 600, color: T.textDim, textTransform: 'uppercase', letterSpacing: 0.5, marginBottom: 4 }}>
                  Soul & context docs
                </div>
                <CtxItem icon="user" color={brand.violet} name="Soul · senior-go-reviewer" meta="active" />
                <CtxItem icon="file" color={T.textDim} name="AGENTS.md" meta="repo" />
                <CtxItem icon="note" color={T.textDim} name="pr_bo schema notes" meta="library" />
              </Card>
            </Appear>
          </div>

          {/* right: dry-run preview */}
          <div style={{ flex: 1, minWidth: 0 }}>
            <Appear delay={28} y={12} style={{ height: '100%' }}>
              <div
                style={{
                  height: '100%',
                  borderRadius: 10,
                  border: `1px solid ${T.border}`,
                  background: T.termBg,
                  display: 'flex',
                  flexDirection: 'column',
                  overflow: 'hidden',
                }}
              >
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    padding: '11px 14px',
                    borderBottom: `1px solid ${T.border}`,
                  }}
                >
                  <Icon name="eye" size={15} color={brand.cyan} />
                  <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 600, color: T.text }}>
                    Dry-run preview — materialized on spawn
                  </span>
                  <Chip tone="accent" style={{ height: 19, fontSize: 11 }}>
                    no session yet
                  </Chip>
                </div>
                <div style={{ flex: 1, padding: 16, display: 'flex', flexDirection: 'column', gap: 9 }}>
                  <Stagger delay={40} step={6} y={8} style={{ display: 'flex', flexDirection: 'column', gap: 9 }}>
                    <PreviewLine icon="folder" text=".claude/skills/golang-feature-implementation/" tone={T.text} />
                    <PreviewLine icon="folder" text=".claude/skills/code-review/" tone={T.text} />
                    <PreviewLine icon="folder" text=".claude/skills/golang-testing/" tone={T.text} />
                    <PreviewLine icon="file" text="SOUL.md → senior-go-reviewer" tone={brand.violet} />
                    <PreviewLine icon="file" text="CLAUDE.md  (AGENTS.md + schema notes)" tone={T.text} />
                    <PreviewLine icon="check" text="3 skills · 1 soul · 2 docs ready" tone={status.working} />
                  </Stagger>
                </div>
                <Appear delay={88} y={8}>
                  <div style={{ padding: '0 16px 16px', display: 'flex', justifyContent: 'flex-end' }}>
                    <Button variant="primary" size="m" icon="play">
                      Spawn with this context
                    </Button>
                  </div>
                </Appear>
              </div>
            </Appear>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="Souls & context, materialized on spawn"
      sub="Preview exactly what each session will get"
    />
  </>
);

// ── scenes ───────────────────────────────────────────────────────────────────
const SCENES: SceneDef[] = [
  { dur: 80, node: <TitleScene />, name: 'Title' },
  { dur: 210, node: <LibraryScene />, name: 'Library' },
  { dur: 220, node: <EvalScene />, name: 'Eval' },
  { dur: 140, node: <ContextScene />, name: 'Context' },
  {
    dur: 130,
    node: (
      <WalkOutro
        title="Skills & Context"
        tagline="Sharper agents, every run."
        pills={[
          { label: 'Skill library', color: '#0a84ff', icon: 'zap' },
          { label: 'Versioned', color: brand.cyan, icon: 'tag' },
          { label: 'Skill eval', color: providers.claude, icon: 'eye' },
          { label: 'Souls & context', color: brand.violet, icon: 'user' },
          { label: 'Self-improving', color: '#28c840', icon: 'refresh' },
        ]}
      />
    ),
    name: 'Outro',
  },
];

export const skillsDuration = scenesDuration(SCENES);
export const Skills: React.FC = () => <Scenes scenes={SCENES} />;
