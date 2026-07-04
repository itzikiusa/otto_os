// Dev-agnostic AI task estimation: batched agent calls with a content-hash
// cache. Level 1 of the three estimation levels (agnostic → per-dev → actual).
//
// The agent (not the developer) is responsible for the estimate: it reads each
// issue's type/points/summary/description and answers with ideal engineering
// days for an average developer on this codebase, plus a `routine` flag for
// repetitive/mechanical work (version bumps, config-only changes …).
//
// Estimates are cached forever by (key, content hash) — an issue is only
// re-estimated when its text changes. Batches are capped per scan so one scan
// never runs away on tokens; the remainder is picked up by the next scan.
'use strict';

const BATCH_SIZE = 15;
const PACE_MS = 500; // between agent calls — the calls themselves are heavy

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

/** FNV-1a over the estimation-relevant content of a record. */
function contentHash(record) {
  const s = `${record.summary}|${record.description_snippet || ''}|${record.type}|${record.points ?? ''}`;
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = (h * 0x01000193) >>> 0;
  }
  return h.toString(36);
}

const DAY = 86400000;

/**
 * Records that still need an estimate: open, or completed within the window
 * (months; 0 = no window → everything) — an explicit `sinceMs` cutoff wins
 * over the month window — and not already cached at this hash.
 */
function selectTargets(records, cache, windowMonths, nowMs, sinceMs = 0) {
  const cutoff = sinceMs > 0 ? sinceMs : windowMonths > 0 ? nowMs - windowMonths * 30 * DAY : -Infinity;
  const out = [];
  for (const r of records) {
    // Estimates size the WHOLE task, never a sub-task: splitting a 2-day task
    // into ten 0.5-day sub-tasks must not manufacture "accurate" estimates.
    if (r.subtask) continue;
    const doneAt = r.eff_done_at ?? r.done_at ?? null;
    if (doneAt !== null && doneAt < cutoff) continue;
    const hit = cache[r.key];
    if (hit && hit.hash === contentHash(r)) continue;
    out.push(r);
  }
  // Most-recent first — the tasks the lead is actually looking at.
  out.sort((a, b) => (b.eff_done_at ?? b.done_at ?? b.updated ?? 0) - (a.eff_done_at ?? a.done_at ?? a.updated ?? 0));
  return out;
}

function batchPrompt(records) {
  const lines = records.map((r) => {
    const desc = (r.description_snippet || '').slice(0, 400);
    return `- key=${r.key} type=${r.type} points=${r.points ?? 'n/a'} summary=${JSON.stringify(r.summary.slice(0, 200))}${desc ? ` description=${JSON.stringify(desc)}` : ''}`;
  });
  return `You size engineering tasks for a delivery-analytics tool. For EACH task below, estimate the ideal effort in engineering days (fractional allowed, 0.25–60 — do use large values for genuinely huge scope like multi-month features) for an AVERAGE developer familiar with the codebase — the estimate must be developer-agnostic, based only on the work described. Also flag routine work: routine=true when the task is repetitive/mechanical (dependency or version upgrade, config-only change, copy tweak, straightforward port of an existing pattern).

Tasks:
${lines.join('\n')}

Answer with STRICT JSON only — no prose, no markdown fence: an array like
[{"key":"ABC-1","days":2.5,"routine":false}, ...] covering every key exactly once.`;
}

/** Lenient parse: first '[' … last ']', validated entries only. */
function parseBatch(text, expectedKeys) {
  const s = String(text || '');
  const a = s.indexOf('[');
  const b = s.lastIndexOf(']');
  if (a < 0 || b <= a) return {};
  let arr;
  try {
    arr = JSON.parse(s.slice(a, b + 1));
  } catch {
    return {};
  }
  if (!Array.isArray(arr)) return {};
  const want = new Set(expectedKeys);
  const out = {};
  for (const e of arr) {
    if (!e || typeof e !== 'object' || !want.has(e.key)) continue;
    const days = Number(e.days);
    if (!Number.isFinite(days) || days <= 0) continue;
    out[e.key] = { days: Math.min(120, Math.max(0.1, Math.round(days * 100) / 100)), routine: e.routine === true };
  }
  return out;
}

/**
 * Run the estimation pass. Mutates and returns `cache`.
 *
 * Batches are split round-robin across the configured `workers`
 * ([{provider, model}], default one claude worker) and run in PARALLEL — one
 * lane per worker, each lane serial with pacing. This lets the lead monetize
 * every provider subscription at once (15 issues to claude, 15 to codex, …)
 * and finishes proportionally faster. A batch whose worker fails is retried
 * once on the first worker; still-failed content is retried by the next scan.
 *
 * opts: {records, cache, windowMonths, sinceMs?, maxBatches, workers?,
 *        agentRun: (prompt, worker)=>Promise<string>,
 *        onProgress?: (done, total)=>void, nowMs}
 * → {estimated, failed_batches, remaining}
 */
async function runEstimation(opts) {
  const { records, cache, agentRun } = opts;
  const nowMs = opts.nowMs || Date.now();
  const targets = selectTargets(records, cache, opts.windowMonths ?? 6, nowMs, opts.sinceMs || 0);
  const maxBatches = opts.maxBatches ?? 40;
  const workers = Array.isArray(opts.workers) && opts.workers.length ? opts.workers : [{ provider: 'claude', model: '' }];
  const batches = [];
  for (let i = 0; i < targets.length && batches.length < maxBatches; i += BATCH_SIZE) {
    batches.push(targets.slice(i, i + BATCH_SIZE));
  }
  const total = batches.reduce((a, b) => a + b.length, 0);
  let estimated = 0;
  let failed = 0;
  let done = 0;

  const apply = (batch, text) => {
    const parsed = parseBatch(text, batch.map((r) => r.key));
    let n = 0;
    for (const r of batch) {
      const e = parsed[r.key];
      if (!e) continue;
      cache[r.key] = { hash: contentHash(r), days: e.days, routine: e.routine, at: nowMs };
      n++;
    }
    return n;
  };

  const lane = async (workerIdx) => {
    for (let bi = workerIdx; bi < batches.length; bi += workers.length) {
      const batch = batches[bi];
      if (bi >= workers.length) await sleep(PACE_MS);
      let text = null;
      try {
        text = await agentRun(batchPrompt(batch), workers[workerIdx]);
      } catch {
        // Worker (provider) failed — retry this batch once on the first worker.
        if (workerIdx !== 0) {
          try {
            text = await agentRun(batchPrompt(batch), workers[0]);
          } catch {
            text = null;
          }
        }
      }
      if (text === null) failed++;
      else estimated += apply(batch, text);
      done += batch.length;
      if (opts.onProgress) opts.onProgress(Math.min(done, total), total);
    }
  };

  await Promise.all(workers.map((_, i) => lane(i)));
  return { estimated, failed_batches: failed, remaining: Math.max(0, targets.length - total) };
}

module.exports = { contentHash, selectTargets, batchPrompt, parseBatch, runEstimation, BATCH_SIZE };
