import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ACTUATOR_PRESETS, defaultPort, fillTemplate, pathProblem, prettyBody, templateVars, workloadKindFor } from '../src/modules/kubernetes/podHttp.ts';

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
