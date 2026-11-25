//go:build component

package main

import (
	"context"
	"fmt"
	"testing"

	"github.com/stretchr/testify/suite"
	"github.com/testcontainers/testcontainers-go"
	"github.com/testcontainers/testcontainers-go/wait"
)

// OrdersSuite runs against its own MariaDB Testcontainer.
type OrdersSuite struct {
	suite.Suite
	ctr testcontainers.Container
}

func (s *OrdersSuite) SetupSuite() {
	ctx := context.Background()
	ctr, err := testcontainers.GenericContainer(ctx, testcontainers.GenericContainerRequest{
		ContainerRequest: testcontainers.ContainerRequest{
			Image:        "mariadb:11.8.9",
			Env:          map[string]string{"MARIADB_ROOT_PASSWORD": "secret", "MARIADB_DATABASE": "shop"},
			ExposedPorts: []string{"3306/tcp"},
			WaitingFor:   wait.ForListeningPort("3306/tcp"),
		},
		Started: true,
	})
	s.Require().NoError(err)
	s.ctr = ctr
	host, err := ctr.Host(ctx)
	s.Require().NoError(err)
	port, err := ctr.MappedPort(ctx, "3306/tcp")
	s.Require().NoError(err)
	db, err = openDB(fmt.Sprintf("root:secret@tcp(%s:%s)/shop", host, port.Port()))
	s.Require().NoError(err)
}

func (s *OrdersSuite) TearDownSuite() {
	if s.ctr != nil {
		_ = s.ctr.Terminate(context.Background())
	}
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
