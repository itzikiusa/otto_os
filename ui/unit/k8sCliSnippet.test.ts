import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  clusterContext,
  kubectlDescribeCmd,
  kubectlExecCmd,
  kubectlGetCmd,
  kubectlLogsCmd,
  shQuote,
} from '../src/modules/kubernetes/cliSnippet.ts';

test('shQuote leaves safe tokens bare and single-quotes metacharacters safely', () => {
  assert.equal(shQuote('billing-0'), 'billing-0');
  assert.equal(shQuote('arn:aws:eks:eu-west-1:123456789012:cluster/prod'), 'arn:aws:eks:eu-west-1:123456789012:cluster/prod');
  assert.equal(shQuote('app=billing,tier=api'), 'app=billing,tier=api');
  assert.equal(shQuote('/bin/sh'), '/bin/sh');

  assert.equal(shQuote(''), "''");
  assert.equal(shQuote('my prod context'), "'my prod context'");
  assert.equal(shQuote("bob's-cluster"), "'bob'\\''s-cluster'");
  assert.equal(shQuote("a''b"), "'a'\\'''\\''b'");
  assert.equal(shQuote('$(rm -rf /)'), "'$(rm -rf /)'");
  assert.equal(shQuote('`id`'), "'`id`'");
  assert.equal(shQuote('foo; echo pwned'), "'foo; echo pwned'");
  assert.equal(shQuote('a|b&c$HOME"x"'), '\'a|b&c$HOME"x"\'');
  assert.equal(shQuote('line1\nline2'), "'line1\nline2'");
  assert.equal(shQuote('~root'), "'~root'");
  assert.equal(shQuote('pod\0name'), 'podname');
});

test('clusterContext resolves context_name, context, or params.context without leaking credentials', () => {
  assert.equal(clusterContext(null), undefined);
  assert.equal(clusterContext(undefined), undefined);
  assert.equal(clusterContext({ context_name: 'prod-eu' }), 'prod-eu');
  assert.equal(clusterContext({ context_name: '  ', context: 'staging-us' }), 'staging-us');
  assert.equal(clusterContext({ context_name: '', params: { context: 'dev-local' } }), 'dev-local');
  assert.equal(clusterContext({ context_name: 'primary', context: 'secondary', params: { context: 'tertiary' } }), 'primary');
  assert.equal(clusterContext({ context_name: '   ', context: '', params: { context: '   ' } }), undefined);
  assert.equal(clusterContext({ params: { context: 123, token: 'secret-token', kubeconfig_path: '/tmp/secret' } }), undefined);
});

test('kubectlGetCmd normalizes kinds, handles --context / -n / -o, and quotes safely', () => {
  assert.equal(
    kubectlGetCmd({ kind: 'pods', name: 'billing-0', ns: 'payments', output: 'yaml' }),
    'kubectl -n payments get pod billing-0 -o yaml',
  );
  assert.equal(
    kubectlGetCmd({ kind: 'deployments', name: 'api', ns: 'prod', context: 'prod-eu', output: 'json' }),
    'kubectl --context prod-eu -n prod get deployment api -o json',
  );
  assert.equal(
    kubectlGetCmd({ kind: 'nodes', name: 'ip-10-0-1-12', ns: '', context: 'eks-prod' }),
    'kubectl --context eks-prod get node ip-10-0-1-12',
  );
  assert.equal(
    kubectlGetCmd({ kind: 'pvcs', name: 'data-0', ns: 'db' }),
    'kubectl -n db get pvc data-0',
  );
  assert.equal(
    kubectlGetCmd({ kind: 'hpas', name: 'web-hpa', ns: 'web' }),
    'kubectl -n web get hpa web-hpa',
  );
  assert.equal(
    kubectlGetCmd({ kind: 'VirtualService', name: 'edge', ns: 'mesh' }),
    'kubectl -n mesh get virtualservice edge',
  );
  assert.equal(
    kubectlGetCmd({ kind: 'pods', name: 'pod 1', ns: "ns'a", context: 'prod $(whoami)', output: 'yaml' }),
    "kubectl --context 'prod $(whoami)' -n 'ns'\\''a' get pod 'pod 1' -o yaml",
  );
});

test('kubectlDescribeCmd builds describe commands with optional --context and -n', () => {
  assert.equal(
    kubectlDescribeCmd({ kind: 'pods', name: 'billing-0', ns: 'payments' }),
    'kubectl -n payments describe pod billing-0',
  );
  assert.equal(
    kubectlDescribeCmd({ kind: 'statefulsets', name: 'ledger', ns: 'payments', context: 'prod-eu' }),
    'kubectl --context prod-eu -n payments describe statefulset ledger',
  );
  assert.equal(
    kubectlDescribeCmd({ kind: 'nodes', name: 'worker-1' }),
    'kubectl describe node worker-1',
  );
  assert.equal(
    kubectlDescribeCmd({ kind: 'deployments', name: 'web;id', ns: 'default', context: 'my ctx' }),
    "kubectl --context 'my ctx' -n default describe deployment 'web;id'",
  );
});

test('kubectlLogsCmd handles single pod and every log viewer flag', () => {
  assert.equal(
    kubectlLogsCmd({ pod: 'billing-0', ns: 'payments' }),
    'kubectl -n payments logs billing-0',
  );
  assert.equal(
    kubectlLogsCmd({
      pod: 'billing-0',
      ns: 'payments',
      context: 'prod-eu',
      container: 'app',
      tail: 500,
      since: '15m',
      previous: true,
      timestamps: true,
      follow: true,
    }),
    'kubectl --context prod-eu -n payments logs billing-0 -c app --tail=500 --since=15m --previous --timestamps -f',
  );
  assert.equal(
    kubectlLogsCmd({
      pod: 'billing-0',
      ns: 'payments',
      since: '2026-09-01T10:00:00Z',
      tail: 0,
    }),
    'kubectl -n payments logs billing-0 --tail=0 --since-time=2026-09-01T10:00:00Z',
  );
  assert.equal(
    kubectlLogsCmd({ pod: 'billing-0', ns: 'payments', tail: -1 }),
    'kubectl -n payments logs billing-0',
  );
  assert.equal(
    kubectlLogsCmd({ pod: 'billing-0', ns: 'payments', tail: 123.9 }),
    'kubectl -n payments logs billing-0 --tail=123',
  );
});

test('kubectlLogsCmd handles workload selector mode and quoting', () => {
  assert.equal(
    kubectlLogsCmd({ selector: 'app=billing,tier=api', ns: 'payments', tail: 500, follow: true }),
    'kubectl -n payments logs -l app=billing,tier=api --all-containers --prefix --tail=500 -f',
  );
  // Named container replaces --all-containers in workload mode.
  assert.equal(
    kubectlLogsCmd({ selector: 'app=billing', ns: 'payments', container: 'sidecar', tail: 100 }),
    'kubectl -n payments logs -l app=billing -c sidecar --prefix --tail=100',
  );
  // Set-based selector with spaces and parens is safely single-quoted.
  assert.equal(
    kubectlLogsCmd({ selector: 'app in (web, api)', ns: 'prod', context: 'cluster 1' }),
    "kubectl --context 'cluster 1' -n prod logs -l 'app in (web, api)' --all-containers --prefix",
  );
});

test('kubectlExecCmd defaults shell to /bin/sh, handles container / --context / -n and quoting', () => {
  assert.equal(
    kubectlExecCmd({ pod: 'billing-0', ns: 'payments', container: 'app' }),
    'kubectl -n payments exec -it billing-0 -c app -- /bin/sh',
  );
  assert.equal(
    kubectlExecCmd({ pod: 'billing-0', ns: 'payments', context: 'prod-eu', shell: '/bin/bash' }),
    'kubectl --context prod-eu -n payments exec -it billing-0 -- /bin/bash',
  );
  assert.equal(
    kubectlExecCmd({ pod: 'billing-0' }),
    'kubectl exec -it billing-0 -- /bin/sh',
  );
  assert.equal(
    kubectlExecCmd({ pod: 'billing-0', ns: 'payments', container: 'my ctr', shell: '/bin/sh' }),
    "kubectl -n payments exec -it billing-0 -c 'my ctr' -- /bin/sh",
  );
});
