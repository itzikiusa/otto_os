import { Config } from '@remotion/cli/config';

// Kept deliberately modest: the render shares a busy machine.
Config.setVideoImageFormat('jpeg');
Config.setJpegQuality(92);
Config.setConcurrency(4);
Config.setOverwriteOutput(true);
