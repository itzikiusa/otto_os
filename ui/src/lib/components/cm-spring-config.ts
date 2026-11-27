// CodeMirror 6 autocompletion for Spring Boot `application*.{properties,yml,yaml}`
// and `bootstrap*.{properties,yml,yaml}` configuration files.
import { autocompletion, type Completion, type CompletionContext, type CompletionResult } from '@codemirror/autocomplete';
import type { Extension } from '@codemirror/state';

export interface SpringPropSpec {
  key: string;
  type: 'string' | 'integer' | 'boolean' | 'enum';
  defaultValue?: string;
  info: string;
  allowed?: readonly string[];
  deprecated?: string;
}

export const SPRING_BOOT_CONFIG_PROPS: readonly SpringPropSpec[] = [
  { key: 'server.port', type: 'integer', defaultValue: '8080', info: 'Server HTTP port.' },
  { key: 'server.address', type: 'string', info: 'Network address to which the server should bind.' },
  { key: 'server.servlet.context-path', type: 'string', defaultValue: '/', info: 'Context path of the application.' },
  { key: 'server.shutdown', type: 'enum', defaultValue: 'immediate', allowed: ['immediate', 'graceful'], info: 'Type of shutdown supported by the server.' },
  { key: 'server.compression.enabled', type: 'boolean', defaultValue: 'false', allowed: ['true', 'false'], info: 'Whether response compression is enabled.' },
  { key: 'spring.application.name', type: 'string', info: 'Application name used for logging, metrics, and service discovery.' },
  { key: 'spring.profiles.active', type: 'string', info: 'Comma-separated list of active Spring profiles.' },
  { key: 'spring.profiles.default', type: 'string', defaultValue: 'default', info: 'Name of the profile to enable if no profile is active.' },
  { key: 'spring.main.banner-mode', type: 'enum', defaultValue: 'console', allowed: ['off', 'console', 'log'], info: 'Mode used to display the banner when the application runs.' },
  { key: 'spring.main.web-application-type', type: 'enum', allowed: ['none', 'servlet', 'reactive'], info: 'Explicitly request a specific web application type.' },
  { key: 'spring.threads.virtual.enabled', type: 'boolean', defaultValue: 'false', allowed: ['true', 'false'], info: 'Whether to use Java 21+ virtual threads.' },
  { key: 'spring.datasource.url', type: 'string', info: 'JDBC URL of the database.' },
  { key: 'spring.datasource.username', type: 'string', info: 'Login username of the database.' },
  { key: 'spring.datasource.password', type: 'string', info: 'Login password of the database.' },
  { key: 'spring.datasource.driver-class-name', type: 'string', info: 'Fully qualified name of the JDBC driver.' },
  { key: 'spring.datasource.hikari.maximum-pool-size', type: 'integer', defaultValue: '10', info: 'Maximum number of connections in the HikariCP pool.' },
  { key: 'spring.jpa.hibernate.ddl-auto', type: 'enum', defaultValue: 'none', allowed: ['none', 'validate', 'update', 'create', 'create-drop'], info: 'DDL mode for Hibernate schema management.' },
  { key: 'spring.jpa.show-sql', type: 'boolean', defaultValue: 'false', allowed: ['true', 'false'], info: 'Enable logging of SQL statements.' },
  { key: 'spring.jpa.open-in-view', type: 'boolean', defaultValue: 'true', allowed: ['true', 'false'], info: 'Register OpenEntityManagerInViewInterceptor.' },
  { key: 'spring.flyway.enabled', type: 'boolean', defaultValue: 'true', allowed: ['true', 'false'], info: 'Enable Flyway database migrations.' },
  { key: 'spring.liquibase.enabled', type: 'boolean', defaultValue: 'true', allowed: ['true', 'false'], info: 'Enable Liquibase database migrations.' },
  { key: 'spring.data.redis.host', type: 'string', defaultValue: 'localhost', info: 'Redis server host.' },
  { key: 'spring.data.redis.port', type: 'integer', defaultValue: '6379', info: 'Redis server port.' },
  { key: 'spring.kafka.bootstrap-servers', type: 'string', defaultValue: 'localhost:9092', info: 'Comma-delimited Kafka bootstrap servers.' },
  { key: 'spring.kafka.consumer.group-id', type: 'string', info: 'Kafka consumer group ID.' },
  { key: 'spring.jackson.serialization.indent-output', type: 'boolean', defaultValue: 'false', allowed: ['true', 'false'], info: 'Pretty-print serialized JSON output.' },
  { key: 'spring.jackson.default-property-inclusion', type: 'enum', defaultValue: 'always', allowed: ['always', 'non_null', 'non_absent', 'non_empty', 'non_default'], info: 'Controls property inclusion during JSON serialization.' },
  { key: 'spring.devtools.restart.enabled', type: 'boolean', defaultValue: 'true', allowed: ['true', 'false'], info: 'Enable automatic DevTools restart.' },
  { key: 'spring.devtools.restart.trigger-file', type: 'string', info: 'Specific trigger file name required for DevTools restart.' },
  { key: 'spring.devtools.livereload.enabled', type: 'boolean', defaultValue: 'true', allowed: ['true', 'false'], info: 'Enable DevTools LiveReload server.' },
  { key: 'management.server.port', type: 'integer', info: 'Management Actuator HTTP port (defaults to server.port).' },
  { key: 'management.endpoints.web.base-path', type: 'string', defaultValue: '/actuator', info: 'Base path for Web Actuator endpoints.' },
  { key: 'management.endpoints.web.exposure.include', type: 'string', defaultValue: 'health', allowed: ['*', 'health,info,metrics,beans,mappings,env', 'health,info'], info: 'Endpoint IDs exposed over HTTP.' },
  { key: 'management.endpoints.web.exposure.exclude', type: 'string', info: 'Endpoint IDs excluded from HTTP exposure.' },
  { key: 'management.endpoint.health.show-details', type: 'enum', defaultValue: 'never', allowed: ['never', 'when-authorized', 'always'], info: 'When to show full health component details.' },
  { key: 'management.endpoint.health.probes.enabled', type: 'boolean', defaultValue: 'false', allowed: ['true', 'false'], info: 'Enable Kubernetes liveness and readiness probes.' },
  { key: 'logging.level.root', type: 'enum', defaultValue: 'INFO', allowed: ['TRACE', 'DEBUG', 'INFO', 'WARN', 'ERROR', 'FATAL', 'OFF'], info: 'Root logger severity level.' },
  { key: 'logging.file.name', type: 'string', info: 'Log file name.' },
];

export function isSpringConfigPath(filePath: string): boolean {
  const base = (filePath.split(/[\\/]/).pop() ?? '').toLowerCase();
  return (
    (base.startsWith('application') || base.startsWith('bootstrap'))
    && (base.endsWith('.properties') || base.endsWith('.yml') || base.endsWith('.yaml'))
  );
}

function lookupSpec(key: string): SpringPropSpec | undefined {
  const exact = SPRING_BOOT_CONFIG_PROPS.find((p) => p.key === key);
  if (exact) return exact;
  if (key.startsWith('logging.level.')) {
    return SPRING_BOOT_CONFIG_PROPS.find((p) => p.key === 'logging.level.root');
  }
  return undefined;
}

function inferYamlParentPrefix(lines: string[], targetIdx: number): string[] {
  const targetLine = lines[targetIdx] ?? '';
  const targetIndent = targetLine.length - targetLine.trimStart().length;
  const stack: Array<{ indent: number; key: string }> = [];
  for (let i = 0; i < targetIdx; i++) {
    const raw = lines[i];
    const trimmed = raw.trim();
    if (!trimmed || trimmed.startsWith('#') || trimmed.startsWith('---') || trimmed.startsWith('- ')) continue;
    const colon = trimmed.indexOf(':');
    if (colon <= 0) continue;
    const indent = raw.length - raw.trimStart().length;
    const seg = trimmed.slice(0, colon).trim().replace(/^['"]|['"]$/g, '');
    const rest = trimmed.slice(colon + 1).trim();
    while (stack.length > 0 && stack[stack.length - 1].indent >= indent) stack.pop();
    if (!rest || rest.startsWith('#')) {
      stack.push({ indent, key: seg });
    }
  }
  while (stack.length > 0 && stack[stack.length - 1].indent >= targetIndent) stack.pop();
  return stack.map((s) => s.key);
}

export interface SpringCompletionMatch {
  from: number;
  options: Completion[];
}

/** Pure completion engine for Spring Boot `.properties` and `.yml`/`.yaml` buffers. */
export function completeSpringConfigAt(
  filePath: string,
  docText: string,
  pos: number,
  explicit: boolean,
): SpringCompletionMatch | null {
  if (!isSpringConfigPath(filePath)) return null;
  const lineStart = docText.lastIndexOf('\n', Math.max(0, pos - 1)) + 1;
  const before = docText.slice(lineStart, pos);
  const trimmedBefore = before.trimStart();
  if (trimmedBefore.startsWith('#') || trimmedBefore.startsWith('!')) return null;

  const isProps = filePath.toLowerCase().endsWith('.properties');
  if (isProps) {
    const eqIdx = before.search(/[=:]/);
    if (eqIdx >= 0) {
      const key = before.slice(0, eqIdx).trim();
      const spec = lookupSpec(key);
      if (!spec?.allowed?.length) return null;
      const valPrefix = before.slice(eqIdx + 1).trimStart();
      const from = pos - valPrefix.length;
      const options: Completion[] = spec.allowed
        .filter((v) => !valPrefix || v.toLowerCase().startsWith(valPrefix.toLowerCase()))
        .map((v) => ({ label: v, type: 'enum', detail: spec.type, info: spec.info }));
      return options.length ? { from, options } : null;
    }
    if (!explicit && trimmedBefore.length < 2) return null;
    const from = pos - trimmedBefore.length;
    const q = trimmedBefore.toLowerCase();
    const options: Completion[] = SPRING_BOOT_CONFIG_PROPS
      .filter((p) => !q || p.key.toLowerCase().includes(q))
      .map((p) => ({
        label: p.key,
        type: 'property',
        detail: p.defaultValue ? `${p.type} = ${p.defaultValue}` : p.type,
        info: p.info,
        apply: `${p.key}=`,
      }));
    return options.length ? { from, options } : null;
  }

  // YAML mode (`.yml` / `.yaml`)
  const lines = docText.split('\n');
  const lineIdx = docText.slice(0, lineStart).split('\n').length - 1;
  const parentPrefix = inferYamlParentPrefix(lines, lineIdx);
  const colonIdx = before.indexOf(':');

  if (colonIdx >= 0) {
    const localKey = before.slice(0, colonIdx).trim();
    const fullKey = [...parentPrefix, localKey].filter(Boolean).join('.');
    const spec = lookupSpec(fullKey);
    if (!spec?.allowed?.length) return null;
    const valPrefix = before.slice(colonIdx + 1).trimStart();
    const from = pos - valPrefix.length;
    const options: Completion[] = spec.allowed
      .filter((v) => !valPrefix || v.toLowerCase().startsWith(valPrefix.toLowerCase()))
      .map((v) => ({ label: v, type: 'enum', detail: spec.type, info: spec.info }));
    return options.length ? { from, options } : null;
  }

  if (!explicit && trimmedBefore.length < 1 && parentPrefix.length === 0) return null;
  const from = pos - trimmedBefore.length;
  const dottedPrefix = parentPrefix.length > 0 ? `${parentPrefix.join('.')}.` : '';
  const q = `${dottedPrefix}${trimmedBefore}`.toLowerCase();

  const seen = new Set<string>();
  const options: Completion[] = [];
  for (const p of SPRING_BOOT_CONFIG_PROPS) {
    if (dottedPrefix && !p.key.startsWith(dottedPrefix)) continue;
    const remainder = dottedPrefix ? p.key.slice(dottedPrefix.length) : p.key;
    const nextSeg = remainder.split('.')[0];
    if (!nextSeg) continue;
    const fullCand = `${dottedPrefix}${nextSeg}`.toLowerCase();
    if (q && !fullCand.startsWith(q) && !p.key.toLowerCase().startsWith(q)) continue;
    if (seen.has(nextSeg)) continue;
    seen.add(nextSeg);
    const isLeaf = !remainder.includes('.');
    options.push({
      label: nextSeg,
      type: 'property',
      detail: isLeaf ? (p.defaultValue ? `${p.type} = ${p.defaultValue}` : p.type) : 'group',
      info: isLeaf ? p.info : `Spring Boot config group (${dottedPrefix}${nextSeg}.*)`,
      apply: isLeaf ? `${nextSeg}: ` : `${nextSeg}:\n  `,
    });
  }
  return options.length ? { from, options } : null;
}

export function springConfigCompletion(filePath: string): Extension {
  if (!isSpringConfigPath(filePath)) return [];
  return autocompletion({
    override: [
      (ctx: CompletionContext): CompletionResult | null => {
        const res = completeSpringConfigAt(filePath, ctx.state.doc.toString(), ctx.pos, ctx.explicit);
        if (!res) return null;
        return { from: res.from, options: res.options, validFor: /^[\w.-]*$/ };
      },
    ],
  });
}
