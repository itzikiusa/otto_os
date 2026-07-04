// DATA_DIR persistence: config, per-project corpus, per-project goals,
// per-project overrides (outlier marks + manual times), per-project estimate
// cache, global people registry, per-account scope goals, git features.
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
const overridesPath = (dataDir, account, project) => path.join(dataDir, 'overrides', `${safe(account)}__${safe(project)}.json`);
const estimatesPath = (dataDir, account, project) => path.join(dataDir, 'estimates', `${safe(account)}__${safe(project)}.json`);
const peoplePath = (dataDir) => path.join(dataDir, 'people.json');
const scopeGoalsPath = (dataDir, account) => path.join(dataDir, 'goals', `scope__${safe(account)}.json`);
const featuresPath = (dataDir) => path.join(dataDir, 'git_features.json');
const reportsDir = (dataDir, account) => path.join(dataDir, 'reports', safe(account));
const reportsIndexPath = (dataDir, account) => path.join(reportsDir(dataDir, account), 'index.json');
const reportFilePath = (dataDir, account, name) => path.join(reportsDir(dataDir, account), safe(name) + '.html');

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

/** Scanned project keys for an account (from corpus files on disk). */
function listProjects(dataDir, account) {
  const prefix = `${safe(account)}__`;
  try {
    return fs
      .readdirSync(path.join(dataDir, 'data'))
      .filter((f) => f.startsWith(prefix) && f.endsWith('.json'))
      .map((f) => f.slice(prefix.length, -5));
  } catch {
    return [];
  }
}

module.exports = {
  readJson,
  writeJsonAtomic,
  configPath,
  corpusPath,
  goalsPath,
  overridesPath,
  estimatesPath,
  peoplePath,
  scopeGoalsPath,
  featuresPath,
  reportsDir,
  reportsIndexPath,
  reportFilePath,
  listCorpora,
  listProjects,
  SCHEMA,
};
