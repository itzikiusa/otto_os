import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  ACTUATOR_PRESETS,
  buildK8sLogsPacket,
  defaultPort,
  fillTemplate,
  pathProblem,
  podHttpToApiDraft,
  prettyBody,
  templateVars,
  workloadKindFor,
} from '../src/modules/kubernetes/podHttp.ts';

test('templates: variables, fill, RESET becomes JSON null', () => {
  const set = ACTUATOR_PRESETS.find((p) => p.id === 'set-level')!;
  assert.deepEqual(templateVars(set.path, set.body), ['logger', 'level']);
  assert.equal(fillTemplate(set.path, { logger: 'com.acme' }), '/actuator/loggers/com.acme');
  assert.equal(fillTemplate(set.body!, { level: 'DEBUG' }), '{"configuredLevel":"DEBUG"}');
  assert.equal(fillTemplate(set.body!, { level: 'RESET' }), '{"configuredLevel":null}');
  assert.equal(fillTemplate('/x/{{missing}}', {}), '/x/{{missing}}');
});

test('path rule mirrors the daemon', () => {
  assert.equal(pathProblem('/actuator/health'), null);
  assert.ok(pathProblem('actuator'));
  assert.ok(pathProblem('/a/../b'));
  assert.ok(pathProblem('/http://x'));
  assert.ok(pathProblem('/a b'));
  assert.ok(pathProblem('/a#frag'));
  assert.ok(pathProblem('/a%2Fb'));
  assert.ok(pathProblem('/actuator/loggers/{{logger}}'));
});

test('workload kinds and default port', () => {
  assert.equal(workloadKindFor('deployments'), 'deployment');
  assert.equal(workloadKindFor('services'), null);
  assert.equal(defaultPort({ spec: { containers: [{ ports: [{ name: 'grpc', containerPort: 9090 }, { name: 'management', containerPort: 8081 }] }] } }), 8081);
  assert.equal(defaultPort({ spec: { template: { spec: { containers: [{ ports: [{ name: 'http', containerPort: 8000 }] }] } } } }), 8000);
  assert.equal(defaultPort(null), 8080);
  assert.equal(prettyBody('{"a":1}'), '{\n  "a": 1\n}');
  assert.equal(prettyBody('plain'), 'plain');
});

test('podHttpToApiDraft fills templates, parses headers/body, and adds port-forward docs', () => {
  const draft = podHttpToApiDraft({
    clusterName: 'prod-eu',
    ns: 'payments',
    pod: 'billing-0',
    workload: { kind: 'statefulset', name: 'billing' },
    target: 'pod',
    method: 'POST',
    port: 8081,
    path: '/actuator/loggers/{{logger}}',
    headersText: 'Content-Type: application/json\nX-Trace: abc\ninvalid-line',
    body: '{"configuredLevel":"{{level}}"}',
    vars: { logger: 'com.acme.billing', level: 'DEBUG' },
  });
  assert.equal(draft.method, 'POST');
  assert.equal(draft.url, 'http://127.0.0.1:8081/actuator/loggers/com.acme.billing');
  assert.equal(draft.body_mode, 'json');
  assert.equal(draft.body, '{"configuredLevel":"DEBUG"}');
  assert.deepEqual(draft.headers, [
    { key: 'Content-Type', value: 'application/json', enabled: true },
    { key: 'X-Trace', value: 'abc', enabled: true },
  ]);
  assert.ok(draft.docs?.includes('kubectl -n payments port-forward pod/billing-0 8081:8081'));

  const getDraft = podHttpToApiDraft({
    clusterName: 'dev',
    ns: 'default',
    workload: { kind: 'deployment', name: 'web' },
    target: 'workload',
    method: 'GET',
    port: 8080,
    path: '/actuator/health',
    headersText: '',
    body: '{"ignored":true}',
    vars: {},
  });
  assert.equal(getDraft.body_mode, 'none');
  assert.equal(getDraft.body, '');
  assert.ok(getDraft.docs?.includes('kubectl -n default port-forward deployment/web 8080:8080'));
});

test('buildK8sLogsPacket tail-caps lines & chars and scrubs inline secrets', () => {
  const lines = Array.from({ length: 250 }, (_, i) =>
    i === 249 ? 'ERROR auth failed Bearer super_secret_token_999' : `line-${i}`,
  );
  const pkt = buildK8sLogsPacket({
    clusterName: 'prod-eu',
    clusterEnv: 'prod',
    ns: 'payments',
    pod: 'billing-0',
    container: 'app',
    search: 'ERROR',
    lines,
  });
  assert.equal(pkt.source, 'kubernetes_logs');
  assert.equal(pkt.total_lines, 250);
  assert.equal(pkt.included_lines, 200);
  assert.equal(pkt.truncated, true);
  assert.ok(!pkt.logs.includes('line-0'));
  assert.ok(pkt.logs.includes('line-50'));
  assert.ok(!pkt.logs.includes('super_secret_token_999'));
  assert.ok(pkt.logs.includes('Bearer [REDACTED]'));

  const charCapped = buildK8sLogsPacket({
    clusterName: 'dev',
    ns: 'default',
    pod: 'p1',
    lines: ['A'.repeat(100), 'TAIL_END'],
    maxChars: 20,
  });
  assert.equal(charCapped.truncated, true);
  assert.equal(charCapped.logs.length, 20);
  assert.ok(charCapped.logs.endsWith('TAIL_END'));
});
