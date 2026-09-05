import catalog from "../../../ui/src/lib/walkthroughs/catalog.json";
export type Chapter = {
  id: string;
  title: string;
  route: string;
  doc: string;
  view: string;
  heading: string;
  detail: string;
  items: string[];
  steps: string[];
  actions: string[];
  source: string;
  start: number;
  duration: number;
  alsoRoutes?: string[];
  stepStarts?: number[];
  voice?: { file: string; duration: number }[];
};
export type Episode = {
  id: string;
  file: string;
  title: string;
  desc: string;
  accent: string;
  number: number;
  introSeconds: number;
  outroSeconds: number;
  duration: number;
  tags: string;
  doc: string;
  chapters: Chapter[];
};
export const episodes: Episode[] = catalog;
export const FPS = 30;
export const beatStarts = (chapter: Chapter) =>
  chapter.stepStarts ?? [1.2, 6.5, 12];
