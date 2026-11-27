import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  completeSpringConfigAt,
  isSpringConfigPath,
  SPRING_BOOT_CONFIG_PROPS,
} from '../src/lib/components/cm-spring-config.ts';

describe('Spring Boot configuration intelligence', () => {
  it('recognizes Spring Boot application and bootstrap config filenames', () => {
    assert.equal(isSpringConfigPath('/repo/src/main/resources/application.properties'), true);
    assert.equal(isSpringConfigPath('/repo/src/main/resources/application-dev.yml'), true);
    assert.equal(isSpringConfigPath('/repo/config/bootstrap-prod.yaml'), true);
    assert.equal(isSpringConfigPath('/repo/src/main/java/App.java'), false);
    assert.equal(isSpringConfigPath('/repo/docker-compose.yml'), false);
  });

  it('shares the single catalog including spring.config.import and spring.profiles.default (DUP-1)', () => {
    const keys = new Set(SPRING_BOOT_CONFIG_PROPS.map((p) => p.key));
    assert.equal(keys.has('spring.config.import'), true);
    assert.equal(keys.has('spring.profiles.default'), true);
    assert.equal(keys.has('spring.redis.host'), false, 'deprecated keys are omitted from completion');
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

  it('completes nested Spring Boot YAML groups with relative indentation (FE-1)', () => {
    const nestedDoc = 'spring:\n  jp';
    const rGroup = completeSpringConfigAt('application.yml', nestedDoc, nestedDoc.length, false);
    assert.ok(rGroup, 'expected nested YAML group completion for jpa under spring');
    const jpaOpt = rGroup.options.find((o) => o.label === 'jpa');
    assert.ok(jpaOpt, 'expected jpa option');
    assert.equal(jpaOpt.apply, 'jpa:\n    ', 'nested group under 2-space indent must indent children to 4 spaces');

    const fourSpaceDoc = 'spring:\n    jp';
    const rFour = completeSpringConfigAt('application.yml', fourSpaceDoc, fourSpaceDoc.length, false);
    const jpaFour = rFour?.options.find((o) => o.label === 'jpa');
    assert.equal(jpaFour?.apply, 'jpa:\n        ', '4-space YAML document indents nested children to 8 spaces');

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
