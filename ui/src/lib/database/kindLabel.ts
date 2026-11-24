// A connection kind in words — the one map the Connections list and the DB
// workbench header both use.
export const KIND_LABEL = {
  ssh: 'SSH',
  mysql: 'MySQL',
  postgres: 'PostgreSQL',
  redis: 'Redis',
  mongodb: 'MongoDB',
  clickhouse: 'ClickHouse',
  custom: 'Custom',
  kafka: 'Kafka',
} as const;

export type KindWithLabel = keyof typeof KIND_LABEL;
