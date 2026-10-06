// Input validators for every value that reaches JQL, git, or the filesystem.
// Each returns the normalized value or throws a ValidationError (status 400).
'use strict';
const fs = require('fs');
const path = require('path');

class ValidationError extends Error {
  constructor(msg) { super(msg); this.name = 'ValidationError'; this.status = 400; }
}

const PROJECT_KEY_RE = /^[A-Z][A-Z0-9_]+$/;
const ACCOUNT_ID_RE = /^[A-Za-z0-9_-]{1,64}$/;

function projectKey(v) {
  const s = String(v ?? '').trim();
  if (s.length > 32 || !PROJECT_KEY_RE.test(s)) throw new ValidationError(`invalid project key`);
  return s;
}

function accountId(v) {
  const s = String(v ?? '').trim();
  if (!ACCOUNT_ID_RE.test(s)) throw new ValidationError('invalid account id');
  return s;
}

// Escape a value for use inside a double-quoted JQL string literal.
function jqlString(v) {
  const s = String(v ?? '');
  if (/[\u0000-\u001f]/.test(s)) throw new ValidationError('control characters not allowed in JQL value');
  return `"${s.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/'/g, "\\'")}"`;
}

function repoPath(v) {
  const s = String(v ?? '');
  if (!s || s.startsWith('-') || s.includes('\0') || !path.isAbsolute(s)) throw new ValidationError('repo path must be absolute');
  const p = path.normalize(s);
  if (path.basename(p).startsWith('-')) throw new ValidationError('invalid repo path');
  let st;
  try { st = fs.statSync(p); } catch { throw new ValidationError('repo path does not exist'); }
  if (!st.isDirectory() || !fs.existsSync(path.join(p, '.git'))) throw new ValidationError('repo path is not a git repository');
  return p;
}

const DEFAULT_MAX = 2 * 1024 * 1024;
function readBodyCapped(req, maxBytes = DEFAULT_MAX) {
  return new Promise((resolve, reject) => {
    const declared = Number(req.headers && req.headers['content-length']);
    if (Number.isFinite(declared) && declared > maxBytes) {
      const e = new ValidationError('request body too large'); e.status = 413; req.resume?.(); return reject(e);
    }
    const chunks = []; let size = 0; let done = false;
    req.on('data', (c) => {
      if (done) return;
      size += c.length;
      if (size > maxBytes) {
        done = true; const e = new ValidationError('request body too large'); e.status = 413;
        req.destroy?.(); return reject(e);
      }
      chunks.push(c);
    });
    req.on('end', () => { if (!done) { done = true; resolve(Buffer.concat(chunks).toString('utf8')); } });
    req.on('error', (e) => { if (!done) { done = true; reject(e); } });
  });
}

module.exports = { ValidationError, projectKey, accountId, jqlString, repoPath, readBodyCapped, PROJECT_KEY_RE };
