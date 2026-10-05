// Team Performance — app controller: Otto handshake + theme, toolbar scope
// (account / projects / period), scan with paced-progress status, tabs,
// data-freshness strip and the person drill-down breadcrumb.
'use strict';
(function () {
  const TP = window.TP;
  const { esc, api, post, icon, store, popover, closePopover, confirmer, toast } = TP;
  const $ = (id) => document.getElementById(id);

  const TABS = [
    ['overview', 'Overview'],
    ['flow', 'Flow'],
    ['quality', 'Quality & rework'],
    ['investment', 'Investment'],
    ['people', 'People'],
    ['estimates', 'Estimates'],
    ['reports', 'Reports'],
    ['settings', 'Settings'],
  ];

  const app = (TP.app = {
    accounts: [],
    account: null,
    projects: [],
    selected: [],
    period: '12',
    periodDate: '',
    tab: 'overview',
    person: null,
    overview: null,
    overviewErr: null,
    people: {},
    scan: null,
    scanIncludedOnly: false,
    providers: ['claude', 'codex'],
    config: null,
  });

  // ---- theme ---------------------------------------------------------------
  // Otto sends the resolved scheme + its live token values; data-theme selects
  // the matching fallback set in app.css, the inline vars make it exact.
  function applyTheme(theme, scheme) {
    const root = document.documentElement;
    if (scheme === 'light' || scheme === 'dark') root.dataset.theme = scheme;
    if (theme) for (const [k, v] of Object.entries(theme)) if (v && k.startsWith('--')) root.style.setProperty(k, v);
  }

  // ---- scope helpers -------------------------------------------------------
  app.sinceMs = function () {
    const now = new Date();
    if (app.period === '') return 0;
    if (app.period === 'ytd') return Date.UTC(now.getUTCFullYear(), 0, 1);
    if (app.period === 'custom') return app.periodDate ? Date.parse(app.periodDate) || 0 : 0;
    const m = parseInt(app.period, 10);
    return Number.isFinite(m) ? Date.UTC(now.getUTCFullYear(), now.getUTCMonth() - m, now.getUTCDate()) : 0;
  };
  app.scopeQ = function () {
    const s = app.sinceMs();
    return `account=${encodeURIComponent(app.account)}&projects=${encodeURIComponent(app.selected.join(','))}${s ? `&since=${s}` : ''}`;
  };
  app.periodLabel = () => $('period').selectedOptions[0]?.textContent + (app.period === 'custom' && app.periodDate ? ' ' + app.periodDate : '');
  const acctKey = (k) => `${app.account}:${k}`;

  // ---- boot ----------------------------------------------------------------
  let booted = false;
  window.addEventListener('message', async (ev) => {
    if (ev.source !== window.parent) return; // only the hosting shell may talk to us
    const m = ev.data;
    if (!m || typeof m !== 'object') return;
    if (m.type === 'otto:theme') return applyTheme(m.theme, m.scheme);
    if (m.type !== 'otto:init') return;
    TP.state.apiBase = m.apiBase;
    TP.state.token = m.token || '';
    if (Array.isArray(m.providers) && m.providers.length) app.providers = m.providers;
    applyTheme(m.theme, m.scheme);
    if (booted) return refresh();
    booted = true;
    await boot();
  });

  // Forward modifier chords so Otto's global shortcuts work inside the frame.
  window.addEventListener(
    'keydown',
    (e) => {
      if (!(e.metaKey || e.ctrlKey || e.altKey)) return;
      window.parent.postMessage({ type: 'otto:keydown', key: e.key, code: e.code, keyCode: e.keyCode, metaKey: e.metaKey, ctrlKey: e.ctrlKey, altKey: e.altKey, shiftKey: e.shiftKey }, '*');
    },
    true,
  );

  async function boot() {
    renderTabs();
    $('view').innerHTML = TP.skeleton(6);
    try {
      const c = await api('/config');
      app.config = c;
      if (c.hours_per_day) TP.state.hpd = c.hours_per_day;
      if (c._version) $('tp-version').textContent = 'v' + c._version;
    } catch {
      /* defaults */
    }
    try {
      app.accounts = await api('/accounts');
    } catch (e) {
      return fatal(`Couldn’t reach the plugin: ${e.message}`, boot);
    }
    if (!app.accounts.length) {
      $('view').innerHTML = TP.emptyState({
        title: 'Connect Jira first',
        body: 'Team Performance reads delivery data from a Jira account configured in Otto.',
        steps: [
          { label: 'Add a Jira account in Otto → Settings → Integrations', done: false },
          { label: 'Pick projects here and run the first scan', done: false },
        ],
      });
      return;
    }
    $('account').innerHTML = app.accounts.map((a) => `<option value="${esc(a.id)}">${esc(a.label)}</option>`).join('');
    const savedAcct = store.get('account');
    app.account = app.accounts.some((a) => a.id === savedAcct) ? savedAcct : app.accounts[0].id;
    $('account').value = app.account;
    app.tab = TABS.some(([k]) => k === store.get('tab')) ? store.get('tab') : 'overview';
    await loadAccount();
  }

  function fatal(msg, retry) {
    $('view').innerHTML = `<div class="inline-error" role="alert">${icon('warn')}<span>${esc(msg)}</span><button type="button" class="compact" id="fatal-retry">Retry</button></div>`;
    $('fatal-retry').onclick = retry;
  }

  async function loadAccount() {
    const acct = app.accounts.find((a) => a.id === app.account);
    TP.state.jiraBase = (acct && (acct.base_url || acct.url)) || '';
    const p = store.get(acctKey('period'));
    if (p !== null) app.period = p;
    app.periodDate = store.get(acctKey('periodDate'), '');
    $('period').value = app.period;
    $('period-date').hidden = app.period !== 'custom';
    $('period-date').value = app.periodDate;
    app.scanIncludedOnly = store.get(acctKey('scanIncluded')) === '1';
    try {
      app.projects = await api(`/projects?account=${encodeURIComponent(app.account)}`);
    } catch (e) {
      return fatal(`Couldn’t load Jira projects: ${e.message}`, loadAccount);
    }
    let saved = [];
    try {
      saved = JSON.parse(store.get(acctKey('projects'), '[]')) || [];
    } catch {
      saved = [];
    }
    saved = saved.filter((k) => app.projects.some((p) => p.key === k));
    app.selected = saved.length ? saved : app.projects.filter((p) => p.scanned).map((p) => p.key);
    if (!app.selected.length && app.projects.length) app.selected = [app.projects[0].key];
    renderProjectLabel();
    await refreshPeople();
    await refresh();
    pollScan(true);
  }

  async function refreshPeople() {
    try {
      app.people = (await api(`/people?account=${encodeURIComponent(app.account)}`)).people || {};
    } catch {
      app.people = {};
    }
  }
  app.refreshPeople = refreshPeople;

  // ---- data ----------------------------------------------------------------
  async function refresh() {
    app.overviewErr = null;
    try {
      app.overview = await api(`/overview?${app.scopeQ()}`);
    } catch (e) {
      app.overview = null;
      if (e.status !== 404) app.overviewErr = e;
    }
    renderFreshness();
    render();
  }
  app.refresh = refresh;

  // ---- toolbar -------------------------------------------------------------
  function renderProjectLabel() {
    const n = app.selected.length;
    $('proj-label').textContent = n === 1 ? app.selected[0] : n ? `${n} projects` : 'Projects';
  }
  $('account').onchange = async (e) => {
    app.account = e.target.value;
    store.set('account', app.account);
    app.person = null;
    await loadAccount();
  };
  $('period').onchange = async (e) => {
    app.period = e.target.value;
    store.set(acctKey('period'), app.period);
    $('period-date').hidden = app.period !== 'custom';
    if (app.period !== 'custom' || app.periodDate) await refresh();
    else $('period-date').focus();
  };
  $('period-date').onchange = async (e) => {
    app.periodDate = e.target.value;
    store.set(acctKey('periodDate'), app.periodDate);
    await refresh();
  };

  // Projects: clamped multi-select popover (checkboxes + filter), Esc closes.
  $('proj-btn').onclick = () => {
    const list = app.projects
      .map(
        (p) => `<label><input type="checkbox" value="${esc(p.key)}" ${app.selected.includes(p.key) ? 'checked' : ''}>
          <span><b class="mono">${esc(p.key)}</b> ${esc(p.name)}${p.scanned ? '' : ' <span class="dim">· not scanned</span>'}</span></label>`,
      )
      .join('');
    popover(
      $('proj-btn'),
      `<label class="field"><span>Filter projects</span><input type="search" id="proj-filter" placeholder="Key or name"></label>
       <div id="proj-items" class="menu" role="group" aria-label="Projects">${list || '<p class="dim">No projects visible to this account.</p>'}</div>
       <p class="dim small">Metrics aggregate across every selected project.</p>`,
      {
        label: 'Projects in scope',
        onOpen(el) {
          el.querySelector('#proj-filter').oninput = (e) => {
            const q = e.target.value.toLowerCase();
            el.querySelectorAll('#proj-items label').forEach((l) => (l.hidden = !l.textContent.toLowerCase().includes(q)));
          };
          el.querySelectorAll('#proj-items input').forEach((cb) => {
            cb.onchange = () => {
              const set = new Set(app.selected);
              if (cb.checked) set.add(cb.value);
              else set.delete(cb.value);
              if (!set.size) {
                cb.checked = true;
                toast('Keep at least one project selected.');
                return;
              }
              app.selected = [...set];
              store.set(acctKey('projects'), JSON.stringify(app.selected));
              renderProjectLabel();
              app.person = null;
              clearTimeout(app._projT);
              app._projT = setTimeout(refresh, 350);
            };
          });
        },
      },
    );
  };

  $('more-btn').onclick = () => {
    popover(
      $('more-btn'),
      `<div class="menu" role="menu" aria-label="More actions">
        <button type="button" role="menuitem" data-m="rescan">${icon('refresh')}Full rescan…</button>
        <button type="button" role="menuitemcheckbox" aria-checked="${app.scanIncludedOnly}" data-m="included">${icon(app.scanIncludedOnly ? 'check' : 'dot')}Scan included people only</button>
        <hr>
        <button type="button" role="menuitem" data-m="reports">${icon('report')}Reports</button>
        <button type="button" role="menuitem" data-m="settings">${icon('gear')}Settings</button>
      </div>`,
      {
        role: 'presentation',
        className: 'menu',
        onOpen(el) {
          el.querySelectorAll('[data-m]').forEach((b) => {
            b.onclick = async () => {
              const m = b.dataset.m;
              closePopover();
              if (m === 'rescan') {
                const ok = await confirmer.ask({
                  title: 'Run a full rescan?',
                  message: `This rebuilds the stored history of ${app.selected.join(', ')} from scratch. It is paced to respect Jira and Bitbucket rate limits, so large projects can take a long time. Existing estimates and your corrections are kept.`,
                  confirmLabel: 'Full rescan',
                });
                if (ok) startScan(true);
              } else if (m === 'included') {
                app.scanIncludedOnly = !app.scanIncludedOnly;
                store.set(acctKey('scanIncluded'), app.scanIncludedOnly ? '1' : '0');
                toast(app.scanIncludedOnly ? 'Scans fetch included people only.' : 'Scans fetch everyone.');
              } else goTab(m);
            };
          });
        },
      },
    );
  };

  // ---- scan ----------------------------------------------------------------
  async function startScan(full) {
    const body = { account: app.account, projects: app.selected, full };
    if (app.scanIncludedOnly) body.assignees = Object.entries(app.people).filter(([, p]) => p.included !== false).map(([id]) => id);
    try {
      await post('/scan', body);
      pollScan(false);
    } catch (e) {
      toast(e.status === 409 ? 'A scan is already running.' : `Scan failed to start: ${e.message}`, 'danger');
    }
  }
  $('scan').onclick = () => startScan(false);
  app.startScan = startScan;

  const STEP_GROUPS = [
    ['jira', 'Jira', /^(starting|fields|search|changelogs|analyze|persist|estimate)/i],
    ['git', 'git', /git/i],
    ['prs', 'Pull requests', /\b(pr|prs|pull|bitbucket|review)/i],
  ];
  function stepGroup(s) {
    if (s.phase && STEP_GROUPS.some(([k]) => k === s.phase)) return s.phase;
    for (const [k, , re] of STEP_GROUPS.slice().reverse()) if (re.test(s.step || '')) return k;
    return 'jira';
  }
  let pollT = null;
  let lastStatus = null;
  function paintScan(s) {
    const box = $('scan-status');
    if (!s || s.state !== 'running') {
      box.innerHTML = s && s.state === 'error' ? `${icon('warn')}<span>Scan failed: ${esc(s.error || 'unknown error')}</span>` : '';
      return;
    }
    const cur = stepGroup(s);
    const idx = STEP_GROUPS.findIndex(([k]) => k === cur);
    const steps = STEP_GROUPS.map(([k, label], i) => `<li data-state="${i < idx ? 'done' : i === idx ? 'active' : 'todo'}">${i < idx ? icon('check') : ''}${esc(label)}${i === idx ? '<span class="sr-only"> (in progress)</span>' : ''}</li>`).join('');
    const proj = s.project_n > 1 ? ` · project ${s.project_i + 1}/${s.project_n} ${esc(s.project || '')}` : s.project ? ` · ${esc(s.project)}` : '';
    const count = s.total != null ? ` ${s.fetched ?? 0}/${s.total}` : s.fetched ? ` ${s.fetched}` : '';
    const now = Date.now();
    const eta = s.next_call_at && s.next_call_at > now ? `next call in ${TP.fmtSecs(s.next_call_at - now)}` : s.pace_ms ? `paced ${Math.round(s.pace_ms / 100) / 10}s/call` : '';
    const backoff = s.backoff_ms ? `rate-limited — backing off ${TP.fmtSecs(s.backoff_ms)}` : s.retries ? `${s.retries} retries` : '';
    box.innerHTML = `<ol class="steps" aria-label="Scan steps">${steps}</ol>
      <span>${esc(s.step || 'working')}${count}${proj}</span>
      ${eta ? `<span class="dim">${esc(eta)}</span>` : ''}
      ${backoff ? TP.badge('warning', backoff) : ''}
      ${s.estimate_remaining ? `<span class="dim">${s.estimate_remaining} estimates queued</span>` : ''}`;
  }
  function pollScan(passive) {
    clearInterval(pollT);
    if (!passive) $('scan').disabled = true;
    const tick = async () => {
      let s;
      try {
        s = await api(`/scan/status?account=${encodeURIComponent(app.account)}`);
      } catch {
        return; // transient — keep polling
      }
      app.scan = s;
      $('scan').disabled = s.state === 'running';
      $('scan').textContent = s.state === 'running' ? 'Scanning…' : 'Scan';
      paintScan(s);
      const wasRunning = lastStatus === 'running';
      lastStatus = s.state;
      if (s.state !== 'running') {
        clearInterval(pollT);
        renderFreshness();
        if (s.state === 'done' && (wasRunning || !passive)) {
          toast('Scan complete — metrics refreshed.', 'success');
          await refreshPeople();
          await refresh();
        }
      }
    };
    tick();
    pollT = setInterval(tick, 1000);
  }

  // ---- freshness -----------------------------------------------------------
  function renderFreshness() {
    const o = app.overview;
    const f = (o && o.freshness) || {};
    const item = (label, ms) => {
      const stale = ms && Date.now() - ms > 3 * 86400000;
      return `<span class="${stale ? 'stale' : ''}" title="${ms ? new Date(ms).toLocaleString() : 'not available yet'}">${esc(label)}: ${ms ? TP.fmtAgo(ms) : 'not available yet'}${stale ? ' (stale)' : ''}</span>`;
    };
    $('freshness').innerHTML = [
      item('Jira', f.jira_at || (o && o.scanned_at) || (app.scan && app.scan.last_scan)),
      item('git', f.git_at),
      item('Pull requests', f.prs_at),
      `<span>Period: ${esc(app.periodLabel())}</span>`,
      o ? `<span>${o.completed ?? 0} completed · ${o.open ?? 0} open tickets</span>` : '',
    ].join('');
  }

  // ---- tabs + routing ------------------------------------------------------
  function renderTabs() {
    $('tabs').innerHTML = TABS.map(
      ([k, label]) => `<button type="button" role="tab" id="tab-${k}" aria-controls="view" aria-selected="${app.tab === k && !app.person}" tabindex="${app.tab === k ? 0 : -1}" data-tab="${k}">${esc(label)}</button>`,
    ).join('');
    $('view').setAttribute('aria-labelledby', 'tab-' + app.tab);
    $('tabs').querySelectorAll('[role=tab]').forEach((b) => (b.onclick = () => goTab(b.dataset.tab)));
  }
  $('tabs').addEventListener('keydown', (e) => {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return;
    const i = TABS.findIndex(([k]) => k === app.tab);
    const rtl = getComputedStyle(document.documentElement).direction === 'rtl';
    const fwd = (e.key === 'ArrowRight') !== rtl;
    const n = e.key === 'Home' ? 0 : e.key === 'End' ? TABS.length - 1 : (i + (fwd ? 1 : -1) + TABS.length) % TABS.length;
    goTab(TABS[n][0]);
    $('tab-' + TABS[n][0]).focus();
    e.preventDefault();
  });
  function goTab(k) {
    app.tab = k;
    app.person = null;
    store.set('tab', k);
    render();
  }
  app.goTab = goTab;

  app.openPerson = function (id) {
    app.personReturn = document.activeElement;
    app.person = id;
    render();
    $('crumb-back') && $('crumb-back').focus();
  };
  function closePerson() {
    app.person = null;
    render();
    if (app.personReturn && document.contains(app.personReturn)) app.personReturn.focus();
  }
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || !app.person) return;
    if (document.querySelector('.modal-backdrop, .popover')) return;
    closePerson();
  });

  function renderCrumbs() {
    if (!app.person) return ($('crumbs').innerHTML = '');
    const a = app.overview && (app.overview.assignees || []).find((x) => x.assignee_id === app.person);
    const name = (a && a.assignee_name) || (app.people[app.person] && app.people[app.person].name) || app.person;
    $('crumbs').innerHTML = `<nav class="breadcrumb" aria-label="Breadcrumb">
      <button type="button" class="compact" id="crumb-back" aria-label="Back to People" title="Back (Esc)">${icon('back')}Back</button>
      <ol><li><button type="button" class="link" id="crumb-people">People</button></li><li aria-current="page">${esc(name)}</li></ol></nav>`;
    $('crumb-back').onclick = closePerson;
    $('crumb-people').onclick = closePerson;
  }

  function firstRunSteps() {
    const anyScanned = app.projects.some((p) => p.scanned);
    return [
      { label: 'Jira account connected', done: !!app.account },
      { label: 'Projects selected', done: app.selected.length > 0 },
      { label: 'First scan finished', done: anyScanned },
      { label: 'People reviewed (roles, aliases, time off) in Settings', done: Object.keys(app.people).length > 0 },
    ];
  }

  function render() {
    renderTabs();
    renderCrumbs();
    const host = $('view');
    host.innerHTML = '';
    const ctx = { app, o: app.overview, host };
    if (app.person) return TP.views.person.render(host, ctx, app.person);
    const needsData = !['reports', 'settings'].includes(app.tab);
    if (needsData && app.overviewErr) {
      host.innerHTML = `<div class="inline-error" role="alert">${icon('warn')}<span>Couldn’t load metrics: ${esc(app.overviewErr.message)}</span><button type="button" class="compact" id="ov-retry">Retry</button></div>`;
      $('ov-retry').onclick = refresh;
      return;
    }
    if (needsData && !app.overview) {
      host.innerHTML = TP.emptyState({
        title: 'No data for this scope yet',
        body: 'Run a scan to build the delivery history: Jira tickets, git commits and pull requests. Later scans are incremental and paced.',
        steps: firstRunSteps(),
        actionLabel: app.scan && app.scan.state === 'running' ? '' : 'Scan now',
        actionId: 'empty-scan',
      });
      const b = $('empty-scan');
      if (b) b.onclick = () => startScan(false);
      return;
    }
    const v = TP.views[app.tab];
    if (v) v.render(host, ctx);
  }
  app.render = render;
})();
