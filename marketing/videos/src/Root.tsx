import React from 'react';
import { Composition } from 'remotion';
import { VIDEO } from './theme';
import { Intro, introDuration } from './compositions/Intro';
import { Sessions, sessionsDuration } from './compositions/Sessions';
import { MissionControl, missionControlDuration } from './compositions/MissionControl';
import { Git, gitDuration } from './compositions/Git';
import { Review, reviewDuration } from './compositions/Review';
import { ProofPacks, proofPacksDuration } from './compositions/ProofPacks';
import { Product, productDuration } from './compositions/Product';
import { Canvas, canvasDuration } from './compositions/Canvas';
import { Swarm, swarmDuration } from './compositions/Swarm';
import { GoalLoops, goalLoopsDuration } from './compositions/GoalLoops';
import { Connections, connectionsDuration } from './compositions/Connections';
import { Database, databaseDuration } from './compositions/Database';
import { Brokers, brokersDuration } from './compositions/Brokers';
import { Channels, channelsDuration } from './compositions/Channels';
import { Workflows, workflowsDuration } from './compositions/Workflows';
import { ScheduledTasks, scheduledTasksDuration } from './compositions/ScheduledTasks';
import { Mcp, mcpDuration } from './compositions/Mcp';
import { Vault, vaultDuration } from './compositions/Vault';
import { Skills, skillsDuration } from './compositions/Skills';
import { SkillsEval, skillsEvalDuration } from './compositions/SkillsEval';
import { UsageInsights, usageInsightsDuration } from './compositions/UsageInsights';
import { Api, apiDuration } from './compositions/Api';
import { Plugins, pluginsDuration } from './compositions/Plugins';
import { TeamMobile, teamMobileDuration } from './compositions/TeamMobile';
import { Platform, platformDuration } from './compositions/Platform';
import { Outro, outroDuration } from './compositions/Outro';

const F = VIDEO.fps;
const W = VIDEO.width;
const H = VIDEO.height;

export const RemotionRoot: React.FC = () => {
  return (
    <>
      <Composition id="Intro" component={Intro} durationInFrames={introDuration} fps={F} width={W} height={H} />
      <Composition id="Sessions" component={Sessions} durationInFrames={sessionsDuration} fps={F} width={W} height={H} />
      <Composition id="MissionControl" component={MissionControl} durationInFrames={missionControlDuration} fps={F} width={W} height={H} />
      <Composition id="Git" component={Git} durationInFrames={gitDuration} fps={F} width={W} height={H} />
      <Composition id="Review" component={Review} durationInFrames={reviewDuration} fps={F} width={W} height={H} />
      <Composition id="ProofPacks" component={ProofPacks} durationInFrames={proofPacksDuration} fps={F} width={W} height={H} />
      <Composition id="Product" component={Product} durationInFrames={productDuration} fps={F} width={W} height={H} />
      <Composition id="Canvas" component={Canvas} durationInFrames={canvasDuration} fps={F} width={W} height={H} />
      <Composition id="Swarm" component={Swarm} durationInFrames={swarmDuration} fps={F} width={W} height={H} />
      <Composition id="GoalLoops" component={GoalLoops} durationInFrames={goalLoopsDuration} fps={F} width={W} height={H} />
      <Composition id="Connections" component={Connections} durationInFrames={connectionsDuration} fps={F} width={W} height={H} />
      <Composition id="Database" component={Database} durationInFrames={databaseDuration} fps={F} width={W} height={H} />
      <Composition id="Brokers" component={Brokers} durationInFrames={brokersDuration} fps={F} width={W} height={H} />
      <Composition id="Channels" component={Channels} durationInFrames={channelsDuration} fps={F} width={W} height={H} />
      <Composition id="Workflows" component={Workflows} durationInFrames={workflowsDuration} fps={F} width={W} height={H} />
      <Composition id="ScheduledTasks" component={ScheduledTasks} durationInFrames={scheduledTasksDuration} fps={F} width={W} height={H} />
      <Composition id="Mcp" component={Mcp} durationInFrames={mcpDuration} fps={F} width={W} height={H} />
      <Composition id="Vault" component={Vault} durationInFrames={vaultDuration} fps={F} width={W} height={H} />
      <Composition id="Skills" component={Skills} durationInFrames={skillsDuration} fps={F} width={W} height={H} />
      <Composition id="SkillsEval" component={SkillsEval} durationInFrames={skillsEvalDuration} fps={F} width={W} height={H} />
      <Composition id="UsageInsights" component={UsageInsights} durationInFrames={usageInsightsDuration} fps={F} width={W} height={H} />
      <Composition id="Api" component={Api} durationInFrames={apiDuration} fps={F} width={W} height={H} />
      <Composition id="Plugins" component={Plugins} durationInFrames={pluginsDuration} fps={F} width={W} height={H} />
      <Composition id="TeamMobile" component={TeamMobile} durationInFrames={teamMobileDuration} fps={F} width={W} height={H} />
      <Composition id="Platform" component={Platform} durationInFrames={platformDuration} fps={F} width={W} height={H} />
      <Composition id="Outro" component={Outro} durationInFrames={outroDuration} fps={F} width={W} height={H} />
    </>
  );
};
