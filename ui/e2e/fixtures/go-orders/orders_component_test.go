//go:build component

package main

import (
	"os"
	"testing"

	"github.com/stretchr/testify/suite"
)

// OrdersSuite runs against the run's own database: ORDERS_TEST_DSN names a
// container started for this run (the e2e starts it with Testcontainers'
// labels, which is how Otto recognises a run's own database).
type OrdersSuite struct {
	suite.Suite
}

func (s *OrdersSuite) SetupSuite() {
	dsn := os.Getenv("ORDERS_TEST_DSN")
	if dsn == "" {
		s.T().Skip("ORDERS_TEST_DSN is not set")
	}
	var err error
	db, err = openDB(dsn)
	s.Require().NoError(err)
}

func (s *OrdersSuite) TestCreate() {
	id, err := insertOrder(db, "ava", 42)
	s.Require().NoError(err)
	s.Require().NoError(markPaid(db, id))
	var status string
	s.Require().NoError(db.QueryRow("SELECT status FROM orders WHERE id = ?", id).Scan(&status))
	s.Equal("paid", status)
}

func TestOrdersSuite(t *testing.T) {
	suite.Run(t, new(OrdersSuite))
}
