// A small, fictional multi-language project ("acme-ledger") for the IDE
// chapter: Python settlement code with pytest tests (run + debug), a Go API
// and a TypeScript web client. One commit, then an uncommitted change so the
// editor's git gutter has something to show. Lives under the capture's temp HOME.
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

export const SETTLEMENT = (edited) => `"""Payout settlement for Acme marketplace sellers."""
import json
from dataclasses import dataclass


@dataclass
class Payout:
    seller: str
    gross_cents: int
    refunds_cents: int = 0


PLATFORM_FEE_BPS = 250  # 2.5 %
${edited ? 'MIN_FEE_CENTS = 30\n' : ''}

def platform_fee(gross_cents: int) -> int:
    """The platform's cut of a payout, in cents (rounded down)."""
    return gross_cents * PLATFORM_FEE_BPS // 10_000


def settle(payout: Payout) -> int:
    net = payout.gross_cents - payout.refunds_cents
${edited ? '    fee = max(platform_fee(net), MIN_FEE_CENTS)\n    return net - fee\n' : '    return net - platform_fee(net)\n'}

def run(batch: list[Payout]) -> dict[str, int]:
    totals = {}
    for payout in batch:
        totals[payout.seller] = settle(payout)
    return totals


if __name__ == "__main__":
    batch = [
        Payout("north-wind", 128_400, 2_100),
        Payout("blue-fern", 86_250),
        Payout("ember-co", 41_990, 990),
    ]
    for seller, cents in run(batch).items():
        print(json.dumps({"event": "settled", "seller": seller, "net_cents": cents}))
`;

const TESTS = `from pyapp.settlement import Payout, platform_fee, run, settle


def test_fee_is_two_and_a_half_percent():
    assert platform_fee(10_000) == 250


def test_refunds_come_off_before_the_fee():
    assert settle(Payout("north-wind", 10_000, 2_000)) == 7_800


def test_small_payouts_pay_the_minimum_fee():
    assert settle(Payout("ember-co", 400)) == 370


def test_batch_settles_every_seller():
    totals = run([Payout("a", 1_000), Payout("b", 2_000)])
    assert set(totals) == {"a", "b"}
`;

const GO_MAIN = `package main

import (
\t"encoding/json"
\t"log"
\t"net/http"
)

// Balance is a seller's settled balance, in cents.
type Balance struct {
\tSeller   string \`json:"seller"\`
\tNetCents int    \`json:"net_cents"\`
}

// balanceHandler returns the latest settled balance for ?seller=.
func balanceHandler(w http.ResponseWriter, r *http.Request) {
\tseller := r.URL.Query().Get("seller")
\tjson.NewEncoder(w).Encode(Balance{Seller: seller, NetCents: 122_092})
}

func main() {
\thttp.HandleFunc("/v1/balance", balanceHandler)
\tlog.Fatal(http.ListenAndServe(":8085", nil))
}
`;

const TOTALS = `export interface Line {
  sku: string;
  priceCents: number;
  qty: number;
}

/** Formats cents as a price, e.g. 12990 → "$129.90". */
export function formatCents(cents: number): string {
  return \`$\${(cents / 100).toFixed(2)}\`;
}

export function orderTotal(lines: Line[]): number {
  return lines.reduce((sum, l) => sum + l.priceCents * l.qty, 0);
}

export const label: number = formatCents(orderTotal([{ sku: "mug", priceCents: 1450, qty: 2 }]));
`;

export function buildLedger(root, home) {
  const write = (rel, body) => {
    const f = join(root, rel);
    mkdirSync(dirname(f), { recursive: true });
    writeFileSync(f, body);
  };
  write('README.md', '# acme-ledger\n\nSeller payouts for the Acme marketplace: settlement (Python), the balance API (Go) and the web totals (TypeScript).\n\n```sh\npytest -q\npython -m pyapp.settlement\n```\n');
  write('.gitignore', '__pycache__/\n.pytest_cache/\nnode_modules/\n');
  write('pyapp/__init__.py', '');
  write('pyapp/settlement.py', SETTLEMENT(false));
  write('tests/__init__.py', '');
  write('tests/test_settlement.py', TESTS);
  write('pytest.ini', '[pytest]\ntestpaths = tests\n');
  write('api/go.mod', 'module example.com/acme/ledger-api\n\ngo 1.22\n');
  write('api/main.go', GO_MAIN);
  write('web/package.json', JSON.stringify({ name: 'acme-ledger-web', version: '1.4.0', private: true, type: 'module' }, null, 2) + '\n');
  write('web/tsconfig.json', JSON.stringify({ compilerOptions: { target: 'ES2022', module: 'ESNext', strict: true, noEmit: true }, include: ['src'] }, null, 2) + '\n');
  write('web/src/totals.ts', TOTALS);
  const env = {
    ...process.env,
    HOME: home,
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_AUTHOR_NAME: 'Maya Chen',
    GIT_AUTHOR_EMAIL: 'maya@example.com',
    GIT_COMMITTER_NAME: 'Maya Chen',
    GIT_COMMITTER_EMAIL: 'maya@example.com',
    GIT_AUTHOR_DATE: '2026-11-12T10:20:00Z',
    GIT_COMMITTER_DATE: '2026-11-12T10:20:00Z',
  };
  for (const args of [['init', '-q', '-b', 'main'], ['add', '-A'], ['commit', '-q', '-m', 'feat: seller payout settlement']]) execFileSync('git', args, { cwd: root, env });
  // Work in progress: a minimum fee (the gutter's added/changed lines).
  write('pyapp/settlement.py', SETTLEMENT(true));
  return root;
}
