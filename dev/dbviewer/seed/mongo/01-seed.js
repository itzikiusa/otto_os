// Seed the Otto DB Explorer dev MongoDB instance.
// Runs in the MONGO_INITDB_DATABASE (shopdb) on first container start.
db = db.getSiblingDB('shopdb');

db.customers.insertMany([
  { _id: 1, email: 'ada@example.com',   name: 'Ada Lovelace',   country: 'GB', tags: ['vip', 'early'] },
  { _id: 2, email: 'alan@example.com',  name: 'Alan Turing',    country: 'GB', tags: ['early'] },
  { _id: 3, email: 'grace@example.com', name: 'Grace Hopper',   country: 'US', tags: [] },
  { _id: 4, email: 'linus@example.com', name: 'Linus Torvalds', country: 'FI', tags: ['vip'] },
]);

db.products.insertMany([
  { _id: 1, sku: 'SKU-1', name: 'Mechanical Keyboard', priceCents: 12900, inStock: true,  meta: { color: 'black', switches: 'brown' } },
  { _id: 2, sku: 'SKU-2', name: 'USB-C Cable',          priceCents: 1500,  inStock: true,  meta: { lengthM: 2 } },
  { _id: 3, sku: 'SKU-3', name: '4K Monitor',           priceCents: 39900, inStock: false, meta: {} },
  { _id: 4, sku: 'SKU-4', name: 'Webcam',               priceCents: 8900,  inStock: true,  meta: { resolution: '1080p' } },
]);

db.orders.insertMany([
  { _id: 1, customerId: 1, status: 'paid',    totalCents: 14400, items: [{ productId: 1, qty: 1 }, { productId: 2, qty: 1 }] },
  { _id: 2, customerId: 1, status: 'shipped', totalCents: 39900, items: [{ productId: 3, qty: 1 }] },
  { _id: 3, customerId: 2, status: 'pending', totalCents: 8900,  items: [{ productId: 4, qty: 1 }] },
  { _id: 4, customerId: 3, status: 'paid',    totalCents: 1500,  items: [{ productId: 2, qty: 1 }] },
]);

db.customers.createIndex({ email: 1 }, { unique: true });
db.orders.createIndex({ customerId: 1 });
db.orders.createIndex({ status: 1 });

// A second database so the explorer shows multiple DBs.
const a = db.getSiblingDB('analytics');
a.events.insertMany([
  { type: 'page_view', path: '/', ts: new Date('2026-06-15T10:00:00Z') },
  { type: 'add_to_cart', productId: 1, ts: new Date('2026-06-15T10:02:00Z') },
  { type: 'checkout', orderId: 1, ts: new Date('2026-06-15T10:05:00Z') },
]);
