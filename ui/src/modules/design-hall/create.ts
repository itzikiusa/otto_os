// Design Hall — creating artifacts: the starter document per studio/format
// (reusing the Product Design Arena's templates and blank starters), the
// create call itself, and file import. Nothing here generates with an agent:
// Phase 0 creates a DRAFT the person edits; generation is Phase 1.

import * as design from '../../lib/api/design';
import type { CreateDesignArtifactReq, DesignArtifactFormat, DesignStudio } from '../../lib/api/types';
import { DESIGN_TEMPLATES, blankSource, type DesignTemplate } from '../product/design/templates';
import { emptyScene, serializeScene, studioScene } from '../product/design/scene3d';
import { fileToB64 } from '../product/design/format';
import { brandStarter, formatForFile, isTextFormat, studioInfo } from './model';

/** A starter graphic: a 1080×1350 portrait tile as plain HTML. */
const GRAPHIC_STARTER =
  '<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n<title>Graphic</title>\n<style>\n' +
  '  html, body { margin: 0; background: #f4f4f5; }\n' +
  '  .tile { width: 1080px; height: 1350px; margin: 0 auto; display: flex; flex-direction: column; justify-content: flex-end;\n' +
  '    padding: 96px; box-sizing: border-box; background: #111827; color: #fff; font: 800 96px/1.05 -apple-system, system-ui, sans-serif; }\n' +
  '  .tile small { display: block; margin-top: 32px; font: 500 32px/1.4 -apple-system, system-ui, sans-serif; opacity: .8; }\n' +
  '</style>\n</head>\n<body>\n<div class="tile">Headline goes here<small>One supporting line.</small></div>\n</body>\n</html>\n';

const D2_STARTER = 'direction: right\nidea -> draft -> review -> shipped\n';

/** Templates the arena ships, offered per format. */
export function templatesFor(format: string): DesignTemplate[] {
  return DESIGN_TEMPLATES.filter((t) => t.format === format);
}

/** The first document of a new artifact. */
export function starterContent(studio: DesignStudio, format: string, title: string, templateId?: string): string {
  const tpl = templateId ? DESIGN_TEMPLATES.find((t) => t.id === templateId) : undefined;
  if (tpl && tpl.format === format) return tpl.source;
  switch (format) {
    case 'html':
      return studio === 'graphics' ? GRAPHIC_STARTER : blankSource('html', emptyScene);
    case 'mermaid':
    case 'excalidraw':
      return blankSource(format, emptyScene);
    case 'scene3d':
      // 3D Studio 1.5 starter (scene3d v2): studio environment, key + rim, a plinth, Idle/Hover, a Hero view.
      return serializeScene(studioScene());
    case 'd2':
      return D2_STARTER;
    case 'svg':
      return (
        '<svg xmlns="http://www.w3.org/2000/svg" width="1080" height="1080" viewBox="0 0 1080 1080">\n' +
        '  <rect width="1080" height="1080" fill="#111827"/>\n' +
        '  <text x="96" y="960" fill="#ffffff" font-family="-apple-system, system-ui, sans-serif" font-size="88" font-weight="800">Headline</text>\n' +
        '</svg>\n'
      );
    case 'otto-brand':
      return JSON.stringify(brandStarter(title), null, 2) + '\n';
    default:
      return '';
  }
}

export interface NewDesignInput {
  workspaceId: string;
  studio: DesignStudio;
  format: string;
  title: string;
  projectId?: string | null;
  storyId?: string | null;
  templateId?: string;
  /** The free-text brief from the lobby prompt (kept on the artifact). */
  brief?: string;
}

/** Create the draft and return its id. */
export async function createDesign(input: NewDesignInput): Promise<string> {
  const body: CreateDesignArtifactReq = {
    workspace_id: input.workspaceId,
    studio: input.studio,
    format: input.format as DesignArtifactFormat,
    title: input.title.trim() || `Untitled ${studioInfo(input.studio).name.toLowerCase()}`,
    content: starterContent(input.studio, input.format, input.title, input.templateId),
    message: input.templateId ? `Started from the “${input.templateId}” template` : 'Created in Design Hall',
  };
  if (input.projectId) body.project_id = input.projectId;
  if (input.storyId) body.story_id = input.storyId;
  if (input.brief?.trim()) body.meta = { brief: input.brief.trim().slice(0, 4000) };
  const res = await design.createArtifact(body);
  return res.artifact.id;
}

/** Import a local file as a new artifact (format from its extension / mime). */
export async function importDesignFile(
  file: File,
  opts: { workspaceId: string; projectId?: string | null },
): Promise<string> {
  const format = formatForFile(file.name, file.type);
  if (!format) throw new Error(`Design Hall can’t import “${file.name}”. Use HTML, SVG, PNG, JPEG, GIF, WebP, PDF, GLB, glTF, Mermaid, D2 or Excalidraw.`);
  const title = file.name.replace(/\.[^.]+$/, '') || file.name;
  const body: CreateDesignArtifactReq = {
    workspace_id: opts.workspaceId,
    format: format as DesignArtifactFormat,
    title,
    message: `Imported ${file.name}`,
  };
  if (opts.projectId) body.project_id = opts.projectId;
  if (isTextFormat(format)) body.content = await file.text();
  else body.content_b64 = await fileToB64(file);
  const res = await design.createArtifact(body);
  return res.artifact.id;
}

/** Files the import picker accepts. */
export const IMPORT_ACCEPT =
  '.html,.htm,.svg,.png,.jpg,.jpeg,.gif,.webp,.pdf,.glb,.gltf,.mmd,.mermaid,.d2,.excalidraw';
