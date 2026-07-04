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

/** Coarse fingerprint of the git change — so an estimate refines as code lands. */
function changeFingerprint(record) {
  const c = record.git_change;
  if (!c) return '';
  const bucket = (n) => (n <= 0 ? 0 : Math.round(Math.log2(n + 1)));
  return `f${bucket(c.files)}i${bucket(c.insertions)}d${bucket(c.deletions)}c${bucket(c.commits)}`;
}

/** FNV-1a over the estimation-relevant content of a record (incl. change size). */
function contentHash(record) {
  const s = `${record.summary}|${record.description_snippet || ''}|${record.type}|${record.points ?? ''}|${changeFingerprint(record)}`;
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
// Prompt generation version. v4: diff size is a SIGNAL, not a ceiling —
// investigation/root-cause effort can make a tiny change a multi-day task.
// Any cached entry from an older prompt is re-estimated once.
const PROMPT_V = 4;

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
    // Re-estimate when the content/change hash changed OR the prompt version
    // advanced (older estimates predate the diff-evidence rubric).
    if (hit && hit.hash === contentHash(r) && hit.v === PROMPT_V) continue;
    out.push(r);
  }
  // Most-recent first — the tasks the lead is actually looking at.
  out.sort((a, b) => (b.eff_done_at ?? b.done_at ?? b.updated ?? 0) - (a.eff_done_at ?? a.done_at ?? a.updated ?? 0));
  return out;
}

/** Compact one-line rendering of a task's git diff evidence for the prompt. */
function evidenceLine(c) {
  if (!c || !c.commits) return '';
  const repos = (c.repos || [])
    .slice(0, 6)
    .map((r) => `${r.name}(${r.files}f +${r.ins}/-${r.del})`)
    .join(', ');
  const subs = (c.subjects || []).slice(0, 5).map((s) => JSON.stringify(s)).join(', ');
  return ` change={commits:${c.commits}, files:${c.files}, +${c.insertions}/-${c.deletions}; repos:[${repos}]; subjects:[${subs}]}`;
}

/** The default, editable calibration rubric (casino-platform scoping norms). */
const DEFAULT_RUBRIC = [
  'Trivial change (log-noise/config/copy tweak, or a version bump — even one propagated across many services): 0.25–0.5d. A bump repeated across N repos is still ONE ~0.5d task, not N.',
  'Small change to an existing service (add a filter/field/endpoint, a few files): 1–3d.',
  'Reverse integration (the game provider implements our API, following an existing pattern): ~1d.',
  'Full new game-provider integration (we implement the provider API from scratch): ~15d (3 weeks).',
  'Full integration WITH significant caveats or infra that must be rewritten in many places (e.g. Mega, Digitain): 30–40d (1.5–2 months).',
  'Judge the CORE work, not mechanical fan-out: if one repo holds an 18-file rewrite and 15 others get a 1-line bump, size the 18-file core plus a little glue — not the sum.',
  'A tiny diff is NOT automatically trivial: a one-line fix to a subtle bug/race/prod issue can be days of investigation. Size the understanding+debugging, not just the lines changed. Only mechanical small changes (bump/config/rename) are cheap.',
];

function batchPrompt(records, rubric, corrections) {
  const lines = records.map((r) => {
    const desc = (r.description_snippet || '').slice(0, 1200);
    const epic = r.epic_hint ? ` part_of=${JSON.stringify(String(r.epic_hint).slice(0, 90))}` : '';
    return `- key=${r.key} type=${r.type} points=${r.points ?? 'n/a'}${epic} summary=${JSON.stringify(r.summary.slice(0, 200))}${desc ? ` description=${JSON.stringify(desc)}` : ''}${evidenceLine(r.git_change)}`;
  });
  const rules = (Array.isArray(rubric) && rubric.length ? rubric : DEFAULT_RUBRIC).map((r) => `  • ${r}`).join('\n');
  // The lead's own past corrections — the strongest signal for calibration.
  const learned = (corrections || []).length
    ? `\nThe team lead has CORRECTED past estimates — learn from these (match this calibration):\n${corrections
        .slice(0, 15)
        .map((c) => `  • ${JSON.stringify((c.summary || c.key).slice(0, 80))} → ${c.corrected}d${c.reason ? ` (${JSON.stringify(c.reason.slice(0, 160))})` : ''}`)
        .join('\n')}\n`
    : '';
  return `You size engineering tasks for a delivery-analytics tool. For EACH task, estimate the IDEAL effort in engineering days (fractional; 0.25–60) for an AVERAGE developer familiar with this codebase. The estimate is developer-AGNOSTIC — ignore who did it, any story points, and any prior estimate; judge the work itself.

Use the ACTUAL CODE CHANGE (the \`change=\` evidence: files touched, lines added/removed, which repos, commit subjects) as a strong signal, weighed against the ticket prose (tickets over- and under-describe). Do NOT inflate — most tasks are small; a large multi-file rewrite is large even if the ticket is terse.

BUT diff size is a SIGNAL, NOT a ceiling: a tiny change can be a large task when the effort was in the INVESTIGATION — root-causing a subtle production bug, a race condition, finding the one config value that fixes it, understanding a gnarly system before the one-line fix. A 1-line change is NOT automatically a 1-minute task. Read the type (Bug/investigation), the description, and the commit subjects (e.g. "fix race", "root cause", "investigate", "reproduce") to judge whether a small change was hard-won, and size the UNDERSTANDING + fix, not just the diff. Only genuinely mechanical small changes (version bump, config toggle, rename, copy tweak) are truly cheap.

Calibration rubric (follow it):
${rules}
${learned}
Also flag routine=true for repetitive/mechanical work (version/dependency bump, config-only, copy tweak, straightforward port of an existing pattern).

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
  const { records, cache, agentRun, rubric, corrections } = opts;
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
      cache[r.key] = { hash: contentHash(r), days: e.days, routine: e.routine, v: PROMPT_V, at: nowMs };
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
        text = await agentRun(batchPrompt(batch, rubric, corrections), workers[workerIdx]);
      } catch {
        // Worker (provider) failed — retry this batch once on the first worker.
        if (workerIdx !== 0) {
          try {
            text = await agentRun(batchPrompt(batch, rubric, corrections), workers[0]);
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

module.exports = { contentHash, selectTargets, batchPrompt, parseBatch, runEstimation, evidenceLine, DEFAULT_RUBRIC, BATCH_SIZE };
