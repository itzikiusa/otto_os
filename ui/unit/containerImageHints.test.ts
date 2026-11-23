// Unit tests for the Run dialog's image hints: reference normalisation
// (registry / namespace / tag / digest), aliases, unknown images, the
// image-declared port fallback and the fill-only-empty "Use" behaviour.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import {
  IMAGE_HINTS,
  fillEmptyFields,
  imageRepoPath,
  lookupImageHint,
  portsFromExposed,
  suggestionSummary,
} from '../src/modules/containers/image-hints.ts';
import { parseCommandArgs, parseEnvLines, parsePortSpecs } from '../src/modules/containers/containers-util.ts';

describe('image hint lookup', () => {
  it('strips registry, library namespace, tag and digest', () => {
    assert.equal(imageRepoPath('mongo:9.0'), 'mongo');
    assert.equal(imageRepoPath('docker.io/library/mongo'), 'mongo');
    assert.equal(imageRepoPath('nginx@sha256:abc'), 'nginx');
    assert.equal(imageRepoPath('localhost:5000/team/api:1'), 'team/api');
    assert.equal(imageRepoPath('  Quay.io/Keycloak/Keycloak:26  '), 'keycloak/keycloak');
  });

  it('matches by name, namespace/name and aliases', () => {
    const name = (ref: string) => lookupImageHint(ref)?.name ?? null;
    assert.equal(name('mongo:9.0'), 'mongo');
    assert.equal(name('docker.io/library/mongo'), 'mongo');
    assert.equal(name('bitnami/postgresql:16'), 'postgres');
    assert.equal(name('mcr.microsoft.com/mssql/server:2022-latest'), 'mssql');
    assert.equal(name('apache/kafka:4.4.0'), 'apache/kafka');
    assert.equal(name('apache/kafka-native:4.4.0'), 'apache/kafka');
    assert.equal(name('bitnami/kafka:4.3'), 'bitnami/kafka');
    assert.equal(name('quay.io/coreos/etcd:v3.6.0'), 'coreos/etcd');
    assert.equal(name('gcr.io/etcd-development/etcd:v3.6.0'), 'coreos/etcd');
    assert.equal(name('jaegertracing/all-in-one:1.6'), 'jaeger');
    assert.equal(name('clickhouse/clickhouse-server'), 'clickhouse');
    assert.equal(name('dpage/pgadmin4'), 'pgadmin');
  });

  it('returns null for unknown images and prototype keys', () => {
    assert.equal(lookupImageHint('ghcr.io/acme/payments:1.4'), null);
    assert.equal(lookupImageHint('acme/server'), null);
    assert.equal(lookupImageHint('constructor'), null);
    // The apache/kafka note and the etcd command path are true only for those images.
    assert.equal(lookupImageHint('wurstmeister/kafka'), null);
    assert.equal(lookupImageHint('confluentinc/cp-kafka:8.0'), null);
    assert.equal(lookupImageHint('bitnami/etcd:3.6'), null);
    assert.equal(lookupImageHint(''), null);
  });

  it('summarises ports, env names and command', () => {
    const mongo = lookupImageHint('mongo:9.0');
    assert.ok(mongo);
    assert.equal(suggestionSummary(mongo), '27017:27017 · MONGO_INITDB_ROOT_USERNAME, MONGO_INITDB_ROOT_PASSWORD');
    const kc = lookupImageHint('quay.io/keycloak/keycloak:26.8.2');
    assert.ok(kc);
    assert.match(suggestionSummary(kc), / · command start-dev$/);
  });

  it('every entry parses with the dialog’s own validators and cites a source', () => {
    for (const [name, h] of Object.entries(IMAGE_HINTS)) {
      assert.match(h.source, /^https:\/\//, name);
      assert.ok(h.ports.length > 0, name);
      assert.equal(parsePortSpecs(h.ports.join(', ')).error, null, name);
      assert.equal(parseEnvLines((h.env ?? []).join('\n')).error, null, name);
      if (h.cmd) assert.ok(parseCommandArgs(h.cmd).length > 0, name);
    }
  });
});

describe('image-declared port fallback', () => {
  it('suggests P:P for TCP ports only', () => {
    assert.deepEqual(portsFromExposed(['53/udp', '5432/tcp', '8080/tcp']), ['5432:5432', '8080:8080']);
    assert.deepEqual(portsFromExposed([]), []);
  });
});

describe('Use fills only empty fields', () => {
  const kc = lookupImageHint('keycloak')!;

  it('fills every empty field', () => {
    assert.deepEqual(fillEmptyFields({ ports: '', env: '  ', cmd: '' }, kc), {
      ports: '8080:8080',
      env: 'KC_BOOTSTRAP_ADMIN_USERNAME=admin\nKC_BOOTSTRAP_ADMIN_PASSWORD=admin',
      cmd: 'start-dev',
    });
  });

  it('never overwrites what the person typed', () => {
    assert.deepEqual(fillEmptyFields({ ports: '9999:8080', env: 'A=1', cmd: 'start' }, kc), {
      ports: '9999:8080',
      env: 'A=1',
      cmd: 'start',
    });
    assert.deepEqual(fillEmptyFields({ ports: '', env: 'A=1', cmd: '' }, { name: 'x', ports: ['80:80'] }), {
      ports: '80:80',
      env: 'A=1',
      cmd: '',
    });
  });
});
