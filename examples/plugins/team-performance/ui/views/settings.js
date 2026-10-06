// Settings: deployments + time (tag patterns, timezone, QA rule), people and
// time off (drives every capacity number), scan, AI estimation, reports,
// feature-scan repos and the status → phase map. One Save at the bottom.
'use strict';
(function () {
  const TP = window.TP;
  const { esc, api, put, post, section, toast, icon, confirmer } = TP;

  const timeZones = () => {
    try {
      return Intl.supportedValuesOf('timeZone');
    } catch {
      return ['UTC'];
    }
  };
  const field = (label, control, hint) => `<label class="field"><span>${esc(label)}</span>${control}${hint ? `<span class="small">${esc(hint)}</span>` : ''}</label>`;
  const num = (id, v, min, max, step = 1) => `<input type="number" id="${id}" min="${min}" max="${max}" step="${step}" value="${esc(v ?? '')}">`;

  function timeOffChips(list, pid) {
    return (list || [])
      .map((t, i) => `<span class="badge">${esc(t.from === t.to ? t.from : `${t.from} → ${t.to}`)}<button type="button" class="link" data-off-del="${esc(pid)}:${i}" aria-label="Remove time off ${esc(t.from)}${t.to !== t.from ? ' to ' + esc(t.to) : ''}" title="Remove">${icon('x')}</button></span>`)
      .join('');
  }

  TP.views.settings = {
    render(host, { app }) {
      const first = app.selected[0] || '';
      let config, peopleResp, repos, statuses;
      let people = {};
      section(host, {
        title: 'Settings',
        sub: 'Status-map, workweek, timezone and people edits recompute instantly from stored data — no rescan needed.',
        skeletonLines: 10,
        load: async () => {
          [config, peopleResp, repos, statuses] = await Promise.all([
            api('/config'),
            api(`/people?account=${encodeURIComponent(app.account)}`),
            api('/repos').catch(() => []),
            first ? api(`/statuses?account=${encodeURIComponent(app.account)}&project=${encodeURIComponent(first)}`).catch(() => []) : Promise.resolve([]),
          ]);
          people = JSON.parse(JSON.stringify(peopleResp.people || {}));
          const patterns = config.deploy_tag_patterns || (config.deploy_tag_pattern ? [config.deploy_tag_pattern] : ['deployed', 'hf', 'hotfix']);
          const tz = config.timezone || Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
          const roleOpts = (sel) => ['', ...(config.roles || [])].map((r) => `<option value="${esc(r)}" ${r === sel ? 'selected' : ''}>${r ? esc(r) : 'No role'}</option>`).join('');
          const sorted = Object.entries(people).sort((a, b) => a[1].name.localeCompare(b[1].name));
          const mergeOpts = (self, sel) => ['<option value="">Separate person</option>', ...sorted.filter(([o]) => o !== self).map(([o, p]) => `<option value="${esc(o)}" ${o === sel ? 'selected' : ''}>Same as ${esc(p.name)}</option>`)].join('');
          const peopleRows = sorted.map(([id, p]) => ({
            attrs: `data-pid="${esc(id)}"${p.merged_into ? ' class="excluded"' : ''}`,
            cells: [
              `${esc(p.name)}`,
              `<label class="sr-only" for="inc-${esc(id)}">Include ${esc(p.name)}</label><input type="checkbox" id="inc-${esc(id)}" class="pp-inc" ${p.included !== false ? 'checked' : ''}>`,
              `<label class="sr-only" for="role-${esc(id)}">Role of ${esc(p.name)}</label><select id="role-${esc(id)}" class="pp-role">${roleOpts(p.role || '')}</select>`,
              `<label class="sr-only" for="al-${esc(id)}">Git aliases of ${esc(p.name)}</label><input type="text" id="al-${esc(id)}" class="pp-alias" placeholder="name or email, comma separated" value="${esc((p.aliases || []).join(', '))}">`,
              `<div class="timeoff-list"><span class="chips" data-chips="${esc(id)}">${timeOffChips(p.time_off, id)}</span>
                <label class="sr-only" for="of-${esc(id)}">Time off from</label><input type="date" id="of-${esc(id)}" class="off-from">
                <label class="sr-only" for="ot-${esc(id)}">Time off to</label><input type="date" id="ot-${esc(id)}" class="off-to">
                <button type="button" class="compact" data-off-add="${esc(id)}" aria-label="Add time off for ${esc(p.name)}" title="Add time off">${icon('plus')}Add</button></div>`,
              `<label class="sr-only" for="mg-${esc(id)}">Merge ${esc(p.name)}</label><select id="mg-${esc(id)}" class="pp-merge">${mergeOpts(id, p.merged_into || '')}</select>`,
            ],
          }));
          const unmatched = (peopleResp.unmatched_authors || []).slice(0, 15).map((u) => TP.badge('warning', `${u.who} ×${u.commits}`)).join(' ');
          const featSet = new Set(config.feature_repos || []);
          const map = (config.status_map || {})[first] || {};
          const workerRow = (w) => `<div class="form-grid wk-row">
              <label class="field"><span>Provider</span><select class="wk-provider">${app.providers.map((p) => `<option ${w.provider === p ? 'selected' : ''}>${esc(p)}</option>`).join('')}</select></label>
              <label class="field"><span>Model (optional)</span><input type="text" class="wk-model" value="${esc(w.model || '')}"></label>
              <span><button type="button" class="compact wk-del" aria-label="Remove worker" title="Remove worker">${icon('x')}Remove</button></span></div>`;
          const tzs = timeZones();
          return `
            <fieldset><legend>Deployments and time</legend><div class="form-grid">
              ${field('Deploy tag contains (comma separated)', `<input type="text" id="cfg-tags" value="${esc(patterns.join(', '))}">`, 'Case-insensitive; a tag matching any word is a deployment (hotfix tags too).')}
              ${field('Timezone', `<select id="cfg-tz">${tzs.map((z) => `<option ${z === tz ? 'selected' : ''}>${esc(z)}</option>`).join('')}</select>`, 'Day boundaries for capacity and phases.')}
              ${field('Workweek', `<select id="cfg-week"><option value="1,2,3,4,5" ${String(config.workweek) === '1,2,3,4,5' ? 'selected' : ''}>Monday–Friday</option><option value="0,1,2,3,4" ${String(config.workweek) === '0,1,2,3,4' ? 'selected' : ''}>Sunday–Thursday</option></select>`)}
              ${field('Working hours per day', num('cfg-hpd', config.hours_per_day ?? 8, 1, 24))}
              ${field('QA counts as dev after commits on ≥ N days', num('cfg-qa', config.qa_work_min_commit_days ?? 2, 1, 30), 'Otherwise QA time is waiting, not dev.')}
            </div></fieldset>

            <fieldset><legend>People and time off</legend>
              <p class="dim small">Unchecked people leave every chart. Time off removes days from that person’s capacity everywhere. Aliases match git commit authors.</p>
              ${sorted.length ? TP.table({ caption: `${sorted.length} people`, cols: [{ label: 'Name' }, { label: 'Included' }, { label: 'Role' }, { label: 'Git aliases' }, { label: 'Time off' }, { label: 'Merge' }], rows: peopleRows }) : '<p class="dim">No people yet — scan a project or load them from Jira.</p>'}
              ${unmatched ? `<h3>Unmatched git authors</h3><div class="chips">${unmatched}</div>` : ''}
              <div class="form-grid spaced">
                ${field('Roles (comma separated)', `<input type="text" id="cfg-roles" value="${esc((config.roles || []).join(', '))}">`)}
                <span><button type="button" class="compact" id="pp-seed" ${first ? '' : 'disabled'}>Load people from Jira${first ? ` (${esc(first)})` : ''}</button></span>
              </div>
            </fieldset>

            <fieldset><legend>Scan</legend><div class="form-grid">
              ${field('Issue types (blank = all)', `<input type="text" id="cfg-types" value="${esc((config.issue_types || []).join(', '))}">`)}
              ${field('Max issues (0 = all)', num('cfg-max', config.max_issues, 0, 1000000))}
              ${field('Git depth (0 = full)', num('cfg-depth', config.git_depth, 0, 1000000))}
              ${field('Jira pacing (ms per call)', num('cfg-pace', config.pace_ms, 0, 5000), 'Doubles automatically when Jira throttles.')}
              ${field('Stale after (days)', num('cfg-stale', config.stale_days, 5, 365))}
              ${field('Fix window (days)', num('cfg-fixwin', config.fix_window_days ?? 30, 1, 365))}
              ${field('Fold fixes from N commits', num('cfg-fixmin', config.fix_include_min_commits ?? 3, 1, 100))}
              ${field('Auto-scan every N minutes (0 = off)', num('cfg-autoscan', config.auto_scan_minutes ?? 15, 0, 1440))}
              <label class="check"><input type="checkbox" id="cfg-fetch" ${config.git_fetch ? 'checked' : ''}> git fetch before scanning</label>
            </div></fieldset>

            <fieldset><legend>AI estimation</legend><div class="form-grid">
              <label class="check"><input type="checkbox" id="cfg-est-on" ${config.estimate_enabled ? 'checked' : ''}> Estimation enabled</label>
              ${field('Estimate tickets done since', `<input type="date" id="cfg-est-since" value="${esc(config.estimate_since || '')}">`)}
              ${field('Window (months, 0 = all)', num('cfg-est-window', config.estimate_window_months, 0, 60))}
              ${field('Max batches per scan', num('cfg-est-batches', config.estimate_max_batches, 1, 200))}
              ${field('Evidence window (months)', num('cfg-ev-months', config.evidence_months ?? 18, 1, 120))}
              ${field('Mode', `<select id="cfg-est-mode"><option value="split" ${(config.estimate_mode || 'split') === 'split' ? 'selected' : ''}>Split — share batches (faster)</option><option value="consensus" ${config.estimate_mode === 'consensus' ? 'selected' : ''}>Consensus — median of every worker</option></select>`)}
            </div>
            <h3>Workers</h3><div id="workers">${(config.estimate_workers || []).map(workerRow).join('')}</div>
            <p><button type="button" class="compact" id="wk-add">${icon('plus')}Add worker</button></p>
            <div id="sum-wrap" ${config.estimate_mode === 'consensus' ? '' : 'hidden'} class="form-grid">
              ${field('Summarizer provider', `<select id="cfg-sum-provider">${app.providers.map((p) => `<option ${(config.estimate_summarizer || {}).provider === p ? 'selected' : ''}>${esc(p)}</option>`).join('')}</select>`)}
              ${field('Summarizer model', `<input type="text" id="cfg-sum-model" value="${esc((config.estimate_summarizer || {}).model || '')}">`)}
            </div>
            ${field('Calibration rubric (one rule per line; blank = defaults)', `<textarea id="cfg-rubric" rows="5">${esc((config.estimate_rubric || []).join('\n'))}</textarea>`)}
            ${field('Extra estimator instructions', `<textarea id="cfg-est-instr" rows="3">${esc(config.estimate_instructions || '')}</textarea>`)}
            </fieldset>

            <fieldset><legend>Reports</legend>
              ${field('Report instructions (blank = defaults)', `<textarea id="cfg-report-instr" rows="4">${esc(config.report_instructions || '')}</textarea>`)}
            </fieldset>

            <fieldset><legend>Feature-scan repos</legend>
              <p class="dim small">Git-only features for work without Jira stories.</p>
              <div class="form-grid">${(repos || []).map((r) => `<label class="check"><input type="checkbox" class="fr-cb" value="${esc(r.name)}" ${featSet.has(r.name) ? 'checked' : ''}> ${esc(r.name)}</label>`).join('') || '<p class="dim">No repos registered in Otto.</p>'}</div>
            </fieldset>

            <fieldset><legend>Status → phase${first ? ` (${esc(first)})` : ''}</legend>
              ${(statuses || []).length ? `<div class="form-grid">${statuses.map((st, i) => field(`${st.name}${st.category ? ` · ${st.category}` : ''}`, `<select class="st-map" id="st-${i}" data-status="${esc(st.name)}">${(config._phase_values || ['design', 'implementation', 'waiting', 'excluded']).map((p) => `<option ${(map[st.name] || st.mapped) === p ? 'selected' : ''}>${p}</option>`).join('')}</select>`)).join('')}</div>` : '<p class="dim">Couldn’t load the project’s statuses.</p>'}
            </fieldset>
            <div class="form-actions"><button type="button" class="primary" id="save-config">Save settings</button></div>`;
        },
        after(body, rerun) {
          // Deep link from another view (e.g. DORA "not available" → deploy tags).
          if (app.settingsFocus) {
            const target = body.querySelector('#' + app.settingsFocus);
            app.settingsFocus = null;
            if (target) {
              if (target.scrollIntoView) target.scrollIntoView({ block: 'center' });
              target.focus();
            }
          }
          const q = (s) => body.querySelector(s);
          const paintChips = (pid) => {
            const box = body.querySelector(`[data-chips="${CSS.escape(pid)}"]`);
            if (box) box.innerHTML = timeOffChips(people[pid].time_off, pid);
          };
          if (!body._offWired) body.addEventListener('click', (e) => {
            const add = e.target.closest('[data-off-add]');
            const del = e.target.closest('[data-off-del]');
            if (add) {
              const pid = add.dataset.offAdd;
              const row = add.closest('tr');
              const from = row.querySelector('.off-from').value;
              const to = row.querySelector('.off-to').value || from;
              if (!from) return toast('Pick a start date.', 'danger');
              if (to < from) return toast('The end date is before the start date.', 'danger');
              (people[pid].time_off ||= []).push({ from, to });
              people[pid].time_off.sort((a, b) => a.from.localeCompare(b.from));
              row.querySelector('.off-from').value = '';
              row.querySelector('.off-to').value = '';
              paintChips(pid);
            } else if (del) {
              const [pid, i] = del.dataset.offDel.split(':');
              people[pid].time_off.splice(+i, 1);
              paintChips(pid);
            }
          });
          body._offWired = true;
          q('#cfg-est-mode').onchange = (e) => (q('#sum-wrap').hidden = e.target.value !== 'consensus');
          const wireDel = () => body.querySelectorAll('.wk-del').forEach((b) => (b.onclick = () => b.closest('.wk-row').remove()));
          wireDel();
          q('#wk-add').onclick = () => {
            q('#workers').insertAdjacentHTML('beforeend', `<div class="form-grid wk-row"><label class="field"><span>Provider</span><select class="wk-provider">${app.providers.map((p) => `<option>${esc(p)}</option>`).join('')}</select></label><label class="field"><span>Model (optional)</span><input type="text" class="wk-model"></label><span><button type="button" class="compact wk-del" aria-label="Remove worker" title="Remove worker">${icon('x')}Remove</button></span></div>`);
            wireDel();
          };
          const seed = q('#pp-seed');
          if (seed)
            seed.onclick = async () => {
              try {
                const r = await post('/people/seed', { account: app.account, project: first });
                toast(`${r.added} people added from Jira.`, 'success');
                await app.refreshPeople();
                rerun();
              } catch (e) {
                toast(`Couldn’t load people: ${e.message}`, 'danger');
              }
            };
          q('#save-config').onclick = async () => {
            const pp = {};
            body.querySelectorAll('tr[data-pid]').forEach((tr) => {
              const id = tr.dataset.pid;
              pp[id] = {
                included: tr.querySelector('.pp-inc').checked,
                role: tr.querySelector('.pp-role').value,
                aliases: tr.querySelector('.pp-alias').value.split(',').map((x) => x.trim()).filter(Boolean),
                merged_into: tr.querySelector('.pp-merge').value || null,
                time_off: people[id].time_off || [],
              };
            });
            const merges = Object.entries(pp).filter(([id, p]) => p.merged_into && p.merged_into !== (peopleResp.people[id] || {}).merged_into);
            if (merges.length) {
              const ok = await confirmer.ask({
                title: 'Merge people?',
                message: `${merges.length} account${merges.length > 1 ? 's' : ''} will be folded into another person: their history is credited to that person everywhere. You can undo it later by choosing “Separate person”.`,
                confirmLabel: 'Merge and save',
              });
              if (!ok) return;
            }
            const status_map = { ...(config.status_map || {}) };
            const pm = {};
            body.querySelectorAll('.st-map').forEach((s) => (pm[s.dataset.status] = s.value));
            if (first && Object.keys(pm).length) status_map[first] = pm;
            const patterns = q('#cfg-tags').value.split(',').map((x) => x.trim()).filter(Boolean);
            const workers = [...body.querySelectorAll('.wk-row')].map((r) => ({ provider: r.querySelector('.wk-provider').value, model: r.querySelector('.wk-model').value.trim() }));
            const int = (id) => parseInt(q('#' + id).value, 10);
            const cfg = {
              deploy_tag_patterns: patterns,
              deploy_tag_pattern: patterns[0] || 'deployed',
              timezone: q('#cfg-tz').value,
              qa_work_min_commit_days: int('cfg-qa'),
              workweek: q('#cfg-week').value.split(',').map(Number),
              hours_per_day: int('cfg-hpd'),
              issue_types: q('#cfg-types').value.split(',').map((x) => x.trim()).filter(Boolean),
              max_issues: int('cfg-max'),
              git_depth: int('cfg-depth'),
              pace_ms: int('cfg-pace'),
              stale_days: int('cfg-stale'),
              fix_window_days: int('cfg-fixwin'),
              fix_include_min_commits: int('cfg-fixmin'),
              auto_scan_minutes: int('cfg-autoscan'),
              git_fetch: q('#cfg-fetch').checked,
              estimate_enabled: q('#cfg-est-on').checked,
              estimate_since: q('#cfg-est-since').value || '',
              estimate_window_months: int('cfg-est-window'),
              estimate_max_batches: int('cfg-est-batches'),
              evidence_months: int('cfg-ev-months'),
              estimate_mode: q('#cfg-est-mode').value,
              estimate_workers: workers.length ? workers : undefined,
              estimate_summarizer: { provider: q('#cfg-sum-provider').value, model: q('#cfg-sum-model').value.trim() },
              estimate_rubric: q('#cfg-rubric').value.split('\n').map((x) => x.trim()).filter(Boolean),
              estimate_instructions: q('#cfg-est-instr').value,
              report_instructions: q('#cfg-report-instr').value,
              feature_repos: [...body.querySelectorAll('.fr-cb:checked')].map((c) => c.value),
              roles: q('#cfg-roles').value.split(',').map((x) => x.trim()).filter(Boolean),
              status_map,
            };
            const btn = q('#save-config');
            btn.disabled = true;
            try {
              if (Object.keys(pp).length) await put('/people', { account: app.account, people: pp });
              const saved = await put('/config', cfg);
              app.config = saved;
              if (saved.hours_per_day) TP.state.hpd = saved.hours_per_day;
              toast('Settings saved — metrics recomputed.', 'success');
              await app.refreshPeople();
              await app.refresh();
            } catch (e) {
              toast(`Couldn’t save settings: ${e.message}`, 'danger');
            } finally {
              btn.disabled = false;
            }
          };
        },
      });
    },
  };
})();
