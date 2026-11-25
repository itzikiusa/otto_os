package service

func TestRealityCheck_Error_GetAllParams(t *testing.T) {
	redisMock := new(redis_factory.MockRedisFactory)
	rcParams := map[string]string{}
	redisMock.On("HGetAll", mock.Anything, mock.Anything).Return(rcParams, errors.New("error getting params from redis"))
	redisMock.On("HSet", mock.Anything, mock.Anything, mock.Anything, mock.Anything).Return(nil)
	assert.Equal(t, err.Error(), "error getting params from redis")
}
