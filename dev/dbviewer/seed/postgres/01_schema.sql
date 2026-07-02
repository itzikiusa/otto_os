-- Seed schema for the Otto DB Explorer dev PostgreSQL instance.
-- Mirrors the MySQL seed (shopdb: customers/products/orders/order_items) so the
-- Postgres driver exercises the same schema introspection, foreign keys (visual
-- JOIN builder), indexes, a view, a materialized view, and a function. Runs
-- against database `shopdb` as user `otto` (POSTGRES_DB/POSTGRES_USER).

CREATE TABLE customers (
    id          SERIAL PRIMARY KEY,
    email       VARCHAR(255) NOT NULL UNIQUE,
    full_name   VARCHAR(255) NOT NULL,
    country     VARCHAR(2)   NOT NULL DEFAULT 'US',
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT now()
);
CREATE INDEX idx_customers_country ON customers(country);

CREATE TABLE products (
    id          SERIAL PRIMARY KEY,
    sku         VARCHAR(64)  NOT NULL UNIQUE,
    name        VARCHAR(255) NOT NULL,
    price_cents INTEGER      NOT NULL,
    in_stock    BOOLEAN      NOT NULL DEFAULT true,
    metadata    JSONB
);

CREATE TABLE orders (
    id          SERIAL PRIMARY KEY,
    customer_id INTEGER      NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    status      VARCHAR(16)  NOT NULL DEFAULT 'pending',
    total_cents INTEGER      NOT NULL DEFAULT 0,
    placed_at   TIMESTAMPTZ  NOT NULL DEFAULT now()
);
CREATE INDEX idx_orders_status ON orders(status);

CREATE TABLE order_items (
    id          SERIAL PRIMARY KEY,
    order_id    INTEGER NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    product_id  INTEGER NOT NULL REFERENCES products(id),
    quantity    INTEGER NOT NULL DEFAULT 1,
    unit_cents  INTEGER NOT NULL
);

CREATE VIEW order_totals AS
    SELECT o.id AS order_id, c.email, o.status, o.total_cents
    FROM orders o JOIN customers c ON c.id = o.customer_id;

INSERT INTO customers (email, full_name, country) VALUES
    ('ada@example.com',   'Ada Lovelace',   'GB'),
    ('alan@example.com',  'Alan Turing',    'GB'),
    ('grace@example.com', 'Grace Hopper',   'US'),
    ('linus@example.com', 'Linus Torvalds', 'FI');

INSERT INTO products (sku, name, price_cents, in_stock, metadata) VALUES
    ('SKU-1', 'Mechanical Keyboard', 12900, true,  '{"color":"black","switches":"brown"}'),
    ('SKU-2', 'USB-C Cable',          1500, true,  '{"length_m":2}'),
    ('SKU-3', '4K Monitor',          39900, false, NULL),
    ('SKU-4', 'Webcam',               8900, true,  '{"resolution":"1080p"}');

INSERT INTO orders (customer_id, status, total_cents) VALUES
    (1, 'paid',    14400),
    (1, 'shipped', 39900),
    (2, 'pending',  8900),
    (3, 'paid',     1500);

INSERT INTO order_items (order_id, product_id, quantity, unit_cents) VALUES
    (1, 1, 1, 12900),
    (1, 2, 1,  1500),
    (2, 3, 1, 39900),
    (3, 4, 1,  8900),
    (4, 2, 1,  1500);

-- A materialized view (exercises the "Materialized Views" folder).
CREATE MATERIALIZED VIEW mv_orders_by_status AS
    SELECT status, count(*) AS n, sum(total_cents) AS revenue_cents
    FROM orders GROUP BY status;

-- A simple SQL function (exercises the "Functions" folder + object detail).
CREATE FUNCTION customer_order_count(cid INTEGER) RETURNS bigint
    LANGUAGE sql STABLE
    AS $$ SELECT count(*) FROM orders WHERE customer_id = cid $$;

-- A second schema so the tree shows more than just `public`.
CREATE SCHEMA reporting;
CREATE TABLE reporting.daily_sales (
    day           DATE PRIMARY KEY,
    orders        INTEGER NOT NULL,
    revenue_cents BIGINT  NOT NULL
);
INSERT INTO reporting.daily_sales VALUES
    ('2026-06-14', 3, 56300),
    ('2026-06-15', 1,  1500);
