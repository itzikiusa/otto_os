import React from 'react';
import { Composition, Still } from 'remotion';
import timing from './generated/timing.json';
import { Tour } from './Tour';
import { Poster } from './Poster';

export const Root: React.FC = () => (
  <>
    <Composition id="OttoTour" component={Tour} durationInFrames={timing.totalFrames} fps={timing.fps} width={1920} height={1080} />
    <Still id="Poster" component={Poster} width={1920} height={1080} />
  </>
);
