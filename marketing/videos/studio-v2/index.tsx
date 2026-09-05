import React from "react";
import { Composition, registerRoot } from "remotion";
import { episodes, FPS } from "./model";
import { Film } from "./Film";
const Root = () => (
  <>
    {episodes.map((episode) => (
      <Composition
        key={episode.id}
        id={episode.id}
        component={Film}
        defaultProps={{ episode }}
        width={1920}
        height={1080}
        fps={FPS}
        durationInFrames={Math.round(episode.duration * FPS)}
      />
    ))}
  </>
);
registerRoot(Root);
