import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, alpha, radius } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  TitleCard,
  Caption,
  Terminal,
  TermLine,
  Segmented,
  Button,
  Icon,
  track,
} from '../components/kit';

// ─── Shared micro-components ─────────────────────────────────────────────────

const SceneTab: React.FC<{ label: string; active?: boolean }> = ({ label, active }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 6,
      height: 26,
      padding: '0 12px',
      borderRadius: radius.s,
      background: active ? T.surface : 'transparent',
      border: `1px solid ${active ? T.border : 'transparent'}`,
      fontFamily: fonts.ui,
      fontSize: 12.5,
      fontWeight: active ? 600 : 500,
      color: active ? T.text : T.textDim,
      cursor: 'default',
    }}
  >
    <Icon name="shapes" size={12} color={active ? brand.cyan : T.textDim} />
    {label}
  </div>
);

const SceneTabStrip: React.FC<{ scenes: { label: string; active?: boolean }[] }> = ({ scenes }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 4,
      padding: '0 12px',
      height: 36,
      borderBottom: `1px solid ${T.border}`,
      background: T.bg,
      flexShrink: 0,
    }}
  >
    {scenes.map((s, i) => (
      <SceneTab key={i} label={s.label} active={s.active} />
    ))}
    <div style={{ flex: 1 }} />
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 5,
        padding: '0 10px',
        height: 22,
        borderRadius: radius.s,
        background: T.surface2,
        border: `1px solid ${T.border}`,
        fontFamily: fonts.ui,
        fontSize: 11.5,
        color: T.textDim,
      }}
    >
      <Icon name="plus" size={11} color={T.textDim} />
      New scene
    </div>
  </div>
);

const CanvasToolbar: React.FC<{ modeActive: number; showPresentHint?: boolean }> = ({
  modeActive,
  showPresentHint,
}) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 10,
      padding: '0 14px',
      height: 40,
      borderBottom: `1px solid ${T.border}`,
      background: T.bg,
      flexShrink: 0,
    }}
  >
    <Segmented options={['Excalidraw', 'Mermaid']} active={modeActive} />
    <div style={{ flex: 1 }} />
    <Button variant={showPresentHint ? 'primary' : 'default'} icon="maximize" size="s">
      Present
    </Button>
    {!showPresentHint && (
      <Button variant="ghost" icon="external" size="s">
        Open in Excalidraw
      </Button>
    )}
  </div>
);

const NodeBox: React.FC<{ label: string; sub: string; color: string }> = ({ label, sub, color }) => (
  <div
    style={{
      width: 155,
      padding: '14px 16px',
      background: alpha(color, 0.07),
      border: `1.5px solid ${alpha(color, 0.5)}`,
      borderRadius: 10,
      boxShadow: `0 6px 20px ${alpha(color, 0.15)}`,
    }}
  >
    <div style={{ fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 700, color, marginBottom: 4 }}>
      {label}
    </div>
    <div style={{ fontFamily: fonts.mono, fontSize: 11, color: alpha('#fff', 0.5) }}>{sub}</div>
  </div>
);

// ─── Scene 1 — Title ─────────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Canvas"
    title="Canvas"
    subtitle="Visual workspaces — Excalidraw or Mermaid, diagrams an agent builds with you."
  />
);

// ─── Scene 2 — Excalidraw mode ───────────────────────────────────────────────

const ExcalidrawScene: React.FC = () => {
  const frame = useCurrentFrame();
  const arrowOp = track(frame, [52, 70], [0, 1]);

  return (
    <>
      <Stage scale={0.86}>
        <OttoWindow nav={<Navigator active="canvas" />} title="Otto — Canvas · auth-flow">
          <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
            <SceneTabStrip
              scenes={[{ label: 'auth-flow', active: true }, { label: 'data-model' }]}
            />
            <CanvasToolbar modeActive={0} />

            {/* Canvas area */}
            <div style={{ flex: 1, position: 'relative', overflow: 'hidden', background: T.termBg }}>

              {/* Excalidraw-style dot grid */}
              <div
                style={{
                  position: 'absolute',
                  inset: 0,
                  backgroundImage: `radial-gradient(circle, ${alpha('#ffffff', 0.1)} 1px, transparent 1px)`,
                  backgroundSize: '28px 28px',
                }}
              />

              {/* SVG arrows */}
              <svg
                style={{
                  position: 'absolute',
                  inset: 0,
                  width: '100%',
                  height: '100%',
                  pointerEvents: 'none',
                  opacity: arrowOp,
                }}
              >
                <defs>
                  <marker id="ca-a1" markerWidth="8" markerHeight="6" refX="7" refY="3" orient="auto">
                    <polygon points="0 0, 8 3, 0 6" fill={alpha(brand.cyan, 0.8)} />
                  </marker>
                  <marker id="ca-a2" markerWidth="8" markerHeight="6" refX="7" refY="3" orient="auto">
                    <polygon points="0 0, 8 3, 0 6" fill={alpha(brand.purple, 0.8)} />
                  </marker>
                  <marker id="ca-a3" markerWidth="8" markerHeight="6" refX="7" refY="3" orient="auto">
                    <polygon points="0 0, 8 3, 0 6" fill={alpha(brand.violet, 0.8)} />
                  </marker>
                </defs>
                {/* Client → API Gateway */}
                <line
                  x1="17%" y1="44%" x2="25%" y2="44%"
                  stroke={alpha(brand.cyan, 0.55)}
                  strokeWidth={1.8}
                  strokeDasharray="6 3"
                  markerEnd="url(#ca-a1)"
                />
                <text x="21%" y="42%" textAnchor="middle" fontFamily={fonts.mono} fontSize={10} fill={alpha('#fff', 0.38)}>HTTP/2</text>
                {/* API Gateway → Auth Service */}
                <line
                  x1="38%" y1="44%" x2="46%" y2="44%"
                  stroke={alpha(brand.purple, 0.55)}
                  strokeWidth={1.8}
                  strokeDasharray="6 3"
                  markerEnd="url(#ca-a2)"
                />
                <text x="42%" y="42%" textAnchor="middle" fontFamily={fonts.mono} fontSize={10} fill={alpha('#fff', 0.38)}>gRPC</text>
                {/* Auth Service → Postgres */}
                <line
                  x1="59%" y1="44%" x2="67%" y2="44%"
                  stroke={alpha(brand.violet, 0.55)}
                  strokeWidth={1.8}
                  strokeDasharray="6 3"
                  markerEnd="url(#ca-a3)"
                />
                <text x="63%" y="42%" textAnchor="middle" fontFamily={fonts.mono} fontSize={10} fill={alpha('#fff', 0.38)}>SQL</text>
              </svg>

              {/* Architecture nodes */}
              <Appear delay={10} style={{ position: 'absolute', left: '5%', top: '34%' }}>
                <NodeBox label="Client" sub="React App" color={brand.cyan} />
              </Appear>
              <Appear delay={22} style={{ position: 'absolute', left: '26%', top: '34%' }}>
                <NodeBox label="API Gateway" sub="Kong · nginx" color={brand.purple} />
              </Appear>
              <Appear delay={34} style={{ position: 'absolute', left: '47%', top: '34%' }}>
                <NodeBox label="Auth Service" sub="JWT · OAuth2" color={brand.violet} />
              </Appear>
              <Appear delay={46} style={{ position: 'absolute', left: '68%', top: '34%' }}>
                <NodeBox label="Postgres" sub="users · sessions" color="#28c840" />
              </Appear>

              {/* Excalidraw selection handle on Auth Service */}
              <Appear delay={84} style={{ position: 'absolute', left: 'calc(47% - 6px)', top: 'calc(34% - 9px)', width: 167, height: 86 }}>
                <div
                  style={{
                    width: '100%',
                    height: '100%',
                    border: `2px dashed ${alpha(brand.cyan, 0.7)}`,
                    borderRadius: 12,
                    boxShadow: `0 0 0 6px ${alpha(brand.cyan, 0.07)}`,
                  }}
                />
              </Appear>

              {/* File badge */}
              <Appear delay={104} style={{ position: 'absolute', bottom: 22, right: 22 }}>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    padding: '7px 13px',
                    borderRadius: radius.m,
                    background: T.surface,
                    border: `1px solid ${T.border}`,
                    fontFamily: fonts.mono,
                    fontSize: 12,
                    color: T.textDim,
                  }}
                >
                  <Icon name="file" size={13} color={brand.cyan} />
                  canvas.json
                </div>
              </Appear>
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={1}
        title="Freeform diagrams on an Excalidraw canvas"
        sub="Shapes and arrows saved live to canvas.json — open in real Excalidraw any time"
      />
    </>
  );
};

// ─── Scene 3 — Mermaid + agent ───────────────────────────────────────────────

const mermaidLines: TermLine[] = [
  { text: 'flowchart LR', tone: 'cmd' },
  { text: '  Client[React App]', tone: 'text' },
  { text: '  GW[API Gateway]', tone: 'text' },
  { text: '  Auth[Auth Service]', tone: 'text' },
  { text: '  DB[(Postgres)]', tone: 'text' },
  { text: '', tone: 'dim' },
  { text: '  Client --> GW', tone: 'accent' },
  { text: '  GW --> Auth', tone: 'accent' },
  { text: '  Auth --> DB', tone: 'accent' },
];

const agentLines: TermLine[] = [
  { text: '> claude: reading canvas.mermaid…', tone: 'dim' },
  { text: '> claude: added Auth → DB edge, set direction LR', tone: 'ok' },
  { text: '> canvas.mermaid saved  ✓', tone: 'ok' },
];

const MermaidScene: React.FC = () => (
  <>
    <Stage scale={0.86}>
      <OttoWindow nav={<Navigator active="canvas" />} title="Otto — Canvas · auth-flow">
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
          <SceneTabStrip
            scenes={[{ label: 'auth-flow', active: true }, { label: 'data-model' }]}
          />
          <CanvasToolbar modeActive={1} />

          {/* Canvas area: code on left, rendered preview on right */}
          <div style={{ flex: 1, display: 'flex', overflow: 'hidden', background: T.termBg }}>

            {/* Left — mermaid source */}
            <div
              style={{
                flex: '0 0 52%',
                borderRight: `1px solid ${T.border}`,
                padding: 20,
                display: 'flex',
                flexDirection: 'column',
                gap: 10,
              }}
            >
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 7,
                  fontFamily: fonts.mono,
                  fontSize: 12,
                  color: T.textDim,
                  marginBottom: 2,
                }}
              >
                <Icon name="file" size={13} color={brand.cyan} />
                canvas.mermaid
              </div>
              <Terminal
                lines={mermaidLines}
                delay={8}
                step={10}
                fontSize={14}
                pad={16}
                style={{ flex: 1 }}
              />
              <Terminal
                lines={agentLines}
                delay={105}
                step={14}
                fontSize={13}
                pad={12}
                style={{
                  background: 'transparent',
                  borderRadius: 0,
                  borderTop: `1px solid ${T.border}`,
                  paddingTop: 10,
                }}
              />
            </div>

            {/* Right — rendered mermaid preview */}
            <div
              style={{
                flex: 1,
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'center',
                justifyContent: 'center',
                padding: 28,
                position: 'relative',
              }}
            >
              <Appear delay={55} style={{ position: 'absolute', top: 16, left: 20 }}>
                <div style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim }}>
                  Rendered preview
                </div>
              </Appear>

              <Appear delay={55}>
                <svg width={510} height={180} style={{ overflow: 'visible' }}>
                  <defs>
                    <marker id="mm-a" markerWidth="8" markerHeight="6" refX="7" refY="3" orient="auto">
                      <polygon points="0 0, 8 3, 0 6" fill={alpha('#ffffff', 0.45)} />
                    </marker>
                  </defs>

                  {/* Client */}
                  <rect x={0} y={60} width={100} height={42} rx={7}
                    fill={alpha(brand.cyan, 0.1)} stroke={alpha(brand.cyan, 0.55)} strokeWidth={1.5} />
                  <text x={50} y={86} textAnchor="middle" fontFamily={fonts.mono} fontSize={12} fill={brand.cyan}>React App</text>

                  <line x1={100} y1={81} x2={134} y2={81}
                    stroke={alpha('#ffffff', 0.35)} strokeWidth={1.5} markerEnd="url(#mm-a)" />

                  {/* API Gateway */}
                  <rect x={136} y={60} width={104} height={42} rx={7}
                    fill={alpha(brand.purple, 0.1)} stroke={alpha(brand.purple, 0.55)} strokeWidth={1.5} />
                  <text x={188} y={79} textAnchor="middle" fontFamily={fonts.mono} fontSize={11} fill={brand.purple}>API</text>
                  <text x={188} y={94} textAnchor="middle" fontFamily={fonts.mono} fontSize={11} fill={brand.purple}>Gateway</text>

                  <line x1={240} y1={81} x2={274} y2={81}
                    stroke={alpha('#ffffff', 0.35)} strokeWidth={1.5} markerEnd="url(#mm-a)" />

                  {/* Auth Service */}
                  <rect x={276} y={60} width={104} height={42} rx={7}
                    fill={alpha(brand.violet, 0.1)} stroke={alpha(brand.violet, 0.55)} strokeWidth={1.5} />
                  <text x={328} y={79} textAnchor="middle" fontFamily={fonts.mono} fontSize={11} fill={brand.violet}>Auth</text>
                  <text x={328} y={94} textAnchor="middle" fontFamily={fonts.mono} fontSize={11} fill={brand.violet}>Service</text>

                  <line x1={380} y1={81} x2={408} y2={81}
                    stroke={alpha('#ffffff', 0.35)} strokeWidth={1.5} markerEnd="url(#mm-a)" />

                  {/* Postgres */}
                  <rect x={410} y={60} width={100} height={42} rx={7}
                    fill={alpha('#28c840', 0.1)} stroke={alpha('#28c840', 0.55)} strokeWidth={1.5} />
                  <text x={460} y={79} textAnchor="middle" fontFamily={fonts.mono} fontSize={11} fill="#28c840">Postgres</text>
                  <text x={460} y={94} textAnchor="middle" fontFamily={fonts.mono} fontSize={10} fill={alpha('#28c840', 0.65)}>(database)</text>
                </svg>
              </Appear>
            </div>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Diagram-as-code in Mermaid"
      sub="An agent edits canvas.mermaid — you chat in the embedded terminal below"
    />
  </>
);

// ─── Scene 4 — Scenes + present ──────────────────────────────────────────────

const ScenesScene: React.FC = () => {
  const frame = useCurrentFrame();
  const presentGlow = track(frame, [38, 58], [0, 1]);

  return (
    <>
      <Stage scale={0.86}>
        <OttoWindow nav={<Navigator active="canvas" />} title="Otto — Canvas">
          <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
            <SceneTabStrip
              scenes={[
                { label: 'auth-flow', active: true },
                { label: 'data-model' },
                { label: 'deploy' },
              ]}
            />
            <CanvasToolbar modeActive={0} showPresentHint />

            {/* Canvas area */}
            <div style={{ flex: 1, position: 'relative', overflow: 'hidden', background: T.termBg }}>

              {/* Dot grid */}
              <div
                style={{
                  position: 'absolute',
                  inset: 0,
                  backgroundImage: `radial-gradient(circle, ${alpha('#ffffff', 0.08)} 1px, transparent 1px)`,
                  backgroundSize: '28px 28px',
                  opacity: 0.7,
                }}
              />

              {/* Ghost of the auth-flow diagram, dimmed */}
              <div
                style={{
                  position: 'absolute',
                  inset: 0,
                  opacity: 0.22,
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  gap: 52,
                }}
              >
                {['Client', 'API Gateway', 'Auth Service', 'Postgres'].map((label, i) => (
                  <div
                    key={i}
                    style={{
                      width: 130,
                      padding: '12px 14px',
                      background: alpha(brand.purple, 0.1),
                      border: `1px solid ${alpha(brand.purple, 0.3)}`,
                      borderRadius: 8,
                      fontFamily: fonts.ui,
                      fontSize: 13,
                      fontWeight: 600,
                      color: alpha('#fff', 0.7),
                      textAlign: 'center',
                    }}
                  >
                    {label}
                  </div>
                ))}
              </div>

              {/* Present mode modal */}
              <Appear
                delay={28}
                style={{
                  position: 'absolute',
                  inset: 0,
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                }}
              >
                <div
                  style={{
                    display: 'flex',
                    flexDirection: 'column',
                    alignItems: 'center',
                    gap: 16,
                    padding: '36px 54px',
                    borderRadius: radius.xl,
                    background: alpha(T.surface, 0.92),
                    border: `1px solid ${alpha(brand.cyan, 0.32 * presentGlow)}`,
                    boxShadow: `0 24px 70px rgba(0,0,0,0.65), 0 0 0 1px ${alpha(brand.cyan, 0.1 * presentGlow)}`,
                  }}
                >
                  <div
                    style={{
                      width: 54,
                      height: 54,
                      borderRadius: radius.l,
                      background: alpha(brand.cyan, 0.13),
                      border: `1.5px solid ${alpha(brand.cyan, 0.45)}`,
                      display: 'grid',
                      placeItems: 'center',
                    }}
                  >
                    <Icon name="maximize" size={24} color={brand.cyan} />
                  </div>
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 21,
                      fontWeight: 700,
                      color: T.text,
                      textAlign: 'center',
                    }}
                  >
                    Present mode
                  </div>
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 14,
                      color: T.textDim,
                      textAlign: 'center',
                      maxWidth: 320,
                      lineHeight: 1.55,
                    }}
                  >
                    Full-screen walkthrough across all canvas scenes — perfect for design reviews and demos.
                  </div>
                  <div
                    style={{
                      display: 'flex',
                      gap: 28,
                      marginTop: 6,
                      fontFamily: fonts.ui,
                      fontSize: 13,
                      color: T.textDim,
                    }}
                  >
                    <span>← Prev</span>
                    <span style={{ color: brand.cyan, fontWeight: 600 }}>● 1 / 3</span>
                    <span>Next →</span>
                  </div>
                </div>
              </Appear>
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={3}
        title="Many scenes per canvas"
        sub="Tab between diagrams · hit Present for a full-screen review walkthrough"
      />
    </>
  );
};

// ─── Scenes array ─────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 75,  node: <TitleScene />,       name: 'Title' },
  { dur: 145, node: <ExcalidrawScene />,  name: 'Excalidraw' },
  { dur: 145, node: <MermaidScene />,     name: 'Mermaid' },
  { dur: 105, node: <ScenesScene />,      name: 'Scenes' },
  {
    dur: 130,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Canvas"
        tagline="Think visually — diagrams an agent can build with you"
        pills={[
          { label: 'Excalidraw', icon: 'shapes' },
          { label: 'Mermaid',    icon: 'split'  },
          { label: 'Agent-edited', icon: 'edit' },
          { label: 'Present mode', icon: 'maximize' },
        ]}
      />
    ),
  },
];

export const canvasDuration = scenesDuration(SCENES);
export const Canvas: React.FC = () => <Scenes scenes={SCENES} />;
