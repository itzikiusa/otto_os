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

const F = VIDEO.fps;

export const RemotionRoot: React.FC = () => {
  return (
    <>
      <Composition id="Intro" component={Intro} durationInFrames={20 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="Settings" component={Settings} durationInFrames={36 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="Shortcuts" component={Shortcuts} durationInFrames={32 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="AgentMode" component={AgentMode} durationInFrames={42 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="Connections" component={Connections} durationInFrames={36 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="GitPr" component={GitPr} durationInFrames={46 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
      <Composition id="Product" component={Product} durationInFrames={60 * F} fps={F} width={VIDEO.width} height={VIDEO.height} />
    </>
  );
};
