// Run-dialog hints for well-known images: the usual port mappings, the
// environment a first local run needs (required first, with safe local-dev
// example values — never real secrets) and, when the image needs one, the
// command. Keyed by repository name without registry, namespace or tag; each
// entry names the docs it was checked against (2026-11-20). Images not listed
// fall back to the ports the local image itself declares (RunContainerModal).

export interface ImageHint {
  ports: string[];
  env?: string[];
  cmd?: string;
  note?: string;
  source: string;
}

/** What the Run dialog offers: a table hint, or ports an image declares. */
export interface RunSuggestion {
  name: string;
  ports: string[];
  env?: string[];
  cmd?: string;
  note?: string;
}

const DL = 'https://github.com/docker-library/docs/tree/master/';
const MAIL_PORTS = ['1025:1025', '8025:8025'];

export const IMAGE_HINTS: Record<string, ImageHint> = {
  postgres: {
    ports: ['5432:5432'],
    env: ['POSTGRES_PASSWORD=postgres', 'POSTGRES_DB=app'],
    source: DL + 'postgres',
  },
  mysql: { ports: ['3306:3306'], env: ['MYSQL_ROOT_PASSWORD=mysql', 'MYSQL_DATABASE=app'], source: DL + 'mysql' },
  mariadb: {
    ports: ['3306:3306'],
    env: ['MARIADB_ROOT_PASSWORD=mariadb', 'MARIADB_DATABASE=app'],
    source: DL + 'mariadb',
  },
  mongo: {
    ports: ['27017:27017'],
    env: ['MONGO_INITDB_ROOT_USERNAME=admin', 'MONGO_INITDB_ROOT_PASSWORD=admin'],
    note: 'Set both or neither; with them, auth is on.',
    source: DL + 'mongo',
  },
  redis: { ports: ['6379:6379'], source: DL + 'redis' },
  valkey: { ports: ['6379:6379'], source: 'https://github.com/valkey-io/valkey-container' },
  memcached: { ports: ['11211:11211'], source: DL + 'memcached' },
  nginx: { ports: ['8080:80'], source: DL + 'nginx' },
  httpd: { ports: ['8080:80'], source: DL + 'httpd' },
  rabbitmq: {
    ports: ['5672:5672', '15672:15672'],
    env: ['RABBITMQ_DEFAULT_USER=admin', 'RABBITMQ_DEFAULT_PASS=admin'],
    note: 'The web UI on 15672 needs a -management tag.',
    source: DL + 'rabbitmq',
  },
  'apache/kafka': {
    ports: ['9092:9092'],
    note: 'Runs one KRaft node with no settings.',
    source: 'https://kafka.apache.org/quickstart',
  },
  'bitnami/kafka': {
    ports: ['9092:9092'],
    env: [
      'KAFKA_CFG_NODE_ID=0',
      'KAFKA_CFG_PROCESS_ROLES=controller,broker',
      'KAFKA_CFG_LISTENERS=PLAINTEXT://:9092,CONTROLLER://:9093',
      'KAFKA_CFG_ADVERTISED_LISTENERS=PLAINTEXT://localhost:9092',
      'KAFKA_CFG_LISTENER_SECURITY_PROTOCOL_MAP=CONTROLLER:PLAINTEXT,PLAINTEXT:PLAINTEXT',
      'KAFKA_CFG_CONTROLLER_LISTENER_NAMES=CONTROLLER',
      'KAFKA_CFG_INTER_BROKER_LISTENER_NAME=PLAINTEXT',
    ],
    source: 'https://github.com/bitnami/containers/blob/main/bitnami/kafka/docker-compose.yml',
  },
  zookeeper: { ports: ['2181:2181'], source: DL + 'zookeeper' },
  elasticsearch: {
    ports: ['9200:9200'],
    env: ['discovery.type=single-node', 'xpack.security.enabled=false'],
    note: 'Needs a version tag (no latest); this turns security off for local use.',
    source: 'https://www.elastic.co/docs/deploy-manage/deploy/self-managed/install-elasticsearch-docker-basic',
  },
  opensearch: {
    ports: ['9200:9200'],
    env: ['discovery.type=single-node', 'OPENSEARCH_INITIAL_ADMIN_PASSWORD=Otto-Local-Dev-1!'],
    note: 'Since 2.12 the admin password must be a strong one.',
    source: 'https://github.com/opensearch-project/opensearch-build/blob/main/docker/release/README.md',
  },
  kibana: {
    ports: ['5601:5601'],
    note: 'Needs a version tag (no latest).',
    source: 'https://www.elastic.co/docs/deploy-manage/deploy/self-managed/install-kibana-with-docker',
  },
  clickhouse: {
    ports: ['8123:8123', '9000:9000'],
    env: ['CLICKHOUSE_PASSWORD=clickhouse'],
    note: 'Without a password the default user can’t connect from outside.',
    source: 'https://github.com/ClickHouse/ClickHouse/blob/master/docker/server/README.md',
  },
  minio: {
    ports: ['9000:9000', '9001:9001'],
    env: ['MINIO_ROOT_USER=minioadmin', 'MINIO_ROOT_PASSWORD=minioadmin'],
    cmd: 'server /data --console-address :9001',
    source: 'https://github.com/minio/minio/blob/master/docs/docker/README.md',
  },
  localstack: {
    ports: ['4566:4566'],
    source: 'https://docs.localstack.cloud/aws/getting-started/installation/',
  },
  mssql: {
    ports: ['1433:1433'],
    env: ['ACCEPT_EULA=Y', 'MSSQL_SA_PASSWORD=Otto-Local-Dev-1'],
    note: 'SA password: 8+ characters from three of upper, lower, digits, symbols.',
    source: 'https://learn.microsoft.com/en-us/sql/linux/quickstart-install-connect-docker',
  },
  cassandra: { ports: ['9042:9042'], source: DL + 'cassandra' },
  neo4j: {
    ports: ['7474:7474', '7687:7687'],
    env: ['NEO4J_AUTH=none'],
    note: 'Auth off for local use; without it, sign in as neo4j / neo4j.',
    source: DL + 'neo4j',
  },
  grafana: {
    ports: ['3000:3000'],
    source: 'https://grafana.com/docs/grafana/latest/setup-grafana/configure-docker/',
  },
  prometheus: { ports: ['9090:9090'], source: 'https://prometheus.io/docs/prometheus/latest/installation/' },
  jaeger: {
    ports: ['16686:16686', '4317:4317', '4318:4318'],
    note: 'UI on 16686; OTLP gRPC 4317 and HTTP 4318.',
    source: 'https://www.jaegertracing.io/docs/latest/getting-started/',
  },
  keycloak: {
    ports: ['8080:8080'],
    env: ['KC_BOOTSTRAP_ADMIN_USERNAME=admin', 'KC_BOOTSTRAP_ADMIN_PASSWORD=admin'],
    cmd: 'start-dev',
    source: 'https://www.keycloak.org/getting-started/getting-started-docker',
  },
  vault: {
    ports: ['8200:8200'],
    note: 'Runs a dev server by default; the root token is in the logs.',
    source: 'https://github.com/hashicorp/vault/blob/main/Dockerfile',
  },
  consul: {
    ports: ['8500:8500', '8600:8600/udp'],
    note: 'Runs a dev agent by default; UI on 8500.',
    source: 'https://developer.hashicorp.com/consul/docs/deploy/server/docker',
  },
  'coreos/etcd': {
    ports: ['2379:2379', '2380:2380'],
    cmd: '/usr/local/bin/etcd --listen-client-urls http://0.0.0.0:2379 --advertise-client-urls http://localhost:2379',
    source: 'https://etcd.io/docs/v3.5/op-guide/container/',
  },
  nats: { ports: ['4222:4222', '8222:8222'], source: DL + 'nats' },
  mailhog: { ports: MAIL_PORTS, note: 'SMTP on 1025, web UI on 8025.', source: 'https://github.com/mailhog/MailHog' },
  mailpit: {
    ports: MAIL_PORTS,
    note: 'SMTP on 1025, web UI on 8025.',
    source: 'https://mailpit.axllent.org/docs/install/docker/',
  },
  adminer: { ports: ['8080:8080'], source: DL + 'adminer' },
  pgadmin: {
    ports: ['5050:80'],
    env: ['PGADMIN_DEFAULT_EMAIL=admin@example.com', 'PGADMIN_DEFAULT_PASSWORD=admin'],
    source: 'https://www.pgadmin.org/docs/pgadmin4/latest/container_deployment.html',
  },
  sonarqube: { ports: ['9000:9000'], source: DL + 'sonarqube' },
  jenkins: {
    ports: ['8080:8080', '50000:50000'],
    note: 'The first admin password is in /var/jenkins_home/secrets/initialAdminPassword.',
    source: 'https://github.com/jenkinsci/docker/blob/master/README.md',
  },
  registry: { ports: ['5000:5000'], source: DL + 'registry' },
};

/** Other names for a listed image (a last path segment or `namespace/name`). */
const ALIASES: Record<string, string> = {
  postgresql: 'postgres',
  'apache/kafka-native': 'apache/kafka',
  'etcd-development/etcd': 'coreos/etcd',
  'clickhouse-server': 'clickhouse',
  'mssql/server': 'mssql',
  'jaegertracing/all-in-one': 'jaeger',
  pgadmin4: 'pgadmin',
};

/** `docker.io/library/mongo:9.0` → `mongo`; `quay.io/keycloak/keycloak:26` → `keycloak/keycloak`. */
export function imageRepoPath(ref: string): string {
  let s = ref.trim().toLowerCase().split('@')[0];
  if (s.lastIndexOf(':') > s.lastIndexOf('/')) s = s.slice(0, s.lastIndexOf(':'));
  const parts = s.split('/').filter(Boolean);
  if (parts.length > 1 && /[.:]|^localhost$/.test(parts[0])) parts.shift();
  if (parts[0] === 'library') parts.shift();
  return parts.join('/');
}

/** The table hint for an image reference: full path, then `namespace/name`, then the name. */
export function lookupImageHint(ref: string): RunSuggestion | null {
  const parts = imageRepoPath(ref).split('/');
  for (const key of [parts.join('/'), parts.slice(-2).join('/'), parts[parts.length - 1]]) {
    const name = Object.hasOwn(ALIASES, key) ? ALIASES[key] : key;
    if (!Object.hasOwn(IMAGE_HINTS, name)) continue;
    const { source: _source, ...hint } = IMAGE_HINTS[name];
    return { name, ...hint };
  }
  return null;
}

/** `P:P` for each TCP port an image declares (`Config.ExposedPorts` keys). */
export function portsFromExposed(exposed: readonly string[]): string[] {
  return exposed.flatMap((p) => {
    const m = /^(\d+)\/tcp$/.exec(p);
    return m ? [`${m[1]}:${m[1]}`] : [];
  });
}

export interface RunFields {
  ports: string;
  env: string;
  cmd: string;
}

/** Fill only the fields that are still empty; what the person typed stays. */
export function fillEmptyFields(fields: RunFields, s: RunSuggestion): RunFields {
  return {
    ports: fields.ports.trim() ? fields.ports : s.ports.join(', '),
    env: fields.env.trim() ? fields.env : (s.env ?? []).join('\n'),
    cmd: fields.cmd.trim() ? fields.cmd : (s.cmd ?? ''),
  };
}

/** `27017:27017 · MONGO_INITDB_ROOT_USERNAME, MONGO_INITDB_ROOT_PASSWORD` */
export function suggestionSummary(s: RunSuggestion): string {
  const env = (s.env ?? []).map((e) => e.slice(0, e.indexOf('=')));
  return [s.ports.join(', '), env.join(', '), s.cmd ? `command ${s.cmd}` : ''].filter(Boolean).join(' · ');
}
