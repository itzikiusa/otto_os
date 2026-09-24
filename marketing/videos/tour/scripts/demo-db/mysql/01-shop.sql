-- Otto tour film: a small, entirely fictional storefront database.
-- Loaded by the throwaway MariaDB container that scripts/capture.mjs starts.
CREATE DATABASE IF NOT EXISTS shopdb;
USE shopdb;

CREATE TABLE customers (
  id INT PRIMARY KEY,
  name VARCHAR(80) NOT NULL,
  email VARCHAR(120) NOT NULL,
  country CHAR(2) NOT NULL,
  plan ENUM('free','pro','team','enterprise') NOT NULL,
  created_at DATETIME NOT NULL
);

CREATE TABLE products (
  id INT PRIMARY KEY,
  sku VARCHAR(16) NOT NULL,
  name VARCHAR(80) NOT NULL,
  category VARCHAR(32) NOT NULL,
  price_cents INT NOT NULL,
  in_stock TINYINT(1) NOT NULL
);

CREATE TABLE orders (
  id INT PRIMARY KEY,
  customer_id INT NOT NULL,
  status ENUM('pending','paid','shipped','refunded') NOT NULL,
  region VARCHAR(16) NOT NULL,
  total_cents INT NOT NULL,
  created_at DATETIME NOT NULL,
  KEY idx_orders_customer (customer_id),
  KEY idx_orders_created (created_at)
);

CREATE TABLE order_items (
  order_id INT NOT NULL,
  product_id INT NOT NULL,
  qty INT NOT NULL,
  PRIMARY KEY (order_id, product_id)
);

INSERT INTO products VALUES
 (1,'KB-101','Mechanical Keyboard','peripherals',12900,1),
 (2,'CB-202','USB-C Cable 2m','accessories',1500,1),
 (3,'MN-303','27" 4K Monitor','displays',39900,0),
 (4,'WC-404','1080p Webcam','peripherals',8900,1),
 (5,'MS-505','Ergonomic Mouse','peripherals',5900,1),
 (6,'DK-606','Thunderbolt Dock','accessories',21900,1),
 (7,'HS-707','Noise-cancel Headset','audio',17900,1),
 (8,'SP-808','Desk Speakers','audio',9900,0),
 (9,'LS-909','Laptop Stand','accessories',4900,1),
 (10,'MN-310','34" Ultrawide','displays',64900,1),
 (11,'MC-111','Studio Microphone','audio',13900,1),
 (12,'LT-112','Key Light','accessories',7900,1);

INSERT INTO customers
SELECT seq,
  ELT(1 + (seq % 20), 'Ada Park','Noah Levi','Mia Rossi','Liam Chen','Zoe Novak','Omar Haddad','Iris Tanaka','Leo Martin',
      'Nora Silva','Ethan Brooks','Ava Kowalski','Lucas Meyer','Sofia Duarte','Kai Nakamura','Yara Aziz','Theo Laurent',
      'Maya Singh','Ivan Petrov','Lena Fischer','Sam Okafor'),
  CONCAT('customer', seq, '@example.com'),
  ELT(1 + (seq % 8), 'US','GB','DE','FR','IL','JP','BR','CA'),
  ELT(1 + (seq * 7 % 4), 'free','pro','team','enterprise'),
  TIMESTAMP('2026-01-01') + INTERVAL (seq * 37 % 240) DAY
FROM seq_1_to_60;

INSERT INTO orders
SELECT seq,
  1 + (seq * 13 % 60),
  ELT(1 + (seq * 11 % 10), 'paid','paid','shipped','paid','shipped','pending','paid','shipped','refunded','paid'),
  ELT(1 + (seq % 5), 'us-east','eu-west','eu-central','ap-south','sa-east'),
  0,
  TIMESTAMP('2026-06-01') + INTERVAL (seq * 17 % 110) DAY + INTERVAL (seq * 97 % 1440) MINUTE
FROM seq_1_to_480;

INSERT INTO order_items
SELECT o.seq, 1 + (o.seq * 3 % 12), 1 + (o.seq % 3) FROM seq_1_to_480 o;
INSERT IGNORE INTO order_items
SELECT o.seq, 1 + (o.seq * 7 % 12), 1 FROM seq_1_to_480 o WHERE o.seq % 2 = 0;

UPDATE orders o
JOIN (SELECT oi.order_id, SUM(oi.qty * p.price_cents) AS t
      FROM order_items oi JOIN products p ON p.id = oi.product_id GROUP BY oi.order_id) x
  ON x.order_id = o.id
SET o.total_cents = x.t;

CREATE TABLE daily_sales AS
SELECT DATE(created_at) AS day, region, COUNT(*) AS orders, SUM(total_cents) AS revenue_cents
FROM orders WHERE status <> 'refunded' GROUP BY DATE(created_at), region;

-- Foreign keys: the visual query builder auto-joins along them.
ALTER TABLE orders ADD CONSTRAINT fk_orders_customer FOREIGN KEY (customer_id) REFERENCES customers(id);
ALTER TABLE order_items ADD CONSTRAINT fk_items_order FOREIGN KEY (order_id) REFERENCES orders(id);
ALTER TABLE order_items ADD CONSTRAINT fk_items_product FOREIGN KEY (product_id) REFERENCES products(id);

GRANT ALL PRIVILEGES ON shopdb.* TO 'otto'@'%';
