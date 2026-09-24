// Otto tour film: a fictional product catalog + event stream for MongoDB.
db = db.getSiblingDB('shopdb');

const names = ['Ada Park', 'Noah Levi', 'Mia Rossi', 'Liam Chen', 'Zoe Novak', 'Omar Haddad', 'Iris Tanaka', 'Leo Martin', 'Nora Silva', 'Ethan Brooks', 'Ava Kowalski', 'Lucas Meyer'];
const countries = ['US', 'GB', 'DE', 'FR', 'IL', 'JP', 'BR', 'CA'];
const plans = ['free', 'pro', 'team', 'enterprise'];

db.customers.insertMany(
  names.map((n, i) => ({
    _id: i + 1,
    name: n,
    email: `customer${i + 1}@example.com`,
    country: countries[i % countries.length],
    plan: plans[(i * 7) % 4],
    tags: i % 3 === 0 ? ['vip', 'beta'] : i % 2 ? ['newsletter'] : [],
    address: { city: ['Austin', 'London', 'Berlin', 'Paris', 'Haifa', 'Osaka'][i % 6], zip: String(10000 + i * 137) },
    createdAt: new Date(Date.UTC(2026, 0, 1 + i * 9)),
  })),
);

const cats = ['peripherals', 'accessories', 'displays', 'audio'];
db.products.insertMany(
  ['Mechanical Keyboard', 'USB-C Cable', '4K Monitor', 'Webcam', 'Ergonomic Mouse', 'Thunderbolt Dock', 'Headset', 'Desk Speakers'].map((n, i) => ({
    _id: i + 1,
    sku: `SKU-${100 + i}`,
    name: n,
    category: cats[i % 4],
    priceCents: [12900, 1500, 39900, 8900, 5900, 21900, 17900, 9900][i],
    inStock: i % 5 !== 2,
    meta: { color: ['black', 'white', 'graphite'][i % 3], warrantyMonths: 12 + (i % 3) * 12 },
  })),
);

const statuses = ['paid', 'paid', 'shipped', 'pending', 'refunded'];
const orders = [];
for (let i = 1; i <= 240; i++) {
  const items = [{ productId: 1 + ((i * 3) % 8), qty: 1 + (i % 3) }];
  if (i % 2 === 0) items.push({ productId: 1 + ((i * 5) % 8), qty: 1 });
  orders.push({
    _id: i,
    customerId: 1 + ((i * 13) % 12),
    status: statuses[(i * 11) % 5],
    region: ['us-east', 'eu-west', 'eu-central', 'ap-south'][i % 4],
    items,
    totalCents: items.reduce((s, it) => s + it.qty * [12900, 1500, 39900, 8900, 5900, 21900, 17900, 9900][it.productId - 1], 0),
    shipping: { carrier: ['UPS', 'DHL', 'FedEx'][i % 3], eta: new Date(Date.UTC(2026, 8, 1 + (i % 28))) },
    createdAt: new Date(Date.UTC(2026, 5, 1 + (i % 110))),
  });
}
db.orders.insertMany(orders);
db.orders.createIndex({ customerId: 1, createdAt: -1 });

