package cache

import "github.com/redis/go-redis/v9"

func (s *Store) Load(ctx context.Context, id int) {
	_ = s.rdb.HGetAll(ctx, "session:token:abc")
	_ = s.rdb.Set(ctx, fmt.Sprintf("player:bal:%d", id), "100", 0)
	// s.rdb.Del(ctx, "legacy:key:1")
}
