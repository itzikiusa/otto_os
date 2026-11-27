import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { completeSpringConfigAt, isSpringConfigPath } from '../src/lib/components/cm-spring-config.ts';

describe('Spring Boot configuration intelligence', () => {
  it('recognizes Spring Boot application and bootstrap config filenames', () => {
    assert.equal(isSpringConfigPath('/repo/src/main/resources/application.properties'), true);
    assert.equal(isSpringConfigPath('/repo/src/main/resources/application-dev.yml'), true);
    assert.equal(isSpringConfigPath('/repo/config/bootstrap-prod.yaml'), true);
    assert.equal(isSpringConfigPath('/repo/src/main/java/App.java'), false);
    assert.equal(isSpringConfigPath('/repo/docker-compose.yml'), false);
  });

  it('completes Spring Boot property keys and enum values in application.properties', () => {
    const doc1 = 'server.po';
    const r1 = completeSpringConfigAt('application.properties', doc1, doc1.length, false);
    assert.ok(r1, 'expected key completion for server.po');
    assert.ok(r1.options.some((o) => o.label === 'server.port'));

    const doc2 = 'spring.jpa.hibernate.ddl-auto=val';
    const r2 = completeSpringConfigAt('application.properties', doc2, doc2.length, false);
    assert.ok(r2, 'expected enum completion for ddl-auto');
    assert.ok(r2.options.some((o) => o.label === 'validate'));
  });

  it('completes nested Spring Boot keys and enum values in application.yml', () => {
    const ymlKeyDoc = 'server:\n  sh';
    const r1 = completeSpringConfigAt('application.yml', ymlKeyDoc, ymlKeyDoc.length, false);
    assert.ok(r1, 'expected nested YAML key completion under server:');
    assert.ok(r1.options.some((o) => o.label === 'shutdown'));

    const ymlValDoc = 'server:\n  shutdown: gr';
    const r2 = completeSpringConfigAt('application.yml', ymlValDoc, ymlValDoc.length, false);
    assert.ok(r2, 'expected nested YAML enum completion for server.shutdown');
    assert.ok(r2.options.some((o) => o.label === 'graceful'));
  });
});
