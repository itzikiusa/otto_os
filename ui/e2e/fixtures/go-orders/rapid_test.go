package main

import (
	"fmt"
	"os"
	"testing"
)

// TestRapid inserts 20 orders, one per loop turn: a breakpoint on the insert
// gives 20 quick stops for DB Watch (ui/e2e/desktop-ide-dbwatch-real.spec.ts).
func TestRapid(t *testing.T) {
	dsn := os.Getenv("ORDERS_DSN")
	if dsn == "" {
		t.Skip("ORDERS_DSN is not set")
	}
	d, err := openDB(dsn)
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 20; i++ {
		if _, err := insertOrder(d, fmt.Sprintf("tick%02d", i), i); err != nil {
			t.Fatal(err)
		}
	}
}
