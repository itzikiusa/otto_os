package client

func call(ctx context.Context, code int) string {
	_, _ = restClient.Get(ctx, "http://wallet:8080/balance")
	return fmt.Sprintf("status:%d", code)
}
