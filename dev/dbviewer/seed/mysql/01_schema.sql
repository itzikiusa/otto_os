-- Seed schema for the Otto DB Explorer dev MySQL instance.
-- Designed to exercise schema introspection, foreign keys (for the visual
-- JOIN builder), indexes, views, and varied column types.

CREATE DATABASE IF NOT EXISTS shopdb;
USE shopdb;

CREATE TABLE customers (
  id           INT AUTO_INCREMENT PRIMARY KEY,
  email        VARCHAR(255) NOT NULL UNIQUE,
  full_name    VARCHAR(255) NOT NULL,
  country      VARCHAR(2)   NOT NULL DEFAULT 'US',
  created_at   TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_customers_country (country)
) ENGINE=InnoDB;

CREATE TABLE products (
  id           INT AUTO_INCREMENT PRIMARY KEY,
  sku          VARCHAR(64)  NOT NULL UNIQUE,
  name         VARCHAR(255) NOT NULL,
  price_cents  INT          NOT NULL,
  in_stock     TINYINT(1)   NOT NULL DEFAULT 1,
  metadata     JSON         NULL
) ENGINE=InnoDB;

CREATE TABLE orders (
  id           INT AUTO_INCREMENT PRIMARY KEY,
  customer_id  INT          NOT NULL,
  status       ENUM('pending','paid','shipped','cancelled') NOT NULL DEFAULT 'pending',
  total_cents  INT          NOT NULL DEFAULT 0,
  placed_at    DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT fk_orders_customer FOREIGN KEY (customer_id) REFERENCES customers(id) ON DELETE CASCADE,
  KEY idx_orders_status (status)
) ENGINE=InnoDB;

CREATE TABLE order_items (
  id           INT AUTO_INCREMENT PRIMARY KEY,
  order_id     INT          NOT NULL,
  product_id   INT          NOT NULL,
  quantity     INT          NOT NULL DEFAULT 1,
  unit_cents   INT          NOT NULL,
  CONSTRAINT fk_items_order   FOREIGN KEY (order_id)   REFERENCES orders(id)   ON DELETE CASCADE,
  CONSTRAINT fk_items_product FOREIGN KEY (product_id) REFERENCES products(id)
) ENGINE=InnoDB;

CREATE VIEW order_totals AS
  SELECT o.id AS order_id, c.email, o.status, o.total_cents
  FROM orders o JOIN customers c ON c.id = o.customer_id;

INSERT INTO customers (email, full_name, country) VALUES
  ('ada@example.com',  'Ada Lovelace',   'GB'),
  ('alan@example.com', 'Alan Turing',    'GB'),
  ('grace@example.com','Grace Hopper',   'US'),
  ('linus@example.com','Linus Torvalds', 'FI');

INSERT INTO products (sku, name, price_cents, in_stock, metadata) VALUES
  ('SKU-1', 'Mechanical Keyboard', 12900, 1, JSON_OBJECT('color','black','switches','brown')),
  ('SKU-2', 'USB-C Cable',          1500, 1, JSON_OBJECT('length_m',2)),
  ('SKU-3', '4K Monitor',          39900, 0, NULL),
  ('SKU-4', 'Webcam',               8900, 1, JSON_OBJECT('resolution','1080p'));

INSERT INTO orders (customer_id, status, total_cents) VALUES
  (1, 'paid',    14400),
  (1, 'shipped', 39900),
  (2, 'pending', 8900),
  (3, 'paid',    1500);

INSERT INTO order_items (order_id, product_id, quantity, unit_cents) VALUES
  (1, 1, 1, 12900),
  (1, 2, 1, 1500),
  (2, 3, 1, 39900),
  (3, 4, 1, 8900),
  (4, 2, 1, 1500);

-- A second database so the explorer shows multiple schemas.
CREATE DATABASE IF NOT EXISTS analytics_mirror;
USE analytics_mirror;
CREATE TABLE daily_sales (
  day        DATE PRIMARY KEY,
  orders     INT NOT NULL,
  revenue_cents BIGINT NOT NULL
) ENGINE=InnoDB;
INSERT INTO daily_sales VALUES ('2026-06-14', 3, 56300), ('2026-06-15', 1, 1500);

-- Make sure the otto user can see both databases.
GRANT ALL PRIVILEGES ON shopdb.* TO 'otto'@'%';
GRANT ALL PRIVILEGES ON analytics_mirror.* TO 'otto'@'%';
FLUSH PRIVILEGES;
