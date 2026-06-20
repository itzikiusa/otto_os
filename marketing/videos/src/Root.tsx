import React from 'react';
import { Composition } from 'remotion';
import { VIDEO } from './theme';
import { Intro } from './compositions/Intro';
import { Settings } from './compositions/Settings';
import { Shortcuts } from './compositions/Shortcuts';
import { AgentMode } from './compositions/AgentMode';
import { Connections } from './compositions/Connections';
import { GitPr } from './compositions/GitPr';
import { Product } from './compositions/Product';
import { Database } from './compositions/Database';
import { Brokers } from './compositions/Brokers';
import { Vault } from './compositions/Vault';
import { Insights } from './compositions/Insights';
import { Skills } from './compositions/Skills';
import { Swarm } from './compositions/Swarm';
import { Sharing } from './compositions/Sharing';

const F = VIDEO.fps;

export const RemotionRoot: React.FC = () => {
  return (
    <>
      <Composition id="Intro"       component={Intro}       durationInFrames={20 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="Settings"    component={Settings}    durationInFrames={36 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="Shortcuts"   component={Shortcuts}   durationInFrames={32 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="AgentMode"   component={AgentMode}   durationInFrames={42 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="Connections" component={Connections} durationInFrames={36 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="GitPr"       component={GitPr}       durationInFrames={46 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="Product"     component={Product}     durationInFrames={60 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      {/* ── New compositions ───────────────────────────────────────────────── */}
      <Composition id="Database"    component={Database}    durationInFrames={36 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="Brokers"     component={Brokers}     durationInFrames={38 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="Vault"       component={Vault}       durationInFrames={30 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="Insights"    component={Insights}    durationInFrames={28 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="Skills"      component={Skills}      durationInFrames={28 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="Swarm"       component={Swarm}       durationInFrames={36 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="Sharing"     component={Sharing}     durationInFrames={34 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
    </>
  );
};
