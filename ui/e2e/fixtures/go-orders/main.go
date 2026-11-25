// A tiny orders service for the agent-debugging e2e
// (ui/e2e/desktop-ide-agent-debug-real.spec.ts): POST /orders writes a row to
// MySQL / MariaDB and a key with a TTL to Redis. ORDERS_DSN and ORDERS_REDIS
// come from the debug launch's env; ORDERS_ADDR is where it listens.
package main

import (
	"database/sql"
	"encoding/json"
	"log"
	"net/http"
	"os"

	_ "github.com/go-sql-driver/mysql"
)

// Config is what a handler sees: the DSN (with its password) is the secret a
// Stop must never show the agent.
type Config struct {
	DSN       string
	RedisAddr string
}

var (
	db  *sql.DB
	cfg Config
)

func main() {
	cfg = Config{DSN: os.Getenv("ORDERS_DSN"), RedisAddr: os.Getenv("ORDERS_REDIS")}
	var err error
	if db, err = openDB(cfg.DSN); err != nil {
		log.Fatalf("db: %v", err)
	}
	http.HandleFunc("/orders", createOrder)
	addr := os.Getenv("ORDERS_ADDR")
	log.Printf("orders listening on %s", addr)
	log.Fatal(http.ListenAndServe(addr, nil))
}

func createOrder(w http.ResponseWriter, r *http.Request) {
	var in struct {
		Customer string `json:"customer"`
		Amount   int    `json:"amount"`
	}
	if err := json.NewDecoder(r.Body).Decode(&in); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	password := "hunter2-local"
	conf := cfg
	id, err := insertOrder(db, in.Customer, in.Amount)
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	if err := markPaid(db, id); err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	if err := redisSetEx(conf.RedisAddr, orderKey(id), "paid", 300); err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	_ = password
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusCreated)
	_ = json.NewEncoder(w).Encode(map[string]any{"id": id, "status": "paid"})
}
