package service

func (s *SinatraServiceImpl) send(ctx context.Context, url string) (string, error) {
	if val, err := s.cache.Get(ctx, url); err == nil {
		logger.InfoF(ctx, ":sendToSinatra: returned response from cache, key = %s, response = %s", url, val)
		return val, nil
	}
	resp, err := s.restClient.GetRequest(ctx, url)
	if err != nil {
		logger.ErrorF(ctx, "failed to get data from sinatra, error = %s", err.Error())
		return "", errors.New("failed to get data from Sinatra")
	}
	return string(resp.Body), nil
}
