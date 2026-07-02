// DATA_DIR persistence: config, per-project corpus, per-project goals.
// Atomic writes (tmp + rename); tolerant loaders (corrupt/missing → fallback).
'use strict';

const fs = require('fs');
const path = require('path');

const SCHEMA = 1;

function readJson(file, fallback) {
  try {
    const obj = JSON.parse(fs.readFileSync(file, 'utf8'));
    if (obj && typeof obj === 'object' && obj.schema !== undefined && obj.schema !== SCHEMA) return fallback;
    return obj;
  } catch {
    return fallback;
  }
}

function writeJsonAtomic(file, obj) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.tmp-${process.pid}`;
  fs.writeFileSync(tmp, JSON.stringify({ schema: SCHEMA, ...obj }));
  fs.renameSync(tmp, file);
}

function safe(s) {
  return String(s).replace(/[^a-zA-Z0-9_-]/g, '_');
}

const configPath = (dataDir) => path.join(dataDir, 'config.json');
const corpusPath = (dataDir, account, project) => path.join(dataDir, 'data', `${safe(account)}__${safe(project)}.json`);
const goalsPath = (dataDir, account, project) => path.join(dataDir, 'goals', `${safe(account)}__${safe(project)}.json`);

/** Every corpus file currently on disk (for config-change recompute). */
function listCorpora(dataDir) {
  try {
    return fs
      .readdirSync(path.join(dataDir, 'data'))
      .filter((f) => f.endsWith('.json'))
      .map((f) => path.join(dataDir, 'data', f));
  } catch {
    return [];
  }
}

module.exports = { readJson, writeJsonAtomic, configPath, corpusPath, goalsPath, listCorpora, SCHEMA };
