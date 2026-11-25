package main

import (
	"bufio"
	"database/sql"
	"fmt"
	"net"
	"strconv"
	"time"
)

func openDB(dsn string) (*sql.DB, error) {
	d, err := sql.Open("mysql", dsn)
	if err != nil {
		return nil, err
	}
	for i := 0; ; i++ {
		if err = d.Ping(); err == nil || i == 60 {
			break
		}
		time.Sleep(500 * time.Millisecond)
	}
	if err != nil {
		return nil, err
	}
	_, err = d.Exec(`CREATE TABLE IF NOT EXISTS orders (
		id INT AUTO_INCREMENT PRIMARY KEY,
		customer VARCHAR(64) NOT NULL,
		amount INT NOT NULL,
		status VARCHAR(16) NOT NULL,
		api_token VARCHAR(64) NOT NULL
	)`)
	return d, err
}

func insertOrder(d *sql.DB, customer string, amount int) (int64, error) {
	res, err := d.Exec("INSERT INTO orders (customer, amount, status, api_token) VALUES (?, ?, 'new', ?)",
		customer, amount, fmt.Sprintf("tok-%s-%d", customer, amount))
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

func markPaid(d *sql.DB, id int64) error {
	_, err := d.Exec("UPDATE orders SET status = 'paid' WHERE id = ?", id)
	return err
}

func orderKey(id int64) string { return "order:" + strconv.FormatInt(id, 10) }

// redisSetEx is SET key value EX ttl over plain RESP (no client library).
func redisSetEx(addr, key, value string, ttl int) error {
	c, err := net.DialTimeout("tcp", addr, 3*time.Second)
	if err != nil {
		return err
	}
	defer c.Close()
	args := []string{"SET", key, value, "EX", strconv.Itoa(ttl)}
	msg := fmt.Sprintf("*%d\r\n", len(args))
	for _, a := range args {
		msg += fmt.Sprintf("$%d\r\n%s\r\n", len(a), a)
	}
	if _, err := c.Write([]byte(msg)); err != nil {
		return err
	}
	line, err := bufio.NewReader(c).ReadString('\n')
	if err != nil {
		return err
	}
	if len(line) == 0 || line[0] != '+' {
		return fmt.Errorf("redis: %q", line)
	}
	return nil
}
