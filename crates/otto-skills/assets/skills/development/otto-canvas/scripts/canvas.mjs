#!/usr/bin/env node
// canvas.mjs — drive Otto's Canvas Studio over HTTP from an agent session.
//
// Zero dependencies (uses Node's global `fetch`, Node 18+). Authenticates with a
// Bearer token and talks to the running ottod daemon. Reads:
//
//   OTTO_API_TOKEN   required — the Bearer token (mint via the otto-api skill)
//   OTTO_BASE        base URL, default http://127.0.0.1:7700
//                    (OTTO_BASE_URL is also honored, for parity with `otto`)
//
// All paths are under /api/v1. Writes need workspace Editor; reads need Viewer.
//
// USAGE
//   node canvas.mjs whoami
//   node canvas.mjs list-scenes  <wsId>
//   node canvas.mjs create-scene <wsId> "<title>" [storyId]   # empty scene
//   node canvas.mjs get-scene    <sceneId>
//   node canvas.mjs add-mermaid  <sceneId> "<mermaid src>"    # append a mermaid node, PUT back
//   node canvas.mjs add-slide    <sceneId> "<slide title>"    # append a slide revealing all nodes
//   node canvas.mjs assist       <sceneId> "<prompt>" [mode]  # assist on a scene (auto|sequence|flow|uml|nodes)
//   node canvas.mjs assist       --preview "<prompt>" [mode]  # assist with no scene
//
// add-mermaid / add-slide do a read-modify-write: GET the scene, JSON.parse its
// doc_json, mutate the Scene object per the schema, then PUT the whole doc back.
//
// Output is pretty JSON on stdout. Any non-2xx prints `HTTP <code>` + the body to
// stderr and exits 1. Other failures (missing token/args) exit 2.

const BASE = (process.env.OTTO_BASE || process.env.OTTO_BASE_URL || 'http://127.0.0.1:7700').replace(/\/+$/, '');
const TOKEN = process.env.OTTO_API_TOKEN || '';

// --- tiny HTTP helper -------------------------------------------------------

/** Call /api/v1<path>. Returns parsed JSON (or null for 204). Throws on non-2xx. */
async function api(method, path, body) {
  if (!TOKEN) fail(2, 'OTTO_API_TOKEN is not set — mint one via the otto-api skill, then `export OTTO_API_TOKEN=…`.');
  const url = `${BASE}/api/v1${path}`;
  const headers = { authorization: `Bearer ${TOKEN}` };
  const init = { method, headers };
  if (body !== undefined) {
    headers['content-type'] = 'application/json';
    init.body = JSON.stringify(body);
  }
  let res;
  try {
    res = await fetch(url, init);
  } catch (e) {
    fail(1, `request failed: ${method} ${url}\n${e && e.message ? e.message : e}`);
  }
  const text = await res.text();
  if (!res.ok) {
    // Surface the daemon's {code,message} (or raw body) verbatim — never swallow it.
    fail(1, `HTTP ${res.status} ${res.statusText}  ${method} ${path}\n${text}`);
  }
  if (res.status === 204 || text.trim() === '') return null;
  try {
    return JSON.parse(text);
  } catch {
    return text; // non-JSON 2xx (shouldn't happen for these routes)
  }
}

function fail(code, msg) {
  process.stderr.write(String(msg).replace(/\s*$/, '') + '\n');
  process.exit(code);
}

function print(v) {
  process.stdout.write((typeof v === 'string' ? v : JSON.stringify(v, null, 2)) + '\n');
}

function need(val, what) {
  if (val === undefined || val === null || val === '') fail(2, `missing argument: ${what}\nRun \`node canvas.mjs --help\` for usage.`);
  return val;
}

// --- scene helpers ----------------------------------------------------------

/** Fetch a scene row and return its parsed Scene document (`doc_json` → object). */
async function loadScene(id) {
  const row = await api('GET', `/canvas/scenes/${encodeURIComponent(id)}`);
  let doc;
  try {
    doc = JSON.parse(row.doc_json);
  } catch (e) {
    fail(1, `scene ${id} has unparseable doc_json: ${e.message}`);
  }
  // Defensive: make sure the arrays exist so we can append.
  doc.schema = doc.schema || 1;
  doc.nodes = Array.isArray(doc.nodes) ? doc.nodes : [];
  doc.edges = Array.isArray(doc.edges) ? doc.edges : [];
  doc.slides = Array.isArray(doc.slides) ? doc.slides : [];
  if (!doc.title) doc.title = row.title || 'Untitled';
  return { row, doc };
}

/** PUT a mutated Scene document back onto a scene. Returns the updated row. */
async function saveScene(id, doc) {
  return api('PUT', `/canvas/scenes/${encodeURIComponent(id)}`, { doc });
}

/** A fresh id for a node/slide, unique enough within one scene. */
function genId(prefix) {
  return `${prefix}${Date.now().toString(36)}${Math.floor(Math.random() * 1e4).toString(36)}`;
}

/** Lay a new node below the current content so it doesn't overlap existing ones. */
function nextY(doc) {
  let maxBottom = 0;
  for (const n of doc.nodes) {
    const b = (Number(n.y) || 0) + (Number(n.h) || 0);
    if (b > maxBottom) maxBottom = b;
  }
  return maxBottom === 0 ? 80 : maxBottom + 40;
}

// --- subcommands ------------------------------------------------------------

async function cmdWhoami() {
  print(await api('GET', '/auth/me'));
}

async function cmdListScenes(wsId) {
  need(wsId, 'wsId');
  print(await api('GET', `/workspaces/${encodeURIComponent(wsId)}/canvas/scenes`));
}

async function cmdCreateScene(wsId, title, storyId) {
  need(wsId, 'wsId');
  need(title, 'title');
  const body = { title };
  if (storyId) body.story_id = storyId;
  print(await api('POST', `/workspaces/${encodeURIComponent(wsId)}/canvas/scenes`, body));
}

async function cmdGetScene(id) {
  need(id, 'sceneId');
  print(await api('GET', `/canvas/scenes/${encodeURIComponent(id)}`));
}

async function cmdAddMermaid(id, src) {
  need(id, 'sceneId');
  need(src, 'mermaid src');
  const { doc } = await loadScene(id);
  // Infer the diagram-kind hint from the first non-empty line (best effort).
  const first = (src.trim().split('\n')[0] || '').toLowerCase();
  const kind = first.startsWith('sequencediagram') ? 'sequence'
    : first.startsWith('flowchart') || first.startsWith('graph') ? 'flowchart'
    : first.startsWith('classdiagram') ? 'class'
    : first.startsWith('statediagram') ? 'state'
    : first.startsWith('erdiagram') ? 'er'
    : undefined;
  const node = {
    id: genId('m'),
    kind: 'mermaid',
    x: 80,
    y: nextY(doc),
    w: 560,
    h: 400,
    mermaid: kind ? { src, kind } : { src },
  };
  doc.nodes.push(node);
  const updated = await saveScene(id, doc);
  print({ added_node: node.id, kind, scene: { id: updated.id, title: updated.title, nodes: doc.nodes.length } });
}

async function cmdAddSlide(id, title) {
  need(id, 'sceneId');
  const { doc } = await loadScene(id);
  // A slide that reveals every node currently in the scene, in one step.
  const slide = {
    id: genId('s'),
    title: title || `Slide ${doc.slides.length + 1}`,
    reveal: [{ nodeIds: doc.nodes.map((n) => n.id) }],
  };
  doc.slides.push(slide);
  const updated = await saveScene(id, doc);
  print({ added_slide: slide.id, reveals_nodes: slide.reveal[0].nodeIds.length, scene: { id: updated.id, slides: doc.slides.length } });
}

async function cmdAssist(target, prompt, mode) {
  need(target, 'sceneId | --preview');
  need(prompt, 'prompt');
  const body = { prompt };
  if (mode) body.mode = mode;
  const path = target === '--preview'
    ? '/canvas/assist/preview'
    : `/canvas/scenes/${encodeURIComponent(target)}/assist`;
  const result = await api('POST', path, body);
  // Reminder: assist does NOT mutate the scene — insert what you want.
  if (result && result.mermaid) {
    process.stderr.write(`note: got a mermaid block — persist it with:\n  node canvas.mjs add-mermaid <sceneId> "<the mermaid above>"\n`);
  }
  print(result);
}

// --- dispatch ---------------------------------------------------------------

const HELP = `canvas.mjs — drive Otto Canvas Studio over HTTP (OTTO_API_TOKEN, OTTO_BASE)

  whoami
  list-scenes  <wsId>
  create-scene <wsId> "<title>" [storyId]
  get-scene    <sceneId>
  add-mermaid  <sceneId> "<mermaid src>"
  add-slide    <sceneId> ["<slide title>"]
  assist       <sceneId> "<prompt>" [auto|sequence|flow|uml|nodes]
  assist       --preview "<prompt>" [mode]

env: OTTO_API_TOKEN (required), OTTO_BASE (default http://127.0.0.1:7700)`;

async function main() {
  const [cmd, ...rest] = process.argv.slice(2);
  switch (cmd) {
    case 'whoami': return cmdWhoami();
    case 'list-scenes': return cmdListScenes(rest[0]);
    case 'create-scene': return cmdCreateScene(rest[0], rest[1], rest[2]);
    case 'get-scene': return cmdGetScene(rest[0]);
    case 'add-mermaid': return cmdAddMermaid(rest[0], rest[1]);
    case 'add-slide': return cmdAddSlide(rest[0], rest[1]);
    case 'assist': return cmdAssist(rest[0], rest[1], rest[2]);
    case undefined:
    case '-h':
    case '--help':
      print(HELP);
      return;
    default:
      fail(2, `unknown command: ${cmd}\n\n${HELP}`);
  }
}

main().catch((e) => fail(1, e && e.stack ? e.stack : String(e)));
